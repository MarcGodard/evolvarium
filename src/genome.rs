// M3: VARIABLE-TOPOLOGY genome. The GA now directs where the NN's input nodes are:
//   sensors = evolvable list of directional food-eyes (count + angle + range).
// Brain resizes to match (dynamic I/O). Hidden-layer size is ALSO evolvable now (add/remove_hidden);
// per-neuron upkeep (BRAIN_COST) is the trade-off. Weights kept as structured layers so growing/
// shrinking a sensor or a hidden neuron is clean.
use crate::rng::Rng;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const MIN_HIDDEN: usize = 2; // brains never shrink below this
pub const MAX_HIDDEN: usize = 16; // ...nor grow beyond (bounds the per-neuron upkeep cost)
pub const OUTPUTS: usize = 2; // [thrust 0..1, turn -1..1]
pub const NFOOD: usize = 4; // plant FAMILY count (sensing hue + kind label only; NOT the metabolic axis)
pub const NUTRIENTS: usize = 10; // distinct nutrients (the metabolic axis: regulatory uptake genome, see 14/05)

pub const MIN_SENSORS: usize = 1;
pub const MAX_SENSORS: usize = 8;
pub const SIG_PER_SENSOR: usize = 2; // each sensor reports [inv-dist, food type/readiness]
pub const GLOBAL_INPUTS: usize = 4; // [energy, daylight, fatigue, bias] -- light+fatigue let brains evolve diurnal/nocturnal rest
pub const CONE_HALF: f32 = 0.7; // sensor field-of-view half-angle (rad)
const RANGE_MIN: f32 = 4.0;
const RANGE_MAX: f32 = 48.0; // long-range vision is possible (big world); its energy cost is the trade-off (see sim SENSE_COST)

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
    #[serde(default = "default_uptake")]
    pub uptake: [f32; NUTRIENTS], // gene i = absorption affinity + DEMAND for nutrient i. The "10 genes":
                                  // many high = generalist (needs varied diet + costly machinery); few = specialist
                                  // (cheap, needs little, but fragile if that nutrient's food vanishes). They feed
                                  // the master expression gene (computed per-life from reserves x uptake, see DietState).
    pub rigidity: f32,       // 0=flexible generalist .. 1=pinned specialist (koala)
    pub bite: f32,           // 0..1 eating strength vs plant defense (arms race, see 13); costs energy
    #[serde(default)]
    pub height: f32,         // 0..1 body height/reach: tall reaches fruit trees but costs upkeep (no free lunch)
    #[serde(default = "half")]
    pub light_pref: f32,     // 0=night .. 1=full sun; being far from preferred light costs energy (diurnal/nocturnal niche)
    #[serde(default = "third")]
    pub size: f32,           // 0..1 body mass: more energy store + combat power, but higher basal + move upkeep
    #[serde(default)]
    pub swim: f32,           // 0..1 aquatic adaptation: fast + cheap in water/wet lowland, clumsy + costly on dry land
    #[serde(default)]
    pub social: f32,         // 0..1 herd instinct: near genetic KIN = predation safety (vigilance); ISOLATED = loneliness energy drain. Drives flocking + speciation; punishes the lone cannibal.
    #[serde(default = "half")]
    pub temp_pref: f32,      // 0=cold-adapted (poles) .. 1=warm-adapted (equator). Local temp far from this costs energy. Drives a LATITUDINAL niche: poles harsh but uncrowded, equator mild but contested (no free lunch).
    #[serde(default = "half")]
    pub longevity: f32,      // 0..1 life-history axis: high = long life (aging slows) but higher basal upkeep; low = short fast life, cheap to run. Default 0.5 = current lifespan + no extra cost (so old saves are unchanged).
    #[serde(default = "half")]
    pub metab: f32,          // 0..1 metabolic tempo: high = frugal (cheaper basal) but sluggish (slower top speed); low = fast (higher top speed) but costly to run. Default 0.5 = neutral (no change), so old saves are unchanged.
    #[serde(default = "half")]
    pub parental: f32,       // 0..1 r/K life-history: 0 = r-strategist (breed young + cheap + many small fragile young), 1 = K-strategist (breed late + costly + few well-provisioned young). Scales repro threshold/cost/birth-energy/maturity. Default 0.5 = current values (neutral), so old saves are unchanged.
    #[serde(default = "zero")]
    pub alpine: f32,         // 0..1 mountain adaptation: high = cheap rock/highland crossing (climber) but a heavy-build penalty on flat ground; low = lowland-light. Mirror of swim for mountains. Default 0 = neutral (no relief, no penalty), so old saves are unchanged.
    #[serde(default = "half")]
    pub adiposity: f32,      // 0..1 fat-storage strategy: high = big fat reserve + easy storage (survives famine) but sluggish (fat mobilizes slow) + carrying-fat upkeep; low = lean/nimble + cheap but famine-fragile. Default 0.5 = baseline.
}

