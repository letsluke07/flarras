use sim::*;

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

#[test]
fn long_frame_is_clamped() {
    let mut clock = SimClock::default();
    let ticks = clock.consume(10.0);
    assert_eq!(
        ticks, DEFAULT_MAX_TICKS_PER_FRAME,
        "should clamp to max ticks, got {ticks}"
    );
}

#[test]
fn zero_frame_does_nothing() {
    let mut clock = SimClock::default();
    assert_eq!(clock.consume(0.0), 0);
}

#[test]
fn negative_frame_does_nothing() {
    let mut clock = SimClock::default();
    assert_eq!(
        clock.consume(-1.0),
        0,
        "negative frame time must not corrupt state"
    );
}

#[test]
fn step_is_exposed() {
    let clock = SimClock::default();
    assert!((clock.step() - DEFAULT_SIM_DT).abs() < 1e-9);
}

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

#[test]
fn clamped_frame_recovers_gradually() {
    let mut clock = SimClock::default();
    let _ = clock.consume(10.0);
    let next = clock.consume(DEFAULT_SIM_DT);
    assert_eq!(next, 1, "after clamp, normal frame runs one tick");
}
