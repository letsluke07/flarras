use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;
const TICKS_PER_SECOND: u32 = DEFAULT_TICK_RATE as u32;

fn test_world(seed: u64) -> World {
    let mut world = World::new();
    world.insert_resource(SimRng::seed(seed));
    world.insert_resource(MobSpawnTimer::default());
    world.insert_resource(GameState::Playing);
    world
}

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

fn mob_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<Mob>>()
        .iter(world)
        .count()
}

fn mob_positions(world: &mut World) -> Vec<Vec2> {
    let mut positions: Vec<Vec2> = world
        .query_filtered::<&Position, With<Mob>>()
        .iter(world)
        .map(|p| p.0)
        .collect();
    positions.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .unwrap()
            .then(a.y.partial_cmp(&b.y).unwrap())
    });
    positions
}

fn run(world: &mut World, ticks: u32) {
    for _ in 0..ticks {
        tick(world, &[], DT);
    }
}

#[test]
fn spawn_timer_starts_at_interval() {
    let timer = MobSpawnTimer::default();
    assert!((timer.0 - MOB_SPAWN_INTERVAL).abs() < 0.001);
}

#[test]
fn mob_spawns_after_interval() {
    let mut world = test_world(42);
    spawn_player_at(&mut world, Vec2::ZERO);

    let ticks_until_spawn = (MOB_SPAWN_INTERVAL / DT).ceil() as u32;

    run(&mut world, ticks_until_spawn - 1);
    assert_eq!(mob_count(&mut world), 0, "no spawn before interval");

    run(&mut world, 1);
    assert_eq!(mob_count(&mut world), 1, "should spawn after interval");
}

#[test]
fn spawn_timer_resets_after_spawn() {
    let mut world = test_world(42);
    spawn_player_at(&mut world, Vec2::ZERO);

    let ticks = (MOB_SPAWN_INTERVAL / DT).ceil() as u32;
    run(&mut world, ticks);
    assert_eq!(mob_count(&mut world), 1);

    let remaining = world.resource::<MobSpawnTimer>().0;
    assert!(
        (remaining - MOB_SPAWN_INTERVAL).abs() < 0.1,
        "timer should be near {MOB_SPAWN_INTERVAL}, got {remaining}"
    );
}

#[test]
fn spawn_at_correct_distance_from_player() {
    let mut world = test_world(42);
    let player_pos = Vec2::new(100.0, 50.0);
    spawn_player_at(&mut world, player_pos);

    let ticks = (MOB_SPAWN_INTERVAL / DT).ceil() as u32;
    run(&mut world, ticks);

    let mob_pos = world
        .query_filtered::<&Position, With<Mob>>()
        .iter(&world)
        .next()
        .map(|p| p.0)
        .expect("mob should have spawned");

    let distance = (mob_pos - player_pos).length();
    assert!(
        (distance - MOB_SPAWN_DISTANCE).abs() < 1.0,
        "expected distance ≈{MOB_SPAWN_DISTANCE}, got {distance}"
    );
}

#[test]
fn no_player_means_no_spawn() {
    let mut world = test_world(42);
    run(&mut world, TICKS_PER_SECOND * 5);
    assert_eq!(mob_count(&mut world), 0);
}

#[test]
fn cap_prevents_spawn() {
    let mut world = test_world(42);
    spawn_player_at(&mut world, Vec2::ZERO);
    for _ in 0..MAX_MOBS {
        spawn_mob(&mut world, Vec2::new(1e6, 0.0));
    }
    assert_eq!(mob_count(&mut world), MAX_MOBS);

    run(&mut world, TICKS_PER_SECOND * 5);
    assert_eq!(mob_count(&mut world), MAX_MOBS, "cap should hold");
}

#[test]
fn missing_timer_means_no_spawn() {
    let mut world = World::new();
    world.insert_resource(SimRng::seed(42));
    world.insert_resource(GameState::Playing);
    spawn_player_at(&mut world, Vec2::ZERO);

    run(&mut world, TICKS_PER_SECOND * 5);
    assert_eq!(mob_count(&mut world), 0);
}

#[test]
fn missing_rng_means_no_spawn() {
    let mut world = World::new();
    world.insert_resource(MobSpawnTimer::default());
    world.insert_resource(GameState::Playing);
    spawn_player_at(&mut world, Vec2::ZERO);

    run(&mut world, TICKS_PER_SECOND * 5);
    assert_eq!(mob_count(&mut world), 0);
}

#[test]
fn dead_state_prevents_spawn() {
    let mut world = test_world(42);
    world.insert_resource(GameState::Dead);
    spawn_player_at(&mut world, Vec2::ZERO);

    run(&mut world, TICKS_PER_SECOND * 5);
    assert_eq!(mob_count(&mut world), 0);
}

#[test]
fn same_seed_produces_same_spawns() {
    let mut world1 = test_world(42);
    let mut world2 = test_world(42);
    spawn_player_at(&mut world1, Vec2::ZERO);
    spawn_player_at(&mut world2, Vec2::ZERO);

    run(&mut world1, TICKS_PER_SECOND * 3);
    run(&mut world2, TICKS_PER_SECOND * 3);

    let pos1 = mob_positions(&mut world1);
    let pos2 = mob_positions(&mut world2);

    assert_eq!(pos1.len(), pos2.len(), "same number of mobs");
    for (p1, p2) in pos1.iter().zip(pos2.iter()) {
        assert!(
            (p1 - p2).length() < 0.001,
            "positions should match: {p1:?} vs {p2:?}"
        );
    }
}

#[test]
fn different_seeds_produce_different_spawns() {
    let mut world1 = test_world(42);
    let mut world2 = test_world(43);
    spawn_player_at(&mut world1, Vec2::ZERO);
    spawn_player_at(&mut world2, Vec2::ZERO);

    run(&mut world1, TICKS_PER_SECOND * 3);
    run(&mut world2, TICKS_PER_SECOND * 3);

    let pos1 = mob_positions(&mut world1);
    let pos2 = mob_positions(&mut world2);

    assert!(!pos1.is_empty(), "should have spawned some mobs");
    assert_eq!(pos1.len(), pos2.len(), "same number of spawns");

    let any_different = pos1
        .iter()
        .zip(pos2.iter())
        .any(|(p1, p2)| (p1 - p2).length() > 1.0);
    assert!(
        any_different,
        "different seeds should produce different positions"
    );
}
