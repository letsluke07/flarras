//! # Movement and velocity tests
//!
//! Every invariant `sim` must uphold about tank movement: input produces
//! motion, diagonals aren't faster, acceleration ramps up, deceleration
//! coasts down, malformed entities don't crash, and multiple entities move
//! independently.
//!
//! ## Constants derived from `sim`
//!
//! `DT` and `TICKS_PER_SECOND` are both computed from `sim`'s public
//! constants. This means every test that ticks "for one second" means
//! *exactly* one second of simulated time, regardless of the tick rate.
//! If the rate ever changes from 64 Hz, all these tests adapt — and any
//! test that *needs* to break (because a tuning constant changed a
//! behaviour) will fail loudly rather than silently drift.
//!
//! ## The `spawn_player` helper
//!
//! Every test uses the same "what is a player" definition: `Player` +
//! `Position` + `Velocity` + `FireCooldown`. If a new component is added
//! to players (Health, Level, Inventory in Phase 3+), one helper changes,
//! every test adapts. Tests that inline-spawn entities would need per-site
//! edits and drift over time.
//!
//! One test deliberately does *not* use the helper
//! (`missing_components_do_not_panic`) — the whole point is an
//! *incomplete* entity.

use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;
const TICKS_PER_SECOND: u32 = DEFAULT_TICK_RATE as u32;

/// Spawn a fully-formed player entity with the standard component set.
///
/// The one place "what is a player" is defined for tests. Keep this
/// consistent with what `client::setup` actually spawns — if they ever
/// diverge, tests would pass against a shape the client doesn't produce,
/// which defeats the point.
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

// ============================================================================
// Happy path — input produces motion
// ============================================================================

/// Holding right for one second moves the tank rightward, at a rate
/// bounded by `TANK_SPEED`.
///
/// The fundamental "does input do anything" test. Asserts **invariants**,
/// not exact values, so it survives `TANK_SPEED` and `TANK_ACCELERATION`
/// tuning:
///
/// - `pos.x > 0` → moved right (not zero, not wrong sign).
/// - `pos.x <= TANK_SPEED + 1.0` → didn't exceed the speed cap over 1s.
///   The `+ 1.0` accounts for float accumulation across 64 ticks.
/// - `pos.y ≈ 0` → didn't drift off-axis.
///
/// **Why invariants and not `pos.x ≈ TANK_SPEED`:** with acceleration,
/// the tank spends part of the second ramping up, so it travels *less*
/// than `TANK_SPEED`. An exact assertion would tie the test to the
/// current `TANK_ACCELERATION` value. The invariant survives tuning; the
/// exact value doesn't.
///
/// **What breaks it:**
/// - Phase 2 input handling broken → `pos.x == 0`.
/// - Velocity integration wrong → `pos.x` wrong by a factor.
/// - `TANK_SPEED` treated as per-tick instead of per-second → `pos.x ≈
///   TANK_SPEED × 64`, wildly fails the upper bound.
/// - Movement leaks into y → fails the y assertion.
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

// ============================================================================
// Normalization — diagonals aren't faster
// ============================================================================

/// Diagonal movement is not faster than cardinal movement.
///
/// Without normalization, `dir = Vec2::X + Vec2::Y` has magnitude √2 ≈
/// 1.414, so diagonal speed would be 41% higher than cardinal. In an
/// `.io` game, that's an exploit and it feels wrong. This test pins the
/// normalization invariant.
///
/// **Why the test structure compares two runs, not against `TANK_SPEED`:**
/// an absolute assertion ("distance ≈ 150") would break if `TANK_SPEED`
/// or `TANK_ACCELERATION` changed. The *property* under test is relative
/// — "diagonal distance equals cardinal distance" — so the test runs both
/// scenarios and compares them. Rate-independent, tuning-independent.
///
/// If `normalize_or_zero` were removed from `apply_inputs`, diagonal
/// distance would exceed cardinal by ~41%, far exceeding the 0.1
/// tolerance.
///
/// **What breaks it:**
/// - `normalize_or_zero` removed from `apply_inputs` → diagonal 41% faster.
/// - Scaling applied per-axis rather than to the normalized vector → same.
/// - `TANK_SPEED` applied per-axis → same.
#[test]
fn diagonal_is_not_faster_than_cardinal() {
    // Run A: cardinal (right only).
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

    // Run B: diagonal (right + up).
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

    // Both runs travel the same distance if normalization is correct.
    assert!(
        (diagonal - cardinal).abs() < 0.1,
        "diagonal ({diagonal}) should equal cardinal ({cardinal})"
    );
}

// ============================================================================
// Safety — zero and missing components
// ============================================================================

/// Zero input produces zero motion, without NaN contamination.
///
/// The `normalize_or_zero` guard. `Vec2::ZERO.normalize()` produces
/// `Vec2::NAN` (division by zero), and NaN propagates permanently — a
/// tank whose position is NaN vanishes and can never be recovered.
/// `normalize_or_zero` returns `Vec2::ZERO` for zero input instead, which
/// is correct and safe.
///
/// **One tick is sufficient.** The bug (if it exists) fires on the first
/// tick. Running more ticks wouldn't catch anything new — either the
/// first tick corrupts the position, or it doesn't.
///
/// **What breaks it:**
/// - `normalize_or_zero` → `normalize()` → `pos` becomes NaN.
/// - Zero direction produces non-zero velocity somehow → `pos != ZERO`.
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

