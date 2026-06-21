// M1 foraging sim: creatures sense nearest food, a tiny NN drives thrust+turn, they eat,
// burn energy, starve; a generational GA selects by food eaten. Proof-of-life milestone (08).
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::components::{Alive, Brain, Creature, DietState, Energy, Fitness, Food, Heading, Locomotion, Rot, Tree};
use crate::genome::{forward, learn, Genome, CONE_HALF, GLOBAL_INPUTS, NFOOD, SIG_PER_SENSOR};
use crate::plant::{PlantGenome, PlantState, P_REPRO, PLANT_CAP, PLANT_MIN};
use crate::rng::Rng;

// Tuning constants live in config.rs; re-exported so existing `sim::FOO` refs still resolve.
pub use crate::config::*;

// Global daylight: 0 at midnight .. 1 at noon, cycling on the tick clock. (Spherical world uses the
// POSITIONAL sphere::daylight_at per creature/plant; this global form remains for whole-world stats/logs.)
pub fn daylight(tick: u32) -> f32 {
    0.5 - 0.5 * (tick as f32 / DAY_TICKS as f32 * std::f32::consts::TAU).cos()
}

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

// --- weather (rain cycle): advance rainfall + update the ground-water grid (sun dries, rain refills) ---
pub fn weather_step(
    gen: Res<GenState>,
    mut weather: ResMut<Weather>,
    mut gw: ResMut<GroundWater>,
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
    let dt = DT;
    let n = SOIL_RES;
    // lightning: during a storm, occasionally strike -> seek out DRY fuel (sample a few cells, ignite the
    // driest). At storm onset the ground is still dry before rain soaks in; rocky highland stays dry too.
    if weather.rain > LIGHTNING_RAIN && rng.f32() < P_LIGHTNING {
        let mut best = 0usize;
        let mut driest = f32::INFINITY;
        for _ in 0..12 {
            let c = (rng.f32() * (n * n) as f32) as usize % (n * n);
            let w = gw.get(cell_center(c));
            if w < driest {
                driest = w;
                best = c;
            }
        }
        if driest < FIRE_WET_MAX {
            fire.cell[best] = 1.0; // struck dry fuel -> it catches
        }
    }
    if fire.cell.iter().all(|&f| f <= 0.02) {
        return; // nothing burning, skip the sweep
    }
    let cur = fire.cell.clone(); // spread reads the pre-tick state
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
            if gw.get(cell_center(ni)) < FIRE_WET_MAX {
                fire.cell[ni] = (fire.cell[ni] + FIRE_SPREAD * f * dt).min(1.0);
            }
        }
    }
}

// The founding homeland: all initial life starts within HOMELAND_CAP radians of this direction, then
// spreads across the globe by reproduction/dispersal. (User: start them all in one area.)
pub fn homeland_center() -> Vec3 {
    Vec3::new(1.0, 0.35, 0.25).normalize()
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

fn diet_state(g: &Genome) -> DietState {
    DietState { expr: g.expr0, g: 0.0, age: 0, fatigue: 0.0 }
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
        },
        Rot { age: 0 },
        Transform::from_translation(p),
    ));
}

// Spawn one creature (no render mesh; viz::add_creature_visuals gives it one in render mode).
// Used for continuous-mode offspring; fresh brain from the genome's priors, learns over its own life.
fn spawn_creature(commands: &mut Commands, g: Genome, pos: Vec3, rng: &mut Rng) {
    let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
    let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
    let diet = diet_state(&g);
    commands.spawn((
        Creature,
        g,
        brain,
        diet,
        Energy(BIRTH_ENERGY),
        Fitness(0.0),
        Heading(h),
        Alive(true),
        Locomotion { start: pos, path: 0.0 },
        Transform::from_translation(pos),
    ));
}

// A fresh (founding) tree genome: rich, tall, slow, with some branches. defense ~1 is irrelevant to
// trees (reach gates them, not bite); kind 0. From here trees evolve via PlantGenome::mutate_tree.
fn tree_genome(rng: &mut Rng) -> PlantGenome {
    PlantGenome {
        kind: 0,
        nutrient: rng.range(0.6, 1.0),
        defense: 0.99,
        quality: rng.range(0.1, 0.4),
        wet: 0.5,
        height: rng.range(TREE_HEIGHT_MIN, 1.0), // wide initial height (never taller than 1.0)
        light_pref: rng.range(0.5, 0.9),
        regrow: rng.range(0.0, 0.4),
        branches: rng.range(0.0, 0.6),
        spread: rng.range(5.0, 9.0),
        maturity: TREE_MATURITY,
    }
}

// Spawn one tree (long-lived plant + Tree marker) from a genome. edible=true fruit tree, false=evergreen.
fn spawn_tree(commands: &mut Commands, mass: f32, pos: Vec3, edible: bool, g: PlantGenome) {
    commands.spawn((
        Food,
        Tree { edible },
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos),
    ));
}

