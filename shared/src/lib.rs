use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

pub const TICK_HZ: f64 = 64.0;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TestPosition(pub Vec2);

pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.component::<TestPosition>().replicate();
    }
}
