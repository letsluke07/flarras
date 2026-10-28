use bevy_ecs::prelude::*;
use bevy_math::Vec2;

// --- Tuning constants ---

// Player
pub const PLAYER_MAX_HP: f32 = 100.0;

// Tank
pub const TANK_SPEED: f32 = 150.0;
pub const TANK_ACCELERATION: f32 = 1000.0;
pub const TANK_RADIUS: f32 = 30.0;

// Bullets
pub const BULLET_SPEED: f32 = 500.0;
pub const BULLET_LIFETIME: f32 = 2.0;
pub const BULLET_RADIUS: f32 = 6.0;
pub const BULLET_DAMAGE: f32 = 25.0;

// Firing
pub const FIRE_INTERVAL: f32 = 1.0;

// Mobs
pub const MOB_MAX_HP: f32 = 100.0;
pub const MOB_RADIUS: f32 = 20.0;
pub const MOB_SPEED: f32 = 100.0;
pub const MOB_ACCELERATION: f32 = 500.0;
pub const MOB_CONTACT_DAMAGE: f32 = 10.0;
pub const CONTACT_INTERVAL: f32 = 1.0;
pub const MOB_SPAWN_DISTANCE: f32 = 600.0;
pub const MOB_SPAWN_INTERVAL: f32 = 2.0;
pub const MAX_MOBS: usize = 20;

// --- Markers ---

#[derive(Component, Debug, Clone, Copy)]
pub struct Player;

#[derive(Component, Debug, Clone, Copy)]
pub struct Bullet;

#[derive(Component, Debug, Clone, Copy)]
pub struct Mob;

// --- Components ---

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Position(pub Vec2);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Velocity(pub Vec2);

#[derive(Component, Debug, Clone, Copy)]
pub struct Lifetime(pub f32);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FireCooldown {
    pub remaining: f32,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn full(max: f32) -> Self {
        Self { current: max, max }
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }

    pub fn take_damage(&mut self, amount: f32) {
        self.current = (self.current - amount).max(0.0);
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Damage(pub f32);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct ContactCooldown {
    pub remaining: f32,
}

// --- Resources ---

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    Playing,
    Dead,
}

// --- Input ---

#[derive(Debug, Clone, Copy)]
pub enum InputCommand {
    Move { direction: Vec2 },
    Fire { direction: Vec2 },
}
