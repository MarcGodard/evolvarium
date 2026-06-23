// Deterministic PRNG (PCG32). Self-contained + seedable + portable = reproducible runs (key for evolution determinism).
// Placeholder until bevy_rand per-entity streams land. See 09-open-questions, 11-crate-stack.
use bevy::prelude::*;
use std::f32::consts::TAU;

#[derive(Resource)]
pub struct Rng {
    state: u64,
    inc: u64,
}

// splitmix64 finalizer: strong avalanche so adjacent (entity_index, tick) values map to uncorrelated seeds.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn seed(seed: u64) -> Self {
        let mut r = Rng { state: 0, inc: (seed << 1) | 1 };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    // Per-entity deterministic stream (PARALLELIZATION.md). Seeds a fresh PCG from hash(run_seed,
    // entity.index, tick) so a parallel decide phase draws order-independently: same (seed,index,tick) ->
    // same stream regardless of thread scheduling -> run-to-run reproducible. entity.index is a stable,
    // deterministic per-entity id given our deterministic spawn/despawn order (no extra component needed).
    pub fn for_entity(seed: u64, index: u32, tick: u32) -> Self {
        let mut h = splitmix64(seed);
        h = splitmix64(h ^ index as u64);
        h = splitmix64(h ^ tick as u64);
        Rng::seed(h)
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

#[cfg(test)]
mod tests {
    use super::*;

    // for_entity: deterministic (same inputs -> same stream) and decorrelated across index/tick.
    #[test]
    fn for_entity_deterministic_and_distinct() {
        let a = Rng::for_entity(7, 100, 42).f32();
        let b = Rng::for_entity(7, 100, 42).f32();
        assert_eq!(a, b, "same (seed,index,tick) must reproduce");

        // adjacent index + adjacent tick + different seed all diverge (no obvious correlation)
        assert_ne!(a, Rng::for_entity(7, 101, 42).f32());
        assert_ne!(a, Rng::for_entity(7, 100, 43).f32());
        assert_ne!(a, Rng::for_entity(8, 100, 42).f32());
    }

    // Per-entity streams stay roughly uniform: mean of many entities' first draw near 0.5.
    #[test]
    fn for_entity_first_draw_uniform() {
        let n = 4000u32;
        let mean: f32 = (0..n).map(|i| Rng::for_entity(1, i, 5).f32()).sum::<f32>() / n as f32;
        assert!((mean - 0.5).abs() < 0.03, "mean {mean} not ~0.5");
    }
}
