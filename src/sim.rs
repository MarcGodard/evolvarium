// M1 foraging sim: creatures sense nearest food, a tiny NN drives thrust+turn, they eat,
// burn energy, starve; a generational GA selects by food eaten. Proof-of-life milestone (08).
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::components::{Alive, Brain, Creature, DietState, Energy, Fitness, Food, Heading, Locomotion, Rot, Tree};
use crate::genome::{forward, learn, Genome, CONE_HALF, GLOBAL_INPUTS, NFOOD, SIG_PER_SENSOR};
use crate::plant::{PlantGenome, PlantState, P_REPRO, PLANT_CAP, PLANT_MIN};
use crate::rng::Rng;

// --- tuning (god-tunable later, see 07) ---
pub const POP: usize = 140;
pub const FOOD: usize = 480;
pub const WORLD_HALF: f32 = 80.0; // square arena [-H, H] in x,z (doubled playground)
pub const GEN_TICKS: u32 = 1500; // fixed-steps per generation (generational mode); also the log interval
pub const MAX_GEN_HEADLESS: u32 = 40; // default headless run length in generations (override: --gens=N)

// Continuous reproduction (default mode). A well-fed creature spends energy to bud a mutated child;
// death is real (despawn + carrion). Selection is emergent (breed before you die), like the plants.
// Breed only when genuinely well-fed: this is density dependence -> high population means low per-capita
// food means few reach the threshold means breeding self-throttles (prevents boom-bust to extinction).
const REPRO_THRESHOLD: f32 = 22.0; // energy to be eligible (near the sustained-forager equilibrium ~18)
const REPRO_COST: f32 = 10.0; // energy the parent spends per child (>BIRTH_ENERGY: birth dissipates some)
const BIRTH_ENERGY: f32 = 8.0; // offspring's starting energy
const P_REPRO_CREATURE: f32 = 0.02; // per-tick reproduction chance while eligible (gentle: damps overshoot)
pub const CREATURE_CAP: usize = 130; // population ceiling (kept below grazing pressure that crashes plants)
// Continuous mode cold-starts badly (random foragers starve before breeding). Bootstrap with a short
// GENERATIONAL warm-up to evolve competent foragers, THEN switch to continuous birth/death.
const WARMUP_GENS: u32 = 12;

const START_ENERGY: f32 = 30.0;
const BASAL_COST: f32 = 1.4; // energy/sec just to live (low -> resting is genuinely valuable)
const MOVE_COST: f32 = 6.0; // movement cost scales with thrust^2 (sprinting dear, gentle motion cheap)
const MOVE_SPEED: f32 = 9.0; // units/sec at full thrust
const TURN_SPEED: f32 = 3.0; // rad/sec at full turn
// Elevation (P3): climbing burns energy per unit height gained; descending refunds less than the
// climb cost so a round trip is a net loss (dissipative, no free lunch -> high ground is "expensive").
const CLIMB_COST: f32 = 1.2;
const DESCEND_REFUND: f32 = 0.4;
const ROCK_MOVE_COST: f32 = 9.0; // extra energy/sec moving over rocky highland (hard to cross)
const SENSE_COST: f32 = 0.012; // energy/sec per unit of total sensor range (long-range vision isn't free)
const EAT_RADIUS: f32 = 1.1;
const ENERGY_MAX: f32 = 60.0; // energy ceiling; eating past it harms (overeating trade-off, see 12)
const OVEREAT_G: f32 = 0.2; // growth-load gained per unit of energy eaten while already full

