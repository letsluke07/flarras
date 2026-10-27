use bevy_ecs::prelude::*;
use bevy_math::Vec2;

#[derive(Component, Debug, Clone, Copy)]
pub struct Position(pub Vec2);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Velocity(pub Vec2);

#[derive(Component)]
pub struct Player;

#[derive(Resource, Debug, Clone)]
pub struct GameConfig {
    pub player_speed: f32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            player_speed: 150.0,
        }
    }
} 
