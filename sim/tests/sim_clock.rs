//! # Fixed-timestep accumulator tests
//!
//! `SimClock` decouples real frame time from sim time. Each test pins one
//! property of that decoupling: frame-to-tick conversion at various frame
//! rates, the spiral-of-death clamp, and accumulator carry-over.
//!
//! ## No `World` needed
//!
//! These tests never construct an ECS world. `SimClock` is a pure data
//! structure with a single method (`consume`). The tests instantiate it
//! directly and check its outputs. That's why this file is the fastest in
//! the suite — no archetype setup, no entity spawns, just arithmetic.
//!
//! ## Every value derived from constants
//!
//! `DEFAULT_TICK_RATE`, `DEFAULT_SIM_DT`, `DEFAULT_MAX_TICKS_PER_FRAME`
//! appear everywhere. No hardcoded 60s, 64s, or 32s. If the tick rate
//! changes, the whole file adapts. (Contrast: the *early* version of this
//! file had `64` in a for-loop and `59..=60` in an assertion; both were
//! bugs waiting for a rate change to expose them.)

use sim::*;

// ============================================================================
// Basic frame-to-tick conversion
// ============================================================================

/// N frames of exactly one step each produce N ticks.
///
/// Technically tautological — "one step of the clock produces one tick"
/// follows from the definition. But it's a useful regression test: if
/// `consume`'s `while` loop is ever broken (e.g., `>` instead of `>=` in
/// the comparison), this test catches it immediately.
///
/// **Loop count = `DEFAULT_TICK_RATE`** so the test means "one second of
/// one-step frames produces one second of ticks." Fixed at exactly one
/// second so the assertion is meaningful, not a magic count.
///
/// **What breaks it:**
/// - `consume` drops a tick (off-by-one in the loop) → total 63 instead
///   of 64.
/// - Accumulator reset each call (no carry-over) → total 0.
#[test]
fn one_step_per_frame_yields_one_tick_each() {
    let mut clock = SimClock::default();
    let mut total = 0;
    for _ in 0..DEFAULT_TICK_RATE as usize {
        total += clock.consume(DEFAULT_SIM_DT);
    }
    let expected = DEFAULT_TICK_RATE as u32;
    assert_eq!(
        total, expected,
        "{expected} frames of one step each = {expected} ticks"
    );
}

/// N frames of two steps each produce 2N ticks (same total per second).
///
/// Tests that "how long the frame is" doesn't matter — only "how much
/// time the frame represents." At 30 fps (2 steps per frame), one second
/// of real time still produces `DEFAULT_TICK_RATE` ticks.
///
/// **What breaks it:**
/// - `consume` only returns 0 or 1 regardless of accumulated time → total
///   ≈ half of expected.
/// - Accumulator doesn't carry over → fractional-time loss compounds.
#[test]
fn two_steps_per_frame_yields_two_ticks_each() {
    let mut clock = SimClock::default();
    let mut total = 0;
    let half_second = DEFAULT_TICK_RATE as usize / 2;
    for _ in 0..half_second {
        total += clock.consume(2.0 * DEFAULT_SIM_DT);
    }
    let expected = DEFAULT_TICK_RATE as u32;
    assert_eq!(
        total, expected,
        "{half_second} frames of two steps each = {expected} ticks"
    );
}

/// High frame rates accumulate fractional ticks correctly across many
/// frames.
///
/// At 144 fps with a 64 Hz sim, most frames get 0 ticks and a few get 1.
/// The accumulator must retain the fractional time between frames or the
/// sim would drift behind real time.
///
/// **The range `expected - 1 ..= expected` reflects float accumulation.**
/// 144 frames × `1.0/144.0` sums to ~0.9999... in `f32`, not exactly
/// 1.0. So the tick count might be 63 or 64 depending on rounding. Either
/// is *correct* — the point is that a whole second of real time produces
/// about a whole second of sim time.
///
/// **What breaks it:**
/// - Accumulator reset each call → most frames produce 0 ticks, total
///   drops well below expected.
/// - Integer division instead of accumulation → same.
/// - Rounding-up behavior (always round up fractional) → total exceeds
///   expected by a meaningful margin.
#[test]
fn one_second_of_high_fps_frames_produces_tick_rate_ticks() {
    let mut clock = SimClock::default();
    let mut total = 0;
    for _ in 0..144 {
        total += clock.consume(1.0 / 144.0);
    }
    let expected = DEFAULT_TICK_RATE as u32;
    assert!(
        (expected - 1..=expected).contains(&total),
        "144 frames at 144fps ≈ {expected} ticks, got {total}"
    );
}

// ============================================================================
// Safety — the spiral-of-death guard
// ============================================================================

