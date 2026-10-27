//! # Flarras.io — Shared vocabulary
//!
//! The **nouns** of the game. Components, markers, protocol messages, and
//! gameplay constants that both `sim` and `client` (and the future `server`)
//! agree on.
//!
//! ## What belongs here
//!
//! Anything that both the sim and the renderer need to *name*:
//!
//! - **Components** — `Position`, `Velocity`, `Lifetime`, `FireCooldown`.
//!   State the sim mutates and the client reads.
//! - **Markers** — `Player`, `Bullet`. Zero-data tags for query filters.
//! - **Protocol types** — `InputCommand`. The shape of "what a player
//!   wants to do this tick." Today it flows through a local resource; in
//!   Phase 9 the same type will flow over the network.
//! - **Tuning constants** — `TANK_SPEED`, `BULLET_LIFETIME`, etc. Numbers
//!   that define gameplay. If changing a number changes how the game
//!   *plays*, it lives here.
//!
//! ## What must NOT belong here
//!
//! - **Rendering types** — `Color`, `Mesh2d`, `Transform`, `Camera2d`.
//!   These live in `client` because `shared` must compile for the headless
//!   server.
//! - **Networking types** — `serde::Serialize`, WebSocket handles, network
//!   IDs. These arrive in Phase 9 and will live in a `protocol` module
//!   *inside* this crate, or a dedicated `net` crate.
//! - **Wall-clock time** — `std::time::Instant`, `Instant::now()`. The sim
//!   receives `dt` as a parameter; it never reads the clock.
//!
//! ## The compile-enforced fence
//!
//! `shared` depends on exactly two crates: `bevy_ecs` (for `Component`) and
//! `bevy_math` (for `Vec2`). That's it. If a type in here needs any other
//! import to compile, it's in the wrong crate.
//!
//! This is checked every build. Adding `use bevy::prelude::*` here would
//! pull in the rendering stack and make the future server impossible to
//! compile without a GPU — the compiler prevents that mistake by design.

use bevy_ecs::prelude::*;
use bevy_math::Vec2;

// ============================================================================
// Tuning constants
// ============================================================================
//
// These are the knobs you twist to change how the game feels. They're grouped
// by the entity they govern, and grouped *together* rather than scattered
// across components, because tuning is an act of comparing numbers against
// each other — you want them in one place.
//
// Units matter:
//
//   - Speeds are **units per second**, not per tick. The sim multiplies by
//     `dt` internally, so a `TANK_SPEED = 150` means "150 units in one
//     second" regardless of tick rate.
//   - Lifetimes are **seconds**. The sim subtracts `dt` each tick.
//   - Radii are **world units** (pixels at default camera zoom).
//
// ----------------------------------------------------------------------------

// --- Tank ---

/// Maximum tank speed, in world units per second.
/// This is the *cap*; the tank ramps up to it over `TANK_SPEED /
/// TANK_ACCELERATION` seconds and coasts down from it at the same rate.
pub const TANK_SPEED: f32 = 150.0;

/// How fast the tank's velocity changes, in units per second squared.
/// At 1000 u/s², the tank takes 0.15s to reach full speed from rest.
/// Higher = snappier feel (more "arcade"), lower = more "weighty".
pub const TANK_ACCELERATION: f32 = 1000.0;

/// Tank collision radius. The visible body is 1 unit smaller, with the
/// border ring covering that gap, so the *visual* outline and the *collision*
/// circle stay in sync.
pub const TANK_RADIUS: f32 = 30.0;

// --- Bullets ---

/// Bullet travel speed, in world units per second.
/// 3.3× the tank's max speed — fast enough to feel like a bullet, slow
/// enough that a moving target can dodge.
pub const BULLET_SPEED: f32 = 500.0;

/// How long a bullet lives before despawning, in seconds.
/// At `BULLET_SPEED = 500`, this gives a maximum range of 1000 units —
/// roughly 16 tank-widths. Once collision exists (Phase 3), most bullets
/// will die on hit; this is the ceiling for misses.
pub const BULLET_LIFETIME: f32 = 2.0;

/// Bullet collision radius. Also used for rendering.
pub const BULLET_RADIUS: f32 = 6.0;

// --- Firing ---

/// Minimum time between shots from one entity, in seconds.
///
/// The sim enforces this via `FireCooldown` on each firing entity. The
/// client can send `Fire` as fast as it likes; the sim rejects shots
/// during cooldown. That makes the cooldown **authoritative** — when the
/// server arrives in Phase 9, this same check will hold, and a modified
/// client cannot cheat by spamming Fire.
pub const FIRE_INTERVAL: f32 = 1.0;

