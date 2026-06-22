// M1 foraging sim: creatures sense nearest food, a tiny NN drives thrust+turn, they eat,
// burn energy, starve; a generational GA selects by food eaten. Proof-of-life milestone (08).
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::components::{Alive, Brain, Creature, DietState, Energy, Ferment, Fitness, Food, Grass, Heading, Locomotion, Rot, Seed, Tree};
use crate::genome::{forward, learn, master_expression, Genome, CONE_HALF, GLOBAL_INPUTS, NFOOD, NUTRIENTS, SIG_PER_SENSOR};
use crate::plant::{PlantGenome, PlantState, P_REPRO, PLANT_CAP, PLANT_MIN};
use crate::rng::Rng;

// Tuning constants live in config.rs; re-exported so existing `sim::FOO` refs still resolve.
pub use crate::config::*;

// (Day/night is POSITIONAL on the planet: sphere::daylight_at per creature/plant. No global daylight.)

// Map a 3D sphere position to pseudo-planar grid coords in [-WORLD_HALF, WORLD_HALF] (longitude -> u,
// latitude -> v) so the existing 2D fertility/water/fire/food grids can index the globe by lon/lat.
fn grid_uv(pos: Vec3) -> (f32, f32) {
    let (lon, lat) = crate::sphere::dir_to_lonlat(pos.normalize_or_zero());
    (lon / std::f32::consts::PI * WORLD_HALF, lat / std::f32::consts::FRAC_PI_2 * WORLD_HALF)
}

#[derive(Resource)]
pub struct GenState {
    pub generation: u32,
    pub ticks_left: u32,
    pub headless: bool,
    pub learn: bool,     // lifetime learning on/off (A/B vs M1 baseline)
    pub poison: bool,    // two food types (legacy --poison mode; sets ntypes=2)
    pub diet: bool,      // epigenetic diet model (NFOOD types, expression, growth-load, disease)
    pub continuous: bool, // continuous reproduction (--continuous) vs discrete generational GA (default)
    pub tick: u32,       // global tick clock (drives season + continuous logging/stop)
    pub max_gens: u32,   // headless run length in generations (--gens=N); continuous uses N*GEN_TICKS
    pub save: Option<String>, // --save=PATH: write survivors at headless run end (BACKLOG P2)
    pub load: Option<String>, // --load=PATH: resume from a saved population instead of random
    pub diverse: bool,        // --diverse: hand-seed niche-adapted creatures across the globe (multi-niche showcase)
    pub mating: bool,         // --mating: offspring = crossover of two nearby genetically-similar parents (assortative -> speciation); else single-parent budding
    pub garden: bool,         // --garden: seed a botanical SHOWCASE (one of every plant species in a grid at the homeland, + a fruit tree, evergreen, vine tree) instead of the normal random world. For inspecting the flora.
    pub plant_lib: Option<String>, // path to a tuned plant seed-bank (tuning harness); when present, the planet is seeded biome-matched FROM it (else archetypes). None = --no-plant-lib (today's archetype seeding).
}

impl GenState {
    // Number of distinct food types in play this mode.
    pub fn ntypes(&self) -> u8 {
        if self.diet { NFOOD as u8 } else if self.poison { 2 } else { 1 }
    }
}

// Dynamic soil-fertility grid (M5 closed loop): decomposing corpses deposit here; plants read it for a
// growth boost; it decays each tick. Coarse grid over the arena so deposits are spatially local.
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

// Mass stripped from each fruit tree this tick (live_step records, plant_step applies). Trees persist
// and regrow; a tree only dies when grazed below TREE_MIN_MASS (over-eaten).
#[derive(Resource, Default)]
pub struct TreeBites(pub HashMap<Entity, f32>);

// Current rainfall intensity 0..1 (0 dry, 1 heavy downpour). Storms onset stochastically + decay.
#[derive(Resource, Default)]
pub struct Weather {
    pub rain: f32,
}

// Seed bank (dormancy gene): seeds buried in the soil that germinate LATER, not now. Each entry counts down
// `ticks`; at 0 plant_step spawns it. The bank lives independent of surface plants, so a patch wiped by
// fire/drought/grazing re-greens from its buried seeds (boom after disturbance). Capped at SEED_BANK_CAP.
#[derive(Resource, Default)]
pub struct SeedBank(pub Vec<(PlantGenome, Vec3, u32)>); // (genome, surface pos, ticks until germination)

// Dynamic ground-water grid (rain cycle): per-cell wetness 0..1 layered over static terrain moisture.
// Sun evaporates it, rain refills it (rocky cells shed runoff). Coarse grid so wetness is spatially local.
#[derive(Resource)]
pub struct GroundWater {
    pub cell: Vec<f32>,
}

// Surface position of a grid cell's center (inverse of Soil::index): lets weather sample terrain per cell.
fn cell_center(c: usize) -> Vec3 {
    let (cx, cz) = (c % SOIL_RES, c / SOIL_RES);
    let to_uv = |k: usize| ((k as f32 + 0.5) / SOIL_RES as f32) * 2.0 * WORLD_HALF - WORLD_HALF;
    let (u, v) = (to_uv(cx), to_uv(cz));
    let lon = u / WORLD_HALF * std::f32::consts::PI;
    let lat = v / WORLD_HALF * std::f32::consts::FRAC_PI_2;
    crate::sphere::lonlat_to_pos(lon, lat, 0.0) // dir * PLANET_R; callers sample fields by direction
}

// Surface position (on the terrain) of grid cell `c`. For the render to place fire/effects on the globe.
pub fn grid_cell_surface(c: usize) -> Vec3 {
    crate::sphere::surface_pos(cell_center(c).normalize_or_zero(), 0.0)
}

// Food spatial grid (perf): bin foods into FGRID^2 cells so a creature scans only nearby cells instead
// of all ~1900 foods. NEAR_QUERY is the min query radius so the global-nearest is always found (plants
// are dense -> nearest is within a few units, well inside this).
const FGRID: usize = 20;
const NEAR_QUERY: f32 = 24.0;
fn fcell(w: f32) -> usize {
    (((w + WORLD_HALF) / (2.0 * WORLD_HALF)) * FGRID as f32).clamp(0.0, (FGRID - 1) as f32) as usize
}
// Food-grid cell (lon, lat bins) for a 3D sphere position.
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

// Slow climate-memory grid (geological): per-cell long-term moisture 0..1. GroundWater is fast (wets on
// rain, dries in hours); Climate is slow -> it low-pass-filters the drifting rain-propensity target over
// MONTHS, so persistently-dry regions drift to desert + persistently-wet regions to lush, and the wet belt
// migrates over years. Drives plant growth/mortality (sim) + globe recolor (render). Same SOIL_RES grid.
#[derive(Resource)]
pub struct Climate {
    pub cell: Vec<f32>,
}

impl Climate {
    pub fn new() -> Self {
        // seed each cell at its STATIC moisture baseline so the world starts exactly as it looks today
        // (no cold-start shock); climate then diverges from here as the rain-propensity anomaly drifts.
        let cell = (0..SOIL_RES * SOIL_RES)
            .map(|c| crate::sphere::moisture(cell_center(c).normalize_or_zero()))
            .collect();
        Climate { cell }
    }
    pub fn get(&self, pos: Vec3) -> f32 {
        self.cell[GroundWater::index(pos)]
    }
    // Bilinear-sampled climate moisture at direction `d` (longitude wraps, latitude clamps at the poles).
    // Smooths the coarse 32x32 grid -> the globe recolor shows soft biome edges, not blocky cells.
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
    // Driest/wettest cell (0..1) -- for headless logging the desert/forest spread.
    pub fn range(&self) -> (f32, f32) {
        let mut lo = 1.0f32;
        let mut hi = 0.0f32;
        for &v in &self.cell {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo, hi)
    }
    // Fraction of LAND cells that are arid (climate moisture below `thr`) -> desert coverage. Watching this
    // climb/fall over a long run shows deserts forming + rainforests greening as the climate drifts.
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

// --- weather (rain cycle): advance rainfall + update the ground-water grid (sun dries, rain refills) ---
pub fn weather_step(
    gen: Res<GenState>,
    mut weather: ResMut<Weather>,
    mut gw: ResMut<GroundWater>,
    mut climate: ResMut<Climate>,
) {
    let dt = DT;
    let tick = gen.tick;
    // Rain is now LOCAL + cloud-driven: each cell wets only when a rain-bearing cloud drifts over it (sun
    // dries it the rest of the time). No global storms. weather.rain = the peak rain anywhere (for lightning
    // gating + logs); local rain per cell drives the ground water + viz.
    let mut peak = 0.0f32;
    for c in 0..gw.cell.len() {
        let cpos = cell_center(c);
        let d = cpos.normalize_or_zero();
        let rain = crate::sphere::rain_at(d, tick);
        let light = crate::sphere::daylight_at(d, tick);
        let absorb = 1.0 - crate::sphere::rockiness(cpos); // rocky sheds runoff, grassy soaks it up
        let w = gw.cell[c];
        let add = rain * absorb * RAIN_RATE * dt;
        let evap = EVAP * (0.2 + 0.8 * light) * w * dt; // sun dries the ground; fastest at noon
        gw.cell[c] = (w + add - evap).clamp(0.0, 1.0);
        peak = peak.max(rain);
    }
    weather.rain = peak;
    // slow climate memory: relax each cell toward its drifting long-run target on a months time constant.
    // CLIMATE_RATE * dt is tiny per tick -> the grid integrates rainfall-propensity over many days, so
    // regions turn to desert / rainforest gradually + the wet belt migrates as the target anomaly rotates.
    for c in 0..climate.cell.len() {
        let d = cell_center(c).normalize_or_zero();
        let target = crate::sphere::climate_target(d, tick);
        let cur = climate.cell[c];
        climate.cell[c] = (cur + (target - cur) * CLIMATE_RATE * dt).clamp(0.0, 1.0);
    }
}

// Fire grid (lightning-ignited wildfires): per-cell burn intensity 0..1. Shares the soil/ground-water
// grid resolution + indexing.
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

// --- fire (lightning): strikes during storms ignite dry cells; fire spreads through dry fuel, is doused
// by rain/wet ground, and deposits ash fertility. plant_step burns vegetation, live_step burns creatures.
pub fn fire_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    weather: Res<Weather>,
    gw: Res<GroundWater>,
    mut fire: ResMut<Fire>,
    mut soil: ResMut<Soil>,
) {
    let _ = gen;
    // Fire flammability fixed: ocean carries no fuel (is_ocean -> 0) and the polar ice cap is now a firebreak
    // (sphere::fuel gates to 0 across the ice-temperature band), so ignition + spread stay on vegetated,
    // non-frozen land only. Flip FIRE_ENABLED to false to disable wildfire (also kills the L god-control).
    const FIRE_ENABLED: bool = true;
    if !FIRE_ENABLED {
        if fire.cell.iter().any(|&f| f > 0.0) {
            fire.cell.iter_mut().for_each(|f| *f = 0.0);
        }
        return;
    }
    let dt = DT;
    let n = SOIL_RES;
    // Min flammable vegetation for a cell to ignite/carry fire. Oceans, bare rock + barren desert + the ice
    // cap have ~0 fuel, so they never burn and act as natural firebreaks (a strike there fizzles; fire can't
    // cross them). Raised so only solidly-vegetated land carries fire -> more firebreaks, fires stay local.
    const FUEL_MIN: f32 = 0.45;
    // lightning: during a storm, occasionally strike VEGETATED, currently-dry ground (storms hit land; wet
    // ground, water, rock + desert won't catch). Sample a few cells, strike the one with the most dry fuel.
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
            fire.cell[c] = 1.0; // struck dry vegetated fuel -> it catches
        }
    }
    if fire.cell.iter().all(|&f| f <= 0.02) {
        return; // nothing burning, skip the sweep
    }
    let cur = fire.cell.clone(); // spread reads the pre-tick state
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
            // spread only into flammable, dry-enough land; water/rock/desert/ice/wet ground are firebreaks.
            // spread rate scales with the NEIGHBOR's fuel density: lush forest catches fast, sparse scrub
            // barely carries fire -> fires stay confined to dense vegetation instead of racing everywhere.
            let fuel_n = crate::sphere::fuel(npos.normalize_or_zero());
            if fuel_n > FUEL_MIN && gw.get(npos) < FIRE_WET_MAX {
                fire.cell[ni] = (fire.cell[ni] + FIRE_SPREAD * f * fuel_n * dt).min(1.0);
            }
        }
    }
}

// The founding homeland: all initial life starts within HOMELAND_CAP radians of this direction, then
// spreads across the globe by reproduction/dispersal. (User: start them all in one area.)
pub fn homeland_center() -> Vec3 {
    Vec3::from(crate::sphere::HOMELAND_DIR).normalize()
}
pub const HOMELAND_CAP: f32 = 0.45; // ~26 deg cap: a continent-sized starting region

// A random LAND surface position in the founding homeland, sitting `offset` above the terrain (initial spawn).
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

// Offspring position: a great-circle hop of up to `spread` from the parent in a random direction, placed
// on the surface. Replaces the old flat x,z box offset; this is how plants/trees/creatures spread the globe.
fn disperse_pos(rng: &mut Rng, parent: Vec3, spread: f32, offset: f32) -> Vec3 {
    let heading = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
    let (d, _h) = crate::sphere::step(parent, heading, rng.range(0.0, spread.max(0.001)));
    crate::sphere::surface_pos(d, offset)
}

