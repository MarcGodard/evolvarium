// Agent tuning harness, Layer 1 engine. Spec: clients/evolvarium/14-tuning-harness.md.
// Runs ONE isolated headless mini-world: small cohort of plants/trees in controlled env band, for `ticks`,
// then writes metrics+genomes result JSON, exits. Agent loop: write scenario.json, run binary, read
// result.json, adjust genes/env, re-run -> evolve cohort toward survival+growth in niche. Own process =
// full ECS isolation, so many run parallel.
//
// UNIFIED cohort runner: `plant_cohort` arm + `creature_cohort` arm (M4). GENE-AGNOSTIC: genome overrides +
// trait-drift go through serde generically, so new PlantGenome gene tunable here with ZERO edits to this file.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::components::{Alive, Creature, DietState, Energy, Tree};
use crate::genome::{master_expression, n_inputs, Genome, Net, Sensor, NFOOD};
use crate::plant::{Archetype, PlantGenome, PlantState};
use crate::sim::{GenState, MASTER_FLOOR, RESERVE_REQ};

// --- scenario input schema ---

#[derive(Deserialize)]
pub struct Scenario {
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_ticks")]
    pub ticks: u32, // run length / cohort lifetime budget. ~6 sim-days at 30000.
    #[serde(default = "default_target")]
    pub target_count: usize, // growth goal agent tunes cohort toward
    #[serde(default)]
    pub world: WorldCfg,
    #[serde(default)]
    pub plant_cohort: Vec<PlantSpec>,
    #[serde(default)]
    pub creature_cohort: Vec<CreatureSpec>,
}

#[derive(Deserialize)]
pub struct CreatureSpec {
    pub count: usize,
    #[serde(default)]
    pub genome: Map<String, Value>, // free-form Genome overrides merged onto random base (ANY gene)
    #[serde(default)]
    pub reflex: Option<String>, // named brain prior: approach-food | flee-predator | rest-at-night | wander
}

#[derive(Deserialize)]
pub struct WorldCfg {
    #[serde(default = "default_band")]
    pub lat_band: [f32; 2], // place cohort + sample climate in |latitude| range. radians, 0=equator..1.57=pole.
    #[serde(default = "half")]
    pub wetness: f32, // 0 dry..1 wet: pins effective moisture (climate grid) for band
    #[serde(default)]
    pub aquatic: bool, // shallow-water niche: place in sea
    #[serde(default)]
    pub rocky: bool, // highland niche: place on high ground
    #[serde(default)]
    pub fire: f32, // 0..1 ambient fire pressure pinned into band. >0.4 kills.
    #[serde(default)]
    pub grazers: usize, // creatures seeded for grazing pressure
    #[serde(default)]
    pub second_band: Option<[f32; 2]>, // MIXED cohort straddles second band too
}

impl Default for WorldCfg {
    fn default() -> Self {
        WorldCfg { lat_band: default_band(), wetness: 0.5, aquatic: false, rocky: false, fire: 0.0, grazers: 0, second_band: None }
    }
}

#[derive(Deserialize)]
pub struct PlantSpec {
    pub count: usize,
    #[serde(default)]
    pub archetype: Option<String>, // named base (plant::Archetype); else random / tree base
    #[serde(default)]
    pub tree: bool, // seed Tree marker (tall/slow) instead of ground plant
    #[serde(default)]
    pub genome: Map<String, Value>, // free-form overrides merged onto base (ANY gene, incl. new ones)
}

fn default_seed() -> u64 {
    1
}
fn default_ticks() -> u32 {
    12000
}
fn default_target() -> usize {
    30
}
fn default_band() -> [f32; 2] {
    [0.0, 0.6]
}
fn half() -> f32 {
    0.5
}

// --- resources ---

#[derive(Resource)]
pub struct ScenarioCfg {
    pub scenario: Scenario,
    pub out: String,
}

