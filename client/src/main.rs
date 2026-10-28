use bevy::{prelude::*, window::PrimaryWindow};
use shared::*;
use sim::*;
use std::sync::atomic::{AtomicU64, Ordering};

// --- Visual constants ---

const BACKGROUND_COLOR: Color = Color::srgb_u8(0xdb, 0xdb, 0xdb);
const TANK_COLOR: Color = Color::srgb_u8(0x3c, 0xa4, 0xcb);
const BARREL_COLOR: Color = Color::srgb_u8(0xa7, 0xa7, 0xa7);
const BULLET_COLOR: Color = TANK_COLOR;
const MOB_COLOR: Color = Color::srgb_u8(0xc0, 0x39, 0x2b);
const HP_BG_COLOR: Color = Color::srgb_u8(0x33, 0x33, 0x33);
const HP_FG_COLOR: Color = Color::srgb_u8(0x4c, 0xaf, 0x50);

const BARREL_LENGTH: f32 = 40.0;
const BARREL_WIDTH: f32 = 8.0;
const BORDER_THICKNESS: f32 = 3.0;
const BORDER_MIX: f32 = 0.3;

const HP_BAR_HEIGHT: f32 = 4.0;
const HP_BAR_OFFSET_ABOVE: f32 = 10.0;
const MOB_HP_BAR_WIDTH: f32 = MOB_RADIUS * 2.0;
const PLAYER_HP_BAR_WIDTH: f32 = TANK_RADIUS * 2.0;

// --- Resources ---

#[derive(Resource, Default)]
struct PendingInput(Vec<(Entity, InputCommand)>);

#[derive(Resource)]
struct LocalPlayer(Entity);

#[derive(Resource)]
struct SimTiming(sim::SimClock);

#[derive(Resource)]
struct GameAssets {
    tank_body_mesh: Handle<Mesh>,
    tank_body_material: Handle<ColorMaterial>,
    tank_border_mesh: Handle<Mesh>,
    tank_border_material: Handle<ColorMaterial>,
    barrel_mesh: Handle<Mesh>,
    barrel_material: Handle<ColorMaterial>,
    barrel_border_mesh: Handle<Mesh>,
    barrel_border_material: Handle<ColorMaterial>,
    bullet_mesh: Handle<Mesh>,
    bullet_material: Handle<ColorMaterial>,
    bullet_border_mesh: Handle<Mesh>,
    bullet_border_material: Handle<ColorMaterial>,
    mob_body_mesh: Handle<Mesh>,
    mob_body_material: Handle<ColorMaterial>,
    mob_border_mesh: Handle<Mesh>,
    mob_border_material: Handle<ColorMaterial>,
    bar_unit_mesh: Handle<Mesh>,
    bar_bg_material: Handle<ColorMaterial>,
    bar_fg_material: Handle<ColorMaterial>,
}

// --- Components ---

#[derive(Component)]
struct Barrel;

#[derive(Component)]
struct HealthBarForeground {
    target: Entity,
    width: f32,
}

#[derive(Component)]
struct GameOverOverlay;

// --- App ---

fn main() {
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
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
                restart_game,
                run_sim,
                show_game_over,
                spawn_bullet_views,
                spawn_mob_views,
                sync_transforms,
                update_health_bars,
                rotate_barrel_toward_cursor,
                camera_follow,
            )
                .chain(),
        )
        .run();
}

// --- Setup ---

fn setup(world: &mut World) {
    world.spawn(Camera2d);

    let assets = create_game_assets(world);
    world.insert_resource(assets);

    let entity = spawn_player_entity(world);
    world.insert_resource(LocalPlayer(entity));

    world.insert_resource(GameState::Playing);
    world.insert_resource(SimRng::seed(fresh_seed()));
    world.insert_resource(MobSpawnTimer::default());
}