// serde defaults for traits absent in old saves
fn half() -> f32 {
    0.5
}
fn third() -> f32 {
    0.33
}
fn zero() -> f32 {
    0.0
}
// serde default for uptake on saves predating the nutrient genome: a mid generalist (all nutrients
// absorbed moderately) so old creatures load as functional omnivores.
fn default_uptake() -> [f32; NUTRIENTS] {
    [0.5; NUTRIENTS]
}

// Master digestion expression: the single gene the 10 uptake genes feed. A gene that absorbs nutrient i
// also DEMANDS it (demand = uptake_i). Expression = demand-weighted mean of how stocked each demanded
// reserve is (reserves_i vs RESERVE_REQ). High when every demanded nutrient is in stock -> rewards a diet
// that covers what the creature is built to use. Floored so a creature is never fully shut off. No uptake
// at all -> falls to the floor (an undifferentiated gut digests poorly). Gates energy extraction (see sim).
pub fn master_expression(uptake: &[f32; NUTRIENTS], reserves: &[f32; NUTRIENTS], req: f32, floor: f32) -> f32 {
    let mut wsum = 0.0;
    let mut sat = 0.0;
    for i in 0..NUTRIENTS {
        let demand = uptake[i];
        wsum += demand;
        sat += demand * (reserves[i] / req).min(1.0); // satisfaction of this demand, capped at 1
    }
    if wsum < 1e-3 {
        return floor; // no uptake genes -> undifferentiated gut -> poor baseline digestion
    }
    (sat / wsum).max(floor)
}

pub fn n_inputs(n_sensors: usize) -> usize {
    n_sensors * SIG_PER_SENSOR + GLOBAL_INPUTS
}