// Running tallies. Presence of THIS resource = scenario-mode flag for plant_step (sim.rs): disables
// PLANT_MIN reseed floor + counts death causes. Absent on normal runs (zero cost).
#[derive(Resource, Default)]
pub struct ScenarioStats {
    pub started: usize,
    pub births: u32,
    pub deaths: u32,
    pub deaths_by_cause: HashMap<String, u32>,
    pub peak_count: usize,
    pub cap: usize, // cohort pop cap (~2x target_count): cohort grows toward target + shows vigor WITHOUT
    // booming to global PLANT_CAP (which saturated every metric).
    pub seeded: Vec<PlantGenome>, // applied cohort genomes (trait-drift baseline)
    pub cstarted: usize,          // creatures seeded
    pub cseeded: Vec<Genome>,     // applied creature genomes (creature trait-drift baseline)
}

impl ScenarioStats {
    // Record plant death by cause. Gene-agnostic string key: new cause needs no schema change.
    pub fn death(&mut self, cause: &str) {
        self.deaths += 1;
        *self.deaths_by_cause.entry(cause.to_string()).or_insert(0) += 1;
    }
}

// Generic serde merge: free-form overrides onto base genome. NO per-gene code (gene-agnostic).
fn apply_overrides(base: PlantGenome, ov: &Map<String, Value>) -> PlantGenome {
    if ov.is_empty() {
        return base;
    }
    let mut v = match serde_json::to_value(&base) {
        Ok(v) => v,
        Err(_) => return base,
    };
    if let Value::Object(map) = &mut v {
        for (k, val) in ov {
            // overwrite real genome fields only; ignore unknown/typo keys (forward-compat slack)
            if map.contains_key(k) {
                map.insert(k.clone(), val.clone());
            } else {
                bevy::log::warn!("scenario override key '{}' is not a PlantGenome field (ignored)", k);
            }
        }
    }
    serde_json::from_value(v).unwrap_or(base)
}

fn archetype_by_name(name: &str) -> Option<Archetype> {
    use Archetype as A;
    Some(match name {
        "Clover" => A::Clover,
        "Wildflower" => A::Wildflower,
        "BerryBush" => A::BerryBush,
        "Fern" => A::Fern,
        "Cactus" => A::Cactus,
        "Reed" => A::Reed,
        "Thistle" => A::Thistle,
        "Nightshade" => A::Nightshade,
        "Moss" => A::Moss,
        "AlpineCushion" => A::AlpineCushion,
        "Tumbleweed" => A::Tumbleweed,
        "Waterlily" => A::Waterlily,
        "Eelgrass" => A::Eelgrass,
        "Kelp" => A::Kelp,
        "AlgaeMat" => A::AlgaeMat,
        _ => return None,
    })
}

// Generic serde merge onto base Genome (mirror of apply_overrides for plants).
// Scalar genes, uptake[], even `sensors` overridable. Caller rebuilds net when sensors change.
fn apply_overrides_genome(base: Genome, ov: &Map<String, Value>) -> Genome {
    if ov.is_empty() {
        return base;
    }
    let mut v = match serde_json::to_value(&base) {
        Ok(v) => v,
        Err(_) => return base,
    };
    if let Value::Object(map) = &mut v {
        for (k, val) in ov {
            if map.contains_key(k) {
                map.insert(k.clone(), val.clone());
            } else {
                bevy::log::warn!("scenario creature override key '{}' is not a Genome field (ignored)", k);
            }
        }
    }
    serde_json::from_value(v).unwrap_or(base)
}

