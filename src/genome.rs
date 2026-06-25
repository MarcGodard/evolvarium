// M3: variable-topology genome. GA places NN input nodes: sensors = evolvable list of directional
// food-eyes (count + angle + range). Brain resizes to match (dynamic I/O). Hidden size also evolvable
// (add/remove_hidden); per-neuron upkeep (BRAIN_COST) = trade-off. Weights = structured layers so
// growing/shrinking a sensor or hidden neuron stays clean.
use crate::rng::Rng;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const MIN_HIDDEN: usize = 2; // floor
pub const MAX_HIDDEN: usize = 16; // ceiling (bounds per-neuron upkeep cost)
pub const OUTPUTS: usize = 7; // [thrust 0..1, turn -1..1, attack, defend, eat, sprint, climb]; idx>=2 = 0..1 intents (climb: 0=descend, 1=climb)
pub const NFOOD: usize = 4; // plant FAMILY count (hue + kind label only; NOT metabolic axis)
pub const NUTRIENTS: usize = 10; // distinct nutrients = metabolic axis (regulatory uptake genome, see 14/05)

pub const MIN_SENSORS: usize = 1;
pub const MAX_SENSORS: usize = 8;
pub const SIG_PER_SENSOR: usize = 2; // each sensor reports [inv-dist, food type/readiness]
// Global (non-sensor) brain inputs, appended after per-sensor signals. Column order:
// [energy, daylight, fatigue, bias, toxic_load, shade, threat_dist, threat_bearing, wet, mag_lat, compass, altitude]
// energy+daylight+fatigue -> diurnal/nocturnal rest; toxic_load -> avoid poison; shade -> seek canopy in heat;
// threat_dist/bearing -> flee bigger predator; wet -> in water; mag_lat+compass -> magnetic nav (gated by
// `magneto` gene); altitude -> own height aloft (fliers manage climb/descend). GOTCHA: M4 widened 4 -> 9;
// magneto added 2 -> 11; flight added 1 -> 12. altitude is LAST so pad_ih_inputs (inserts before bias)
// aligns old saved nets correctly. Old saved nets zero-padded for new columns on load, see ensure_net_shape.
pub const GLOBAL_INPUTS: usize = 12;
pub const CONE_HALF: f32 = 0.7; // sensor FOV half-angle (rad)
const RANGE_MIN: f32 = 4.0;
const RANGE_MAX: f32 = 48.0; // long-range vision possible (big world); energy cost = trade-off (see sim SENSE_COST)

// One directional food-eye. angle = offset from heading; range = sight distance.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Sensor {
    pub angle: f32,
    pub range: f32,
}

