use bevy::prelude::*;
use shared::ProtocolPlugin;
use lightyear::prelude::client::*;

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
        .add_plugins(ClientPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / 64.0),
        })
        .add_plugins(ProtocolPlugin)
        .run();
}