// Reflex brain priors: hand-wired starting weights; lifetime learning refines them in run. Input layout
// (genome.rs): per sensor i -> [inv_dist @ 2i, type @ 2i+1]; globals at base = n_sensors*2:
// [energy, daylight, fatigue, bias, toxic_load, shade, threat_dist, threat_bear, wet]. Outputs
// [thrust, turn, attack, defend, eat, sprint]. Priors wire only thrust/turn (rows 0/1); combat rows 2..6
// stay zero and are learned. None for unknown name (caller keeps random net).
fn reflex_brain(name: &str, sensors: &[Sensor]) -> Option<Net> {
    let n_s = sensors.len();
    let base = n_s * crate::genome::SIG_PER_SENSOR;
    let n_in = n_inputs(n_s);
    let n_hidden = 3usize;
    // zeroed net of right shape: ih = n_hidden rows of n_in+1; ho = OUTPUTS rows of n_hidden+1
    let mut ih: Vec<Vec<f32>> = (0..n_hidden).map(|_| vec![0.0; n_in + 1]).collect();
    let mut ho: Vec<Vec<f32>> = (0..crate::genome::OUTPUTS).map(|_| vec![0.0; n_hidden + 1]).collect();
    let (i_daylight, i_threat_d, i_threat_b) = (base + 1, base + 6, base + 7);
    match name {
        "approach-food" => {
            // h0 = food proximity (sum sensor inv-dist); h1 = steering (food left vs right)
            for (i, s) in sensors.iter().enumerate() {
                ih[0][2 * i] = 1.0;
                ih[1][2 * i] = if s.angle < 0.0 { -1.0 } else { 1.0 };
            }
            ih[0][n_in] = -0.2; // bias: ~0 when no food in view
            ho[0][0] = 3.0; // thrust rises with food proximity
            ho[0][n_hidden] = -0.3; // idle when nothing near
            ho[1][1] = 2.0; // turn toward food side
        }
        "flee-predator" => {
            ih[0][i_threat_d] = 2.0; // h0 = predator proximity
            ih[1][i_threat_b] = 1.0; // h1 = predator bearing
            ho[0][0] = 3.0; // sprint when predator near
            ho[0][n_hidden] = -0.4;
            ho[1][1] = -2.5; // steer AWAY from predator bearing
        }
        "rest-at-night" => {
            ih[0][i_daylight] = 1.0; // h0 = daylight
            ho[0][0] = 2.0; // active by day
            ho[0][n_hidden] = -0.5; // rest (low thrust) at night
        }
        "wander" => {
            ho[0][n_hidden] = 0.2; // steady low cruise, no target
        }
        _ => return None,
    }
    Some(Net { ih, ho })
}

