//! # Flarras.io — Client
//!
//! The renderer and input layer. This crate owns **no gameplay state**. It
//! reads devices (keyboard, mouse), converts them into `InputCommand`s for
//! the sim, advances the sim, and reflects the resulting `Position`s back
//! onto screen-space `Transform`s.
//!
//! ## The three-world model
//!
//! Three kinds of data coexist in this file, and mixing them up is the
//! single biggest source of client bugs. The rule is simple:
//!
//! 1. **Sim data** (from `shared`) — `Position`, `Velocity`, `Player`,
//!    `Bullet`, `FireCooldown`. Written *only* by `sim::tick`. The client
//!    may read them; it must not mutate them.
//!
//! 2. **Render data** (from Bevy) — `Mesh2d`, `MeshMaterial2d`, `Transform`,
//!    `Camera2d`. Read by the GPU, written by the client's spawn/sync code.
//!
//! 3. **The bridge** — systems that translate between the two worlds.
//!    Three are *core*: `read_input` (devices → command), `run_sim` (advance
//!    sim), `sync_transforms` (sim position → render position). Three more
//!    are *auxiliary*: `read_fire_input` (mouse → command),
//!    `spawn_bullet_views` (new sim entities → render entities),
//!    `rotate_barrel_toward_cursor` (input + sim position → render rotation).
//!
//! The invariant that keeps this tractable:
//!
//! > **`Transform.translation` is written only by `sync_transforms`**
//! > (plus one-time initialization at spawn).
//!
//! If that ever stops being true, the sim/render boundary has leaked and
//! you will spend hours chasing "why does the tank look right but the hitbox
//! is wrong."
//!
//! ## The per-frame pipeline
//!
//! The Update schedule is a single `.chain()`. Every system below runs in
//! exactly the order shown, once per frame:
//!
//! ```text
//!   keyboard ──► read_input ────────┐
//!                                    ├──► PendingInput ──► run_sim ──► sim::tick × N
//!   mouse ────► read_fire_input ────┘                                        │
//!                                                                            ▼
//!                                     (sim spawns bullets with only sim components)
//!                                                                            │
//!                                     spawn_bullet_views ◄───────────────────┘
//!                                     (adds Mesh2d + Transform to fresh bullets)
//!                                                                            │
//!                                     sync_transforms ◄──────────────────────┤
//!                                     (Position → Transform for tank + bullets)
//!                                                                            │
//!                                     rotate_barrel_toward_cursor ◄──────────┘
//!                                     (barrel points at cursor, using NEW tank Position)
//!                                                                            │
//!                                                                            ▼
//!                                                                      Bevy renders
//! ```
//!
//! Unchaining them lets Bevy's scheduler reorder the pipeline, producing a
//! 1-frame input lag and bullets that render at origin for one frame. Both
//! are silent bugs.
//!
//! ## Panic output
//!
//! `console_error_panic_hook::set_once()` is installed at the top of
//! `main()`. Without it, a Rust panic in WASM shows only
//! `unreachable executed` in the browser console — no file, no line, no
//! message. With it, panics print the message and a stack trace, which is
//! the difference between a 5-second fix and a 20-minute hunt.

use bevy::{prelude::*, window::PrimaryWindow};
use shared::*;

// ============================================================================
// Visual constants
// ============================================================================
//
// These live in the client, not `shared`, because they are `Color` values,
// and `Color` is a rendering type. `shared` deliberately depends only on
// `bevy_ecs` + `bevy_math` so it can compile for headless targets (the
// future server). Gameplay constants (speeds, radii, lifetimes) live in
// `shared`; anything that ends up as a pixel lives here.
//
// ----------------------------------------------------------------------------

const BACKGROUND_COLOR: Color = Color::srgb_u8(0xdb, 0xdb, 0xdb);
const TANK_COLOR: Color = Color::srgb_u8(0x3c, 0xa4, 0xcb);
const BARREL_COLOR: Color = Color::srgb_u8(0xa7, 0xa7, 0xa7);