// Effective seed dispersal distance: the genome `spread` STRETCHED by wind dispersal and DRAGGED in by seed
// weight. Light + windborne -> far (dandelion/samara); heavy -> drops near the parent (oak acorn). Kept >=0.5
// so a seed always clears the parent stem. Shared by plants + trees (animal-carry multiplies this further).
fn eff_spread(g: &PlantGenome) -> f32 {
    (g.spread * (1.0 + WIND_RANGE * g.windborne) * (1.0 - SEED_DRAG * g.seed_weight)).max(0.5)
}

// Plant genetic distance for the species concept (--mating). 0 = identical, larger = more diverged. A `kind`
// mismatch alone (0.5) already exceeds PLANT_SPECIES_SIM, so different food families never cross (hard gate);
// within a family, distance is the mean gap across the ecologically meaningful genes. Under PLANT_SPECIES_SIM
// the two are the same species -> cross-compatible. Drift past it -> reproductive isolation -> speciation.
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

// Find the nearest cross-compatible mate for a seeding plant: same reproductive class (plant vs tree), within
// PLANT_MATE_RADIUS, genetic distance under PLANT_SPECIES_SIM, not itself. None -> selfing. mate_pool is
// (entity, pos, is_tree, genome), built once per tick only in --mating mode.
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

// Produce a child genome for a SEEDING plant/tree: in --mating mode cross with the nearest compatible mate
// (else self), then mutate. mate_pool empty (no --mating) -> always selfing, i.e. the old single-parent path.
// is_tree routes both the mate class and the mutation kind (mutate_tree vs mutate).
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

// A random LAND surface position anywhere on the globe, sitting `offset` above the terrain (dispersal/reseed).
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

// Plant spawn position: land OR shallow coastal water (avoids only DEEP ocean), so aquatic flora seeds the
// shallows -> a food base for the swimmer/"fish" niche. `homeland` clusters it with the founding pop.
fn plant_spawn_pos(rng: &mut Rng, homeland: bool, offset: f32) -> Vec3 {
    let (center, cap) = if homeland {
        (homeland_center(), HOMELAND_CAP)
    } else {
        (Vec3::Y, std::f32::consts::PI)
    };
    let mut d = crate::sphere::random_dir_in_cap(rng, center, cap);
    for _ in 0..8 {
        if crate::sphere::elevation01(d) >= crate::sphere::AQUATIC_FLOOR {
            break; // land or any non-abyssal water is fine; only deep ocean is rejected
        }
        d = crate::sphere::random_dir_in_cap(rng, center, cap);
    }
    crate::sphere::surface_pos(d, offset)
}

// --- hand-seeded DIVERSE world (--diverse): place niche-adapted creatures in matching regions so the
// showcase starts with coexisting niches (swimmers in wet coast, cold creatures at the poles, warm
// grazers at the equator, tall browsers temperate) instead of one converged winner. Genes are overridden
// for the niche; the brain is kept from a competent base genome so they actually forage + survive. ---

// A LAND position matching a niche: low (wet/coastal) vs high ground, near a target |latitude|.
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

