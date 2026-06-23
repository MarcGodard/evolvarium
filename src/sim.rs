// Core sim: creatures sense nearest food, tiny NN drives thrust+turn, eat, burn energy, starve.
// Generational GA selects by food eaten. Spec 08 (M1 proof-of-life origin).
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::components::{Alive, Brain, Creature, DietState, Energy, Ferment, Fitness, Food, Grass, Heading, Locomotion, Rot, Seaweed, Seed, Tree};
use crate::genome::{forward, learn, master_expression, Genome, CONE_HALF, GLOBAL_INPUTS, NFOOD, NUTRIENTS, SIG_PER_SENSOR};
use crate::plant::{PlantGenome, PlantState, P_REPRO, PLANT_CAP, PLANT_MIN, WHOLE_PLANET_SEED_MULT};
use crate::rng::Rng;

// Tuning constants in config.rs. Re-exported so `sim::FOO` refs resolve.
pub use crate::config::*;

// Day/night is POSITIONAL: sphere::daylight_at per creature/plant. No global daylight.

// 3D sphere pos -> pseudo-planar grid coords in [-WORLD_HALF, WORLD_HALF] (lon -> u, lat -> v).
// Lets 2D fertility/water/fire/food grids index the globe by lon/lat.
fn grid_uv(pos: Vec3) -> (f32, f32) {
    let (lon, lat) = crate::sphere::dir_to_lonlat(pos.normalize_or_zero());
    (lon / std::f32::consts::PI * WORLD_HALF, lat / std::f32::consts::FRAC_PI_2 * WORLD_HALF)
}

#[derive(Resource)]
pub struct GenState {
    pub generation: u32,
    pub ticks_left: u32,
    pub headless: bool,
    pub learn: bool,     // lifetime learning on/off (A/B vs baseline)
    pub poison: bool,    // legacy --poison: two food types (ntypes=2)
    pub diet: bool,      // epigenetic diet model (NFOOD types, expression, growth-load, disease)
    pub continuous: bool, // --continuous reproduction vs discrete generational GA (default)
    pub tick: u32,       // global tick clock. Drives season + continuous logging/stop.
    pub seed: u64,       // run seed (mirrors Rng seed). Parallel decide phases derive per-entity RNG from (seed, entity.index, tick) -> order-independent determinism. PARALLELIZATION.md.
    pub max_gens: u32,   // --gens=N run length. Continuous uses N*GEN_TICKS.
    pub save: Option<String>, // --save=PATH: write survivors at headless run end
    pub load: Option<String>, // --load=PATH: resume saved population vs random
    pub diverse: bool,        // --diverse: hand-seed niche-adapted creatures globe-wide (multi-niche showcase)
    pub mating: bool,         // --mating: offspring = crossover of 2 nearby similar parents (assortative -> speciation); else single-parent budding
    pub garden: bool,         // --garden: botanical SHOWCASE grid at homeland (one of every species + sample trees) vs random world. Flora inspection.
    pub plant_lib: Option<String>, // tuned plant seed-bank path. Present -> seed planet biome-matched FROM it. None = --no-plant-lib (archetype seeding).
    pub until_sustain: bool,   // --until-sustain: headless run stops when all niches self-sustain (no rescue for NICHE_SUSTAIN_WINDOW), not at --gens. Saves best snapshot.
    pub metrics: Option<String>, // --metrics=PATH: on headless run end, write niche balance result JSON (harness reads it to score config tweaks)
}

impl GenState {
    // Distinct food types this mode.
    pub fn ntypes(&self) -> u8 {
        if self.diet { NFOOD as u8 } else if self.poison { 2 } else { 1 }
    }
}

// Dynamic soil-fertility grid (M5 closed loop): corpses deposit here, plants read for growth boost,
// decays each tick. Coarse grid so deposits stay spatially local.
#[derive(Resource)]
pub struct Soil {
    pub cell: Vec<f32>,
}

impl Soil {
    pub fn new() -> Self {
        Soil { cell: vec![0.0; SOIL_RES * SOIL_RES] }
    }
    fn index(pos: Vec3) -> usize {
        let (u, v) = grid_uv(pos);
        let to_cell = |w: f32| {
            (((w + WORLD_HALF) / (2.0 * WORLD_HALF)) * SOIL_RES as f32).clamp(0.0, (SOIL_RES - 1) as f32) as usize
        };
        to_cell(v) * SOIL_RES + to_cell(u)
    }
    fn add(&mut self, pos: Vec3, amt: f32) {
        self.cell[Self::index(pos)] += amt;
    }
    fn get(&self, pos: Vec3) -> f32 {
        self.cell[Self::index(pos)]
    }
    fn decay(&mut self) {
        for c in &mut self.cell {
            *c *= SOIL_DECAY;
        }
    }
    pub fn avg(&self) -> f32 {
        self.cell.iter().sum::<f32>() / self.cell.len() as f32
    }
}

// Mass stripped per fruit tree this tick. live_step records, plant_step applies. Trees persist + regrow;
// die only when grazed below TREE_MIN_MASS (over-eaten).
#[derive(Resource, Default)]
pub struct TreeBites(pub HashMap<Entity, f32>);

// Rainfall intensity 0..1 (0 dry, 1 heavy). Storms onset stochastically + decay.
#[derive(Resource, Default)]
pub struct Weather {
    pub rain: f32,
}

// Seed bank (dormancy gene): buried seeds that germinate LATER. Each entry counts down `ticks`; at 0
// plant_step spawns it. Independent of surface plants, so a patch wiped by fire/drought/grazing re-greens
// from buried seeds (boom after disturbance). Capped at SEED_BANK_CAP.
#[derive(Resource, Default)]
pub struct SeedBank(pub Vec<(PlantGenome, Vec3, u32)>); // (genome, surface pos, ticks until germination)

// Dynamic ground-water grid (rain cycle): per-cell wetness 0..1 over static terrain moisture. Sun evaporates,
// rain refills (rocky cells shed runoff). Coarse grid so wetness stays spatially local.
#[derive(Resource)]
pub struct GroundWater {
    pub cell: Vec<f32>,
}

// Surface pos of grid cell center (inverse of Soil::index). Lets weather sample terrain per cell.
fn cell_center(c: usize) -> Vec3 {
    let (cx, cz) = (c % SOIL_RES, c / SOIL_RES);
    let to_uv = |k: usize| ((k as f32 + 0.5) / SOIL_RES as f32) * 2.0 * WORLD_HALF - WORLD_HALF;
    let (u, v) = (to_uv(cx), to_uv(cz));
    let lon = u / WORLD_HALF * std::f32::consts::PI;
    let lat = v / WORLD_HALF * std::f32::consts::FRAC_PI_2;
    crate::sphere::lonlat_to_pos(lon, lat, 0.0) // dir * PLANET_R; callers sample fields by direction
}

// Surface pos (on terrain) of grid cell `c`. Render places fire/effects on the globe.
pub fn grid_cell_surface(c: usize) -> Vec3 {
    crate::sphere::surface_pos(cell_center(c).normalize_or_zero(), 0.0)
}

// Food spatial grid (perf): bin foods into FGRID^2 cells -> creature scans nearby cells, not all ~1900 foods.
// NEAR_QUERY = min query radius so global-nearest always found (plants dense -> nearest within a few units).
const FGRID: usize = 20;
const NEAR_QUERY: f32 = 24.0;
fn fcell(w: f32) -> usize {
    (((w + WORLD_HALF) / (2.0 * WORLD_HALF)) * FGRID as f32).clamp(0.0, (FGRID - 1) as f32) as usize
}
// Food-grid cell (lon, lat bins) for a 3D sphere pos.
fn fcell_uv(pos: Vec3) -> (usize, usize) {
    let (u, v) = grid_uv(pos);
    (fcell(u), fcell(v))
}

impl GroundWater {
    pub fn new() -> Self {
        GroundWater { cell: vec![0.0; SOIL_RES * SOIL_RES] }
    }
    fn index(pos: Vec3) -> usize {
        let (u, v) = grid_uv(pos);
        let to_cell = |w: f32| {
            (((w + WORLD_HALF) / (2.0 * WORLD_HALF)) * SOIL_RES as f32).clamp(0.0, (SOIL_RES - 1) as f32) as usize
        };
        to_cell(v) * SOIL_RES + to_cell(u)
    }
    pub fn get(&self, pos: Vec3) -> f32 {
        self.cell[Self::index(pos)]
    }
    pub fn avg(&self) -> f32 {
        self.cell.iter().sum::<f32>() / self.cell.len() as f32
    }
}

// Slow climate-memory grid (geological): per-cell long-term moisture 0..1. GroundWater is fast (wets on rain,
// dries in hours); Climate low-pass-filters the drifting rain-propensity target over MONTHS, so persistently
// dry regions drift to desert + wet ones to lush, wet belt migrates over years. Drives plant growth/mortality
// (sim) + globe recolor (render). Same SOIL_RES grid.
#[derive(Resource)]
pub struct Climate {
    pub cell: Vec<f32>,
}

impl Climate {
    pub fn new() -> Self {
        // seed each cell at STATIC moisture baseline -> world starts as it looks today (no cold-start shock).
        // Climate then diverges as the rain-propensity anomaly drifts.
        let cell = (0..SOIL_RES * SOIL_RES)
            .map(|c| crate::sphere::moisture(cell_center(c).normalize_or_zero()))
            .collect();
        Climate { cell }
    }
    pub fn get(&self, pos: Vec3) -> f32 {
        self.cell[GroundWater::index(pos)]
    }
    // Bilinear climate moisture at dir `d` (lon wraps, lat clamps at poles). Smooths coarse 32x32 grid
    // -> globe recolor shows soft biome edges, not blocky cells.
    pub fn sample(&self, d: Vec3) -> f32 {
        let (u, v) = grid_uv(d);
        let n = SOIL_RES as i32;
        let fx = ((u + WORLD_HALF) / (2.0 * WORLD_HALF)) * SOIL_RES as f32 - 0.5; // cell centers at +0.5
        let fy = ((v + WORLD_HALF) / (2.0 * WORLD_HALF)) * SOIL_RES as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let wrapx = |i: i32| (((i % n) + n) % n) as usize; // longitude wraps
        let clampy = |j: i32| j.clamp(0, n - 1) as usize; // latitude clamps at poles
        let (x0i, x1i) = (wrapx(x0 as i32), wrapx(x0 as i32 + 1));
        let (y0i, y1i) = (clampy(y0 as i32), clampy(y0 as i32 + 1));
        let at = |cx: usize, cy: usize| self.cell[cy * SOIL_RES + cx];
        let top = at(x0i, y0i) * (1.0 - tx) + at(x1i, y0i) * tx;
        let bot = at(x0i, y1i) * (1.0 - tx) + at(x1i, y1i) * tx;
        top * (1.0 - ty) + bot * ty
    }
    pub fn avg(&self) -> f32 {
        self.cell.iter().sum::<f32>() / self.cell.len() as f32
    }
    // Driest/wettest cell (0..1). Headless logging of desert/forest spread.
    pub fn range(&self) -> (f32, f32) {
        let mut lo = 1.0f32;
        let mut hi = 0.0f32;
        for &v in &self.cell {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo, hi)
    }
    // Fraction of LAND cells arid (climate moisture below `thr`) -> desert coverage. Climbs/falls over a long
    // run as deserts form + rainforests green with climate drift.
    pub fn land_arid_frac(&self, thr: f32) -> f32 {
        let mut land = 0u32;
        let mut arid = 0u32;
        for (c, &v) in self.cell.iter().enumerate() {
            let d = cell_center(c).normalize_or_zero();
            if crate::sphere::is_ocean(d) {
                continue;
            }
            land += 1;
            if v < thr {
                arid += 1;
            }
        }
        if land == 0 { 0.0 } else { arid as f32 / land as f32 }
    }
}

// weather (rain cycle): advance rainfall + update ground-water grid (sun dries, rain refills).
pub fn weather_step(
    gen: Res<GenState>,
    mut weather: ResMut<Weather>,
    mut gw: ResMut<GroundWater>,
    mut climate: ResMut<Climate>,
) {
    let _g = crate::profile::scope("weather");
    let dt = DT;
    let tick = gen.tick;
    // Rain is LOCAL + cloud-driven: each cell wets only when a rain cloud drifts over it (sun dries otherwise).
    // No global storms. weather.rain = peak rain anywhere (lightning gating + logs); local rain per cell drives
    // ground water + viz.
    let mut peak = 0.0f32;
    for c in 0..gw.cell.len() {
        let cpos = cell_center(c);
        let d = cpos.normalize_or_zero();
        let rain = crate::sphere::rain_at(d, tick);
        let light = crate::sphere::daylight_at(d, tick);
        let absorb = 1.0 - crate::sphere::rockiness(cpos); // rocky sheds runoff, grassy soaks it up
        let w = gw.cell[c];
        let add = rain * absorb * RAIN_RATE * dt;
        let evap = EVAP * (0.2 + 0.8 * light) * w * dt; // sun dries ground; fastest at noon
        gw.cell[c] = (w + add - evap).clamp(0.0, 1.0);
        peak = peak.max(rain);
    }
    weather.rain = peak;
    // slow climate memory: relax each cell toward its drifting long-run target on a months time constant.
    // CLIMATE_RATE * dt tiny per tick -> grid integrates rain-propensity over many days, so regions desertify /
    // reforest gradually + wet belt migrates as target anomaly rotates.
    for c in 0..climate.cell.len() {
        let d = cell_center(c).normalize_or_zero();
        let target = crate::sphere::climate_target(d, tick);
        let cur = climate.cell[c];
        climate.cell[c] = (cur + (target - cur) * CLIMATE_RATE * dt).clamp(0.0, 1.0);
    }
}

// Fire grid (lightning wildfires): per-cell burn intensity 0..1. Shares soil/ground-water grid res + indexing.
#[derive(Resource)]
pub struct Fire {
    pub cell: Vec<f32>,
}

impl Fire {
    pub fn new() -> Self {
        Fire { cell: vec![0.0; SOIL_RES * SOIL_RES] }
    }
    pub fn get(&self, pos: Vec3) -> f32 {
        self.cell[GroundWater::index(pos)]
    }
    pub fn avg(&self) -> f32 {
        self.cell.iter().sum::<f32>() / self.cell.len() as f32
    }
}

// fire (lightning): storm strikes ignite dry cells; fire spreads through dry fuel, doused by rain/wet ground,
// deposits ash fertility. plant_step burns vegetation, live_step burns creatures.
pub fn fire_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    weather: Res<Weather>,
    gw: Res<GroundWater>,
    mut fire: ResMut<Fire>,
    mut soil: ResMut<Soil>,
) {
    let _g = crate::profile::scope("fire");
    let _ = gen;
    // Flammability: ocean = no fuel (is_ocean -> 0), polar ice cap = firebreak (sphere::fuel gates 0 across
    // ice-temp band). Ignition + spread stay on vegetated non-frozen land. FIRE_ENABLED=false disables wildfire
    // (also kills the L god-control).
    const FIRE_ENABLED: bool = true;
    if !FIRE_ENABLED {
        if fire.cell.iter().any(|&f| f > 0.0) {
            fire.cell.iter_mut().for_each(|f| *f = 0.0);
        }
        return;
    }
    let dt = DT;
    let n = SOIL_RES;
    // Min flammable vegetation to ignite/carry fire. Ocean/bare rock/desert/ice ~0 fuel -> never burn, act as
    // firebreaks (strike fizzles, fire can't cross). 0.45: tuned so only solidly-vegetated land carries fire
    // -> more firebreaks, fires stay local.
    const FUEL_MIN: f32 = 0.45;
    // lightning: in a storm, occasionally strike VEGETATED dry ground (wet ground/water/rock/desert won't catch).
    // Sample a few cells, strike the one with most dry fuel.
    if weather.rain > LIGHTNING_RAIN && rng.f32() < P_LIGHTNING {
        let mut best: Option<usize> = None;
        let mut best_fuel = FUEL_MIN;
        for _ in 0..12 {
            let c = (rng.f32() * (n * n) as f32) as usize % (n * n);
            let pos = cell_center(c);
            let fuel = crate::sphere::fuel(pos.normalize_or_zero());
            if fuel > best_fuel && gw.get(pos) < FIRE_WET_MAX {
                best_fuel = fuel;
                best = Some(c);
            }
        }
        if let Some(c) = best {
            fire.cell[c] = 1.0; // struck dry vegetated fuel -> catches
        }
    }
    if fire.cell.iter().all(|&f| f <= 0.02) {
        return; // nothing burning, skip the sweep
    }
    let cur = fire.cell.clone(); // spread reads pre-tick state (snapshot)
    #[allow(clippy::needless_range_loop)] // c indexes cur + fire.cell AND derives grid neighbors (c%n, c/n)
    for c in 0..cur.len() {
        let f = cur[c];
        if f <= 0.02 {
            continue;
        }
        let cpos = cell_center(c);
        let wet = gw.get(cpos);
        fire.cell[c] = (f - (FIRE_DECAY + FIRE_DOUSE * wet) * dt).max(0.0); // decay + rain douses
        soil.add(cpos, FIRE_ASH * f * dt); // ash enriches the burned ground
        // spread to the 4 orthogonal neighbours that are dry enough to catch
        let (cxi, czi) = (c % n, c / n);
        let mut nbrs = [usize::MAX; 4];
        let mut k = 0;
        if cxi > 0 { nbrs[k] = czi * n + cxi - 1; k += 1; }
        if cxi < n - 1 { nbrs[k] = czi * n + cxi + 1; k += 1; }
        if czi > 0 { nbrs[k] = (czi - 1) * n + cxi; k += 1; }
        if czi < n - 1 { nbrs[k] = (czi + 1) * n + cxi; k += 1; }
        for &ni in &nbrs[..k] {
            let npos = cell_center(ni);
            // spread only into flammable dry-enough land; water/rock/desert/ice/wet are firebreaks. Rate scales
            // with NEIGHBOR fuel density: lush forest catches fast, sparse scrub barely carries -> fires stay in
            // dense vegetation.
            let fuel_n = crate::sphere::fuel(npos.normalize_or_zero());
            if fuel_n > FUEL_MIN && gw.get(npos) < FIRE_WET_MAX {
                fire.cell[ni] = (fire.cell[ni] + FIRE_SPREAD * f * fuel_n * dt).min(1.0);
            }
        }
    }
}

// Founding homeland: initial life starts within HOMELAND_CAP radians of this dir, then spreads globe-wide by
// reproduction/dispersal.
pub fn homeland_center() -> Vec3 {
    Vec3::from(crate::sphere::HOMELAND_DIR).normalize()
}
pub const HOMELAND_CAP: f32 = 0.45; // ~26 deg cap: continent-sized starting region

// Random LAND surface pos in the homeland, `offset` above terrain (initial spawn).
fn homeland_pos(rng: &mut Rng, offset: f32) -> Vec3 {
    let mut d = crate::sphere::random_dir_in_cap(rng, homeland_center(), HOMELAND_CAP);
    for _ in 0..8 {
        if !crate::sphere::is_ocean(d) {
            break;
        }
        d = crate::sphere::random_dir_in_cap(rng, homeland_center(), HOMELAND_CAP);
    }
    crate::sphere::surface_pos(d, offset)
}

// Offspring pos: great-circle hop up to `spread` from parent in a random dir, on the surface. How
// plants/trees/creatures spread the globe.
fn disperse_pos(rng: &mut Rng, parent: Vec3, spread: f32, offset: f32) -> Vec3 {
    let heading = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
    let (d, _h) = crate::sphere::step(parent, heading, rng.range(0.0, spread.max(0.001)));
    crate::sphere::surface_pos(d, offset)
}

// Effective seed dispersal dist: genome `spread` STRETCHED by windborne, DRAGGED by seed_weight. Light+windborne
// -> far (dandelion/samara); heavy -> near parent (acorn). Kept >=0.5 so seed clears the parent stem. Shared by
// plants + trees (animal-carry multiplies further).
fn eff_spread(g: &PlantGenome) -> f32 {
    (g.spread * (1.0 + WIND_RANGE * g.windborne) * (1.0 - SEED_DRAG * g.seed_weight)).max(0.5)
}

// Plant genetic distance (species concept, --mating). 0 = identical, larger = diverged. `kind` mismatch alone
// (0.5) exceeds PLANT_SPECIES_SIM -> different food families never cross (hard gate). Within a family, mean gap
// over ecologically meaningful genes. Under PLANT_SPECIES_SIM = same species (cross-compatible); drift past ->
// reproductive isolation -> speciation.
fn plant_gene_dist(a: &PlantGenome, b: &PlantGenome) -> f32 {
    let kind = if a.kind != b.kind { 0.5 } else { 0.0 };
    let pairs = [
        (a.nutrient, b.nutrient), (a.defense, b.defense), (a.quality, b.quality), (a.wet, b.wet),
        (a.height, b.height), (a.light_pref, b.light_pref), (a.temp_pref, b.temp_pref),
        (a.submerged, b.submerged), (a.fruiting, b.fruiting), (a.toxicity, b.toxicity),
    ];
    let sum: f32 = pairs.iter().map(|(x, y)| (x - y).abs()).sum();
    kind + sum / pairs.len() as f32
}

// Nearest cross-compatible mate for a seeding plant: same repro class (plant vs tree), within PLANT_MATE_RADIUS,
// gene dist under PLANT_SPECIES_SIM, not self. None -> selfing. pool = (entity, pos, is_tree, genome), built
// once per tick only in --mating mode.
fn find_plant_mate<'a>(
    pool: &'a [(Entity, Vec3, bool, PlantGenome)],
    me: Entity,
    pos: Vec3,
    g: &PlantGenome,
    is_tree: bool,
) -> Option<&'a PlantGenome> {
    let r2 = PLANT_MATE_RADIUS * PLANT_MATE_RADIUS;
    pool.iter()
        .filter(|(e, p, t, mg)| {
            *e != me && *t == is_tree && pos.distance_squared(*p) < r2 && plant_gene_dist(g, mg) < PLANT_SPECIES_SIM
        })
        .min_by(|a, b| {
            pos.distance_squared(a.1).partial_cmp(&pos.distance_squared(b.1)).unwrap()
        })
        .map(|(_, _, _, mg)| mg)
}

// Child genome for a SEEDING plant/tree: --mating crosses with nearest compatible mate (else self), then mutate.
// Empty pool (no --mating) -> always selfing (single-parent path). is_tree routes mate class + mutation kind
// (mutate_tree vs mutate).
fn mate_or_self(
    pool: &[(Entity, Vec3, bool, PlantGenome)],
    me: Entity,
    pos: Vec3,
    g: &PlantGenome,
    is_tree: bool,
    rng: &mut Rng,
) -> PlantGenome {
    let mut child = match (pool.is_empty(), find_plant_mate(pool, me, pos, g, is_tree)) {
        (false, Some(mate)) => PlantGenome::crossover(g, mate, rng),
        _ => g.clone(),
    };
    if is_tree {
        child.mutate_tree(rng);
    } else {
        child.mutate(rng);
    }
    child
}