// Living food (see 13). Eat success = sigmoid(BITE_K*(bite - defense)); energy ∝ mass*nutrient.
const BITE_K: f32 = 8.0;
const BITE_COST: f32 = 1.5; // energy/sec maintenance cost of bite strength
const EAT_GAIN: f32 = 7.0; // energy per (mass * nutrient) consumed
const MEAT_BONUS: f32 = 1.6; // meat (carrion) is richer + longer-lasting than plant food
// Predation (M5): a creature attacks an adjacent creature; `bite` is the combat stat (attack AND
// defense). A kill = top energy + fitness. Predators need high bite, whose upkeep (BITE_COST) is the cost.
const ATTACK_RADIUS: f32 = 1.6; // must be adjacent to attack
const PREDATION_GAIN: f32 = 22.0; // energy a predator gains from a kill
// Trees (BACKLOG): long-lived, near-uneatable plants. Only TALL creatures (sensors >= TREE_REACH)
// can reach + eat them; moisture-immune; grow large; reproduce slowly toward TREE_CAP.
const N_TREES: usize = 40; // initial trees
const TREE_CAP: usize = 70; // max trees
const TREE_REACH_H: f32 = 0.6; // creature height needed to reach + eat fruit-tree food
const TREE_NUTRIENT: f32 = 0.9; // trees are rich food (worth the reach)
const TREE_MATURITY: f32 = 14.0; // trees grow large before reproducing
const P_TREE_REPRO: f32 = 0.004; // slow reproduction (long-lived, sparse)
const TREE_DENSITY_R: f32 = 18.0; // trees self-limit clustering within this radius
const TREE_MAX_LOCAL: usize = 4; // max trees within TREE_DENSITY_R before a tree stops seeding nearby
const TREE_BITE_MASS: f32 = 2.5; // mass a creature strips per feeding (tree survives + regrows)
const TREE_MIN_MASS: f32 = 1.0; // below this a fruit tree is over-eaten and dies
const HEIGHT_COST: f32 = 1.4; // energy/sec upkeep per unit height (tall reaches trees but costs more)
const SEED_VIA_GUT: f32 = 0.5; // max chance (x quality) an eaten plant disperses an offspring (13)
const PLANT_START_MASS: f32 = 0.6;

// Rot chain (P3): a dead creature drops carrion. Fresh = rich meat; over ROT_GONE ticks its
// nutrition fades to 0 and toxin rises to TOXIN_MAX, then it fully decomposes (despawns).
const CARRION_KIND: u8 = 0; // meat = food type 0 (couples to diet expr only via sensing, not digestion)
const CARRION_MASS: f32 = 3.0; // a meaty chunk: worth scavenging while fresh
const CARRION_NUTRIENT: f32 = 0.9; // fresh meat is energy-dense
pub const ROT_GONE: u32 = 900; // ticks from death to full decomposition (~15s sim); viz reads it for color
const TOXIN_MAX: f32 = 9.0; // energy hit from eating fully-rotten carrion (poison)
const TOXIN_G: f32 = 0.15; // growth-load per unit toxin ingested

// Moisture pressure (P3): a plant whose `wet` preference is far from local soil moisture is stressed
// and may die; dead plants become poison detritus (rotting vegetation). Drives spatial niches.
const DETRITUS_NUTRIENT: f32 = 0.3; // dead vegetation: poor food fresh, rots to poison
const MOISTURE_TOLERANCE: f32 = 0.3; // mismatch under this is harmless
const MOISTURE_KILL: f32 = 0.012; // per-tick death scale for mismatch beyond tolerance
const HABITAT_KILL: f32 = 0.03; // per-tick death scale in poor sites (deep water / arid desert)
const SEASON_FREQ: f32 = 0.4; // seasonal wet/dry oscillation speed (radians per generation)
// Defense also taxes REPRODUCTION, not just growth (tuning): at carrying capacity, growth cost is
// toothless (plants stay capped anyway), so armored plants pegged defense ~free. Penalizing their
// repro lets cheaper plants win the cap-slot competition -> defense settles at an interior value.
const DEF_REPRO_COST: f32 = 0.7; // armored plant (def=1) reproduces at (1-0.7)=30% the base rate

// Nutrient closed loop (M5): un-eaten carrion/detritus that fully decomposes releases fertility into a
// soil grid at that spot; fertile patches boost nearby plant growth. Death -> soil -> richer food.
pub const SOIL_RES: usize = 32; // fertility grid cells per axis
const SOIL_DECAY: f32 = 0.999; // fertility leaches/is taken up each tick
const DECOMP_FERT: f32 = 3.0; // fertility released on full decomposition (x corpse nutrient)
const FERT_GROWTH: f32 = 0.6; // max growth-rate bonus from saturated soil
const FERT_CAP: f32 = 1.5; // fertility level at which the growth bonus saturates
const PLANT_REPRO_FRAC: f32 = 0.5; // fraction of mass kept after budding off a child