// A SHALLOW-WATER position for the swimmer niche: in the sea but ABOVE the barren abyss, where aquatic flora
// grows -> swimmers spawn on their food. (niche_pos placed them on dry land, where swim costs SWIM_LAND_COST
// and they starved -> aquatic always died out.)
pub(crate) fn niche_water_pos(rng: &mut Rng, target_lat: f32, offset: f32) -> Vec3 {
    for _ in 0..120 {
        let d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
        let e = crate::sphere::elevation01(d);
        if e >= crate::sphere::SEA_LEVEL || e < crate::sphere::AQUATIC_FLOOR {
            continue; // want submerged-but-not-abyssal (the flora band)
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

// Override `g`'s trait genes for niche `i%5` and return a matching spawn position. Keeps g's brain/sensors.
fn diverse_creature(mut g: Genome, i: usize, rng: &mut Rng) -> (Genome, Vec3) {
    // niches 0,1 are swimmers -> seed them IN shallow water (on the aquatic flora); the rest are land niches.
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

fn diet_state(_g: &Genome) -> DietState {
    // newborns start with reserves stocked to the satisfaction level, so they aren't instantly deficient
    DietState { reserves: [RESERVE_REQ; NUTRIENTS], g: 0.0, age: 0, fatigue: 0.0, starve: 0, toxic_load: 0.0 }
}

// Eating plant matter: absorb each nutrient into reserves (delivered = plant baseline x soil fertility,
// gated by this creature's uptake gene for that nutrient), capped. Returns the toxin to burn (plant
// toxicity x amount eaten). Reserves later gate the master expression gene + drive deficiency (live_step).
fn absorb_and_toxin(reserves: &mut [f32; NUTRIENTS], uptake: &[f32; NUTRIENTS], pg: &PlantGenome, soil_f: f32, amount: f32) -> f32 {
    for i in 0..NUTRIENTS {
        reserves[i] = (reserves[i] + pg.nutrients[i] * soil_f * uptake[i] * NUTRIENT_ABSORB).min(RESERVE_CAP);
    }
    pg.toxicity * PLANT_TOX_HIT * amount
}

// Spawn carrion (meat) at a spot: a Food entity with the Rot clock. Used by death + predation kills.
fn spawn_carrion(commands: &mut Commands, pos: Vec3, mass: f32) {
    let p = crate::sphere::surface_pos(pos, FOOD_Y); // carrion lies on the surface at the death spot
    commands.spawn((
        Food,
        PlantState { mass, age: 0 },
        PlantGenome {
            kind: CARRION_KIND,
            nutrient: CARRION_NUTRIENT,
            defense: 0.0,    // meat has no bite-defense: easy to scavenge while fresh
            quality: 0.0,    // unused for carrion (separate eat branch); never disperses
            wet: 0.5,        // unused for carrion (excluded from moisture mortality by Without<Rot>)
            height: 0.0,     // carrion lies on the ground
            light_pref: 0.5, // unused for carrion
            regrow: 0.0,     // unused for carrion (eaten whole)
            branches: 0.0,   // unused for carrion
            spread: 0.0,
            maturity: 999.0, // never reproduces via plant_step (also excluded by Without<Rot>)
            nutrients: [0.0; NUTRIENTS], // unused: meat tops reserves flat (balanced tissue), separate eat branch
            toxicity: 0.0,
            fruit_toxicity: 0.0,
            // new genes: all neutral/off for carrion (it never grows, reproduces, or renders as a plant)
            temp_pref: 0.5,
            succulence: 0.0,
            submerged: 0.0,
            fruiting: 0.0,
            nitrogen_fix: 0.0,
            fire_seed: 0.0,
            climb: 0.0,
            allelopathy: 0.0,
            seed_weight: 0.0, // unused for carrion (never disperses)
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

// Spawn one creature (no render mesh; viz::add_creature_visuals gives it one in render mode).
// Used for continuous-mode offspring; fresh brain from the genome's priors, learns over its own life.
pub(crate) fn spawn_creature(commands: &mut Commands, g: Genome, pos: Vec3, rng: &mut Rng, birth_energy: f32) {
    // migrate older saved nets to the current brain-input width before the Brain copies the weights (a
    // shape mismatch would index out of bounds in forward()). No-op for fresh genomes + births.
    let mut g = g;
    g.ensure_net_shape();
    let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
    let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
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
        Locomotion { start: pos, path: 0.0 },
        Transform::from_translation(pos),
    ));
}

// God-control: drop a burst of `n` creatures scattered over the land (kids' "make more life!" button -> B
// in the render). Clones of random LIVING creatures + a light mutation, so they have competent brains and
// actually survive/behave (random genomes just flail at full thrust and die). Falls back to random genomes
// if the world is empty. Baseline provisioning, spread planet-wide. Used by viz::god_disturbances.
pub fn seed_burst(commands: &mut Commands, rng: &mut Rng, parents: &[Genome], n: usize) {
    for _ in 0..n {
        let g = if parents.is_empty() {
            Genome::random(rng)
        } else {
            let idx = ((rng.f32() * parents.len() as f32) as usize).min(parents.len() - 1);
            let mut c = parents[idx].clone();
            c.mutate(rng, 0.1, 0.1); // light tweak so the burst is varied, not identical clones
            c
        };
        let pos = rand_pos(rng, CREATURE_Y); // any land, scattered planet-wide
        spawn_creature(commands, g, pos, rng, BIRTH_ENERGY);
    }
}

// God-control: populate the WHOLE planet with plants + trees + creatures, each placed in habitat it can
// live in (aquatic flora + swimmers in the sea, alpine creatures + land flora + trees on land, climate-
// matched temp_pref). Lets you seed an empty/sparse world all over instead of waiting for spread. Render
// god-control P. Counts are targets; ocean/land/abyss filtering retries a few times per item.
pub fn seed_planet(commands: &mut Commands, rng: &mut Rng, parents: &[Genome], _ntypes: u8, n_creatures: usize, n_plants: usize, n_trees: usize) {
    use std::f32::consts::PI;
    let whole = |rng: &mut Rng| crate::sphere::random_dir_in_cap(rng, Vec3::Y, PI); // uniform over the globe
    // creatures: adapt each to where it lands so it survives there -> every region gets life
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
    // plants: aquatic flora (high wet) in water, land flora elsewhere; skip the barren abyss
    for _ in 0..n_plants {
        let mut d = whole(rng);
        for _ in 0..6 {
            if crate::sphere::elevation01(d) >= crate::sphere::AQUATIC_FLOOR {
                break;
            }
            d = whole(rng);
        }
        let pg = plant_for_site(rng, d); // species by biome (aquatic in water, land flora on land)
        spawn_plant(commands, pg, rng.range(0.5, 1.3) * PLANT_START_MASS, crate::sphere::surface_pos(d, FOOD_Y));
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

// A fresh (founding) tree genome: rich, tall, slow, with some branches. kind 0. From here trees evolve via
// PlantGenome::mutate_tree. Defense is kept LOW: trees are gated by REACH (height/branches vs creature
// height), not by bite-vs-defense (live_step), so a tree's defense protects nothing -- but growth_rate()
// taxes defense QUADRATICALLY (0.85*def^2), so a high defense just starves the tree below TREE_MATURITY ->
// a sterile, never-reproducing, never-dying "zombie tree". Low defense lets it grow + fruit + reproduce.
pub(crate) fn tree_genome(rng: &mut Rng) -> PlantGenome {
    PlantGenome {
        kind: 0,
        nutrient: rng.range(0.6, 1.0),
        defense: rng.f32() * 0.1,
        quality: rng.range(0.1, 0.4),
        wet: 0.5,
        height: rng.range(TREE_HEIGHT_MIN, 1.0), // wide initial height (never taller than 1.0)
        light_pref: rng.range(0.5, 0.9),
        regrow: rng.range(0.0, 0.4),
        branches: rng.range(0.0, 0.6),
        spread: rng.range(5.0, 9.0),
        maturity: TREE_MATURITY,
        // fruit trees are nutrient-rich (fruit is good food): a broad, generous profile + low toxicity
        nutrients: [0.55; NUTRIENTS],
        toxicity: rng.f32() * 0.15,
        fruit_toxicity: rng.f32() * 0.4, // fruit trees protect unripe fruit; ripens sweet for dispersal
        // trees: warm-ish climate niche, fruit-bearing crown, occasional blossom; form ignored (Tree marker)
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

// Spawn one tree (long-lived plant + Tree marker) from a genome. edible=true fruit tree, false=evergreen.
pub(crate) fn spawn_tree(commands: &mut Commands, mass: f32, pos: Vec3, edible: bool, g: PlantGenome) {
    commands.spawn((
        Food,
        Tree { edible },
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos),
    ));
}

// Pick a founding plant species for a surface site by biome (real-world distribution): ocean depth ->
// deep kelp (needs less sun) vs shallow lily/eelgrass/algae; land by temperature + moisture -> cold
// alpine/moss, arid cactus/tumbleweed/thistle, wet reed/fern, else a mixed meadow. The species then
// evolves from here. Replaces uniform-random plants so the world reads as real biomes.
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
    // cold cutoff matches the biome/sea-ice onset (temp < 0.34) so the frozen fringe gets cold-adapted
    // species (alpine cushion, moss), not temperate plants that would just sit on the ice and die back.
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

// Library-backed site seeding (tuning harness): when a tuned plant library is loaded, draw a biome-matched
// EVOLVED genome from it (mutated a touch for variety); else fall back to the archetype for the site so no
// biome ever goes bare. `lib` is None when no library file exists or --no-plant-lib was passed.
fn site_plant(rng: &mut Rng, lib: Option<&crate::persist::PlantLibrary>, d: Vec3) -> PlantGenome {
    if let Some(l) = lib {
        if let Some(mut g) = l.pick_for_site(rng, d, false) {
            g.mutate(rng);
            randomize_flower_color(&mut g, rng); // library genomes predate the bloom sat/light genes -> all bloom
            return g; // the same dull color; re-roll the COSMETIC flower color so the world seeds in varied bright hues
        }
    }
    plant_for_site(rng, d)
}

// Give a seeded plant a fresh BRIGHT flower color (cosmetic only, zero sim cost). Used on library draws whose
// saved genomes lack the flower_sat/flower_light genes (they load at a flat default -> identical dull blooms).
// flower_hue is full-wheel; sat/light span their bright sub-ranges in plant::flower_color -> any vivid color.
fn randomize_flower_color(g: &mut PlantGenome, rng: &mut Rng) {
    g.flower_hue = rng.f32();
    g.flower_sat = rng.f32();
    g.flower_light = rng.f32();
}

// Same, for trees (Tree marker). Draws a tuned tree genome biome-matched; else a fresh founding tree genome.
fn site_tree(rng: &mut Rng, lib: Option<&crate::persist::PlantLibrary>, d: Vec3) -> PlantGenome {
    if let Some(l) = lib {
        if let Some(mut g) = l.pick_for_site(rng, d, true) {
            g.mutate_tree(rng);
            randomize_flower_color(&mut g, rng); // varied bright blossoms (also re-rolls the blossom-presence hue)
            return g;
        }
    }
    tree_genome(rng)
}

// Botanical SHOWCASE (--garden): one of every plant species laid out in a tidy grid at the homeland, plus a
// back row of trees (fruit, evergreen, vine-draped evergreen). For inspecting every form at once instead of
// hunting for them across biomes. Plants spawn already-grown (mass ~ maturity) so they show at full size.
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
    let sp = 0.03_f32; // grid spacing in radians (~2.4 units): tight so the plants fill the view
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

// Spawn one plant (living food). No render mesh; add_plant_visuals (render mode) gives it one.
pub(crate) fn spawn_plant(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    commands.spawn((
        Food,
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos),
    ));
}

// Spawn one grass tuft. RENDER-ONLY ground cover: NO `Food` -> grass stays out of the per-tick food
// scan/sensing entirely (8000+ tufts would otherwise crush the food clone + crash foraging). Edibility is
// modeled by POSITION instead (live_step: a hungry creature on grass-bearing soil nibbles a trickle).
// grass_step owns its lifecycle + cap; oriented to the surface normal so blades stand up on the sphere.
fn spawn_grass(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    let up = pos.normalize_or_zero();
    commands.spawn((
        Grass,
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos).with_rotation(Quat::from_rotation_arc(Vec3::Y, up)),
    ));
}

// Effective grass habitability at a surface dir: static plant_habitability LIFTED by local rain (ground
// water raises effective soil moisture) -> dry desert greens briefly after a downpour, then dries out and
// the tufts die back. Lush ground is already moisture-capped, so rain can't over-grow it (no runaway food).
// gw = None -> static habitability (used at world seed, before any rain has fallen).
fn grass_hab(d: Vec3, gw: Option<&GroundWater>) -> f32 {
    let water = gw.map_or(0.0, |g| g.get(crate::sphere::surface_pos(d, 0.0)));
    let m = (crate::sphere::moisture(d) + WET_GAIN * water).clamp(0.0, 1.0);
    crate::sphere::plant_habitability_with_moisture(d, m)
}

// Whole-planet: grass blankets ALL plant-capable land worldwide (not just the homeland). Rejection samples
// the full sphere until a non-ocean spot whose EFFECTIVE habitability (with rain) clears GRASS_HAB_MIN, so a
// rained-on desert patch becomes eligible and the turf blooms there until it dries out.
fn grass_pos(rng: &mut Rng, gw: Option<&GroundWater>) -> Vec3 {
    let mut d = crate::sphere::random_dir_in_cap(rng, Vec3::Y, std::f32::consts::PI);
    for _ in 0..8 {
        if !crate::sphere::is_ocean(d) {
            // normal grassland + rain-bloomed desert clear the habitability gate; rocky ground still gets
            // SOME grass between the rocks, but only a small fraction of rocky samples take -> sparse + thin.
            let lush = grass_hab(d, gw) > GRASS_HAB_MIN;
            let rocky = crate::sphere::rockiness(d) > 0.12 && rng.f32() < ROCK_GRASS_FRAC;
            if lush || rocky {
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
    // --load resumes a saved population; otherwise a random founding pop. Positions re-randomized.
    let snap = gen.load.as_deref().and_then(crate::persist::load_snapshot);
    let genomes: Vec<Genome> = match &snap {
        Some(s) if !s.creatures.is_empty() => s.creatures.clone(),
        _ => (0..POP).map(|_| Genome::random(&mut rng)).collect(),
    };
    // loading a saved population into continuous mode skips the warm-up (the genomes are already
    // competent) -> drop straight into a living world. Desync energy + age so they don't act in lockstep.
    let skip_warmup = gen.continuous && snap.as_ref().is_some_and(|s| !s.creatures.is_empty());
    if skip_warmup {
        gen.generation = WARMUP_GENS;
    }
    for (i, g) in genomes.into_iter().enumerate() {
        // --diverse: niche-adapt each creature + place it in its region; else founding pop in the homeland.
        let (g, p) = if gen.diverse { diverse_creature(g, i, &mut rng) } else { (g, homeland_pos(&mut rng, CREATURE_Y)) };
        let mut g = g;
        g.ensure_net_shape(); // migrate older saved nets to the current brain-input width
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
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
            Locomotion { start: p, path: 0.0 },
            Transform::from_translation(p),
        ));
    }
    // diverse mode spreads life globally (creatures are placed in niches worldwide, so food must be too).
    if gen.garden {
        // showcase: one of every species in a grid at the homeland (+ a few trees) instead of a random world
        seed_garden(&mut commands, &mut rng);
    } else {
        // tuned plant seed-bank (tuning harness): when present, seed the planet biome-matched FROM it.
        let lib = gen.plant_lib.as_deref().and_then(crate::persist::load_plant_library);
        // resuming a populated world (e.g. cargo run auto-loading the showcase seed) or --diverse seeds the
        // WHOLE planet (plants in every biome). A fresh founding run (no saved plants) keeps the homeland start
        // (life begins in one area + spreads). Trees already seed whole-planet (spawn_trees).
        let resuming = snap.as_ref().is_some_and(|s| !s.plants.is_empty());
        let whole_planet = gen.diverse || resuming;
        let food_pos = |rng: &mut Rng| plant_spawn_pos(rng, !whole_planet, FOOD_Y); // land + shallow water (aquatic flora)
        match &snap {
            Some(s) if !s.plants.is_empty() => {
                for sp in &s.plants {
                    let p = food_pos(&mut rng);
                    // regenerate every loaded plant as fresh biome-matched flora (we do not carry legacy plants
                    // forward); sp.mass is kept so the food web reloads grown, not all seedlings.
                    let g = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero());
                    spawn_plant(&mut commands, g, sp.mass, p);
                }
            }
            _ => {
                for _ in 0..FOOD {
                    let p = food_pos(&mut rng);
                    let pg = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero()); // tuned-library or biome archetype
                    spawn_plant(&mut commands, pg, rng.range(0.3, 1.4) * PLANT_START_MASS, p); // varied mass desyncs the food supply
                }
            }
        }
        spawn_trees(&mut commands, &mut rng, gen.diverse, lib.as_ref());
    }
    // start the turf half-full; grass_step tops it up to GRASS_CAP from here. (Skip in --garden: a clean
    // showcase ground without the tall turf hiding the specimen plants.)
    if !gen.garden {
        for _ in 0..GRASS_CAP / 2 {
            spawn_grass(&mut commands, PlantGenome::grass(&mut rng), GRASS_START_MASS, grass_pos(&mut rng, None));
        }
    }
}

// Scatter the initial trees (half tall fruit trees, half uneatable evergreens) on habitable land. Always
// WHOLE-PLANET now: trees seed worldwide (then ambient reproduction fills toward TREE_CAP globally), so the
// planet -- not just the homeland -- grows forests. (`_global` kept for the call sites.)
fn spawn_trees(commands: &mut Commands, rng: &mut Rng, _global: bool, lib: Option<&crate::persist::PlantLibrary>) {
    let tree_pos = |rng: &mut Rng| rand_pos(rng, FOOD_Y);
    for i in 0..N_TREES {
        let mut p = tree_pos(rng);
        for _ in 0..6 {
            if crate::sphere::plant_habitability(p.normalize_or_zero()) > 0.4 {
                break;
            }
            p = tree_pos(rng);
        }
        // alternate fruit tree / evergreen; tuned-library tree genome if available, else a fresh founding one
        let g = site_tree(rng, lib, p.normalize_or_zero());
        spawn_tree(commands, rng.range(3.0, 9.0), p, i % 2 == 0, g);
    }
}

// Render: same components + mesh/material so we can watch. Plant visuals added by add_plant_visuals.
pub fn spawn_world_render(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    mut gen: ResMut<GenState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let creature_mesh = meshes.add(Capsule3d::new(0.4, 0.8));
    // shared creature mesh as a resource so viz::add_creature_visuals can dress creatures BORN mid-sim
    // (spawn_creature adds no mesh) -> newborns + B-button creatures become visible, not just the seed pop.
    commands.insert_resource(crate::viz::CreatureMesh(creature_mesh.clone()));
    // creature body PARTS (M4 genetic visuals): head/eye/leg meshes, child entities of each creature so a
    // genome's head size, eye count, and leg count are visible. Unit-ish base sizes; viz scales per genome.
    commands.insert_resource(crate::viz::CreatureParts {
        head: meshes.add(Sphere::new(0.5).mesh().ico(2).unwrap()), // diameter 1 at child scale 1
        eye: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        leg: meshes.add(Cylinder::new(0.5, 1.0)), // radius 0.5, height 1 -> thin legs after scaling
    });
    // per-form plant mesh library: one silhouette per plant::form (viz::add_plant_visuals picks by genome).
    // Round forms = icospheres; tall/leafy forms = procedural frond clumps; lily pad = a flat disc.
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
        trunk: meshes.add(Cylinder::new(0.14, 1.4)), // short stub; the canopy skirt drapes over most of it
        // fuller broadleaf crown: a cluster of overlapping blobs (centered ~origin; placed in the crown)
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
    // shared grass tuft mesh + one green material for ALL tufts (grass is ubiquitous; size_grass scales
    // each tuft's length by local soil). Double-sided so the thin blades show from both faces.
    commands.insert_resource(crate::viz::GrassMesh(meshes.add(crate::viz::grass_tuft_mesh())));
    commands.insert_resource(crate::viz::GrassMaterial(materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.52, 0.18),
        perceptual_roughness: 0.95,
        double_sided: true,
        cull_mode: None,
        ..default()
    })));

    // scattered boulders on rocky highland (render-only dressing): one shared low-poly icosphere + a gray
    // stone material, instanced many times with varied squat scale + spin so rocky land reads as a field of
    // rocks (grass still seeds between them where habitability allows). Static -> spawned once here, no
    // per-frame cost. Rejection-samples non-ocean ground, denser the rockier the spot; base sunk slightly
    // into the surface so each rock sits on the ground. NotShadowCaster keeps the shadow pass cheap.
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
                continue; // only genuinely rocky ground; probability rises with rockiness (denser on peaks)
            }
            let up = d;
            let base = crate::sphere::surface_pos(up, 0.0);
            let s = rng.range(0.4, 1.8) * (0.6 + rock); // bigger boulders on the rockiest peaks
            let (sx, sy, sz) = (s * rng.range(0.7, 1.3), s * rng.range(0.5, 1.0), s * rng.range(0.7, 1.3)); // squat
            let mut tf = Transform::from_translation(base - up * (sy * 0.18)); // sink the base into the ground
            tf.rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(rng.range(-PI, PI));
            tf.scale = Vec3::new(sx, sy, sz);
            commands.spawn((Mesh3d(rock_mesh.clone()), MeshMaterial3d(rock_mat.clone()), tf, bevy::light::NotShadowCaster));
            placed += 1;
        }
    }

    // --load resumes a saved population; otherwise a random founding pop. Positions re-randomized.
    let snap = gen.load.as_deref().and_then(crate::persist::load_snapshot);
    let genomes: Vec<Genome> = match &snap {
        Some(s) if !s.creatures.is_empty() => s.creatures.clone(),
        _ => (0..POP).map(|_| Genome::random(&mut rng)).collect(),
    };
    // loading a saved population into continuous mode skips warm-up (genomes already competent) ->
    // drop straight into a living world. Desync energy + age so they don't act in lockstep.
    let skip_warmup = gen.continuous && snap.as_ref().is_some_and(|s| !s.creatures.is_empty());
    if skip_warmup {
        gen.generation = WARMUP_GENS;
    }
    // --garden showcase: just a few creatures wandering the garden, not the whole population.
    let n_creatures = if gen.garden { 4 } else { usize::MAX };
    for (i, g) in genomes.into_iter().take(n_creatures).enumerate() {
        let (g, p) = if gen.diverse { diverse_creature(g, i, &mut rng) } else { (g, homeland_pos(&mut rng, CREATURE_Y)) };
        let mut g = g;
        g.ensure_net_shape(); // migrate older saved nets to the current brain-input width
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
        let mut diet = diet_state(&g);
        if skip_warmup {
            diet.age = (rng.f32() * 600.0) as u32;
        }
        let e = if skip_warmup { rng.range(0.7, 1.2) * START_ENERGY } else { START_ENERGY };
        // no mesh here: viz::add_creature_visuals dresses EVERY creature (founders + mid-sim births) with the
        // body + genetic head/eyes/legs, so there's one code path for the look (avoids plain-capsule founders).
        commands.spawn((
            Creature,
            g,
            brain,
            diet,
            Energy::from_total(e),
            Fitness(0.0),
            Heading(h),
            Alive(true),
            Locomotion { start: p, path: 0.0 },
            Transform::from_translation(p),
        ));
    }
    if gen.garden {
        // showcase: one of every species in a grid at the homeland (+ a few trees) instead of a random world
        seed_garden(&mut commands, &mut rng);
    } else {
        // tuned plant seed-bank (tuning harness): when present, seed the planet biome-matched FROM it.
        let lib = gen.plant_lib.as_deref().and_then(crate::persist::load_plant_library);
        // resuming a populated world (e.g. cargo run auto-loading the showcase seed) or --diverse seeds the
        // WHOLE planet (plants in every biome). A fresh founding run (no saved plants) keeps the homeland start
        // (life begins in one area + spreads). Trees already seed whole-planet (spawn_trees).
        let resuming = snap.as_ref().is_some_and(|s| !s.plants.is_empty());
        let whole_planet = gen.diverse || resuming;
        let food_pos = |rng: &mut Rng| plant_spawn_pos(rng, !whole_planet, FOOD_Y); // land + shallow water (aquatic flora)
        match &snap {
            Some(s) if !s.plants.is_empty() => {
                for sp in &s.plants {
                    let p = food_pos(&mut rng);
                    // regenerate every loaded plant as fresh biome-matched flora (we do not carry legacy plants
                    // forward); sp.mass is kept so the food web reloads grown, not all seedlings.
                    let g = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero());
                    spawn_plant(&mut commands, g, sp.mass, p);
                }
            }
            _ => {
                for _ in 0..FOOD {
                    let p = food_pos(&mut rng);
                    let pg = site_plant(&mut rng, lib.as_ref(), p.normalize_or_zero()); // tuned-library or biome archetype
                    spawn_plant(&mut commands, pg, rng.range(0.3, 1.4) * PLANT_START_MASS, p); // varied mass desyncs the food supply
                }
            }
        }
        spawn_trees(&mut commands, &mut rng, gen.diverse, lib.as_ref());
    }
    // start the turf half-full; grass_step tops it up to GRASS_CAP from here. (Skip in --garden: a clean
    // showcase ground without the tall turf hiding the specimen plants.)
    if !gen.garden {
        for _ in 0..GRASS_CAP / 2 {
            spawn_grass(&mut commands, PlantGenome::grass(&mut rng), GRASS_START_MASS, grass_pos(&mut rng, None));
        }
    }
}