// Random LAND surface pos anywhere on the globe, `offset` above terrain (dispersal/reseed).
fn rand_pos(rng: &mut Rng, offset: f32) -> Vec3 {
    let mut d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    for _ in 0..8 {
        if !crate::sphere::is_ocean(d) {
            break;
        }
        d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    }
    crate::sphere::surface_pos(d, offset)
}

// Plant spawn pos: land OR shallow coastal water (avoids only DEEP ocean) -> aquatic flora seeds shallows
// (food base for swimmer/fish niche). `homeland` clusters it with founding pop.
fn plant_spawn_pos(rng: &mut Rng, homeland: bool, offset: f32) -> Vec3 {
    let (center, cap) = if homeland {
        (homeland_center(), HOMELAND_CAP)
    } else {
        (Vec3::Y, std::f32::consts::PI)
    };
    let mut d = crate::sphere::random_dir_in_cap(rng, center, cap);
    for _ in 0..8 {
        if crate::sphere::elevation01(d) >= crate::sphere::AQUATIC_FLOOR {
            break; // land or non-abyssal water OK; only deep ocean rejected
        }
        d = crate::sphere::random_dir_in_cap(rng, center, cap);
    }
    crate::sphere::surface_pos(d, offset)
}

// hand-seeded DIVERSE world (--diverse): niche-adapted creatures in matching regions -> showcase starts with
// coexisting niches (swimmers in wet coast, cold at poles, warm grazers at equator, tall browsers temperate)
// vs one converged winner. Genes overridden per niche; brain kept from a competent base genome so they forage
// + survive.

// LAND pos matching a niche: low (wet/coastal) vs high ground, near a target |latitude|.
pub(crate) fn niche_pos(rng: &mut Rng, low_elev: bool, target_lat: f32, offset: f32) -> Vec3 {
    for _ in 0..50 {
        let d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
        if crate::sphere::is_ocean(d) {
            continue;
        }
        let (_lon, lat) = crate::sphere::dir_to_lonlat(d);
        let elev = crate::sphere::elevation(d);
        let elev_ok = if low_elev { elev < 3.0 } else { elev >= 1.0 };
        if (lat.abs() - target_lat).abs() < 0.3 && elev_ok {
            return crate::sphere::surface_pos(d, offset);
        }
    }
    rand_pos(rng, offset) // fallback: any land
}

// SHALLOW-WATER pos for swimmer niche: in sea but ABOVE the barren abyss, where aquatic flora grows -> swimmers
// spawn on their food. (niche_pos put them on dry land, swim costs SWIM_LAND_COST -> aquatic always starved out.)
pub(crate) fn niche_water_pos(rng: &mut Rng, target_lat: f32, offset: f32) -> Vec3 {
    for _ in 0..120 {
        let d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
        let e = crate::sphere::elevation01(d);
        if e >= crate::sphere::SEA_LEVEL || e < crate::sphere::AQUATIC_FLOOR {
            continue; // want submerged-but-not-abyssal (flora band)
        }
        let (_lon, lat) = crate::sphere::dir_to_lonlat(d);
        if (lat.abs() - target_lat).abs() < 0.45 {
            return crate::sphere::surface_pos(d, offset);
        }
    }
    // fallback: any shallow-water cell with food
    for _ in 0..240 {
        let d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
        let e = crate::sphere::elevation01(d);
        if e < crate::sphere::SEA_LEVEL && e >= crate::sphere::AQUATIC_FLOOR {
            return crate::sphere::surface_pos(d, offset);
        }
    }
    rand_pos(rng, offset)
}

// Override `g` trait genes for niche `i%5`, return matching spawn pos. Keeps g's brain/sensors.
fn diverse_creature(mut g: Genome, i: usize, rng: &mut Rng) -> (Genome, Vec3) {
    // niches 0,1 = swimmers -> seed IN shallow water (on aquatic flora); rest are land niches.
    match i % 5 {
        0 => {
            g.swim = 0.9; g.temp_pref = 0.85; g.height = 0.3; // warm-sea swimmer ("fish")
            (g, niche_water_pos(rng, 0.15, CREATURE_Y))
        }
        1 => {
            g.swim = 0.9; g.temp_pref = 0.25; g.height = 0.3; // cool-sea swimmer
            (g, niche_water_pos(rng, 0.8, CREATURE_Y))
        }
        2 => { g.swim = 0.1; g.temp_pref = 0.85; g.height = 0.25; (g, niche_pos(rng, true, 0.15, CREATURE_Y)) } // warm land grazer
        3 => { g.swim = 0.1; g.temp_pref = 0.15; g.height = 0.25; (g, niche_pos(rng, false, 1.05, CREATURE_Y)) } // cold highland
        _ => { g.swim = 0.1; g.temp_pref = 0.5; g.height = 0.9; (g, niche_pos(rng, false, 0.5, CREATURE_Y)) }    // tall browser
    }
}

// Spawn pos for a LOADED (already-evolved) creature: scatter planet-wide into a spot MATCHING its OWN genome
// (saved pop repopulates the whole globe, not just homeland), WITHOUT overwriting genes (unlike diverse_creature).
// Lat from temp_pref (warm -> equator, cold -> poles); swimmers -> shallow water; alpine -> highland.
pub(crate) fn loaded_creature_pos(g: &Genome, rng: &mut Rng) -> Vec3 {
    let lat = ((1.0 - g.temp_pref) * 1.3).clamp(0.0, 1.45); // warm pref -> low |lat|, cold -> high |lat|
    if g.swim > 0.6 {
        niche_water_pos(rng, lat, CREATURE_Y)
    } else {
        niche_pos(rng, g.alpine < 0.5, lat, CREATURE_Y) // alpine -> highland, else lowland
    }
}

fn diet_state(_g: &Genome) -> DietState {
    // newborns start with reserves stocked to satisfaction (RESERVE_REQ) -> not instantly deficient
    DietState { reserves: [RESERVE_REQ; NUTRIENTS], g: 0.0, age: 0, fatigue: 0.0, starve: 0, toxic_load: 0.0 }
}

// Eat plant matter: absorb each nutrient into reserves (delivered = plant baseline x soil fertility, gated by
// uptake gene for that nutrient), capped. Returns toxin to burn (plant toxicity x amount). Reserves later gate
// master expression gene + drive deficiency (live_step).
fn absorb_and_toxin(reserves: &mut [f32; NUTRIENTS], uptake: &[f32; NUTRIENTS], pg: &PlantGenome, soil_f: f32, amount: f32) -> f32 {
    for i in 0..NUTRIENTS {
        reserves[i] = (reserves[i] + pg.nutrients[i] * soil_f * uptake[i] * NUTRIENT_ABSORB).min(RESERVE_CAP);
    }
    pg.toxicity * PLANT_TOX_HIT * amount
}

// Spawn carrion (meat): a Food entity with Rot clock. Used by death + predation kills. `fat` 0..1 = carcass
// fattiness (prey fat reserves at death), carried in the otherwise-unused `quality` field so the eat branch can
// model rabbit starvation: lean meat (low fat) = mostly protein, needs eater carbs to use; fat carcass feeds direct.
fn spawn_carrion(commands: &mut Commands, pos: Vec3, mass: f32, fat: f32) {
    let p = crate::sphere::surface_pos(pos, FOOD_Y); // carrion lies on surface at death spot
    commands.spawn((
        Food,
        PlantState { mass, age: 0 },
        PlantGenome {
            kind: CARRION_KIND,
            nutrient: CARRION_NUTRIENT,
            defense: 0.0,    // no bite-defense: easy to scavenge while fresh
            quality: fat.clamp(0.0, 1.0), // FAT content (prey fatness at death) -> rabbit-starvation model
            wet: 0.5,        // unused (excluded from moisture mortality by Without<Rot>)
            height: 0.0,     // lies on ground
            light_pref: 0.5,
            regrow: 0.0,
            branches: 0.0,
            spread: 0.0,
            maturity: 999.0, // never reproduces via plant_step (also excluded by Without<Rot>)
            nutrients: [0.0; NUTRIENTS], // unused: meat tops reserves flat (balanced tissue) in a separate eat branch
            toxicity: 0.0,
            fruit_toxicity: 0.0,
            // remaining genes neutral/off: carrion never grows, reproduces, or renders as a plant
            temp_pref: 0.5,
            succulence: 0.0,
            submerged: 0.0,
            fruiting: 0.0,
            nitrogen_fix: 0.0,
            fire_seed: 0.0,
            climb: 0.0,
            allelopathy: 0.0,
            seed_weight: 0.0, // never disperses
            windborne: 0.0,
            clonal: 0.0,
            cling: 0.0,
            dormancy: 0.0,
            hydrochory: 0.0,
            form: crate::plant::form::HERB,
            flower: 0.0,
            flower_hue: 0.5,
            flower_sat: 0.5,
            flower_light: 0.5,
            leaf_hue: 0.5,
            bushiness: 0.5,
            droop: 0.0,
        },
        Rot { age: 0 },
        Transform::from_translation(p),
    ));
}

// Spawn one creature (no render mesh; viz::add_creature_visuals gives one in render mode). Continuous-mode
// offspring; fresh brain from genome priors, learns over its own life.
pub(crate) fn spawn_creature(commands: &mut Commands, g: Genome, pos: Vec3, rng: &mut Rng, birth_energy: f32) {
    // migrate older saved nets to current brain-input width before Brain copies weights (shape mismatch ->
    // out-of-bounds index in forward()). No-op for fresh genomes + births.
    let mut g = g;
    g.ensure_net_shape();
    let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
    let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY, attack: 0.0, defend: 0.0, fight_reward: 0.0 };
    let diet = diet_state(&g);
    commands.spawn((
        Creature,
        g,
        brain,
        diet,
        Energy::from_total(birth_energy),
        Fitness(0.0),
        Heading(h),
        Alive(true),
        Locomotion { start: pos, path: 0.0, alt: 0.0 },
        Transform::from_translation(pos),
    ));
}

// God-control (render B button): drop `n` creatures scattered over land. Clones of random LIVING creatures +
// light mutation -> competent brains that survive (random genomes just flail at full thrust + die). Falls back
// to random genomes if world empty. Baseline provisioning, spread planet-wide. Used by viz::god_disturbances.
pub fn seed_burst(commands: &mut Commands, rng: &mut Rng, parents: &[Genome], n: usize) {
    for _ in 0..n {
        let g = if parents.is_empty() {
            Genome::random(rng)
        } else {
            let idx = ((rng.f32() * parents.len() as f32) as usize).min(parents.len() - 1);
            let mut c = parents[idx].clone();
            c.mutate(rng, 0.1, 0.1); // light tweak -> varied burst, not identical clones
            c
        };
        let pos = rand_pos(rng, CREATURE_Y); // any land, scattered planet-wide
        spawn_creature(commands, g, pos, rng, BIRTH_ENERGY);
    }
}

// God-control (render P button): populate the WHOLE planet with plants+trees+creatures, each in livable habitat
// (aquatic flora+swimmers in sea, alpine+land flora+trees on land, climate-matched temp_pref). Seeds an
// empty/sparse world all over vs waiting for spread. Counts are targets; ocean/land/abyss filtering retries a
// few times per item.
pub fn seed_planet(commands: &mut Commands, rng: &mut Rng, parents: &[Genome], _ntypes: u8, n_creatures: usize, n_plants: usize, n_trees: usize) {
    use std::f32::consts::PI;
    let whole = |rng: &mut Rng| crate::sphere::random_dir_in_cap(rng, Vec3::Y, PI); // uniform over the globe
    // creatures: adapt each to where it lands -> every region gets survivable life
    for _ in 0..n_creatures {
        let mut g = if parents.is_empty() {
            Genome::random(rng)
        } else {
            let idx = ((rng.f32() * parents.len() as f32) as usize).min(parents.len() - 1);
            let mut c = parents[idx].clone();
            c.mutate(rng, 0.1, 0.1);
            c
        };
        let d = whole(rng);
        if crate::sphere::is_ocean(d) {
            g.swim = rng.range(0.7, 1.0); // swimmer
            g.alpine = 0.0;
        } else if crate::sphere::rockiness(d) > 0.4 {
            g.alpine = rng.range(0.6, 1.0); // mountaineer
            g.swim = 0.1;
        } else {
            g.swim = 0.1;
        }
        g.temp_pref = (crate::sphere::base_temperature(d) + rng.normal() * 0.1).clamp(0.0, 1.0); // climate match
        spawn_creature(commands, g, crate::sphere::surface_pos(d, CREATURE_Y), rng, BIRTH_ENERGY);
    }
    // plants: aquatic flora in water, land flora elsewhere; skip the barren abyss
    for _ in 0..n_plants {
        let mut d = whole(rng);
        for _ in 0..6 {
            if crate::sphere::elevation01(d) >= crate::sphere::AQUATIC_FLOOR {
                break;
            }
            d = whole(rng);
        }
        let pg = plant_for_site(rng, d); // species by biome (aquatic in water, land flora on land)
        // spawn near maturity (like trees spawn full-grown) so god-populate is VISIBLE at once, not tiny
        // seedlings lost in the existing carpet. mass ~ this species' maturity -> full render size + can breed.
        let mass = rng.range(0.8, 1.1) * pg.maturity;
        spawn_plant(commands, pg, mass, crate::sphere::surface_pos(d, FOOD_Y));
    }
    // trees: land only + habitable ground
    for _ in 0..n_trees {
        let mut d = whole(rng);
        for _ in 0..8 {
            if !crate::sphere::is_ocean(d) && crate::sphere::plant_habitability(d) > 0.4 {
                break;
            }
            d = whole(rng);
        }
        if !crate::sphere::is_ocean(d) {
            spawn_tree(commands, rng.range(3.0, 9.0), crate::sphere::surface_pos(d, FOOD_Y), rng.f32() < 0.5, tree_genome(rng));
        }
    }
}

// Fresh founding tree genome: rich, tall, slow, some branches. kind 0. Trees evolve from here via
// PlantGenome::mutate_tree. Defense kept LOW: trees gated by REACH (height/branches vs creature height), NOT
// bite-vs-defense (live_step), so tree defense protects nothing AND growth_rate() taxes it QUADRATICALLY
// (0.85*def^2) -> high defense starves the tree below TREE_MATURITY = sterile never-dying "zombie tree". Low
// defense lets it grow+fruit+reproduce.
pub(crate) fn tree_genome(rng: &mut Rng) -> PlantGenome {
    PlantGenome {
        kind: 0,
        nutrient: rng.range(0.6, 1.0),
        defense: rng.f32() * 0.1,
        quality: rng.range(0.1, 0.4),
        wet: 0.5,
        height: rng.range(TREE_HEIGHT_MIN, 1.0), // never taller than 1.0
        light_pref: rng.range(0.5, 0.9),
        regrow: rng.range(0.0, 0.4),
        branches: rng.range(0.0, 0.6),
        spread: rng.range(5.0, 9.0),
        maturity: TREE_MATURITY,
        // fruit trees nutrient-rich (fruit = good food): broad generous profile + low toxicity
        nutrients: [0.55; NUTRIENTS],
        toxicity: rng.f32() * 0.15,
        fruit_toxicity: rng.f32() * 0.4, // protects unripe fruit; ripens sweet for dispersal
        // warm-ish climate niche, fruit-bearing crown, occasional blossom; form ignored (Tree marker)
        temp_pref: rng.range(0.4, 0.8),
        succulence: 0.0,
        submerged: 0.0,
        fruiting: rng.range(0.4, 0.8),
        nitrogen_fix: 0.0,
        fire_seed: rng.f32() * 0.2,
        climb: 0.0,
        allelopathy: 0.0,
        seed_weight: rng.range(0.3, 0.7), // trees: heavier provisioned seeds (acorn..samara), some drift
        windborne: rng.f32() * 0.4,
        clonal: 0.0,                      // trees seed; no clonal pathway in the tree branch yet
        cling: rng.f32() * 0.2,           // some trees bear hooked/sticky seeds
        dormancy: rng.range(0.3, 0.6),    // acorn/nut seed bank: trees bank seed in the soil between mast years
        hydrochory: rng.f32() * 0.2,      // riverside/coastal trees float their seeds (evolves up where it pays)
        form: crate::plant::form::SHRUB,
        flower: rng.f32() * 0.6, // some trees blossom (render: crown bloom ring)
        flower_hue: rng.f32(),
        flower_sat: rng.f32(),
        flower_light: rng.f32(),
        leaf_hue: rng.range(0.3, 0.45),
        bushiness: rng.range(0.4, 0.8),
        droop: 0.0,
    }
}

// Spawn one tree (long-lived plant + Tree marker). edible=true fruit tree, false=evergreen.
pub(crate) fn spawn_tree(commands: &mut Commands, mass: f32, pos: Vec3, edible: bool, g: PlantGenome) {
    // F33: only EDIBLE trees benefit from fruiting (fruit drop + animal dispersal). On an evergreen it's pure
    // growth tax (0.2*fruiting in growth_rate) with no payoff -> zero it so evergreens grow vigorous.
    let mut g = g;
    if !edible {
        g.fruiting = 0.0;
    }
    commands.spawn((
        Food,
        Tree { edible },
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos),
    ));
}

// Founding plant species for a surface site, by biome: ocean depth -> deep kelp (less sun) vs shallow
// lily/eelgrass/algae; land by temp+moisture -> cold alpine/moss, arid cactus/tumbleweed/thistle, wet reed/fern,
// else mixed meadow. Species evolves from here. World reads as real biomes (vs uniform-random plants).
pub(crate) fn plant_for_site(rng: &mut Rng, d: Vec3) -> PlantGenome {
    use crate::plant::Archetype as A;
    let pick = |rng: &mut Rng, opts: &[A]| opts[(rng.f32() * opts.len() as f32) as usize % opts.len()];
    if crate::sphere::is_ocean(d) {
        let e01 = crate::sphere::elevation01(d);
        let submersion = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
        let a = if submersion > 0.6 {
            A::Kelp // deep + dim -> shade-loving kelp (needs less sun)
        } else {
            pick(rng, &[A::Waterlily, A::Eelgrass, A::AlgaeMat]) // shallows -> sun-lit aquatics
        };
        return PlantGenome::archetype(rng, a);
    }
    let t = crate::sphere::base_temperature(d);
    let m = crate::sphere::moisture(d);
    // cold cutoff 0.34 matches biome/sea-ice onset -> frozen fringe gets cold-adapted species (alpine cushion,
    // moss), not temperate plants that just sit on ice + die back.
    let a = if t < 0.34 {
        pick(rng, &[A::AlpineCushion, A::Moss]) // cold poles / high ground (frozen fringe)
    } else if m < 0.32 {
        pick(rng, &[A::Cactus, A::Tumbleweed, A::Thistle]) // desert / arid
    } else if m > 0.68 {
        pick(rng, &[A::Reed, A::Fern, A::Wildflower, A::Moss]) // wet / lush
    } else {
        pick(rng, &[A::Clover, A::Wildflower, A::BerryBush, A::Nightshade, A::Fern]) // mixed meadow
    };
    PlantGenome::archetype(rng, a)
}

// Library-backed site seeding (tuning harness): tuned library loaded -> draw a biome-matched EVOLVED genome
// (mutated a touch for variety); else archetype for the site so no biome goes bare. `lib` None = no library file
// or --no-plant-lib.
fn site_plant(rng: &mut Rng, lib: Option<&crate::persist::PlantLibrary>, d: Vec3) -> PlantGenome {
    if let Some(l) = lib {
        if let Some(mut g) = l.pick_for_site(rng, d, false) {
            g.mutate(rng);
            randomize_flower_color(&mut g, rng); // library genomes predate bloom sat/light genes (all bloom same
            return g; // dull color) -> re-roll COSMETIC flower color so world seeds in varied bright hues
        }
    }
    plant_for_site(rng, d)
}

// Fresh BRIGHT flower color (cosmetic, zero sim cost). For library draws whose saved genomes lack
// flower_sat/flower_light genes (load at flat default -> identical dull blooms). flower_hue full-wheel; sat/light
// span their bright sub-ranges in plant::flower_color -> any vivid color.
fn randomize_flower_color(g: &mut PlantGenome, rng: &mut Rng) {
    g.flower_hue = rng.f32();
    g.flower_sat = rng.f32();
    g.flower_light = rng.f32();
}

// Same for trees (Tree marker): biome-matched tuned tree genome; else fresh founding tree genome.
fn site_tree(rng: &mut Rng, lib: Option<&crate::persist::PlantLibrary>, d: Vec3) -> PlantGenome {
    if let Some(l) = lib {
        if let Some(mut g) = l.pick_for_site(rng, d, true) {
            g.mutate_tree(rng);
            randomize_flower_color(&mut g, rng); // varied bright blossoms (also re-rolls blossom-presence hue)
            return g;
        }
    }
    tree_genome(rng)
}

// Botanical SHOWCASE (--garden): one of every plant species in a grid at the homeland + a back row of trees
// (fruit, evergreen, vine-draped evergreen). Inspect every form at once vs hunting across biomes. Plants spawn
// already-grown (mass ~ maturity) -> shown at full size.
fn seed_garden(commands: &mut Commands, rng: &mut Rng) {
    use crate::plant::Archetype as A;
    let species = [
        A::Clover, A::Wildflower, A::BerryBush, A::Fern, A::Cactus, // row -1
        A::Reed, A::Thistle, A::Nightshade, A::Moss, A::AlpineCushion, // row 0
        A::Tumbleweed, A::Waterlily, A::Eelgrass, A::Kelp, A::AlgaeMat, // row +1
    ];
    let c = homeland_center();
    // tangent basis at the homeland (u = "across", v = "forward")
    let seed_up = if c.cross(Vec3::Y).length() > 0.01 { Vec3::Y } else { Vec3::X };
    let u = c.cross(seed_up).normalize();
    let v = c.cross(u).normalize();
    let cols = 5usize;
    let sp = 0.03_f32; // grid spacing radians (~2.4 units): tight so plants fill view
    let place = |col: f32, row: f32| (c + u * (col * sp) + v * (row * sp)).normalize();
    for (i, a) in species.iter().enumerate() {
        let col = (i % cols) as f32 - 2.0;
        let row = (i / cols) as f32 - 1.0;
        let g = PlantGenome::archetype(rng, *a);
        spawn_plant(commands, g, 9.0, crate::sphere::surface_pos(place(col, row), FOOD_Y)); // grown to full size
    }
    // back row: a blossoming fruit tree, an evergreen, and a vine-draped evergreen
    let trees: [(f32, bool, bool); 3] = [(-1.5, true, false), (0.0, false, false), (1.5, false, true)];
    for (col, edible, vine) in trees {
        let mut g = tree_genome(rng);
        g.flower_hue = if vine { 0.8 } else { 0.3 }; // flower_hue > 0.58 -> add_plant_visuals drapes a vine
        if edible {
            g.flower = 0.6; // blossoms on the fruit tree
        }
        spawn_tree(commands, 6.0, crate::sphere::surface_pos(place(col, 1.7), FOOD_Y), edible, g);
    }
}