// ============================================================================
// Markers
// ============================================================================
//
// Markers are empty components. They carry no data — their only job is to
// make an entity *match a query filter*. A "player tank" is not "an entity
// with a `PlayerData` component"; it's "an entity with a `Player` marker
// plus whatever data components it needs."
//
// This is the standard ECS idiom: "data in components, meaning in markers."
// It keeps queries cheap (a marker is a zero-sized type, stored as a single
// bit in the archetype bitset) and lets you add data to players incrementally
// without ever touching the marker.
//
// ----------------------------------------------------------------------------

/// Marker: this entity is a player-controlled tank.
///
/// Distinct from `LocalPlayer` (which lives in the client) — `Player` tags
/// *all* players in the world (N of them in multiplayer). `LocalPlayer`
/// answers "which one does *my* keyboard control." Today they coincide;
/// in Phase 9 they diverge, and that distinction is what makes the
/// transition from single-player to multiplayer tractable.
#[derive(Component, Debug, Clone, Copy)]
pub struct Player;

/// Marker: this entity is a bullet.
///
/// Used by:
///   - `spawn_bullet_views` in the client (finds fresh bullets to dress with
///     render components)
///   - Collision detection in Phase 3 (bullets vs tanks, bullets vs mobs)
///   - The sim's bullet-count queries in tests
#[derive(Component, Debug, Clone, Copy)]
pub struct Bullet;

// ============================================================================
// Components
// ============================================================================
//
// These carry the *state* that the sim reads and writes. Every component here
// is `Copy` and small (2-8 bytes) — they're designed to be moved in and out
// of the world cheaply, so the sim can iterate thousands of them without
// hitting the allocator.
//
// `Debug` is included everywhere so test failures print readable values. If
// you ever see `Position { x: NaN, y: 0.0 }` in a failing test, that's `Debug`
// doing its job.
//
// ----------------------------------------------------------------------------

/// Where a gameplay entity sits in the world. Sim-owned.
///
/// The tuple struct wraps `Vec2` directly — no named fields — because:
///
///   1. Brevity at call sites: `pos.0` instead of `pos.value`.
///   2. The wrapper exists to hide the *type* (`Position` vs a raw `Vec2`),
///      not to add fields. Fields would be noise.
///   3. It matches Bevy's idiom (`Transform.translation` is a `Vec3`).
///
/// `Default` gives `Vec2::ZERO` — the origin. Fresh spawns can call
/// `Position::default()` and land at (0, 0) without needing the Vec2 import
/// at the call site.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Position(pub Vec2);

/// How fast it's moving, in world units per second.
///
/// Same tuple-struct pattern as `Position`, for the same reasons. The
/// units are per-second (not per-tick), so the sim multiplies by `dt`
/// each integration step.
///
/// `Velocity` is deliberately *separate* from `Position`, not fused into
/// a `Motion { pos, vel }` struct, because:
///
///   1. The sim has two phases with different access patterns: phase 1
///      needs `&mut Velocity` by entity ID; phase 2 needs
///      `(&Velocity, &mut Position)` iterated. Separate components let
///      Bevy's query engine express each phase independently.
///
///   2. Future entity types compose them differently. A bullet has
///      `Position + Velocity + Lifetime`. A petal has `Position` but no
///      `Velocity` (it orbits via rotation). A wall has `Position` but
///      nothing else. Fusing them would force every entity to carry both.
///
///   3. When Phase 9 arrives, `Position` is what gets replicated (it's
///      the visible state); `Velocity` is derived. Keeping them separate
///      means you replicate only what's needed.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Velocity(pub Vec2);

/// Remaining seconds before this entity despawns.
///
/// A plain `f32` rather than Bevy's `Timer` because:
///
///   1. `Timer` has pause/resume/once/repeating modes, but here we only
///      need "decrement every tick until zero." The simpler type is
///      easier to reason about and test.
///
///   2. `f32` is `Copy` and 4 bytes — trivially movable, cheap to query.
///      `Timer` is 24 bytes and not `Copy`.
///
///   3. Determinism: the sim decrements `life.0 -= dt` where `dt` is the
///      fixed timestep. A `Timer` would do the same internally, but the
///      explicit form makes the tick-by-tick behaviour auditable.
///
/// No `Default` derive: a `Lifetime` of 0 would despawn immediately, and
/// there's no sensible default value. Callers must pass an explicit
/// duration (`Lifetime(BULLET_LIFETIME)`), which is what you want —
/// spawning with the wrong lifetime should require a deliberate value.
#[derive(Component, Debug, Clone, Copy)]
pub struct Lifetime(pub f32);

