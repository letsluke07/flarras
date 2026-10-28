use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;

// --- Timing constants ---

pub const DEFAULT_TICK_RATE: f32 = 64.0;
pub const DEFAULT_SIM_DT: f32 = 1.0 / DEFAULT_TICK_RATE;
pub const DEFAULT_MAX_TICKS_PER_FRAME: u32 = 5;

// --- SimClock ---

#[derive(Debug, Clone)]
pub struct SimClock {
    accumulator: f32,
    step: f32,
    max_ticks_per_frame: u32,
}

impl SimClock {
    pub fn new(step: f32, max_ticks_per_frame: u32) -> Self {
        Self {
            accumulator: 0.0,
            step,
            max_ticks_per_frame,
        }
    }

    pub fn default() -> Self {
        Self::new(DEFAULT_SIM_DT, DEFAULT_MAX_TICKS_PER_FRAME)
    }

    pub fn step(&self) -> f32 {
        self.step
    }

    pub fn consume(&mut self, frame_dt: f32) -> u32 {
        self.accumulator += frame_dt.max(0.0);

        let max_accumulated = self.step * self.max_ticks_per_frame as f32;
        if self.accumulator > max_accumulated {
            self.accumulator = max_accumulated;
        }

        let mut ticks = 0;
        while self.accumulator >= self.step && ticks < self.max_ticks_per_frame {
            self.accumulator -= self.step;
            ticks += 1;
        }
        ticks
    }
}

// --- SimRng ---

#[derive(Resource, Debug, Clone)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    pub fn seed(seed: u64) -> Self {
        let mut state = seed;
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z = z ^ (z >> 31);

        Self { state: z.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    pub fn next_angle(&mut self) -> f32 {
        self.next_f32() * std::f32::consts::TAU
    }
}

// --- MobSpawnTimer ---

#[derive(Resource, Debug, Clone, Copy)]
pub struct MobSpawnTimer(pub f32);

impl Default for MobSpawnTimer {
    fn default() -> Self {
        Self(MOB_SPAWN_INTERVAL)
    }
}

// --- Spawning ---

pub fn spawn_bullet(world: &mut World, origin: Vec2, direction: Vec2) -> Entity {
    let velocity = direction.normalize_or_zero() * BULLET_SPEED;
    world
        .spawn((
            Bullet,
            Position(origin),
            Velocity(velocity),
            Lifetime(BULLET_LIFETIME),
            Damage(BULLET_DAMAGE),
        ))
        .id()
}

pub fn spawn_mob(world: &mut World, position: Vec2) -> Entity {
    world
        .spawn((
            Mob,
            Position(position),
            Velocity(Vec2::ZERO),
            Health::full(MOB_MAX_HP),
            ContactCooldown::default(),
        ))
        .id()
}

// --- Tick ---

pub fn tick(world: &mut World, inputs: &[(Entity, InputCommand)], dt: f32) {
    let playing = world
        .get_resource::<GameState>()
        .map(|s| *s == GameState::Playing)
        .unwrap_or(true);
    if !playing {
        return;
    }

    tick_cooldowns(world, dt);
    spawn_mobs(world, dt);
    apply_inputs(world, inputs, dt);
    update_mob_ai(world, dt);
    integrate_positions(world, dt);
    apply_collisions(world);
    check_player_death(world);
    tick_lifetimes_and_despawn(world, dt);
}

fn tick_cooldowns(world: &mut World, dt: f32) {
    tick_fire_cooldowns(world, dt);
    tick_contact_cooldowns(world, dt);
}

fn tick_fire_cooldowns(world: &mut World, dt: f32) {
    let mut query = world.query::<&mut FireCooldown>();
    for mut cd in query.iter_mut(world) {
        cd.remaining = (cd.remaining - dt).max(0.0);
    }
}

fn tick_contact_cooldowns(world: &mut World, dt: f32) {
    let mut query = world.query::<&mut ContactCooldown>();
    for mut cd in query.iter_mut(world) {
        cd.remaining = (cd.remaining - dt).max(0.0);
    }
}

