use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;
const TICKS_PER_SECOND: u32 = DEFAULT_TICK_RATE as u32;

fn bullet_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<Bullet>>()
        .iter(world)
        .count()
}

#[test]
fn spawn_bullet_creates_expected_components() {
    let mut world = World::new();
    let bullet = spawn_bullet(&mut world, Vec2::new(10.0, 20.0), Vec2::X);

    assert!(
        world.get::<Bullet>(bullet).is_some(),
        "should have Bullet marker"
    );

    let pos = world
        .get::<Position>(bullet)
        .expect("should have Position")
        .0;
    assert_eq!(pos, Vec2::new(10.0, 20.0));

    let vel = world
        .get::<Velocity>(bullet)
        .expect("should have Velocity")
        .0;
    assert!(
        (vel.length() - BULLET_SPEED).abs() < 0.1,
        "velocity magnitude should be {BULLET_SPEED}, got {vel:?}"
    );
    assert!(vel.x > 0.0, "should travel right, got {vel:?}");
    assert!(vel.y.abs() < 0.01);

    let life = world
        .get::<Lifetime>(bullet)
        .expect("should have Lifetime")
        .0;
    assert!((life - BULLET_LIFETIME).abs() < 1e-5);
}

#[test]
fn bullet_travels_at_bullet_speed() {
    let mut world = World::new();
    let bullet = spawn_bullet(&mut world, Vec2::ZERO, Vec2::X);

    let ticks = TICKS_PER_SECOND / 2;
    for _ in 0..ticks {
        tick(&mut world, &[], DT);
    }

    let pos = world.get::<Position>(bullet).unwrap().0;
    let expected = BULLET_SPEED * ticks as f32 * DT;
    assert!(
        (pos.x - expected).abs() < 0.5,
        "expected x ≈ {expected}, got {pos:?}"
    );
    assert!(pos.y.abs() < 0.01);
}

#[test]
fn bullet_despawns_after_lifetime() {
    let mut world = World::new();
    let bullet = spawn_bullet(&mut world, Vec2::ZERO, Vec2::X);

    let ticks_to_expire = (BULLET_LIFETIME / DT).ceil() as usize + 1;
    for _ in 0..ticks_to_expire {
        tick(&mut world, &[], DT);
    }

    assert!(
        world.get::<Bullet>(bullet).is_none(),
        "bullet should be despawned after {ticks_to_expire} ticks"
    );
}

#[test]
fn bullet_survives_just_before_lifetime() {
    let mut world = World::new();
    let bullet = spawn_bullet(&mut world, Vec2::ZERO, Vec2::X);

    let ticks_before_expiry = (BULLET_LIFETIME / DT).floor() as usize - 1;
    for _ in 0..ticks_before_expiry {
        tick(&mut world, &[], DT);
    }

    assert!(
        world.get::<Bullet>(bullet).is_some(),
        "bullet should still exist before lifetime expires"
    );
}

#[test]
fn fire_input_spawns_bullet_at_player_position() {
    let mut world = World::new();
    let player = world
        .spawn((
            Player,
            Position(Vec2::new(100.0, 50.0)),
            Velocity::default(),
            FireCooldown::default(),
        ))
        .id();

    tick(
        &mut world,
        &[(player, InputCommand::Fire { direction: Vec2::X })],
        DT,
    );

    let bullets: Vec<Entity> = world
        .query_filtered::<Entity, With<Bullet>>()
        .iter(&world)
        .collect();
    assert_eq!(bullets.len(), 1, "exactly one bullet should exist");

    let bullet_pos = world.get::<Position>(bullets[0]).unwrap().0;
    let expected_x = 100.0 + BULLET_SPEED * DT;
    assert!(
        (bullet_pos.x - expected_x).abs() < 0.5,
        "expected x ≈ {expected_x}, got {bullet_pos:?}"
    );
    assert!((bullet_pos.y - 50.0).abs() < 0.5);
}

#[test]
fn fire_without_position_does_not_panic_and_spawns_nothing() {
    let mut world = World::new();
    let bare = world.spawn((Player, FireCooldown::default())).id();

    tick(
        &mut world,
        &[(bare, InputCommand::Fire { direction: Vec2::X })],
        DT,
    );

    assert_eq!(
        bullet_count(&mut world),
        0,
        "no bullet should spawn from an entity without Position"
    );
}

#[test]
fn multiple_bullets_coexist_independently() {
    let mut world = World::new();
    let b1 = spawn_bullet(&mut world, Vec2::new(0.0, 0.0), Vec2::X);
    let b2 = spawn_bullet(&mut world, Vec2::new(0.0, 100.0), Vec2::X);

    for _ in 0..TICKS_PER_SECOND / 2 {
        tick(&mut world, &[], DT);
    }

    assert!(world.get::<Bullet>(b1).is_some(), "b1 should still exist");
    assert!(world.get::<Bullet>(b2).is_some(), "b2 should still exist");

    let pos1 = world.get::<Position>(b1).unwrap().0;
    let pos2 = world.get::<Position>(b2).unwrap().0;
    assert!(pos1.x > 100.0, "b1 should have travelled right: {pos1:?}");
    assert!(pos2.x > 100.0, "b2 should have travelled right: {pos2:?}");
    assert!(
        (pos2.y - 100.0).abs() < 0.01,
        "b2 should have stayed at y=100, got {pos2:?}"
    );
}

#[test]
fn fire_cooldown_blocks_rapid_fire() {
    let mut world = World::new();
    let player = world
        .spawn((
            Player,
            Position::default(),
            Velocity::default(),
            FireCooldown::default(),
        ))
        .id();

    tick(
        &mut world,
        &[(player, InputCommand::Fire { direction: Vec2::X })],
        DT,
    );
    assert_eq!(
        bullet_count(&mut world),
        1,
        "first fire should spawn one bullet"
    );

    tick(
        &mut world,
        &[(player, InputCommand::Fire { direction: Vec2::X })],
        DT,
    );
    assert_eq!(
        bullet_count(&mut world),
        1,
        "cooldown should block the second shot"
    );

    let cd = world.get::<FireCooldown>(player).unwrap().remaining;
    assert!(cd > 0.0, "cooldown should be active, got {cd}");
}

#[test]
fn fire_allowed_after_cooldown_expires() {
    let mut world = World::new();
    let player = world
        .spawn((
            Player,
            Position::default(),
            Velocity::default(),
            FireCooldown::default(),
        ))
        .id();

    tick(
        &mut world,
        &[(player, InputCommand::Fire { direction: Vec2::X })],
        DT,
    );

    let ticks = (FIRE_INTERVAL / DT).ceil() as u32;
    for _ in 0..ticks {
        tick(
            &mut world,
            &[(
                player,
                InputCommand::Move {
                    direction: Vec2::ZERO,
                },
            )],
            DT,
        );
    }

    let cd = world.get::<FireCooldown>(player).unwrap().remaining;
    assert_eq!(cd, 0.0, "cooldown should have expired, got {cd}");

    tick(
        &mut world,
        &[(player, InputCommand::Fire { direction: Vec2::X })],
        DT,
    );
    assert_eq!(
        bullet_count(&mut world),
        2,
        "second bullet should spawn after cooldown"
    );
}
