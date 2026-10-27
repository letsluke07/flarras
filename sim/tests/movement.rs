use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

fn step(world: &mut World) {
    build_sim_schedule().run(world);
}

#[test]
fn velocity_integrates_into_position() {
    let mut world = World::new();
    world.insert_resource(SimTime { dt: 0.5 });
    let e = world
        .spawn((Position(Vec2::ZERO), Velocity(Vec2::new(10.0, -4.0))))
        .id();

    step(&mut world);

    let got = world.get::<Position>(e).unwrap().0;
    let expected = Vec2::new(5.0, -2.0);
    assert_eq!(
        got, expected,
        "movement_system must integrate pos += vel * dt \
         (vel=(10,-4), dt=0.5); got {got:?}, expected {expected:?}"
    );
}

#[test]
fn zero_velocity_leaves_position_unchanged() {
    let mut world = World::new();
    world.insert_resource(SimTime { dt: 1.0 });
    let start = Vec2::new(3.0, 7.0);
    let e = world.spawn((Position(start), Velocity::default())).id();

    step(&mut world);

    let got = world.get::<Position>(e).unwrap().0;
    assert_eq!(
        got, start,
        "a stationary entity must not drift; movement_system added motion \
         to a zero-velocity entity; got {got:?}, expected {start:?}"
    );
}

#[test]
fn every_entity_is_updated() {
    let mut world = World::new();
    world.insert_resource(SimTime { dt: 1.0 });
    let a = world.spawn((Position(Vec2::ZERO), Velocity(Vec2::X))).id();
    let b = world.spawn((Position(Vec2::ZERO), Velocity(Vec2::Y))).id();

    step(&mut world);

    let got_a = world.get::<Position>(a).unwrap().0;
    let got_b = world.get::<Position>(b).unwrap().0;
    assert_eq!(
        got_a,
        Vec2::X,
        "entity A (vel=X, dt=1) must end at (1,0); \
         query may be filtering out entities incorrectly; \
         got {got_a:?}, expected {:?}",
        Vec2::X
    );
    assert_eq!(
        got_b,
        Vec2::Y,
        "entity B (vel=Y, dt=1) must end at (0,1); \
         query may be filtering out entities incorrectly; \
         got {got_b:?}, expected {:?}",
        Vec2::Y
    );
}