// Spawn one plant (living food). No render mesh; add_plant_visuals (render mode) gives one.
pub(crate) fn spawn_plant(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    commands.spawn((
        Food,
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos),
    ));
}

// Spawn one grass tuft. RENDER-ONLY ground cover: NO `Food` -> grass stays out of the per-tick food scan/sensing
// (8000+ tufts would crush the food clone + crash foraging). Edibility modeled by POSITION instead (live_step:
// hungry creature on grass-bearing soil nibbles a trickle). grass_step owns lifecycle + cap; oriented to surface
// normal so blades stand up on the sphere.
fn spawn_grass(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    let up = pos.normalize_or_zero();
    commands.spawn((
        Grass,
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos).with_rotation(Quat::from_rotation_arc(Vec3::Y, up)),
    ));
}

// Effective grass habitability at a surface dir: static plant_habitability LIFTED by local rain (ground water
// raises effective soil moisture) -> dry desert greens briefly after a downpour, then dries + tufts die back.
// Lush ground already moisture-capped -> rain can't over-grow it (no runaway food). gw=None -> static
// habitability (world seed, before any rain).
fn grass_hab(d: Vec3, gw: Option<&GroundWater>) -> f32 {
    let water = gw.map_or(0.0, |g| g.get(crate::sphere::surface_pos(d, 0.0)));
    let m = (crate::sphere::moisture(d) + WET_GAIN * water).clamp(0.0, 1.0);
    // cold gate: grass alone lacks plant_step's freeze kill, so without this it carpets the rendered ice cap.
    // Fade habitability to 0 across ICE_CAP_TEMP..FREEZE_TEMP -> tundra fringe at the frost edge, bare on the
    // white cap (matches biome_color). grass_step also hard-culls below FREEZE_TEMP as a backstop.
    let t = crate::sphere::base_temperature(d);
    let cold = ((t - FREEZE_TEMP) / (crate::config::ICE_CAP_TEMP - FREEZE_TEMP)).clamp(0.0, 1.0);
    crate::sphere::plant_habitability_with_moisture(d, m) * cold
}

// Whole-planet: grass blankets ALL plant-capable land worldwide (not just homeland). Rejection-samples the
// sphere until a non-ocean spot whose EFFECTIVE habitability (with rain) clears GRASS_HAB_MIN -> rained-on desert
// patch becomes eligible + blooms until it dries.
fn grass_pos(rng: &mut Rng, gw: Option<&GroundWater>) -> Vec3 {
    let mut d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    for _ in 0..8 {
        // frozen cap excluded for ALL paths (rocky too) -> no spawn-then-freeze-cull churn on the ice
        if !crate::sphere::is_ocean(d) && crate::sphere::base_temperature(d) >= FREEZE_TEMP {
            // grassland + rain-bloomed desert clear the gate; rocky ground gets SOME grass between rocks but only
            // a small fraction of rocky samples take -> sparse + thin.
            let hab = grass_hab(d, gw);
            // density tracks wetness: accept lush candidate with prob ~ habitability/wetness -> wet ground wins
            // MORE tufts (lush meadow), passable ground fewer (sparse), so tuft COUNT (not just size) follows
            // moisture. Cleared sites still always take eventually.
            let lush = hab > GRASS_HAB_MIN && rng.f32() < hab;
            let rocky = crate::sphere::rockiness(d) > 0.12 && rng.f32() < ROCK_GRASS_FRAC;
            if lush || rocky {
                break;
            }
        }
        d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    }
    crate::sphere::surface_pos(d, FOOD_Y)
}

// Seaweed = ocean grass. Shared components, own marker so seaweed_step owns cap+lifecycle + add_seaweed_visuals
// gives kelp fronds. Anchored to seafloor (surface_pos rides terrain = seabed underwater), oriented to surface
// normal so fronds rise.
fn spawn_seaweed(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    let up = pos.normalize_or_zero();
    commands.spawn((
        Seaweed,
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos).with_rotation(Quat::from_rotation_arc(Vec3::Y, up)),
    ));
}

// Whole-ocean: kelp blankets the SUBMERGED band (ocean, above abyssal floor where too deep/dark).
// Rejection-samples the sphere until an ocean spot in that band -> reliable swimmer food carpet.
fn seaweed_pos(rng: &mut Rng) -> Vec3 {
    let mut d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    for _ in 0..8 {
        let e01 = crate::sphere::elevation01(d);
        // skip pack-ice water (below SEA_ICE_TEMP) so kelp doesn't seed under the rendered ice
        if crate::sphere::is_ocean(d) && e01 > crate::sphere::AQUATIC_FLOOR && crate::sphere::base_temperature(d) >= crate::config::SEA_ICE_TEMP {
            // denser in lit shallows, thinner deep -> accept with prob ~ shallowness
            let shallow = ((e01 - crate::sphere::AQUATIC_FLOOR) / (crate::sphere::SEA_LEVEL - crate::sphere::AQUATIC_FLOOR)).clamp(0.0, 1.0);
            if rng.f32() < 0.35 + 0.65 * shallow {
                break;
            }
        }
        d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    }
    crate::sphere::surface_pos(d, FOOD_Y)
}

// --- spawn ---

// Headless: components only, no render assets (absent under MinimalPlugins).
pub fn spawn_world_headless(mut commands: Commands, mut rng: ResMut<Rng>, mut gen: ResMut<GenState>) {
    // --load resumes a saved population; else random founding pop. Positions re-randomized.
    let snap = gen.load.as_deref().and_then(crate::persist::load_snapshot);
    let genomes: Vec<Genome> = match &snap {
        Some(s) if !s.creatures.is_empty() => s.creatures.clone(),
        _ => (0..POP).map(|_| Genome::random(&mut rng)).collect(),
    };
    // loading a saved population into continuous mode skips warm-up (genomes already competent) -> drop
    // straight into a living world. Desync energy + age so they don't act in lockstep.
    let loaded = snap.as_ref().is_some_and(|s| !s.creatures.is_empty());
    let skip_warmup = gen.continuous && loaded;
    if skip_warmup {
        gen.generation = WARMUP_GENS;
    }
    for (i, g) in genomes.into_iter().enumerate() {
        // --diverse: niche-adapt + place each in its region; else founding pop in homeland.
        let (g, p) = if gen.diverse {
            diverse_creature(g, i, &mut rng)
        } else if loaded {
            let p = loaded_creature_pos(&g, &mut rng); // saved pop repopulates the whole globe
            (g, p)
        } else {
            (g, homeland_pos(&mut rng, CREATURE_Y)) // fresh founding: one homeland start, spreads over time
        };
        let mut g = g;
        g.ensure_net_shape(); // migrate older saved nets to current brain-input width
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY, attack: 0.0, defend: 0.0, fight_reward: 0.0 };
        let mut diet = diet_state(&g);
        if skip_warmup {
            diet.age = (rng.f32() * 600.0) as u32;
        }
        let e = if skip_warmup { rng.range(0.7, 1.2) * START_ENERGY } else { START_ENERGY };
        commands.spawn((
            Creature,
            g,
            brain,
            diet,
            Energy::from_total(e),
            Fitness(0.0),
            Heading(h),
            Alive(true),
            Locomotion { start: p, path: 0.0, alt: 0.0 },
            Transform::from_translation(p),
        ));
    }
    // diverse mode spreads life globally (creatures placed in niches worldwide, so food must be too).
    if gen.garden {
        // showcase: one of every species in a grid at homeland (+ trees) vs random world
        seed_garden(&mut commands, &mut rng);
    } else {
        // tuned plant seed-bank: present -> seed planet biome-matched FROM it.
        let lib = gen.plant_lib.as_deref().and_then(crate::persist::load_plant_library);
        // resuming a populated world (e.g. cargo run auto-loading showcase seed) or --diverse seeds the WHOLE
        // planet (plants every biome). Fresh founding run (no saved plants) keeps homeland start (life begins in
        // one area + spreads). Trees already seed whole-planet (spawn_trees).
        let resuming = snap.as_ref().is_some_and(|s| !s.plants.is_empty());
        let whole_planet = gen.diverse || resuming || loaded; // loaded population => seed plants planet-wide too
        let food_pos = |rng: &mut Rng| plant_spawn_pos(rng, !whole_planet, FOOD_Y); // land + shallow water (aquatic flora)
        match &snap {
            Some(s) if !s.plants.is_empty() => {
                for sp in &s.plants {
                    let p = food_pos(&mut rng);
                    // regenerate each loaded plant as fresh biome-matched flora (legacy plants not carried
                    // forward); sp.mass kept so the food web reloads grown, not all seedlings.
                    let g = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero());
                    spawn_plant(&mut commands, g, sp.mass, p);
                }
            }
            _ => {
                let n_seed = if whole_planet { FOOD * WHOLE_PLANET_SEED_MULT } else { FOOD };
                for _ in 0..n_seed {
                    let p = food_pos(&mut rng);
                    let pg = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero()); // tuned-library or biome archetype
                    spawn_plant(&mut commands, pg, rng.range(0.3, 1.4) * PLANT_START_MASS, p); // varied mass desyncs food supply
                }
            }
        }
        spawn_trees(&mut commands, &mut rng, gen.diverse, lib.as_ref());
    }
    // start turf half-full; grass_step tops up to GRASS_CAP. (Skip in --garden: clean showcase ground, no tall
    // turf hiding the specimens.)
    if !gen.garden {
        for _ in 0..GRASS_CAP / 2 {
            spawn_grass(&mut commands, PlantGenome::grass(&mut rng), GRASS_START_MASS, grass_pos(&mut rng, None));
        }
        // kelp carpet half-full too; seaweed_step tops up to SEAWEED_CAP -> oceans start green.
        for _ in 0..SEAWEED_CAP / 2 {
            spawn_seaweed(&mut commands, PlantGenome::seaweed(&mut rng), SEAWEED_START_MASS, seaweed_pos(&mut rng));
        }
    }
}

// Scatter initial trees (half fruit trees, half uneatable evergreens) on habitable land. Always WHOLE-PLANET:
// trees seed worldwide (then ambient reproduction fills toward TREE_CAP globally) -> the whole planet grows
// forests. (`_global` kept for call sites.)
fn spawn_trees(commands: &mut Commands, rng: &mut Rng, _global: bool, lib: Option<&crate::persist::PlantLibrary>) {
    let tree_pos = |rng: &mut Rng| rand_pos(rng, FOOD_Y);
    for i in 0..N_TREES {
        let mut p = tree_pos(rng);
        for _ in 0..6 {
            // density tracks site quality: accept with prob ~ habitability so wetter/richer ground seeds MORE
            // trees (groves), marginal ground fewer -> initial forests cluster where it's good.
            let hab = crate::sphere::plant_habitability(p.normalize_or_zero());
            if hab > 0.4 && rng.f32() < hab {
                break;
            }
            p = tree_pos(rng);
        }
        // alternate fruit tree / evergreen; tuned-library tree genome if available, else fresh founding one
        let g = site_tree(rng, lib, p.normalize_or_zero());
        spawn_tree(commands, rng.range(3.0, 9.0), p, i % 2 == 0, g);
    }
}

// Render: same components + mesh/material so we can watch. Plant visuals added by add_plant_visuals.
// NOTE: creature/plant/grass/seaweed SEEDING logic below mirrors spawn_world_headless. Keep in sync.
pub fn spawn_world_render(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    mut gen: ResMut<GenState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let creature_mesh = meshes.add(Capsule3d::new(0.4, 0.8));
    // shared creature mesh resource so viz::add_creature_visuals can dress creatures BORN mid-sim (spawn_creature
    // adds no mesh) -> newborns + B-button creatures become visible, not just the seed pop.
    commands.insert_resource(crate::viz::CreatureMesh(creature_mesh.clone()));
    // creature body PARTS (M4 genetic visuals): head/eye/leg meshes, child entities of each creature -> genome's
    // head size, eye count, leg count visible. Unit-ish base sizes; viz scales per genome.
    commands.insert_resource(crate::viz::CreatureParts {
        head: meshes.add(Sphere::new(0.5).mesh().ico(2).unwrap()), // diameter 1 at child scale 1
        eye: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        leg: meshes.add(Cylinder::new(0.5, 1.0)), // radius 0.5, height 1 -> thin legs after scaling
        fin: meshes.add(Cone { radius: 0.5, height: 1.0 }), // apex +Y, base -Y; scaled flat -> fish fins
        seg: meshes.add(Cuboid::new(1.0, 1.0, 1.0)), // unit box; scaled per tail/side-fin use
        wing: meshes.add(crate::viz::wing_mesh()), // bird wing planform (root at origin, flaps about root)
    });
    // per-form plant mesh library: one silhouette per plant::form (viz::add_plant_visuals picks by genome).
    // Round forms = icospheres; tall/leafy = procedural frond clumps; lily pad = flat disc.
    {
        use crate::plant::form;
        use crate::viz::blob_cluster_mesh as cluster;
        let v = Vec3::new; // brevity for blob centers
        let mut forms = vec![bevy::asset::Handle::default(); form::COUNT as usize];
        // bushy multi-blob clumps (base ~y=0, grow up) read as full foliage, not one ball
        forms[form::HERB as usize] =
            meshes.add(cluster(&[(v(0.0, 0.22, 0.0), 0.24, 0.85), (v(0.16, 0.30, 0.05), 0.19, 1.0), (v(-0.13, 0.27, -0.08), 0.18, 0.92)]));
        forms[form::SHRUB as usize] = meshes.add(cluster(&[
            (v(0.0, 0.28, 0.0), 0.3, 0.8),
            (v(0.26, 0.4, 0.08), 0.24, 0.95),
            (v(-0.24, 0.36, -0.1), 0.23, 0.9),
            (v(0.05, 0.6, -0.2), 0.22, 1.0),
            (v(-0.1, 0.34, 0.26), 0.21, 0.88),
        ]));
        forms[form::GROUNDCOVER as usize] = meshes.add(cluster(&[
            (v(0.0, 0.12, 0.0), 0.2, 0.85),
            (v(0.3, 0.1, 0.1), 0.17, 0.95),
            (v(-0.28, 0.1, -0.12), 0.16, 0.9),
            (v(0.1, 0.1, -0.32), 0.16, 1.0),
        ]));
        forms[form::MOSS as usize] = meshes.add(cluster(&[
            (v(0.0, 0.07, 0.0), 0.13, 0.8),
            (v(0.22, 0.06, 0.1), 0.11, 0.95),
            (v(-0.2, 0.06, -0.1), 0.1, 0.9),
            (v(0.08, 0.06, -0.22), 0.1, 1.0),
        ]));
        forms[form::FERN as usize] = meshes.add(crate::viz::frond_clump_mesh(9, 0.11, 0.18, 0.55, 0.4));
        forms[form::SUCCULENT as usize] = meshes.add(crate::viz::cactus_mesh());
        forms[form::REED as usize] = meshes.add(crate::viz::frond_clump_mesh(7, 0.035, 0.10, 0.12, 0.05));
        forms[form::FLOWER_STALK as usize] = meshes.add(Cylinder::new(0.05, 1.0));
        forms[form::ROSETTE as usize] = meshes.add(crate::viz::frond_clump_mesh(11, 0.12, 0.10, 0.2, 0.9));
        forms[form::LILYPAD as usize] = meshes.add(crate::viz::disc_mesh(20));
        forms[form::KELP as usize] = meshes.add(crate::viz::frond_clump_mesh(6, 0.14, 0.12, 0.6, 0.3));
        forms[form::MUSHROOM as usize] = meshes.add(Cylinder::new(0.06, 0.5));
        commands.insert_resource(crate::viz::PlantForms {
            forms,
            flower: meshes.add(crate::viz::flower_mesh(6)), // petalled bloom
            berry: meshes.add(Sphere::new(0.09).mesh().ico(1).unwrap()),
            cap: meshes.add(crate::viz::dome_mesh()), // domed mushroom cap
        });
    }
    commands.insert_resource(crate::viz::TreeMeshes {
        trunk: meshes.add(Cylinder::new(0.14, 1.4)), // short stub; canopy skirt drapes over most of it
        // fuller broadleaf crown: cluster of overlapping blobs (centered ~origin; placed in crown)
        broadleaf: meshes.add(crate::viz::blob_cluster_mesh(&[
            (Vec3::new(0.0, 0.0, 0.0), 1.0, 0.75),
            (Vec3::new(0.7, 0.3, 0.3), 0.8, 0.95),
            (Vec3::new(-0.6, 0.2, -0.4), 0.8, 0.9),
            (Vec3::new(0.2, 0.6, -0.5), 0.7, 1.0),
            (Vec3::new(-0.3, -0.15, 0.55), 0.7, 0.85),
        ])),
        conifer: meshes.add(crate::viz::conifer_mesh()), // stacked-cone Christmas-tree silhouette
        vine: meshes.add(crate::viz::vine_mesh(0.16)),    // helix vine hugging the trunk (radius ~ trunk)
    });
    // shared grass tuft mesh + one green material for ALL tufts (ubiquitous; size_grass scales each tuft length
    // by local soil). Double-sided so thin blades show from both faces.
    commands.insert_resource(crate::viz::GrassMesh(meshes.add(crate::viz::grass_tuft_mesh())));
    commands.insert_resource(crate::viz::GrassMaterial(materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.52, 0.18),
        perceptual_roughness: 0.95,
        double_sided: true,
        cull_mode: None,
        ..default()
    })));
    // shared kelp-frond mesh + brown-green material for ALL seaweed (ocean's grass). Broad arcing fronds,
    // double-sided so both faces show; add_seaweed_visuals scales each by mass + depth.
    commands.insert_resource(crate::viz::SeaweedMesh(meshes.add(crate::viz::frond_clump_mesh(5, 0.085, 0.10, 0.42, 0.22))));
    commands.insert_resource(crate::viz::SeaweedMaterial(materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.40, 0.22),
        perceptual_roughness: 0.9,
        double_sided: true,
        cull_mode: None,
        ..default()
    })));

    // scattered boulders on rocky highland (render-only dressing): one shared low-poly icosphere + gray stone
    // material, instanced many times with varied squat scale + spin -> rocky land reads as a field of rocks
    // (grass still seeds between them where habitability allows). Static -> spawned once, no per-frame cost.
    // Rejection-samples non-ocean ground, denser the rockier the spot; base sunk slightly into surface so each
    // rock sits on the ground. NotShadowCaster keeps the shadow pass cheap.
    {
        use std::f32::consts::PI;
        let rock_mesh = meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap());
        let rock_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.40, 0.39, 0.37),
            perceptual_roughness: 1.0,
            ..default()
        });
        let (mut placed, mut tries) = (0usize, 0usize);
        while placed < ROCK_SCATTER && tries < ROCK_SCATTER * 30 {
            tries += 1;
            let d = crate::sphere::random_dir_in_cap(&mut rng, Vec3::Y, PI);
            if crate::sphere::is_ocean(d) {
                continue;
            }
            let rock = crate::sphere::rockiness(d);
            if rock < 0.12 || rng.f32() > rock {
                continue; // only rocky ground; prob rises with rockiness (denser on peaks)
            }
            let up = d;
            let base = crate::sphere::surface_pos(up, 0.0);
            let s = rng.range(0.4, 1.8) * (0.6 + rock); // bigger boulders on rockiest peaks
            let (sx, sy, sz) = (s * rng.range(0.7, 1.3), s * rng.range(0.5, 1.0), s * rng.range(0.7, 1.3)); // squat
            let mut tf = Transform::from_translation(base - up * (sy * 0.18)); // sink base into ground
            tf.rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(rng.range(-PI, PI));
            tf.scale = Vec3::new(sx, sy, sz);
            commands.spawn((Mesh3d(rock_mesh.clone()), MeshMaterial3d(rock_mat.clone()), tf, bevy::light::NotShadowCaster));
            placed += 1;
        }
    }

    // --load resumes a saved population; else random founding pop. Positions re-randomized.
    let snap = gen.load.as_deref().and_then(crate::persist::load_snapshot);
    let genomes: Vec<Genome> = match &snap {
        Some(s) if !s.creatures.is_empty() => s.creatures.clone(),
        _ => (0..POP).map(|_| Genome::random(&mut rng)).collect(),
    };
    // loading a saved population into continuous mode skips warm-up (genomes already competent) -> drop straight
    // into a living world. Desync energy + age so they don't act in lockstep.
    let loaded = snap.as_ref().is_some_and(|s| !s.creatures.is_empty());
    let skip_warmup = gen.continuous && loaded;
    if skip_warmup {
        gen.generation = WARMUP_GENS;
    }
    // --garden showcase: just a few creatures wandering the garden, not the whole population.
    let n_creatures = if gen.garden { 4 } else { usize::MAX };
    for (i, g) in genomes.into_iter().take(n_creatures).enumerate() {
        let (g, p) = if gen.diverse {
            diverse_creature(g, i, &mut rng)
        } else if loaded {
            let p = loaded_creature_pos(&g, &mut rng); // saved pop repopulates the whole globe
            (g, p)
        } else {
            (g, homeland_pos(&mut rng, CREATURE_Y)) // fresh founding: one homeland start, spreads over time
        };
        let mut g = g;
        g.ensure_net_shape(); // migrate older saved nets to current brain-input width
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY, attack: 0.0, defend: 0.0, fight_reward: 0.0 };
        let mut diet = diet_state(&g);
        if skip_warmup {
            diet.age = (rng.f32() * 600.0) as u32;
        }
        let e = if skip_warmup { rng.range(0.7, 1.2) * START_ENERGY } else { START_ENERGY };
        // no mesh here: viz::add_creature_visuals dresses EVERY creature (founders + mid-sim births) with body +
        // genetic head/eyes/legs -> one code path for the look (avoids plain-capsule founders).
        commands.spawn((
            Creature,
            g,
            brain,
            diet,
            Energy::from_total(e),
            Fitness(0.0),
            Heading(h),
            Alive(true),
            Locomotion { start: p, path: 0.0, alt: 0.0 },
            Transform::from_translation(p),
        ));
    }
    if gen.garden {
        // showcase: one of every species in a grid at homeland (+ trees) vs random world
        seed_garden(&mut commands, &mut rng);
    } else {
        // tuned plant seed-bank: present -> seed planet biome-matched FROM it.
        let lib = gen.plant_lib.as_deref().and_then(crate::persist::load_plant_library);
        // resuming a populated world (e.g. cargo run auto-loading showcase seed) or --diverse seeds the WHOLE
        // planet (plants every biome). Fresh founding run (no saved plants) keeps homeland start (life begins in
        // one area + spreads). Trees already seed whole-planet (spawn_trees).
        let resuming = snap.as_ref().is_some_and(|s| !s.plants.is_empty());
        let whole_planet = gen.diverse || resuming || loaded; // loaded population => seed plants planet-wide too
        let food_pos = |rng: &mut Rng| plant_spawn_pos(rng, !whole_planet, FOOD_Y); // land + shallow water (aquatic flora)
        match &snap {
            Some(s) if !s.plants.is_empty() => {
                for sp in &s.plants {
                    let p = food_pos(&mut rng);
                    // regenerate each loaded plant as fresh biome-matched flora (legacy plants not carried
                    // forward); sp.mass kept so the food web reloads grown, not all seedlings.
                    let g = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero());
                    spawn_plant(&mut commands, g, sp.mass, p);
                }
            }
            _ => {
                let n_seed = if whole_planet { FOOD * WHOLE_PLANET_SEED_MULT } else { FOOD };
                for _ in 0..n_seed {
                    let p = food_pos(&mut rng);
                    let pg = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero()); // tuned-library or biome archetype
                    spawn_plant(&mut commands, pg, rng.range(0.3, 1.4) * PLANT_START_MASS, p); // varied mass desyncs food supply
                }
            }
        }
        spawn_trees(&mut commands, &mut rng, gen.diverse, lib.as_ref());
    }
    // start turf half-full; grass_step tops up to GRASS_CAP. (Skip in --garden: clean showcase ground, no tall
    // turf hiding the specimens.)
    if !gen.garden {
        for _ in 0..GRASS_CAP / 2 {
            spawn_grass(&mut commands, PlantGenome::grass(&mut rng), GRASS_START_MASS, grass_pos(&mut rng, None));
        }
        // kelp carpet half-full too; seaweed_step tops up to SEAWEED_CAP -> oceans start green.
        for _ in 0..SEAWEED_CAP / 2 {
            spawn_seaweed(&mut commands, PlantGenome::seaweed(&mut rng), SEAWEED_START_MASS, seaweed_pos(&mut rng));
        }
    }
}

