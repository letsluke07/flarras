use bevy::prelude::*;
use lightyear::prelude::server::*;
use shared::ProtocolPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ServerPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / 64.0),
        })
        .add_plugins(ProtocolPlugin)
        .run();
}
