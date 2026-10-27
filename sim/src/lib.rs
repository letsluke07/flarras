//! # Flarras.io — Simulation
//!
//! The **verbs** of the game. `tick` advances the world by one fixed
//! timestep. `SimClock` converts variable real-time frame deltas into a
//! discrete number of ticks to run.
//!
//! ## The two guarantees this crate provides
//!
//! **1. Determinism.** Given the same `World` state, the same `inputs`,
//! and the same `dt`, `tick` produces bit-for-bit identical output — every
//! time, on every machine. This is what makes replays possible, what makes
//! the client's prediction agree with the server's authority in Phase 9,
//! and what makes `cargo test -p sim` meaningful at all.
//!
//! **2. Headless-ness.** No rendering, no networking, no wall clock, no
//! file I/O, no `std::time`. The sim receives `dt` as a parameter; it never
//! reads it from anywhere. When the future server runs this crate, it runs
//! the *exact same code* the client runs locally.
//!
//! ## Why `tick` is a plain function, not a Bevy plugin
//!
//! `tick` takes `&mut World` directly. It is not registered in a `Schedule`,
//! it does not go through Bevy's system parameter injection, and it does not
//! use `FixedUpdate`. Three reasons:
//!
//! 1. **Callable from anywhere.** Tests call it, the client calls it inside
//!    an exclusive system, the future server will call it in a loop. Bevy
//!    systems can only be called by Bevy's scheduler.
//!
//! 2. **Explicit ordering.** The body of `tick` *is* the execution order,
//!    readable top-to-bottom. With Bevy systems you'd need `.chain()` and
//!    `.before()` constraints and trust the scheduler to honor them. Here,
//!    the order is source code.
//!
//! 3. **No hidden state.** Everything `tick` touches comes through its
//!    `&mut World` argument. There's no `Res<Time>`, no `Res<ButtonInput>`,
//!    no implicit world access. It's a pure function of its arguments.
//!
//! ## The four phases of a tick
//!
//! Each tick runs four phases in a fixed order. The order is not arbitrary —
//! each phase depends on the previous:
//!
//! ```text
//!   1. tick_cooldowns               FireCooldown -= dt
//!   2. apply_inputs                 Move -> Velocity, Fire -> spawn bullet
//!   3. integrate_positions          Position += Velocity * dt
//!   4. tick_lifetimes_and_despawn   Lifetime -= dt, despawn at <= 0
//! ```
//!
//! See each function's doc for why it sits where it sits.

use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use shared::*;

// ============================================================================
// Timing
// ============================================================================
//
// The sim runs at a *fixed* rate — every tick advances game time by exactly
// `DEFAULT_SIM_DT` seconds. The frame rate (how often `tick` gets called in
// real time) is variable and irrelevant to the sim; `SimClock` decides how
// many ticks to run per frame so that "one second of real time" always
// produces "one second of game time," regardless of hardware.
//
// ----------------------------------------------------------------------------

/// Ticks per second of game time.
///
/// **64, not 60.** Three reasons:
///
/// 1. It's a **power of two**, so `1.0 / 64.0 = 0.015625` is *exactly*
///    representable in `f32`. `1.0 / 60.0` is a repeating decimal in binary,
///    so every tick accumulates a tiny rounding error that compounds over
///    thousands of ticks.
///
/// 2. **No aliasing with 60 Hz monitors.** If the sim ran at 60 Hz and the
///    display refreshed at 60 Hz, their clocks would beat against each other,
///    producing visible stutter (sometimes 2 ticks per frame, sometimes 0).
///    At 64 Hz the two rates drift cleanly, giving smooth tick distribution.
///
/// 3. **Matches Bevy's `FixedUpdate` default** — useful if we ever migrate
///    to Bevy's scheduling, and a sign the number is well-chosen.
pub const DEFAULT_TICK_RATE: f32 = 64.0;

/// The fixed timestep passed to every `sim::tick` call. Exactly `1.0 / 64`.
pub const DEFAULT_SIM_DT: f32 = 1.0 / DEFAULT_TICK_RATE;

