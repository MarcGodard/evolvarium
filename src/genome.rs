// M3: VARIABLE-TOPOLOGY genome. The GA now directs where the NN's input nodes are:
//   sensors = evolvable list of directional food-eyes (count + angle + range).
// Brain resizes to match (dynamic I/O). Hidden size fixed for now; full morphology + env-driven
// growth come later (02/03). Weights kept as structured layers so adding/removing a sensor is clean.
use crate::rng::Rng;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const HIDDEN: usize = 6;
pub const OUTPUTS: usize = 2; // [thrust 0..1, turn -1..1]
pub const NFOOD: usize = 4; // food types (epigenetic diet model, see 12)

pub const MIN_SENSORS: usize = 1;
pub const MAX_SENSORS: usize = 8;
pub const SIG_PER_SENSOR: usize = 2; // each sensor reports [inv-dist, food type/readiness]
pub const GLOBAL_INPUTS: usize = 2; // [energy, bias]
pub const CONE_HALF: f32 = 0.7; // sensor field-of-view half-angle (rad)
const RANGE_MIN: f32 = 4.0;
const RANGE_MAX: f32 = 14.0;

// One directional food-eye. angle = offset from heading; range = how far it sees.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Sensor {
    pub angle: f32,
    pub range: f32,
}

// Layered weights. ih: HIDDEN rows, each (n_in + 1) incl. bias. ho: OUTPUTS rows, each (HIDDEN + 1).
#[derive(Clone, Serialize, Deserialize)]
pub struct Net {
    pub ih: Vec<Vec<f32>>,
    pub ho: Vec<Vec<f32>>,
}

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Genome {
    pub sensors: Vec<Sensor>,
    pub net: Net,        // initial weights (heritable priors)
    pub plast: Net,      // per-weight plasticity 0..1, same shape as net
    pub expr0: [f32; NFOOD], // innate diet-gene expression baseline (see 12)
    pub rigidity: f32,       // 0=flexible generalist .. 1=pinned specialist (koala)
    pub bite: f32,           // 0..1 eating strength vs plant defense (arms race, see 13); costs energy
}

pub fn n_inputs(n_sensors: usize) -> usize {
    n_sensors * SIG_PER_SENSOR + GLOBAL_INPUTS
}

fn random_net(rng: &mut Rng, n_in: usize, plasticity: bool) -> Net {
    let cell = |rng: &mut Rng| if plasticity { rng.f32() * 0.2 } else { rng.range(-1.0, 1.0) };
    let ih = (0..HIDDEN).map(|_| (0..n_in + 1).map(|_| cell(rng)).collect()).collect();
    let ho = (0..OUTPUTS).map(|_| (0..HIDDEN + 1).map(|_| cell(rng)).collect()).collect();
    Net { ih, ho }
}

impl Genome {
    pub fn random(rng: &mut Rng) -> Self {
        let n_sensors = 2 + (rng.f32() * 2.0) as usize; // 2..3 to start
        let sensors = (0..n_sensors)
            .map(|_| Sensor {
                angle: rng.range(-std::f32::consts::PI, std::f32::consts::PI),
                range: rng.range(RANGE_MIN, RANGE_MAX),
            })
            .collect();
        let n_in = n_inputs(n_sensors);
        let mut expr0 = [0.0f32; NFOOD];
        for e in expr0.iter_mut() {
            *e = rng.f32();
        }
        Genome {
            sensors,
            net: random_net(rng, n_in, false),
            plast: random_net(rng, n_in, true),
            expr0,
            rigidity: rng.f32(),
            bite: rng.f32() * 0.5,
        }
    }

    pub fn n_sensors(&self) -> usize {
        self.sensors.len()
    }