/// Bullets share the tank's colour. Deliberate: the bullet and the tank read
/// as "the same player's stuff," so bullet-vs-mob collisions (Phase 3) will
/// be visually obvious — your shots are the same hue as your body.
const BULLET_COLOR: Color = TANK_COLOR;

const BARREL_LENGTH: f32 = 40.0;
const BARREL_WIDTH: f32 = 8.0;

/// Uniform border width for tank ring, barrel outline, and bullet ring.
/// One number so all three elements read as a coherent visual family.
const BORDER_THICKNESS: f32 = 3.0;

/// How much black to mix into a base colour for its border.
/// Lower = lighter border. Applied via `Color::mix(&BLACK, BORDER_MIX)` so
/// every border derives from its own body colour — change `TANK_COLOR` and
/// the tank border follows automatically.
const BORDER_MIX: f32 = 0.3;

// ============================================================================
// Resources
// ============================================================================
//
// A `Resource` is a single global value — one instance per type. These four
// are the client's entire persistent state outside the ECS entities.
//
// ----------------------------------------------------------------------------

/// Inputs accumulated this frame by `read_input` + `read_fire_input`, and
/// consumed by `run_sim`.
///
/// Shape is `Vec<(Entity, InputCommand)>` — one entry per controllable
/// entity. It's a `Vec` rather than a single value because Phase 9 will
/// produce N entries per tick (one per player), and the client should share
/// the shape now so the swap later is a transport change, not a type change.
#[derive(Resource, Default)]
struct PendingInput(Vec<(Entity, InputCommand)>);

/// Which entity this client's keyboard and mouse control.
///
/// Distinct from the `Player` *component*, which tags "this entity is a
/// player" (N of them in multiplayer). `LocalPlayer` answers "which one is
/// *mine*" (exactly 1). Today the two coincide; in Phase 9 they diverge,
/// and this resource is what makes that transition cheap.
#[derive(Resource)]
struct LocalPlayer(Entity);

/// The fixed-timestep accumulator that decides how many `sim::tick`s run
/// this frame. Wraps `sim::SimClock` in a Bevy resource so systems can
/// access it.
///
/// `SimClock` itself lives in `sim` (not here) because the future server
/// will need the same accumulator, and putting it in the client would force
/// a reimplementation there — with a chance of drift between the two copies.
#[derive(Resource)]
struct SimTiming(sim::SimClock);

/// Reusable mesh + material handles for every bullet.
///
/// Key detail: `Handle<Mesh>` and `Handle<ColorMaterial>` are cheap
/// reference-counted pointers to GPU-resident assets. Cloning a handle for
/// each bullet costs 8 bytes and does **not** re-upload geometry. A thousand
/// bullets share one mesh and one material on the GPU — that's what makes
/// this scale.
///
/// If bullets ever become visually diverse (rarities, damage types), this
/// resource grows into a map from bullet-type -> assets.
#[derive(Resource)]
struct BulletAssets {
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
    border_mesh: Handle<Mesh>,
    border_material: Handle<ColorMaterial>,
}

// ============================================================================
// Components
// ============================================================================
//
// Client-only markers used for queries. These never enter `sim`.
//
// ----------------------------------------------------------------------------

/// Tags the barrel entity so `rotate_barrel_toward_cursor` can find it.
/// Empty — the barrel has no per-entity state beyond its `Transform`, which
/// belongs to the renderer.
#[derive(Component)]
struct Barrel;

// ============================================================================
// App
// ============================================================================