fn create_game_assets(world: &mut World) -> GameAssets {
    let (
        tank_body_mesh,
        tank_border_mesh,
        barrel_mesh,
        barrel_border_mesh,
        bullet_mesh,
        bullet_border_mesh,
        mob_body_mesh,
        mob_border_mesh,
        bar_unit_mesh,
    ) = {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        (
            meshes.add(Circle::new(TANK_RADIUS)),
            meshes.add(Circle::new(TANK_RADIUS).to_ring(BORDER_THICKNESS)),
            meshes.add(Rectangle::new(BARREL_LENGTH, BARREL_WIDTH)),
            meshes.add(Rectangle::new(BARREL_LENGTH, BARREL_WIDTH).to_ring(BORDER_THICKNESS)),
            meshes.add(Circle::new(BULLET_RADIUS)),
            meshes.add(Circle::new(BULLET_RADIUS).to_ring(BORDER_THICKNESS)),
            meshes.add(Circle::new(MOB_RADIUS)),
            meshes.add(Circle::new(MOB_RADIUS).to_ring(BORDER_THICKNESS)),
            meshes.add(Rectangle::new(1.0, 1.0)),
        )
    };

    let (
        tank_body_material,
        tank_border_material,
        barrel_material,
        barrel_border_material,
        bullet_material,
        bullet_border_material,
        mob_body_material,
        mob_border_material,
        bar_bg_material,
        bar_fg_material,
    ) = {
        let mut materials = world.resource_mut::<Assets<ColorMaterial>>();
        (
            materials.add(TANK_COLOR),
            materials.add(TANK_COLOR.mix(&Color::BLACK, BORDER_MIX)),
            materials.add(BARREL_COLOR),
            materials.add(BARREL_COLOR.mix(&Color::BLACK, BORDER_MIX)),
            materials.add(BULLET_COLOR),
            materials.add(BULLET_COLOR.mix(&Color::BLACK, BORDER_MIX)),
            materials.add(MOB_COLOR),
            materials.add(MOB_COLOR.mix(&Color::BLACK, BORDER_MIX)),
            materials.add(HP_BG_COLOR),
            materials.add(HP_FG_COLOR),
        )
    };

    GameAssets {
        tank_body_mesh,
        tank_body_material,
        tank_border_mesh,
        tank_border_material,
        barrel_mesh,
        barrel_material,
        barrel_border_mesh,
        barrel_border_material,
        bullet_mesh,
        bullet_material,
        bullet_border_mesh,
        bullet_border_material,
        mob_body_mesh,
        mob_body_material,
        mob_border_mesh,
        mob_border_material,
        bar_unit_mesh,
        bar_bg_material,
        bar_fg_material,
    }
}

fn spawn_player_entity(world: &mut World) -> Entity {
    let a = {
        let x = world.resource::<GameAssets>();
        (
            x.tank_body_mesh.clone(),
            x.tank_body_material.clone(),
            x.tank_border_mesh.clone(),
            x.tank_border_material.clone(),
            x.barrel_mesh.clone(),
            x.barrel_material.clone(),
            x.barrel_border_mesh.clone(),
            x.barrel_border_material.clone(),
            x.bar_unit_mesh.clone(),
            x.bar_bg_material.clone(),
            x.bar_fg_material.clone(),
        )
    };
    let (
        body_mesh,
        body_mat,
        border_mesh,
        border_mat,
        barrel_mesh,
        barrel_mat,
        barrel_border_mesh,
        barrel_border_mat,
        bar_mesh,
        bar_bg_mat,
        bar_fg_mat,
    ) = a;

    let mut em = world.spawn((
        Player,
        Position(Vec2::ZERO),
        Velocity(Vec2::ZERO),
        FireCooldown::default(),
        Health::full(PLAYER_MAX_HP),
        Mesh2d(body_mesh),
        MeshMaterial2d(body_mat),
        Transform::default(),
    ));
    let entity = em.id();

    em.with_children(|parent| {
        parent.spawn((Mesh2d(border_mesh), MeshMaterial2d(border_mat)));

        parent
            .spawn((
                Barrel,
                Mesh2d(barrel_mesh),
                MeshMaterial2d(barrel_mat),
                Transform::from_xyz(BARREL_LENGTH / 2.0, 0.0, -0.1),
            ))
            .with_children(|b| {
                b.spawn((
                    Mesh2d(barrel_border_mesh),
                    MeshMaterial2d(barrel_border_mat),
                    Transform::from_xyz(0.0, 0.0, 0.05),
                ));
            });

        parent.spawn((
            Mesh2d(bar_mesh.clone()),
            MeshMaterial2d(bar_bg_mat.clone()),
            Transform::from_xyz(0.0, TANK_RADIUS + HP_BAR_OFFSET_ABOVE, 0.1).with_scale(Vec3::new(
                PLAYER_HP_BAR_WIDTH,
                HP_BAR_HEIGHT,
                1.0,
            )),
        ));

        parent.spawn((
            HealthBarForeground {
                target: entity,
                width: PLAYER_HP_BAR_WIDTH,
            },
            Mesh2d(bar_mesh),
            MeshMaterial2d(bar_fg_mat),
            Transform::from_xyz(0.0, TANK_RADIUS + HP_BAR_OFFSET_ABOVE, 0.11)
                .with_scale(Vec3::new(PLAYER_HP_BAR_WIDTH, HP_BAR_HEIGHT, 1.0)),
        ));
    });

    entity
}