// Layered weights. ih: HIDDEN rows, each (n_in + 1) incl. trailing bias. ho: OUTPUTS rows, each (HIDDEN + 1).
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
    pub uptake: [f32; NUTRIENTS], // gene i = absorption affinity + DEMAND for nutrient i ("10 genes").
                                  // Many high = generalist (varied diet + costly machinery); few = specialist
                                  // (cheap, fragile if that nutrient's food vanishes). Feed master expression
                                  // gene (per-life from reserves x uptake, see DietState).
    pub rigidity: f32,       // 0=flexible generalist .. 1=pinned specialist (koala)
    pub bite: f32,           // 0..1 eating strength vs plant defense (arms race, see 13); costs energy
    #[serde(default)]
    pub height: f32,         // 0..1 body height/reach: tall reaches fruit trees, costs upkeep (no free lunch)
    #[serde(default = "half")]
    pub light_pref: f32,     // 0=night .. 1=full sun; far from preferred light costs energy (diurnal/nocturnal niche)
    #[serde(default = "third")]
    pub size: f32,           // 0..1 body mass: more energy store + combat power, higher basal + move upkeep
    #[serde(default)]
    pub swim: f32,           // 0..1 aquatic: fast+cheap in water/wet lowland, clumsy+costly on dry land
    #[serde(default)]
    pub social: f32,         // 0..1 herd instinct: near KIN = predation safety (vigilance); ISOLATED = loneliness energy drain. Drives flocking + speciation; punishes lone cannibal.
    #[serde(default = "half")]
    pub temp_pref: f32,      // 0=cold (poles) .. 1=warm (equator). Local temp far from this costs energy. LATITUDINAL niche: poles harsh+uncrowded, equator mild+contested (no free lunch).
    #[serde(default = "half")]
    pub longevity: f32,      // 0..1 life-history: high = long life (aging slows) + higher basal; low = short fast cheap life. Default 0.5 = current lifespan + no extra cost (old saves unchanged).
    #[serde(default = "half")]
    pub metab: f32,          // 0..1 metabolic tempo: high = frugal (cheaper basal) but sluggish (slower top speed); low = fast (higher top speed) but costly. Default 0.5 = neutral (old saves unchanged).
    #[serde(default = "half")]
    pub parental: f32,       // 0..1 r/K: 0 = r (breed young+cheap, many small fragile young), 1 = K (breed late+costly, few provisioned young). Scales repro threshold/cost/birth-energy/maturity. Default 0.5 = neutral (old saves unchanged).
    #[serde(default = "zero")]
    pub alpine: f32,         // 0..1 mountain: high = cheap highland crossing (climber) + heavy-build penalty on flat; low = lowland-light. Mirror of swim for mountains. Default 0 = neutral (old saves unchanged).
    #[serde(default = "half")]
    pub adiposity: f32,      // 0..1 fat storage: high = big reserve (survives famine) but sluggish (fat mobilizes slow) + carry upkeep; low = lean+cheap but famine-fragile. Default 0.5 = baseline.

    // --- M4 creature expansion (all #[serde(default)] -> neutral, old saves load unchanged) ---
    #[serde(default = "d30")]
    pub detox: f32,          // 0..1 toxin-clearance: high = clears toxic_load fast + eats toxic plants/rotten meat safely (liver upkeep DETOX_COST); low = cheap but poisons easily. Default 0.3.
    #[serde(default = "d30")]
    pub carnivory: f32,      // 0..1 gut herbivore..carnivore: high = digests meat/protein, poor at plant sugar; low = opposite. Central to rabbit starvation (lean-meat protein cap). Default 0.3.
    #[serde(default = "d20")]
    pub pelt: f32,           // 0..1 fur cover: insulation cuts cold-side temp cost; costs heat-side + swim drag + basal. Default 0.2.
    #[serde(default = "zero")]
    pub armor: f32,          // 0..1 body armor: lowers predation success vs it; costs move + basal. Default 0.
    #[serde(default = "zero")]
    pub venom: f32,          // 0..1 toxic flesh: predator eating it takes toxic_load hit (deterrent); costs basal + aposematic look. Default 0.
    #[serde(default = "d40")]
    pub limbs: f32,          // 0..1 -> 2..8 legs: more = land traction (speed/stability on rough ground); costs move energy per limb. Default 0.4 (~4 legs).
    #[serde(default = "zero")]
    pub climb: f32,          // 0..1 tree-climbing: reach fruit trees w/o tall height + tree-refuge safety; costs penalty on open flat (arboreal build). Default 0.
    #[serde(default = "d40")]
    pub eyes: f32,           // 0..1 -> 1..6 eyes: small detection bonus (extends effective sense); per-eye upkeep. Default 0.4.
    #[serde(default = "d40")]
    pub head: f32,           // 0..1 head size: bigger houses brain cheaper (cuts per-neuron BRAIN_COST); head mass adds basal. Default 0.4.
    #[serde(default = "d40")]
    pub skin_hue: f32,       // 0..1 base body hue (render). Default 0.4.
    #[serde(default = "half")]
    pub skin_sat: f32,       // 0..1 body saturation (render). Default 0.5.
    #[serde(default = "zero")]
    pub pattern: f32,        // 0..1 markings intensity: stripes/spots (render). Default 0.
    // Body-plan render genes (cosmetic, NO sim/balance effect): drive silhouette variety so creatures read
    // as fish/snake/octopus/cow/squirrel etc. Used only in viz::creature_look + add_creature_visuals.
    #[serde(default = "zero")]
    pub elongate: f32,       // 0..1 body length (Z stretch) + slimmer cross-section: low=blocky (cow), high=snake/eel/weasel. Cosmetic.
    #[serde(default = "zero")]
    pub tail: f32,           // 0..1 tail size: caudal fin if swimmer, bushy if furry land (squirrel), rod taper else. Cosmetic.
    #[serde(default = "zero")]
    pub fin: f32,            // 0..1 dorsal-fin prominence (ridge along spine). Cosmetic.
    #[serde(default = "zero")]
    pub magneto: f32,        // 0..1 magnetoreception switch: above soft knee, feeds 2 brain inputs (mag_lat
                             // "map" + compass heading) for nav; costs MAG_COST upkeep (magnetite organ +
                             // neural processing). Default 0 = sense off (old saves unchanged).
    #[serde(default = "zero")]
    pub flight: f32,         // 0..1 aerial: above FLIGHT_KNEE creature can climb (brain out[6]) -> fast aloft +
                             // skips ground collision/drowning; holding altitude burns energy + big wings clumsy
                             // grounded (mirror of swim). Default 0 = grounded (old saves unchanged). Drives bird niche.
    #[serde(default = "zero")]
    pub beak: f32,           // 0..1 snout/beak length (render only, NO sim effect): birds get a forward beak, other
                             // body plans a snout. Cosmetic, backfilled by ensure_cosmetic on old saves.

    // M5 generative morphology: Karl-Sims part-graph that GROWS the body (see morph.rs). Drives the mesh +
    // geometry-derived stats (mass/drag/reach/wing/fin), so SHAPE is under selection, not the old cosmetic
    // scalars. serde default = single capsule -> old saves load + render as today's creature.
    #[serde(default = "crate::morph::default_body")]
    pub body: crate::morph::BodyGraph,

    // Cached geometry-derived stats of `body` (mass/reach/areas/limbs). NOT serialized (derived from body):
    // populated by ensure_net_shape at every spawn so live_step reads it without re-developing per tick.
    #[serde(skip)]
    pub morph: Option<crate::morph::Morphometrics>,
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
// neutral defaults for M4 creature-expansion genes (old saves load unchanged)
fn d20() -> f32 {
    0.2
}
fn d30() -> f32 {
    0.3
}
fn d40() -> f32 {
    0.4
}
// serde default uptake for saves predating nutrient genome: mid generalist (all absorbed moderately) ->
// old creatures load as functional omnivores.
fn default_uptake() -> [f32; NUTRIENTS] {
    [0.5; NUTRIENTS]
}

