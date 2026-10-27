//! # Bullet behaviour tests
//!
//! Every invariant `sim` must uphold about bullets: spawning, travel,
//! lifetime, despawning, Fire-triggered spawning, cooldown enforcement,
//! and multi-bullet independence.
//!
//! ## What this file does NOT test
//!
//! - **Collision** — doesn't exist yet (Phase 3).
//! - **Damage** — no `Health` component yet.
//! - **Visuals** — rendering lives in the client, tested manually in the
//!   browser.
//!
//! ## The testing pattern
//!
//! Each test constructs a bare `World`, spawns exactly the entities it
//! needs, calls `sim::tick` in a loop, and asserts on the resulting
//! components. No `App`, no window, no browser, no `Bevy` scheduler — just
//! `World` and `tick`. That's the entire payoff of the sim/client split:
//! behavioural correctness runs in under a second.
//!
//! ## Constants
//!
//! Both `DT` and `TICKS_PER_SECOND` derive from `sim`'s public constants
//! rather than hardcoded numbers. If the tick rate ever changes from 64
//! to something else, every test adapts automatically — and the tests that
//! *wouldn't* adapt are exactly the ones you want to fail loudly (they'd
//! signal a behavioural change, not a tuning change).

use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;
use sim::*;

const DT: f32 = DEFAULT_SIM_DT;
const TICKS_PER_SECOND: u32 = DEFAULT_TICK_RATE as u32;

/// Count the entities carrying the `Bullet` marker. Used in tests that
/// assert "exactly N bullets exist" without caring which they are.
///
/// Takes `&mut World` rather than `&World` because `query_filtered` needs
/// mutable access to initialize its archetype cache. This is a Bevy API
/// quirk, not a semantic requirement — the query is read-only in spirit.
fn bullet_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<Bullet>>()
        .iter(world)
        .count()
}

// ============================================================================
// Spawn contract
// ============================================================================

/// `spawn_bullet` produces an entity with exactly the four expected
/// components, each with the expected initial value.
///
/// This is the *canonical* bullet spawn path. Every bullet in the game —
/// whether spawned by a `Fire` input or a test — goes through this
/// function, so this test pins the invariant that "a bullet" means
/// precisely these components. If bullet structure ever changes (adding
/// `Damage`, `Owner`, `Piercing`, whatever), this test is the checklist of
/// what needs to keep working.
///
/// **What breaks it:**
/// - Forgetting the `Bullet` marker → first assertion fails.
/// - Not setting `Position(origin)` → `assert_eq!` fails (no epsilon
///   because origin is passed through unchanged).
/// - Not scaling velocity by `BULLET_SPEED` → magnitude check fails by 500×.
/// - Not setting `Lifetime(BULLET_LIFETIME)` → last assertion fails.
#[test]
fn spawn_bullet_creates_expected_components() {
    let mut world = World::new();
    let bullet = spawn_bullet(&mut world, Vec2::new(10.0, 20.0), Vec2::X);

    // Bullet marker — the tag that makes it queryable.
    assert!(
        world.get::<Bullet>(bullet).is_some(),
        "should have Bullet marker"
    );

    // Position — the origin passed in, unmodified.
    // Uses `assert_eq!` (no tolerance) because no arithmetic happens to
    // origin inside `spawn_bullet`; it's a pure pass-through.
    let pos = world
        .get::<Position>(bullet)
        .expect("should have Position")
        .0;
    assert_eq!(pos, Vec2::new(10.0, 20.0));

    // Velocity — magnitude = BULLET_SPEED, direction = the input direction.
    // Two checks: magnitude (0.1 tolerance for float) and per-axis
    // components (verifies direction is preserved).
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

    // Lifetime — initialized to BULLET_LIFETIME. Tolerance is 1e-5 because
    // it's an f32 pass-through; could be `assert_eq!` today but the
    // tolerance is defensive against future changes.
    let life = world
        .get::<Lifetime>(bullet)
        .expect("should have Lifetime")
        .0;
    assert!((life - BULLET_LIFETIME).abs() < 1e-5);
}

// ============================================================================
// Integration — bullets actually move
// ============================================================================