/// Malformed entities (Player with no Position/Velocity) don't panic.
///
/// The `if let Some(...)` guards in `apply_inputs` and phase 2 are
/// deliberate, not accidental. When the server (Phase 9) sends entity IDs
/// over the network, an entity might be mid-spawn or mid-despawn — an
/// entity with only *some* components. The sim must tolerate this.
///
/// **No assertions.** The test is "does not panic" — Rust's test harness
/// treats a completed function as a pass, and a panic as a fail. Adding
/// assertions here would be noise; the test's purpose is the *absence* of
/// a crash.
///
/// **The bare spawn is deliberate.** `world.spawn(Player)` — only the
/// marker, no Position, no Velocity, no FireCooldown. This is the
/// *minimal* malformed case. If the guards are ever `.unwrap()`ed, this
/// test panics immediately.
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

// ============================================================================
// Contract documentation — behaviour that will change
// ============================================================================

/// An entity omitted from `inputs` keeps its velocity (no deceleration).
///
/// **This test pins *current* behaviour, and it will need to change when
/// deceleration is added.** Right now, `apply_inputs` only touches
/// entities that appear in `inputs`. An entity not in `inputs` has its
/// `Velocity` untouched — it drifts forever.
///
/// That's a contract that the *client* must uphold: `read_input` always
/// pushes the local player, even with zero direction, so the player is
/// never missing from `inputs`. If `read_input` ever stopped doing that,
/// this test's opposite (a "tank drifts forever when you release keys"
/// bug) would become the production behaviour.
///
/// **When deceleration is added** (a default "decelerate all players
/// before applying inputs" pass), this test should be replaced with
/// `tank_coasts_to_stop_without_input` — the better contract. The test
/// changing is the signal that the behaviour improved.
///
/// **What breaks it:**
/// - Adding deceleration without updating this test → fails, correctly.
/// - Skipping zero-direction inputs entirely → velocity stays, this test
///   passes but the *client* breaks. (The paired
///   `tank_coasts_to_stop_without_input` catches the client-visible
///   failure.)
#[test]
fn velocity_persists_when_input_is_omitted() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

    // Tick 1: input sets velocity.
    tick(
        &mut world,
        &[(player, InputCommand::Move { direction: Vec2::X })],
        DT,
    );
    // Tick 2: no input for this player.
    tick(&mut world, &[], DT);

    let vel = world.get::<Velocity>(player).unwrap().0;
    assert!(
        vel.x > 0.0,
        "velocity should persist when entity is omitted from inputs, got {vel:?}"
    );
}

// ============================================================================
// Multi-entity — no cross-contamination
// ============================================================================

/// Input for entity A doesn't affect entity B.
///
/// The sim half of "two players don't interfere." In single-player, this
/// is trivially true; in Phase 9 multiplayer, it's essential. Testing now
/// means the invariant is locked in before multiplayer adds variables.
///
/// **What breaks it:**
/// - Input applied to all entities with `Velocity` (over-application) →
///   `pos_b` moves.
/// - Shared velocity state (resource instead of component) → both move.
/// - `spawn_player` produces duplicate entity IDs → one entity can't be
///   found.
#[test]
fn multiple_entities_move_independently() {
    let mut world = World::new();
    let a = spawn_player(&mut world);
    let b = spawn_player(&mut world);

    // Only `a` gets input.
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

// ============================================================================
// Acceleration feel — ramp up and coast down
// ============================================================================

/// Sustained input reaches `TANK_SPEED` after the expected ramp-up duration.
///
/// **The tick count is derived from constants:**
/// `ceil(TANK_SPEED / (TANK_ACCELERATION * DT))` — how many ticks of
/// acceleration it takes to reach max speed. Change any of the three
/// constants, this test adapts.
///
/// **The `+ ceil` matters:** `TANK_SPEED / (TANK_ACCELERATION * DT)` might
/// be fractional (e.g., 9.375 ticks). The `ceil` guarantees we run enough
/// ticks to actually *reach* the target — running 9 instead of 10 when
/// it takes 9.375 would fail (velocity at 96% of max).
///
/// **What breaks it:**
/// - `approach` removed → velocity snaps instantly (test passes but
///   `tank_coasts_to_stop_without_input` fails).
/// - `TANK_ACCELERATION` used as a target instead of a rate → velocity
///   reaches the wrong value.
/// - Ramp-up bug (e.g., accumulating instead of approaching) → velocity
///   diverges or stalls short of max.
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

/// Releasing input coasts the tank to a stop.
///
/// **This is the test that proves `Velocity` is load-bearing.** Under the
/// old direct-position-mutation code, this test would be meaningless —
/// velocity was always zero, so "coasts to stop" would be trivially true.
/// With velocity-based movement, the tank's velocity must decay toward
/// zero over the ramp-down duration, and *that* is what the test pins.
///
/// **One full second of deceleration is generous.** At `TANK_ACCELERATION
/// = 1000` and `TANK_SPEED = 150`, decel takes `150/1000 = 0.15s` — well
/// within one second. The one-second window gives plenty of margin.
///
/// **What breaks it:**
/// - `approach` on the Move handler broken (velocity doesn't decay) →
///   `vel.length() > 0.1`.
/// - Zero-direction `Move` set to a non-zero target → same.
/// - `apply_inputs` skips zero-direction inputs (a "performance
///   optimization" someone might try) → velocity never decrements.
#[test]
fn tank_coasts_to_stop_without_input() {
    let mut world = World::new();
    let player = spawn_player(&mut world);

    // Accelerate for one second.
    for _ in 0..TICKS_PER_SECOND {
        tick(
            &mut world,
            &[(player, InputCommand::Move { direction: Vec2::X })],
            DT,
        );
    }

    // Release input for one second.
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
