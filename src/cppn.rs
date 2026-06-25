// CPPN/HyperNEAT indirect brain encoding (opt-in, alongside the direct-encoded net in genome.rs). A small
// FIXED-topology pattern net maps a pair of neuron substrate coords -> one connection weight. genome::develop_brain
// queries it across the body-geometry substrate to PAINT net.ih/net.ho; forward()/learn() then run unchanged.
// Why: regularity (symmetry/repetition/gradients) + tiny genomes + a mutated body/sensor inherits CPPN-painted
// (not random-padded) control -> the morphology-coupling fix (P3). Activations FIXED at birth; GA evolves weights.
use crate::rng::Rng;
use bevy::prelude::Vec3;
use serde::{Deserialize, Serialize};

const HIDDEN: usize = 10; // units per CPPN hidden layer
const WEIGHT_SCALE: f32 = 3.0; // CPPN out -> connection weight (into the direct net's useful ~-5..5 band)
const WEIGHT_CLAMP: f32 = 8.0; // clamp painted weight (unbounded activations like abs can't blow forward() up)

// Per-unit activation. The MIX is the CPPN signature: sin -> repetition, gauss -> bumps/symmetry, abs ->
// ridges, tanh/sigmoid -> smooth gradients, ident -> linear pass.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
pub enum Act {
    Sin,
    Gauss,
    Tanh,
    Abs,
    Sigmoid,
    Ident,
}

impl Act {
    fn apply(self, x: f32) -> f32 {
        match self {
            Act::Sin => (x * 2.0).sin(),
            Act::Gauss => (-(x * x)).exp(),
            Act::Tanh => x.tanh(),
            Act::Abs => x.abs().min(3.0),
            Act::Sigmoid => 1.0 / (1.0 + (-x).exp()),
            Act::Ident => x.clamp(-3.0, 3.0),
        }
    }
    fn from_u(u: u32) -> Act {
        match u % 6 {
            0 => Act::Sin,
            1 => Act::Gauss,
            2 => Act::Tanh,
            3 => Act::Abs,
            4 => Act::Sigmoid,
            _ => Act::Ident,
        }
    }
}

// Dense layer: w[out][in] + per-out bias + per-out activation.
#[derive(Clone, Serialize, Deserialize)]
struct Layer {
    w: Vec<Vec<f32>>,
    b: Vec<f32>,
    act: Vec<Act>,
}

// Fixed topology: 7 inputs (a.xyz, b.xyz, |a-b|) -> HIDDEN -> HIDDEN -> 1 weight.
#[derive(Clone, Serialize, Deserialize)]
pub struct Cppn {
    layers: Vec<Layer>,
}

const DIMS: [usize; 4] = [7, HIDDEN, HIDDEN, 1];

impl Cppn {
    pub fn random(rng: &mut Rng) -> Cppn {
        let mut layers = Vec::with_capacity(DIMS.len() - 1);
        for l in 0..DIMS.len() - 1 {
            let (nin, nout) = (DIMS[l], DIMS[l + 1]);
            let w = (0..nout).map(|_| (0..nin).map(|_| rng.range(-1.5, 1.5)).collect()).collect();
            let b = (0..nout).map(|_| rng.range(-0.5, 0.5)).collect();
            // output layer = ident (linear weight readout); hidden layers = varied activations
            let last = l == DIMS.len() - 2;
            let act = (0..nout).map(|_| if last { Act::Ident } else { Act::from_u(rng.next_u32()) }).collect();
            layers.push(Layer { w, b, act });
        }
        Cppn { layers }
    }

    // Pure + deterministic (no RNG): same cppn + coords -> same weight. Required for reproducible develop_brain.
    pub fn query(&self, a: Vec3, b: Vec3) -> f32 {
        let mut v = vec![a.x, a.y, a.z, b.x, b.y, b.z, (a - b).length()];
        for layer in &self.layers {
            let mut out = vec![0.0f32; layer.b.len()];
            for (o, slot) in out.iter_mut().enumerate() {
                let mut s = layer.b[o];
                for (i, &vi) in v.iter().enumerate() {
                    s += layer.w[o][i] * vi;
                }
                *slot = layer.act[o].apply(s);
            }
            v = out;
        }
        (v[0] * WEIGHT_SCALE).clamp(-WEIGHT_CLAMP, WEIGHT_CLAMP)
    }

    pub fn mutate(&mut self, rng: &mut Rng, rate: f32, std: f32) {
        for layer in &mut self.layers {
            for row in &mut layer.w {
                for x in row.iter_mut() {
                    if rng.f32() < rate {
                        *x = (*x + rng.normal() * std).clamp(-6.0, 6.0);
                    }
                }
            }
            for x in layer.b.iter_mut() {
                if rng.f32() < rate {
                    *x = (*x + rng.normal() * std).clamp(-6.0, 6.0);
                }
            }
        }
    }

    // Cell-aligned per-weight crossover (fixed topology -> shapes match, unlike the variable-topology direct net).
    pub fn crossover(a: &Cppn, b: &Cppn, rng: &mut Rng) -> Cppn {
        let mut c = a.clone();
        if a.layers.len() != b.layers.len() {
            return c; // defensive: topology mismatch -> just inherit a
        }
        for (lc, lb) in c.layers.iter_mut().zip(&b.layers) {
            for (rc, rb) in lc.w.iter_mut().zip(&lb.w) {
                for (xc, xb) in rc.iter_mut().zip(rb) {
                    if rng.f32() < 0.5 {
                        *xc = *xb;
                    }
                }
            }
            for (xc, xb) in lc.b.iter_mut().zip(&lb.b) {
                if rng.f32() < 0.5 {
                    *xc = *xb;
                }
            }
            // activations kept from a (identical fixed topology)
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_deterministic_and_finite() {
        let mut rng = Rng::seed(3);
        let c = Cppn::random(&mut rng);
        let a = Vec3::new(0.3, -0.2, 0.7);
        let b = Vec3::new(-0.5, 0.1, 0.0);
        assert_eq!(c.query(a, b), c.query(a, b), "same coords -> same weight");
        // sweep a grid: every painted weight finite + within clamp
        for i in 0..7 {
            for j in 0..7 {
                let p = Vec3::new(i as f32 / 3.0 - 1.0, j as f32 / 3.0 - 1.0, 0.2);
                let w = c.query(p, a);
                assert!(w.is_finite() && w.abs() <= WEIGHT_CLAMP + 1e-3, "weight {w} out of range");
            }
        }
    }

    #[test]
    fn serde_round_trip_preserves_weights() {
        let mut rng = Rng::seed(9);
        let c = Cppn::random(&mut rng);
        let json = serde_json::to_string(&c).unwrap();
        let c2: Cppn = serde_json::from_str(&json).unwrap();
        let (a, b) = (Vec3::new(0.1, 0.2, -0.3), Vec3::new(0.4, -0.5, 0.6));
        assert_eq!(c.query(a, b), c2.query(a, b), "round-trip must paint identical weights");
    }

    #[test]
    fn distinct_coords_give_varied_weights() {
        let mut rng = Rng::seed(21);
        let c = Cppn::random(&mut rng);
        let base = Vec3::ZERO;
        let w0 = c.query(base, Vec3::new(0.9, 0.0, 0.0));
        let w1 = c.query(base, Vec3::new(-0.9, 0.0, 0.0));
        assert!((w0 - w1).abs() > 1e-4, "a non-degenerate CPPN should vary across the substrate");
    }
}