fn apply_inputs(world: &mut World, inputs: &[(Entity, InputCommand)], dt: f32) {
    for (entity, command) in inputs {
        match command {
            InputCommand::Move { direction } => {
                let target = direction.normalize_or_zero() * TANK_SPEED;
                if let Some(mut vel) = world.get_mut::<Velocity>(*entity) {
                    vel.0 = approach(vel.0, target, TANK_ACCELERATION * dt);
                }
            }

            InputCommand::Fire { direction } => {
                let ready = world
                    .get::<FireCooldown>(*entity)
                    .map(|cd| cd.remaining <= 0.0)
                    .unwrap_or(false);
                if !ready {
                    continue;
                }

                let origin = world.get::<Position>(*entity).map(|p| p.0);
                if let Some(origin) = origin {
                    spawn_bullet(world, origin, *direction);
                    if let Some(mut cd) = world.get_mut::<FireCooldown>(*entity) {
                        cd.remaining = FIRE_INTERVAL;
                    }
                }
            }
        }
    }
}

fn update_mob_ai(world: &mut World, dt: f32) {
    let player_positions: Vec<Vec2> = world
        .query_filtered::<&Position, With<Player>>()
        .iter(world)
        .map(|p| p.0)
        .collect();

    let contact_range = MOB_RADIUS + TANK_RADIUS;
    let contact_sq = contact_range * contact_range;

    if player_positions.is_empty() {
        let mut query = world.query_filtered::<&mut Velocity, With<Mob>>();
        for mut vel in query.iter_mut(world) {
            vel.0 = Vec2::ZERO;
        }
        return;
    }

    let mut query = world.query_filtered::<(&Position, &mut Velocity), With<Mob>>();
    for (pos, mut vel) in query.iter_mut(world) {
        let mut closest = player_positions[0];
        let mut best_dist_sq = pos.0.distance_squared(closest);
        for p in &player_positions[1..] {
            let d = pos.0.distance_squared(*p);
            if d < best_dist_sq {
                best_dist_sq = d;
                closest = *p;
            }
        }

        let target = if best_dist_sq <= contact_sq {
            Vec2::ZERO
        } else {
            let dir = (closest - pos.0).normalize_or_zero();
            dir * MOB_SPEED
        };
        vel.0 = approach(vel.0, target, MOB_ACCELERATION * dt);
    }
}

fn integrate_positions(world: &mut World, dt: f32) {
    let mut query = world.query::<(&Velocity, &mut Position)>();
    for (vel, mut pos) in query.iter_mut(world) {
        pos.0 += vel.0 * dt;
    }
}

fn apply_collisions(world: &mut World) {
    apply_bullet_mob_collisions(world);
    apply_mob_player_contacts(world);
}

fn apply_bullet_mob_collisions(world: &mut World) {
    let mobs: Vec<(Entity, Vec2)> = world
        .query_filtered::<(Entity, &Position), With<Mob>>()
        .iter(world)
        .map(|(e, p)| (e, p.0))
        .collect();

    let bullets: Vec<(Entity, Vec2, f32)> = world
        .query_filtered::<(Entity, &Position, &Damage), With<Bullet>>()
        .iter(world)
        .map(|(e, p, d)| (e, p.0, d.0))
        .collect();

    let hit_range = BULLET_RADIUS + MOB_RADIUS;
    let hit_sq = hit_range * hit_range;

    let mut bullets_to_despawn: Vec<Entity> = Vec::new();
    let mut damage_events: Vec<(Entity, f32)> = Vec::new();

    for (bullet_entity, bullet_pos, damage) in &bullets {
        for (mob_entity, mob_pos) in &mobs {
            if bullet_pos.distance_squared(*mob_pos) <= hit_sq {
                bullets_to_despawn.push(*bullet_entity);
                damage_events.push((*mob_entity, *damage));
                break;
            }
        }
    }

    let mut mobs_to_despawn: Vec<Entity> = Vec::new();
    for (mob_entity, damage) in damage_events {
        if let Some(mut hp) = world.get_mut::<Health>(mob_entity) {
            hp.take_damage(damage);
            if hp.is_dead() && !mobs_to_despawn.contains(&mob_entity) {
                mobs_to_despawn.push(mob_entity);
            }
        }
    }

    for bullet in bullets_to_despawn {
        let _ = world.despawn(bullet);
    }
    for mob in mobs_to_despawn {
        let _ = world.despawn(mob);
    }
}