/// Maximum number of ticks `SimClock` will ever run in a single frame.
///
/// This is the **spiral-of-death guard**. Without a clamp, if a frame takes
/// a very long time (a lag spike, a backgrounded tab coming back, a slow
/// GC pause), the clock would request hundreds or thousands of ticks to
/// catch up. Those ticks take CPU time, which extends the frame, which
/// requests even more ticks — the game never recovers.
///
/// With the clamp, the worst case is: run 5 ticks, drop the extra
/// accumulated time, and accept that the sim falls slightly behind wall
/// clock. The game visibly slows down for a moment instead of freezing.
///
/// 5 is a heuristic. 5 × 15.6ms = 78ms per frame maximum sim work, which
/// corresponds to a "12.8 fps floor" for the sim to keep up with real time.
/// Below that, the sim starts lagging behind — acceptable, because at
/// 12 fps the game is already unplayable for other reasons.
pub const DEFAULT_MAX_TICKS_PER_FRAME: u32 = 5;

/// Fixed-timestep accumulator.
///
/// Converts variable real time (how long the last frame took) into a
/// discrete number of *fixed-duration* sim ticks to run this frame.
///
/// ## The accumulator pattern
///
/// The clock holds a running total of "leftover real time that hasn't yet
/// become a tick." Each call to `consume(frame_dt)`:
///
/// 1. Adds `frame_dt` to the accumulator.
/// 2. Drains it in `step`-sized chunks, returning the count.
///
/// At 144 fps with a 64 Hz sim: each frame adds ~6.9ms. After 2–3 frames,
/// the accumulator crosses 15.6ms and a tick fires. Over 144 frames of
/// real time, ~64 ticks fire — correct.
///
/// At 30 fps: each frame adds ~33.3ms, which is > 15.6ms. Two ticks fire
/// immediately, 2ms of leftover carries forward. Over 30 frames, ~64
/// ticks fire — same result.
///
/// ## Why this isn't a Bevy `Resource`
///
/// `SimClock` is a plain struct. The *client* wraps it in `SimTiming`,
/// which is the `Resource`. The *server* (Phase 9) will wrap it however it
/// wants. `sim` doesn't know or care about Bevy resources — it knows about
/// accumulation and arithmetic.
#[derive(Debug, Clone)]
pub struct SimClock {
    /// Fractional real-time seconds that haven't yet been spent on a tick.
    /// Values in `[0.0, step)` after `consume` returns.
    accumulator: f32,
    /// The fixed duration of one tick, in seconds.
    step: f32,
    /// The cap on `consume`'s return value. See
    /// `DEFAULT_MAX_TICKS_PER_FRAME`.
    max_ticks_per_frame: u32,
}

impl SimClock {
    /// Construct a clock with a custom step and clamp. Most callers should
    /// use `default()` — this constructor exists for tests that need
    /// non-standard rates.
    pub fn new(step: f32, max_ticks_per_frame: u32) -> Self {
        Self {
            accumulator: 0.0,
            step,
            max_ticks_per_frame,
        }
    }

    /// The standard 64 Hz clock.
    pub fn default() -> Self {
        Self::new(DEFAULT_SIM_DT, DEFAULT_MAX_TICKS_PER_FRAME)
    }

    /// The fixed dt that callers should pass to each `sim::tick`.
    ///
    /// Exposed separately from `consume` so the caller can do the
    /// natural thing: `let n = clock.consume(frame_dt); for _ in 0..n {
    /// tick(world, inputs, clock.step()); }`.
    pub fn step(&self) -> f32 {
        self.step
    }

    /// Feed this frame's elapsed real time; return how many sim ticks to
    /// run this frame.
    ///
    /// See the struct doc for the accumulator mechanics.
    pub fn consume(&mut self, frame_dt: f32) -> u32 {
        // `.max(0.0)` guards against negative `frame_dt`. This can happen
        // on some platforms if the clock is adjusted backwards, or from a
        // buggy caller. Without the guard, a negative value would *subtract*
        // from the accumulator, effectively running time backwards.
        self.accumulator += frame_dt.max(0.0);

        // Clamp accumulated time to the maximum we're willing to spend this
        // frame. Without this, a 30-second stalled tab would accumulate 30
        // seconds of backlog and try to run ~1920 ticks in one frame,
        // freezing the browser.
        let max_accumulated = self.step * self.max_ticks_per_frame as f32;
        if self.accumulator > max_accumulated {
            self.accumulator = max_accumulated;
        }

        // Drain in `step`-sized chunks. The `ticks < max_ticks_per_frame`
        // bound is redundant after the clamp above but harmless — it makes
        // the guarantee explicit even if the clamp logic changes.
        let mut ticks = 0;
        while self.accumulator >= self.step && ticks < self.max_ticks_per_frame {
            self.accumulator -= self.step;
            ticks += 1;
        }
        ticks
    }
}