// grass: render-only ground cover (no Food; edibility modeled by position in live_step). Own lifecycle + cap
// (off PLANT_CAP): killed by fire/drown/poor-soil so it stays on plant-capable land, grows/regrows, refills
// toward GRASS_CAP each tick -> whole-planet turf.
// grass_step death intent: produced in the parallel decide phase, drained + applied serially after
// (despawn + optional soil-ash). index = entity.index() = stable sort key for deterministic apply order.
struct GrassDeath {
    index: u32,
    entity: Entity,
    ash: f32, // soil deposit on fire death; 0 = freeze/mortality (no detritus)
    pos: Vec3,
}

pub fn grass_step(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
    mut soil: ResMut<Soil>,
    gw: Res<GroundWater>,
    fire: Res<Fire>,
    mut q: Query<(Entity, &mut PlantState, &PlantGenome, &Transform), (With<Grass>, Without<Rot>)>,
) {
    let _g = crate::profile::scope("grass");
    if gen.garden {
        return; // --garden: no turf (would top up to GRASS_CAP + bury the specimens)
    }
    let season = (gen.tick as f32 / GEN_TICKS as f32 * SEASON_FREQ).sin();
    let count = q.iter().count();
    let tick = gen.tick;
    let seed = gen.seed;
    // read-only snapshots for parallel decide (soil/gw/fire not mutated until the serial apply below)
    let soil_r: &Soil = &soil;
    let gw_r: &GroundWater = &gw;
    let fire_r: &Fire = &fire;
    // DECIDE (parallel): each tuft updates its OWN mass/age in place; dying tufts push a death intent into a
    // per-thread queue. Mortality roll draws a per-entity deterministic RNG -> order-independent. PARALLELIZATION.md.
    let mut deaths: bevy::utils::Parallel<Vec<GrassDeath>> = bevy::utils::Parallel::default();
    q.par_iter_mut().for_each_init(
        || deaths.borrow_local_mut(),
        |out, (e, mut st, g, tf)| {
            let ppos = tf.translation;
            let pdir = ppos.normalize_or_zero();
            // wildfire burns the tuft up: small biomass -> ash (deposited in apply). burned turf regrows richer.
            if fire_r.get(ppos) > FIRE_KILL {
                out.push(GrassDeath { index: e.index().index(), entity: e, ash: FIRE_BURN_ASH * st.mass * 0.5, pos: ppos });
                return;
            }
            // hard freeze: grass on the frozen ice cap dies outright (mirrors plant_step). grass_hab cold-fade
            // already thins tufts toward the edge; this culls any that landed on the rendered white cap.
            if crate::sphere::base_temperature(pdir) < FREEZE_TEMP {
                out.push(GrassDeath { index: e.index().index(), entity: e, ash: 0.0, pos: ppos });
                return;
            }
            // mortality off plant-capable soil: dry/wet mismatch, poor site (rock/desert/cold), or submerged.
            let water = gw_r.get(ppos);
            let m = (crate::sphere::moisture(pdir) + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0);
            let stress = (m - g.wet).abs();
            let mut hab = grass_hab(pdir, Some(gw_r)); // rain-lifted: desert turf lives while wet, dies as it dries
            if crate::sphere::rockiness(pdir) > 0.12 {
                hab = hab.max(ROCK_GRASS_HAB); // thin grass clings between rocks: don't cull as "poor site"
            }
            let e01 = crate::sphere::elevation01(pdir);
            let submersion = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
            let drown = DROWN_KILL * submersion * (1.0 - g.wet.max(g.submerged)); // F22/F27: submerged = 2nd aquatic axis -> deep-water plants don't drown
            let p_mort = MOISTURE_KILL * (stress - MOISTURE_TOLERANCE).max(0.0) + HABITAT_KILL * (0.3 - hab).max(0.0) + drown;
            if crate::rng::Rng::for_entity(seed, e.index().index(), tick).f32() < p_mort {
                out.push(GrassDeath { index: e.index().index(), entity: e, ash: 0.0, pos: ppos });
                return;
            }
            // grow/regrow: soil fertility + rain + light match, capped at maturity.
            let light = crate::sphere::daylight_at(pdir, tick);
            let fert = soil_r.get(ppos);
            let boost = (1.0 + FERT_GROWTH * (fert / FERT_CAP).min(1.0)) * (1.0 + WET_GROWTH * water);
            let lf = 0.35 + 0.65 * (1.0 - (light - g.light_pref).abs());
            st.mass = (st.mass + g.growth_rate() * boost * hab * lf * DT).min(g.maturity);
            st.age += 1;
        },
    );
    // APPLY (serial, deterministic): sort deaths by stable entity index so soil-ash accumulation + despawn are
    // scheduling-independent (same seed reproduces). Parallel drain order is unspecified, hence the sort.
    let mut dead: Vec<GrassDeath> = Vec::new();
    deaths.drain_into(&mut dead);
    dead.sort_unstable_by_key(|d| d.index);
    for d in &dead {
        if d.ash > 0.0 {
            soil.add(d.pos, d.ash);
        }
        commands.entity(d.entity).despawn();
    }
    // refill toward target density: blanket plant-capable ground + replace grazed/burned tufts.
    let alive = count.saturating_sub(dead.len());
    let mut spawned = 0;
    while alive + spawned < GRASS_CAP {
        spawn_grass(&mut commands, PlantGenome::grass(&mut rng), GRASS_START_MASS, grass_pos(&mut rng, Some(&gw)));
        spawned += 1;
    }
}

// seaweed/kelp lifecycle: OCEAN twin of grass_step. Keeps a SEAWEED_CAP kelp carpet across the submerged band,
// grows it (dimmer+slower the deeper), culls fronds stranded out of band, refills each tick -> reliable swimmer
// food carpet that never touches the fragile plant_step lifecycle.
pub fn seaweed_step(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
    mut q: Query<(Entity, &mut PlantState, &PlantGenome, &Transform), With<Seaweed>>,
) {
    let _g = crate::profile::scope("seaweed");
    if gen.garden {
        return; // --garden: no ambient carpet
    }
    let count = q.iter().count();
    let tick = gen.tick;
    // DECIDE (parallel): each frond grows its OWN mass/age in place; out-of-band/iced fronds push a despawn
    // intent. No RNG here (death is deterministic band+ice checks) so determinism is automatic. PARALLELIZATION.md.
    let mut deaths: bevy::utils::Parallel<Vec<(u32, Entity)>> = bevy::utils::Parallel::default();
    q.par_iter_mut().for_each_init(
        || deaths.borrow_local_mut(),
        |out, (e, mut st, g, tf)| {
            let pdir = tf.translation.normalize_or_zero();
            let e01 = crate::sphere::elevation01(pdir);
            // cull fronds out of the submerged band: stranded above sea level, or sunk past the abyssal floor.
            if !crate::sphere::is_ocean(pdir) || e01 <= crate::sphere::AQUATIC_FLOOR {
                out.push((e.index().index(), e));
                return;
            }
            // pack ice: cold polar ocean freezes over -> cull kelp below the sea-ice band (no kelp through ice).
            if crate::sphere::base_temperature(pdir) < crate::config::SEA_ICE_TEMP {
                out.push((e.index().index(), e));
                return;
            }
            // grow toward maturity: light fades with depth (deeper = dimmer) -> shallow kelp grows fastest.
            let depth = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
            let light = crate::sphere::daylight_at(pdir, tick) * (1.0 - 0.6 * depth);
            let lf = 0.4 + 0.6 * light.clamp(0.0, 1.0);
            st.mass = (st.mass + g.growth_rate() * lf * DT).min(g.maturity);
            st.age += 1;
        },
    );
    // APPLY (serial): despawn culled fronds (sorted by index = deterministic order), then refill the carpet.
    let mut dead: Vec<(u32, Entity)> = Vec::new();
    deaths.drain_into(&mut dead);
    dead.sort_unstable_by_key(|d| d.0);
    for (_, e) in &dead {
        commands.entity(*e).despawn();
    }
    // refill toward target carpet density across the submerged band.
    let alive = count.saturating_sub(dead.len());
    let mut spawned = 0;
    while alive + spawned < SEAWEED_CAP {
        spawn_seaweed(&mut commands, PlantGenome::seaweed(&mut rng), SEAWEED_START_MASS, seaweed_pos(&mut rng));
        spawned += 1;
    }
}