// Startup: pin env band, seed ONLY the cohort.
pub fn spawn_scenario_world(
    mut commands: Commands,
    mut rng: ResMut<crate::rng::Rng>,
    cfg: Res<ScenarioCfg>,
    mut stats: ResMut<ScenarioStats>,
    mut gw: ResMut<crate::sim::GroundWater>,
    mut climate: ResMut<crate::sim::Climate>,
    mut fire: ResMut<crate::sim::Fire>,
) {
    let w = &cfg.scenario.world;
    // Pin controlled env ONCE: no weather_step in scenario mode, so these stay fixed -> reproducible niche.
    // `wetness` IS effective local moisture: pin slow CLIMATE grid to it (CLIMATE_VEG=1 -> plants read
    // climate as moisture). Ground water left ~0: on real planet gw is transient rain averaging ~0.01;
    // adding here DOUBLE-COUNTS wetness (m = climate + WET_GAIN*gw) so wetness=0.6 would feel ~0.87 and a
    // plant tuned to wet=0.6 wet-stresses. With gw~0, effective moisture = wetness (+ seasonal wobble):
    // intuitive to tune AND faithful to real-planet biome moisture, so tuned genomes transfer to planet.
    for c in gw.cell.iter_mut() {
        *c = 0.0;
    }
    for c in climate.cell.iter_mut() {
        *c = w.wetness;
    }
    for c in fire.cell.iter_mut() {
        *c = w.fire;
    }

    // Place cohort member: pick |latitude| in band (or second band for MIXED), then matching surface pos
    // (aquatic = shallow sea, rocky = high ground, else low land).
    // GOTCHA: creatures forage only if they + food share a LOCAL patch; full latitude ring spreads them
    // planet-wide -> never meet food. So with a creature cohort present, co-locate whole cohort (plants too)
    // in one compact patch round a center. Plant-only scenarios keep wide latitude-ring (climate is tuned
    // there, position moot).
    let patch = !cfg.scenario.creature_cohort.is_empty();
    const PATCH_CAP: f32 = 0.13; // ~7.5 deg cap: local foraging region
    let mid_lat = (w.lat_band[0] + w.lat_band[1]) * 0.5;
    let patch_center = if w.aquatic {
        crate::sim::niche_water_pos(&mut rng, mid_lat, 0.0).normalize_or_zero()
    } else {
        crate::sim::niche_pos(&mut rng, !w.rocky, mid_lat, 0.0).normalize_or_zero()
    };
    let place = |rng: &mut crate::rng::Rng, y: f32| -> Vec3 {
        if patch {
            // scatter in patch; keep land/water matching niche (retry like homeland_pos)
            let mut d = crate::sphere::random_dir_in_cap(rng, patch_center, PATCH_CAP);
            for _ in 0..8 {
                if crate::sphere::is_ocean(d) == w.aquatic {
                    break;
                }
                d = crate::sphere::random_dir_in_cap(rng, patch_center, PATCH_CAP);
            }
            return crate::sphere::surface_pos(d, y);
        }
        let band = match (w.second_band, rng.f32() < 0.5) {
            (Some(b2), true) => b2,
            _ => w.lat_band,
        };
        let target_lat = rng.range(band[0], band[1]);
        if w.aquatic {
            crate::sim::niche_water_pos(rng, target_lat, y)
        } else {
            crate::sim::niche_pos(rng, !w.rocky, target_lat, y)
        }
    };

    for spec in &cfg.scenario.plant_cohort {
        // base genome: named archetype, else tree base for trees, else random
        for _ in 0..spec.count {
            let base = match spec.archetype.as_deref().and_then(archetype_by_name) {
                Some(a) => PlantGenome::archetype(&mut rng, a),
                None if spec.tree => crate::sim::tree_genome(&mut rng),
                None => PlantGenome::random(&mut rng, NFOOD as u8),
            };
            let mut g = apply_overrides(base, &spec.genome);
            let pos = place(&mut rng, crate::sim::FOOD_Y);
            // F45: in CREATURE scenario plant_cohort is just FOOD, not the tuning subject. Untuned archetype
            // climate defaults collapse in hostile band (Cactus/Reed died) -> creatures starve. Pin food
            // climate genes to THIS spot's band (unless spec overrode them) so food survives. Gated on
            // creature cohort present, so PLANT tuner (no creature_cohort, relies on temp_pref drift signal)
            // unaffected.
            if !cfg.scenario.creature_cohort.is_empty() {
                if !spec.genome.contains_key("temp_pref") {
                    g.temp_pref = crate::sphere::base_temperature(pos.normalize_or_zero());
                }
                if !spec.genome.contains_key("wet") {
                    g.wet = w.wetness.clamp(0.05, 0.95);
                }
            }
            stats.seeded.push(g.clone());
            if spec.tree {
                crate::sim::spawn_tree(&mut commands, rng.range(2.0, 5.0), pos, true, g);
            } else {
                crate::sim::spawn_plant(&mut commands, g, rng.range(0.5, 1.3) * crate::sim::PLANT_START_MASS, pos);
            }
        }
    }
    stats.started = stats.seeded.len();
    // cap cohort near target: grows toward goal + shows vigor, no boom to PLANT_CAP.
    stats.cap = (cfg.scenario.target_count * 2).max(20);

    // Creature cohort (M4): free-form overrides onto random base, rebuild net if sensors overridden (shape
    // must match), apply optional reflex prior, spawn into band.
    for spec in &cfg.scenario.creature_cohort {
        for _ in 0..spec.count {
            let mut g = apply_overrides_genome(Genome::random(&mut rng), &spec.genome);
            if spec.genome.contains_key("sensors") {
                g.rebuild_random_net(&mut rng);
            }
            if let Some(name) = spec.reflex.as_deref() {
                if let Some(net) = reflex_brain(name, &g.sensors) {
                    g.net = net;
                }
            }
            stats.cseeded.push(g.clone());
            let pos = place(&mut rng, crate::sim::CREATURE_Y);
            crate::sim::spawn_creature(&mut commands, g, pos, &mut rng, crate::sim::BIRTH_ENERGY);
        }
    }
    stats.cstarted = stats.cseeded.len();

    // grazing pressure: random creatures in band. Continuous off -> they don't reseed.
    for _ in 0..w.grazers {
        let pos = place(&mut rng, crate::sim::CREATURE_Y);
        crate::sim::spawn_creature(&mut commands, Genome::random(&mut rng), pos, &mut rng, crate::sim::BIRTH_ENERGY);
    }
}