// Diet/epigenetic model (--diet, see 12). Expression ramps on eaten type, decays on others.
const EXPR_RAMP: f32 = 0.08; // how fast expression of the eaten type rises (x (1-rigidity))
const EXPR_DECAY: f32 = 0.04; // how fast unused types' expression falls (x (1-rigidity))
const EXPR_OVERHEAD: f32 = 1.2; // maintenance energy/sec per unit total expression (generalist cost)
const G_GAIN: f32 = 0.6; // growth-load gained per low-efficiency (mismatch) eat
const G_DECAY: f32 = 0.01; // growth-load shed per tick when on-diet
const DISEASE_K: f32 = 0.012; // per-tick disease mortality per unit growth-load
const MISMATCH_STRESS: f32 = 3.0; // energy hit for eating a poorly-expressed (wrong) food
const AGE_HAZARD: f32 = 0.02; // late-life mortality ceiling (decelerates -> ~plateau)
const AGE_SCALE: f32 = 900.0; // ticks; age at which aging hazard reaches half its ceiling

const ELITE_FRAC: f32 = 0.3;
const MUT_RATE: f32 = 0.12;
const MUT_STD: f32 = 0.3;

// M2 lifetime learning (see 04 + 09). Reward = approach-shaping + eat bonus; Hebbian/Oja tunes brain.
const LEARN_RATE: f32 = 0.04;
const R_APPROACH: f32 = 0.6; // reward per unit closer to nearest food this tick
const R_EAT: f32 = 1.0; // bonus reward on the tick food is eaten

const CREATURE_Y: f32 = 0.5;
const FOOD_Y: f32 = 0.4;