// Spawn one plant (living food). No render mesh; add_plant_visuals (render mode) gives it one.
fn spawn_plant(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    commands.spawn((
        Food,
        PlantState { mass, age: 0 },
        g,
        Transform::from_translation(pos),
    ));
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
    let skip_warmup = gen.continuous && snap.as_ref().map_or(false, |s| !s.creatures.is_empty());
    if skip_warmup {
        gen.generation = WARMUP_GENS;
    }
    for g in genomes {
        let p = homeland_pos(&mut rng, CREATURE_Y); // founding pop starts in one region, then spreads
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
            Energy(e),
            Fitness(0.0),
            Heading(h),
            Alive(true),
            Locomotion { start: p, path: 0.0 },
            Transform::from_translation(p),
        ));
    }
    let ntypes = gen.ntypes();
    match &snap {
        Some(s) if !s.plants.is_empty() => {
            for sp in &s.plants {
                let p = homeland_pos(&mut rng, FOOD_Y);
                spawn_plant(&mut commands, sp.g.clone(), sp.mass, p);
            }
        }
        _ => {
            for _ in 0..FOOD {
                let p = homeland_pos(&mut rng, FOOD_Y);
                let pg = PlantGenome::random(&mut rng, ntypes);
                spawn_plant(&mut commands, pg, rng.range(0.3, 1.4) * PLANT_START_MASS, p); // varied mass desyncs the food supply
            }
        }
    }
    spawn_trees(&mut commands, &mut rng);
}