// Result schema. Deserialize too: --merge CLI reads it back to build library entries.
#[derive(Serialize, Deserialize)]
pub struct BestGenome {
    pub genome: PlantGenome,
    pub mass: f32,
    pub tree: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ScenarioResult {
    pub seed: u64,
    pub ticks: u32,
    pub target_count: usize,
    pub started: usize,
    pub survived: usize,
    pub peak_count: usize,
    pub final_count: usize,
    pub reached_target: bool,
    pub mean_mass: f32,
    pub max_mass: f32,
    pub mean_age: f32,
    pub births: u32,
    pub deaths: u32,
    pub r: f32,
    pub mean_growth_rate: f32,
    pub deaths_by_cause: HashMap<String, u32>,
    pub trait_drift: HashMap<String, [f32; 2]>,
    pub health_score: f32,
    pub best_genomes: Vec<BestGenome>,
    // Creature arm (M4): all #[serde(default)] so plant-only results still parse.
    #[serde(default)]
    pub creature_started: usize,
    #[serde(default)]
    pub creature_survived: usize,
    #[serde(default)]
    pub creature_survival: f32, // survived / started: creature health signal tuner maximizes
    #[serde(default)]
    pub creature_mean_age: f32,
    #[serde(default)]
    pub creature_mean_energy: f32,
    #[serde(default)]
    pub creature_mean_master: f32, // mean digestion expression (diet fit to niche food)
    #[serde(default)]
    pub creature_trait_drift: HashMap<String, [f32; 2]>,
    #[serde(default)]
    pub best_creatures: Vec<Genome>, // top survivors by fitness (harvested into seed snapshot)
}

// Classify plant genome -> biome niche label from its OWN adapted prefs (temp_pref/wet/submerged/
// light_pref). Mirrors sim::plant_for_site biome logic. Used by whole-planet co-evolution harvest:
// survivor genome self-describes its adapted biome, so we tag without needing position.
fn biome_label(g: &PlantGenome) -> &'static str {
    if g.submerged > 0.5 || g.wet > 0.85 {
        if g.light_pref < 0.4 {
            "deep-kelp"
        } else {
            "shallow-sunlit"
        }
    } else if g.temp_pref < 0.34 {
        "polar-alpine"
    } else if g.wet < 0.32 {
        "arid-desert"
    } else if g.wet > 0.68 {
        "tropical-wet"
    } else {
        "temperate-meadow"
    }
}