// --- grass: render-only ground cover (no Food; edibility modeled by position in live_step). Own lifecycle
// + cap (kept off PLANT_CAP): killed by fire/drown/poor-soil so it persists only on plant-capable land,
// grows/regrows, and refills toward GRASS_CAP each tick -> whole-planet turf. ---
pub fn grass_step(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
    mut soil: ResMut<Soil>,
    gw: Res<GroundWater>,
    fire: Res<Fire>,
    mut q: Query<(Entity, &mut PlantState, &PlantGenome, &Transform), (With<Grass>, Without<Rot>)>,
) {
    if gen.garden {
        return; // --garden showcase: no turf (it would top back up to GRASS_CAP and bury the specimens)
    }
    let season = (gen.tick as f32 / GEN_TICKS as f32 * SEASON_FREQ).sin();
    let mut count = q.iter().count();
    for (e, mut st, g, tf) in &mut q {
        let ppos = tf.translation;
        let pdir = ppos.normalize_or_zero();
        // wildfire burns the tuft up: its (small) biomass becomes ash on top of the cell's per-tick ash
        // -> burned turf regrows richer. Low factor since grass is a lesser plant.
        if fire.get(ppos) > FIRE_KILL {
            soil.add(ppos, FIRE_BURN_ASH * st.mass * 0.5);
            commands.entity(e).despawn();
            count = count.saturating_sub(1);
            continue;
        }
        // mortality off plant-capable soil: dry/wet mismatch, poor site (rock/desert/cold), or submerged.
        // Keeps grass confined to the band where plants can grow. Dead grass just vanishes (no detritus).
        let water = gw.get(ppos);
        let m = (crate::sphere::moisture(pdir) + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0);
        let stress = (m - g.wet).abs();
        let mut hab = grass_hab(pdir, Some(&gw)); // rain-lifted: desert turf survives while wet, dies as it dries
        if crate::sphere::rockiness(pdir) > 0.12 {
            hab = hab.max(ROCK_GRASS_HAB); // thin grass clings between the rocks: don't cull it as a "poor site"
        }
        let e01 = crate::sphere::elevation01(pdir);
        let submersion = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
        let drown = DROWN_KILL * submersion * (1.0 - g.wet);
        let p_mort = MOISTURE_KILL * (stress - MOISTURE_TOLERANCE).max(0.0) + HABITAT_KILL * (0.3 - hab).max(0.0) + drown;
        if rng.f32() < p_mort {
            commands.entity(e).despawn();
            count = count.saturating_sub(1);
            continue;
        }
        // grow/regrow: same drivers as plants (soil fertility + rain + light match), capped at maturity.
        let light = crate::sphere::daylight_at(pdir, gen.tick);
        let fert = soil.get(ppos);
        let boost = (1.0 + FERT_GROWTH * (fert / FERT_CAP).min(1.0)) * (1.0 + WET_GROWTH * water);
        let lf = 0.35 + 0.65 * (1.0 - (light - g.light_pref).abs());
        st.mass = (st.mass + g.growth_rate() * boost * hab * lf * DT).min(g.maturity);
        st.age += 1;
    }
    // refill toward the target density: blankets plant-capable ground + replaces grazed/burned tufts.
    let mut spawned = 0;
    while count + spawned < GRASS_CAP {
        spawn_grass(&mut commands, PlantGenome::grass(&mut rng), GRASS_START_MASS, grass_pos(&mut rng, Some(&gw)));
        spawned += 1;
    }
}