// Scatter the initial trees (half tall fruit trees, half uneatable evergreens) on habitable homeland.
fn spawn_trees(commands: &mut Commands, rng: &mut Rng) {
    for i in 0..N_TREES {
        let mut p = homeland_pos(rng, FOOD_Y);
        for _ in 0..6 {
            if crate::sphere::plant_habitability(p.normalize_or_zero()) > 0.4 {
                break;
            }
            p = homeland_pos(rng, FOOD_Y);
        }
        // alternate fruit tree / evergreen; each gets a fresh evolvable genome (evolves from here)
        let g = tree_genome(rng);
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
    // shared plant sphere mesh; viz::add_plant_visuals colors each plant by its genome
    commands.insert_resource(crate::viz::PlantMesh(meshes.add(Sphere::new(0.35))));
    commands.insert_resource(crate::viz::TreeMeshes {
        trunk: meshes.add(Cylinder::new(0.16, 3.0)),
        broadleaf: meshes.add(Sphere::new(1.3)),
        conifer: meshes.add(Cone { radius: 1.2, height: 3.2 }),
    });

    // --load resumes a saved population; otherwise a random founding pop. Positions re-randomized.
    let snap = gen.load.as_deref().and_then(crate::persist::load_snapshot);
    let genomes: Vec<Genome> = match &snap {
        Some(s) if !s.creatures.is_empty() => s.creatures.clone(),
        _ => (0..POP).map(|_| Genome::random(&mut rng)).collect(),
    };
    // loading a saved population into continuous mode skips warm-up (genomes already competent) ->
    // drop straight into a living world. Desync energy + age so they don't act in lockstep.
    let skip_warmup = gen.continuous && snap.as_ref().map_or(false, |s| !s.creatures.is_empty());
    if skip_warmup {
        gen.generation = WARMUP_GENS;
    }
    for g in genomes {
        let p = homeland_pos(&mut rng, CREATURE_Y); // founding pop starts in one region, then spreads
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
        let mut diet = diet_state(&g);
        if skip_warmup {
            diet.age = (rng.f32() * 600.0) as u32;
        }
        let e = if skip_warmup { rng.range(0.7, 1.2) * START_ENERGY } else { START_ENERGY };
        // own material per creature so viz can recolor it by evolved traits (see viz.rs)
        let mat = materials.add(Color::srgb(0.9, 0.6, 0.3));
        commands.spawn((
            Creature,
            g,
            brain,
            diet,
            Energy(e),
            Fitness(0.0),
            Heading(h),
            Alive(true),
            Locomotion { start: p, path: 0.0 },
            Mesh3d(creature_mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_translation(p),
        ));
    }
    let ntypes = gen.ntypes();
    match &snap {
        Some(s) if !s.plants.is_empty() => {
            for sp in &s.plants {
                let p = homeland_pos(&mut rng, FOOD_Y);
                spawn_plant(&mut commands, sp.g.clone(), sp.mass, p);
            }
        }
        _ => {
            for _ in 0..FOOD {
                let p = homeland_pos(&mut rng, FOOD_Y);
                let pg = PlantGenome::random(&mut rng, ntypes);
                spawn_plant(&mut commands, pg, rng.range(0.3, 1.4) * PLANT_START_MASS, p); // varied mass desyncs the food supply
            }
        }
    }
    spawn_trees(&mut commands, &mut rng);
}

// --- plants: grow, reproduce (disperse mutated offspring), reseed if the web nearly collapses (13) ---
pub fn plant_step(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
    mut soil: ResMut<Soil>,
    gw: Res<GroundWater>,
    fire: Res<Fire>,
    mut tree_bites: ResMut<TreeBites>,
    mut q: Query<(Entity, &mut PlantState, &PlantGenome, &Transform, Option<&Tree>), Without<Rot>>, // not carrion
) {
    soil.decay(); // fertility leaches / is taken up over time
    // season drifts on the global tick clock (advances in both modes; generation is frozen in continuous)
    let season = (gen.tick as f32 / GEN_TICKS as f32 * SEASON_FREQ).sin(); // -1 dry .. +1 wet
    let mut plant_count = q.iter().filter(|(.., t)| t.is_none()).count();
    let tree_count = q.iter().filter(|(.., t)| t.is_some()).count();
    let tree_positions: Vec<Vec3> = q.iter().filter_map(|(_, _, _, tf, t)| t.map(|_| tf.translation)).collect();
    let mut births: Vec<(PlantGenome, Vec3)> = Vec::new();
    let mut tree_births: Vec<(Vec3, bool, PlantGenome)> = Vec::new(); // (pos, edible, child genome)
    let mut detritus: Vec<(PlantGenome, f32, Vec3)> = Vec::new(); // moisture-killed plants -> poison
    for (e, mut st, g, tf, tree) in &mut q {
        let ppos = tf.translation;
        let pdir = ppos.normalize_or_zero();
        // wildfire: a plant/tree in a strongly-burning cell burns up (despawn). fire_step already laid
        // down ash fertility here, so burned ground regrows richer.
        if fire.get(ppos) > FIRE_KILL {
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
            // fed-on this tick? (key present even for harmless branch-feeders, who do 0 mass damage).
            // Apply the recorded mass damage; a fruit tree grazed below TREE_MIN_MASS is over-eaten -> dies.
            let grazed = tree_bites.0.contains_key(&e);
            if grazed {
                st.mass = (st.mass - tree_bites.0[&e]).max(0.0);
                if st.mass < TREE_MIN_MASS {
                    commands.entity(e).despawn();
                    continue;
                }
            }
            // trees: moisture-immune (long-lived), grow slowly + large. Two reproduction paths:
            //  - ambient: fertility-weighted, density-limited wind-fall near the parent;
            //  - dispersal-on-eat: a fruit tree that was grazed this tick may fling a seed FAR (animal-
            //    carried). Being reachable+eaten thus pays in reproduction -> bounds how tall trees evolve.
            st.mass += g.growth_rate() * boost * lf * TREE_GROWTH_SCALE * DT; // trees grow slowly (long-lived)
            st.age += 1;
            let r2 = TREE_DENSITY_R * TREE_DENSITY_R;
            let local = tree_positions.iter().filter(|p| p.distance_squared(tf.translation) < r2).count();
            let fert_boost = 0.4 + 1.6 * (fert / FERT_CAP).min(1.0); // richer ground -> more new trees
            let mature = st.mass >= g.maturity;
            let ambient = mature && local <= TREE_MAX_LOCAL && rng.f32() < P_TREE_REPRO * fert_boost;
            let disperse = mature && tree.edible && grazed && rng.f32() < P_TREE_EAT_DISPERSE; // seed carried off
            if (ambient || disperse) && tree_count + tree_births.len() < TREE_CAP {
                let spread = if disperse { g.spread * TREE_EAT_SPREAD_MULT } else { g.spread };
                let pos = disperse_pos(&mut rng, ppos, spread, FOOD_Y);
                // child inherits the parent's full tree genome, mutated (trees evolve like plants)
                let mut child = g.clone();
                child.mutate_tree(&mut rng);
                tree_births.push((pos, tree.edible, child));
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
                commands.entity(e).despawn();
                soil.add(ppos, DEATH_FERT * 0.3); // a consumed plant returns some nutrients to the ground
                plant_count = plant_count.saturating_sub(1);
                continue;
            }
        }
        // mortality from moisture mismatch OR a poor site (deep water / desert). Effective moisture =
        // static terrain moisture + rain-fed ground water -> wet-liking plants thrive after a downpour,
        // get stressed in drought (temporal selection on `wet`).
        let m = (crate::sphere::moisture(pdir) + 0.2 * season + WET_GAIN * water).clamp(0.0, 1.0);
        let stress = (m - g.wet).abs();
        let hab = crate::sphere::plant_habitability(pdir); // 0 in ocean/desert/cold, 1 on good land
        let p_mort =
            MOISTURE_KILL * (stress - MOISTURE_TOLERANCE).max(0.0) + HABITAT_KILL * (0.3 - hab).max(0.0);
        if rng.f32() < p_mort {
            commands.entity(e).despawn();
            detritus.push((g.clone(), st.mass, tf.translation));
            soil.add(ppos, DEATH_FERT * 0.3); // a dead plant enriches the ground where it falls
            plant_count = plant_count.saturating_sub(1);
            continue;
        }
        // fertile soil speeds growth (M5); scales with habitability (P3) and light match (day/night)
        st.mass += g.growth_rate() * boost * hab * lf * DT;
        st.age += 1;
        if st.mass >= g.maturity
            && plant_count + births.len() < PLANT_CAP
            && rng.f32() < P_REPRO * (1.0 - DEF_REPRO_COST * g.defense)
        {
            let mut child = g.clone();
            child.mutate(&mut rng);
            let pos = disperse_pos(&mut rng, ppos, g.spread, FOOD_Y);
            births.push((child, pos));
            st.mass *= PLANT_REPRO_FRAC;
        }
    }
    // dead plants -> poison detritus (completes the rot chain, P3): poor food that turns toxic, then gone
    for (g, mass, pos) in detritus {
        commands.spawn((
            Food,
            PlantState { mass: mass.min(CARRION_MASS), age: 0 },
            PlantGenome { nutrient: DETRITUS_NUTRIENT, defense: 0.0, quality: 0.0, ..g },
            Rot { age: 0 },
            Transform::from_translation(pos),
        ));
    }
    // reseed floor: keep a minimal seed bank so creatures can't drive food fully extinct
    let ntypes = gen.ntypes();
    while plant_count + births.len() < PLANT_MIN {
        births.push((PlantGenome::random(&mut rng, ntypes), rand_pos(&mut rng, FOOD_Y)));
    }
    for (g, pos) in births {
        spawn_plant(&mut commands, g, rng.range(0.5, 1.3) * PLANT_START_MASS, pos); // varied reseed mass (staggered maturity)
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
    // snapshot living creatures: (entity, pos, combat, energy, kin-signature). Combat = bite + size.
    let snap: Vec<(Entity, Vec3, f32, f32, [f32; 8])> = cq
        .iter()
        .filter(|(_, _, _, _, a, _)| a.0)
        .map(|(e, t, en, _, _, g)| (e, t.translation, g.bite + SIZE_COMBAT * g.size, en.0, signature(g)))
        .collect();
    if snap.len() < 2 {
        return;
    }
    let mut killed: HashSet<Entity> = HashSet::new();
    let mut gains: HashMap<Entity, f32> = HashMap::new();
    let r2 = ATTACK_RADIUS * ATTACK_RADIUS;
    let rs2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
    for (ai, &(ae, apos, abite, aenergy, _asig)) in snap.iter().enumerate() {
        if killed.contains(&ae) {
            continue; // a creature killed this tick doesn't also attack
        }
        // only a HUNGRY creature hunts -> well-fed crowds don't cannibalize each other (this was the
        // continuous-mode killer: at high density, packed well-fed creatures mass-cannibalized -> crash).
        if aenergy > PREDATION_HUNGER {
            continue;
        }
        let mut best: Option<(f32, usize)> = None;
        for (bi, &(be, bpos, _, _, _)) in snap.iter().enumerate() {
            if bi == ai || killed.contains(&be) {
                continue;
            }
            let d2 = apos.distance_squared(bpos);
            if d2 < r2 && best.map_or(true, |(bd, _)| d2 < bd) {
                best = Some((d2, bi));
            }
        }
        if let Some((_, bi)) = best {
            let (be, bpos, bbite, _, bsig) = snap[bi];
            // herd safety: prey surrounded by KIN is harder to pick off (vigilance) -> being social pays
            let mut kin = 0.0f32;
            for (e2, p2, _, _, s2) in &snap {
                if *e2 != be && bpos.distance_squared(*p2) < rs2 && sig_dist(&bsig, s2) < SOCIAL_SIM {
                    kin += 1.0;
                }
            }
            let prey_kin = (kin / SOCIAL_TARGET).min(1.0);
            // success = attacker combat vs prey combat (bite doubles as defense), minus a required edge
            // (PREDATION_BIAS: equal-combat creatures barely prey on each other), reduced by herd safety
            if rng.f32() < sigmoid(BITE_K * (abite - bbite) - PREDATION_BIAS) * (1.0 - SOCIAL_SAFETY * prey_kin) {
                killed.insert(be);
                *gains.entry(ae).or_insert(0.0) += PREDATION_GAIN;
            }
        }
    }
    if killed.is_empty() {
        return;
    }
    let continuous_live = gen.continuous && gen.generation >= WARMUP_GENS;
    for (e, t, mut energy, mut fit, mut alive, gen_e) in &mut cq {
        if let Some(g) = gains.get(&e) {
            energy.0 = (energy.0 + g).min(energy_max(gen_e));
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
    fq: Query<(Entity, &Transform, &PlantState, &PlantGenome, Option<&Rot>, Option<&Tree>), (With<Food>, Without<Creature>)>,
) {
    let dt = DT;
    let ntypes = gen.ntypes();
    let mut pop = cq.iter().count(); // live population (for the continuous-mode reproduction cap)
    // continuous birth/death is active only AFTER the generational warm-up (see WARMUP_GENS)
    let live_continuous = gen.continuous && gen.generation >= WARMUP_GENS;
    // (daylight is now POSITIONAL: computed per-creature from its location, see `light` inside the loop)
    // snapshot: (entity, pos, genome, mass, rot_age, tree). rot_age=Some -> carrion; tree=Some(edible) ->
    // a tree (edible fruit tree if true, uneatable evergreen if false); else a living plant.
    let foods: Vec<(Entity, Vec3, PlantGenome, f32, Option<u32>, Option<bool>)> = fq
        .iter()
        .map(|(e, t, st, pg, rot, tree)| {
            (e, t.translation, pg.clone(), st.mass, rot.map(|r| r.age), tree.map(|t| t.edible))
        })
        .collect();
    let mut eaten: HashSet<Entity> = HashSet::new();
    let mut sample_genome: Option<Genome> = None; // a living genome, for the near-extinction reseed floor
    // creature snapshot for the social/kin need: (entity, pos, phenotype signature)
    let cre_snap: Vec<(Entity, Vec3, [f32; 8])> = cq
        .iter()
        .filter(|(_, _, _, _, _, a, _, _, _, _)| a.0)
        .map(|(e, t, _, _, _, _, g, _, _, _)| (e, t.translation, signature(g)))
        .collect();
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

        // SINGLE grid-bounded pass over nearby foods (perf): scan only the food-grid cells within the
        // query radius (>= max sensor range, so every sensable food is covered; >= NEAR_QUERY so the
        // global-nearest is found given dense plants). Computes the global nearest (approach/eat) AND each
        // sensor's nearest-food-in-cone together, atan2 once per in-range food. Not bit-identical (cell
        // iteration order differs), but far fewer foods touched -> big speedup at scale.
        let n_s = genome.sensors.len();
        let max_range = genome.sensors.iter().map(|s| s.range).fold(0.0f32, f32::max);
        let mut best: Option<(usize, f32)> = None;
        let mut sd = vec![f32::INFINITY; n_s]; // nearest dist per sensor
        let mut skind = vec![0u8; n_s]; // food kind that nearest sensor-food is
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
                    if best.map_or(true, |(_, bd2)| d2 < bd2) {
                        best = Some((i, d2));
                    }
                    let dist = d2.sqrt();
                    if dist > max_range {
                        continue; // out of every sensor's range -> skip the bearing + cone tests
                    }
                    // bearing in the local tangent frame: 0 = north (toward +Y pole), +pi/2 = east
                    let bearing = wrap_angle(to.dot(east).atan2(to.dot(north)) - head.0);
                    for (si, s) in genome.sensors.iter().enumerate() {
                        if dist <= s.range && dist < sd[si] && wrap_angle(bearing - s.angle).abs() <= CONE_HALF {
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

        // build inputs from the EVOLVABLE sensors: each is a directional eye that reports nearest food
        // in its cone (+ what type). The GA decides how many sensors + where they point.
        let mut input: Vec<f32> = Vec::with_capacity(n_s * SIG_PER_SENSOR + GLOBAL_INPUTS);
        for si in 0..n_s {
            if sd[si].is_finite() {
                input.push(1.0 / (1.0 + sd[si])); // inv-distance in this sensor's cone
                let sig = if gen.diet {
                    diet.expr[skind[si] as usize] // readiness to digest what this eye sees
                } else {
                    (skind[si] as f32 / (ntypes.max(2) - 1) as f32) * 2.0 - 1.0 // food type
                };
                input.push(sig);
            } else {
                input.push(0.0);
                input.push(0.0);
            }
        }
        input.push(energy.0 / START_ENERGY);
        input.push(light * 2.0 - 1.0); // daylight signal (-1 night .. +1 noon): brain can time rest by it
        input.push(diet.fatigue * 2.0 - 1.0); // exertion debt: lets the brain "feel tired" and choose to rest
        input.push(1.0); // bias

        // think (per-life learned brain, dynamic topology matching this genome's sensor count)
        let (h, out) = forward(&brain.net, &input);
        let thrust = out[0];
        let turn = out[1];
        // fatigue saps usable output (tired = sluggish); intended effort still costs full MOVE_COST below,
        // so flailing while exhausted is a net loss -> resting to recover is the only way out.
        let move_thrust = thrust * (1.0 - FATIGUE_DRAG * diet.fatigue);
        // aquatic factor at this spot: 1 in water / wet lowland, 0 on high dry ground. Swimmers move
        // faster here (fins) and pay a penalty on dry land (see metabolism) -> a wetland/fish niche.
        // aquatic factor at this spot: 1 in ocean / wet lowland, 0 on high dry ground (low elevation = wet)
        let h0 = crate::sphere::elevation(pdir);
        let wet_here = ((SWIM_WET_LEVEL - h0) / SWIM_WET_LEVEL).clamp(0.0, 1.0);
        let speed = MOVE_SPEED * (1.0 + SWIM_SPEED * genome.swim * wet_here);

        // act: turn, then take a great-circle step along the heading over the planet surface
        head.0 = wrap_angle(head.0 + turn * TURN_SPEED * dt);
        let (nd, nh) = crate::sphere::step(pos, head.0, move_thrust * speed * dt);
        head.0 = nh; // heading parallel-transported into the new tangent frame
        let np = crate::sphere::surface_pos(nd, CREATURE_Y); // ride the terrain surface
        // pay for elevation change (P3): uphill costs, downhill partially refunds
        let h1 = crate::sphere::elevation(nd);
        let dh = h1 - h0;
        energy.0 -= if dh > 0.0 { CLIMB_COST * dh } else { DESCEND_REFUND * dh };
        loco.path += np.distance(pos); // accumulate 3D distance walked (diagnostic)
        ct.translation = np;
        ct.rotation = Quat::from_rotation_arc(Vec3::Y, nd); // stand upright on the sphere (long axis = surface normal)

        // metabolism: basal + movement (convex in speed) + bite upkeep + rocky crossing + vision upkeep.
        // Longer/more sensors see farther but cost energy (SENSE_COST x total range) -> range is a trade-off.
        let rock = crate::sphere::rockiness(nd);
        let sense_range: f32 = genome.sensors.iter().map(|s| s.range).sum();
        energy.0 -= (BASAL_COST
            + SIZE_BASAL * genome.size // bigger body costs more just to maintain
            + MOVE_COST * (1.0 + SIZE_MOVE * genome.size) * thrust * thrust // more mass to push
            + BITE_COST * genome.bite
            + ROCK_MOVE_COST * rock * thrust.abs()
            + SENSE_COST * sense_range
            + BRAIN_COST * genome.net.ih.len() as f32 // bigger brain = more upkeep
            + HEIGHT_COST * genome.height
            + LIGHT_COST * (light - genome.light_pref).abs() // global daylight (clouds don't perturb the creature cost: unpredictable local light destabilized populations)
            + SWIM_LAND_COST * genome.swim * (1.0 - wet_here) // fins are a liability on dry land
            + STRESS_COST * diet.fatigue)
            * dt;
        // fatigue dynamics: exertion (thrust) accrues debt; idling (low thrust) sheds it. Clamped 0..1.
        diet.fatigue = (diet.fatigue + (FATIGUE_GAIN * thrust - FATIGUE_REST * (1.0 - thrust)) * dt).clamp(0.0, 1.0);
        // wildfire: standing in fire burns energy fast (deadly to anything caught in a blaze)
        let here_fire = fire.get(np);
        if here_fire > 0.05 {
            energy.0 -= FIRE_DAMAGE * here_fire * dt;
        }
        // social/kin need: a social creature isolated from genetic kin drains energy (loneliness). Being
        // in a herd of kin removes the drain (and grants predation safety, see predation_step). The drain
        // is SCALED BY POPULATION DENSITY (pop/CREATURE_CAP): at a healthy density social creatures must
        // herd, but when the population is sparse the pressure relaxes -> no Allee death-spiral (a constant
        // loneliness drain on a spread-out population feeds back to extinction; this self-limits it).
        if genome.social > 0.0 {
            let kinf = kin_fraction(entity, np, &signature(genome), &cre_snap);
            let density = (pop as f32 / CREATURE_CAP as f32).min(1.0);
            energy.0 -= SOCIAL_COST * genome.social * (1.0 - kinf) * density * dt;
        }

        // eat nearest plant on contact, IF bite beats its defense (arms race, see 13)
        let mut eat_reward = 0.0;
        if let Some((i, _)) = best {
            let (e, fp, mass) = (foods[i].0, foods[i].1, foods[i].3);
            let pg = foods[i].2.clone();
            let rot_age = foods[i].4;
            let tree = foods[i].5; // None=plant/carrion, Some(true)=fruit tree, Some(false)=evergreen
            if np.distance(fp) < EAT_RADIUS {
                // trees: a creature reaches an EDIBLE tree if its height + TREE_REACH_MARGIN, EXTENDED by
                // the tree's branches (BRANCH_REACH), >= the tree's height. So a tall bare tree feeds only
                // tall creatures, but a branchy one hangs fruit low enough for short creatures too.
                // Evergreens never eatable. Plants/carrion: bite vs defense as usual.
                let tree_reach = genome.height + TREE_REACH_MARGIN + pg.branches * BRANCH_REACH;
                let success = match tree {
                    Some(edible) => edible && tree_reach >= pg.height,
                    // plant: creature must be tall enough to reach it (height defense) AND bite its defense
                    None => {
                        genome.height + 0.15 >= pg.height
                            && rng.f32() < sigmoid(BITE_K * (genome.bite - pg.defense))
                    }
                };
                if success {
                    if let Some(true) = tree {
                        // FRUIT TREE: the tree persists + regrows; dies only if grazed below TREE_MIN_MASS.
                        // Mass dilutes nutrition (TREE_MASS_NUTRI): a bulkier tree gives less energy/bite.
                        // A creature too SHORT to reach without branches (reaching only via the low branches)
                        // feeds HARMLESSLY -> 0 mass damage; a creature tall enough to reach the crown strips
                        // mass and can over-graze. Either way the feeding is recorded (triggers dispersal).
                        let bite_mass = TREE_BITE_MASS.min(mass);
                        let mass_nutri = 1.0 - TREE_MASS_NUTRI * (mass / TREE_MATURITY).min(1.0);
                        let base = bite_mass * pg.nutrient * mass_nutri * (0.5 + pg.quality);
                        let eff = if gen.diet { diet.expr[pg.kind as usize] } else { 1.0 };
                        energy.0 += EAT_GAIN * base * eff;
                        fit.0 += base * eff;
                        let short = genome.height + TREE_REACH_MARGIN < pg.height; // only reached via branches
                        let damage = if short { 0.0 } else { bite_mass }; // branch-feeders don't harm the tree
                        *tree_bites.0.entry(e).or_insert(0.0) += damage;
                        eat_reward = R_EAT;
                    } else if let Some(age) = rot_age {
                        // CARRION / MEAT (P3): eating another creature = TOP nutrition, near-zero toxicity
                        // while fresh, and richer + longer-lasting than plants (MEAT_BONUS). Toxin only
                        // ramps once well-rotted (>60%). Not gated by diet expr; never disperses seeds.
                        let f = (age as f32 / ROT_GONE as f32).clamp(0.0, 1.0); // 0 fresh .. 1 rotten
                        let freshness = 1.0 - (f / 0.6).min(1.0); // stays ~1 for the first 60% of decomposition
                        let meat = mass * pg.nutrient * freshness;
                        let toxin = TOXIN_MAX * ((f - 0.6) / 0.4).max(0.0); // no toxin until 60% rotted
                        energy.0 += EAT_GAIN * MEAT_BONUS * meat;
                        fit.0 += meat * MEAT_BONUS;
                        energy.0 -= toxin;
                        diet.g += toxin * TOXIN_G;
                        eat_reward = freshness * 2.0 - 1.0; // fresh -> +1 (good), rotten -> -1 (avoid)
                    } else {
                        // regular plant: strip a fraction set by `regrow` -- carrot (~whole) vs berry bush
                        // (small bite, persists). Recorded as grazing; plant_step reduces mass / despawns.
                        let frac = (1.0 - 0.85 * pg.regrow).clamp(0.12, 1.0);
                        let bite_mass = mass * frac;
                        *tree_bites.0.entry(e).or_insert(0.0) += bite_mass;
                        // quality scales extractable energy: factor 0.5..1.5, ~1.0 at quality 0.5 (balance-neutral)
                        let base = bite_mass * pg.nutrient * (0.5 + pg.quality);
                        if gen.diet {
                            let t = pg.kind as usize;
                            let eff = diet.expr[t];
                            energy.0 += EAT_GAIN * base * eff;
                            fit.0 += base * eff;
                            let rate = 1.0 - genome.rigidity;
                            diet.expr[t] += EXPR_RAMP * rate * (1.0 - diet.expr[t]);
                            for j in 0..NFOOD {
                                if j != t {
                                    diet.expr[j] -= EXPR_DECAY * rate * diet.expr[j];
                                }
                            }
                            if eff < 0.5 {
                                diet.g += G_GAIN * (0.5 - eff);
                                energy.0 -= MISMATCH_STRESS;
                            }
                            eat_reward = (eff - 0.5) * 4.0;
                        } else {
                            energy.0 += EAT_GAIN * base;
                            fit.0 += base;
                            eat_reward = R_EAT;
                        }
                    }
                    // overeating trade-off (12): energy is capped; eating while already full converts
                    // the excess into growth-load (harm) -> gorging shortens life. Eat best, in moderation.
                    let emax = energy_max(genome);
                    if energy.0 > emax {
                        let excess = energy.0 - emax;
                        energy.0 = emax;
                        diet.g += excess * OVEREAT_G;
                    }
                    // carrion is eaten whole (despawn). Plants + trees PERSIST -- their mass is reduced by
                    // the grazing recorded above, and plant_step despawns any grazed below its min mass.
                    if tree.is_none() {
                        eaten.insert(e); // prevent same-tick re-eat
                        if rot_age.is_some() {
                            commands.entity(e).despawn(); // carrion consumed
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

        // per-tick upkeep. Age every creature; diet mode adds expression overhead + growth-load decay.
        diet.age += 1;
        if gen.diet {
            let total_expr: f32 = diet.expr.iter().sum();
            energy.0 -= EXPR_OVERHEAD * total_expr * dt; // cost of keeping many genes expressed
            diet.g = (diet.g - G_DECAY).max(0.0);
        }
        // mortality from the diet model (aging + disease). In continuous mode death is otherwise
        // starvation-driven (density-dependent), which regulates the population logistically.
        if gen.diet {
            // aging only in generational mode (fixed lifespan -> ~95 plateau). In continuous it would
            // sync-kill the warm-up cohort; there death is starvation + disease (density-regulated).
            let aging = if gen.continuous {
                0.0
            } else {
                let age_frac = diet.age as f32 / AGE_SCALE;
                AGE_HAZARD * (age_frac / (age_frac + 1.0))
            };
            let p_death = (aging + DISEASE_K * diet.g) * dt;
            if rng.f32() < p_death {
                alive.0 = false; // old-age / disease death
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

        if energy.0 <= 0.0 {
            alive.0 = false;
        }

        // continuous reproduction: a well-fed creature spends energy to bud a mutated child nearby.
        // Energy cost is the trade-off (breeding vs survival); cap bounds the population.
        if live_continuous
            && alive.0
            && energy.0 > REPRO_THRESHOLD
            && diet.age > REPRO_MIN_AGE // newborns must establish before breeding (paces birth waves)
            && pop < CREATURE_CAP
            // density-dependent: breeding rate tapers to 0 as pop approaches cap -> population asymptotes
            // to carrying capacity instead of slamming the cap and crashing (no boom-bust overshoot).
            && rng.f32() < P_REPRO_CREATURE * (1.0 - pop as f32 / CREATURE_CAP as f32)
        {
            energy.0 -= REPRO_COST;
            let mut child = genome.clone();
            child.mutate(&mut rng, MUT_RATE, MUT_STD);
            let cp = disperse_pos(&mut rng, ct.translation, 2.0, CREATURE_Y); // child appears beside the parent
            spawn_creature(&mut commands, child, cp, &mut rng);
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
    if live_continuous && pop > 0 && pop < CREATURE_MIN {
        if let Some(g) = sample_genome {
            for _ in 0..(CREATURE_MIN - pop) {
                let mut child = g.clone();
                child.mutate(&mut rng, MUT_RATE, MUT_STD);
                let p = rand_pos(&mut rng, CREATURE_Y);
                spawn_creature(&mut commands, child, p, &mut rng);
            }
        }
    }
    // eaten plants are despawned above; population is replenished by plant_step (reproduction).
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

// Per-creature energy-store ceiling: bigger bodies bank more energy (buffers starvation, costs upkeep).
fn energy_max(g: &Genome) -> f32 {
    ENERGY_MAX * (1.0 + SIZE_ENERGY * g.size)
}

// Compact phenotype signature for KIN similarity (diet + body traits). Two creatures are "kin" when
// their signatures are within SOCIAL_SIM -> drives flocking-by-species + the social need.
fn signature(g: &Genome) -> [f32; 8] {
    [g.expr0[0], g.expr0[1], g.expr0[2], g.expr0[3], g.size, g.swim, g.light_pref, g.height]
}
fn sig_dist(a: &[f32; 8], b: &[f32; 8]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum()
}

// Fraction of social satisfaction from nearby kin (0 isolated .. 1 fully in a herd), given a snapshot
// of (entity, pos, signature). Excludes self by entity id.
fn kin_fraction(me: Entity, pos: Vec3, sig: &[f32; 8], snap: &[(Entity, Vec3, [f32; 8])]) -> f32 {
    let r2 = SOCIAL_RADIUS * SOCIAL_RADIUS;
    let mut kin = 0.0f32;
    for (e, p, s) in snap {
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
    pq: Query<(&PlantGenome, &PlantState), Without<Rot>>, // living plants only (carrion excluded from stats/save)
    tq: Query<&PlantGenome, With<Tree>>, // trees only, for the evolvable-height stat
    soil: Res<Soil>,
    gw: Res<GroundWater>,
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
        if gen.tick % CONT_LOG_TICKS == 0 || done {
            let n = pop.max(1) as f32;
            let mut e = 0.0;
            let mut f = 0.0;
            let mut sens = 0.0;
            let mut bite = 0.0;
            let mut rig = 0.0;
            let mut age = 0.0;
            for (_t, en, fit, _h, _a, g, _b, diet, _l) in cq.iter() {
                e += en.0;
                f += fit.0;
                sens += g.n_sensors() as f32;
                bite += g.bite;
                rig += g.rigidity;
                age += diet.age as f32;
            }
            let plant_n = pq.iter().len().max(1);
            let avg_def: f32 = pq.iter().map(|(g, _)| g.defense).sum::<f32>() / plant_n as f32;
            let avg_nut: f32 = pq.iter().map(|(g, _)| g.nutrient).sum::<f32>() / plant_n as f32;
            let avg_qual: f32 = pq.iter().map(|(g, _)| g.quality).sum::<f32>() / plant_n as f32;
            let avg_wet: f32 = pq.iter().map(|(g, _)| g.wet).sum::<f32>() / plant_n as f32;
            info!(
                "t {:>6} | pop {:>3} | energy {:.1} | life-fit {:.1} | age {:.0} | sens {:.1} | bite {:.2} | rig {:.2} | def {:.2} nut {:.2} qual {:.2} wet {:.2} | plants {} | soil {:.2} | rain {:.2} fire {:.3}",
                gen.tick, pop, e / n, f / n, age / n, sens / n, bite / n, rig / n, avg_def, avg_nut, avg_qual, avg_wet, plant_n, soil.avg(), weather.rain, fire.avg()
            );
            // Track the best healthy snapshot for --save. Score = pop, gated on well-fed (avg energy >= 30)
            // so we never bank a starving crowd. Captured only when saving (snapshot clone is not free).
            if gen.save.is_some() {
                let avg_e = e / n;
                let score = if avg_e >= 30.0 { pop as f32 } else { 0.0 };
                if score > 0.0 && best.as_ref().map_or(true, |(s, _)| score > *s) {
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
        info!("gen {:>3} | nutri {:>6.2} | sens {:.1} r{:.0} | rig {:.2} | bite {:.2} vs def {:.2} | light {:.2} sz {:.2} sw {:.2} so {:.2} brain {:.1} | plant-nut {:.2} qual {:.2} wet {:.2} | roam {:.2} elev {:.1} | plants {} soil {:.2} gw {:.2} fire {:.3} | trees {} h{:.2} b{:.2}", gen.generation, avg, avg_sensors, avg_range, avg_rig, avg_bite, avg_def, avg_light, avg_size, avg_swim, avg_social, avg_hidden, avg_nut, avg_qual, avg_wet, avg_roam, avg_elev, plant_n, soil.avg(), gw.avg(), fire.avg(), tree_n, avg_tree_h, avg_tree_b);
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
        diet.expr = child.expr0;
        diet.g = 0.0;
        diet.age = if desync { (rng.f32() * 600.0) as u32 } else { 0 };
        diet.fatigue = 0.0;
        *g = child;
        energy.0 = if desync { rng.range(0.8, 1.2) * START_ENERGY } else { START_ENERGY }; // stagger but never lethally low
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
