use bevy_ecs::prelude::*;
use shared::*;

#[derive(Resource, Debug, Clone, Copy)]
pub struct SimTime {
    pub dt: f32,
}

impl Default for SimTime {
    fn default() -> Self {
        Self { dt: 1.0 / 64.0 }
    }
}

pub fn movement_system(time: Res<SimTime>, mut query: Query<(&mut Position, &Velocity)>) {
    for (mut pos, vel) in query.iter_mut() {
        pos.0 += vel.0 * time.dt;
    }
}

pub fn build_sim_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(movement_system);
    schedule
}