fn random_net(rng: &mut Rng, n_in: usize, n_hidden: usize, plasticity: bool) -> Net {
    let cell = |rng: &mut Rng| if plasticity { rng.f32() * 0.2 } else { rng.range(-1.0, 1.0) };
    let ih = (0..n_hidden).map(|_| (0..n_in + 1).map(|_| cell(rng)).collect()).collect();
    let ho = (0..OUTPUTS).map(|_| (0..n_hidden + 1).map(|_| cell(rng)).collect()).collect();
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
        let n_hidden = 3 + (rng.f32() * 4.0) as usize; // 3..6 to start; evolves via add/remove_hidden
        // uptake: sparse-ish founders -> ~1/3 of nutrients absorbed strongly, rest weakly, so founders
        // span specialists..partial-generalists and selection can broaden or narrow the gut.
        let mut uptake = [0.0f32; NUTRIENTS];
        for u in uptake.iter_mut() {
            *u = if rng.f32() < 0.35 { rng.f32() } else { rng.f32() * 0.2 };
        }
        Genome {
            sensors,
            net: random_net(rng, n_in, n_hidden, false),
            plast: random_net(rng, n_in, n_hidden, true),
            uptake,
            rigidity: rng.f32(),
            bite: rng.f32() * 0.5,
            height: rng.f32() * 0.5,
            light_pref: rng.f32(),
            size: rng.range(0.2, 0.6),
            swim: rng.f32() * 0.3,
            social: rng.f32(),
            temp_pref: rng.f32(), // founders span cold..warm preferences -> spread across latitudes
            longevity: rng.f32(),
            metab: rng.f32(),
            parental: rng.f32(),
            alpine: rng.f32(), // founders span lowland..mountain builds -> a highland niche can emerge
            adiposity: rng.f32(), // founders span lean..fatty storage strategies
        }
    }

    pub fn n_sensors(&self) -> usize {
        self.sensors.len()
    }

    // Two-parent recombination (--mating mode). Body STRUCTURE (sensors + brain net/plast) comes from parent
    // `a` (variable-topology nets can't be crossed cell-by-cell), while the scalar trait genes + diet
    // expression are uniform-crossed from both parents. The caller mutates the result. With assortative
    // mate choice (only similar kin mate) this gives reproductive isolation -> speciation.
    pub fn crossover(a: &Genome, b: &Genome, rng: &mut Rng) -> Genome {
        let pick = |rng: &mut Rng, x: f32, y: f32| if rng.f32() < 0.5 { x } else { y };
        let mut c = a.clone();
        c.rigidity = pick(rng, a.rigidity, b.rigidity);
        c.bite = pick(rng, a.bite, b.bite);
        c.height = pick(rng, a.height, b.height);
        c.light_pref = pick(rng, a.light_pref, b.light_pref);
        c.size = pick(rng, a.size, b.size);
        c.swim = pick(rng, a.swim, b.swim);
        c.social = pick(rng, a.social, b.social);
        c.temp_pref = pick(rng, a.temp_pref, b.temp_pref);
        c.longevity = pick(rng, a.longevity, b.longevity);
        c.metab = pick(rng, a.metab, b.metab);
        c.parental = pick(rng, a.parental, b.parental);
        c.alpine = pick(rng, a.alpine, b.alpine);
        c.adiposity = pick(rng, a.adiposity, b.adiposity);
        for i in 0..NUTRIENTS {
            c.uptake[i] = pick(rng, a.uptake[i], b.uptake[i]);
        }
        c
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
                s.range = (s.range + rng.normal() * 3.5).clamp(RANGE_MIN, RANGE_MAX);
            }
        }
        // nutrient-uptake genes (the regulatory gut)
        for u in &mut self.uptake {
            if rng.f32() < rate {
                *u = (*u + rng.normal() * 0.15).clamp(0.0, 1.0);
            }
        }
        if rng.f32() < rate {
            self.rigidity = (self.rigidity + rng.normal() * 0.2).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.bite = (self.bite + rng.normal() * 0.15).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.height = (self.height + rng.normal() * 0.15).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.light_pref = (self.light_pref + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.size = (self.size + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.swim = (self.swim + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.social = (self.social + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.temp_pref = (self.temp_pref + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.longevity = (self.longevity + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.metab = (self.metab + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.parental = (self.parental + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.alpine = (self.alpine + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.adiposity = (self.adiposity + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        // structural: add / remove a sensor (and the matching input-weight columns)
        if rng.f32() < 0.06 && self.sensors.len() < MAX_SENSORS {
            self.add_sensor(rng);
        }
        if rng.f32() < 0.06 && self.sensors.len() > MIN_SENSORS {
            self.remove_sensor(rng);
        }
        // structural: grow / shrink the HIDDEN layer (brain capacity evolves; per-neuron upkeep is the cost)
        if rng.f32() < 0.05 && self.net.ih.len() < MAX_HIDDEN {
            self.add_hidden(rng);
        }
        if rng.f32() < 0.05 && self.net.ih.len() > MIN_HIDDEN {
            self.remove_hidden(rng);
        }
    }

    // Grow the hidden layer by one neuron: a new ih row (input weights) + a new column in every ho row
    // (its output weights), in both net + plast (kept same shape). Brain rebuilt from genome on spawn.
    fn add_hidden(&mut self, rng: &mut Rng) {
        let n_in1 = self.net.ih[0].len(); // n_in + 1 (incl. bias column)
        self.net.ih.push((0..n_in1).map(|_| rng.range(-1.0, 1.0)).collect());
        self.plast.ih.push((0..n_in1).map(|_| rng.f32() * 0.2).collect());
        for row in &mut self.net.ho {
            let bias = row.len() - 1; // insert the new hidden->output weight before the bias
            row.insert(bias, rng.range(-1.0, 1.0));
        }
        for row in &mut self.plast.ho {
            let bias = row.len() - 1;
            row.insert(bias, rng.f32() * 0.2);
        }
    }

    // Shrink the hidden layer: drop a random hidden neuron's ih row + its column in every ho row.
    fn remove_hidden(&mut self, rng: &mut Rng) {
        let h = self.net.ih.len();
        let idx = (rng.f32() * h as f32) as usize % h;
        self.net.ih.remove(idx);
        self.plast.ih.remove(idx);
        for row in &mut self.net.ho {
            row.remove(idx); // idx < bias position, so this drops that neuron's output weight
        }
        for row in &mut self.plast.ho {
            row.remove(idx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_expression_rewards_stocked_demanded_nutrients() {
        // gut demands nutrients 0 and 1 only
        let mut uptake = [0.0f32; NUTRIENTS];
        uptake[0] = 1.0;
        uptake[1] = 1.0;
        // fully stocked on what it demands -> expression ~1
        let mut full = [0.0f32; NUTRIENTS];
        full[0] = 0.6;
        full[1] = 0.6;
        let m_full = master_expression(&uptake, &full, 0.6, 0.2);
        assert!(m_full > 0.95, "stocked specialist should express ~1, got {m_full}");
        // missing one demanded nutrient -> expression drops toward half (one of two demands unmet)
        let mut half = [0.0f32; NUTRIENTS];
        half[0] = 0.6; // nutrient 1 empty
        let m_half = master_expression(&uptake, &half, 0.6, 0.2);
        assert!(m_half < 0.6 && m_half >= 0.2, "half-deficient should drop, got {m_half}");
    }

    #[test]
    fn master_expression_floors_with_no_uptake() {
        let uptake = [0.0f32; NUTRIENTS]; // undifferentiated gut
        let reserves = [1.0f32; NUTRIENTS];
        assert_eq!(master_expression(&uptake, &reserves, 0.6, 0.2), 0.2);
    }
}
