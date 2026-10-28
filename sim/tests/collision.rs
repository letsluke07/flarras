use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;

fn spawn_player_at(world: &mut World, position: Vec2) -> Entity {
    world
        .spawn((
            Player,
            Position(position),
            Velocity::default(),
            FireCooldown::default(),
            Health::full(PLAYER_MAX_HP),
        ))
        .id()
}

fn spawn_mob_at(world: &mut World, position: Vec2) -> Entity {
    spawn_mob(world, position)
}

#[test]
fn bullet_damages_mob_on_hit() {
    let mut world = World::new();
    let mob = spawn_mob_at(&mut world, Vec2::new(100.0, 0.0));
    spawn_bullet(&mut world, Vec2::new(100.0, 0.0), Vec2::X);

    tick(&mut world, &[], DT);

    let hp = world.get::<Health>(mob).unwrap();
    assert!(
        (hp.current - (MOB_MAX_HP - BULLET_DAMAGE)).abs() < 0.01,
        "mob should have taken {BULLET_DAMAGE} damage, got {}",
        hp.current
    );
}

#[test]
fn bullet_despawns_on_hit() {
    let mut world = World::new();
    spawn_mob_at(&mut world, Vec2::new(100.0, 0.0));
    let bullet = spawn_bullet(&mut world, Vec2::new(100.0, 0.0), Vec2::X);

    tick(&mut world, &[], DT);

    assert!(
        world.get::<Bullet>(bullet).is_none(),
        "bullet should despawn after hitting a mob"
    );
}

#[test]
fn bullet_kills_mob_at_zero_hp() {
    let mut world = World::new();
    let mob = spawn_mob_at(&mut world, Vec2::new(100.0, 0.0));
    let hits_needed = (MOB_MAX_HP / BULLET_DAMAGE).ceil() as usize;
    for _ in 0..hits_needed {
        spawn_bullet(&mut world, Vec2::new(100.0, 0.0), Vec2::X);
    }

    tick(&mut world, &[], DT);

    assert!(
        world.get::<Mob>(mob).is_none(),
        "mob should be despawned at zero HP"
    );
}

#[test]
fn multiple_bullets_can_damage_same_mob() {
    let mut world = World::new();
    let mob = spawn_mob_at(&mut world, Vec2::new(100.0, 0.0));
    spawn_bullet(&mut world, Vec2::new(100.0, 0.0), Vec2::X);
    spawn_bullet(&mut world, Vec2::new(100.0, 0.0), Vec2::X);
    spawn_bullet(&mut world, Vec2::new(100.0, 0.0), Vec2::X);

    tick(&mut world, &[], DT);

    let hp = world.get::<Health>(mob).unwrap();
    let expected = MOB_MAX_HP - 3.0 * BULLET_DAMAGE;
    assert!(
        (hp.current - expected).abs() < 0.01,
        "mob should have taken 3×{BULLET_DAMAGE} damage, got {}",
        hp.current
    );
}

#[test]
fn bullet_outside_range_misses() {
    let mut world = World::new();
    let mob = spawn_mob_at(&mut world, Vec2::new(100.0, 0.0));
    let bullet = spawn_bullet(&mut world, Vec2::new(0.0, 0.0), Vec2::X);

    tick(&mut world, &[], DT);

    let hp = world.get::<Health>(mob).unwrap();
    assert!(
        (hp.current - MOB_MAX_HP).abs() < 0.01,
        "mob should not have taken damage, got {}",
        hp.current
    );
    assert!(
        world.get::<Bullet>(bullet).is_some(),
        "bullet should still be alive"
    );
}

#[test]
fn mob_damages_player_on_contact() {
    let mut world = World::new();
    let player = spawn_player_at(&mut world, Vec2::ZERO);
    spawn_mob_at(&mut world, Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0));

    tick(&mut world, &[], DT);

    let hp = world.get::<Health>(player).unwrap();
    assert!(
        (hp.current - (PLAYER_MAX_HP - MOB_CONTACT_DAMAGE)).abs() < 0.01,
        "player should have taken {MOB_CONTACT_DAMAGE} damage, got {}",
        hp.current
    );
}