// --- Input ---

fn read_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    local: Option<Res<LocalPlayer>>,
    mut pending: ResMut<PendingInput>,
) {
    let Some(local) = local else {
        return;
    };

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

    pending.0.clear();
    pending.0.push((local.0, InputCommand::Move { direction }));
}

fn read_fire_input(
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<GameState>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    positions: Query<&Position>,
    local: Option<Res<LocalPlayer>>,
    mut pending: ResMut<PendingInput>,
) {
    if *state != GameState::Playing {
        return;
    }
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
    let Ok(world_pos) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };

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

// --- Restart ---

fn restart_game(world: &mut World) {
    let should_restart = {
        let is_dead = world
            .get_resource::<GameState>()
            .map(|s| *s == GameState::Dead)
            .unwrap_or(false);
        let clicked = world
            .get_resource::<ButtonInput<MouseButton>>()
            .map(|m| m.just_pressed(MouseButton::Left))
            .unwrap_or(false);
        is_dead && clicked
    };
    if !should_restart {
        return;
    }

    let mut q = world.query_filtered::<Entity, Or<(With<Player>, With<Mob>, With<Bullet>)>>();
    let to_despawn: Vec<Entity> = q.iter(world).collect();
    for e in to_despawn {
        let _ = world.despawn(e);
    }

    let mut q = world.query_filtered::<Entity, With<GameOverOverlay>>();
    let to_despawn: Vec<Entity> = q.iter(world).collect();
    for e in to_despawn {
        let _ = world.despawn(e);
    }

    world.insert_resource(GameState::Playing);
    world.insert_resource(SimRng::seed(fresh_seed()));
    world.insert_resource(MobSpawnTimer::default());
    if let Some(mut pending) = world.get_resource_mut::<PendingInput>() {
        pending.0.clear();
    }

    let entity = spawn_player_entity(world);
    world.insert_resource(LocalPlayer(entity));
}

// --- Sim bridge ---

fn run_sim(world: &mut World) {
    let frame_dt = world.resource::<Time>().delta_secs();

    let inputs = {
        let mut pending = world.resource_mut::<PendingInput>();
        std::mem::take(&mut pending.0)
    };

    let (tick_count, step) = {
        let mut timing = world.resource_mut::<SimTiming>();
        let n = timing.0.consume(frame_dt);
        (n, timing.0.step())
    };

    for _ in 0..tick_count {
        sim::tick(world, &inputs, step);
    }
}

// --- Rendering ---

fn show_game_over(
    state: Res<GameState>,
    existing: Query<(), With<GameOverOverlay>>,
    mut commands: Commands,
) {
    if *state != GameState::Dead || !existing.is_empty() {
        return;
    }
    commands.spawn((
        GameOverOverlay,
        Text2d::new("You died!\nClick to restart"),
        TextFont::from_font_size(48.0),
        TextColor(Color::BLACK),
        TextLayout::justify(Justify::Center),
        Transform::from_xyz(0.0, 0.0, 100.0),
    ));
}