// CLI --merge-snapshot: harvest surviving plants of whole-planet co-evolution run (a --save snapshot) into
// library, biome-labeled by each genome's prefs (+ `suffix`, e.g. "-coevo", so they coexist with isolated
// entries for comparison). Tree-like genomes (maturity in tree range) skipped: SavedPlant carries no Tree
// marker, can't re-seed as trees. score = mass -> merge keeps most vigorous per biome. persist.rs hooks.
pub fn merge_snapshot_into_library(snap_path: &str, lib_path: &str, per_niche_cap: usize, suffix: &str) {
    let snap = match crate::persist::load_snapshot(snap_path) {
        Some(s) => s,
        None => {
            println!("merge-snapshot: snapshot load failed ({})", snap_path);
            return;
        }
    };
    let entries: Vec<crate::persist::LibEntry> = snap
        .plants
        .iter()
        .filter(|sp| sp.g.maturity < 8.0) // skip tree-like (trees clamp maturity >= 8; ground plants <= 10)
        .map(|sp| crate::persist::LibEntry { niche: format!("{}{}", biome_label(&sp.g), suffix), tree: false, score: sp.mass, genome: sp.g.clone(), missing: Vec::new() })
        .collect();
    let added = entries.len();
    let mut lib = crate::persist::load_plant_library(lib_path).unwrap_or_default();
    lib.version = 1;
    lib.merge_in(entries, per_niche_cap);
    crate::persist::save_plant_library(lib_path, &lib);
    println!("merge-snapshot: harvested {} ground plants from {} -> {} now has {} entries", added, snap_path, lib_path, lib.entries.len());
}

// CLI --merge: fold scenario result's best survivor genomes into plant seed-bank library under `niche`,
// accumulating across runs (load -> merge -> keep best per niche -> save). Harness synthesize stage calls
// once per tuned cohort. Gene-agnostic end to end (genomes carry every gene). persist.rs hooks.
pub fn merge_result_into_library(result_path: &str, niche: &str, lib_path: &str, per_niche_cap: usize) {
    let text = match std::fs::read_to_string(result_path) {
        Ok(t) => t,
        Err(e) => {
            println!("merge: result read failed ({}): {}", result_path, e);
            return;
        }
    };
    let res: ScenarioResult = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            println!("merge: result parse failed ({}): {}", result_path, e);
            return;
        }
    };
    let entries: Vec<crate::persist::LibEntry> = res
        .best_genomes
        .into_iter()
        .map(|b| crate::persist::LibEntry { niche: niche.to_string(), tree: b.tree, score: res.health_score, genome: b.genome, missing: Vec::new() })
        .collect();
    let added = entries.len();
    let mut lib = crate::persist::load_plant_library(lib_path).unwrap_or_default();
    lib.version = 1;
    lib.merge_in(entries, per_niche_cap);
    crate::persist::save_plant_library(lib_path, &lib);
    println!("merge: +{} genomes for niche '{}' (score {:.2}) -> {} now has {} entries", added, niche, res.health_score, lib_path, lib.entries.len());
}

// CLI --merge-creatures: harvest scenario result's best survivor creatures into population SNAPSHOT (showcase
// seed, e.g. evolved-continuous.json), accumulating across runs (load -> append -> cap -> save). Creature
// synthesize stage calls once per tuned niche; capped multi-niche snapshot becomes fresh evolved seed.
// Gene-agnostic. On overflow keeps most-recently-added so later niches represented. persist.rs hooks.
pub fn merge_creatures_into_snapshot(result_path: &str, snap_path: &str, cap: usize) {
    let text = match std::fs::read_to_string(result_path) {
        Ok(t) => t,
        Err(e) => {
            println!("merge-creatures: result read failed ({}): {}", result_path, e);
            return;
        }
    };
    let res: ScenarioResult = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            println!("merge-creatures: result parse failed ({}): {}", result_path, e);
            return;
        }
    };
    let mut snap = crate::persist::load_snapshot(snap_path).unwrap_or(crate::persist::Snapshot { generation: 0, creatures: Vec::new(), plants: Vec::new(), world: None });
    let added = res.best_creatures.len();
    snap.creatures.extend(res.best_creatures);
    if snap.creatures.len() > cap {
        let excess = snap.creatures.len() - cap;
        snap.creatures.drain(0..excess); // keep newest (later niches), cap total for balanced seed
    }
    crate::persist::save_snapshot(snap_path, &snap);
    println!("merge-creatures: +{} from {} -> {} now has {} creatures", added, result_path, snap_path, snap.creatures.len());
}