// Master digestion expression: single gene the 10 uptake genes feed. Absorbing nutrient i also DEMANDS it
// (demand = uptake_i). Expression = demand-weighted mean of how stocked each demanded reserve is (reserves_i
// vs RESERVE_REQ). High when every demanded nutrient stocked -> rewards diet matching what creature uses.
// Floored so never fully shut off. No uptake -> falls to floor (undifferentiated gut digests poorly). Gates
// energy extraction (see sim).
pub fn master_expression(uptake: &[f32; NUTRIENTS], reserves: &[f32; NUTRIENTS], req: f32, floor: f32) -> f32 {
    let mut wsum = 0.0;
    let mut sat = 0.0;
    for i in 0..NUTRIENTS {
        let demand = uptake[i];
        wsum += demand;
        sat += demand * (reserves[i] / req).min(1.0); // demand satisfaction, capped at 1
    }
    if wsum < 1e-3 {
        return floor; // no uptake -> undifferentiated gut -> poor baseline digestion
    }
    (sat / wsum).max(floor)
}

pub fn n_inputs(n_sensors: usize) -> usize {
    n_sensors * SIG_PER_SENSOR + GLOBAL_INPUTS
}

/// Magnetoreception expression from `magneto` gene: soft switch (smoothstep over 0.2..0.6 knee). Below
/// knee sense off (0), above it full (1). Scales the 2 magnetic brain inputs AND MAG_COST upkeep, so
/// half-built organ gives little signal for partial cost (selection sharpens it).
pub fn mag_expression(magneto: f32) -> f32 {
    let t = ((magneto - 0.2) / 0.4).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Pad every ih row to `want_in` input columns (excl. trailing +1 bias), inserting `fill` per new column
// right before bias weight. Used by ensure_net_shape to migrate older, narrower saved nets.
fn pad_ih_inputs(net: &mut Net, want_in: usize, fill: f32) {
    for row in net.ih.iter_mut() {
        let have_in = row.len().saturating_sub(1); // row = input cols + 1 trailing bias
        if have_in < want_in {
            let at = row.len() - 1; // insert before trailing bias weight
            for _ in 0..(want_in - have_in) {
                row.insert(at, fill);
            }
        }
    }
}

// Migration default biases for outputs ADDED after a seed saved. Migrated creature must behave as before
// new outputs existed: combat+effort OFF (strong negative bias -> sigmoid ~0, no unearned ATTACK_COST /
// SPRINT_COST / brace-drag), EAT ON (positive bias -> sigmoid ~1, still feeds on contact like pre-eat-gate
// code). Fresh founders use random_net instead (varied combat outputs) so emergence works. Indices:
// [thrust, turn, attack, defend, eat, sprint, climb]; 0/1 never padded (always present). climb biased
// negative -> migrated net sinks to ground (no unearned flight), matching pre-flight grounded behavior.
const OUTPUT_MIGRATE_BIAS: [f32; OUTPUTS] = [0.0, 0.0, -4.0, -4.0, 4.0, -4.0, -4.0];

// Grow NET ho layer to want_rows (migrate seed saved when OUTPUTS smaller, e.g. pre-combat 2-output nets).
// New output row = hidden+1 long (matches live hidden count): zero hidden->output weights + per-output
// default bias so migrated output defaults off/on sensibly. No-op when ho already has enough rows.
fn pad_net_ho(net: &mut Net, want_rows: usize) {
    if net.ho.is_empty() {
        return;
    }
    let row_len = net.ho[0].len(); // hidden + 1 (trailing bias), matches live hidden count
    while net.ho.len() < want_rows {
        let idx = net.ho.len();
        let mut row = vec![0.0; row_len];
        *row.last_mut().unwrap() = OUTPUT_MIGRATE_BIAS.get(idx).copied().unwrap_or(0.0); // bias sets default activation
        net.ho.push(row);
    }
}

// Grow PLAST ho layer: new rows = uniform small-plasticity, so migrated outputs CAN be tuned by lifetime
// learning (drift off default bias once reward favors using them).
fn pad_plast_ho(plast: &mut Net, want_rows: usize) {
    if plast.ho.is_empty() {
        return;
    }
    let row_len = plast.ho[0].len();
    while plast.ho.len() < want_rows {
        plast.ho.push(vec![0.2; row_len]);
    }
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
        // uptake: sparse-ish founders -> ~1/3 nutrients absorbed strongly, rest weakly, so founders span
        // specialists..partial-generalists and selection can broaden or narrow the gut.
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
            temp_pref: rng.f32(), // founders span cold..warm -> spread across latitudes
            longevity: rng.f32(),
            metab: rng.f32(),
            parental: rng.f32(),
            alpine: rng.f32(), // founders span lowland..mountain -> highland niche can emerge
            adiposity: rng.f32(), // span lean..fatty storage
            // M4: founders span full range per new axis so selection has variation
            detox: rng.f32() * 0.5,   // mostly low-detox (cheap), few tolerant
            carnivory: rng.f32(),     // span herbivore..carnivore guts
            pelt: rng.f32() * 0.5,    // mostly light coats
            armor: rng.f32() * 0.3,   // mostly unarmored
            venom: rng.f32() * 0.2,   // mostly non-toxic
            limbs: rng.f32(),         // span few..many limbs
            climb: rng.f32() * 0.4,   // mostly ground-dwellers, few climbers
            eyes: rng.f32(),          // span eye counts
            head: rng.range(0.3, 0.7),// mid heads (brain housing)
            skin_hue: rng.f32(),      // span color wheel
            skin_sat: rng.range(0.3, 0.9),
            pattern: rng.f32() * 0.6, // span plain..marked
            elongate: rng.f32() * rng.f32(), // skew short; few very long (snake/eel)
            tail: rng.f32(),          // span tailless..long-tailed
            fin: rng.f32() * rng.f32(), // skew finless; few prominent dorsal fins
            magneto: rng.f32() * 0.3, // mostly sense-off, few magnetoreceptive -> selection can switch on
            // skew grounded, but ~15% of founders are TRUE fliers (>FLIGHT_KNEE/wing threshold) so the bird niche
            // is visible from gen 0 (not waiting many gens for mutation to cross 0.5). Rest stay low (ground-biased).
            flight: if rng.f32() < 0.15 { rng.range(0.55, 1.0) } else { rng.f32() * rng.f32() * 0.4 },
            beak: rng.f32() * rng.f32(), // skew short snouts; few long beaks/snouts
            body: crate::morph::BodyGraph::random(rng), // generative body-graph (mesh + derived stats)
            morph: None, // populated at spawn (ensure_net_shape)
        }
    }

    pub fn n_sensors(&self) -> usize {
        self.sensors.len()
    }

    // Linear body-size factor from the size gene: tiny "mouse" -> huge "dinosaur". SINGLE source of truth for
    // size. Drives render scale (viz::body_scale) AND the flight physics: mass ~ scale^3, wing area ~ scale^2,
    // so wing loading ~ scale -> big bodies can't fly (sim::wing_loading folds this in). ~13x linear span here
    // (size 0..1 -> 0.3..3.9) -> ~2400x mass range. Combat/defense also scale off it (size pays rent).
    pub fn size_scale(&self) -> f32 {
        0.3 + 3.6 * self.size
    }

    // Maturity-time multiplier from size: big bodies develop slower (offspring take longer to reach adult size
    // + breeding age) -> a K-strategy cost balancing large creatures' combat dominance. Anchored ~1 at mid size
    // so existing pacing holds; mouse ~0.4x (breeds fast), dino ~2x (slow). Scales repro_min_age + visual grow-in.
    pub fn maturity_scale(&self) -> f32 {
        0.4 + 1.6 * self.size
    }

    // Rebuild net + plast as fresh random weights sized to CURRENT sensors (+ existing hidden count). Used
    // by scenario harness after overriding `sensors` so net shape matches before optional reflex prior.
    // (Bare scalar-only override keeps base net, so only called when sensors change.)
    pub fn rebuild_random_net(&mut self, rng: &mut Rng) {
        let n_in = n_inputs(self.n_sensors());
        let n_hidden = self.net.ih.len().clamp(MIN_HIDDEN, MAX_HIDDEN);
        self.net = random_net(rng, n_in, n_hidden, false);
        self.plast = random_net(rng, n_in, n_hidden, true);
    }

    // Migrate loaded net to CURRENT input width. New M4 global brain-inputs widened n_inputs, so older saved
    // net has fewer input columns than live code expects. Insert columns before each row's trailing bias: net
    // gets 0.0 (new input no influence at first), plast gets small value (new input CAN be learned). Keeps
    // existing learned weights aligned. No-op when shape already matches (fresh genomes + births). Also grows
    // ho rows if OUTPUTS expanded since seed saved.
    pub fn ensure_net_shape(&mut self) {
        let want = n_inputs(self.n_sensors());
        pad_ih_inputs(&mut self.net, want, 0.0);
        pad_ih_inputs(&mut self.plast, want, 0.2);
        // ho rows: pre-combat seed (OUTPUTS 2) gets 4 new output rows appended (attack/defend/eat/sprint),
        // net biased to safe defaults (combat off, eat on) + plast learnable. learn() loops ho generically,
        // so both layers must grow or it indexes plast.ho out of bounds.
        pad_net_ho(&mut self.net, OUTPUTS);
        pad_plast_ho(&mut self.plast, OUTPUTS);
        self.ensure_cosmetic(); // backfill body-plan genes on saves predating them -> loaded pop looks varied at once
        self.morph = Some(crate::morph::Morphometrics::of(&self.body)); // cache derived stats for live_step
    }

    // Old saves lack body-plan render genes (elongate/tail/fin) -> all-zero loads as identical capsules, so a
    // loaded showcase pop would look monochrome+uniform. Derive a DETERMINISTIC per-creature spread from a hash
    // of stable genome floats (no rng needed inside load path) so the loaded world is visually varied at once.
    // Heritable after: mutate() drifts these, random() seeds fresh founders. Only fires when ALL three still 0
    // (unset) -> fresh/evolved genomes (which set nonzero) keep their values; reaching exactly 0 on all 3 is
    // effectively impossible after random()+drift.
    pub fn ensure_cosmetic(&mut self) {
        if self.elongate != 0.0 || self.tail != 0.0 || self.fin != 0.0 {
            return;
        }
        let mut h: u32 = 2166136261; // FNV-1a over stable genome floats
        for v in [self.skin_hue, self.skin_sat, self.size, self.swim, self.head, self.bite, self.metab, self.carnivory] {
            h ^= (v * 9973.0) as i32 as u32;
            h = h.wrapping_mul(16777619);
        }
        for s in &self.sensors {
            h ^= (s.angle * 1000.0) as i32 as u32;
            h = h.wrapping_mul(16777619);
        }
        let u = |shift: u32| ((h >> shift) & 0xFF) as f32 / 255.0; // byte slice -> 0..1
        let e = u(0);
        self.elongate = e * e; // skew short, few long
        self.tail = u(8);
        let f = u(16);
        self.fin = f * f; // skew finless, few prominent
        let bk = u(24);
        self.beak = bk * bk; // skew short snouts, few long beaks
    }

    // Two-parent recombination (--mating mode). Body STRUCTURE (sensors + brain net/plast) from parent `a`
    // (variable-topology nets can't cross cell-by-cell); scalar trait genes + diet uniform-crossed from both.
    // Caller mutates result. With assortative mate choice (only similar kin mate) -> reproductive isolation
    // -> speciation.
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
        c.detox = pick(rng, a.detox, b.detox);
        c.carnivory = pick(rng, a.carnivory, b.carnivory);
        c.pelt = pick(rng, a.pelt, b.pelt);
        c.armor = pick(rng, a.armor, b.armor);
        c.venom = pick(rng, a.venom, b.venom);
        c.limbs = pick(rng, a.limbs, b.limbs);
        c.climb = pick(rng, a.climb, b.climb);
        c.eyes = pick(rng, a.eyes, b.eyes);
        c.head = pick(rng, a.head, b.head);
        c.skin_hue = pick(rng, a.skin_hue, b.skin_hue);
        c.skin_sat = pick(rng, a.skin_sat, b.skin_sat);
        c.pattern = pick(rng, a.pattern, b.pattern);
        c.elongate = pick(rng, a.elongate, b.elongate);
        c.tail = pick(rng, a.tail, b.tail);
        c.fin = pick(rng, a.fin, b.fin);
        c.magneto = pick(rng, a.magneto, b.magneto);
        c.flight = pick(rng, a.flight, b.flight);
        c.beak = pick(rng, a.beak, b.beak);
        for i in 0..NUTRIENTS {
            c.uptake[i] = pick(rng, a.uptake[i], b.uptake[i]);
        }
        c
    }

    pub fn mutate(&mut self, rng: &mut Rng, rate: f32, std: f32) {
        // weights clamped -5..5
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
        // sensor placement: angle wraps pi, range clamps RANGE_MIN..MAX
        for s in &mut self.sensors {
            if rng.f32() < rate {
                s.angle = wrap_pi(s.angle + rng.normal() * 0.4);
            }
            if rng.f32() < rate {
                s.range = (s.range + rng.normal() * 3.5).clamp(RANGE_MIN, RANGE_MAX);
            }
        }
        // nutrient-uptake genes (regulatory gut)
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
        // M4 genes drift like the rest
        if rng.f32() < rate {
            self.detox = (self.detox + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.carnivory = (self.carnivory + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.pelt = (self.pelt + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.armor = (self.armor + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.venom = (self.venom + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.limbs = (self.limbs + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.climb = (self.climb + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.eyes = (self.eyes + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.head = (self.head + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.skin_hue = (self.skin_hue + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.skin_sat = (self.skin_sat + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.pattern = (self.pattern + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.elongate = (self.elongate + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.tail = (self.tail + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.fin = (self.fin + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.magneto = (self.magneto + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.flight = (self.flight + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        if rng.f32() < rate {
            self.beak = (self.beak + rng.normal() * 0.12).clamp(0.0, 1.0);
        }
        // generative body-graph drift (param + structural; bounded inside morph::mutate)
        self.body.mutate(rng, rate);
        // structural: add/remove sensor (+ matching input-weight columns), p=0.06 each
        if rng.f32() < 0.06 && self.sensors.len() < MAX_SENSORS {
            self.add_sensor(rng);
        }
        if rng.f32() < 0.06 && self.sensors.len() > MIN_SENSORS {
            self.remove_sensor(rng);
        }
        // structural: grow/shrink HIDDEN layer (brain capacity evolves; per-neuron upkeep = cost), p=0.05 each
        if rng.f32() < 0.05 && self.net.ih.len() < MAX_HIDDEN {
            self.add_hidden(rng);
        }
        if rng.f32() < 0.05 && self.net.ih.len() > MIN_HIDDEN {
            self.remove_hidden(rng);
        }
    }

    // Grow hidden layer by one neuron: new ih row (input weights) + new column in every ho row (its output
    // weights), in both net + plast (same shape). Brain rebuilt from genome on spawn.
    fn add_hidden(&mut self, rng: &mut Rng) {
        let n_in1 = self.net.ih[0].len(); // n_in + 1 (incl. bias column)
        self.net.ih.push((0..n_in1).map(|_| rng.range(-1.0, 1.0)).collect());
        self.plast.ih.push((0..n_in1).map(|_| rng.f32() * 0.2).collect());
        for row in &mut self.net.ho {
            let bias = row.len() - 1; // insert new hidden->output weight before bias
            row.insert(bias, rng.range(-1.0, 1.0));
        }
        for row in &mut self.plast.ho {
            let bias = row.len() - 1;
            row.insert(bias, rng.f32() * 0.2);
        }
    }

    // Shrink hidden layer: drop random hidden neuron's ih row + its column in every ho row.
    fn remove_hidden(&mut self, rng: &mut Rng) {
        let h = self.net.ih.len();
        let idx = (rng.f32() * h as f32) as usize % h;
        self.net.ih.remove(idx);
        self.plast.ih.remove(idx);
        for row in &mut self.net.ho {
            row.remove(idx); // idx < bias position -> drops that neuron's output weight
        }
        for row in &mut self.plast.ho {
            row.remove(idx);
        }
    }

    // New sensor's 2 input columns inserted right before GLOBAL_INPUTS (energy, ...).
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

// Forward pass over Net for input vector. Returns (hidden activations, outputs).
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
    // combat/effort intents: attack, defend/brace, eat-gate, sprint. all 0..1 (sigmoid). loop so future
    // OUTPUTS bump can't leave raw unactivated output.
    for o in out.iter_mut().skip(2) {
        *o = sigmoid(*o);
    }
    (h, out)
}

// Reward-modulated Hebbian (see 04). Moves weights only when reward != 0; plasticity scales per-weight.
// Loops ho generically -> plast.ho must match net.ho rows (see ensure_net_shape) or indexes OOB.
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

// Wrap angle to [-PI, PI). rem_euclid keeps result in [0, TAU).
fn wrap_pi(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
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
        // fully stocked on demands -> expression ~1
        let mut full = [0.0f32; NUTRIENTS];
        full[0] = 0.6;
        full[1] = 0.6;
        let m_full = master_expression(&uptake, &full, 0.6, 0.2);
        assert!(m_full > 0.95, "stocked specialist should express ~1, got {m_full}");
        // missing one demanded nutrient -> expression drops toward half (1 of 2 demands unmet)
        let mut half = [0.0f32; NUTRIENTS];
        half[0] = 0.6; // nutrient 1 empty
        let m_half = master_expression(&uptake, &half, 0.6, 0.2);
        assert!(m_half < 0.6 && m_half >= 0.2, "half-deficient should drop, got {m_half}");
    }

    #[test]
    fn ensure_net_shape_pads_old_narrow_nets() {
        // simulate OLD save (pre-M4 + pre-magneto + pre-flight: 8 fewer global inputs) by stripping columns,
        // then migrate. 5 M4 globals: toxic_load/shade/threat_dist/threat_bear/wet; 2 magneto: lat/compass;
        // 1 flight: altitude.
        let mut rng = Rng::seed(7);
        let mut g = Genome::random(&mut rng);
        let want = n_inputs(g.n_sensors());
        let strip = 8; // 5 M4 globals + 2 magneto globals + 1 flight global
        for row in g.net.ih.iter_mut().chain(g.plast.ih.iter_mut()) {
            let at = row.len() - 1 - strip; // before trailing bias
            row.drain(at..at + strip);
        }
        assert!(g.net.ih[0].len() < want + 1, "rows should be too narrow before migration");
        g.ensure_net_shape();
        for row in g.net.ih.iter().chain(g.plast.ih.iter()) {
            assert_eq!(row.len(), want + 1, "every ih row padded to n_inputs + bias");
        }
        // forward pass at current input width must not panic (shape matches)
        let input = vec![0.0f32; want];
        let _ = forward(&g.net, &input);
    }

    #[test]
    fn ensure_net_shape_grows_old_output_rows() {
        // simulate PRE-COMBAT save (OUTPUTS was 2) by truncating ho to 2 rows, then migrate to OUTPUTS=6.
        let mut rng = Rng::seed(11);
        let mut g = Genome::random(&mut rng);
        g.net.ho.truncate(2);
        g.plast.ho.truncate(2);
        assert_eq!(g.net.ho.len(), 2, "pre-migration ho has the old output count");
        g.ensure_net_shape();
        assert_eq!(g.net.ho.len(), OUTPUTS, "net ho padded up to current OUTPUTS");
        assert_eq!(g.plast.ho.len(), OUTPUTS, "plast ho padded up too (learn() indexes it)");
        let hidden_plus_bias = g.net.ih.len() + 1;
        for row in g.net.ho.iter().chain(g.plast.ho.iter()) {
            assert_eq!(row.len(), hidden_plus_bias, "new output rows are hidden+1 wide");
        }
        // forward + learn at OUTPUTS=6 must not panic (migration covers both layers)
        let want = n_inputs(g.n_sensors());
        let input = vec![0.1f32; want];
        let (h, out) = forward(&g.net, &input);
        learn(&mut g.net, &g.plast, &input, &h, &out, 1.0, 0.04);
    }

    #[test]
    fn master_expression_floors_with_no_uptake() {
        let uptake = [0.0f32; NUTRIENTS]; // undifferentiated gut
        let reserves = [1.0f32; NUTRIENTS];
        assert_eq!(master_expression(&uptake, &reserves, 0.6, 0.2), 0.2);
    }
}