fn main() {
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                // Makes the canvas resize with its HTML parent (set in
                // `index.html` to 100vw × 100vh). Without this, the canvas
                // stays at its internal resolution regardless of screen size.
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BACKGROUND_COLOR))
        .init_resource::<PendingInput>()
        .insert_resource(SimTiming(sim::SimClock::default()))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                read_input,
                read_fire_input,
                run_sim,
                spawn_bullet_views,
                sync_transforms,
                rotate_barrel_toward_cursor,
            )
                // `.chain()` forces strict sequential execution in the
                // listed order. Without it, Bevy's parallel scheduler could
                // run `run_sim` before `read_input`, applying this frame's
                // input on the *next* frame — a silent 1-frame input lag.
                //
                // Each step depends on the previous:
                //
                //   read_input           PendingInput = this frame's Move
                //   read_fire_input      PendingInput += this frame's Fire
                //   run_sim              drain PendingInput, call sim::tick
                //   spawn_bullet_views   new bullets get Mesh2d+Transform
                //   sync_transforms      Position -> Transform
                //   rotate_barrel_...    barrel points at cursor
                //
                // Rotation is last because it reads the tank's *new*
                // `Position` this frame, not last frame's.
                .chain(),
        )
        .run();
}

// ============================================================================
// Startup — camera + local player
// ============================================================================

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    // ------------------------------------------------------------------------
    // Tank meshes
    // ------------------------------------------------------------------------
    //
    // Body is a filled circle 1 unit smaller than `TANK_RADIUS`. The border
    // ring sits on top of that 1-unit gap, so the combined visual outline
    // matches the collision radius exactly. Small detail, but it keeps "the
    // tank *looks* 30 units across" and "the tank *is* 30 units across"
    // in sync — which will matter once collision exists.
    let body_mesh = meshes.add(Circle::new(TANK_RADIUS - 1.0));
    let body_material = materials.add(TANK_COLOR);
    let border_mesh = meshes.add(Circle::new(TANK_RADIUS).to_ring(BORDER_THICKNESS));
    let border_material = materials.add(TANK_COLOR.mix(&Color::BLACK, BORDER_MIX));

    // ------------------------------------------------------------------------
    // Barrel meshes
    // ------------------------------------------------------------------------
    //
    // A rectangle, centered at its own origin. The trick to make it pivot
    // around the *tank* center (not its own) lives in the spawn transform
    // and in `rotate_barrel_toward_cursor`.
    let barrel_mesh = meshes.add(Rectangle::new(BARREL_LENGTH, BARREL_WIDTH));
    let barrel_material = materials.add(BARREL_COLOR);
    let barrel_border_mesh =
        meshes.add(Rectangle::new(BARREL_LENGTH, BARREL_WIDTH).to_ring(BORDER_THICKNESS));
    let barrel_border_material = materials.add(BARREL_COLOR.mix(&Color::BLACK, BORDER_MIX));

    // ------------------------------------------------------------------------
    // Bullet assets
    // ------------------------------------------------------------------------
    //
    // Created once here; cloned into every bullet by `spawn_bullet_views`.
    let bullet_mesh = meshes.add(Circle::new(BULLET_RADIUS));
    let bullet_material = materials.add(BULLET_COLOR);
    let bullet_border_mesh = meshes.add(Circle::new(BULLET_RADIUS).to_ring(BORDER_THICKNESS));
    let bullet_border_material = materials.add(BULLET_COLOR.mix(&Color::BLACK, BORDER_MIX));
    commands.insert_resource(BulletAssets {
        mesh: bullet_mesh,
        material: bullet_material,
        border_mesh: bullet_border_mesh,
        border_material: bullet_border_material,
    });

    // ------------------------------------------------------------------------
    // Tank entity
    // ------------------------------------------------------------------------
    //
    // Six components on one entity. Three are sim data (from `shared`),
    // three are render data (from Bevy). This dual-nature is temporary: in
    // Phase 9 the server will own the sim half and the client will own the
    // render half, on separate entities linked by entity ID.
    let entity = commands
        .spawn((
            // Sim components — the sim sees and mutates these
            Player,
            Position(Vec2::ZERO),
            Velocity(Vec2::ZERO),
            FireCooldown::default(),
            // Render components — the GPU sees these
            Mesh2d(body_mesh),
            MeshMaterial2d(body_material),
            // `Transform::default()` puts the tank at origin with identity
            // rotation and scale. `sync_transforms` overwrites
            // `translation.x/y` every frame from `Position`, so the exact
            // starting value here is irrelevant after frame 1.
            Transform::default(),
        ))
        .with_children(|parent| {
            // --- Tank border ----------------------------------------------
            //
            // No `Transform` — the child's default is identity, so it
            // renders exactly at the parent's position. Zero per-frame sync
            // cost, no risk of drift. The hierarchy *is* the sync.
            parent.spawn((Mesh2d(border_mesh), MeshMaterial2d(border_material)));

            // --- Barrel ----------------------------------------------------
            //
            // `Transform::from_xyz(BARREL_LENGTH / 2.0, 0.0, -0.1)`:
            //
            //   x = +20 : shifts the rectangle's center 20 units along local
            //             +x. Since the rectangle is 40 wide, this puts its
            //             *left edge* at the tank center. At angle 0, the
            //             barrel extends from tank center to (40, 0) — a
            //             proper barrel emerging from the tank.
            //
            //             This x-offset is only correct at angle 0. Once
            //             rotation kicks in, if x/y stayed fixed the barrel
            //             would pivot around (20, 0) instead of (0, 0). See
            //             `rotate_barrel_toward_cursor` for the fix that
            //             rewrites translation every frame.
            //
            //   z = -0.1 : behind the tank body and border (both z=0). The
            //             `Camera2d` is orthographic, so z is *only*
            //             layering, not distance. The barrel is physically
            //             40 units long, tank radius is 30, so 10 units of
            //             barrel poke out past the tank silhouette — exactly
            //             the visible "gun barrel" look.
            parent
                .spawn((
                    Barrel,
                    Mesh2d(barrel_mesh),
                    MeshMaterial2d(barrel_material),
                    Transform::from_xyz(BARREL_LENGTH / 2.0, 0.0, -0.1),
                ))
                .with_children(|barrel| {
                    // --- Barrel border ------------------------------------
                    //
                    // z = +0.05 in barrel-local space, which is
                    // -0.1 + 0.05 = -0.05 in world space. That's in front
                    // of the barrel body (-0.1) but behind the tank body
                    // (0). The visible portion of the barrel is outlined;
                    // the buried portion stays hidden behind the tank — no
                    // double-outline.
                    //
                    // The border inherits the barrel's rotation through
                    // transform propagation — no per-frame system needed.
                    barrel.spawn((
                        Mesh2d(barrel_border_mesh),
                        MeshMaterial2d(barrel_border_material),
                        Transform::from_xyz(0.0, 0.0, 0.05),
                    ));
                });
        })
        .id();

    // Record the tank's entity ID so input systems know who to drive.
    //
    // `insert_resource` here is *deferred* — it's queued until end of
    // `Startup`. That's why `read_input` takes `Option<Res<LocalPlayer>>`:
    // if for any reason it runs before the flush, it skips this frame
    // instead of panicking.
    commands.insert_resource(LocalPlayer(entity));
}