// plant_step intents: produced per-entity in the parallel decide phase, drained + applied serially after
// (despawn, soil, spawns). Every entry carries the parent's entity.index() as a stable sort key so the
// serial apply (caps, soil-sum, and especially SPAWN ORDER -> new entity-index assignment) is
// scheduling-independent and reproduces run-to-run. PARALLELIZATION.md.
#[derive(Default)]
struct PlantBatch {
    despawns: Vec<(u32, Entity, bool, Option<&'static str>)>, // idx, entity, is_plant(non-tree), scenario death cause
    soil_adds: Vec<(u32, Vec3, f32)>,                          // ash / death-fert / nfix (deferred ResMut<Soil> write)
    detritus: Vec<(u32, PlantGenome, f32, Vec3)>,
    fruit_drops: Vec<(u32, PlantGenome, Vec3)>,
    births: Vec<(u32, PlantGenome, Vec3)>,
    tree_births: Vec<(u32, Vec3, bool, PlantGenome)>,
    bank: Vec<(u32, PlantGenome, Vec3, u32)>, // this tick's NEW dormant seeds (idx, genome, pos, wait)
}

// plants: grow, reproduce (disperse mutated offspring), reseed if the web nearly collapses (spec 13).
pub fn plant_step(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
    mut soil: ResMut<Soil>,
    gw: Res<GroundWater>,
    climate: Res<Climate>,
    fire: Res<Fire>,
    mut tree_bites: ResMut<TreeBites>,
    mut bank: ResMut<SeedBank>,
    // scenario tuning harness: present ONLY under --scenario. Its presence disables reseed floor (isolated cohort
    // not flooded) + counts births/deaths-by-cause. Absent in normal/headless runs (free).
    mut stats: Option<ResMut<crate::scenario::ScenarioStats>>,
    mut q: Query<(Entity, &mut PlantState, &PlantGenome, &Transform, Option<&Tree>), (Without<Rot>, Without<Grass>)>, // not carrion, not grass (grass_step owns grass)
) {
    let _g = crate::profile::scope("plant");
    soil.decay(); // fertility leaches / taken up over time
    // scenario mode: no PLANT_MIN reseed (cohort IS the only plants); normal mode keeps the floor. Caps are
    // cohort-scale in scenario (stats.cap ~ 2x target) so a viable cohort grows toward its target + shows vigor
    // vs booming to global PLANT_CAP/TREE_CAP.
    let (reseed_floor, pcap, tcap) = match stats.as_ref() {
        Some(s) => (0, s.cap, s.cap),
        None => (PLANT_MIN, PLANT_CAP, TREE_CAP),
    };
    // season drifts on global tick clock (advances both modes; generation frozen in continuous)
    let season = (gen.tick as f32 / GEN_TICKS as f32 * SEASON_FREQ).sin(); // -1 dry .. +1 wet
    let mut plant_count = q.iter().filter(|(.., t)| t.is_none()).count();
    let tree_count = q.iter().filter(|(.., t)| t.is_some()).count();
    // coarse start-of-tick repro gates. Serial gated each birth by `count + births.len() < cap` (and
    // short-circuited the rng + mass cost when full). Parallel decide can't see other entities' births, so we
    // gate on the start-of-tick count -> blocks seeding (and its mass cost) when already AT cap, matching serial
    // in the common at-cap case; the precise cap is re-enforced at spawn in apply. Divergence only in the narrow
    // band where count < cap but count+births > cap (a few parents pay mass with no surviving seed). PARALLELIZATION.md.
    let seeding_open = plant_count < pcap;
    let tree_open = tree_count < tcap;
    let tree_positions: Vec<Vec3> = q.iter().filter_map(|(_, _, _, tf, t)| t.map(|_| tf.translation)).collect();
    // mating mode (--mating, shared with creatures): pool of (entity, pos, is_tree, genome) so a seeding
    // plant/tree finds a nearby genetically-similar MATE to cross. Built only when --mating (cloning every plant
    // genome each tick isn't free); else empty -> reproduction is single-parent budding.
    let mate_pool: Vec<(Entity, Vec3, bool, PlantGenome)> = if gen.mating {
        q.iter().map(|(e, _, g, tf, tree)| (e, tf.translation, tree.is_some(), g.clone())).collect()
    } else {
        Vec::new()
    };
    let seed = gen.seed;
    let tick = gen.tick;
    // read-only snapshots for the parallel decide (soil/tree_bites not mutated until the serial apply below)
    let soil_r: &Soil = &soil;
    let gw_r: &GroundWater = &gw;
    let climate_r: &Climate = &climate;
    let fire_r: &Fire = &fire;
    let bites_r: &TreeBites = &tree_bites;
    let tpos_r: &[Vec3] = &tree_positions;
    let pool_r: &[(Entity, Vec3, bool, PlantGenome)] = &mate_pool;
    // DECIDE (parallel): each plant/tree updates its OWN mass/age in place; every side effect (despawn, soil
    // deposit, detritus, fruit drop, birth, tree birth, dormant seed) is pushed as an intent carrying the
    // parent's entity index. RNG is per-entity (for_entity) -> order-independent. Caps + scenario stats +
    // spawns are resolved in the serial apply, sorted by index. PARALLELIZATION.md.
    let mut batch: bevy::utils::Parallel<PlantBatch> = bevy::utils::Parallel::default();
    q.par_iter_mut().for_each_init(
        || batch.borrow_local_mut(),
        |out, (e, mut st, g, tf, tree)| {
            let idx = e.index().index();
            let mut prng = crate::rng::Rng::for_entity(seed, idx, tick);
            let ppos = tf.translation;
            let pdir = ppos.normalize_or_zero();
            // wildfire burn-up: biomass -> ash (trees ~3x). Burned ground regrows richer. Serotiny: fire-adapted
            // plant releases a seed AS it burns (post-fire recruitment onto fresh ash).
            if fire_r.get(ppos) > FIRE_KILL {
                let biomass = if tree.is_some() { 3.0 } else { 1.0 };
                out.soil_adds.push((idx, ppos, FIRE_BURN_ASH * st.mass * biomass));
                if tree.is_none() && prng.f32() < g.fire_seed {
                    let child = mate_or_self(pool_r, e, ppos, g, false, &mut prng);
                    out.births.push((idx, child, disperse_pos(&mut prng, ppos, g.spread, FOOD_Y)));
                }
                out.despawns.push((idx, e, tree.is_none(), Some("fire")));
                return;
            }
            // hard freeze: polar ice core -> any plant/tree dies outright (absolute kill, temp_pref-independent).
            if crate::sphere::base_temperature(pdir) < FREEZE_TEMP {
                out.despawns.push((idx, e, tree.is_none(), Some("frozen")));
                return;
            }
            let light = crate::sphere::daylight_at(pdir, tick);
            let fert = soil_r.get(ppos);
            let water = gw_r.get(ppos);
            let boost = (1.0 + FERT_GROWTH * (fert / FERT_CAP).min(1.0)) * (1.0 + WET_GROWTH * water);
            let lf = 0.35 + 0.65 * (1.0 - (light - g.light_pref).abs());
            if let Some(tree) = tree {
                // trees land-only: a tree in water drowns fast (no kelp/mangrove forests).
                if crate::sphere::is_ocean(pdir) && prng.f32() < DROWN_TREE {
                    out.despawns.push((idx, e, false, Some("drown")));
                    return;
                }
                // grazed this tick? apply recorded mass damage; over-eaten below TREE_MIN_MASS -> dies.
                let grazed = bites_r.0.contains_key(&e);
                if grazed {
                    st.mass = (st.mass - bites_r.0[&e]).max(0.0);
                    if st.mass < TREE_MIN_MASS {
                        out.despawns.push((idx, e, false, Some("eaten")));
                        return;
                    }
                }
                // tree climate niche: moisture-immune (deep roots) but feels temperature. Far off thermal band -> dies.
                let tmiss = (crate::sphere::base_temperature(pdir) - g.temp_pref).abs();
                if prng.f32() < TREE_TEMP_KILL * (tmiss - TREE_TEMP_TOL).max(0.0) {
                    out.despawns.push((idx, e, false, Some("temp")));
                    return;
                }
                let temp_grow = TEMP_FLOOR + (1.0 - TEMP_FLOOR) * (1.0 - tmiss);
                // soil response shapes growth speed + final SIZE (survival stays moisture-immune): rich + ideally
                // moist ground grows bigger trees. Still MATURES at g.maturity so food/spread unchanged.
                let clim = crate::sphere::moisture(pdir) * (1.0 - CLIMATE_VEG) + climate_r.get(ppos) * CLIMATE_VEG;
                let m = (clim + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0);
                let moist_q = (1.0 - (m - TREE_WET_OPT).abs() / TREE_WET_TOL).clamp(0.0, 1.0);
                let fert_q = (fert / FERT_CAP).min(1.0);
                let soil_q = moist_q * (0.4 + 0.6 * fert_q);
                let grow_mult = (1.0 + FERT_GROWTH * fert_q) * (TREE_WET_FLOOR + (1.0 - TREE_WET_FLOOR) * moist_q);
                let full_size = g.maturity * (1.0 + TREE_SOIL_SIZE * soil_q);
                st.mass = (st.mass + g.growth_rate() * grow_mult * lf * temp_grow * TREE_GROWTH_SCALE * DT).min(full_size);
                st.age += 1;
                let r2 = TREE_DENSITY_R * TREE_DENSITY_R;
                let local = tpos_r.iter().filter(|p| p.distance_squared(ppos) < r2).count();
                let fert_boost = 0.3 + 2.2 * fert_q;
                let mature = st.mass >= g.maturity;
                if mature && tree.edible && prng.f32() < P_FRUIT_DROP * (0.5 + g.nutrient) {
                    let fpos = disperse_pos(&mut prng, ppos, 3.0, FOOD_Y); // within crown footprint
                    out.fruit_drops.push((idx, g.clone(), fpos));
                }
                let ambient = mature && local <= TREE_MAX_LOCAL && prng.f32() < P_TREE_REPRO * fert_boost;
                let disperse = mature && tree.edible && grazed && prng.f32() < P_TREE_EAT_DISPERSE; // seed carried off
                // tree_open = start-of-tick tcap gate (precise cap re-enforced at spawn). rng above always drawn (matches serial order).
                if (ambient || disperse) && tree_open {
                    let base = eff_spread(g);
                    let spread = if disperse { base * TREE_EAT_SPREAD_MULT } else { base };
                    let pos = disperse_pos(&mut prng, ppos, spread, FOOD_Y);
                    if !crate::sphere::is_ocean(pos.normalize_or_zero()) {
                        let child = mate_or_self(pool_r, e, ppos, g, true, &mut prng);
                        out.tree_births.push((idx, pos, tree.edible, child));
                    }
                    st.mass *= PLANT_REPRO_FRAC; // budding a seed costs parent mass either way
                }
                return;
            }
            // --- regular plant ---
            // grazing from live_step: eaten below PLANT_MIN_MASS = consumed -> gone; high-regrow bush survives.
            if let Some(&bite) = bites_r.0.get(&e) {
                st.mass = (st.mass - bite).max(0.0);
                if st.mass < PLANT_MIN_MASS {
                    out.soil_adds.push((idx, ppos, DEATH_FERT * 0.3));
                    out.despawns.push((idx, e, true, Some("eaten")));
                    return;
                }
            }
            // mortality from moisture mismatch / poor site / drown / desiccate / temp. Effective moisture = slow
            // CLIMATE moisture (drifts -> deserts/rainforests) + season + rain-fed ground water.
            let clim = crate::sphere::moisture(pdir) * (1.0 - CLIMATE_VEG) + climate_r.get(ppos) * CLIMATE_VEG;
            let m = (clim + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0);
            // succulence buffers DROUGHT only (water-storers survive drier than their `wet`); soggy side unbuffered.
            let dry_deficit = (g.wet - m).max(0.0);
            let stress = ((m - g.wet).abs() - SUCC_BUFFER * g.succulence * dry_deficit).max(0.0);
            let hab = crate::sphere::plant_habitability_with_moisture(pdir, clim);
            let e01 = crate::sphere::elevation01(pdir);
            let submersion = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
            let drown = DROWN_KILL * submersion * (1.0 - g.wet.max(g.submerged)); // F22/F27
            // desiccation (mirror of drown): aquatic plant stranded on dry land dries out.
            let aquatic = ((g.wet - 0.85) / 0.15).clamp(0.0, 1.0);
            let desiccate = DESICCATE_KILL * aquatic * (1.0 - submersion) * (1.0 - (m / 0.6).min(1.0));
            let temp = crate::sphere::base_temperature(pdir);
            let tmiss = (temp - g.temp_pref).abs();
            let temp_grow = TEMP_FLOOR + (1.0 - TEMP_FLOOR) * (1.0 - tmiss);
            let m_moist = MOISTURE_KILL * (stress - MOISTURE_TOLERANCE).max(0.0);
            // aquatic plants spared the LAND-habitat penalty in their ocean home (scaled by at-home-in-water).
            let aq_home = ((g.wet.max(g.submerged) - 0.85) / 0.15).clamp(0.0, 1.0) * submersion;
            let m_hab = HABITAT_KILL * (0.3 - hab).max(0.0) * (1.0 - aq_home);
            let m_temp = TEMP_KILL * (tmiss - TEMP_TOL).max(0.0);
            let p_mort = m_moist + m_hab + drown + desiccate + m_temp;
            if prng.f32() < p_mort {
                // scenario tuning: attribute death to the dominant cause.
                let cause = {
                    let causes = [("moisture", m_moist), ("habitat", m_hab), ("drown", drown), ("desiccate", desiccate), ("temp", m_temp)];
                    causes.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|c| c.0).unwrap_or("moisture")
                };
                // allelopathic litter: chemical-warfare plant leaves extra-toxic detritus suppressing competitors.
                let mut litter = g.clone();
                litter.toxicity = litter.toxicity.max(g.allelopathy);
                out.detritus.push((idx, litter, st.mass, ppos));
                out.soil_adds.push((idx, ppos, DEATH_FERT * 0.3));
                out.despawns.push((idx, e, true, Some(cause)));
                return;
            }
            // underwater the column dims light with depth -> deep plants need shade tolerance; vine climbs toward light.
            let light_uw = light * (1.0 - WATER_LIGHT_ATTEN * submersion);
            let lf = (0.35 + 0.65 * (1.0 - (light_uw - g.light_pref).abs()) + CLIMB_LIGHT * g.climb).min(1.0);
            // nitrogen-fixer (legume): root nodules enrich local soil each tick.
            if g.nitrogen_fix > 0.0 {
                out.soil_adds.push((idx, ppos, NFIX_RATE * g.nitrogen_fix * DT));
            }
            st.mass += g.growth_rate() * boost * hab * lf * temp_grow * DT;
            st.age += 1;
            let mature = st.mass >= g.maturity;
            // fruiting non-tree (berry bush, nightshade) drops fallen fruit -> fast-energy + ferment chain.
            if mature && g.fruiting > 0.2 && prng.f32() < P_FRUIT_DROP * g.fruiting {
                out.fruit_drops.push((idx, g.clone(), disperse_pos(&mut prng, ppos, 2.0, FOOD_Y)));
            }
            // endozoochory: fruiting plant that survived grazing has a seed carried off + dropped far. Toxic fruit
            // disperses less. pcap enforced in apply.
            if mature
                && g.fruiting > 0.2
                && bites_r.0.contains_key(&e)
                && seeding_open
                && prng.f32() < P_PLANT_EAT_DISPERSE * g.fruiting * (1.0 - g.fruit_toxicity)
            {
                let child = mate_or_self(pool_r, e, ppos, g, false, &mut prng);
                let pos = disperse_pos(&mut prng, ppos, eff_spread(g) * PLANT_EAT_SPREAD_MULT, FOOD_Y);
                if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                    out.births.push((idx, child, pos));
                }
            }
            // clonal spread (rhizome/runner/sucker): short-range true clone beside parent (no mutate). pcap in apply.
            if mature && g.clonal > 0.0 && seeding_open && prng.f32() < P_CLONAL * g.clonal {
                let pos = disperse_pos(&mut prng, ppos, CLONAL_RADIUS, FOOD_Y);
                if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                    out.births.push((idx, g.clone(), pos)); // identical ramet: fidelity over variation
                    st.mass *= PLANT_REPRO_FRAC; // budding a ramet costs parent
                }
            }
            // cling (epizoochory): burr/sticky seed snagged by passing animal -> even inedible plants disperse.
            if mature && g.cling > 0.0 && seeding_open && prng.f32() < P_CLING * g.cling {
                let child = mate_or_self(pool_r, e, ppos, g, false, &mut prng);
                let pos = disperse_pos(&mut prng, ppos, eff_spread(g) * CLING_SPREAD_MULT, FOOD_Y);
                if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                    out.births.push((idx, child, pos));
                }
            }
            // ordinary seeding (+ hydrochory long water reach + dormancy bank). pcap + bank cap enforced in apply.
            if mature && seeding_open && prng.f32() < P_REPRO * (1.0 - DEF_REPRO_COST * g.defense) {
                let child = mate_or_self(pool_r, e, ppos, g, false, &mut prng);
                let near_water = ((crate::sphere::SEA_LEVEL + HYDRO_COAST_BAND - e01) / HYDRO_COAST_BAND).clamp(0.0, 1.0);
                let hydro = 1.0 + HYDRO_RANGE * g.hydrochory * near_water;
                let pos = disperse_pos(&mut prng, ppos, eff_spread(g) * hydro, FOOD_Y);
                // aquatic plants (high wet) only seed into water; seed on dry ground dropped. Parent pays either way.
                if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                    // dormancy: a fraction go DORMANT into the soil bank vs sprouting now (lineage survives a wipe).
                    if prng.f32() < DORMANCY_FRAC * g.dormancy {
                        let wait = DORMANT_TICKS_MIN + (prng.f32() * (DORMANT_TICKS_MAX - DORMANT_TICKS_MIN) as f32) as u32;
                        out.bank.push((idx, child, pos, wait));
                    } else {
                        out.births.push((idx, child, pos));
                    }
                }
                st.mass *= PLANT_REPRO_FRAC;
            }
        },
    );
    // APPLY (serial, deterministic): merge per-thread batches, sort every list by parent index (drain order is
    // unspecified) so caps, soil-sum, and SPAWN order -> new entity-index assignment all reproduce run-to-run.
    let mut despawns: Vec<(u32, Entity, bool, Option<&'static str>)> = Vec::new();
    let mut soil_adds: Vec<(u32, Vec3, f32)> = Vec::new();
    let mut detritus: Vec<(u32, PlantGenome, f32, Vec3)> = Vec::new();
    let mut fruit_drops: Vec<(u32, PlantGenome, Vec3)> = Vec::new();
    let mut births: Vec<(u32, PlantGenome, Vec3)> = Vec::new();
    let mut tree_births: Vec<(u32, Vec3, bool, PlantGenome)> = Vec::new();
    let mut new_bank: Vec<(u32, PlantGenome, Vec3, u32)> = Vec::new();
    for b in batch.iter_mut() {
        despawns.append(&mut b.despawns);
        soil_adds.append(&mut b.soil_adds);
        detritus.append(&mut b.detritus);
        fruit_drops.append(&mut b.fruit_drops);
        births.append(&mut b.births);
        tree_births.append(&mut b.tree_births);
        new_bank.append(&mut b.bank);
    }
    despawns.sort_by_key(|d| d.0);
    soil_adds.sort_by_key(|d| d.0);
    detritus.sort_by_key(|d| d.0);
    fruit_drops.sort_by_key(|d| d.0);
    births.sort_by_key(|d| d.0);
    tree_births.sort_by_key(|d| d.0);
    new_bank.sort_by_key(|d| d.0);
    // despawns (+ scenario death tally + live plant-count tracking)
    for (_, e, is_plant, cause) in &despawns {
        if let Some(s) = stats.as_deref_mut() {
            if let Some(c) = cause {
                s.death(c);
            }
        }
        commands.entity(*e).despawn();
        if *is_plant {
            plant_count = plant_count.saturating_sub(1);
        }
    }
    // soil deposits in sorted order -> deterministic float accumulation
    for (_, pos, amt) in &soil_adds {
        soil.add(*pos, *amt);
    }
    // this tick's NEW dormant seeds enter the bank up to SEED_BANK_CAP; overflow sprouts now (serial's else arm)
    for (idx, g, pos, wait) in new_bank {
        if bank.0.len() < SEED_BANK_CAP {
            bank.0.push((g, pos, wait));
        } else {
            births.push((idx, g, pos)); // re-sorted below
        }
    }
    births.sort_by_key(|d| d.0);
    // existing seed bank germination: tick down; one reaching 0 sprouts (cap permitting) + leaves the bank.
    let mut germinated: Vec<(PlantGenome, Vec3)> = Vec::new();
    bank.0.retain_mut(|(g, pos, ticks)| {
        *ticks = ticks.saturating_sub(1);
        if *ticks == 0 && plant_count + births.len() + germinated.len() < pcap {
            germinated.push((g.clone(), *pos));
            false
        } else {
            true
        }
    });
    // dead plants -> fermenting detritus (poor food, ferments to a little toxic FAST energy, then gone).
    for (_, g, mass, pos) in detritus {
        commands.spawn((
            Food,
            PlantState { mass: mass.min(CARRION_MASS), age: 0 },
            PlantGenome { nutrient: DETRITUS_NUTRIENT, defense: 0.0, quality: 0.0, ..g },
            Rot { age: 0 },
            Ferment { toxic: FERMENT_TOX_DETRITUS },
            Transform::from_translation(pos),
        ));
    }
    // fruit drops: Food carrying full parent genome (Seed) + Rot + Ferment; Food genome height/defense-zeroed so
    // fruit renders flat on ground + any creature can eat it.
    for (_, g, pos) in fruit_drops {
        let ftox = g.fruit_toxicity;
        commands.spawn((
            Food,
            PlantState { mass: FALLEN_FRUIT_MASS, age: 0 },
            PlantGenome { defense: 0.0, height: 0.0, ..g.clone() },
            Seed(g),
            Rot { age: 0 },
            Ferment { toxic: ftox },
            Transform::from_translation(pos),
        ));
    }
    // scenario tuning: count this tick's offspring (reseed off in scenario, so these are all real reproduction).
    if let Some(s) = stats.as_deref_mut() {
        s.births += (births.len() + germinated.len() + tree_births.len()) as u32;
    }
    // spawn offspring, capping total plants at pcap via a running pop counter (serial enforced this per-birth in
    // the loop; we enforce it once here, in deterministic sorted order so new entity indices reproduce).
    let mut pop = plant_count;
    for (_, g, pos) in births {
        if pop >= pcap {
            break;
        }
        let est = 0.6 + 0.8 * g.seed_weight; // heavy seed -> bigger hardier seedling (head start)
        spawn_plant(&mut commands, g, rng.range(0.5, 1.3) * PLANT_START_MASS * est, pos);
        pop += 1;
    }
    for (g, pos) in germinated {
        if pop >= pcap {
            break;
        }
        let est = 0.6 + 0.8 * g.seed_weight;
        spawn_plant(&mut commands, g, rng.range(0.5, 1.3) * PLANT_START_MASS * est, pos);
        pop += 1;
    }
    // reseed floor: keep a minimal biome-matched seed base so creatures can't drive food fully extinct.
    // Disabled in scenario mode (reseed_floor=0).
    while pop < reseed_floor {
        let pos = rand_pos(&mut rng, FOOD_Y);
        let g = plant_for_site(&mut rng, pos.normalize_or_zero());
        let est = 0.6 + 0.8 * g.seed_weight;
        spawn_plant(&mut commands, g, rng.range(0.5, 1.3) * PLANT_START_MASS * est, pos);
        pop += 1;
    }
    let mut spawned_trees = 0usize;
    for (_, pos, edible, g) in tree_births {
        if tree_count + spawned_trees >= tcap {
            break; // tcap (decide gated on start-of-tick count; precise cap re-enforced here, deterministic since sorted)
        }
        spawn_tree(&mut commands, PLANT_START_MASS, pos, edible, g);
        spawned_trees += 1;
    }
    tree_bites.0.clear(); // consumed this tick
}

// predation (M5): creatures attack + eat each other. attack combat = bite + size; defense = attack + armor.
// NN-driven: brain attack output past ATTACK_INTENT_THRESH commits a hunt.
pub fn predation_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    mut commands: Commands,
    mut soil: ResMut<Soil>,
    mut cq: Query<(Entity, &Transform, &mut Energy, &mut Fitness, &mut Alive, &Genome, &mut Brain), With<Creature>>,
) {
    let _g = crate::profile::scope("predation");
    // snapshot living creatures: (entity, pos, ATTACK combat, energy, kin-sig, DEFENSE combat, venom, climb,
    // attack-intent, defend-intent). attack = bite + size; defense = attack + armor (armor protects, doesn't help
    // hunt); intents = brain out[2]/out[3] stashed this tick in live_step.
    let snap: Vec<(Entity, Vec3, f32, f32, [f32; 10], f32, f32, f32, f32, f32)> = cq
        .iter()
        .filter(|(_, _, _, _, a, _, _)| a.0)
        .map(|(e, t, en, _, _, g, b)| {
            let attack = g.bite + SIZE_COMBAT * g.size;
            (e, t.translation, attack, en.total(), signature(g), attack + ARMOR_DEF * g.armor, g.venom, g.climb, b.attack, b.defend)
        })
        .collect();
    if snap.len() < 2 {
        return;
    }
    let mut killed: HashSet<Entity> = HashSet::new();
    let mut gains: HashMap<Entity, f32> = HashMap::new();
    let mut committed: HashSet<Entity> = HashSet::new(); // attackers that chose to hunt this tick (intent > thresh)
    let mut defended: HashSet<Entity> = HashSet::new(); // prey that braced and repelled an attack
    let r2 = ATTACK_RADIUS * ATTACK_RADIUS;
    let rs2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
    for (ai, &(ae, apos, abite, _aenergy, _asig, _, _, _, a_atk_intent, _)) in snap.iter().enumerate() {
        if killed.contains(&ae) {
            continue; // a creature killed this tick doesn't also attack
        }
        // NN-DRIVEN: hunt iff brain attack output past threshold (no hunger gate). Mass-cannibalism check =
        // ATTACK_COST (paid in live_step) + learned R_WASTE penalty for whiffing, not a well-fed skip.
        if a_atk_intent <= ATTACK_INTENT_THRESH {
            continue;
        }
        committed.insert(ae);
        let mut best: Option<(f32, usize)> = None;
        for (bi, &(be, bpos, _, _, _, _, _, _, _, _)) in snap.iter().enumerate() {
            if bi == ai || killed.contains(&be) {
                continue;
            }
            let d2 = apos.distance_squared(bpos);
            if d2 < r2 && best.is_none_or(|(bd, _)| d2 < bd) {
                best = Some((d2, bi));
            }
        }
        if let Some((_, bi)) = best {
            let (be, bpos, _battack, _, bsig, bdef, bven, bclimb, _, b_def_intent) = snap[bi];
            // herd safety: prey surrounded by KIN is harder to pick off (vigilance) -> being social pays
            let mut kin = 0.0f32;
            for (e2, p2, _, _, s2, _, _, _, _, _) in &snap {
                if *e2 != be && bpos.distance_squared(*p2) < rs2 && sig_dist(&bsig, s2) < SOCIAL_SIM {
                    kin += 1.0;
                }
            }
            let prey_kin = (kin / SOCIAL_TARGET).min(1.0);
            // success = attacker combat vs prey EFFECTIVE defense (combat + armor + active BRACE), minus required
            // edge PREDATION_BIAS, reduced by herd safety AND prey climb agility (arboreal escape).
            let eff_def = bdef + BRACE_DEF * b_def_intent;
            let success = sigmoid(BITE_K * (abite - eff_def) - PREDATION_BIAS)
                * (1.0 - SOCIAL_SAFETY * prey_kin)
                * (1.0 - CLIMB_EVADE * bclimb);
            if rng.f32() < success {
                killed.insert(be);
                // venomous prey is a sickening kill -> the predator gains far less (the venom deterrent)
                *gains.entry(ae).or_insert(0.0) += PREDATION_GAIN * (1.0 - VENOM_DETER * bven);
            } else if b_def_intent > 0.5 {
                defended.insert(be); // attack repelled while actively bracing -> reward that defense
            }
        }
    }
    if committed.is_empty() {
        return; // nobody chose to attack -> no kills, no combat rewards to assign
    }
    let continuous_live = gen.continuous && gen.generation >= WARMUP_GENS;
    for (e, t, mut energy, mut fit, mut alive, gen_e, mut brain) in &mut cq {
        if let Some(g) = gains.get(&e) {
            energy.add_fat(*g, fat_cap(gen_e)); // a kill = meat -> fat store
            fit.0 += g * 0.3; // predation counts toward selection
            brain.fight_reward += R_KILL; // this attack paid off -> reinforce the attack output
        } else if committed.contains(&e) {
            brain.fight_reward += R_WASTE; // committed but landed nothing -> discourage pointless aggression
        }
        if defended.contains(&e) {
            brain.fight_reward += R_DEFEND; // braced + survived an attack -> reinforce the defend output
        }
        if killed.contains(&e) {
            alive.0 = false;
            let fat = (energy.fat / fat_cap(gen_e).max(0.01)).clamp(0.0, 1.0); // how fatty the prey was
            spawn_carrion(&mut commands, t.translation, CARRION_MASS * 0.5, fat); // predator already ate some
            soil.add(t.translation, DEATH_FERT); // death enriches the ground here
            if continuous_live {
                commands.entity(e).despawn();
            }
        }
    }
}

// carrion decomposition (P3): age each corpse, shrink mass, despawn when fully rotted. On full decomposition
// release fertility to soil at that spot (M5 closed loop: death -> soil).
pub fn rot_step(
    mut commands: Commands,
    mut soil: ResMut<Soil>,
    mut q: Query<(Entity, &mut Rot, &mut PlantState, &PlantGenome, &Transform)>,
) {
    let _g = crate::profile::scope("rot");
    for (e, mut rot, mut st, g, tf) in &mut q {
        rot.age += 1;
        st.mass = (st.mass - CARRION_MASS / ROT_GONE as f32).max(0.0); // decompose: less to scavenge
        if rot.age >= ROT_GONE {
            soil.add(tf.translation, DECOMP_FERT * g.nutrient); // return nutrients
            commands.entity(e).despawn(); // fully decomposed
        }
    }
}