/// Ticks down to zero; while active, blocks new `Fire` inputs.
///
/// **Presence is opt-in:** an entity *without* `FireCooldown` cannot fire
/// at all. This is deliberate. Fire rate is a gameplay rule, and putting
/// the component on an entity is an explicit assertion "this entity is
/// subject to fire-rate limits." If a future entity type (a mob with a
/// ranged attack, a summoner tower) forgets to opt in, it can't fire —
/// which is a safe failure, not a runaway-fire bug.
///
/// The sim's check is:
///
/// ```text
/// ready = entity has FireCooldown && FireCooldown.remaining <= 0.0
/// ```
///
/// If `ready`, spawn the bullet and reset `remaining = FIRE_INTERVAL`.
/// Otherwise, silently skip the Fire input — no error, no bullet.
///
/// Named field (`remaining` rather than `.0`) because this is the one
/// component likely to grow: future features might add `queued: bool` or
/// `last_fire_tick: u64`. Named fields absorb that growth without call-
/// site changes.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FireCooldown {
    pub remaining: f32,
}

// ============================================================================
// Input — the protocol between client and sim
// ============================================================================
//
// `InputCommand` is what makes the sim headlessly testable. Tests never
// construct a keyboard or a window — they build a `Vec<(Entity,
// InputCommand)>` and hand it to `sim::tick`. In Phase 9, the *same type*
// flows over the network: the client serializes it, the server deserializes
// it, and both sides agree on exactly what a "player wants to Move right"
// means.
//
// ----------------------------------------------------------------------------

/// One command a player can issue in a tick.
///
/// **Why an enum, not a struct with optional fields.**
///
/// A struct would look like `{ move_dir: Vec2, fire: bool }`, which is
/// fine until you add a third action. Then it becomes
/// `{ move_dir, fire, aim_dir, swap_petal: Option<u8>, sprint: bool, ... }`
/// — most fields `Option` or default-zero, and every caller responsible
/// for setting all of them correctly. That's a maintenance trap.
///
/// An enum says "an input is *one of* these actions." Adding an action is
/// a new variant; existing match arms get a compiler error until they
/// handle it. Extensibility by compiler enforcement.
///
/// **Why both variants use struct form (`{ direction }`).**
///
/// `Move { direction: Vec2 }` and `Fire { direction: Vec2 }` are structurally
/// identical to `Move(Vec2)` and `Fire(Vec2)` — same data, same memory layout.
/// The struct form is chosen because:
///
///   1. Field names document intent: `direction` means "in this direction,"
///      not "from this position" or "at this target."
///   2. It's the natural form for `Serialize` in Phase 9 — JSON like
///      `{"Fire": {"direction": [1.0, 0.0]}}` is self-documenting.
///   3. Future fields (`weapon_slot`, `power`, `charge`) can be added
///      without touching existing match arms.
///
/// **Why `Copy` and no `Serialize` yet.**
///
/// `Copy` is safe here (the enum is 12 bytes and has no heap data). But
/// `Serialize`/`Deserialize` derives are deliberately deferred — they add
/// a dependency and constraint that isn't needed until the network
/// protocol is real. When Phase 9 arrives, adding the derives here is a
/// one-line change that immediately makes every existing `InputCommand`
/// cross the wire.
#[derive(Debug, Clone, Copy)]
pub enum InputCommand {
    /// Desired movement direction. Length may be 0..=√2; sim clamps it.
    ///
    /// The length range is intentional: raw keyboard input gives 0
    /// (no keys), 1 (one key), or √2 ≈ 1.414 (two perpendicular keys).
    /// The client does not normalize — the sim does, via
    /// `normalize_or_zero()` inside `apply_inputs`. Keeping normalization
    /// in the sim means:
    ///
    ///   - A modified client cannot smuggle a faster-than-allowed
    ///     direction past the server (defense in depth).
    ///   - One place to change the movement rules, not every input source.
    Move { direction: Vec2 },

    /// Fire one bullet in the given direction. Ignored if `FireCooldown`
    /// is active on the firing entity.
    ///
    /// The direction is the *aim* direction, distinct from movement
    /// direction. This is what lets you strafe while shooting — the whole
    /// point of top-down shooters. Cursor position (converted to world
    /// space) determines `direction`; the tank's velocity is irrelevant.
    ///
    /// In Phase 9 this becomes a network message. The client sends it
    /// once per click (or per frame while held, with the cooldown
    /// absorbing duplicates on the server); the server validates the
    /// cooldown and either spawns a bullet or silently drops the input.
    Fire { direction: Vec2 },
}