// --- plants: grow, reproduce (disperse mutated offspring), reseed if the web nearly collapses (13) ---
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
    // scenario tuning harness: present ONLY under --scenario. Its presence disables the reseed floor (so an
    // isolated cohort isn't flooded) + counts births/deaths-by-cause. Absent in normal/headless runs (free).
    mut stats: Option<ResMut<crate::scenario::ScenarioStats>>,
    mut q: Query<(Entity, &mut PlantState, &PlantGenome, &Transform, Option<&Tree>), (Without<Rot>, Without<Grass>)>, // not carrion, not grass (grass_step owns grass)
) {
    soil.decay(); // fertility leaches / is taken up over time
    // scenario mode: no PLANT_MIN reseed (the cohort IS the only plants); normal mode keeps the floor.
    // Population caps are cohort-scale in scenario mode (stats.cap ~ 2x target) so a viable cohort grows
    // toward its target + shows vigor instead of booming to the global PLANT_CAP/TREE_CAP.
    let (reseed_floor, pcap, tcap) = match stats.as_ref() {
        Some(s) => (0, s.cap, s.cap),
        None => (PLANT_MIN, PLANT_CAP, TREE_CAP),
    };
    // season drifts on the global tick clock (advances in both modes; generation is frozen in continuous)
    let season = (gen.tick as f32 / GEN_TICKS as f32 * SEASON_FREQ).sin(); // -1 dry .. +1 wet
    let mut plant_count = q.iter().filter(|(.., t)| t.is_none()).count();
    let tree_count = q.iter().filter(|(.., t)| t.is_some()).count();
    let tree_positions: Vec<Vec3> = q.iter().filter_map(|(_, _, _, tf, t)| t.map(|_| tf.translation)).collect();
    // mating mode (--mating, shared with creatures): a pool of (entity, pos, is_tree, genome) so a seeding
    // plant/tree can find a nearby genetically-similar MATE to cross with. Built only when --mating (cloning
    // every plant genome each tick isn't free); else stays empty and reproduction is single-parent budding.
    let mate_pool: Vec<(Entity, Vec3, bool, PlantGenome)> = if gen.mating {
        q.iter().map(|(e, _, g, tf, tree)| (e, tf.translation, tree.is_some(), g.clone())).collect()
    } else {
        Vec::new()
    };
    let mut births: Vec<(PlantGenome, Vec3)> = Vec::new();
    let mut tree_births: Vec<(Vec3, bool, PlantGenome)> = Vec::new(); // (pos, edible, child genome)
    let mut detritus: Vec<(PlantGenome, f32, Vec3)> = Vec::new(); // moisture-killed plants -> poison
    let mut fruit_drops: Vec<(PlantGenome, Vec3)> = Vec::new(); // mature fruit trees -> fallen fruit (fast energy)
    for (e, mut st, g, tf, tree) in &mut q {
        let ppos = tf.translation;
        let pdir = ppos.normalize_or_zero();
        // wildfire: a plant/tree in a strongly-burning cell burns up (despawn). fire_step lays down ash
        // fertility per tick; ON BURN-UP the plant's own biomass also becomes ash -> extra soil nutrients
        // where it burned (x mass; trees deposit ~3x, bigger biomass). Burned ground regrows richer.
        if fire.get(ppos) > FIRE_KILL {
            let biomass = if tree.is_some() { 3.0 } else { 1.0 };
            soil.add(ppos, FIRE_BURN_ASH * st.mass * biomass);
            // serotiny: a fire-adapted plant releases a seed AS it burns (post-fire recruitment onto the
            // fresh ash, where competition just cleared) -> fire spreads its lineage, not just kills it.
            if tree.is_none() && rng.f32() < g.fire_seed {
                let child = mate_or_self(&mate_pool, e, ppos, g, false, &mut rng);
                births.push((child, disperse_pos(&mut rng, ppos, g.spread, FOOD_Y)));
            }
            if let Some(s) = stats.as_deref_mut() {
                s.death("fire");
            }
            commands.entity(e).despawn();
            if tree.is_none() {
                plant_count = plant_count.saturating_sub(1);
            }
            continue;
        }
        let light = crate::sphere::daylight_at(pdir, gen.tick); // positional day/night at this plant
        let fert = soil.get(ppos);
        let water = gw.get(ppos); // dynamic rain-fed ground water at this spot
        // fertility AND rain-watered ground both speed growth (rain visibly greens the land)
        let boost = (1.0 + FERT_GROWTH * (fert / FERT_CAP).min(1.0)) * (1.0 + WET_GROWTH * water);
        // light factor: growth peaks when daylight matches this plant's light_pref (sun vs shade species).
        // (Clouds are visual-only -- folding their dimming into growth reduced average food enough to tip
        // marginal populations; light niches come from the day/night cycle, which is predictable + stable.)
        let lf = 0.35 + 0.65 * (1.0 - (light - g.light_pref).abs());
        if let Some(tree) = tree {
            // trees are land-only: a tree standing in water drowns fast (no kelp/mangrove forests). Clears
            // coastal seeds that landed in the sea + any tree left underwater by a rising sea level.
            if crate::sphere::is_ocean(pdir) && rng.f32() < DROWN_TREE {
                if let Some(s) = stats.as_deref_mut() {
                    s.death("drown");
                }
                commands.entity(e).despawn();
                continue;
            }
            // fed-on this tick? (key present even for harmless branch-feeders, who do 0 mass damage).
            // Apply the recorded mass damage; a fruit tree grazed below TREE_MIN_MASS is over-eaten -> dies.
            let grazed = tree_bites.0.contains_key(&e);
            if grazed {
                st.mass = (st.mass - tree_bites.0[&e]).max(0.0);
                if st.mass < TREE_MIN_MASS {
                    if let Some(s) = stats.as_deref_mut() {
                        s.death("eaten");
                    }
                    commands.entity(e).despawn();
                    continue;
                }
            }
            // tree climate niche: trees stay moisture-immune (long-lived, deep roots) but DO feel temperature.
            // tmiss = how far local temp sits from this tree's temp_pref. Far off its (wide) thermal band a
            // tree dies back -> no forests on the frozen pole or in the desert heat; near its band it's spared.
            let tmiss = (crate::sphere::base_temperature(pdir) - g.temp_pref).abs();
            if rng.f32() < TREE_TEMP_KILL * (tmiss - TREE_TEMP_TOL).max(0.0) {
                if let Some(s) = stats.as_deref_mut() {
                    s.death("temp");
                }
                commands.entity(e).despawn();
                continue;
            }
            // off-niche grows slower too (shared floor: never zero, just sluggish) -> trees fastest in their band.
            let temp_grow = TEMP_FLOOR + (1.0 - TEMP_FLOOR) * (1.0 - tmiss);
            // SOIL response (survival stays moisture-immune; this only shapes growth speed + final SIZE): a
            // tree grows faster AND to a BIGGER size on good ground -- nutritious (fertile) soil AND a moisture
            // SWEET SPOT (wet enough, not waterlogged). Bone-dry, swampy, or poor soil -> slower + smaller.
            let clim = crate::sphere::moisture(pdir) * (1.0 - CLIMATE_VEG) + climate.get(ppos) * CLIMATE_VEG;
            let m = (clim + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0); // effective local moisture
            let moist_q = (1.0 - (m - TREE_WET_OPT).abs() / TREE_WET_TOL).clamp(0.0, 1.0); // hump: 1 at the sweet spot
            let fert_q = (fert / FERT_CAP).min(1.0);
            let soil_q = moist_q * (0.4 + 0.6 * fert_q); // 0 (bad ground) .. 1 (rich + ideally moist)
            let grow_mult = (1.0 + FERT_GROWTH * fert_q) * (TREE_WET_FLOOR + (1.0 - TREE_WET_FLOOR) * moist_q);
            // good soil lets a tree grow to a BIGGER final size (cap), but it still MATURES (fruits +
            // reproduces) at g.maturity so the food supply + tree spread are unchanged -- it just keeps
            // growing past maturity toward full_size (fruit drop is mass-free; only seeding buds cost mass),
            // so a tree on rich, ideally-moist ground ends up visibly larger.
            let full_size = g.maturity * (1.0 + TREE_SOIL_SIZE * soil_q);
            st.mass = (st.mass + g.growth_rate() * grow_mult * lf * temp_grow * TREE_GROWTH_SCALE * DT).min(full_size);
            st.age += 1;
            let r2 = TREE_DENSITY_R * TREE_DENSITY_R;
            let local = tree_positions.iter().filter(|p| p.distance_squared(tf.translation) < r2).count();
            let fert_boost = 0.4 + 1.6 * fert_q; // richer ground -> more new trees
            let mature = st.mass >= g.maturity;
            // mature fruit trees drop fruit nearby (the forageable fast-energy source). Drop rate scales
            // with the tree's nutrient richness -> a richer tree fruits more (its growth already paid for it).
            if mature && tree.edible && rng.f32() < P_FRUIT_DROP * (0.5 + g.nutrient) {
                let fpos = disperse_pos(&mut rng, ppos, 3.0, FOOD_Y); // falls within the crown's footprint
                fruit_drops.push((g.clone(), fpos));
            }
            let ambient = mature && local <= TREE_MAX_LOCAL && rng.f32() < P_TREE_REPRO * fert_boost;
            let disperse = mature && tree.edible && grazed && rng.f32() < P_TREE_EAT_DISPERSE; // seed carried off
            if (ambient || disperse) && tree_count + tree_births.len() < tcap {
                let base = eff_spread(g); // wind/weight shape reach (samara flies, acorn drops); animal-carry adds more
                let spread = if disperse { base * TREE_EAT_SPREAD_MULT } else { base };
                let pos = disperse_pos(&mut rng, ppos, spread, FOOD_Y);
                // trees are land-only: drop seeds that landed in the sea (no ocean forests)
                if !crate::sphere::is_ocean(pos.normalize_or_zero()) {
                    // child inherits the parent's tree genome: crossed with a nearby compatible tree in
                    // --mating mode (oak x oak -> tree species), else selfed; then mutated (trees evolve like plants)
                    let child = mate_or_self(&mate_pool, e, ppos, g, true, &mut rng);
                    tree_births.push((pos, tree.edible, child));
                }
                st.mass *= PLANT_REPRO_FRAC; // budding a seed costs the parent mass either way
            }
            continue;
        }
        // --- regular plant ---
        // apply grazing from live_step: a plant eaten below PLANT_MIN_MASS was consumed (carrot) -> gone;
        // a berry-bush (high regrow) only loses a small bite and survives to regrow.
        if let Some(&bite) = tree_bites.0.get(&e) {
            st.mass = (st.mass - bite).max(0.0);
            if st.mass < PLANT_MIN_MASS {
                if let Some(s) = stats.as_deref_mut() {
                    s.death("eaten");
                }
                commands.entity(e).despawn();
                soil.add(ppos, DEATH_FERT * 0.3); // a consumed plant returns some nutrients to the ground
                plant_count = plant_count.saturating_sub(1);
                continue;
            }
        }
        // mortality from moisture mismatch OR a poor site (deep water / desert). Effective moisture =
        // slow CLIMATE moisture (long-term, drifts -> deserts/rainforests form) + season + rain-fed ground
        // water -> wet-liking plants thrive after a downpour, get stressed in drought, AND whole regions
        // dry to desert / green to forest over years as climate drifts (temporal + geological selection).
        let clim = crate::sphere::moisture(pdir) * (1.0 - CLIMATE_VEG) + climate.get(ppos) * CLIMATE_VEG;
        let m = (clim + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0);
        // succulence buffers DROUGHT: water-storing plants (cactus/aloe) tolerate sites drier than their
        // preferred `wet` -> they survive the desert where a thirsty plant would dry out. Wet-side stress
        // (too soggy) is NOT buffered -> a cactus still rots in a swamp.
        let dry_deficit = (g.wet - m).max(0.0);
        let stress = ((m - g.wet).abs() - SUCC_BUFFER * g.succulence * dry_deficit).max(0.0);
        // habitability uses the SAME slow climate moisture, so a region drying out loses its plant base
        // (desertifies) and a wetting region gains one (reforests). Ocean/thermal branches are unaffected.
        let hab = crate::sphere::plant_habitability_with_moisture(pdir, clim); // 0 in deep ocean/desert/cold, 1 on good land
        // drowning: underwater a plant needs the aquatic `wet` gene to live. submersion (depth below sea) x
        // (1-wet) -> land flora (low wet) drowns, aquatic flora (high wet) thrives. Splits land vs aquatic.
        let e01 = crate::sphere::elevation01(pdir);
        let submersion = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
        let drown = DROWN_KILL * submersion * (1.0 - g.wet);
        // desiccation (mirror of drown): an aquatic plant (high wet) stranded on DRY land (not submerged AND
        // low moisture) dries out and dies -> aquatic flora can't carpet the land. Marsh/wet ground (high m)
        // and shallow water spare it, so reeds at the water's edge survive.
        let aquatic = ((g.wet - 0.85) / 0.15).clamp(0.0, 1.0);
        let desiccate = DESICCATE_KILL * aquatic * (1.0 - submersion) * (1.0 - (m / 0.6).min(1.0));
        // climate niche: a plant grows best where local temperature matches temp_pref (alpine cushion in the
        // cold, cactus in the heat); off-niche it grows slowly and, far off its band, dies back.
        let temp = crate::sphere::base_temperature(pdir);
        let tmiss = (temp - g.temp_pref).abs();
        let temp_grow = TEMP_FLOOR + (1.0 - TEMP_FLOOR) * (1.0 - tmiss);
        let m_moist = MOISTURE_KILL * (stress - MOISTURE_TOLERANCE).max(0.0);
        let m_hab = HABITAT_KILL * (0.3 - hab).max(0.0);
        let m_temp = TEMP_KILL * (tmiss - TEMP_TOL).max(0.0);
        let p_mort = m_moist + m_hab + drown + desiccate + m_temp;
        if rng.f32() < p_mort {
            // scenario tuning: attribute the death to its dominant cause (so the agent sees WHY a cohort dies).
            if let Some(s) = stats.as_deref_mut() {
                let causes = [("moisture", m_moist), ("habitat", m_hab), ("drown", drown), ("desiccate", desiccate), ("temp", m_temp)];
                let cause = causes.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|c| c.0).unwrap_or("moisture");
                s.death(cause);
            }
            // allelopathic litter: a chemical-warfare plant leaves extra-toxic detritus (juglone-style leaf
            // litter) that suppresses competitors germinating on the same ground. Litter carries >= allelopathy.
            let mut litter = g.clone();
            litter.toxicity = litter.toxicity.max(g.allelopathy);
            commands.entity(e).despawn();
            detritus.push((litter, st.mass, tf.translation));
            soil.add(ppos, DEATH_FERT * 0.3); // a dead plant enriches the ground where it falls
            plant_count = plant_count.saturating_sub(1);
            continue;
        }
        // underwater the water column dims sunlight with depth -> deep plants get little light, so only shade
        // species (low light_pref, e.g. kelp) thrive deep; sun-lovers (lily) need the shallows. A vine adds
        // a light bonus (climbs toward the canopy) without paying the height growth cost.
        let light_uw = light * (1.0 - WATER_LIGHT_ATTEN * submersion);
        let lf = (0.35 + 0.65 * (1.0 - (light_uw - g.light_pref).abs()) + CLIMB_LIGHT * g.climb).min(1.0);
        // nitrogen-fixer (legume): root nodules enrich local soil fertility each tick (clover/beans).
        if g.nitrogen_fix > 0.0 {
            soil.add(ppos, NFIX_RATE * g.nitrogen_fix * DT);
        }
        // fertile soil speeds growth (M5); scales with habitability (P3), light match, and climate niche
        st.mass += g.growth_rate() * boost * hab * lf * temp_grow * DT;
        st.age += 1;
        let mature = st.mass >= g.maturity;
        // a fruiting non-tree (berry bush, nightshade) drops fallen fruit -> the fast-energy + ferment chain,
        // exactly like a fruit tree. Drop rate scales with the fruiting gene (its growth already paid for it).
        if mature && g.fruiting > 0.2 && rng.f32() < P_FRUIT_DROP * g.fruiting {
            fruit_drops.push((g.clone(), disperse_pos(&mut rng, ppos, 2.0, FOOD_Y)));
        }
        // endozoochory: a fruiting plant that SURVIVED being grazed this tick can have a seed carried off in
        // the eater + dropped FAR (animal dispersal, like fruit trees). Toxic fruit is eaten less, so the
        // chance scales DOWN with toxicity -> a toxic plant disperses little and its offspring stay clustered
        // around the parent (toxicity keeps them close, the user's ask). aquatic-only seeds still gated to water.
        if mature
            && g.fruiting > 0.2
            && tree_bites.0.contains_key(&e)
            && plant_count + births.len() < pcap
            && rng.f32() < P_PLANT_EAT_DISPERSE * g.fruiting * (1.0 - g.fruit_toxicity)
        {
            let child = mate_or_self(&mate_pool, e, ppos, g, false, &mut rng);
            let pos = disperse_pos(&mut rng, ppos, eff_spread(g) * PLANT_EAT_SPREAD_MULT, FOOD_Y);
            if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                births.push((child, pos));
            }
        }
        // clonal spread (rhizome / runner / sucker): a separate SHORT-range pathway that sprouts a true clone
        // (no mutate, no gene shuffle) right beside the parent -> dense local patch WITHOUT seeding (strawberry,
        // aspen). Lets a plant dominate ground clonally; pays in growth (growth_rate cost) + parent mass.
        if mature
            && g.clonal > 0.0
            && plant_count + births.len() < pcap
            && rng.f32() < P_CLONAL * g.clonal
        {
            let pos = disperse_pos(&mut rng, ppos, CLONAL_RADIUS, FOOD_Y);
            if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                births.push((g.clone(), pos)); // identical ramet: fidelity over variation
                st.mass *= PLANT_REPRO_FRAC; // budding a ramet costs the parent
            }
        }
        // cling (epizoochory): a passing animal snags a burr/sticky seed + carries it FAR, even though this
        // plant is never eaten -> defended/toxic/inedible plants still disperse by animal (burdock, cleavers).
        // Flat per-tick chance (abstracts animal traffic; no proximity scan). Independent of fruiting/toxicity.
        if mature
            && g.cling > 0.0
            && plant_count + births.len() < pcap
            && rng.f32() < P_CLING * g.cling
        {
            let child = mate_or_self(&mate_pool, e, ppos, g, false, &mut rng);
            let pos = disperse_pos(&mut rng, ppos, eff_spread(g) * CLING_SPREAD_MULT, FOOD_Y);
            if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                births.push((child, pos));
            }
        }
        if mature
            && plant_count + births.len() < pcap
            && rng.f32() < P_REPRO * (1.0 - DEF_REPRO_COST * g.defense)
        {
            let child = mate_or_self(&mate_pool, e, ppos, g, false, &mut rng);
            // hydrochory: a seed from a plant AT/NEAR water floats + rides far. near_water = 1 at/below sea
            // level, tapering to 0 a short band above -> only coastal/aquatic plants get the long water reach.
            let near_water = ((crate::sphere::SEA_LEVEL + HYDRO_COAST_BAND - e01) / HYDRO_COAST_BAND).clamp(0.0, 1.0);
            let hydro = 1.0 + HYDRO_RANGE * g.hydrochory * near_water;
            // wind + seed weight set how far this seed travels (dandelion flies, acorn drops near the parent)
            let pos = disperse_pos(&mut rng, ppos, eff_spread(g) * hydro, FOOD_Y);
            // aquatic plants (high wet) only seed into water; a seed that lands on dry ground is dropped
            // (mirror of land-only trees). Parent still pays the budding cost either way.
            if !(g.wet > 0.85 && !crate::sphere::is_ocean(pos.normalize_or_zero())) {
                // dormancy: a fraction of seeds go DORMANT into the soil bank (germinate later) instead of
                // sprouting now -> the lineage survives a surface wipe (fire/drought) and booms after. The
                // rest sprout immediately. Bank is capped; when full, dormant seeds are simply lost.
                if rng.f32() < DORMANCY_FRAC * g.dormancy && bank.0.len() < SEED_BANK_CAP {
                    let wait = DORMANT_TICKS_MIN + (rng.f32() * (DORMANT_TICKS_MAX - DORMANT_TICKS_MIN) as f32) as u32;
                    bank.0.push((child, pos, wait));
                } else {
                    births.push((child, pos));
                }
            }
            st.mass *= PLANT_REPRO_FRAC;
        }
    }
    // seed bank germination: count every buried seed down; one that reaches 0 sprouts this tick (subject to
    // the cap) and leaves the bank. A patch cleared by fire/drought thus re-greens from its bank later (dormancy).
    let mut germinated: Vec<(PlantGenome, Vec3)> = Vec::new();
    bank.0.retain_mut(|(g, pos, ticks)| {
        *ticks = ticks.saturating_sub(1);
        if *ticks == 0 && plant_count + births.len() + germinated.len() < pcap {
            germinated.push((g.clone(), *pos));
            false // sprouted -> remove from the bank
        } else {
            true // still dormant (or ready but the cap is full this tick: retry next tick)
        }
    });
    births.extend(germinated);
    // dead plants -> detritus that FERMENTS (completes the rot chain, P3): poor food, fermenting to a
    // little toxic FAST energy, then gone. Ferment marker routes it to the plant-matter eat branch.
    for (g, mass, pos) in detritus {
        commands.spawn((
            Food,
            PlantState { mass: mass.min(CARRION_MASS), age: 0 },
            PlantGenome { nutrient: DETRITUS_NUTRIENT, defense: 0.0, quality: 0.0, ..g },
            Rot { age: 0 },
            Ferment { toxic: FERMENT_TOX_DETRITUS },
            Transform::from_translation(pos),
        ));
    }
    // fruit drops: a mature fruit tree drops a fruit nearby. Fallen fruit is a Food carrying the tree's
    // genome (rich + sugary) + a Rot clock + Ferment -> fresh = sugar, ferments to FAST energy, then gone.
    for (g, pos) in fruit_drops {
        // ferment/unripe toxicity is now the plant's genetic fruit_toxicity (not a constant). The Seed carries
        // the FULL parent genome so a ripe-eaten fruit plants a true offspring; the Food genome stays height/
        // defense-zeroed so the fruit renders flat on the ground and any creature can eat it.
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
    // reseed floor: keep a minimal seed bank so creatures can't drive food fully extinct (biome-matched).
    // Disabled in scenario mode (reseed_floor=0) so a tuned cohort stays isolated.
    while plant_count + births.len() < reseed_floor {
        let pos = rand_pos(&mut rng, FOOD_Y);
        births.push((plant_for_site(&mut rng, pos.normalize_or_zero()), pos));
    }
    // scenario tuning: count this tick's cohort offspring (reseed floor is off in scenario, so births +
    // tree_births are all real reproduction) -> drives the R (births/deaths) reproductive-success metric.
    if let Some(s) = stats.as_deref_mut() {
        s.births += (births.len() + tree_births.len()) as u32;
    }
    for (g, pos) in births {
        // heavy (well-provisioned) seeds establish as BIGGER, hardier seedlings (head start); light seeds
        // start tiny -> the seed-weight trade-off: far dispersal vs strong establishment.
        let est = 0.6 + 0.8 * g.seed_weight; // 0.6x .. 1.4x establishment mass
        spawn_plant(&mut commands, g, rng.range(0.5, 1.3) * PLANT_START_MASS * est, pos); // varied reseed mass (staggered maturity)
    }
    for (pos, edible, g) in tree_births {
        spawn_tree(&mut commands, PLANT_START_MASS, pos, edible, g);
    }
    tree_bites.0.clear(); // consumed this tick
}