// per-tick life: sense -> think -> move -> eat -> metabolism -> learn.
pub fn live_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    mut cq: Query<
        (Entity, &mut Transform, &mut Energy, &mut Fitness, &mut Heading, &mut Alive, &Genome, &mut Brain, &mut DietState, &mut Locomotion),
        (With<Creature>, Without<Food>),
    >,
    mut commands: Commands,
    mut tree_bites: ResMut<TreeBites>,
    mut soil: ResMut<Soil>,
    fire: Res<Fire>,
    fq: Query<(Entity, &Transform, &PlantState, &PlantGenome, Option<&Rot>, Option<&Tree>, Option<&Ferment>, Option<&Seed>), (With<Food>, Without<Creature>)>,
) {
    let _g = crate::profile::scope("live");
    let dt = DT;
    let ntypes = gen.ntypes();
    let mut pop = cq.iter().count(); // live population (continuous-mode reproduction cap)
    // continuous birth/death active only AFTER generational warm-up (WARMUP_GENS)
    let live_continuous = gen.continuous && gen.generation >= WARMUP_GENS;
    // daylight is POSITIONAL: computed per-creature from location, see `light` in the loop.
    // snapshot: (entity, pos, genome, mass, rot_age, tree, ferment_toxic, seed). rot_age=Some+ferment=None ->
    // animal carrion (meat); rot_age=Some+ferment=Some -> fermenting plant matter (fruit/detritus);
    // tree=Some(edible) -> a tree; else a living plant.
    let foods: Vec<(Entity, Vec3, PlantGenome, f32, Option<u32>, Option<bool>, Option<f32>, Option<PlantGenome>)> = fq
        .iter()
        .map(|(e, t, st, pg, rot, tree, ferment, seed)| {
            (e, t.translation, pg.clone(), st.mass, rot.map(|r| r.age), tree.map(|t| t.edible), ferment.map(|f| f.toxic), seed.map(|s| s.0.clone()))
        })
        .collect();
    let mut eaten: HashSet<Entity> = HashSet::new();
    // creature snapshot for social/kin need + threat sense: (entity, pos, signature, combat, body_radius).
    // combat = bite + size, so a creature senses a bigger-combat neighbor as predator (flee).
    let cre_snap: Vec<(Entity, Vec3, [f32; 10], f32, f32)> = cq
        .iter()
        .filter(|(_, _, _, _, _, a, _, _, _, _)| a.0)
        .map(|(e, t, _, _, _, _, g, _, _, _)| (e, t.translation, signature(g), g.bite + SIZE_COMBAT * g.size, body_radius(g)))
        .collect();
    // per-niche live counts: continuous repro tapers on the breeder's OWN niche fill (NICHE_CAP), not global
    // pop -> each habitat self-limits independently so no niche grabs the shared cap (was winner-take-all).
    let mut niche_pop = [0usize; crate::niche::NICHE_COUNT];
    for (_, _, _, _, _, a, g, _, _, _) in cq.iter() {
        if a.0 {
            niche_pop[crate::niche::niche_of(g).idx()] += 1;
        }
    }
    // mating mode: pool of (entity, pos, signature, genome) so a breeding creature finds a nearby
    // genetically-similar MATE to cross. Built only when --mating (cloning genomes isn't free).
    let mate_pool: Vec<(Entity, Vec3, [f32; 10], Genome)> = if gen.mating {
        cq.iter()
            .filter(|(_, _, _, _, _, a, _, _, _, _)| a.0)
            .map(|(e, t, _, _, _, _, g, _, _, _)| (e, t.translation, signature(g), g.clone()))
            .collect()
    } else {
        Vec::new()
    };
    // bin food indices into the spatial grid (built once per tick)
    let mut fgrid: Vec<Vec<u32>> = vec![Vec::new(); FGRID * FGRID];
    for (i, f) in foods.iter().enumerate() {
        let (fu, fv) = fcell_uv(f.1);
        fgrid[fv * FGRID + fu].push(i as u32);
    }

    for (entity, mut ct, mut energy, mut fit, mut head, mut alive, genome, mut brain, mut diet, mut loco) in &mut cq {
        if !alive.0 {
            continue;
        }
        let pos = ct.translation;
        let fat_max = fat_cap(genome); // adiposity + size set this creature's fat-store ceiling

        // SINGLE grid-bounded pass over nearby foods (perf): scan only food-grid cells within query radius
        // (>= max sensor range so every sensable food covered; >= NEAR_QUERY so global-nearest found given dense
        // plants). Computes global nearest (approach/eat) AND each sensor's nearest-food-in-cone together, atan2
        // once per in-range food. NOT bit-identical (cell iteration order differs) but far fewer foods touched ->
        // big speedup at scale.
        let n_s = genome.sensors.len();
        // eyes gene: more eyes = small effective-range boost (sharper sight) on every sensor.
        let eye_mult = 1.0 + EYE_SENSE_BONUS * genome.eyes;
        let max_range = genome.sensors.iter().map(|s| s.range).fold(0.0f32, f32::max) * eye_mult;
        let mut best: Option<(usize, f32)> = None;
        let mut sd = vec![f32::INFINITY; n_s]; // nearest dist per sensor
        let mut skind = vec![0u8; n_s]; // food kind of nearest sensor-food
        let mut nearest_tree_d2 = f32::INFINITY; // nearest tree (canopy shade), any kind
        let flier = genome.flight >= FLIGHT_KNEE; // bird: picks fruit from the canopy by HORIZONTAL distance (any altitude)
        let _r = max_range.max(NEAR_QUERY);
        // scan a neighborhood of food-grid (lon/lat) cells around this creature. SPAN cells each way covers
        // sensor + near-query radius at this grid res. (Longitude doesn't wrap here + pole cells narrow -> minor
        // perception approximation near date line/poles; food dense so fine.)
        const SPAN: usize = 3;
        let (pu, pv) = fcell_uv(pos);
        let (cu0, cu1) = (pu.saturating_sub(SPAN), (pu + SPAN).min(FGRID - 1));
        let (cv0, cv1) = (pv.saturating_sub(SPAN), (pv + SPAN).min(FGRID - 1));
        let pdir = pos.normalize_or_zero();
        let (east, north) = crate::sphere::tangent_frame(pdir);
        for cv in cv0..=cv1 {
            for cu in cu0..=cu1 {
                for &fi in &fgrid[cv * FGRID + cu] {
                    let i = fi as usize;
                    let f = &foods[i];
                    if eaten.contains(&f.0) {
                        continue;
                    }
                    let to = f.1 - pos;
                    let mut d2 = to.length_squared();
                    // fruit tree (f.5 = Some(true)): fruit is up in the CANOPY. A flier targets it by HORIZONTAL
                    // distance (strip the altitude gap) so a bird cruising near a tree homes on the crown fruit
                    // from any height. Ground creatures keep 3D distance (reach the base, climb/height gates up).
                    if flier && matches!(f.5, Some(true)) {
                        let horiz = to - to.dot(pdir) * pdir; // remove radial (altitude) component
                        d2 = horiz.length_squared();
                    }
                    if best.is_none_or(|(_, bd2)| d2 < bd2) {
                        best = Some((i, d2));
                    }
                    if f.5.is_some() && d2 < nearest_tree_d2 {
                        nearest_tree_d2 = d2; // f.5 = tree marker -> overhead canopy for shade
                    }
                    let dist = d2.sqrt();
                    if dist > max_range {
                        continue; // out of every sensor range -> skip bearing + cone tests
                    }
                    // bearing in local tangent frame: 0 = north (toward +Y pole), +pi/2 = east
                    let bearing = wrap_angle(to.dot(east).atan2(to.dot(north)) - head.0);
                    for (si, s) in genome.sensors.iter().enumerate() {
                        if dist <= s.range * eye_mult && dist < sd[si] && wrap_angle(bearing - s.angle).abs() <= CONE_HALF {
                            sd[si] = dist;
                            skind[si] = f.2.kind;
                        }
                    }
                }
            }
        }
        let cur_dist = best.map(|(_, d2)| d2.sqrt()).unwrap_or(f32::INFINITY);
        // positional day/night: daylight depends on WHERE on the globe (lit half faces sun, terminator sweeps as
        // planet spins) -> light niches vary by location.
        let light = crate::sphere::daylight_at(pdir, gen.tick);
        // aquatic factor here: 1 in ocean / wet lowland, 0 on high dry ground (low elevation = wet). Swimmers move
        // faster here (fins) + pay on dry land (see metabolism); also a brain input.
        let h0 = crate::sphere::elevation(pdir);
        let wet_here = ((SWIM_WET_LEVEL - h0) / SWIM_WET_LEVEL).clamp(0.0, 1.0);
        // aerial factor: how high aloft (0 grounded .. 1 ceiling) from last tick's altitude. Drives flier speed
        // bonus + altitude-hold cost + a brain input. Swimmers (low flight gene) zero out via flight-gated terms.
        let air_here = (loco.alt / MAX_FLIGHT_ALT).clamp(0.0, 1.0);
        // shade: how shaded by overhead canopy (near a tree). Relieves open-sun heat + brain input to seek it.
        let tree_shade = if nearest_tree_d2.is_finite() {
            (1.0 - nearest_tree_d2.sqrt() / SHADE_RADIUS).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let shade01 = tree_shade; // overhead canopy shade (brain input + heat relief)
        // nearest THREAT: a bigger-combat creature nearby drives flee. O(n) over the snapshot.
        let my_combat = genome.bite + SIZE_COMBAT * genome.size;
        let mut threat_d2 = f32::INFINITY;
        let mut threat_pos = Vec3::ZERO;
        for (e2, p2, _, c2, _) in &cre_snap {
            if *e2 == entity || *c2 <= my_combat + THREAT_MARGIN {
                continue;
            }
            let d2 = pos.distance_squared(*p2);
            if d2 < threat_d2 {
                threat_d2 = d2;
                threat_pos = *p2;
            }
        }
        let (threat_dist, threat_bear) = if threat_d2 < THREAT_RADIUS * THREAT_RADIUS {
            let to = threat_pos - pos;
            let bearing = wrap_angle(to.dot(east).atan2(to.dot(north)) - head.0);
            (1.0 / (1.0 + threat_d2.sqrt()), bearing / std::f32::consts::PI)
        } else {
            (0.0, 0.0)
        };

        // magnetoreception (gated by `magneto` gene): expression scales both inputs + upkeep cost, so a
        // switched-off sense feeds ~0 (brain ignores) for ~0 cost.
        let mexpr = crate::genome::mag_expression(genome.magneto);
        // build inputs from EVOLVABLE sensors: each is a directional eye reporting nearest food in its cone
        // (+ type). GA decides sensor count + where they point.
        let mut input: Vec<f32> = Vec::with_capacity(n_s * SIG_PER_SENSOR + GLOBAL_INPUTS);
        for si in 0..n_s {
            if sd[si].is_finite() {
                input.push(1.0 / (1.0 + sd[si])); // inv-distance in this sensor's cone
                // food TYPE (family) seen, normalized -1..1. Digestibility depends on nutrient profile vs gut
                // uptake genes (not kind), so eye reports identity; brain + lifetime learning associate types
                // with good/bad outcomes.
                input.push((skind[si] as f32 / (ntypes.max(2) - 1) as f32) * 2.0 - 1.0);
            } else {
                input.push(0.0);
                input.push(0.0);
            }
        }
        input.push(energy.total() / START_ENERGY);
        input.push(light * 2.0 - 1.0); // daylight signal (-1 night .. +1 noon): brain can time rest
        input.push(diet.fatigue * 2.0 - 1.0); // exertion debt: brain "feels tired" + can choose to rest
        input.push(1.0); // bias
        // M4 global inputs (order MUST match GLOBAL_INPUTS + ensure_net_shape padding):
        input.push((diet.toxic_load / TOX_LOAD_CAP).clamp(0.0, 1.0)); // own toxic load -> avoid poison
        input.push(shade01); // overhead canopy shade -> seek in heat
        input.push(threat_dist); // nearest bigger-predator inv-distance -> flee
        input.push(threat_bear); // bearing to that predator (-1..1) -> which way to flee
        input.push(wet_here); // submersion / in-water
        // magneto globals (last, matching ensure_net_shape padding order). ~0 when sense off:
        input.push(crate::sphere::mag_latitude(pdir) * mexpr); // magnetic latitude "map" (-1 .. +1)
        input.push(wrap_angle(crate::sphere::mag_north_bearing(pdir) - head.0) / std::f32::consts::PI * mexpr); // compass: rel bearing to magnetic north
        input.push(air_here); // own altitude fraction (LAST global, see GLOBAL_INPUTS): flier/diver manages climb/descend

        // think (per-life learned brain, dynamic topology matching this genome's sensor count)
        let (h, out) = forward(&brain.net, &input);
        // power clamp: thrust limited by instantly-available power (fat mobilizes slow) -> fat-only or near-starved
        // creature can't sprint. The "slow burning" teeth of the fat store.
        let power_frac = (energy.power() / MOVE_POWER_REF).clamp(0.0, 1.0);
        let thrust = out[0] * power_frac;
        let turn = out[1];
        // stash combat intents for predation_step (runs later this tick): attack drives hunting, defend/brace
        // raises defense there at the cost of mobility (applied here).
        brain.attack = out[2];
        brain.defend = out[3];
        let sprint = out[5]; // burst-effort 0..1: faster chase OR flee, paid in energy + fatigue below
        // fatigue saps usable output (tired = sluggish); intended effort still costs full MOVE_COST below, so
        // flailing while exhausted is a net loss -> resting to recover is the only way out. Bracing (defend)
        // immobilizes: trade ground speed for a harder-to-kill stance.
        let move_thrust = thrust * (1.0 - FATIGUE_DRAG * diet.fatigue) * (1.0 - BRACE_DRAG * out[3]);
        // metabolic tempo: frugal (metab>0.5) trades top speed for cheaper basal; fast (metab<0.5) reverse
        let metab_f = genome.metab - 0.5; // -0.5 fast .. +0.5 frugal
        let speed = MOVE_SPEED
            * (1.0 + SWIM_SPEED * genome.swim * wet_here) // swimmers fast in water
            * (1.0 + FLIGHT_SPEED * genome.flight * air_here) // fliers fast aloft (gene x altitude)
            * (1.0 + LIMB_TRACTION * genome.limbs * (1.0 - wet_here)) // more legs = land traction (ground speed)
            * (1.0 - 0.5 * metab_f)
            * (1.0 + SPRINT_BOOST * sprint); // sprint: burst speed for chase/flee

        // act: turn, then take a great-circle step along the heading over the planet surface
        head.0 = wrap_angle(head.0 + turn * TURN_SPEED * dt);
        let (nd, nh) = crate::sphere::step(pos, head.0, move_thrust * speed * dt);
        head.0 = nh; // heading parallel-transported into the new tangent frame
        let h1 = crate::sphere::elevation(nd);
        // vertical DOF (out[6] rise intent): fliers climb into the sky, swimmers rise off the seafloor through the
        // water column toward the waterline. Ceiling set by medium; gravity sinks fliers, swimmers ~neutral
        // buoyancy (hover, gentler vertical). Land creatures: ceiling 0 -> pinned to surface (no behavior change).
        let rise = (out[6] - 0.5) * 2.0; // -1 sink/dive .. +1 climb
        let (climb_rate, ceil) = if genome.flight >= FLIGHT_KNEE {
            (FLIGHT_CLIMB_RATE, MAX_FLIGHT_ALT * genome.flight) // sky: higher flight gene -> higher ceiling
        } else if genome.swim >= SWIM_DROWN_MIN {
            // ocean only (-h1 = water column above seafloor, >0 in sea). Rise toward waterline, stay submerged
            // (CREATURE_Y base + 0.4 margin below surface). Dry land: -h1<0 -> ceil 0.
            (FLIGHT_CLIMB_RATE * 0.6, ((-h1 - CREATURE_Y - 0.4) * genome.swim).max(0.0))
        } else {
            (0.0, 0.0) // grounded walker: pinned to surface (no behavior change)
        };
        // brain climbs/descends, then neutral buoyancy relaxes toward cruise -> a neutral brain hovers aloft /
        // mid-water (visible birds + fish); landing to eat = a sustained descend that overcomes buoyancy.
        let cruise = FLIGHT_CRUISE * ceil;
        loco.alt += rise * climb_rate * dt;
        loco.alt += (cruise - loco.alt) * FLIGHT_BUOYANCY * dt;
        loco.alt = loco.alt.clamp(0.0, ceil);
        let np = crate::sphere::surface_pos(nd, CREATURE_Y + loco.alt); // ride terrain + vertical offset (sky / water column)
        // pay for elevation change (P3): uphill costs, downhill partially refunds
        let dh = h1 - h0;
        let climb = if dh > 0.0 { CLIMB_COST * dh } else { DESCEND_REFUND * dh };
        if climb > 0.0 {
            energy.burn(climb); // uphill burns
        } else {
            energy.add_fast(-climb, FAST_CAP); // downhill coasting refunds a little quick energy
        }
        loco.path += np.distance(pos); // accumulate 3D distance walked (diagnostic)
        ct.translation = np;
        // orient body: local +Y = surface normal (upright), local +Z = travel dir (head/eyes at +Z) -> creature
        // FACES where it walks instead of yawing arbitrarily.
        let up = nd; // outward surface normal (unit)
        let fwd = crate::sphere::heading_tangent(nd, nh); // unit tangent along heading
        let right = up.cross(fwd).normalize_or_zero();
        ct.rotation = Quat::from_mat3(&Mat3::from_cols(right, up, fwd));

        // body collision: creatures are solid. Sum overlap vs every neighbor whose collision radius intersects
        // ours (snapshot positions), shove out along surface tangent so bodies don't stack, pay a jostle energy
        // cost. SOCIAL creatures crowd-tolerant -> cost x(1-social).
        let my_r = body_radius(genome);
        let mut push = Vec3::ZERO;
        let mut overlap_sum = 0.0f32;
        for (e2, p2, _, _, r2) in &cre_snap {
            if *e2 == entity {
                continue;
            }
            let to = np - *p2;
            let d2 = to.length_squared();
            let rr = my_r + *r2;
            if d2 < rr * rr && d2 > 1e-6 {
                let d = d2.sqrt();
                let depth = rr - d;
                overlap_sum += depth;
                push += (to / d) * depth; // away from the neighbor, weighted by penetration depth
            }
        }
        if overlap_sum > 0.0 {
            // shove only along surface tangent (drop radial part) so creature stays on the shell
            let tangential = push - up * push.dot(up);
            if tangential.length_squared() > 1e-9 {
                let shoved = (np + tangential * SEPARATION_STRENGTH).normalize_or_zero();
                ct.translation = crate::sphere::surface_pos(shoved, CREATURE_Y + loco.alt); // keep altitude through shove
            }
            // jostle hurts: crowd-tolerant (social) herder barely feels it, a loner gets drained
            energy.burn(COLLIDE_COST * overlap_sum * (1.0 - genome.social) * dt);
        }

        // drowning (hard kill): NON-aquatic creature (swim below SWIM_DROWN_MIN) in genuinely deep OPEN ocean
        // drowns outright. Shallow/coastal water stays crossable (gradual WATER_PRESSURE_COST handles wading);
        // only submersion past DROWN_DEPTH lethal, so only real swimmers live at sea. Leaves no carrion (corpse
        // sinks to abyss, unforageable).
        if genome.swim < SWIM_DROWN_MIN && loco.alt < GROUND_EPS {
            // grounded only: a flier crossing above open ocean (alt >= GROUND_EPS) doesn't drown
            let sub = ((crate::sphere::SEA_LEVEL - crate::sphere::elevation01(nd)) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
            if sub > DROWN_DEPTH {
                alive.0 = false;
                continue;
            }
        }

        // metabolism: basal + movement (convex in speed) + bite upkeep + rocky crossing + vision upkeep.
        // More/longer sensors see farther but cost energy (SENSE_COST x total range) -> range trade-off.
        let rock = crate::sphere::rockiness(nd);
        let lifespan_mult = 0.4 + 1.2 * genome.longevity; // 0.4 -> 1.6 around 1.0 baseline; upkeep + aging
        let sense_range: f32 = genome.sensors.iter().map(|s| s.range).sum();
        let fat_frac = energy.fat / fat_max.max(0.01); // 0..1 fat-store fullness (drives upkeep)
        // thermal niche split into cold + warm sides so pelt (fur) insulates the COLD side only.
        let temp_here = crate::sphere::base_temperature(pdir);
        let cold_miss = (genome.temp_pref - temp_here).max(0.0); // colder than preferred (pelt helps)
        let warm_miss = (temp_here - genome.temp_pref).max(0.0); // warmer than preferred (pelt hurts)
        energy.burn((BASAL_COST * (1.0 - 0.6 * metab_f) // frugal metabolism lowers the cost of living
            + SIZE_BASAL * genome.size.powf(SIZE_BASAL_EXP) // size = energy use: basal scales allometrically with mass
            + MOVE_COST * (1.0 + SIZE_MOVE * genome.size + ARMOR_MOVE * genome.armor + LIMB_MOVE_COST * genome.limbs) * thrust * thrust // mass + plates + legs to push
            + BITE_COST * genome.bite
            + ROCK_MOVE_COST * rock * thrust.abs() * (1.0 - ALPINE_RELIEF * genome.alpine) // alpine climbers cross rock cheaply
            + ALPINE_FLAT_COST * genome.alpine * (1.0 - rock) // heavy mountain build wastes energy on flat ground
            + CLIMB_FLAT_COST * genome.climb * (1.0 - rock) // arboreal build wastes energy on open flat ground
            + SENSE_COST * sense_range
            + BRAIN_COST * (1.0 - HEAD_BRAIN_RELIEF * genome.head) * genome.net.ih.len() as f32 // a roomy head houses the brain cheaper
            + HEIGHT_COST * genome.height
            + LIGHT_COST * (light - genome.light_pref).abs() // positional daylight at this creature's location
            + TEMP_COST * (warm_miss + cold_miss * (1.0 - PELT_COLD_RELIEF * genome.pelt)) // fur insulates the cold side
            + HEAT_SUN_COST * (light * temp_here - HEAT_COMFORT).max(0.0) * (1.0 - SHADE_RELIEF * shade01) // open-sun heat: seek shade
            + PELT_HEAT_COST * genome.pelt * temp_here // a coat overheats in hot places
            + PELT_WATER_DRAG * genome.pelt * wet_here // a waterlogged coat drags in water
            + PELT_UPKEEP * genome.pelt // growing + carrying a coat
            + DETOX_COST * genome.detox // running a liver to clear toxins
            + ARMOR_BASAL * genome.armor // armor plate upkeep
            + VENOM_UPKEEP * genome.venom // making toxins costs
            + EYE_COST * genome.eyes // eyes are metabolically pricey
            + HEAD_BASAL * genome.head // carrying a big head
            + MAG_COST * mexpr // magnetoreception organ + neural processing (no free lunch)
            + LONGEVITY_COST * (lifespan_mult - 1.0).max(0.0) // a long-lived body costs more to maintain
            + SWIM_LAND_COST * genome.swim * (1.0 - wet_here) // fins are a liability on dry land
            + (FLIGHT_ALT_COST + FLIGHT_SIZE_LIFT * genome.size) // wing loading: heavy body costs more to lift -> selects SMALL birds
                * genome.flight * air_here
                * (1.0 - GLIDE_RELIEF * genome.flight * move_thrust.abs()).max(0.25) // glide: fast forward flight soars cheap (airspeed lift)
            + FLIGHT_GROUND_COST * genome.flight * (1.0 - air_here) // big wings clumsy when grounded (mirror SWIM_LAND_COST)
            + WATER_PRESSURE_COST * (1.0 - genome.swim) * (-h1 / crate::sphere::SEA_FLOOR_MAX).clamp(0.0, 1.0) // non-swimmers struggle in deep water (depth pressure)
            + FAT_UPKEEP * genome.adiposity * fat_frac // carrying fat costs upkeep (no free lunch)
            + SPRINT_COST * sprint // burst effort burns extra fuel
            + ATTACK_COST * brain.attack // committing to an attack costs energy whether or not it lands (the stabilizer replacing the hunger gate)
            + STRESS_COST * diet.fatigue)
            * dt);
        // fast store leaks even at rest (volatile -> can't bank quick energy, use-it-or-lose-it)
        energy.fast = (energy.fast - FAST_LEAK * dt).max(0.0);
        // fatigue dynamics: exertion (thrust) accrues debt; idling (low thrust) sheds it. Clamped 0..1.
        diet.fatigue = (diet.fatigue + (FATIGUE_GAIN * thrust + SPRINT_FATIGUE * sprint - FATIGUE_REST * (1.0 - thrust)) * dt).clamp(0.0, 1.0);
        // wildfire: standing in fire burns energy fast (deadly to anything in a blaze)
        let here_fire = fire.get(np);
        if here_fire > 0.05 {
            energy.burn(FIRE_DAMAGE * here_fire * dt);
        }
        // social/kin need: a social creature isolated from kin drains energy (loneliness). A herd of kin removes
        // the drain (+ grants predation safety, see predation_step). Drain SCALED BY DENSITY (pop/CREATURE_CAP):
        // at healthy density social creatures must herd, but sparse population relaxes the pressure -> no Allee
        // death-spiral (constant loneliness drain on a spread-out pop feeds back to extinction; this self-limits).
        if genome.social > 0.0 {
            let kinf = kin_fraction(entity, np, &signature(genome), &cre_snap);
            let density = (pop as f32 / CREATURE_CAP as f32).min(1.0);
            energy.burn(SOCIAL_COST * genome.social * (1.0 - kinf) * density * dt);
        }

        // eat nearest plant on contact, IF bite beats its defense (arms race, spec 13)
        let mut eat_reward = 0.0;
        if let Some((i, _)) = best {
            let (e, fp, mass) = (foods[i].0, foods[i].1, foods[i].3);
            let pg = foods[i].2.clone();
            let rot_age = foods[i].4;
            let tree = foods[i].5; // None=plant/carrion, Some(true)=fruit tree, Some(false)=evergreen
            let ferment = foods[i].6; // Some(toxic) -> fermenting plant matter (fruit/detritus); None+rot -> meat
            let seed = foods[i].7.clone(); // Some(genome) -> fallen fruit carrying a viable seed (planted if eaten ripe)
            // eat-gate (out[4]): ingestion is a CHOICE -> brain can refuse bad food (unripe/spoiled/toxic).
            // EAT_GATE sits BELOW the fresh-net 0.5 baseline so founders feed before learning (no gen-0 starve).
                // fruit-tree eating uses HORIZONTAL distance for a flier (picks from the canopy at any altitude);
                // everyone else uses 3D (reach the base from the ground). Plants/carrion always 3D.
                let eat_dist = if flier && matches!(tree, Some(true)) {
                    let to = fp - np;
                    let pnorm = np.normalize_or_zero();
                    (to - to.dot(pnorm) * pnorm).length() // strip altitude gap -> horizontal proximity to crown
                } else {
                    np.distance(fp)
                };
                if out[4] > EAT_GATE && eat_dist < EAT_RADIUS {
                // fruit hangs at the crown, pulled DOWN by branches: branchy tree offers fruit LOW (short creatures
                // reach it); a bare tree holds fruit only at the TOP (tall/climbers/fliers). fruit_height = tree
                // height minus the branch drop. Ground creature reaches up to height+margin+climb; FLIERS reach
                // ANY crown (wings clear the canopy) = flight payoff + birds become fruit dispersers (a low-height
                // bird still counts as "short" below -> 0 tree damage, just picks + disperses fruit). Evergreens
                // never eatable. Plants/carrion: bite vs defense.
                let fruit_height = (pg.height - pg.branches * BRANCH_REACH).max(0.0);
                let ground_reach = genome.height + TREE_REACH_MARGIN + CLIMB_REACH * genome.climb;
                let success = match tree {
                    Some(edible) => edible && (flier || ground_reach >= fruit_height),
                    // plant: creature must be tall enough to reach it (height defense) AND bite its defense
                    None => {
                        genome.height + 0.15 >= pg.height
                            && rng.f32() < sigmoid(BITE_K * (genome.bite - pg.defense))
                    }
                };
                if success {
                    // digestion efficiency = MASTER expression gene (reserves vs uptake demand). Gates energy
                    // from ALL food in diet mode; legacy --no-diet ungated (eff=1).
                    let eff = if gen.diet { master_expression(&genome.uptake, &diet.reserves, RESERVE_REQ, MASTER_FLOOR) } else { 1.0 };
                    let fert = soil.get(np);
                    let soil_f = 1.0 - SOIL_NUTRI + SOIL_NUTRI * (fert / FERT_CAP).min(1.0); // richer soil -> more nutrients delivered
                    if let Some(true) = tree {
                        // FRUIT TREE: persists + regrows; dies only if grazed below TREE_MIN_MASS. Mass dilutes
                        // nutrition (TREE_MASS_NUTRI): bulkier tree gives less energy/bite. Creature too SHORT to
                        // reach without branches feeds HARMLESSLY (0 mass damage); tall enough to reach crown
                        // strips mass + can over-graze. Either way feeding is recorded (triggers dispersal).
                        let bite_mass = TREE_BITE_MASS.min(mass);
                        let mass_nutri = 1.0 - TREE_MASS_NUTRI * (mass / TREE_MATURITY).min(1.0);
                        let base = bite_mass * pg.nutrient * mass_nutri * (0.5 + pg.quality);
                        let wasted = energy.add_sugar(EAT_GAIN * base * eff, SUGAR_CAP, fat_max); // fruit flesh -> sugar
                        diet.g += wasted * OVEREAT_G; // overflow -> growth-load (gorging harms)
                        fit.0 += base * eff;
                        if gen.diet {
                            let toxin = absorb_and_toxin(&mut diet.reserves, &genome.uptake, &pg, soil_f, base);
                            diet.toxic_load = (diet.toxic_load + toxin * TOX_LOAD_GAIN).min(TOX_LOAD_CAP); // toxin -> accumulating toxic load
                        }
                        let short = genome.height + TREE_REACH_MARGIN < pg.height; // only reached via branches
                        let damage = if short { 0.0 } else { bite_mass }; // branch-feeders don't harm the tree
                        *tree_bites.0.entry(e).or_insert(0.0) += damage;
                        eat_reward = R_EAT;
                    } else if let (Some(age), Some(toxic)) = (rot_age, ferment) {
                        // FERMENTING PLANT MATTER (fallen fruit / detritus): 3-stage clock over ROT_GONE.
                        //   fresh (< FERMENT_START)   -> SUGAR (just ripe fruit/greens) + nutrients
                        //   fermenting (START..END)   -> FAST energy (ethanol), few nutrients + toxicity
                        //   spoiled (>= END)          -> near-zero yield, full toxicity (avoid)
                        // fruit (low `toxic`) ferments richly; detritus (high `toxic`) = scraps + poison.
                        let f = (age as f32 / ROT_GONE as f32).clamp(0.0, 1.0);
                        let base = mass * pg.nutrient * (0.5 + pg.quality);
                        // fast-energy yield is whichever source this is (fruit rich, detritus poor)
                        let fast_gain = if pg.nutrient >= DETRITUS_NUTRIENT + 0.05 { FRUIT_FAST_GAIN } else { DETRITUS_FAST_GAIN };
                        if f < FERMENT_START {
                            if seed.is_some() {
                                // FRUIT, pre-ferment: RIPENESS gradient. r=0 just-dropped (unripe) .. 1 ripe at
                                // RIPEN_FRAC. Unripe = little sugar + genetic fruit_toxicity (`toxic`); ripening
                                // sweetens + detoxifies, so ripe fruit sweet+safe EVEN IF body toxic (fruit toxicity
                                // decoupled from body). Eating unripe bitter/poisonous -> brain learns to wait, +
                                // the despawn block destroys the unripe seed (no reproduction).
                                let r = (f / RIPEN_FRAC).min(1.0);
                                let yield_mult = UNRIPE_YIELD + (1.0 - UNRIPE_YIELD) * r;
                                let wasted = energy.add_sugar(EAT_GAIN * base * yield_mult * eff, SUGAR_CAP, fat_max);
                                diet.g += wasted * OVEREAT_G;
                                fit.0 += base * yield_mult * eff;
                                if gen.diet {
                                    // tops reserves (ripe fruit nutrition); discard absorb's body-toxin (fruit uses its own)
                                    let _ = absorb_and_toxin(&mut diet.reserves, &genome.uptake, &pg, soil_f, base * yield_mult);
                                }
                                let toxin = TOXIN_MAX * toxic * (1.0 - r); // full fruit_toxicity unripe -> ~0 when ripe
                                diet.toxic_load = (diet.toxic_load + toxin * TOX_LOAD_GAIN).min(TOX_LOAD_CAP); // toxin -> accumulating toxic load
                                eat_reward = if r < 1.0 { -1.0 + r } else { R_EAT }; // unripe bad, ripe good
                            } else {
                            // fresh detritus/greens (no seed): sugary plant (gated by eff + tops nutrient reserves)
                            let wasted = energy.add_sugar(EAT_GAIN * base * eff, SUGAR_CAP, fat_max);
                            diet.g += wasted * OVEREAT_G;
                            fit.0 += base * eff;
                            if gen.diet {
                                let toxin = absorb_and_toxin(&mut diet.reserves, &genome.uptake, &pg, soil_f, base);
                                diet.toxic_load = (diet.toxic_load + toxin * TOX_LOAD_GAIN).min(TOX_LOAD_CAP); // toxin -> accumulating toxic load
                            }
                            eat_reward = R_EAT;
                            }
                        } else {
                            // ferment-ness ramps 0->1 across the window, then spoilage kills the yield. Ethanol =
                            // empty calories: FAST energy, no nutrient reserves (fermentation degrades them).
                            let fermentness = ((f - FERMENT_START) / (FERMENT_END - FERMENT_START)).clamp(0.0, 1.0);
                            let spoiled = ((f - FERMENT_END) / (1.0 - FERMENT_END)).clamp(0.0, 1.0);
                            let yield_mult = fermentness * (1.0 - spoiled);
                            let wasted = energy.add_fast(fast_gain * base * yield_mult, FAST_CAP); // ethanol -> FAST store
                            diet.g += wasted * OVEREAT_G;
                            fit.0 += base * yield_mult;
                            // toxicity: scales with how fermented/spoiled it is x this matter's toxic factor
                            let toxin = TOXIN_MAX * toxic * (0.3 * fermentness + spoiled);
                            diet.toxic_load = (diet.toxic_load + toxin * TOX_LOAD_GAIN).min(TOX_LOAD_CAP); // toxin -> accumulating toxic load
                            eat_reward = (yield_mult - spoiled) * 2.0 - 1.0; // ripe ferment good, spoiled bad
                        }
                    } else if let Some(age) = rot_age {
                        // CARRION / MEAT (P3): eating another creature = TOP nutrition, near-zero toxicity while
                        // fresh, richer + longer-lasting than plants (MEAT_BONUS). Toxin ramps only once well-rotted
                        // (>60%). Not gated by master expr; balanced for reserves.
                        let f = (age as f32 / ROT_GONE as f32).clamp(0.0, 1.0); // 0 fresh .. 1 rotten
                        let freshness = 1.0 - (f / 0.6).min(1.0); // ~1 for first 60% of decomposition
                        let meat = mass * pg.nutrient * freshness;
                        let toxin = TOXIN_MAX * ((f - 0.6) / 0.4).max(0.0); // no toxin until 60% rotted
                        // RABBIT STARVATION (real mechanic): usable ENERGY of a carcass = its FAT. Lean meat (low
                        // fat) = mostly PROTEIN; converting protein to usable energy/fat needs CARBS (eater sugar
                        // store). Eating lean prey with no carbs yields little energy AND dumps unconvertible
                        // protein as metabolic toxic load (ammonia) -> obligate carnivore on lean kills starves +
                        // poisons itself. Fatty prey (or plant carbs) avoids it. `gut` (carnivory) scales extraction.
                        let fat_content = pg.quality.clamp(0.0, 1.0); // prey fatness carried on the carrion
                        let gut = (PROTEIN_FLOOR + PROTEIN_CARN * genome.carnivory).min(1.0);
                        let carb_buffer = (energy.sugar / SUGAR_CAP).clamp(0.0, 1.0); // carbs to build fat from protein
                        let meat_e = EAT_GAIN * MEAT_BONUS * meat * gut; // total extractable IF fully usable
                        let fat_part = meat_e * fat_content; // fat = direct energy, no carbs needed
                        let protein_part = meat_e * (1.0 - fat_content); // lean protein: only usable WITH carbs
                        let protein_usable = protein_part * carb_buffer;
                        let protein_wasted = protein_part * (1.0 - carb_buffer); // rabbit starvation: ammonia
                        let gain = fat_part + protein_usable;
                        // meat -> mostly fat store, a little sugar; fat overflow spills to sugar
                        let overflow_fat = energy.add_fat(gain * 0.8, fat_max);
                        let wasted = energy.add_sugar(gain * 0.2 + overflow_fat, SUGAR_CAP, fat_max);
                        diet.g += wasted * OVEREAT_G;
                        fit.0 += (fat_part + protein_usable) / (EAT_GAIN * MEAT_BONUS).max(1.0); // fitness ~ usable energy
                        // balanced animal tissue: tops EVERY nutrient reserve (meat is nutritionally complete)
                        if gen.diet {
                            for r in diet.reserves.iter_mut() {
                                *r = (*r + MEAT_RESERVE * freshness).min(RESERVE_CAP);
                            }
                        }
                        // rot toxin + unconvertible-protein ammonia -> accumulating toxic load
                        diet.toxic_load = (diet.toxic_load + toxin * TOX_LOAD_GAIN + protein_wasted * PROTEIN_TOX).min(TOX_LOAD_CAP);
                        eat_reward = freshness * 2.0 - 1.0; // fresh -> +1 (good), rotten -> -1 (avoid)
                    } else {
                        // regular plant: strip a fraction set by `regrow`: carrot (~whole) vs berry bush (small
                        // bite, persists). Recorded as grazing; plant_step reduces mass / despawns.
                        let frac = (1.0 - 0.85 * pg.regrow).clamp(0.12, 1.0);
                        let bite_mass = mass * frac;
                        *tree_bites.0.entry(e).or_insert(0.0) += bite_mass;
                        // quality scales extractable energy: factor 0.5..1.5, ~1.0 at quality 0.5 (balance-neutral)
                        let base = bite_mass * pg.nutrient * (0.5 + pg.quality);
                        let wasted = energy.add_sugar(EAT_GAIN * base * eff, SUGAR_CAP, fat_max); // plant -> sugar
                        diet.g += wasted * OVEREAT_G;
                        fit.0 += base * eff;
                        if gen.diet {
                            let toxin = absorb_and_toxin(&mut diet.reserves, &genome.uptake, &pg, soil_f, base);
                            diet.toxic_load = (diet.toxic_load + toxin * TOX_LOAD_GAIN).min(TOX_LOAD_CAP); // toxin -> accumulating toxic load
                            eat_reward = (eff - 0.5) * 4.0; // reward eating most when well-nourished (high master expr)
                        } else {
                            eat_reward = R_EAT;
                        }
                    }
                    // overeating trade-off (spec 12): each store capped; eating past it converts excess into
                    // growth-load (harm, via add_* helpers above) -> gorging shortens life. Carrion eaten whole
                    // (despawn). Plants + trees PERSIST: mass reduced by the grazing recorded above; plant_step
                    // despawns any grazed below its min mass.
                    if tree.is_none() {
                        eaten.insert(e); // prevent same-tick re-eat
                        if let Some(seed_g) = seed {
                            // FALLEN FRUIT: eaten whole (despawn). RIPE (past RIPEN_FRAC) -> seed viable, passes
                            // through eater + planted nearby (endozoochory). UNRIPE -> seed undeveloped, nothing
                            // planted (eating early = no reproduction).
                            let f = rot_age.map(|a| a as f32 / ROT_GONE as f32).unwrap_or(0.0);
                            if f >= RIPEN_FRAC && foods.len() < PLANT_CAP && rng.f32() < pg.quality * SEED_VIA_GUT {
                                let mut child = seed_g.clone();
                                child.mutate(&mut rng);
                                let sp = disperse_pos(&mut rng, np, seed_g.spread, FOOD_Y); // carried + dropped nearby
                                spawn_plant(&mut commands, child, PLANT_START_MASS, sp);
                            }
                            commands.entity(e).despawn();
                        } else if rot_age.is_some() {
                            commands.entity(e).despawn(); // carrion / detritus consumed whole
                        } else if foods.len() < PLANT_CAP && rng.f32() < pg.quality * SEED_VIA_GUT {
                            // endozoochory (spec 13): grazing a living plant may disperse a mutated offspring
                            let mut child = pg.clone();
                            child.mutate(&mut rng);
                            let sp = disperse_pos(&mut rng, np, pg.spread, FOOD_Y); // seed carried + dropped nearby
                            spawn_plant(&mut commands, child, PLANT_START_MASS, sp);
                        }
                    }
                }
                // failed bite: the plant's defense held; it survives this contact
            }
        }

        // passive grass+seaweed graze: a thin FALLBACK. Grass/kelp render-only (not in food scan), edibility
        // modeled by POSITION: a HUNGRY creature on grass-bearing soil (plant-capable non-ocean land) or over the
        // kelp band nibbles a sugar trickle. eat-gated (out[4]) + hunger-gated (stops at GRAZE_FULL -> no gorging);
        // never distracts foraging (carpet isn't sensed). A HERBIVORE-tuned gut can live off the carpet ALONE (cow
        // grazes grass, aquatic grazer grazes kelp), but it's a SPECIALTY: value scales by herbivory (1-carnivory)
        // so a carnivore gets ~nothing. Whether grass alone NET-survives depends on upkeep (cheap frugal low-metab
        // grazers thrive; big costly bodies need richer food) -> emergent carrying cap.
        if out[4] > EAT_GATE && energy.total() < GRAZE_FULL {
            let gdir = np.normalize_or_zero();
            let herbivory = 1.0 - genome.carnivory; // herbivore gut digests the carpet; a carnivore can't
            let hab = crate::sphere::plant_habitability(gdir);
            if !crate::sphere::is_ocean(gdir) && hab > GRASS_HAB_MIN {
                let gain = GRASS_GRAZE * hab * herbivory * genome.uptake[GRASS_FORAGE_IDX] * dt; // grass: grazer staple where it's grassy
                energy.add_sugar(gain, SUGAR_CAP, fat_max);
                // refill grass FORAGE nutrient x gut tuning: grazer with uptake on grass axis stays fed; mismatched
                // gut gets energy but still starves of deficiency (no free lunch).
                let r = &mut diet.reserves[GRASS_FORAGE_IDX];
                *r = (*r + GRAZE_NUTRIENT * genome.uptake[GRASS_FORAGE_IDX] * herbivory * hab * dt).min(RESERVE_CAP);
                fit.0 += gain * 0.1;
            }
            // seaweed: OCEAN twin. Submerged grazer over the kelp band (ocean, above abyssal floor) grazes kelp.
            // Non-swimmers reach only shallow kelp (deep water drowns them) -> deep carpet is a swimmer-grazer's
            // larder. Grazeable across the WHOLE ocean (as grass covers all land) so a swimmer grazer is fed
            // anywhere, not just over the visible kelp shelf. Deeper water yields more (more submerged biomass).
            if crate::sphere::is_ocean(gdir) {
                let e01 = crate::sphere::elevation01(gdir);
                let depth = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
                let band = 0.5 + 0.5 * depth; // richer the deeper/more-submerged
                let gain = SEAWEED_GRAZE * band * herbivory * genome.uptake[SEAWEED_FORAGE_IDX] * dt;
                energy.add_sugar(gain, SUGAR_CAP, fat_max);
                let r = &mut diet.reserves[SEAWEED_FORAGE_IDX]; // kelp forage nutrient, gut-matched like grass above
                *r = (*r + GRAZE_NUTRIENT * genome.uptake[SEAWEED_FORAGE_IDX] * herbivory * band * dt).min(RESERVE_CAP);
                fit.0 += gain * 0.1;
            }
        }

        // per-tick upkeep. Age every creature; diet mode runs nutrient metabolism: gut-upkeep cost, reserve
        // depletion (burn the nutrients you're built to use), deficiency -> growth-load.
        diet.age += 1;
        if gen.diet {
            let total_uptake: f32 = genome.uptake.iter().sum();
            energy.burn(UPTAKE_OVERHEAD * total_uptake * dt); // broad gut costs upkeep (generalist tax)
            // deplete reserves by use (x uptake demand) + measure demand-weighted deficiency
            let mut wsum = 0.0;
            let mut deficit = 0.0;
            for i in 0..NUTRIENTS {
                diet.reserves[i] = (diet.reserves[i] - NUTRIENT_USE * genome.uptake[i] * dt).max(0.0);
                let demand = genome.uptake[i];
                wsum += demand;
                deficit += demand * (1.0 - (diet.reserves[i] / RESERVE_REQ).min(1.0)); // shortfall on a demanded nutrient
            }
            if wsum > 1e-3 {
                diet.g += DEFICIT_G * (deficit / wsum) * dt; // soft-gradient: deficiency raises disease load
            }
            diet.g = (diet.g - G_DECAY).max(0.0);
        }
        // toxic load: clear slowly each tick (faster with detox gene). While loaded drains energy + raises disease
        // load; acute death hazard added in the mortality block. Runs in ALL modes so meat/ferment poisons bite
        // even in legacy --no-diet runs.
        if diet.toxic_load > 0.0 {
            let clear = (TOX_CLEAR_BASE + TOX_CLEAR_DETOX * genome.detox) * dt;
            diet.toxic_load = (diet.toxic_load - clear).max(0.0);
            energy.burn(TOX_LOAD_DRAIN * diet.toxic_load * dt);
            diet.g += TOX_LOAD_G * diet.toxic_load * dt;
        }
        // mortality from diet model (aging + disease). In continuous mode death is otherwise starvation-driven
        // (density-dependent), which regulates population logistically.
        if gen.diet {
            // senescence (both modes): old-age hazard rises with age toward a ceiling -> real finite lifespan, not
            // immortal-if-fed. Ages staggered (warmup desync + spread-out births) so this does NOT sync-kill a
            // cohort. Turnover keeps gene pool flowing (old die, young replace) -> a true life cycle.
            let age_frac = diet.age as f32 / (AGE_SCALE * lifespan_mult); // longevity gene stretches lifespan
            let aging = AGE_HAZARD * (age_frac / (age_frac + 1.0));
            let p_death = (aging + DISEASE_K * diet.g + TOX_LOAD_HAZARD * diet.toxic_load) * dt;
            if rng.f32() < p_death {
                alive.0 = false; // old-age / disease / poisoning death
            }
        }

        // learn: reward = approach shaping (base mode only) + eat signal + combat outcome. Tunes brain (spec 04/09).
        // Poison/diet drop approach shaping (it rewards nearing ANY food, fighting selectivity). fight_reward set
        // last tick by predation_step (kill/braced-survival/whiff) -> reinforces attack/defend outputs. Cleared
        // every tick below (even when not learning) so it can't accumulate unbounded.
        if gen.learn {
            let approach = if !gen.diet && brain.prev_dist.is_finite() && cur_dist.is_finite() {
                (brain.prev_dist - cur_dist).clamp(-1.0, 1.0) * R_APPROACH
            } else {
                0.0
            };
            let reward = approach + eat_reward + brain.fight_reward;
            learn(&mut brain.net, &genome.plast, &input, &h, &out, reward, LEARN_RATE);
        }
        brain.fight_reward = 0.0; // consumed (or discarded when not learning): never accumulate across ticks
        brain.prev_dist = cur_dist;

        // starvation death. Outright empty -> dead. Else a creature pinned BELOW STARVE_FLOOR (clinging near-zero,
        // e.g. grass-trickle zombie that can't pay real upkeep) dies after STARVE_TICKS grace; a forager that
        // briefly dips low between meals climbs back above the floor + resets.
        if energy.total() <= 0.0 {
            alive.0 = false;
        } else if energy.total() < STARVE_FLOOR {
            diet.starve = diet.starve.saturating_add(1);
            if diet.starve >= STARVE_TICKS {
                alive.0 = false;
            }
        } else {
            diet.starve = 0;
        }

        // continuous reproduction: a well-fed creature spends energy to bud a mutated child nearby. Energy cost =
        // trade-off (breeding vs survival); cap bounds population. r/K life-history (parental gene, 0.5 = neutral):
        // r = breed young/cheap/many fragile young; K = breed late/costly/few well-provisioned young. Density taper
        // bounds population.
        let k = genome.parental;
        let repro_thr = REPRO_THRESHOLD * (0.8 + 0.4 * k);
        let repro_min_age = (REPRO_MIN_AGE as f32 * (0.6 + 0.8 * k)) as u32;
        let ni = crate::niche::niche_of(genome).idx(); // breeder's niche -> its OWN carrying cap governs repro
        let ncap = NICHE_CAP[ni].max(1) as f32;
        if live_continuous
            && alive.0
            && energy.total() > repro_thr
            && diet.age > repro_min_age // newborns must establish before breeding (paces birth waves)
            && pop < CREATURE_CAP // global hard ceiling (loose backstop)
            && (niche_pop[ni] as f32) < ncap
            // density-dependent on the breeder's OWN niche: rate tapers to 0 as that niche approaches ITS cap ->
            // each habitat asymptotes independently (no winner-take-all, no boom-bust overshoot).
            && rng.f32() < P_REPRO_CREATURE * (1.0 - niche_pop[ni] as f32 / ncap)
        {
            energy.burn(REPRO_COST * (0.7 + 0.6 * k)); // K-parents spend more per child
            // mating mode: cross with nearest genetically-similar mate (assortative -> reproductive
            // isolation/speciation); else single-parent budding if no compatible mate nearby.
            let mut child = if gen.mating {
                let my_sig = signature(genome);
                let r2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
                let mate = mate_pool
                    .iter()
                    .filter(|(e, p, s, _)| *e != entity && ct.translation.distance_squared(*p) < r2 && sig_dist(&my_sig, s) < SOCIAL_SIM)
                    .min_by(|a, b| {
                        ct.translation.distance_squared(a.1).partial_cmp(&ct.translation.distance_squared(b.1)).unwrap()
                    });
                match mate {
                    Some((_, _, _, mg)) => Genome::crossover(genome, mg, &mut rng),
                    None => genome.clone(),
                }
            } else {
                genome.clone()
            };
            child.mutate(&mut rng, MUT_RATE, MUT_STD);
            let cp = disperse_pos(&mut rng, ct.translation, 2.0, CREATURE_Y); // child appears beside the parent
            let birth_e = BIRTH_ENERGY * (0.7 + 0.6 * k); // K-young start better-provisioned (survive); r-young cheap + fragile
            spawn_creature(&mut commands, child, cp, &mut rng, birth_e);
            pop += 1;
            niche_pop[ni] += 1; // child counts toward parent's niche (so within-tick births keep tapering)
        }

        // died this tick (loop skips already-dead at top) -> drop carrion here, rots into poison (rot_step).
        // Closes part of the nutrient loop: death feeds the food web (P3).
        if !alive.0 {
            let fat = (energy.fat / fat_max.max(0.01)).clamp(0.0, 1.0); // fattiness at death
            spawn_carrion(&mut commands, ct.translation, CARRION_MASS, fat);
            soil.add(ct.translation, DEATH_FERT); // death enriches ground here
            // continuous (post-warmup): corpse entity gone (became carrion). Generational mode + warm-up keep it
            // (Alive=false) to recycle into next generation.
            if live_continuous {
                commands.entity(entity).despawn();
                pop = pop.saturating_sub(1);
            }
        }
    }
    // Reseed floor now PER-NICHE (niche::niche_step, runs after this): each habitat (aquatic/aerial/highland/
    // cold/warm/land) has its own floor + hall-of-fame bank, so a collapsing niche revives from ITS OWN evolved
    // genomes instead of being refilled by whatever survivor dominates (old global floor let specialist niches
    // go quietly extinct). Banks persist -> even total extinction recovers. Scenario mode disables it there.
    // eaten plants despawned above; population replenished by plant_step (reproduction).
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

// Per-creature FAT-store ceiling: adiposity sets storage strategy, body size adds capacity. Fat is the big bank
// (fast/sugar caps fixed small/medium). Bigger+fattier bodies buffer famine but pay carrying upkeep (FAT_UPKEEP)
// + sluggishness (FAT_POWER) -> no free lunch.
fn fat_cap(g: &Genome) -> f32 {
    FAT_CAP * (0.4 + ADIPOSITY_CAP * g.adiposity) * (1.0 + SIZE_ENERGY * g.size)
}

// Collision body radius (~visual half-width): build factor (sensor count + size) x COLLIDE_R. Matches rendered
// girth so bodies stop just shy of visible overlap. Shared by snapshot + move loop.
fn body_radius(g: &Genome) -> f32 {
    (0.7 + 0.06 * g.n_sensors() as f32) * (0.6 + 0.9 * g.size) * crate::config::COLLIDE_R
}

// Compact phenotype signature for KIN similarity (diet + body traits). Kin when signatures within SOCIAL_SIM
// -> drives flocking-by-species + the social need.
fn signature(g: &Genome) -> [f32; 10] {
    // diet + body identity; carnivory + pelt added (M4) so gut-axis + coat divergence speciate too. Both default
    // identical across an old seed (serde default) -> add 0 distance there (no change to existing pop, only
    // newly-evolved divergence).
    [g.uptake[0], g.uptake[1], g.uptake[2], g.uptake[3], g.size, g.swim, g.light_pref, g.height, g.carnivory, g.pelt]
}
fn sig_dist(a: &[f32; 10], b: &[f32; 10]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum()
}

// Fraction of social satisfaction from nearby kin (0 isolated .. 1 fully in herd). Excludes self by entity id.
fn kin_fraction(me: Entity, pos: Vec3, sig: &[f32; 10], snap: &[(Entity, Vec3, [f32; 10], f32, f32)]) -> f32 {
    let r2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
    let mut kin = 0.0f32;
    for (e, p, s, _, _) in snap {
        if *e == me {
            continue;
        }
        if pos.distance_squared(*p) < r2 && sig_dist(sig, s) < SOCIAL_SIM {
            kin += 1.0;
        }
    }
    (kin / SOCIAL_TARGET).min(1.0)
}

// generation boundary: select + reproduce.
pub fn generation_step(
    mut gen: ResMut<GenState>,
    mut rng: ResMut<Rng>,
    mut cq: Query<
        (&mut Transform, &mut Energy, &mut Fitness, &mut Heading, &mut Alive, &mut Genome, &mut Brain, &mut DietState, &mut Locomotion),
        With<Creature>,
    >,
    pq: Query<(&PlantGenome, &PlantState), (Without<Rot>, Without<Grass>)>, // living plants only (carrion + grass excluded from stats/save)
    tq: Query<&PlantGenome, With<Tree>>, // trees only, for the evolvable-height stat
    soil: Res<Soil>,
    gw: Res<GroundWater>,
    climate: Res<Climate>,
    fire: Res<Fire>,
    weather: Res<Weather>,
    mut exit: MessageWriter<AppExit>,
    niche: Res<crate::niche::NicheTracker>, // --until-sustain stop: all niches quiet for a full window
    // Best healthy snapshot seen this run (score, snapshot). --save writes THIS, not the final-tick population:
    // continuous pop oscillates (~13..72), so an arbitrary end-tick can capture a trough -> near-empty seed that
    // limps back up. Saving the peak gives a full balanced living world.
    mut best: Local<Option<(f32, crate::persist::Snapshot)>>,
) {
    let _g = crate::profile::scope("generation");
    gen.tick = gen.tick.wrapping_add(1); // global clock: drives season (plant_step) + continuous timing

    // continuous mode (after warm-up): no generation boundary. Snapshot the ecosystem periodically; stop headless
    // at max_gens*GEN_TICKS or on extinction. Selection emergent (live_step).
    if gen.continuous && gen.generation >= WARMUP_GENS {
        let pop = cq.iter().count();
        // --until-sustain: stop early once all niches hold themselves up (no rescue for a full window). Else stop
        // at --gens or extinction. max_gens still caps --until-sustain (safety: if a niche never self-sustains).
        let sustained = gen.until_sustain && niche.self_sustaining(gen.tick);
        let done = gen.headless && (gen.tick >= gen.max_gens * GEN_TICKS || pop == 0 || sustained);
        if sustained {
            info!("SELF-SUSTAINING at tick {} (pop {}): no niche rescue for {} ticks | total rescues/niche {:?}", gen.tick, pop, NICHE_SUSTAIN_WINDOW, niche.total_rescues);
        }
        if gen.tick.is_multiple_of(CONT_LOG_TICKS) || done {
            let n = pop.max(1) as f32;
            let mut e = 0.0;
            let mut f = 0.0;
            let mut fa = 0.0; // mean fast store (fed only by fermented food -> >0 proves fermentation web works)
            let mut su = 0.0; // mean sugar store
            let mut ft = 0.0; // mean fat store
            let mut adp = 0.0; // mean adiposity gene
            let mut mast = 0.0; // mean master digestion expression (nutrient sufficiency 0..1)
            let mut brd = 0.0; // mean diet breadth = count actively-absorbed nutrients (uptake > 0.4)
            let mut sens = 0.0;
            let mut bite = 0.0;
            let mut rig = 0.0;
            let mut age = 0.0;
            let mut temp = 0.0;
            let mut lng = 0.0;
            let mut met = 0.0;
            let mut par = 0.0;
            let mut alp = 0.0; // mean alpine (mountain adaptation)
            let mut sw = 0.0; // mean swim (aquatic adaptation)
            let mut aq = 0u32; // count of aquatic creatures (swim > 0.6) -> fish niche size
            let mut hi = 0u32; // count of highland creatures (alpine > 0.5) -> mountain niche size
            let mut abslat = 0.0; // mean |latitude| of the population (0 equator .. ~1.57 pole) -> spread check
            for (t, en, fit, _h, _a, g, _b, diet, _l) in cq.iter() {
                e += en.total();
                fa += en.fast;
                su += en.sugar;
                ft += en.fat;
                adp += g.adiposity;
                mast += master_expression(&g.uptake, &diet.reserves, RESERVE_REQ, MASTER_FLOOR);
                brd += g.uptake.iter().filter(|u| **u > 0.4).count() as f32;
                f += fit.0;
                sens += g.n_sensors() as f32;
                bite += g.bite;
                rig += g.rigidity;
                age += diet.age as f32;
                temp += g.temp_pref;
                lng += g.longevity;
                met += g.metab;
                par += g.parental;
                alp += g.alpine;
                sw += g.swim;
                if g.swim > 0.6 { aq += 1; }
                if g.alpine > 0.5 { hi += 1; }
                abslat += crate::sphere::dir_to_lonlat(t.translation.normalize_or_zero()).1.abs();
            }
            let plant_n = pq.iter().len().max(1);
            let avg_def: f32 = pq.iter().map(|(g, _)| g.defense).sum::<f32>() / plant_n as f32;
            let avg_nut: f32 = pq.iter().map(|(g, _)| g.nutrient).sum::<f32>() / plant_n as f32;
            let avg_qual: f32 = pq.iter().map(|(g, _)| g.quality).sum::<f32>() / plant_n as f32;
            let avg_wet: f32 = pq.iter().map(|(g, _)| g.wet).sum::<f32>() / plant_n as f32;
            info!(
                "t {:>6} | pop {:>3} | energy {:.1} [f{:.1}/s{:.1}/F{:.1}] adp {:.2} | mast {:.2} brd {:.1} | life-fit {:.1} | age {:.0} | sens {:.1} | bite {:.2} | rig {:.2} | temp {:.2} lng {:.2} met {:.2} par {:.2} lat {:.2} | swim {:.2} alp {:.2} aq {} hi {} | def {:.2} nut {:.2} qual {:.2} wet {:.2} | plants {} | soil {:.2} | rain {:.2} fire {:.3}",
                gen.tick, pop, e / n, fa / n, su / n, ft / n, adp / n, mast / n, brd / n, f / n, age / n, sens / n, bite / n, rig / n, temp / n, lng / n, met / n, par / n, abslat / n, sw / n, alp / n, aq, hi, avg_def, avg_nut, avg_qual, avg_wet, plant_n, soil.avg(), weather.rain, fire.avg()
            );
            // Track best healthy snapshot for --save. Score = pop, gated on well-fed (avg energy >= 30) so we never
            // bank a starving crowd. Captured only when saving (snapshot clone not free).
            if gen.save.is_some() {
                let avg_e = e / n;
                let score = if avg_e >= 30.0 { pop as f32 } else { 0.0 };
                if score > 0.0 && best.as_ref().is_none_or(|(s, _)| score > *s) {
                    let mut creatures: Vec<(f32, Genome)> =
                        cq.iter().map(|(_, _, fit, _, _, g, _, _, _)| (fit.0, g.clone())).collect();
                    creatures.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                    let plants: Vec<crate::persist::SavedPlant> = pq
                        .iter()
                        .map(|(g, st)| crate::persist::SavedPlant { g: g.clone(), mass: st.mass })
                        .collect();
                    *best = Some((
                        score,
                        crate::persist::Snapshot {
                            generation: gen.tick / GEN_TICKS,
                            creatures: creatures.into_iter().map(|(_, g)| g).collect(),
                            plants,
                        },
                    ));
                }
            }
        }
        if done {
            if let Some(path) = &gen.save {
                // Prefer best healthy snapshot seen; else final state if none qualified.
                if let Some((score, snap)) = best.take() {
                    crate::persist::save_snapshot(path, &snap);
                    info!("saved best snapshot: pop {} (score {:.0})", snap.creatures.len(), score);
                } else {
                    let mut creatures: Vec<(f32, Genome)> =
                        cq.iter().map(|(_, _, fit, _, _, g, _, _, _)| (fit.0, g.clone())).collect();
                    creatures.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                    let plants: Vec<crate::persist::SavedPlant> = pq
                        .iter()
                        .map(|(g, st)| crate::persist::SavedPlant { g: g.clone(), mass: st.mass })
                        .collect();
                    crate::persist::save_snapshot(
                        path,
                        &crate::persist::Snapshot {
                            generation: gen.tick / GEN_TICKS,
                            creatures: creatures.into_iter().map(|(_, g)| g).collect(),
                            plants,
                        },
                    );
                }
            }
            if let Some(mpath) = &gen.metrics {
                let avg_e = if pop > 0 { cq.iter().map(|(_, en, ..)| en.total()).sum::<f32>() / pop as f32 } else { 0.0 };
                crate::niche::write_metrics(mpath, sustained, gen.tick, pop, avg_e, &niche);
            }
            info!("continuous headless done at tick {} (pop {})", gen.tick, pop);
            exit.write(AppExit::Success);
        }
        return;
    }

    gen.ticks_left = gen.ticks_left.saturating_sub(1);
    let any_alive = cq.iter().any(|(.., alive, _, _, _, _)| alive.0);
    if gen.ticks_left > 0 && any_alive {
        return;
    }

    // rank by fitness
    let mut scored: Vec<(f32, Genome)> =
        cq.iter().map(|(_, _, fit, _, _, g, _, _, _)| (fit.0, g.clone())).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    // capture fitness-ranked survivors for --save (only when saving)
    let ranked: Vec<Genome> = if gen.save.is_some() {
        scored.iter().map(|(_, g)| g.clone()).collect()
    } else {
        Vec::new()
    };

    // roam ratio = net displacement / path walked, averaged. ~1 = straight rovers, ~0 = circlers.
    let mut roam_sum = 0.0f32;
    let mut roam_n = 0u32;
    for it in cq.iter() {
        if it.8.path > 0.5 {
            roam_sum += (it.0.translation - it.8.start).length() / it.8.path;
            roam_n += 1;
        }
    }
    let avg_roam = if roam_n > 0 { roam_sum / roam_n as f32 } else { 0.0 };

    // avg terrain elevation creatures occupy at gen end (P3 diagnostic): spreading over relief?
    let mut elev_sum = 0.0f32;
    let mut elev_n = 0u32;
    for it in cq.iter() {
        elev_sum += it.0.translation.y - CREATURE_Y;
        elev_n += 1;
    }
    let avg_elev = if elev_n > 0 { elev_sum / elev_n as f32 } else { 0.0 };

    let n = scored.len().max(1);
    let avg: f32 = scored.iter().map(|(f, _)| f).sum::<f32>() / n as f32;
    let avg_sensors: f32 = scored.iter().map(|(_, g)| g.n_sensors() as f32).sum::<f32>() / n as f32;
    let avg_range: f32 = scored
        .iter()
        .map(|(_, g)| g.sensors.iter().map(|s| s.range).sum::<f32>() / g.sensors.len().max(1) as f32)
        .sum::<f32>()
        / n as f32;
    let avg_bite: f32 = scored.iter().map(|(_, g)| g.bite).sum::<f32>() / n as f32;
    let avg_light: f32 = scored.iter().map(|(_, g)| g.light_pref).sum::<f32>() / n as f32; // niche: <0.5 nocturnal, >0.5 diurnal
    let avg_size: f32 = scored.iter().map(|(_, g)| g.size).sum::<f32>() / n as f32;
    let avg_swim: f32 = scored.iter().map(|(_, g)| g.swim).sum::<f32>() / n as f32; // >0.5 => aquatic niche emerging
    let avg_social: f32 = scored.iter().map(|(_, g)| g.social).sum::<f32>() / n as f32; // herd instinct
    let avg_hidden: f32 = scored.iter().map(|(_, g)| g.net.ih.len() as f32).sum::<f32>() / n as f32; // brain size
    let tree_n = tq.iter().len();
    let avg_tree_h = if tree_n > 0 { tq.iter().map(|g| g.height).sum::<f32>() / tree_n as f32 } else { 0.0 }; // tree reach gene
    let avg_tree_b = if tree_n > 0 { tq.iter().map(|g| g.branches).sum::<f32>() / tree_n as f32 } else { 0.0 }; // tree branches gene
    // plant (food) co-evolution stats: arms race = bite vs defense
    let plant_n = pq.iter().len().max(1); // .max(1): avoid div-by-zero in means below
    let avg_def: f32 = pq.iter().map(|(g, _)| g.defense).sum::<f32>() / plant_n as f32;
    let avg_nut: f32 = pq.iter().map(|(g, _)| g.nutrient).sum::<f32>() / plant_n as f32;
    let avg_qual: f32 = pq.iter().map(|(g, _)| g.quality).sum::<f32>() / plant_n as f32;
    let avg_wet: f32 = pq.iter().map(|(g, _)| g.wet).sum::<f32>() / plant_n as f32;
    if gen.diet {
        let avg_rig: f32 = scored.iter().map(|(_, g)| g.rigidity).sum::<f32>() / n as f32;
        info!("gen {:>3} | nutri {:>6.2} | sens {:.1} r{:.0} | rig {:.2} | bite {:.2} vs def {:.2} | light {:.2} sz {:.2} sw {:.2} so {:.2} brain {:.1} | plant-nut {:.2} qual {:.2} wet {:.2} | roam {:.2} elev {:.1} | plants {} soil {:.2} gw {:.2} clim {:.2}[{:.2}-{:.2}] desert {:.0}% fire {:.3} | trees {} h{:.2} b{:.2}", gen.generation, avg, avg_sensors, avg_range, avg_rig, avg_bite, avg_def, avg_light, avg_size, avg_swim, avg_social, avg_hidden, avg_nut, avg_qual, avg_wet, avg_roam, avg_elev, plant_n, soil.avg(), gw.avg(), climate.avg(), climate.range().0, climate.range().1, climate.land_arid_frac(0.25) * 100.0, fire.avg(), tree_n, avg_tree_h, avg_tree_b);
    } else {
        info!("gen {:>3} | food {:>6.2} | sens {:.1} r{:.0} | bite {:.2} vs def {:.2} | plant-nut {:.2} qual {:.2} wet {:.2} | roam {:.2} elev {:.1} | plants {} soil {:.2} gw {:.2}", gen.generation, avg, avg_sensors, avg_range, avg_bite, avg_def, avg_nut, avg_qual, avg_wet, avg_roam, avg_elev, plant_n, soil.avg(), gw.avg());
    }

    // elite pool (clone+mutate, asexual)
    let elite_count = ((n as f32 * ELITE_FRAC).ceil() as usize).max(1);
    let elites: Vec<Genome> = scored.into_iter().take(elite_count).map(|(_, g)| g).collect();

    // build next-gen genomes
    let mut next: Vec<Genome> = Vec::with_capacity(n);
    for _ in 0..n {
        let parent = &elites[(rng.f32() * elite_count as f32) as usize % elite_count];
        let mut child = parent.clone();
        child.mutate(&mut rng, MUT_RATE, MUT_STD);
        next.push(child);
    }

    // overwrite creatures with next gen, reset state + reposition. Brain relearns from scratch each life: w :=
    // child priors (learned weights NOT inherited). Diet expression resets to child innate baseline (epigenetic
    // state not inherited). The cohort about to ENTER continuous mode (last warm-up reset) is DESYNCHRONIZED:
    // random starting energy + life-age so they don't all hit breed threshold / min-age / death on the same tick.
    // Synchronized cohorts breed in one burst -> newborn wave starves together -> boom-bust extinction. Staggering
    // spreads births + deaths so the population can overlap.
    let desync = gen.continuous && gen.generation + 1 >= WARMUP_GENS;
    for ((mut t, mut energy, mut fit, mut head, mut alive, mut g, mut brain, mut diet, mut loco), child) in
        cq.iter_mut().zip(next)
    {
        brain.net = child.net.clone();
        brain.prev_dist = f32::INFINITY;
        brain.attack = 0.0; // clear stale combat intent/reward so a reused entity starts fresh
        brain.defend = 0.0;
        brain.fight_reward = 0.0;
        diet.reserves = [RESERVE_REQ; NUTRIENTS]; // fresh life starts with stocked reserves
        diet.g = 0.0;
        diet.age = if desync { (rng.f32() * 600.0) as u32 } else { 0 };
        diet.fatigue = 0.0;
        *g = child;
        *energy = Energy::from_total(if desync { rng.range(0.8, 1.2) * START_ENERGY } else { START_ENERGY }); // stagger but never lethally low
        fit.0 = 0.0;
        head.0 = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        alive.0 = true;
        let p = rand_pos(&mut rng, CREATURE_Y);
        t.translation = p;
        t.rotation = Quat::from_rotation_y(head.0);
        loco.start = p;
        loco.path = 0.0;
    }

    gen.generation += 1;
    gen.ticks_left = GEN_TICKS;
    // Plants NOT reset at the creature-generation boundary: they live + evolve continuously (own GA in
    // plant_step), so food supply co-evolves across creature generations.

    if gen.headless && gen.generation >= gen.max_gens {
        // --save: persist fitness-ranked survivors + current food web so the run can resume.
        if let Some(path) = &gen.save {
            let plants: Vec<crate::persist::SavedPlant> = pq
                .iter()
                .map(|(g, st)| crate::persist::SavedPlant { g: g.clone(), mass: st.mass })
                .collect();
            crate::persist::save_snapshot(
                path,
                &crate::persist::Snapshot { generation: gen.generation, creatures: ranked, plants },
            );
        }
        info!("headless run done after {} generations", gen.generation);
        exit.write(AppExit::Success);
    }
}

fn wrap_angle(a: f32) -> f32 {
    let mut a = a;
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}
