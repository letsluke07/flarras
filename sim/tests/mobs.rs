use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;
const TICKS_PER_SECOND: u32 = DEFAULT_TICK_RATE as u32;

fn spawn_player(world: &mut World, position: Vec2) -> Entity {
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

#[test]
fn spawn_mob_creates_expected_components() {
    let mut world = World::new();
    let mob = spawn_mob(&mut world, Vec2::new(50.0, 75.0));

    assert!(world.get::<Mob>(mob).is_some(), "should have Mob marker");

    let pos = world.get::<Position>(mob).expect("should have Position").0;
    assert_eq!(pos, Vec2::new(50.0, 75.0));

    let vel = world.get::<Velocity>(mob).expect("should have Velocity").0;
    assert_eq!(vel, Vec2::ZERO, "mob should spawn stationary");

    let hp = world.get::<Health>(mob).expect("should have Health");
    assert!((hp.current - MOB_MAX_HP).abs() < 0.01);
    assert!((hp.max - MOB_MAX_HP).abs() < 0.01);

    assert!(
        world.get::<ContactCooldown>(mob).is_some(),
        "should have ContactCooldown"
    );
}

#[test]
fn mob_moves_toward_player() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let mob = spawn_mob(&mut world, Vec2::new(300.0, 0.0));

    tick(&mut world, &[], DT);

    let vel = world.get::<Velocity>(mob).unwrap().0;
    assert!(
        vel.x < 0.0,
        "mob should move left toward player, got {vel:?}"
    );

    let speed = vel.length();
    assert!(speed > 0.0, "mob should be moving, got {speed}");
    assert!(
        speed <= MOB_SPEED + 0.01,
        "mob speed should not exceed {MOB_SPEED}, got {speed}"
    );
}

#[test]
fn mob_position_updates_toward_player() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let mob = spawn_mob(&mut world, Vec2::new(300.0, 0.0));

    for _ in 0..TICKS_PER_SECOND {
        tick(&mut world, &[], DT);
    }

    let pos = world.get::<Position>(mob).unwrap().0;
    assert!(pos.x < 300.0, "mob should have moved left, got {pos:?}");
    assert!(
        pos.x > 0.0,
        "mob should not have passed the player, got {pos:?}"
    );
}

#[test]
fn mob_stops_at_contact_range() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let contact_range = MOB_RADIUS + TANK_RADIUS;
    let mob = spawn_mob(&mut world, Vec2::new(contact_range - 1.0, 0.0));

    tick(&mut world, &[], DT);
    let vel = world.get::<Velocity>(mob).unwrap().0;
    assert_eq!(vel, Vec2::ZERO, "mob inside contact range should not move");

    for _ in 0..10 {
        tick(&mut world, &[], DT);
    }
    let pos = world.get::<Position>(mob).unwrap().0;
    assert!(
        (pos.x - (contact_range - 1.0)).abs() < 0.01,
        "mob should stay put, got {pos:?}"
    );
}

#[test]
fn mob_stops_when_reaching_contact_range() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let contact_range = MOB_RADIUS + TANK_RADIUS;
    let mob = spawn_mob(&mut world, Vec2::new(300.0, 0.0));

    let max_ticks = TICKS_PER_SECOND * 10;
    for _ in 0..max_ticks {
        tick(&mut world, &[], DT);
    }

    let pos = world.get::<Position>(mob).unwrap().0;
    let decel_distance = MOB_SPEED * MOB_SPEED / (2.0 * MOB_ACCELERATION);

    assert!(
        pos.x <= contact_range + 1.0,
        "mob should not stop before reaching contact, got {pos:?}"
    );
    assert!(
        pos.x >= contact_range - decel_distance - 1.0,
        "mob overshoot should be ≤ {decel_distance}, got {pos:?}"
    );

    let vel = world.get::<Velocity>(mob).unwrap().0;
    assert!(
        vel.length() < 1.0,
        "mob should have stopped at contact, got {vel:?}"
    );
}

#[test]
fn mob_without_player_does_not_move() {
    let mut world = World::new();
    let mob = spawn_mob(&mut world, Vec2::new(100.0, 100.0));

    for _ in 0..TICKS_PER_SECOND {
        tick(&mut world, &[], DT);
    }

    let pos = world.get::<Position>(mob).unwrap().0;
    assert_eq!(
        pos,
        Vec2::new(100.0, 100.0),
        "mob without target should not move"
    );
}

#[test]
fn mob_targets_nearest_player() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::new(100.0, 0.0));
    spawn_player(&mut world, Vec2::new(-100.0, 0.0));
    let mob = spawn_mob(&mut world, Vec2::new(10.0, 0.0));

    tick(&mut world, &[], DT);

    let vel = world.get::<Velocity>(mob).unwrap().0;
    assert!(
        vel.x > 0.0,
        "mob should move toward the nearer (right) player, got {vel:?}"
    );
}

#[test]
fn multiple_mobs_move_independently() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let m1 = spawn_mob(&mut world, Vec2::new(100.0, 100.0));
    let m2 = spawn_mob(&mut world, Vec2::new(-100.0, -100.0));

    for _ in 0..TICKS_PER_SECOND / 2 {
        tick(&mut world, &[], DT);
    }

    let p1 = world.get::<Position>(m1).unwrap().0;
    let p2 = world.get::<Position>(m2).unwrap().0;
    assert!(
        p1.x < 100.0 && p1.y < 100.0,
        "m1 should move toward origin, got {p1:?}"
    );
    assert!(
        p2.x > -100.0 && p2.y > -100.0,
        "m2 should move toward origin, got {p2:?}"
    );
}

#[test]
fn mob_without_health_still_gets_ai_velocity() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let bare = world
        .spawn((Mob, Position(Vec2::new(300.0, 0.0)), Velocity::default()))
        .id();

    tick(&mut world, &[], DT);

    let vel = world.get::<Velocity>(bare).unwrap().0;
    assert!(
        vel.x < 0.0,
        "mob AI should work even without Health, got {vel:?}"
    );
}

#[test]
fn mob_velocity_reaches_max_after_ramp_up() {
    let mut world = World::new();
    spawn_player(&mut world, Vec2::ZERO);
    let mob = spawn_mob(&mut world, Vec2::new(1000.0, 0.0));

    let ticks_to_max = (MOB_SPEED / (MOB_ACCELERATION * DT)).ceil() as usize;
    for _ in 0..ticks_to_max {
        tick(&mut world, &[], DT);
    }

    let vel = world.get::<Velocity>(mob).unwrap().0;
    assert!(
        (vel.length() - MOB_SPEED).abs() < 1.0,
        "mob velocity should be at max after {ticks_to_max} ticks, got {vel:?}"
    );
}