// ============================================================================
// Spawning
// ============================================================================
//
// Public spawn functions are the *only* way the sim should create entities.
// This keeps entity construction in one place, so a bullet always has the
// same components whether it was spawned by a `Fire` input in-game or by a
// test harness. If bullet construction ever needs to change (add a damage
// component, an owner field, whatever), one function changes.
//
// ----------------------------------------------------------------------------

/// Spawn a bullet at `origin` travelling in `direction` at `BULLET_SPEED`.
///
/// **Direction is normalized internally.** Callers can pass a raw input
/// direction (length 0 to √2) or a pre-normalized unit vector — both produce
/// a bullet at exactly `BULLET_SPEED`. This is deliberate: it means the
/// client doesn't have to pre-normalize before sending `Fire`, and a buggy
/// client that sends an un-normalized direction can't produce a
/// faster-than-legal bullet. Defense in depth.
///
/// **Zero direction is safe but odd.** A zero-length direction produces a
/// *stationary* bullet (`velocity = Vec2::ZERO`) that still expires after
/// `BULLET_LIFETIME` seconds. In practice `read_fire_input` never sends
/// zero (it skips Fire if the cursor is on the tank), but the sim doesn't
/// crash on it — it just spawns a useless bullet that despawns later.
///
/// Returns the spawned `Entity` so callers can track it if needed. Most
/// callers ignore the return value.
pub fn spawn_bullet(world: &mut World, origin: Vec2, direction: Vec2) -> Entity {
    let velocity = direction.normalize_or_zero() * BULLET_SPEED;
    world
        .spawn((
            Bullet,
            Position(origin),
            Velocity(velocity),
            Lifetime(BULLET_LIFETIME),
        ))
        .id()
}

// ============================================================================
// The tick — advance the world by one fixed step
// ============================================================================

/// Advance the simulation by one tick. Call once per fixed step.
///
/// This is the entire public API of the simulation. Everything else in this
/// file is either a helper called by `tick` or a type used by callers to
/// drive `tick` (`SimClock`, `spawn_bullet`).
///
/// ## The phase order is load-bearing
///
/// ```text
///   1. tick_cooldowns               FireCooldown -= dt
///   2. apply_inputs                 Move -> Velocity, Fire -> spawn bullet
///   3. integrate_positions          Position += Velocity * dt
///   4. tick_lifetimes_and_despawn   Lifetime -= dt, despawn at <= 0
/// ```
///
/// Each phase's placement is justified in its own doc. The short version:
/// cooldowns must tick before inputs so the first shot fires immediately
/// after cooldown expiry; inputs must run before integration so a bullet
/// spawned this tick moves this tick; integration must run before lifetime
/// so a bullet's final position is the one it had when it expired.
///
/// ## Why the inputs are a parameter, not a resource
///
/// The caller (client or server) owns the input stream. `tick` just
/// consumes a slice of `(Entity, InputCommand)` pairs. This is what makes
/// the sim testable — a test can hand `tick` a hand-crafted `Vec` without
/// needing a keyboard, a window, or a network connection.
pub fn tick(world: &mut World, inputs: &[(Entity, InputCommand)], dt: f32) {
    tick_cooldowns(world, dt);
    apply_inputs(world, inputs, dt);
    integrate_positions(world, dt);
    tick_lifetimes_and_despawn(world, dt);
}

/// Phase 1 — decrement every `FireCooldown` by `dt`, clamped at zero.
///
/// **Why this runs before `apply_inputs`:**
///
/// Consider firing on tick N. `apply_inputs` sets the cooldown to
/// `FIRE_INTERVAL`. If `tick_cooldowns` ran *after* that, it would
/// immediately subtract `dt`, so the cooldown would end up at
/// `FIRE_INTERVAL - dt` after the firing tick. The next shot would be
/// allowed one tick early.
///
/// By running cooldowns first, the sequence is: cooldown is 0, input sets
/// it to `FIRE_INTERVAL`, integration runs, tick ends. Next tick starts by
/// decrementing, and the cooldown reaches exactly 0 after exactly
/// `FIRE_INTERVAL / dt` ticks. Precise.
fn tick_cooldowns(world: &mut World, dt: f32) {
    let mut query = world.query::<&mut FireCooldown>();
    for mut cd in query.iter_mut(world) {
        // `.max(0.0)` clamps at zero. Without it, the cooldown would go
        // negative and take an extra tick to recover — a subtle
        // "cooldown is shorter than FIRE_INTERVAL" bug.
        cd.remaining = (cd.remaining - dt).max(0.0);
    }
}