// ============================================================================
// Input — read devices, produce `InputCommand`s
// ============================================================================
//
// Input systems never touch `sim` state. They only write to `PendingInput`.
// This is the boundary that makes the sim headlessly testable: no test ever
// needs a window or keyboard, because `sim::tick` only sees `InputCommand`s.
//
// ----------------------------------------------------------------------------

/// WASD/arrows → `InputCommand::Move`.
///
/// Movement is *level-triggered*: pressed → moving every frame. That's why
/// this uses `pressed()` (not `just_pressed`). The sim handles acceleration
/// and deceleration internally, so this system only sends intent.
///
/// **Contract with the sim:** every controllable player must appear in
/// `inputs` every tick, even with zero direction. Otherwise the sim can't
/// distinguish "not pressing keys" from "not connected," and the tank
/// coasts forever instead of stopping. That's why the `push` happens
/// unconditionally, even when `direction == Vec2::ZERO`.
fn read_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    local: Option<Res<LocalPlayer>>,
    mut pending: ResMut<PendingInput>,
) {
    let Some(local) = local else {
        return;
    };

    // Build a raw direction vector from held keys. Not normalized here —
    // W+D gives (1, 1), not (0.707, 0.707). Normalization is the sim's job
    // (see `sim::apply_inputs`), because it's a physics decision and belongs
    // in one place. If the client normalized, it would be making a gameplay
    // decision, and multiplayer clients could disagree.
    //
    // `any_pressed([a, b])` is a shortcut for `pressed(a) || pressed(b)`
    // that also documents intent: "this action is bound to any of these
    // keys." Four identical-shaped lines read as a small keybinding table.
    let mut direction = Vec2::ZERO;
    if keyboard.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        direction.y += 1.0;
    }
    if keyboard.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        direction.y -= 1.0;
    }
    if keyboard.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        direction.x -= 1.0;
    }
    if keyboard.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        direction.x += 1.0;
    }

    // Overwrite (not append) so this frame's input replaces last frame's.
    // `read_fire_input` runs after this and appends its Fire commands,
    // giving `PendingInput` an exact "this frame's inputs" state.
    pending.0.clear();
    pending.0.push((local.0, InputCommand::Move { direction }));
}