// --- predation (M5): creatures attack + eat each other. bite = combat (attack + defense). Opportunistic
// for now; NN-driven attack arrives with the creature-sensing batch. ---
pub fn predation_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    mut commands: Commands,
    mut soil: ResMut<Soil>,
    mut cq: Query<(Entity, &Transform, &mut Energy, &mut Fitness, &mut Alive, &Genome), With<Creature>>,
) {
    // snapshot living creatures: (entity, pos, ATTACK combat, energy, kin-sig, DEFENSE combat, venom, climb).
    // attack = bite + size; defense = attack + armor (armor protects but doesn't help you hunt).
    let snap: Vec<(Entity, Vec3, f32, f32, [f32; 10], f32, f32, f32)> = cq
        .iter()
        .filter(|(_, _, _, _, a, _)| a.0)
        .map(|(e, t, en, _, _, g)| {
            let attack = g.bite + SIZE_COMBAT * g.size;
            (e, t.translation, attack, en.total(), signature(g), attack + ARMOR_DEF * g.armor, g.venom, g.climb)
        })
        .collect();
    if snap.len() < 2 {
        return;
    }
    let mut killed: HashSet<Entity> = HashSet::new();
    let mut gains: HashMap<Entity, f32> = HashMap::new();
    let r2 = ATTACK_RADIUS * ATTACK_RADIUS;
    let rs2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
    for (ai, &(ae, apos, abite, aenergy, _asig, _, _, _)) in snap.iter().enumerate() {
        if killed.contains(&ae) {
            continue; // a creature killed this tick doesn't also attack
        }
        // only a HUNGRY creature hunts -> well-fed crowds don't cannibalize each other (this was the
        // continuous-mode killer: at high density, packed well-fed creatures mass-cannibalized -> crash).
        if aenergy > PREDATION_HUNGER {
            continue;
        }
        let mut best: Option<(f32, usize)> = None;
        for (bi, &(be, bpos, _, _, _, _, _, _)) in snap.iter().enumerate() {
            if bi == ai || killed.contains(&be) {
                continue;
            }
            let d2 = apos.distance_squared(bpos);
            if d2 < r2 && best.is_none_or(|(bd, _)| d2 < bd) {
                best = Some((d2, bi));
            }
        }
        if let Some((_, bi)) = best {
            let (be, bpos, _battack, _, bsig, bdef, bven, bclimb) = snap[bi];
            // herd safety: prey surrounded by KIN is harder to pick off (vigilance) -> being social pays
            let mut kin = 0.0f32;
            for (e2, p2, _, _, s2, _, _, _) in &snap {
                if *e2 != be && bpos.distance_squared(*p2) < rs2 && sig_dist(&bsig, s2) < SOCIAL_SIM {
                    kin += 1.0;
                }
            }
            let prey_kin = (kin / SOCIAL_TARGET).min(1.0);
            // success = attacker combat vs prey DEFENSE (combat + armor), minus a required edge
            // (PREDATION_BIAS: equal-combat creatures barely prey on each other), reduced by herd safety
            // AND the prey's climb agility (arboreal escape).
            let success = sigmoid(BITE_K * (abite - bdef) - PREDATION_BIAS)
                * (1.0 - SOCIAL_SAFETY * prey_kin)
                * (1.0 - CLIMB_EVADE * bclimb);
            if rng.f32() < success {
                killed.insert(be);
                // venomous prey is a sickening kill -> the predator gains far less (the venom deterrent)
                *gains.entry(ae).or_insert(0.0) += PREDATION_GAIN * (1.0 - VENOM_DETER * bven);
            }
        }
    }
    if killed.is_empty() {
        return;
    }
    let continuous_live = gen.continuous && gen.generation >= WARMUP_GENS;
    for (e, t, mut energy, mut fit, mut alive, gen_e) in &mut cq {
        if let Some(g) = gains.get(&e) {
            energy.add_fat(*g, fat_cap(gen_e)); // a kill = meat -> fat store
            fit.0 += g * 0.3; // predation counts toward selection
        }
        if killed.contains(&e) {
            alive.0 = false;
            spawn_carrion(&mut commands, t.translation, CARRION_MASS * 0.5); // predator already ate some
            soil.add(t.translation, DEATH_FERT); // death enriches the ground here
            if continuous_live {
                commands.entity(e).despawn();
            }
        }
    }
}

// --- carrion decomposition (P3): age each corpse, shrink its mass, despawn when fully rotted ---
// On full decomposition, release fertility to the soil at that spot (M5 closed loop): death -> soil.
pub fn rot_step(
    mut commands: Commands,
    mut soil: ResMut<Soil>,
    mut q: Query<(Entity, &mut Rot, &mut PlantState, &PlantGenome, &Transform)>,
) {
    for (e, mut rot, mut st, g, tf) in &mut q {
        rot.age += 1;
        st.mass = (st.mass - CARRION_MASS / ROT_GONE as f32).max(0.0); // decompose: less to scavenge
        if rot.age >= ROT_GONE {
            soil.add(tf.translation, DECOMP_FERT * g.nutrient); // return nutrients
            commands.entity(e).despawn(); // fully decomposed
        }
    }
}