/// Phase 2 — translate each `InputCommand` into a world mutation.
///
/// Two variants today:
///
/// - **`Move`** updates the entity's `Velocity` toward the target. Uses
///   `approach` for smooth acceleration — the tank ramps up to speed
///   rather than snapping.
///
/// - **`Fire`** spawns a bullet at the entity's `Position`, if the entity
///   is not on cooldown. Sets the cooldown if a shot fires.
///
/// Inputs are processed **sequentially per entity**. If the same entity
/// appears twice in `inputs` (which shouldn't happen — one command per
/// entity per tick is the client's contract — but might under buggy
/// callers), both commands execute. The last `Move` wins for velocity
/// (each call to `approach` starts from the previous result); every `Fire`
/// fires (subject to cooldown).
fn apply_inputs(world: &mut World, inputs: &[(Entity, InputCommand)], dt: f32) {
    for (entity, command) in inputs {
        match command {
            InputCommand::Move { direction } => {
                // Normalize + scale to target velocity. Zero direction
                // produces `Vec2::ZERO` target — the tank coasts to a stop.
                let target = direction.normalize_or_zero() * TANK_SPEED;

                // `if let Some` — entities without `Velocity` are silently
                // skipped. This is robustness against malformed entities
                // (a bullet doesn't receive Move commands, but a test
                // might). No panic, no error.
                if let Some(mut vel) = world.get_mut::<Velocity>(*entity) {
                    vel.0 = approach(vel.0, target, TANK_ACCELERATION * dt);
                }
            }

            InputCommand::Fire { direction } => {
                // Readiness check. `.unwrap_or(false)` means "entity has no
                // FireCooldown => cannot fire." This is deliberate — see the
                // FireCooldown doc in `shared`. Fire is opt-in.
                let ready = world
                    .get::<FireCooldown>(*entity)
                    .map(|cd| cd.remaining <= 0.0)
                    .unwrap_or(false);
                if !ready {
                    continue;
                }

                // Position read. Same defensive pattern — an entity without
                // Position can't fire (no origin). Silently skip.
                let origin = world.get::<Position>(*entity).map(|p| p.0);
                if let Some(origin) = origin {
                    spawn_bullet(world, origin, *direction);

                    // Set cooldown. `if let Some` because the entity might
                    // have lost its FireCooldown between the readiness check
                    // and this line (it can't, in a single-threaded tick,
                    // but the defensive pattern is consistent with the rest
                    // of this function).
                    if let Some(mut cd) = world.get_mut::<FireCooldown>(*entity) {
                        cd.remaining = FIRE_INTERVAL;
                    }
                }
            }
        }
    }
}

/// Phase 3 — integrate `Velocity` into `Position` for every entity with both.
///
/// **Why this iterates all entities, not just those in `inputs`:**
///
/// Bullets have `Velocity` but never appear in `inputs`. Mobs (Phase 3)
/// will have `Velocity` driven by AI, not player input. The integration
/// is universal — "anything with velocity moves by velocity × dt" — and
/// this is exactly the right scope.
///
/// If integration only iterated `inputs`, bullets would spawn and sit
/// frozen at their origin forever. That's why the phases are separate:
/// phase 2 is input-driven (small set, random access by ID), phase 3 is
/// world-driven (all entities, sequential iteration).
///
/// **Why explicit Euler:**
///
/// `pos += vel * dt` is the simplest integrator. It's stable for our
/// forces (bounded velocity, no springs), deterministic, and cheap. More
/// sophisticated integrators (Verlet, RK4) buy accuracy we don't need and
/// cost determinism-friendly arithmetic.
fn integrate_positions(world: &mut World, dt: f32) {
    let mut query = world.query::<(&Velocity, &mut Position)>();
    for (vel, mut pos) in query.iter_mut(world) {
        pos.0 += vel.0 * dt;
    }
}