/// Left mouse → `InputCommand::Fire`, aimed from tank to cursor.
///
/// Uses `pressed()` (not `just_pressed`) intentionally: the sim has a
/// `FireCooldown` that rejects rapid Fire inputs, so holding the button
/// produces auto-fire at the cooldown's rate. This matches arras/florr
/// behaviour and avoids "lost clicks" on frames where `run_sim` had zero
/// ticks (which happens on high-refresh displays where the accumulator
/// hasn't filled yet).
///
/// The whole `let Some(...) else { return }` cascade is defensive. Each
/// piece of state (local player, tank position, window, cursor, camera)
/// might legitimately be missing — e.g., the cursor left the window — and
/// we just don't fire that frame. No panic, no error spam.
fn read_fire_input(
    mouse: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    positions: Query<&Position>,
    local: Option<Res<LocalPlayer>>,
    mut pending: ResMut<PendingInput>,
) {
    if !mouse.pressed(MouseButton::Left) {
        return;
    }
    let Some(local) = local else {
        return;
    };
    let Some(tank_pos) = positions.get(local.0).ok() else {
        return;
    };
    let Some(window) = window_q.iter().next() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Some((camera, cam_transform)) = camera_q.iter().next() else {
        return;
    };

    // Convert screen-space cursor to world-space. The camera's orthographic
    // projection + `GlobalTransform` determine this; the helper handles the
    // details. Fails if the camera's projection is degenerate — we skip.
    let Ok(world_pos) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };

    // Fire *from tank toward cursor*. Using `Position` (sim data), not
    // `Transform` (render data), because the direction is a gameplay input
    // that flows into the sim. Direction is normalized here so the
    // `!= ZERO` check is meaningful — (0,0) would mean "fire at self."
    let fire_dir = (world_pos - tank_pos.0).normalize_or_zero();
    if fire_dir != Vec2::ZERO {
        pending.0.push((
            local.0,
            InputCommand::Fire {
                direction: fire_dir,
            },
        ));
    }
}

// ============================================================================
// Sim bridge — advance the simulation
// ============================================================================