// CLI --merge-snap: append one creature SNAPSHOT's creatures into another (gym cohort -> growing seed). The
// gym (--gym-evolve --save) writes a creatures-only Snapshot; the creature tune workflow's synthesize stage
// folds each niche cohort into the seed sequentially (no race). Keeps newest on overflow.
pub fn merge_snapshot_creatures(src_path: &str, dst_path: &str, cap: usize) {
    let src = match crate::persist::load_snapshot(src_path) {
        Some(s) => s,
        None => {
            println!("merge-snap: source load failed ({})", src_path);
            return;
        }
    };
    let mut dst = crate::persist::load_snapshot(dst_path).unwrap_or(crate::persist::Snapshot { generation: 0, creatures: Vec::new(), plants: Vec::new(), world: None });
    let added = src.creatures.len();
    dst.creatures.extend(src.creatures);
    if dst.creatures.len() > cap {
        let excess = dst.creatures.len() - cap;
        dst.creatures.drain(0..excess);
    }
    crate::persist::save_snapshot(dst_path, &dst);
    println!("merge-snap: +{} from {} -> {} now has {} creatures", added, src_path, dst_path, dst.creatures.len());
}

// Gene-agnostic per-field numeric means over genome set (serde reflection -> covers any new gene).
// Generic over genome type: serves both PlantGenome and Genome. Top-level f64 fields only; nested
// arrays/objects (net/sensors/uptake) skipped, which is exactly the scalar genes we want.
fn numeric_means<T: Serialize>(genomes: &[T]) -> HashMap<String, f32> {
    let mut sums: HashMap<String, f64> = HashMap::new();
    let n = genomes.len().max(1) as f64;
    for g in genomes {
        if let Ok(Value::Object(m)) = serde_json::to_value(g) {
            for (k, v) in m {
                if let Some(f) = v.as_f64() {
                    *sums.entry(k).or_insert(0.0) += f;
                }
            }
        }
    }
    sums.into_iter().map(|(k, s)| (k, (s / n) as f32)).collect()
}