    pub fn mutate(&mut self, rng: &mut Rng, rate: f32, std: f32) {
        // weight perturbation
        for row in self.net.ih.iter_mut().chain(self.net.ho.iter_mut()) {
            for x in row.iter_mut() {
                if rng.f32() < rate {
                    *x = (*x + rng.normal() * std).clamp(-5.0, 5.0);
                }
            }
        }
        for row in self.plast.ih.iter_mut().chain(self.plast.ho.iter_mut()) {
            for p in row.iter_mut() {
                if rng.f32() < rate {
                    *p = (*p + rng.normal() * 0.15).clamp(0.0, 1.0);
                }
            }
        }
        // sensor placement
        for s in &mut self.sensors {
            if rng.f32() < rate {
                s.angle = wrap_pi(s.angle + rng.normal() * 0.4);
            }
            if rng.f32() < rate {
                s.range = (s.range + rng.normal() * 1.5).clamp(RANGE_MIN, RANGE_MAX);
            }
        }
        // diet genes
        for e in &mut self.expr0 {
            if rng.f32() < rate {
                *e = (*e + rng.normal() * 0.2).clamp(0.0, 1.0);
            }
        }
        if rng.f32() < rate {
            self.rigidity = (self.rigidity + rng.normal() * 0.2).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.bite = (self.bite + rng.normal() * 0.15).clamp(0.0, 1.0);
        }
        // structural: add / remove a sensor (and the matching input-weight columns)
        if rng.f32() < 0.06 && self.sensors.len() < MAX_SENSORS {
            self.add_sensor(rng);
        }
        if rng.f32() < 0.06 && self.sensors.len() > MIN_SENSORS {
            self.remove_sensor(rng);
        }
    }

    // New sensor's two input columns are inserted right before the GLOBAL_INPUTS (energy, bias).
    fn add_sensor(&mut self, rng: &mut Rng) {
        self.sensors.push(Sensor {
            angle: rng.range(-std::f32::consts::PI, std::f32::consts::PI),
            range: rng.range(RANGE_MIN, RANGE_MAX),
        });
        let insert_at = (self.sensors.len() - 1) * SIG_PER_SENSOR; // before globals
        for row in &mut self.net.ih {
            for k in 0..SIG_PER_SENSOR {
                row.insert(insert_at + k, rng.range(-1.0, 1.0));
            }
        }
        for row in &mut self.plast.ih {
            for k in 0..SIG_PER_SENSOR {
                row.insert(insert_at + k, rng.f32() * 0.2);
            }
        }
    }

    fn remove_sensor(&mut self, rng: &mut Rng) {
        let s = (rng.f32() * self.sensors.len() as f32) as usize % self.sensors.len();
        self.sensors.remove(s);
        let at = s * SIG_PER_SENSOR;
        for row in self.net.ih.iter_mut().chain(self.plast.ih.iter_mut()) {
            row.drain(at..at + SIG_PER_SENSOR);
        }
    }
}

// Forward pass over a Net for a given input vector. Returns hidden activations + outputs.
pub fn forward(net: &Net, input: &[f32]) -> (Vec<f32>, [f32; OUTPUTS]) {
    let n_in = input.len();
    let mut h = vec![0.0f32; net.ih.len()];
    for (hj, row) in h.iter_mut().zip(&net.ih) {
        let mut sum = row[n_in]; // bias
        for i in 0..n_in {
            sum += row[i] * input[i];
        }
        *hj = sum.tanh();
    }
    let mut out = [0.0f32; OUTPUTS];
    for (k, row) in net.ho.iter().enumerate() {
        let mut sum = row[h.len()]; // bias
        for (j, &hv) in h.iter().enumerate() {
            sum += row[j] * hv;
        }
        out[k] = sum;
    }
    out[0] = sigmoid(out[0]); // thrust
    out[1] = out[1].tanh(); // turn
    (h, out)
}

// Reward-modulated Hebbian (see 04). Moves weights only when reward != 0; plasticity scales per-weight.
pub fn learn(net: &mut Net, plast: &Net, input: &[f32], h: &[f32], out: &[f32; OUTPUTS], reward: f32, lr: f32) {
    let n_in = input.len();
    for (ri, row) in net.ih.iter_mut().enumerate() {
        let post = h[ri];
        for i in 0..n_in {
            apply(&mut row[i], plast.ih[ri][i], input[i], post, reward, lr);
        }
        let bi = n_in;
        apply(&mut row[bi], plast.ih[ri][bi], 1.0, post, reward, lr);
    }
    for (ri, row) in net.ho.iter_mut().enumerate() {
        let post = out[ri];
        for (j, &hv) in h.iter().enumerate() {
            apply(&mut row[j], plast.ho[ri][j], hv, post, reward, lr);
        }
        let bi = h.len();
        apply(&mut row[bi], plast.ho[ri][bi], 1.0, post, reward, lr);
    }
}

#[inline]
fn apply(w: &mut f32, plast: f32, pre: f32, post: f32, reward: f32, lr: f32) {
    let dw = reward * (pre * post - 0.1 * *w);
    *w = (*w + plast * lr * dw).clamp(-5.0, 5.0);
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn wrap_pi(a: f32) -> f32 {
    let mut a = a;
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}