/// Advance the simulation by 0..N fixed ticks this frame.
///
/// This is an **exclusive system** — it takes `&mut World` instead of typed
/// parameters. That's required because `sim::tick` itself takes `&mut World`.
/// Exclusive systems have full access to the world, so they can call any
/// system-shaped function.
///
/// `SimClock` decides *how many* ticks to run based on real elapsed time
/// (`frame_dt`). Each tick always advances the sim by exactly `1/64s`,
/// regardless of frame rate. The two time concepts are decoupled by design:
/// slow frames run more ticks, fast frames run fewer, and the sim never
/// sees variable `dt`.
fn run_sim(world: &mut World) {
    let frame_dt = world.resource::<Time>().delta_secs();

    // Move `PendingInput.0` into a local var, leaving an empty Vec behind.
    // This *releases the mutable borrow* of the world that would otherwise
    // persist while we call `sim::tick(world, ...)`. The alternative —
    // holding a borrow of `PendingInput` while passing `&mut World` — is
    // rejected by the borrow checker.
    let inputs = {
        let mut pending = world.resource_mut::<PendingInput>();
        std::mem::take(&mut pending.0)
    };

    // Determine how many ticks to run and the fixed `dt` for each.
    // `consume` accumulates fractional real time, drains it in `step`-sized
    // chunks, and clamps the max catch-up to avoid spiral-of-death when a
    // tab was backgrounded for seconds.
    let (tick_count, step) = {
        let mut timing = world.resource_mut::<SimTiming>();
        let n = timing.0.consume(frame_dt);
        (n, timing.0.step())
    };

    // Each tick sees the same `inputs`. On a slow frame, `tick_count == 2`
    // and `sim::tick` is called twice with the same Move — harmless, since
    // Move just sets velocity to the same target. Fire, however, is rate-
    // limited by `FireCooldown`, so a replayed Fire is a no-op. If Fire
    // ever becomes uncooldown'd, this replay would need fixing — but the
    // cooldown design makes replay *correct*, not just survivable.
    for _ in 0..tick_count {
        sim::tick(world, &inputs, step);
    }
}

// ============================================================================
// Rendering — reflect sim state visually
// ============================================================================

/// Attach render components to bullets the sim just spawned.
///
/// `sim::tick` creates bullets with only sim components (`Bullet`,
/// `Position`, `Velocity`, `Lifetime`) — it doesn't know what a `Mesh2d` is,
/// and never will. This system detects those fresh entities and dresses them
/// for the renderer.
///
/// The query `(With<Bullet>, Without<Mesh2d>)` is exact: "is a bullet, has
/// no mesh yet." Since the insert adds `Mesh2d`, an entity matches this
/// query exactly once (between being spawned by the sim and being dressed
/// by this system), then never again. No cleanup, no state tracking.
///
/// `Transform::from_xyz(pos.0.x, pos.0.y, 0.0)` sets the render position
/// from the sim position *at insert time*. `sync_transforms` runs after
/// this system in the chain, so if we didn't initialize `Transform` here,
/// the bullet would render at (0, 0) for one frame.
fn spawn_bullet_views(
    mut commands: Commands,
    assets: Res<BulletAssets>,
    new_bullets: Query<(Entity, &Position), (With<Bullet>, Without<Mesh2d>)>,
) {
    for (entity, pos) in &new_bullets {
        commands
            .entity(entity)
            .insert((
                Mesh2d(assets.mesh.clone()),             // handle clone — cheap
                MeshMaterial2d(assets.material.clone()), // handle clone — cheap
                Transform::from_xyz(pos.0.x, pos.0.y, 0.0),
            ))
            .with_children(|parent| {
                // Border child, at z = 0.05 relative to body (0.0) — renders
                // on top of the body, hiding the outer edge.
                parent.spawn((
                    Mesh2d(assets.border_mesh.clone()),
                    MeshMaterial2d(assets.border_material.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.05),
                ));
            });
    }
}

/// Copy `Position` (sim) into `Transform.translation` (render) for every
/// entity that has both.
///
/// **This is the only system in the codebase that writes
/// `Transform.translation.x/y`.** If that ever appears anywhere else, the
/// architectural boundary has leaked. A quick check:
/// `grep -rn "transform.translation" client/src/` should return only this
/// function (plus the bullet `Transform` initializer above, which sets it
/// once at spawn).
///
/// Runs on everything with `Position + Transform` — the tank, its bullets,
/// and anything else. No `With<Player>` filter, because bullets also need
/// syncing. If you later add entities that have `Position` but shouldn't
/// have their `Transform` synced (say, a UI-anchored entity with a `Position`
/// for some other reason), that's the moment to add a filter.
fn sync_transforms(mut query: Query<(&Position, &mut Transform)>) {
    for (pos, mut transform) in &mut query {
        transform.translation.x = pos.0.x;
        transform.translation.y = pos.0.y;
    }
}