/// Bullets advance at exactly `BULLET_SPEED` units per second, integrated
/// by the same phase-2 code that moves tanks.
///
/// The bullet version of `player_moves_right_when_told_to`. If phase 2 of
/// `tick` is broken, tanks and bullets both fail; if only bullets fail,
/// the bug is in `spawn_bullet` (wrong velocity) or in the bullet-specific
/// path (none currently).
///
/// **Ticks = half a second.** Half a second gives plenty of integration
/// steps (32 at 64 Hz) without approaching `BULLET_LIFETIME` (2.0s) — the
/// bullet stays alive throughout, so we measure travel not despawn.
/// `TICKS_PER_SECOND / 2` expresses "half a second" symbolically rather
/// than as `32`, which would silently mean something different at a
/// different tick rate.
///
/// **Expected value is computed from constants.** `BULLET_SPEED × ticks ×
/// DT` is the physics formula, not a hardcoded number. Change
/// `BULLET_SPEED`, the test follows.
///
/// **What breaks it:**
/// - Phase 2 integration removed or broken → `pos.x ≈ 0`.
/// - Velocity magnitude wrong → `pos.x` off by factor.
/// - Bullet despawns mid-test → `pos.x` frozen at despawn tick.
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
    // Sanity: bullet should have stayed on its axis. Catches bugs where
    // velocity integration mixes x and y components.
    assert!(pos.y.abs() < 0.01);
}

// ============================================================================
// Lifetime — the two-sided boundary
// ============================================================================
//
// These two tests form a *pair*. One asserts the bullet despawns after
// enough ticks; the other asserts it survives until just before. Testing
// only one side would allow a bug that despawns bullets either too early
// or too late to slip through. Together they pin both edges of the
// boundary, which is where off-by-one errors live.

/// A bullet longer-lived than `BULLET_LIFETIME` ticks is gone.
///
/// **`ticks_to_expire = ceil(LIFETIME / DT) + 1`.** The `+1` guarantees
/// we're *past* the expiry tick, not landing on it. If `Lifetime` ever
/// changes from `<= 0.0` to `< 0.0`, an exact-boundary test could go
/// either way; the `+1` makes this test unambiguous regardless.
///
/// **What breaks it:**
/// - `tick_lifetimes_and_despawn` not called in `tick` → bullet survives.
/// - Lifetime not decremented → same.
/// - `world.despawn` skipped even though lifetime expired → same.
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

/// A bullet survives until *just before* its lifetime expires.
///
/// The complementary half of the boundary test. Without it, a bug that
/// despawned bullets on their spawn tick would still pass
/// `bullet_despawns_after_lifetime` — the bullet *is* gone after enough
/// ticks, so "correct" technically. This test catches that by asserting
/// the bullet still exists one tick short of expiry.
///
/// **`ticks_before_expiry = floor(LIFETIME / DT) - 1`.** The `-1` stops
/// one tick short of the ambiguous boundary (exactly at `LIFETIME`), so
/// the "still exists" assertion is unambiguous.
///
/// **What breaks it:**
/// - Bullet despawned too eagerly → fails.
/// - `Lifetime` initialized to a shorter duration → fails partway through.
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

// ============================================================================
// Fire integration — input path → spawn path
// ============================================================================

/// `InputCommand::Fire` spawns a bullet at the firing entity's position.
///
/// Tests the integration between the input handler in `sim::apply_inputs`
/// and `spawn_bullet`. Sending a Fire command should result in exactly one
/// bullet appearing at the shooter's position, moving in the given
/// direction.
///
/// **The expected position includes one integration step.** The `Fire`
/// branch of `apply_inputs` runs in phase 2, *before* phase 3 integration,
/// so the bullet is spawned at the shooter's position and then immediately
/// moved by one `dt` of velocity. The test's `expected_x = 100.0 +
/// BULLET_SPEED * DT` accounts for that. This is honest about how the tick
/// works — if `tick` were restructured so Fire spawned after integration,
/// this test would need to change, and that change would be a real
/// behavioural difference.
///
/// **What breaks it:**
/// - Fire handler doesn't call `spawn_bullet` → `bullets.len() == 0`.
/// - `spawn_bullet` called with wrong origin → `bullet_pos.x ≈ BULLET_SPEED
///   * DT ≈ 7.8`, fails by ~100.
/// - Fire spawns *two* bullets (duplicated call) → `bullets.len() == 2`.
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
    // If direction were angled incorrectly, y would drift from 50.
    assert!((bullet_pos.y - 50.0).abs() < 0.5);
}

