use bevy::prelude::*;
use lightyear::prelude::server::*;
use shared::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ServerPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / TICK_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .run();
}