fn apply_mob_player_contacts(world: &mut World) {
    let players: Vec<(Entity, Vec2)> = world
        .query_filtered::<(Entity, &Position), With<Player>>()
        .iter(world)
        .map(|(e, p)| (e, p.0))
        .collect();

    let mobs: Vec<(Entity, Vec2)> = world
        .query_filtered::<(Entity, &Position), With<Mob>>()
        .iter(world)
        .map(|(e, p)| (e, p.0))
        .collect();

    let contact_range = MOB_RADIUS + TANK_RADIUS;
    let contact_sq = contact_range * contact_range;

    for (mob_entity, mob_pos) in &mobs {
        let ready = world
            .get::<ContactCooldown>(*mob_entity)
            .map(|cd| cd.remaining <= 0.0)
            .unwrap_or(false);
        if !ready {
            continue;
        }

        for (player_entity, player_pos) in &players {
            if mob_pos.distance_squared(*player_pos) <= contact_sq {
                if let Some(mut hp) = world.get_mut::<Health>(*player_entity) {
                    hp.take_damage(MOB_CONTACT_DAMAGE);
                }
                if let Some(mut cd) = world.get_mut::<ContactCooldown>(*mob_entity) {
                    cd.remaining = CONTACT_INTERVAL;
                }
            }
        }
    }
}

fn check_player_death(world: &mut World) {
    let playing = world
        .get_resource::<GameState>()
        .map(|s| *s == GameState::Playing)
        .unwrap_or(true);
    if !playing {
        return;
    }

    let any_dead = world
        .query_filtered::<&Health, With<Player>>()
        .iter(world)
        .any(|hp| hp.is_dead());

    if any_dead {
        if let Some(mut state) = world.get_resource_mut::<GameState>() {
            *state = GameState::Dead;
        } else {
            world.insert_resource(GameState::Dead);
        }
    }
}

fn tick_lifetimes_and_despawn(world: &mut World, dt: f32) {
    let mut query = world.query::<(&mut Lifetime, Entity)>();
    let mut to_despawn: Vec<Entity> = Vec::new();

    for (mut life, entity) in query.iter_mut(world) {
        life.0 -= dt;
        if life.0 <= 0.0 {
            to_despawn.push(entity);
        }
    }

    for entity in to_despawn {
        let _ = world.despawn(entity);
    }
}

fn spawn_mobs(world: &mut World, dt: f32) {
    let ready = match world.get_resource_mut::<MobSpawnTimer>() {
        Some(mut timer) => {
            timer.0 -= dt;
            timer.0 <= 0.0
        }
        None => return,
    };
    if !ready {
        return;
    }

    let mob_count = world
        .query_filtered::<Entity, With<Mob>>()
        .iter(world)
        .count();
    if mob_count >= MAX_MOBS {
        return;
    }

    let player_pos = world
        .query_filtered::<&Position, With<Player>>()
        .iter(world)
        .next()
        .map(|p| p.0);
    let Some(player_pos) = player_pos else {
        return;
    };

    let angle = match world.get_resource_mut::<SimRng>() {
        Some(mut rng) => rng.next_angle(),
        None => return,
    };

    let offset = Vec2::from_angle(angle) * MOB_SPAWN_DISTANCE;
    spawn_mob(world, player_pos + offset);

    if let Some(mut timer) = world.get_resource_mut::<MobSpawnTimer>() {
        timer.0 = MOB_SPAWN_INTERVAL;
    }
}

// --- Helpers ---

fn approach(current: Vec2, target: Vec2, max_step: f32) -> Vec2 {
    let delta = target - current;
    if delta.length_squared() <= max_step * max_step {
        target
    } else {
        current + delta.normalize() * max_step
    }
}
