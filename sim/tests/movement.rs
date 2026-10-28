use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;
const TICKS_PER_SECOND: u32 = DEFAULT_TICK_RATE as u32;

fn spawn_player(world: &mut World) -> Entity {
    world
        .spawn((
            Player,
            Position::default(),
            Velocity::default(),
            FireCooldown::default(),
        ))
        .id()
}

#[test]
fn player_moves_right_when_told_to() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

    for _ in 0..TICKS_PER_SECOND {
        tick(
            &mut world,
            &[(player, InputCommand::Move { direction: Vec2::X })],
            DT,
        );
    }

    let pos = world.get::<Position>(player).unwrap().0;
    assert!(pos.x > 0.0, "should have moved right, got {pos:?}");
    assert!(
        pos.x <= TANK_SPEED + 1.0,
        "should not exceed {TANK_SPEED} units in 1s, got {pos:?}"
    );
    assert!(pos.y.abs() < 0.01, "should not move on y: {pos:?}");
}

#[test]
fn diagonal_is_not_faster_than_cardinal() {
    let cardinal = {
        let mut world = World::new();
        let player = spawn_player(&mut world);
        for _ in 0..TICKS_PER_SECOND {
            tick(
                &mut world,
                &[(player, InputCommand::Move { direction: Vec2::X })],
                DT,
            );
        }
        world.get::<Position>(player).unwrap().0.length()
    };

    let diagonal = {
        let mut world = World::new();
        let player = spawn_player(&mut world);
        for _ in 0..TICKS_PER_SECOND {
            tick(
                &mut world,
                &[(
                    player,
                    InputCommand::Move {
                        direction: Vec2::X + Vec2::Y,
                    },
                )],
                DT,
            );
        }
        world.get::<Position>(player).unwrap().0.length()
    };

    assert!(
        (diagonal - cardinal).abs() < 0.1,
        "diagonal ({diagonal}) should equal cardinal ({cardinal})"
    );
}

#[test]
fn zero_input_does_not_nan() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

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

    let pos = world.get::<Position>(player).unwrap().0;
    assert!(pos.is_finite(), "position must be finite, got {pos:?}");
    assert_eq!(pos, Vec2::ZERO, "should not move on zero input");
}

#[test]
fn missing_components_do_not_panic() {
    let mut world = World::new();
    let bare = world.spawn(Player).id();

    tick(
        &mut world,
        &[(bare, InputCommand::Move { direction: Vec2::X })],
        DT,
    );
}

#[test]
fn velocity_persists_when_input_is_omitted() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

    tick(
        &mut world,
        &[(player, InputCommand::Move { direction: Vec2::X })],
        DT,
    );
    tick(&mut world, &[], DT);

    let vel = world.get::<Velocity>(player).unwrap().0;
    assert!(
        vel.x > 0.0,
        "velocity should persist when entity is omitted from inputs, got {vel:?}"
    );
}

#[test]
fn multiple_entities_move_independently() {
    let mut world = World::new();
    let a = spawn_player(&mut world);
    let b = spawn_player(&mut world);

    for _ in 0..TICKS_PER_SECOND {
        tick(
            &mut world,
            &[(a, InputCommand::Move { direction: Vec2::X })],
            DT,
        );
    }

    let pos_a = world.get::<Position>(a).unwrap().0;
    let pos_b = world.get::<Position>(b).unwrap().0;
    assert!(pos_a.x > 0.0, "a should have moved: {pos_a:?}");
    assert!(pos_b == Vec2::ZERO, "b had no input, should not move");
}

#[test]
fn velocity_reaches_max_after_ramp_up() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

    let ticks_to_max = (TANK_SPEED / (TANK_ACCELERATION * DT)).ceil() as usize;
    for _ in 0..ticks_to_max {
        tick(
            &mut world,
            &[(player, InputCommand::Move { direction: Vec2::X })],
            DT,
        );
    }

    let vel = world.get::<Velocity>(player).unwrap().0;
    assert!(
        (vel.x - TANK_SPEED).abs() < 1.0,
        "velocity should be at max after {ticks_to_max} ticks, got {vel:?}"
    );
}

#[test]
fn tank_coasts_to_stop_without_input() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

    for _ in 0..TICKS_PER_SECOND {
        tick(
            &mut world,
            &[(player, InputCommand::Move { direction: Vec2::X })],
            DT,
        );
    }

    for _ in 0..TICKS_PER_SECOND {
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

    let vel = world.get::<Velocity>(player).unwrap().0;
    assert!(vel.length() < 0.1, "should be stopped, got {vel:?}");
}