/// Phase 4 — decrement every `Lifetime`, despawn anything that hits zero.
///
/// **Why two passes (collect, then despawn):**
///
/// You can't despawn an entity while iterating a query over it — the
/// iteration holds a borrow of the world, and `world.despawn()` needs
/// `&mut World`. So the pattern is:
///
/// 1. Iterate once, decrement, collect entities to despawn into a `Vec`.
/// 2. Drop the query (implicitly, by ending the `for` block).
/// 3. Iterate the `Vec`, call `world.despawn()` on each.
///
/// This is the standard Bevy pattern for "remove while iterating."
///
/// **Why `<= 0.0` and not `< 0.0`:**
///
/// At exactly zero, the entity's lifetime is over. Whether that's "it
/// expired this tick" or "it expired last tick and we're late" doesn't
/// matter for the visible behavior — the bullet should be gone. `<= 0.0`
/// means the despawn happens on the tick where `Lifetime` reaches zero,
/// which is what tests in `bullets.rs` expect.
///
/// **Why `.despawn()` and not `.despawn_recursive()`:**
///
/// In Bevy 0.16+, `Children` is a `RelationshipTarget`, and despawning a
/// parent cascades to its children via the relationship's `on_replace`
/// hook. The bullet's border child (added by `spawn_bullet_views` in the
/// client) despawns automatically. If you ever see bullet bodies disappear
/// but border dots remain, that's the cue to switch to `despawn_recursive`.
fn tick_lifetimes_and_despawn(world: &mut World, dt: f32) {
    let mut query = world.query::<(&mut Lifetime, Entity)>();
    let mut to_despawn: Vec<Entity> = Vec::new();

    for (mut life, entity) in query.iter_mut(world) {
        life.0 -= dt;
        if life.0 <= 0.0 {
            to_despawn.push(entity);
        }
    }

    // Query is dropped here — `to_despawn` holds only the entity IDs,
    // which are `Copy` and safe to use after the borrow ends.
    for entity in to_despawn {
        // `let _ =` because despawn returns `Result`. If the entity was
        // already removed (double-despawn bug elsewhere), the error is
        // silently ignored. A panic here would be worse than a silent
        // skip — a bullet that's already gone is harmless.
        let _ = world.despawn(entity);
    }
}

/// Move `current` toward `target` by at most `max_step` units.
///
/// This is the primitive that makes smooth acceleration possible. Instead
/// of `vel = target` (instant snap), phase 2 does `vel = approach(vel,
/// target, ACCELERATION * dt)` — the velocity moves closer to the target
/// each tick, by at most `ACCELERATION * dt` per step.
///
/// ## The three cases
///
/// 1. **Already at target** (`delta == 0`): returns `target`. No change.
/// 2. **Close enough** (`|delta| <= max_step`): returns `target`. Snaps
///    cleanly. Without this case, velocity would oscillate forever around
///    the target — step past, step back, step past, step back.
/// 3. **Far** (`|delta| > max_step`): returns `current + normalize(delta) *
///    max_step`. Moves `max_step` units along the direction to target.
///
/// ## Why `length_squared` and not `length`
///
/// The comparison `|delta| <= max_step` is equivalent to
/// `|delta|² <= max_step²` (both sides non-negative). The squared form
/// avoids a `sqrt` per call, per entity, per tick. At 64 ticks/s × 500
/// entities, that's 32,000 sqrts/s saved — not huge, but free.
///
/// ## Why `normalize()` in the else branch is safe
///
/// The `else` branch only executes when `|delta|² > max_step²`, which
/// implies `|delta| > 0`, which implies `delta != Vec2::ZERO`. So
/// `normalize()` never sees the zero vector — no division by zero, no
/// `NaN`. The `if` branch is the guard.
fn approach(current: Vec2, target: Vec2, max_step: f32) -> Vec2 {
    let delta = target - current;
    if delta.length_squared() <= max_step * max_step {
        // Close enough — snap to target. This terminates the ramp
        // cleanly. Without it, the tank's velocity would jitter by
        // ±(ACCELERATION × dt) around the target forever.
        target
    } else {
        // Far — step toward target by exactly max_step.
        // `normalize()` is safe here: the branch above guarantees
        // `delta != ZERO`.
        current + delta.normalize() * max_step
    }
}