#[test]
fn contact_cooldown_prevents_repeated_damage() {
    let mut world = World::new();
    let player = spawn_player_at(&mut world, Vec2::ZERO);
    spawn_mob_at(&mut world, Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0));

    tick(&mut world, &[], DT);
    let hp_after_first = world.get::<Health>(player).unwrap().current;

    for _ in 0..10 {
        tick(&mut world, &[], DT);
    }

    let hp_after_wait = world.get::<Health>(player).unwrap().current;
    assert!(
        (hp_after_wait - hp_after_first).abs() < 0.01,
        "no damage should land during cooldown: before={hp_after_first} after={hp_after_wait}"
    );
}

#[test]
fn contact_cooldown_expires_after_interval() {
    let mut world = World::new();
    let player = spawn_player_at(&mut world, Vec2::ZERO);
    spawn_mob_at(&mut world, Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0));

    tick(&mut world, &[], DT);
    let hp_after_first = world.get::<Health>(player).unwrap().current;

    let ticks = (CONTACT_INTERVAL / DT).ceil() as u32 + 1;
    for _ in 0..ticks {
        tick(&mut world, &[], DT);
    }

    let hp_after_second = world.get::<Health>(player).unwrap().current;
    assert!(
        hp_after_second < hp_after_first,
        "damage should have landed again after cooldown: {hp_after_first} -> {hp_after_second}"
    );
}

#[test]
fn player_death_transitions_to_dead_state() {
    let mut world = World::new();
    world.insert_resource(GameState::Playing);
    let player = spawn_player_at(&mut world, Vec2::ZERO);
    world.get_mut::<Health>(player).unwrap().current = MOB_CONTACT_DAMAGE;
    spawn_mob_at(&mut world, Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0));

    tick(&mut world, &[], DT);

    assert_eq!(
        *world.resource::<GameState>(),
        GameState::Dead,
        "game should transition to Dead on player death"
    );
    assert!(
        world.get::<Health>(player).unwrap().is_dead(),
        "player health should be zero"
    );
}

#[test]
fn dead_state_skips_simulation() {
    let mut world = World::new();
    world.insert_resource(GameState::Dead);
    let mob = spawn_mob_at(&mut world, Vec2::new(300.0, 0.0));
    let player = spawn_player_at(&mut world, Vec2::ZERO);

    tick(&mut world, &[], DT);

    let mob_pos = world.get::<Position>(mob).unwrap().0;
    assert_eq!(
        mob_pos,
        Vec2::new(300.0, 0.0),
        "mob should not move in Dead"
    );

    let hp = world.get::<Health>(player).unwrap();
    assert!(
        (hp.current - PLAYER_MAX_HP).abs() < 0.01,
        "player should not take damage in Dead"
    );
}

#[test]
fn killing_blow_protects_player_from_contact() {
    let mut world = World::new();
    let player = spawn_player_at(&mut world, Vec2::ZERO);
    let mob = spawn_mob_at(&mut world, Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0));

    world.get_mut::<Health>(mob).unwrap().current = BULLET_DAMAGE;
    spawn_bullet(
        &mut world,
        Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0),
        Vec2::X,
    );

    tick(&mut world, &[], DT);

    let hp = world.get::<Health>(player).unwrap();
    assert!(
        (hp.current - PLAYER_MAX_HP).abs() < 0.01,
        "player should be protected by killing the mob, got HP {}",
        hp.current
    );
    assert!(world.get::<Mob>(mob).is_none(), "mob should be dead");
}

#[test]
fn mob_without_contact_cooldown_deals_no_damage() {
    let mut world = World::new();
    let player = spawn_player_at(&mut world, Vec2::ZERO);
    let bare = world
        .spawn((
            Mob,
            Position(Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0)),
            Velocity::default(),
            Health::full(MOB_MAX_HP),
        ))
        .id();

    tick(&mut world, &[], DT);

    let hp = world.get::<Health>(player).unwrap();
    assert!(
        (hp.current - PLAYER_MAX_HP).abs() < 0.01,
        "mob without ContactCooldown should deal no damage"
    );
    assert!(world.get::<Mob>(bare).is_some());
}

#[test]
fn player_without_health_takes_no_damage_and_no_panic() {
    let mut world = World::new();
    let bare_player = world
        .spawn((
            Player,
            Position(Vec2::ZERO),
            Velocity::default(),
            FireCooldown::default(),
        ))
        .id();
    spawn_mob_at(&mut world, Vec2::new(MOB_RADIUS + TANK_RADIUS - 1.0, 0.0));

    tick(&mut world, &[], DT);

    assert!(world.get::<Player>(bare_player).is_some());
}