/// A single very long frame is clamped to `DEFAULT_MAX_TICKS_PER_FRAME`
/// ticks, no more.
///
/// The scenario: the tab was backgrounded for 10 seconds, or a GC pause
/// stalled the JS event loop. Without a clamp, `consume` would return
/// 10 / (1/64) = 640 ticks, and running 640 sim ticks in one frame would
/// freeze the browser.
///
/// **What breaks it:**
/// - Clamp removed → returns 640, test fails.
/// - Clamp applied to tick count but not accumulated time → subsequent
///   frames would each drain a huge backlog (this test alone doesn't
///   catch that, but `clamped_frame_recovers_gradually` does).
#[test]
fn long_frame_is_clamped() {
    let mut clock = SimClock::default();
    let ticks = clock.consume(10.0);
    assert_eq!(
        ticks, DEFAULT_MAX_TICKS_PER_FRAME,
        "should clamp to max ticks, got {ticks}"
    );
}

/// A zero-length frame produces no ticks and doesn't corrupt state.
///
/// Trivial case, but worth pinning: `consume(0.0)` on a fresh clock
/// returns 0, and the clock is still usable afterward.
///
/// **What breaks it:**
/// - `while accumulator > 0.0` instead of `>= step` (a plausible typo) →
///   infinite loop, test hangs (which counts as a failure).
/// - Off-by-one in the accumulator reset → returns 1 erroneously.
#[test]
fn zero_frame_does_nothing() {
    let mut clock = SimClock::default();
    assert_eq!(clock.consume(0.0), 0);
}

/// A negative frame time doesn't subtract from the accumulator.
///
/// Negative `frame_dt` shouldn't happen in practice (Time::delta_secs is
/// non-negative on all supported platforms), but defensive code beats
/// surprising failures. Without the `.max(0.0)` guard, a negative value
/// would *reduce* the accumulator, effectively running time backwards and
/// delaying the next tick.
///
/// **What breaks it:**
/// - `.max(0.0)` guard removed → negative accumulation, returns 0 but
///   leaves the clock in a slightly wrong state. This test only asserts
///   the return value, but a follow-up positive frame would reveal the
///   corruption.
#[test]
fn negative_frame_does_nothing() {
    let mut clock = SimClock::default();
    assert_eq!(
        clock.consume(-1.0),
        0,
        "negative frame time must not corrupt state"
    );
}

// ============================================================================
// API contract
// ============================================================================

/// `step()` returns exactly `DEFAULT_SIM_DT`.
///
/// The getter is trivial but its contract isn't: the value returned here
/// is what callers pass to `sim::tick` as `dt`. If it ever drifted from
/// `DEFAULT_SIM_DT`, every sim tick would advance by the wrong amount.
///
/// **`1e-9` tolerance** because `DEFAULT_SIM_DT` is `1.0/64.0`, which is
/// exactly representable in `f32`. The tolerance is defensive — could be
/// `assert_eq!` but the epsilon costs nothing and protects against future
/// changes to `DEFAULT_SIM_DT`.
#[test]
fn step_is_exposed() {
    let clock = SimClock::default();
    assert!((clock.step() - DEFAULT_SIM_DT).abs() < 1e-9);
}

// ============================================================================
// Accumulator carry-over — the reason SimClock exists
// ============================================================================

/// Sub-step time carries across frames.
///
/// Two calls of half a step each should produce exactly one tick total —
/// not zero, not two. This is the property that makes "fractional frames"
/// work. Without carry-over, a 144 fps client would permanently get
/// zero ticks and the sim would never advance.
///
/// **First call: 0 ticks.** Half a step isn't enough for one.
/// **Second call: 1 tick.** The accumulated time is now exactly one step.
///
/// **What breaks it:**
/// - Accumulator reset each call → both calls return 0, total is 0.
/// - Rounding behavior (always round up) → first call returns 1, second
///   returns 0, total is still 1 but the timing is off. (This test
///   wouldn't catch that specific bug — but the "12 ms/frame" behaviour
///   would manifest elsewhere as visible stutter.)
#[test]
fn accumulator_carries_over_between_frames() {
    let mut clock = SimClock::default();
    assert_eq!(
        clock.consume(DEFAULT_SIM_DT * 0.5),
        0,
        "half step: no tick yet"
    );
    assert_eq!(
        clock.consume(DEFAULT_SIM_DT * 0.5),
        1,
        "second half: one tick"
    );
}

/// After a clamped frame, the next normal frame returns to normal.
///
/// **The clamp is a hard ceiling, not a debt.** When a 10-second frame is
/// clamped to 5 ticks, the extra ~9.92s of accumulated time is
/// *discarded*, not carried forward. Otherwise the next frames would
/// continue running 5 ticks each until the backlog drained — which is
/// exactly the death spiral the clamp is supposed to prevent.
///
/// This test verifies: after a clamped frame, the very next normal frame
/// runs exactly 1 tick (the normal amount), not 5 (draining backlog).
///
/// **What breaks it:**
/// - Clamp on *tick count* only, not on *accumulated time* → next frame
///   returns 5 (draining some of the discarded backlog).
/// - Clamp resets accumulator to `max_accumulated` instead of just capping
///   → next frame returns 1 but state is subtly wrong for several ticks.
#[test]
fn clamped_frame_recovers_gradually() {
    let mut clock = SimClock::default();
    let _ = clock.consume(10.0); // clamped to 5 ticks, backlog discarded
    let next = clock.consume(DEFAULT_SIM_DT);
    assert_eq!(next, 1, "after clamp, normal frame runs one tick");
}