// Per-tick: advance clock (day/night + season), track peak, write result + exit at budget.
pub fn scenario_step(
    mut gen: ResMut<GenState>,
    cfg: Res<ScenarioCfg>,
    mut stats: ResMut<ScenarioStats>,
    mut exit: MessageWriter<AppExit>,
    q: Query<(&PlantState, &PlantGenome, Option<&Tree>), (Without<crate::components::Rot>, Without<crate::components::Grass>)>,
    cq: Query<(&Genome, &DietState, &Energy, &Alive, &crate::components::Fitness), With<Creature>>,
) {
    gen.tick = gen.tick.wrapping_add(1); // drives daylight_at + season in plant_step

    let count = q.iter().count();
    if count > stats.peak_count {
        stats.peak_count = count;
    }

    if gen.tick < cfg.scenario.ticks {
        return;
    }

    // budget reached: gather metrics, write result JSON, exit.
    let mut survivors: Vec<PlantGenome> = Vec::new();
    let mut best: Vec<(f32, PlantGenome, bool)> = Vec::new();
    let (mut sum_mass, mut max_mass, mut sum_age, mut sum_growth) = (0.0f32, 0.0f32, 0.0f64, 0.0f32);
    for (st, g, tree) in &q {
        survivors.push(g.clone());
        sum_mass += st.mass;
        max_mass = max_mass.max(st.mass);
        sum_age += st.age as f64;
        sum_growth += g.growth_rate();
        best.push((st.mass, g.clone(), tree.is_some()));
    }
    let survived = survivors.len();
    let n = survived.max(1) as f32;
    let target = cfg.scenario.target_count.max(1);

    // trait drift: [seeded mean, survivor mean] per numeric gene (auto-covers new genes)
    let seeded_means = numeric_means(&stats.seeded);
    let survivor_means = numeric_means(&survivors);
    let mut trait_drift: HashMap<String, [f32; 2]> = HashMap::new();
    for (k, sm) in &seeded_means {
        let vm = survivor_means.get(k).copied().unwrap_or(*sm);
        trait_drift.insert(k.clone(), [*sm, vm]);
    }

    let r = stats.births as f32 / stats.deaths.max(1) as f32; // R = reproductive success over run
    // health_score 0..1: cohort FILLED toward target by end (sustained, not transient peak that crashed) AND
    // self-sustaining (R>=1). final_fill saturates at target, so cohort holding ~target with R>=1 scores
    // ~1.0; dying back or barely surviving scores low. Bounded so candidates rank cleanly (old
    // survived/started ratio overflowed once cohort reproduced).
    let final_fill = (survived as f32 / target as f32).min(1.0);
    let r_term = 0.5 + 0.5 * r.min(1.0);
    let health_score = final_fill * r_term;

    best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let best_genomes: Vec<BestGenome> = best.into_iter().take(12).map(|(mass, genome, tree)| BestGenome { genome, mass, tree }).collect();

    // creature cohort metrics: survivors = creatures alive at budget. Best (by fitness) harvested into seed
    // snapshot by --merge-creatures.
    let mut csurv: Vec<Genome> = Vec::new();
    let mut cbest: Vec<(f32, Genome)> = Vec::new();
    let (mut csum_age, mut csum_e, mut csum_master) = (0.0f64, 0.0f32, 0.0f32);
    for (g, diet, en, alive, fit) in &cq {
        if !alive.0 {
            continue;
        }
        csum_age += diet.age as f64;
        csum_e += en.total();
        csum_master += master_expression(&g.uptake, &diet.reserves, RESERVE_REQ, MASTER_FLOOR);
        cbest.push((fit.0, g.clone()));
        csurv.push(g.clone());
    }
    let creature_survived = csurv.len();
    let cn = creature_survived.max(1) as f32;
    let cseeded_means = numeric_means(&stats.cseeded);
    let csurv_means = numeric_means(&csurv);
    let mut creature_trait_drift: HashMap<String, [f32; 2]> = HashMap::new();
    for (k, sm) in &cseeded_means {
        let vm = csurv_means.get(k).copied().unwrap_or(*sm);
        creature_trait_drift.insert(k.clone(), [*sm, vm]);
    }
    cbest.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    // harvest up to 50 top survivors (was 12): diversity runs want 30+ varieties per niche merged into the seed.
    let best_creatures: Vec<Genome> = cbest.into_iter().take(50).map(|(_, g)| g).collect();
    let creature_survival = if stats.cstarted > 0 { creature_survived as f32 / stats.cstarted as f32 } else { 0.0 };

    let result = ScenarioResult {
        seed: cfg.scenario.seed,
        ticks: cfg.scenario.ticks,
        target_count: cfg.scenario.target_count,
        started: stats.started,
        survived,
        peak_count: stats.peak_count,
        final_count: survived,
        reached_target: stats.peak_count >= target,
        mean_mass: sum_mass / n,
        max_mass,
        mean_age: (sum_age / n as f64) as f32,
        births: stats.births,
        deaths: stats.deaths,
        r,
        mean_growth_rate: sum_growth / n,
        deaths_by_cause: stats.deaths_by_cause.clone(),
        trait_drift,
        health_score,
        best_genomes,
        creature_started: stats.cstarted,
        creature_survived,
        creature_survival,
        creature_mean_age: (csum_age / cn as f64) as f32,
        creature_mean_energy: csum_e / cn,
        creature_mean_master: csum_master / cn,
        creature_trait_drift,
        best_creatures,
    };

    match serde_json::to_string_pretty(&result) {
        Ok(s) => match std::fs::write(&cfg.out, s) {
            Ok(()) => info!("scenario done: {}/{} survived, peak {}, health {:.2} -> {}", survived, stats.started, stats.peak_count, health_score, cfg.out),
            Err(e) => error!("scenario result write failed ({}): {}", cfg.out, e),
        },
        Err(e) => error!("scenario result serialize failed: {}", e),
    }
    exit.write(AppExit::Success);
}