fn spawn_bullet_views(
    mut commands: Commands,
    assets: Res<GameAssets>,
    new_bullets: Query<(Entity, &Position), (With<Bullet>, Without<Mesh2d>)>,
) {
    for (entity, pos) in &new_bullets {
        commands
            .entity(entity)
            .insert((
                Mesh2d(assets.bullet_mesh.clone()),
                MeshMaterial2d(assets.bullet_material.clone()),
                Transform::from_xyz(pos.0.x, pos.0.y, 0.0),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Mesh2d(assets.bullet_border_mesh.clone()),
                    MeshMaterial2d(assets.bullet_border_material.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.05),
                ));
            });
    }
}

fn spawn_mob_views(
    mut commands: Commands,
    assets: Res<GameAssets>,
    new_mobs: Query<(Entity, &Position), (With<Mob>, Without<Mesh2d>)>,
) {
    for (entity, pos) in &new_mobs {
        commands
            .entity(entity)
            .insert((
                Mesh2d(assets.mob_body_mesh.clone()),
                MeshMaterial2d(assets.mob_body_material.clone()),
                Transform::from_xyz(pos.0.x, pos.0.y, 0.0),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Mesh2d(assets.mob_border_mesh.clone()),
                    MeshMaterial2d(assets.mob_border_material.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.05),
                ));
                parent.spawn((
                    Mesh2d(assets.bar_unit_mesh.clone()),
                    MeshMaterial2d(assets.bar_bg_material.clone()),
                    Transform::from_xyz(0.0, MOB_RADIUS + HP_BAR_OFFSET_ABOVE, 0.1)
                        .with_scale(Vec3::new(MOB_HP_BAR_WIDTH, HP_BAR_HEIGHT, 1.0)),
                ));
                parent.spawn((
                    HealthBarForeground {
                        target: entity,
                        width: MOB_HP_BAR_WIDTH,
                    },
                    Mesh2d(assets.bar_unit_mesh.clone()),
                    MeshMaterial2d(assets.bar_fg_material.clone()),
                    Transform::from_xyz(0.0, MOB_RADIUS + HP_BAR_OFFSET_ABOVE, 0.11)
                        .with_scale(Vec3::new(MOB_HP_BAR_WIDTH, HP_BAR_HEIGHT, 1.0)),
                ));
            });
    }
}

fn sync_transforms(mut query: Query<(&Position, &mut Transform)>) {
    for (pos, mut transform) in &mut query {
        transform.translation.x = pos.0.x;
        transform.translation.y = pos.0.y;
    }
}

fn update_health_bars(
    healths: Query<&Health>,
    mut bars: Query<(&HealthBarForeground, &mut Transform)>,
) {
    for (bar, mut tf) in &mut bars {
        if let Ok(hp) = healths.get(bar.target) {
            let ratio = (hp.current / hp.max).clamp(0.0, 1.0);
            tf.scale.x = ratio * bar.width;
            tf.translation.x = -(bar.width * 0.5) * (1.0 - ratio);
        }
    }
}

fn rotate_barrel_toward_cursor(
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    positions: Query<&Position>,
    local: Option<Res<LocalPlayer>>,
    mut barrel_q: Query<&mut Transform, With<Barrel>>,
) {
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
        return;
    }
    let angle = delta.y.atan2(delta.x);
    let half_len = BARREL_LENGTH / 2.0;

    for mut transform in &mut barrel_q {
        transform.translation.x = angle.cos() * half_len;
        transform.translation.y = angle.sin() * half_len;
        transform.rotation = Quat::from_rotation_z(angle);
    }
}

fn camera_follow(
    local: Option<Res<LocalPlayer>>,
    positions: Query<&Position>,
    mut cameras: Query<&mut Transform, (With<Camera2d>, Without<Position>)>,
) {
    let Some(local) = local else {
        return;
    };
    let Ok(player_pos) = positions.get(local.0) else {
        return;
    };
    for mut cam in &mut cameras {
        cam.translation.x = player_pos.0.x;
        cam.translation.y = player_pos.0.y;
    }
}

// --- Utilities ---

fn fresh_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(42);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}