/// Point the barrel at the cursor, pivoting around the **tank's** centre.
///
/// ## The pivot problem
///
/// The barrel mesh is a rectangle centred at its own origin. To make it
/// *look* like a barrel emerging from the tank, we want it to rotate around
/// the tank's centre — not its own.
///
/// The naive approach sets the barrel's `Transform.translation` once, at
/// `(LENGTH/2, 0, -0.1)`, then just rotates it around that fixed point.
/// That's wrong: at angle 0 it looks fine, but as the barrel rotates the
/// near end swings around `(LENGTH/2, 0)` — the pivot is `(LENGTH/2, 0)`,
/// not `(0, 0)`. The tip then traces a circle offset from the cursor by
/// that amount. That's the "barrel doesn't quite point at the cursor" bug.
///
/// ## The fix
///
/// Each frame, set the barrel's translation so its *near end* lands at the
/// tank centre. Specifically:
///
/// ```text
/// translation = rotate(half_len * (1, 0), angle)
///             = (half_len * cos(angle), half_len * sin(angle))
/// ```
///
/// Combined with `rotation = rotate_z(angle)`, the near-end vertex (local
/// `-L/2` along the barrel's own axis) ends up at world `(0, 0) +
/// tank_pos`, and the far-end vertex at `L` units along `angle`. Both ends
/// are correct at every angle, so the tip is always exactly on the ray from
/// tank centre toward cursor.
///
/// (An alternative — a "pivot" entity with zero translation whose child is
/// the barrel offset by `L/2` — gives the same result with one extra
/// entity. The dynamic-translation approach uses fewer nodes for the same
/// visual.)
///
/// ## Why `z` is untouched here
///
/// `Transform.translation.z` is set once in `setup` (-0.1) to layer the
/// barrel behind the tank, and this system leaves it alone. Orthographic
/// projection means z is only layering, so as long as it stays at -0.1,
/// layering stays correct regardless of angle.
fn rotate_barrel_toward_cursor(
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    positions: Query<&Position>,
    local: Option<Res<LocalPlayer>>,
    mut barrel_q: Query<&mut Transform, With<Barrel>>,
) {
    // Same defensive cascade as `read_fire_input`. Each `let Some(...) else`
    // is a legitimate "this state is momentarily unavailable" case, not an
    // error. Skipping a frame leaves the barrel at its previous angle.
    let Some(local) = local else {
        return;
    };
    let Some(window) = window_q.iter().next() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Some((camera, cam_transform)) = camera_q.iter().next() else {
        return;
    };
    let Ok(world_pos) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };
    let Some(tank_pos) = positions.get(local.0).ok() else {
        return;
    };

    let delta = world_pos - tank_pos.0;
    if delta == Vec2::ZERO {
        // Cursor is exactly on the tank centre — no angle is meaningful.
        // Keep the previous rotation (skip this frame).
        return;
    }

    // `atan2(y, x)` gives the angle from the +x axis, in (-PI, PI]. Bevy's
    // 2D coordinates are y-up (positive y goes up on screen), and
    // `viewport_to_world_2d` returns world coordinates, so this is the
    // angle a user expects: right = 0, up = PI/2, left = PI, down = -PI/2.
    let angle = delta.y.atan2(delta.x);
    let half_len = BARREL_LENGTH / 2.0;

    for mut transform in &mut barrel_q {
        // Rotate the "near-end-at-tank-centre" offset by the current angle
        // (see the function doc — this is the crux of the pivot fix).
        transform.translation.x = angle.cos() * half_len;
        transform.translation.y = angle.sin() * half_len;
        // Preserve z (layering) — do not touch it.
        transform.rotation = Quat::from_rotation_z(angle);
    }
}