// --- per-tick life: sense -> think -> move -> eat -> metabolism -> learn ---
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
    scen: Option<Res<crate::scenario::ScenarioStats>>, // present => scenario mode: disable the global reseed floor
) {
    let dt = DT;
    let ntypes = gen.ntypes();
    let mut pop = cq.iter().count(); // live population (for the continuous-mode reproduction cap)
    // continuous birth/death is active only AFTER the generational warm-up (see WARMUP_GENS)
    let live_continuous = gen.continuous && gen.generation >= WARMUP_GENS;
    // (daylight is now POSITIONAL: computed per-creature from its location, see `light` inside the loop)
    // snapshot: (entity, pos, genome, mass, rot_age, tree, ferment_toxic). rot_age=Some + ferment=None ->
    // animal carrion (meat); rot_age=Some + ferment=Some -> fermenting plant matter (fruit/detritus);
    // tree=Some(edible) -> a tree; else a living plant.
    let foods: Vec<(Entity, Vec3, PlantGenome, f32, Option<u32>, Option<bool>, Option<f32>, Option<PlantGenome>)> = fq
        .iter()
        .map(|(e, t, st, pg, rot, tree, ferment, seed)| {
            (e, t.translation, pg.clone(), st.mass, rot.map(|r| r.age), tree.map(|t| t.edible), ferment.map(|f| f.toxic), seed.map(|s| s.0.clone()))
        })
        .collect();
    let mut eaten: HashSet<Entity> = HashSet::new();
    let mut sample_genome: Option<Genome> = None; // a living genome, for the near-extinction reseed floor
    // creature snapshot for the social/kin need + threat sense: (entity, pos, signature, combat). Combat =
    // bite + size, so a creature can sense a bigger-combat neighbor as a predator (flee).
    let cre_snap: Vec<(Entity, Vec3, [f32; 10], f32)> = cq
        .iter()
        .filter(|(_, _, _, _, _, a, _, _, _, _)| a.0)
        .map(|(e, t, _, _, _, _, g, _, _, _)| (e, t.translation, signature(g), g.bite + SIZE_COMBAT * g.size))
        .collect();
    // mating mode: a pool of (entity, pos, signature, genome) so a breeding creature can find a nearby
    // genetically-similar MATE to cross with. Built only when --mating (cloning genomes isn't free).
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
        if sample_genome.is_none() {
            sample_genome = Some(genome.clone());
        }
        let pos = ct.translation;
        let fat_max = fat_cap(genome); // adiposity + size set this creature's fat-store ceiling

        // SINGLE grid-bounded pass over nearby foods (perf): scan only the food-grid cells within the
        // query radius (>= max sensor range, so every sensable food is covered; >= NEAR_QUERY so the
        // global-nearest is found given dense plants). Computes the global nearest (approach/eat) AND each
        // sensor's nearest-food-in-cone together, atan2 once per in-range food. Not bit-identical (cell
        // iteration order differs), but far fewer foods touched -> big speedup at scale.
        let n_s = genome.sensors.len();
        // eyes gene: more eyes = a small effective-range boost (sharper sight) on every sensor.
        let eye_mult = 1.0 + EYE_SENSE_BONUS * genome.eyes;
        let max_range = genome.sensors.iter().map(|s| s.range).fold(0.0f32, f32::max) * eye_mult;
        let mut best: Option<(usize, f32)> = None;
        let mut sd = vec![f32::INFINITY; n_s]; // nearest dist per sensor
        let mut skind = vec![0u8; n_s]; // food kind that nearest sensor-food is
        let mut nearest_tree_d2 = f32::INFINITY; // nearest tree (canopy shade), any kind
        let _r = max_range.max(NEAR_QUERY);
        // scan a neighborhood of food-grid (lon/lat) cells around this creature. SPAN cells each way covers
        // the sensor + near-query radius at this grid resolution. (Longitude does not wrap here + pole cells
        // are narrow -> a minor perception approximation near the date line/poles; food is dense, so fine.)
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
                    let d2 = to.length_squared();
                    if best.is_none_or(|(_, bd2)| d2 < bd2) {
                        best = Some((i, d2));
                    }
                    if f.5.is_some() && d2 < nearest_tree_d2 {
                        nearest_tree_d2 = d2; // f.5 = tree marker -> overhead canopy for shade
                    }
                    let dist = d2.sqrt();
                    if dist > max_range {
                        continue; // out of every sensor's range -> skip the bearing + cone tests
                    }
                    // bearing in the local tangent frame: 0 = north (toward +Y pole), +pi/2 = east
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
        // positional day/night: daylight depends on WHERE on the globe this creature is (the lit half faces
        // the sun, terminator sweeps as the planet spins) -> light niches now also vary by location.
        let light = crate::sphere::daylight_at(pdir, gen.tick);
        // aquatic factor at this spot: 1 in ocean / wet lowland, 0 on high dry ground (low elevation = wet).
        // Swimmers move faster here (fins) + pay on dry land (see metabolism); also a brain input.
        let h0 = crate::sphere::elevation(pdir);
        let wet_here = ((SWIM_WET_LEVEL - h0) / SWIM_WET_LEVEL).clamp(0.0, 1.0);
        // shade: how shaded by overhead canopy (near a tree). Relieves open-sun heat + a brain input to seek it.
        let shade01 = if nearest_tree_d2.is_finite() {
            (1.0 - nearest_tree_d2.sqrt() / SHADE_RADIUS).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // nearest THREAT: a bigger-combat creature nearby drives flee behavior. O(n) over the snapshot.
        let my_combat = genome.bite + SIZE_COMBAT * genome.size;
        let mut threat_d2 = f32::INFINITY;
        let mut threat_pos = Vec3::ZERO;
        for (e2, p2, _, c2) in &cre_snap {
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

        // build inputs from the EVOLVABLE sensors: each is a directional eye that reports nearest food
        // in its cone (+ what type). The GA decides how many sensors + where they point.
        let mut input: Vec<f32> = Vec::with_capacity(n_s * SIG_PER_SENSOR + GLOBAL_INPUTS);
        for si in 0..n_s {
            if sd[si].is_finite() {
                input.push(1.0 / (1.0 + sd[si])); // inv-distance in this sensor's cone
                // food TYPE (family) the eye sees, normalized -1..1. Digestibility now depends on the food's
                // nutrient profile vs the gut's uptake genes (not on kind), so the eye reports identity; the
                // brain + lifetime learning associate types with good/bad outcomes.
                input.push((skind[si] as f32 / (ntypes.max(2) - 1) as f32) * 2.0 - 1.0);
            } else {
                input.push(0.0);
                input.push(0.0);
            }
        }
        input.push(energy.total() / START_ENERGY);
        input.push(light * 2.0 - 1.0); // daylight signal (-1 night .. +1 noon): brain can time rest by it
        input.push(diet.fatigue * 2.0 - 1.0); // exertion debt: lets the brain "feel tired" and choose to rest
        input.push(1.0); // bias
        // M4 global inputs (order must match GLOBAL_INPUTS + ensure_net_shape padding):
        input.push((diet.toxic_load / TOX_LOAD_CAP).clamp(0.0, 1.0)); // own toxic load -> avoid poison
        input.push(shade01); // overhead canopy shade -> seek it in heat
        input.push(threat_dist); // nearest bigger-predator inv-distance -> flee
        input.push(threat_bear); // bearing to that predator (-1..1) -> which way to flee
        input.push(wet_here); // submersion / in-water

        // think (per-life learned brain, dynamic topology matching this genome's sensor count)
        let (h, out) = forward(&brain.net, &input);
        // power clamp: thrust limited by instantly-available power (fat mobilizes slow) -> a fat-only or
        // near-starved creature can't sprint. This is the "slow burning" teeth of the fat store.
        let power_frac = (energy.power() / MOVE_POWER_REF).clamp(0.0, 1.0);
        let thrust = out[0] * power_frac;
        let turn = out[1];
        // fatigue saps usable output (tired = sluggish); intended effort still costs full MOVE_COST below,
        // so flailing while exhausted is a net loss -> resting to recover is the only way out.
        let move_thrust = thrust * (1.0 - FATIGUE_DRAG * diet.fatigue);
        // metabolic tempo: frugal (metab>0.5) trades top speed for cheaper basal; fast (metab<0.5) the reverse
        let metab_f = genome.metab - 0.5; // -0.5 fast .. +0.5 frugal
        let speed = MOVE_SPEED
            * (1.0 + SWIM_SPEED * genome.swim * wet_here) // swimmers fast in water
            * (1.0 + LIMB_TRACTION * genome.limbs * (1.0 - wet_here)) // more legs = land traction (ground speed)
            * (1.0 - 0.5 * metab_f);

        // act: turn, then take a great-circle step along the heading over the planet surface
        head.0 = wrap_angle(head.0 + turn * TURN_SPEED * dt);
        let (nd, nh) = crate::sphere::step(pos, head.0, move_thrust * speed * dt);
        head.0 = nh; // heading parallel-transported into the new tangent frame
        let np = crate::sphere::surface_pos(nd, CREATURE_Y); // ride the terrain surface
        // pay for elevation change (P3): uphill costs, downhill partially refunds
        let h1 = crate::sphere::elevation(nd);
        let dh = h1 - h0;
        let climb = if dh > 0.0 { CLIMB_COST * dh } else { DESCEND_REFUND * dh };
        if climb > 0.0 {
            energy.burn(climb); // uphill burns
        } else {
            energy.add_fast(-climb, FAST_CAP); // downhill coasting refunds a little quick energy
        }
        loco.path += np.distance(pos); // accumulate 3D distance walked (diagnostic)
        ct.translation = np;
        // orient the whole body: local +Y = surface normal (stand upright), local +Z = travel direction
        // (head/eyes sit at +Z) so the creature FACES where it walks instead of yawing arbitrarily.
        let up = nd; // outward surface normal (unit)
        let fwd = crate::sphere::heading_tangent(nd, nh); // unit tangent along the heading
        let right = up.cross(fwd).normalize_or_zero();
        ct.rotation = Quat::from_mat3(&Mat3::from_cols(right, up, fwd));

        // metabolism: basal + movement (convex in speed) + bite upkeep + rocky crossing + vision upkeep.
        // Longer/more sensors see farther but cost energy (SENSE_COST x total range) -> range is a trade-off.
        let rock = crate::sphere::rockiness(nd);
        let lifespan_mult = 0.4 + 1.2 * genome.longevity; // 0.5 -> 1.0 (baseline); used for upkeep + aging
        let sense_range: f32 = genome.sensors.iter().map(|s| s.range).sum();
        let fat_frac = energy.fat / fat_max.max(0.01); // 0..1 how full the fat store is (drives upkeep)
        // thermal niche split into cold + warm sides so the pelt (fur) insulates the COLD side only.
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
            + LONGEVITY_COST * (lifespan_mult - 1.0).max(0.0) // a long-lived body costs more to maintain
            + SWIM_LAND_COST * genome.swim * (1.0 - wet_here) // fins are a liability on dry land
            + WATER_PRESSURE_COST * (1.0 - genome.swim) * (-h1 / crate::sphere::SEA_FLOOR_MAX).clamp(0.0, 1.0) // non-swimmers struggle in deep water (depth pressure)
            + FAT_UPKEEP * genome.adiposity * fat_frac // carrying fat costs upkeep (no free lunch)
            + STRESS_COST * diet.fatigue)
            * dt);
        // fast store leaks even at rest (volatile -> can't bank quick energy, use-it-or-lose-it)
        energy.fast = (energy.fast - FAST_LEAK * dt).max(0.0);
        // fatigue dynamics: exertion (thrust) accrues debt; idling (low thrust) sheds it. Clamped 0..1.
        diet.fatigue = (diet.fatigue + (FATIGUE_GAIN * thrust - FATIGUE_REST * (1.0 - thrust)) * dt).clamp(0.0, 1.0);
        // wildfire: standing in fire burns energy fast (deadly to anything caught in a blaze)
        let here_fire = fire.get(np);
        if here_fire > 0.05 {
            energy.burn(FIRE_DAMAGE * here_fire * dt);
        }
        // social/kin need: a social creature isolated from genetic kin drains energy (loneliness). Being
        // in a herd of kin removes the drain (and grants predation safety, see predation_step). The drain
        // is SCALED BY POPULATION DENSITY (pop/CREATURE_CAP): at a healthy density social creatures must
        // herd, but when the population is sparse the pressure relaxes -> no Allee death-spiral (a constant
        // loneliness drain on a spread-out population feeds back to extinction; this self-limits it).
        if genome.social > 0.0 {
            let kinf = kin_fraction(entity, np, &signature(genome), &cre_snap);
            let density = (pop as f32 / CREATURE_CAP as f32).min(1.0);
            energy.burn(SOCIAL_COST * genome.social * (1.0 - kinf) * density * dt);
        }

        // eat nearest plant on contact, IF bite beats its defense (arms race, see 13)
        let mut eat_reward = 0.0;
        if let Some((i, _)) = best {
            let (e, fp, mass) = (foods[i].0, foods[i].1, foods[i].3);
            let pg = foods[i].2.clone();
            let rot_age = foods[i].4;
            let tree = foods[i].5; // None=plant/carrion, Some(true)=fruit tree, Some(false)=evergreen
            let ferment = foods[i].6; // Some(toxic) -> fermenting plant matter (fruit/detritus); None+rot -> meat
            let seed = foods[i].7.clone(); // Some(genome) -> fallen fruit carrying a viable seed (planted if eaten ripe)
            if np.distance(fp) < EAT_RADIUS {
                // trees: a creature reaches an EDIBLE tree if its height + TREE_REACH_MARGIN, EXTENDED by
                // the tree's branches (BRANCH_REACH), >= the tree's height. So a tall bare tree feeds only
                // tall creatures, but a branchy one hangs fruit low enough for short creatures too.
                // Evergreens never eatable. Plants/carrion: bite vs defense as usual.
                let tree_reach = genome.height + TREE_REACH_MARGIN + pg.branches * BRANCH_REACH + CLIMB_REACH * genome.climb;
                let success = match tree {
                    Some(edible) => edible && tree_reach >= pg.height,
                    // plant: creature must be tall enough to reach it (height defense) AND bite its defense
                    None => {
                        genome.height + 0.15 >= pg.height
                            && rng.f32() < sigmoid(BITE_K * (genome.bite - pg.defense))
                    }
                };
                if success {
                    // digestion efficiency = the MASTER expression gene (reserves vs uptake demand). Gates
                    // energy extracted from ALL food in diet mode; legacy --no-diet runs ungated (eff=1).
                    let eff = if gen.diet { master_expression(&genome.uptake, &diet.reserves, RESERVE_REQ, MASTER_FLOOR) } else { 1.0 };
                    let fert = soil.get(np);
                    let soil_f = 1.0 - SOIL_NUTRI + SOIL_NUTRI * (fert / FERT_CAP).min(1.0); // richer soil -> more nutrients delivered
                    if let Some(true) = tree {
                        // FRUIT TREE: the tree persists + regrows; dies only if grazed below TREE_MIN_MASS.
                        // Mass dilutes nutrition (TREE_MASS_NUTRI): a bulkier tree gives less energy/bite.
                        // A creature too SHORT to reach without branches (reaching only via the low branches)
                        // feeds HARMLESSLY -> 0 mass damage; a creature tall enough to reach the crown strips
                        // mass and can over-graze. Either way the feeding is recorded (triggers dispersal).
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
                        // FERMENTING PLANT MATTER (fallen fruit / detritus): a 3-stage clock over ROT_GONE.
                        //   fresh (< FERMENT_START)        -> SUGAR (just ripe fruit/greens) + nutrients
                        //   fermenting (START..END)        -> FAST energy (ethanol), few nutrients + toxicity
                        //   spoiled (>= END)               -> near-zero yield, full toxicity (avoid)
                        // fruit (low `toxic`) ferments richly; detritus (high `toxic`) gives scraps + poison.
                        let f = (age as f32 / ROT_GONE as f32).clamp(0.0, 1.0);
                        let base = mass * pg.nutrient * (0.5 + pg.quality);
                        // fast-energy yield is whichever source this is (fruit rich, detritus poor)
                        let fast_gain = if pg.nutrient >= DETRITUS_NUTRIENT + 0.05 { FRUIT_FAST_GAIN } else { DETRITUS_FAST_GAIN };
                        if f < FERMENT_START {
                            if seed.is_some() {
                                // FRUIT, pre-ferment: a RIPENESS gradient. r=0 just-dropped (unripe) .. 1 ripe at
                                // RIPEN_FRAC. Unripe = little sugar + the genetic fruit_toxicity (`toxic`); ripening
                                // sweetens + detoxifies, so ripe fruit is sweet + safe EVEN IF the plant body is toxic
                                // (fruit toxicity decoupled from body). Eating unripe is bitter/poisonous -> the brain
                                // learns to wait, and the despawn block destroys the unripe seed (no reproduction).
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
                            // fresh detritus/greens (no seed): a sugary plant (gated by eff + tops nutrient reserves)
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
                            // ferment-ness ramps 0->1 across the window, then spoilage kills the yield. Ethanol
                            // is empty calories: FAST energy, no nutrient reserves (fermentation degrades them).
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
                        // CARRION / MEAT (P3): eating another creature = TOP nutrition, near-zero toxicity
                        // while fresh, and richer + longer-lasting than plants (MEAT_BONUS). Toxin only
                        // ramps once well-rotted (>60%). Not gated by master expr; balanced for reserves.
                        let f = (age as f32 / ROT_GONE as f32).clamp(0.0, 1.0); // 0 fresh .. 1 rotten
                        let freshness = 1.0 - (f / 0.6).min(1.0); // stays ~1 for the first 60% of decomposition
                        let meat = mass * pg.nutrient * freshness;
                        let toxin = TOXIN_MAX * ((f - 0.6) / 0.4).max(0.0); // no toxin until 60% rotted
                        // carnivory gut: a herbivore extracts little usable energy from meat; a carnivore
                        // extracts most. RABBIT STARVATION: processing protein WITHOUT a carb/fat buffer
                        // (empty sugar store) makes metabolic toxic load (ammonia) -> an all-meat creature
                        // with no plant carbs slowly poisons + starves even while "full".
                        let usable = (PROTEIN_FLOOR + PROTEIN_CARN * genome.carnivory).min(1.0);
                        let carb_buffer = (energy.sugar / SUGAR_CAP).clamp(0.0, 1.0); // carbs on hand BEFORE this meal
                        // meat = animal tissue -> mostly fat, some sugar; fat overflow spills to sugar
                        let gain = EAT_GAIN * MEAT_BONUS * meat * usable;
                        let overflow_fat = energy.add_fat(gain * 0.8, fat_max);
                        let wasted = energy.add_sugar(gain * 0.2 + overflow_fat, SUGAR_CAP, fat_max);
                        diet.g += wasted * OVEREAT_G;
                        fit.0 += meat * MEAT_BONUS * usable;
                        // balanced animal tissue: tops EVERY nutrient reserve (meat is nutritionally complete)
                        if gen.diet {
                            for r in diet.reserves.iter_mut() {
                                *r = (*r + MEAT_RESERVE * freshness).min(RESERVE_CAP);
                            }
                        }
                        // rot toxin + protein-without-carbs ammonia -> accumulating toxic load
                        diet.toxic_load = (diet.toxic_load
                            + toxin * TOX_LOAD_GAIN
                            + gain * PROTEIN_TOX * (1.0 - carb_buffer))
                            .min(TOX_LOAD_CAP);
                        eat_reward = freshness * 2.0 - 1.0; // fresh -> +1 (good), rotten -> -1 (avoid)
                    } else {
                        // regular plant: strip a fraction set by `regrow` -- carrot (~whole) vs berry bush
                        // (small bite, persists). Recorded as grazing; plant_step reduces mass / despawns.
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
                    // overeating trade-off (12): each store has a cap; eating past it converts the excess
                    // into growth-load (harm, via the add_* helpers above) -> gorging shortens life.
                    // carrion is eaten whole (despawn). Plants + trees PERSIST -- their mass is reduced by
                    // the grazing recorded above, and plant_step despawns any grazed below its min mass.
                    if tree.is_none() {
                        eaten.insert(e); // prevent same-tick re-eat
                        if let Some(seed_g) = seed {
                            // FALLEN FRUIT: eaten whole (despawn). If it was RIPE (past RIPEN_FRAC) the seed is
                            // viable -> it passes through the eater + is planted nearby (endozoochory). Eaten
                            // UNRIPE the seed isn't developed -> nothing is planted (eating early = no reproduction).
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
                            // endozoochory (13): grazing a living plant may disperse a mutated offspring
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

        // passive grass graze: a thin FALLBACK. Grass is render-only (not in the food scan), so its
        // edibility is modeled by POSITION: a HUNGRY creature standing on grass-bearing soil (plant-capable,
        // non-ocean land = where grass grows) nibbles a small sugar trickle. Hunger-gated so well-fed
        // creatures ignore it (no gorging), and it never distracts foraging (grass isn't sensed at all).
        if energy.total() < START_ENERGY {
            let gdir = np.normalize_or_zero();
            let hab = crate::sphere::plant_habitability(gdir);
            if !crate::sphere::is_ocean(gdir) && hab > GRASS_HAB_MIN {
                let gain = GRASS_GRAZE * hab * dt; // thin trickle, scaled by how grassy the ground is
                energy.add_sugar(gain, SUGAR_CAP, fat_max);
                fit.0 += gain * 0.1;
            }
        }

        // per-tick upkeep. Age every creature; diet mode runs the nutrient metabolism: gut-upkeep cost,
        // reserve depletion (you burn the nutrients you're built to use), and deficiency -> growth-load.
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
        // toxic load: clear slowly each tick (faster with the detox gene). While loaded it drains energy +
        // raises disease load; the acute death hazard is added in the mortality block. Runs in all modes so
        // meat/ferment poisons bite even in legacy --no-diet runs.
        if diet.toxic_load > 0.0 {
            let clear = (TOX_CLEAR_BASE + TOX_CLEAR_DETOX * genome.detox) * dt;
            diet.toxic_load = (diet.toxic_load - clear).max(0.0);
            energy.burn(TOX_LOAD_DRAIN * diet.toxic_load * dt);
            diet.g += TOX_LOAD_G * diet.toxic_load * dt;
        }
        // mortality from the diet model (aging + disease). In continuous mode death is otherwise
        // starvation-driven (density-dependent), which regulates the population logistically.
        if gen.diet {
            // senescence (both modes now): old-age hazard rises with age toward a ceiling, so creatures
            // have a real finite lifespan instead of living forever if well-fed. Ages are staggered (warmup
            // desync + spread-out births) so this does NOT sync-kill a cohort. Turnover keeps the gene pool
            // flowing (old die, young replace) -> a true life cycle to watch.
            let age_frac = diet.age as f32 / (AGE_SCALE * lifespan_mult); // longevity gene stretches lifespan
            let aging = AGE_HAZARD * (age_frac / (age_frac + 1.0));
            let p_death = (aging + DISEASE_K * diet.g + TOX_LOAD_HAZARD * diet.toxic_load) * dt;
            if rng.f32() < p_death {
                alive.0 = false; // old-age / disease / poisoning death
            }
        }

        // learn: reward = approach shaping (base mode only) + eat signal. Tunes brain (04/09).
        // Poison/diet drop approach shaping (it rewards nearing ANY food, fighting selectivity).
        if gen.learn {
            let approach = if !gen.diet && brain.prev_dist.is_finite() && cur_dist.is_finite() {
                (brain.prev_dist - cur_dist).clamp(-1.0, 1.0) * R_APPROACH
            } else {
                0.0
            };
            let reward = approach + eat_reward;
            learn(&mut brain.net, &genome.plast, &input, &h, &out, reward, LEARN_RATE);
        }
        brain.prev_dist = cur_dist;

        // starvation death. Outright empty -> dead. Otherwise, a creature pinned BELOW the starvation floor
        // (clinging at near-zero, e.g. a grass-trickle zombie whose real upkeep it can't pay) dies after a
        // grace period; a forager that briefly dips low between meals climbs back above the floor and resets.
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

        // continuous reproduction: a well-fed creature spends energy to bud a mutated child nearby.
        // Energy cost is the trade-off (breeding vs survival); cap bounds the population.
        // r/K life-history (parental gene, 0.5 = current values): r = breed young/cheap/many small fragile
        // young; K = breed late/costly/few well-provisioned young. Density taper still bounds the population.
        let k = genome.parental;
        let repro_thr = REPRO_THRESHOLD * (0.8 + 0.4 * k);
        let repro_min_age = (REPRO_MIN_AGE as f32 * (0.6 + 0.8 * k)) as u32;
        if live_continuous
            && alive.0
            && energy.total() > repro_thr
            && diet.age > repro_min_age // newborns must establish before breeding (paces birth waves)
            && pop < CREATURE_CAP
            // density-dependent: breeding rate tapers to 0 as pop approaches cap -> population asymptotes
            // to carrying capacity instead of slamming the cap and crashing (no boom-bust overshoot).
            && rng.f32() < P_REPRO_CREATURE * (1.0 - pop as f32 / CREATURE_CAP as f32)
        {
            energy.burn(REPRO_COST * (0.7 + 0.6 * k)); // K-parents spend more per child
            // mating mode: cross with the nearest genetically-similar mate (assortative -> reproductive
            // isolation/speciation); fall back to single-parent budding if no compatible mate is nearby.
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
        }

        // died this tick (loop skips already-dead creatures at the top) -> drop carrion here, which
        // rots into poison (rot_step). Closes part of the nutrient loop: death feeds the food web (P3).
        if !alive.0 {
            spawn_carrion(&mut commands, ct.translation, CARRION_MASS);
            soil.add(ct.translation, DEATH_FERT); // death enriches the ground here
            // continuous (post-warmup): the corpse entity is gone (became carrion). Generational mode
            // and the warm-up keep it (Alive=false) to be recycled into the next generation.
            if live_continuous {
                commands.entity(entity).despawn();
                pop = pop.saturating_sub(1);
            }
        }
    }
    // Creature reseed floor (safety net, mirrors the plant PLANT_MIN floor): if continuous population
    // crashes toward extinction, spawn mutated offspring of a survivor so the world can't fully die.
    // Only fires near-extinction -> self-sustaining populations never touch it. Needs a survivor (pop>0).
    // scenario mode (ScenarioStats present): NO reseed floor -> the isolated cohort stays the only creatures
    // (rand_pos reseed would scatter strangers planet-wide + break cohort isolation).
    if scen.is_none() && live_continuous && pop > 0 && pop < CREATURE_MIN {
        if let Some(g) = sample_genome {
            for _ in 0..(CREATURE_MIN - pop) {
                let mut child = g.clone();
                child.mutate(&mut rng, MUT_RATE, MUT_STD);
                let p = rand_pos(&mut rng, CREATURE_Y);
                spawn_creature(&mut commands, child, p, &mut rng, BIRTH_ENERGY); // reseed: baseline provisioning
            }
        }
    }
    // eaten plants are despawned above; population is replenished by plant_step (reproduction).
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

// Per-creature FAT-store ceiling: adiposity sets the storage strategy, body size adds capacity.
// fat is the big bank (fast/sugar caps are fixed small/medium). Bigger + fattier bodies buffer famine
// but pay carrying upkeep (FAT_UPKEEP) + sluggishness (FAT_POWER) -> no free lunch.
fn fat_cap(g: &Genome) -> f32 {
    FAT_CAP * (0.4 + ADIPOSITY_CAP * g.adiposity) * (1.0 + SIZE_ENERGY * g.size)
}

// Compact phenotype signature for KIN similarity (diet + body traits). Two creatures are "kin" when
// their signatures are within SOCIAL_SIM -> drives flocking-by-species + the social need.
fn signature(g: &Genome) -> [f32; 10] {
    // diet + body identity; carnivory + pelt added (M4) so gut-axis + coat divergence speciate too.
    // Both default identical across an old seed (serde default), so they add 0 distance there -> no
    // change to an existing population, only newly-evolved divergence.
    [g.uptake[0], g.uptake[1], g.uptake[2], g.uptake[3], g.size, g.swim, g.light_pref, g.height, g.carnivory, g.pelt]
}
fn sig_dist(a: &[f32; 10], b: &[f32; 10]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum()
}

// Fraction of social satisfaction from nearby kin (0 isolated .. 1 fully in a herd), given a snapshot
// of (entity, pos, signature). Excludes self by entity id.
fn kin_fraction(me: Entity, pos: Vec3, sig: &[f32; 10], snap: &[(Entity, Vec3, [f32; 10], f32)]) -> f32 {
    let r2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
    let mut kin = 0.0f32;
    for (e, p, s, _) in snap {
        if *e == me {
            continue;
        }
        if pos.distance_squared(*p) < r2 && sig_dist(sig, s) < SOCIAL_SIM {
            kin += 1.0;
        }
    }
    (kin / SOCIAL_TARGET).min(1.0)
}

// --- generation boundary: select + reproduce ---
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
    // Best healthy snapshot seen this run (score, snapshot). --save writes THIS, not the final-tick
    // population: continuous pop oscillates (~13..72), so saving at an arbitrary end-tick can capture a
    // trough -> a near-empty seed that limps back up. Saving the peak gives a full, balanced living world.
    mut best: Local<Option<(f32, crate::persist::Snapshot)>>,
) {
    gen.tick = gen.tick.wrapping_add(1); // global clock: drives season (plant_step) + continuous timing

    // --- continuous mode (after warm-up): no generation boundary. Snapshot the ecosystem periodically;
    // stop headless at MAX_TICKS_HEADLESS or on extinction. Selection is emergent (live_step). ---
    if gen.continuous && gen.generation >= WARMUP_GENS {
        let pop = cq.iter().count();
        let done = gen.headless && (gen.tick >= gen.max_gens * GEN_TICKS || pop == 0);
        if gen.tick.is_multiple_of(CONT_LOG_TICKS) || done {
            let n = pop.max(1) as f32;
            let mut e = 0.0;
            let mut f = 0.0;
            let mut fa = 0.0; // mean fast store (only fed by fermented food -> >0 proves fermentation web works)
            let mut su = 0.0; // mean sugar store
            let mut ft = 0.0; // mean fat store
            let mut adp = 0.0; // mean adiposity gene
            let mut mast = 0.0; // mean master digestion expression (nutrient sufficiency 0..1)
            let mut brd = 0.0; // mean diet breadth = count of actively-absorbed nutrients (uptake > 0.4)
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
            // Track the best healthy snapshot for --save. Score = pop, gated on well-fed (avg energy >= 30)
            // so we never bank a starving crowd. Captured only when saving (snapshot clone is not free).
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
                // Prefer the best healthy snapshot seen; fall back to the final state if none qualified.
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

    // capture fitness-ranked survivors for --save (only when saving; cheap clone of POP genomes)
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

    // avg terrain elevation creatures occupy at gen end (P3 diagnostic): are they spreading over relief?
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
    let plant_n = pq.iter().len().max(1);
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

    // build next generation genomes
    let mut next: Vec<Genome> = Vec::with_capacity(n);
    for _ in 0..n {
        let parent = &elites[(rng.f32() * elite_count as f32) as usize % elite_count];
        let mut child = parent.clone();
        child.mutate(&mut rng, MUT_RATE, MUT_STD);
        next.push(child);
    }

    // overwrite creatures with next gen, reset state + reposition.
    // Brain relearns from scratch each life: w := child priors (learned weights NOT inherited).
    // Diet expression resets to the child's innate baseline (epigenetic state is not inherited).
    // The cohort about to ENTER continuous mode (this is the last warm-up reset) is desynchronized:
    // random starting energy + life-age so they don't all reach the breed threshold / min-age / death
    // at the same tick. Synchronized cohorts breed in one burst -> a newborn wave that starves together
    // -> boom-bust extinction. Staggering ages spreads births + deaths so the population can overlap.
    let desync = gen.continuous && gen.generation + 1 >= WARMUP_GENS;
    for ((mut t, mut energy, mut fit, mut head, mut alive, mut g, mut brain, mut diet, mut loco), child) in
        cq.iter_mut().zip(next)
    {
        brain.net = child.net.clone();
        brain.prev_dist = f32::INFINITY;
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
    // Plants are NOT reset at the creature-generation boundary: they live + evolve continuously
    // (their own GA in plant_step), so the food supply co-evolves across creature generations.

    if gen.headless && gen.generation >= gen.max_gens {
        // --save: persist the fitness-ranked survivors + current food web so the run can resume.
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