// Fixed sim step. Constant (not wall-clock) so headless can fast-forward and runs are
// deterministic. Render mode runs this at 60 Hz for real-time visuals; headless spins it flat-out.
pub const DT: f32 = 1.0 / 60.0;

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
    fn index(x: f32, z: f32) -> usize {
        let to_cell = |w: f32| {
            (((w + WORLD_HALF) / (2.0 * WORLD_HALF)) * SOIL_RES as f32).clamp(0.0, (SOIL_RES - 1) as f32) as usize
        };
        to_cell(z) * SOIL_RES + to_cell(x)
    }
    fn add(&mut self, x: f32, z: f32, amt: f32) {
        self.cell[Self::index(x, z)] += amt;
    }
    fn get(&self, x: f32, z: f32) -> f32 {
        self.cell[Self::index(x, z)]
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

fn rand_pos(rng: &mut Rng, y: f32) -> Vec3 {
    let x = rng.range(-WORLD_HALF, WORLD_HALF);
    let z = rng.range(-WORLD_HALF, WORLD_HALF);
    Vec3::new(x, y + crate::terrain::height(x, z), z) // sit on the terrain surface (P3)
}

fn diet_state(g: &Genome) -> DietState {
    DietState { expr: g.expr0, g: 0.0, age: 0 }
}

// Spawn carrion (meat) at a spot: a Food entity with the Rot clock. Used by death + predation kills.
fn spawn_carrion(commands: &mut Commands, pos: Vec3, mass: f32) {
    let p = Vec3::new(pos.x, FOOD_Y + crate::terrain::height(pos.x, pos.z), pos.z);
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

// Tree genome: near-uneatable (defense ~1) so only tall creatures reach it; rich; grows large + slow.
fn tree_genome() -> PlantGenome {
    PlantGenome { kind: 0, nutrient: TREE_NUTRIENT, defense: 0.99, quality: 0.2, wet: 0.5, height: 1.0, spread: 7.0, maturity: TREE_MATURITY }
}

// Spawn one tree (long-lived plant + Tree marker). edible=true tall fruit tree, false=evergreen.
fn spawn_tree(commands: &mut Commands, mass: f32, pos: Vec3, edible: bool) {
    commands.spawn((
        Food,
        Tree { edible },
        PlantState { mass, age: 0 },
        tree_genome(),
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
pub fn spawn_world_headless(mut commands: Commands, mut rng: ResMut<Rng>, gen: Res<GenState>) {
    // --load resumes a saved population; otherwise a random founding pop. Positions re-randomized.
    let snap = gen.load.as_deref().and_then(crate::persist::load_snapshot);
    let genomes: Vec<Genome> = match &snap {
        Some(s) if !s.creatures.is_empty() => s.creatures.clone(),
        _ => (0..POP).map(|_| Genome::random(&mut rng)).collect(),
    };
    for g in genomes {
        let p = rand_pos(&mut rng, CREATURE_Y);
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
        let diet = diet_state(&g);
        commands.spawn((
            Creature,
            g,
            brain,
            diet,
            Energy(START_ENERGY),
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
                let p = rand_pos(&mut rng, FOOD_Y);
                spawn_plant(&mut commands, sp.g.clone(), sp.mass, p);
            }
        }
        _ => {
            for _ in 0..FOOD {
                let p = rand_pos(&mut rng, FOOD_Y);
                let pg = PlantGenome::random(&mut rng, ntypes);
                spawn_plant(&mut commands, pg, PLANT_START_MASS, p);
            }
        }
    }
    spawn_trees(&mut commands, &mut rng);
}

// Scatter the initial trees (half tall fruit trees, half uneatable evergreens) on habitable land.
fn spawn_trees(commands: &mut Commands, rng: &mut Rng) {
    for i in 0..N_TREES {
        let mut p = rand_pos(rng, FOOD_Y);
        for _ in 0..6 {
            if crate::terrain::plant_habitability(p.x, p.z, 0.0) > 0.4 {
                break;
            }
            p = rand_pos(rng, FOOD_Y);
        }
        spawn_tree(commands, rng.range(3.0, 9.0), p, i % 2 == 0); // alternate fruit tree / evergreen
    }
}

// Render: same components + mesh/material so we can watch. Plant visuals added by add_plant_visuals.
pub fn spawn_world_render(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
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
    for g in genomes {
        let p = rand_pos(&mut rng, CREATURE_Y);
        let h = rng.range(-std::f32::consts::PI, std::f32::consts::PI);
        let brain = Brain { net: g.net.clone(), prev_dist: f32::INFINITY };
        let diet = diet_state(&g);
        // own material per creature so viz can recolor it by evolved traits (see viz.rs)
        let mat = materials.add(Color::srgb(0.9, 0.6, 0.3));
        commands.spawn((
            Creature,
            g,
            brain,
            diet,
            Energy(START_ENERGY),
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
                let p = rand_pos(&mut rng, FOOD_Y);
                spawn_plant(&mut commands, sp.g.clone(), sp.mass, p);
            }
        }
        _ => {
            for _ in 0..FOOD {
                let p = rand_pos(&mut rng, FOOD_Y);
                let pg = PlantGenome::random(&mut rng, ntypes);
                spawn_plant(&mut commands, pg, PLANT_START_MASS, p);
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
    let mut tree_births: Vec<(Vec3, bool)> = Vec::new(); // (pos, edible)
    let mut detritus: Vec<(PlantGenome, f32, Vec3)> = Vec::new(); // moisture-killed plants -> poison
    for (e, mut st, g, tf, tree) in &mut q {
        let (px, pz) = (tf.translation.x, tf.translation.z);
        let fert = soil.get(px, pz);
        let boost = 1.0 + FERT_GROWTH * (fert / FERT_CAP).min(1.0);
        if let Some(tree) = tree {
            // apply this tick's grazing; a fruit tree grazed below TREE_MIN_MASS is over-eaten -> dies
            if let Some(&bite) = tree_bites.0.get(&e) {
                st.mass = (st.mass - bite).max(0.0);
                if st.mass < TREE_MIN_MASS {
                    commands.entity(e).despawn();
                    continue;
                }
            }
            // trees: moisture-immune (long-lived), grow slowly + large. Reproduction FOLLOWS GROUND
            // FERTILITY (fertile soil seeds more trees) and SELF-LIMITS by local density (no over-cluster).
            st.mass += g.growth_rate() * boost * DT;
            st.age += 1;
            let r2 = TREE_DENSITY_R * TREE_DENSITY_R;
            let local = tree_positions.iter().filter(|p| p.distance_squared(tf.translation) < r2).count();
            let fert_boost = 0.4 + 1.6 * (fert / FERT_CAP).min(1.0); // richer ground -> more new trees
            if st.mass >= g.maturity
                && tree_count + tree_births.len() < TREE_CAP
                && local <= TREE_MAX_LOCAL
                && rng.f32() < P_TREE_REPRO * fert_boost
            {
                let off = Vec3::new(rng.range(-g.spread, g.spread), 0.0, rng.range(-g.spread, g.spread));
                let mut pos = tf.translation + off;
                pos.x = pos.x.clamp(-WORLD_HALF, WORLD_HALF);
                pos.z = pos.z.clamp(-WORLD_HALF, WORLD_HALF);
                pos.y = FOOD_Y + crate::terrain::height(pos.x, pos.z);
                tree_births.push((pos, tree.edible));
                st.mass *= PLANT_REPRO_FRAC;
            }
            continue;
        }
        // --- regular plant: mortality from moisture mismatch OR a poor site (deep water / desert) ---
        let m = crate::terrain::moisture(px, pz, season);
        let stress = (m - g.wet).abs();
        let hab = crate::terrain::plant_habitability(px, pz, season); // 0 in water/desert, 1 on good land
        let p_mort =
            MOISTURE_KILL * (stress - MOISTURE_TOLERANCE).max(0.0) + HABITAT_KILL * (0.3 - hab).max(0.0);
        if rng.f32() < p_mort {
            commands.entity(e).despawn();
            detritus.push((g.clone(), st.mass, tf.translation));
            plant_count = plant_count.saturating_sub(1);
            continue;
        }
        // fertile soil speeds growth (M5); growth also scales with habitability (P3 sparse water/desert)
        st.mass += g.growth_rate() * boost * hab * DT;
        st.age += 1;
        if st.mass >= g.maturity
            && plant_count + births.len() < PLANT_CAP
            && rng.f32() < P_REPRO * (1.0 - DEF_REPRO_COST * g.defense)
        {
            let mut child = g.clone();
            child.mutate(&mut rng);
            let off = Vec3::new(rng.range(-g.spread, g.spread), 0.0, rng.range(-g.spread, g.spread));
            let mut pos = tf.translation + off;
            pos.x = pos.x.clamp(-WORLD_HALF, WORLD_HALF);
            pos.z = pos.z.clamp(-WORLD_HALF, WORLD_HALF);
            pos.y = FOOD_Y + crate::terrain::height(pos.x, pos.z);
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
        spawn_plant(&mut commands, g, PLANT_START_MASS, pos);
    }
    for (pos, edible) in tree_births {
        spawn_tree(&mut commands, PLANT_START_MASS, pos, edible);
    }
    tree_bites.0.clear(); // consumed this tick
}

// --- predation (M5): creatures attack + eat each other. bite = combat (attack + defense). Opportunistic
// for now; NN-driven attack arrives with the creature-sensing batch. ---
pub fn predation_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    mut commands: Commands,
    mut cq: Query<(Entity, &Transform, &mut Energy, &mut Fitness, &mut Alive, &Genome), With<Creature>>,
) {
    // snapshot living creatures: (entity, pos, bite)
    let snap: Vec<(Entity, Vec3, f32)> = cq
        .iter()
        .filter(|(_, _, _, _, a, _)| a.0)
        .map(|(e, t, _, _, _, g)| (e, t.translation, g.bite))
        .collect();
    if snap.len() < 2 {
        return;
    }
    let mut killed: HashSet<Entity> = HashSet::new();
    let mut gains: HashMap<Entity, f32> = HashMap::new();
    let r2 = ATTACK_RADIUS * ATTACK_RADIUS;
    for (ai, &(ae, apos, abite)) in snap.iter().enumerate() {
        if killed.contains(&ae) {
            continue; // a creature killed this tick doesn't also attack
        }
        let mut best: Option<(f32, usize)> = None;
        for (bi, &(be, bpos, _)) in snap.iter().enumerate() {
            if bi == ai || killed.contains(&be) {
                continue;
            }
            let d2 = apos.distance_squared(bpos);
            if d2 < r2 && best.map_or(true, |(bd, _)| d2 < bd) {
                best = Some((d2, bi));
            }
        }
        if let Some((_, bi)) = best {
            let (be, _, bbite) = snap[bi];
            // success = attacker bite vs prey bite (bite doubles as defense)
            if rng.f32() < sigmoid(BITE_K * (abite - bbite)) {
                killed.insert(be);
                *gains.entry(ae).or_insert(0.0) += PREDATION_GAIN;
            }
        }
    }
    if killed.is_empty() {
        return;
    }
    let continuous_live = gen.continuous && gen.generation >= WARMUP_GENS;
    for (e, t, mut energy, mut fit, mut alive, _g) in &mut cq {
        if let Some(g) = gains.get(&e) {
            energy.0 = (energy.0 + g).min(ENERGY_MAX);
            fit.0 += g * 0.3; // predation counts toward selection
        }
        if killed.contains(&e) {
            alive.0 = false;
            spawn_carrion(&mut commands, t.translation, CARRION_MASS * 0.5); // predator already ate some
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
            soil.add(tf.translation.x, tf.translation.z, DECOMP_FERT * g.nutrient); // return nutrients
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
    fq: Query<(Entity, &Transform, &PlantState, &PlantGenome, Option<&Rot>, Option<&Tree>), (With<Food>, Without<Creature>)>,
) {
    let dt = DT;
    let ntypes = gen.ntypes();
    let mut pop = cq.iter().count(); // live population (for the continuous-mode reproduction cap)
    // continuous birth/death is active only AFTER the generational warm-up (see WARMUP_GENS)
    let live_continuous = gen.continuous && gen.generation >= WARMUP_GENS;
    // snapshot: (entity, pos, genome, mass, rot_age, tree). rot_age=Some -> carrion; tree=Some(edible) ->
    // a tree (edible fruit tree if true, uneatable evergreen if false); else a living plant.
    let foods: Vec<(Entity, Vec3, PlantGenome, f32, Option<u32>, Option<bool>)> = fq
        .iter()
        .map(|(e, t, st, pg, rot, tree)| {
            (e, t.translation, pg.clone(), st.mass, rot.map(|r| r.age), tree.map(|t| t.edible))
        })
        .collect();
    let mut eaten: HashSet<Entity> = HashSet::new();

    for (entity, mut ct, mut energy, mut fit, mut head, mut alive, genome, mut brain, mut diet, mut loco) in &mut cq {
        if !alive.0 {
            continue;
        }
        let pos = ct.translation;

        // nearest non-eaten food (index into foods)
        let mut best: Option<(usize, f32)> = None;
        for (i, f) in foods.iter().enumerate() {
            if eaten.contains(&f.0) {
                continue;
            }
            let d2 = pos.distance_squared(f.1);
            if best.map_or(true, |(_, bd2)| d2 < bd2) {
                best = Some((i, d2));
            }
        }
        let cur_dist = best.map(|(_, d2)| d2.sqrt()).unwrap_or(f32::INFINITY);

        // build inputs from the EVOLVABLE sensors: each is a directional eye (angle + range) that
        // reports nearest food in its cone. The GA decides how many sensors and where they point.
        let mut input: Vec<f32> = Vec::with_capacity(genome.sensors.len() * SIG_PER_SENSOR + GLOBAL_INPUTS);
        for s in &genome.sensors {
            let mut sd = f32::INFINITY;
            let mut skind = 0u8;
            for f in &foods {
                if eaten.contains(&f.0) {
                    continue;
                }
                let to = f.1 - pos;
                let dist = to.length();
                if dist > s.range {
                    continue;
                }
                let bearing = wrap_angle(to.x.atan2(to.z) - head.0);
                if wrap_angle(bearing - s.angle).abs() <= CONE_HALF && dist < sd {
                    sd = dist;
                    skind = f.2.kind;
                }
            }
            if sd.is_finite() {
                input.push(1.0 / (1.0 + sd)); // inv-distance in this sensor's cone
                let sig = if gen.diet {
                    diet.expr[skind as usize] // readiness to digest what this eye sees
                } else {
                    (skind as f32 / (ntypes.max(2) - 1) as f32) * 2.0 - 1.0 // food type
                };
                input.push(sig);
            } else {
                input.push(0.0);
                input.push(0.0);
            }
        }
        input.push(energy.0 / START_ENERGY);
        input.push(1.0); // bias

        // think (per-life learned brain, dynamic topology matching this genome's sensor count)
        let (h, out) = forward(&brain.net, &input);
        let thrust = out[0];
        let turn = out[1];

        // act
        head.0 = wrap_angle(head.0 + turn * TURN_SPEED * dt);
        let dir = Vec3::new(head.0.sin(), 0.0, head.0.cos());
        let mut np = pos + dir * (thrust * MOVE_SPEED * dt);
        np.x = np.x.clamp(-WORLD_HALF, WORLD_HALF);
        np.z = np.z.clamp(-WORLD_HALF, WORLD_HALF);
        // ride the terrain; pay for elevation change (P3): uphill costs, downhill partially refunds
        let h0 = crate::terrain::height(pos.x, pos.z);
        let h1 = crate::terrain::height(np.x, np.z);
        np.y = CREATURE_Y + h1;
        let dh = h1 - h0;
        energy.0 -= if dh > 0.0 { CLIMB_COST * dh } else { DESCEND_REFUND * dh };
        loco.path += np.distance(pos); // accumulate 3D distance walked (diagnostic)
        ct.translation = np;
        ct.rotation = Quat::from_rotation_y(head.0);

        // metabolism: basal + movement (convex in speed) + bite upkeep + rocky crossing + vision upkeep.
        // Longer/more sensors see farther but cost energy (SENSE_COST x total range) -> range is a trade-off.
        let rock = crate::terrain::rockiness(np.x, np.z);
        let sense_range: f32 = genome.sensors.iter().map(|s| s.range).sum();
        energy.0 -= (BASAL_COST
            + MOVE_COST * thrust * thrust
            + BITE_COST * genome.bite
            + ROCK_MOVE_COST * rock * thrust.abs()
            + SENSE_COST * sense_range
            + HEIGHT_COST * genome.height)
            * dt;

        // eat nearest plant on contact, IF bite beats its defense (arms race, see 13)
        let mut eat_reward = 0.0;
        if let Some((i, _)) = best {
            let (e, fp, mass) = (foods[i].0, foods[i].1, foods[i].3);
            let pg = foods[i].2.clone();
            let rot_age = foods[i].4;
            let tree = foods[i].5; // None=plant/carrion, Some(true)=fruit tree, Some(false)=evergreen
            if np.distance(fp) < EAT_RADIUS {
                // trees: only a TALL creature (sensors >= TREE_REACH) reaches an EDIBLE tree; evergreens
                // (edible=false) are never eatable. Plants/carrion: bite vs defense as usual.
                let success = match tree {
                    Some(edible) => edible && genome.height >= TREE_REACH_H, // tall creature reaches fruit tree
                    // plant: creature must be tall enough to reach it (height defense) AND bite its defense
                    None => {
                        genome.height + 0.15 >= pg.height
                            && rng.f32() < sigmoid(BITE_K * (genome.bite - pg.defense))
                    }
                };
                if success {
                    if let Some(true) = tree {
                        // FRUIT TREE: strip a chunk of foliage; the tree persists + regrows and only dies
                        // if grazed below TREE_MIN_MASS (over-eaten). Rich reward for the reach.
                        let bite_mass = TREE_BITE_MASS.min(mass);
                        let base = bite_mass * pg.nutrient * (0.5 + pg.quality);
                        let eff = if gen.diet { diet.expr[pg.kind as usize] } else { 1.0 };
                        energy.0 += EAT_GAIN * base * eff;
                        fit.0 += base * eff;
                        *tree_bites.0.entry(e).or_insert(0.0) += bite_mass;
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
                        // quality scales extractable energy: factor 0.5..1.5, ~1.0 at quality 0.5 (balance-neutral)
                        let base = mass * pg.nutrient * (0.5 + pg.quality);
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
                    if energy.0 > ENERGY_MAX {
                        let excess = energy.0 - ENERGY_MAX;
                        energy.0 = ENERGY_MAX;
                        diet.g += excess * OVEREAT_G;
                    }
                    // plants/carrion are consumed (despawn); trees PERSIST (mass reduced in plant_step)
                    if tree.is_none() {
                        commands.entity(e).despawn();
                        eaten.insert(e);
                        // endozoochory (13): an eaten LIVING plant may disperse a mutated offspring near
                        // the eater. Chance scales with quality -> tasty plants pay growth but win dispersal.
                        if rot_age.is_none() && foods.len() < PLANT_CAP && rng.f32() < pg.quality * SEED_VIA_GUT {
                            let mut child = pg.clone();
                            child.mutate(&mut rng);
                            let off =
                                Vec3::new(rng.range(-pg.spread, pg.spread), 0.0, rng.range(-pg.spread, pg.spread));
                            let mut sp = np + off;
                            sp.x = sp.x.clamp(-WORLD_HALF, WORLD_HALF);
                            sp.z = sp.z.clamp(-WORLD_HALF, WORLD_HALF);
                            sp.y = FOOD_Y + crate::terrain::height(sp.x, sp.z);
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
            && pop < CREATURE_CAP
            && rng.f32() < P_REPRO_CREATURE
        {
            energy.0 -= REPRO_COST;
            let mut child = genome.clone();
            child.mutate(&mut rng, MUT_RATE, MUT_STD);
            let off = Vec3::new(rng.range(-2.0, 2.0), 0.0, rng.range(-2.0, 2.0));
            let mut cp = ct.translation + off;
            cp.x = cp.x.clamp(-WORLD_HALF, WORLD_HALF);
            cp.z = cp.z.clamp(-WORLD_HALF, WORLD_HALF);
            cp.y = CREATURE_Y + crate::terrain::height(cp.x, cp.z);
            spawn_creature(&mut commands, child, cp, &mut rng);
            pop += 1;
        }

        // died this tick (loop skips already-dead creatures at the top) -> drop carrion here, which
        // rots into poison (rot_step). Closes part of the nutrient loop: death feeds the food web (P3).
        if !alive.0 {
            spawn_carrion(&mut commands, ct.translation, CARRION_MASS);
            // continuous (post-warmup): the corpse entity is gone (became carrion). Generational mode
            // and the warm-up keep it (Alive=false) to be recycled into the next generation.
            if live_continuous {
                commands.entity(entity).despawn();
                pop = pop.saturating_sub(1);
            }
        }
    }
    // eaten plants are despawned above; population is replenished by plant_step (reproduction).
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
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
    soil: Res<Soil>,
    mut exit: MessageWriter<AppExit>,
) {
    gen.tick = gen.tick.wrapping_add(1); // global clock: drives season (plant_step) + continuous timing

    // --- continuous mode (after warm-up): no generation boundary. Snapshot the ecosystem periodically;
    // stop headless at MAX_TICKS_HEADLESS or on extinction. Selection is emergent (live_step). ---
    if gen.continuous && gen.generation >= WARMUP_GENS {
        let pop = cq.iter().count();
        let done = gen.headless && (gen.tick >= gen.max_gens * GEN_TICKS || pop == 0);
        if gen.tick % GEN_TICKS == 0 || done {
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
                "t {:>6} | pop {:>3} | energy {:.1} | life-fit {:.1} | age {:.0} | sens {:.1} | bite {:.2} | rig {:.2} | def {:.2} nut {:.2} qual {:.2} wet {:.2} | plants {} | soil {:.2}",
                gen.tick, pop, e / n, f / n, age / n, sens / n, bite / n, rig / n, avg_def, avg_nut, avg_qual, avg_wet, plant_n, soil.avg()
            );
        }
        if done {
            if let Some(path) = &gen.save {
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
    // plant (food) co-evolution stats: arms race = bite vs defense
    let plant_n = pq.iter().len().max(1);
    let avg_def: f32 = pq.iter().map(|(g, _)| g.defense).sum::<f32>() / plant_n as f32;
    let avg_nut: f32 = pq.iter().map(|(g, _)| g.nutrient).sum::<f32>() / plant_n as f32;
    let avg_qual: f32 = pq.iter().map(|(g, _)| g.quality).sum::<f32>() / plant_n as f32;
    let avg_wet: f32 = pq.iter().map(|(g, _)| g.wet).sum::<f32>() / plant_n as f32;
    if gen.diet {
        let avg_rig: f32 = scored.iter().map(|(_, g)| g.rigidity).sum::<f32>() / n as f32;
        info!("gen {:>3} | nutri {:>6.2} | sens {:.1} r{:.0} | rig {:.2} | bite {:.2} vs def {:.2} | plant-nut {:.2} qual {:.2} wet {:.2} | roam {:.2} elev {:.1} | plants {} soil {:.2}", gen.generation, avg, avg_sensors, avg_range, avg_rig, avg_bite, avg_def, avg_nut, avg_qual, avg_wet, avg_roam, avg_elev, plant_n, soil.avg());
    } else {
        info!("gen {:>3} | food {:>6.2} | sens {:.1} r{:.0} | bite {:.2} vs def {:.2} | plant-nut {:.2} qual {:.2} wet {:.2} | roam {:.2} elev {:.1} | plants {} soil {:.2}", gen.generation, avg, avg_sensors, avg_range, avg_bite, avg_def, avg_nut, avg_qual, avg_wet, avg_roam, avg_elev, plant_n, soil.avg());
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
    for ((mut t, mut energy, mut fit, mut head, mut alive, mut g, mut brain, mut diet, mut loco), child) in
        cq.iter_mut().zip(next)
    {
        brain.net = child.net.clone();
        brain.prev_dist = f32::INFINITY;
        diet.expr = child.expr0;
        diet.g = 0.0;
        diet.age = 0;
        *g = child;
        energy.0 = START_ENERGY;
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