/// Fire on an entity without `Position` is a no-op — no bullet, no panic.
///
/// Robustness test. The entity has `Player` and `FireCooldown` but no
/// `Position`, which means `spawn_bullet` has no origin to spawn from.
/// The guard in `apply_inputs` (`if let Some(origin) = origin`) silently
/// skips. No panic, no bullet.
///
/// **Why this matters for Phase 9:** the server will send entity IDs over
/// the network. A packet might arrive for an entity that's mid-spawn or
/// mid-despawn — an entity with only *some* components. The sim must
/// tolerate this. If it panicked, one bad packet would crash the whole
/// server tick.
///
/// **Why `FireCooldown::default()` is on the entity:** so this test
/// specifically exercises the *missing Position* path, not the *missing
/// cooldown* path. With `FireCooldown` present and ready, the readiness
/// check passes, and the failure-to-spawn comes from missing `Position`.
/// If both were missing, the test would pass for the wrong reason.
///
/// **What breaks it:**
/// - `.unwrap()` on `world.get::<Position>` → panic.
/// - Fallback to `Vec2::ZERO` on missing position → bullet spawned at
///   origin, `bullet_count == 1`.
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

// ============================================================================
// Multi-instance — bullets don't interfere
// ============================================================================

/// Two bullets spawned at different positions track their own state
/// independently.
///
/// The multi-entity version of `bullet_travels_at_bullet_speed`. Catches
/// bugs that only manifest with >1 bullet:
///
/// - Shared mutable state (e.g., a `Lifetime` resource instead of
///   component → both bullets despawn together).
/// - `integrate_positions` mixing components between entities.
/// - Despawn logic despawning by index instead of entity ID (would remove
///   one arbitrary bullet).
///
/// **The `pos2.y ≈ 100.0` assertion is the sharpest check.** Both bullets
/// move right, so "pos.x > 100" alone doesn't prove independence — any
/// "bullet moves right" bug preserves that. But `pos2.y` staying at its
/// spawn y-value proves that *b1's* velocity isn't contaminating *b2's*
/// position, and vice versa. That's what "independence" means in a test.
///
/// **What breaks it:**
/// - Bullets share lifetime/velocity state → one despawns early or y drifts.
/// - Despawn logic removes wrong entity → one missing.
/// - `spawn_bullet` returns duplicate entity ID → only one bullet exists.
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

// ============================================================================
// Cooldown — the two-sided boundary again
// ============================================================================
//
// Same pattern as the lifetime tests: one asserts the cooldown blocks a
// shot during cooldown, the other asserts a shot succeeds after cooldown
// expires. Together they pin both edges of the timing window.

/// A second Fire input during cooldown is rejected — no bullet, no state
/// change beyond the cooldown ticking down.
///
/// **What breaks it:**
/// - Cooldown check removed from `apply_inputs` → second bullet spawns,
///   `bullet_count == 2`.
/// - Cooldown reset to zero after the first shot → same.
/// - `FireCooldown.remaining` not set after firing → same.
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

    // Cooldown should be actively counting down. `> 0.0` rather than a
    // specific value because the exact remaining depends on how many
    // integration ticks have run since the fire.
    let cd = world.get::<FireCooldown>(player).unwrap().remaining;
    assert!(cd > 0.0, "cooldown should be active, got {cd}");
}

/// After the cooldown expires, the next Fire succeeds.
///
/// The complementary half. Without it, a bug that made cooldowns *never*
/// expire would pass `fire_cooldown_blocks_rapid_fire` — the cooldown does
/// block, so that test succeeds. This one catches the "cooldown stuck
/// forever" case.
///
/// **Ticks = `ceil(FIRE_INTERVAL / DT)`**, exactly the duration of one
/// cooldown. The intermediate `Move` inputs aren't meaningful — they just
/// give `tick` a valid input list. Using `Move` rather than an empty slice
/// exercises the `apply_inputs` path for a non-Fire command during the
/// wait, which is what a real client would send.
///
/// **What breaks it:**
/// - Cooldown doesn't decrement → `remaining` still > 0 after the wait.
/// - `FIRE_INTERVAL` set incorrectly (too large) → same.
/// - Cooldown check happens after the spawn (order bug) → shot fires but
///   cooldown doesn't get set, so this test passes for the wrong reason.
///   The paired `fire_cooldown_blocks_rapid_fire` catches that.
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

    // Wait out the cooldown without firing. `ceil()` to guarantee we've
    // passed the expiry tick.
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

    // Fire again — should succeed now.
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
