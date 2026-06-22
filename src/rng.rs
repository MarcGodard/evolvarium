// Deterministic PRNG (PCG32). Self-contained + seedable + portable = reproducible runs (key for evolution determinism).
// Placeholder until bevy_rand per-entity streams land. See 09-open-questions, 11-crate-stack.
use bevy::prelude::*;
use std::f32::consts::TAU;

#[derive(Resource)]
pub struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    pub fn seed(seed: u64) -> Self {
        let mut r = Rng { state: 0, inc: (seed << 1) | 1 };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6364136223846793005).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    // Uniform [0,1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32 + 1.0)
    }

    // Uniform [a,b).
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }

    // Standard normal via Box-Muller. Used for NN weight mutation.
    pub fn normal(&mut self) -> f32 {
        let u1 = self.f32().max(1e-7);
        let u2 = self.f32();
        (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
    }
}
