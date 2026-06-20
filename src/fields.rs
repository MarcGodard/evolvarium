// Spatial fields. One abstraction powers gravity now, fluid/pressure/nutrients later (see 06).
use bevy::prelude::*;

// Sample a field value at a world position. M0: gravity only, uniform.
// Later: zones, grids, mutable nutrient fields. Keep sample() cheap, it runs in hot loops.
#[derive(Resource)]
pub struct GravityField {
    pub accel: Vec3,
}

impl Default for GravityField {
    fn default() -> Self {
        // Earth-ish down. God-tunable later.
        Self { accel: Vec3::new(0.0, -9.81, 0.0) }
    }
}

impl GravityField {
    // Uniform for M0. Becomes position-dependent (zones) at M4.
    pub fn sample(&self, _pos: Vec3) -> Vec3 {
        self.accel
    }
}
