// M1 foraging sim: creatures sense nearest food, a tiny NN drives thrust+turn, they eat,
// burn energy, starve; a generational GA selects by food eaten. Proof-of-life milestone (08).
use bevy::prelude::*;
use std::collections::HashSet;

use crate::components::{Alive, Brain, Creature, DietState, Energy, Fitness, Food, FoodKind, Heading, Locomotion};
use crate::genome::{forward, learn, Genome, CONE_HALF, GLOBAL_INPUTS, NFOOD, SIG_PER_SENSOR};
use crate::plant::{PlantGenome, PlantState, P_REPRO, PLANT_CAP, PLANT_MIN};
use crate::rng::Rng;

// --- tuning (god-tunable later, see 07) ---
pub const POP: usize = 60;
pub const FOOD: usize = 80;
pub const WORLD_HALF: f32 = 25.0; // square arena [-H, H] in x,z
pub const GEN_TICKS: u32 = 1500; // fixed-steps per generation
pub const MAX_GEN_HEADLESS: u32 = 40; // headless stops after this many gens

const START_ENERGY: f32 = 30.0;
const BASAL_COST: f32 = 1.4; // energy/sec just to live (low -> resting is genuinely valuable)
const MOVE_COST: f32 = 6.0; // movement cost scales with thrust^2 (sprinting dear, gentle motion cheap)
const MOVE_SPEED: f32 = 9.0; // units/sec at full thrust
const TURN_SPEED: f32 = 3.0; // rad/sec at full turn
// Elevation (P3): climbing burns energy per unit height gained; descending refunds less than the
// climb cost so a round trip is a net loss (dissipative, no free lunch -> high ground is "expensive").
const CLIMB_COST: f32 = 1.2;
const DESCEND_REFUND: f32 = 0.4;
const EAT_RADIUS: f32 = 1.1;
const FOOD_VALUE: f32 = 14.0;
const ENERGY_MAX: f32 = 60.0; // energy ceiling; eating past it harms (overeating trade-off, see 12)
const OVEREAT_G: f32 = 0.2; // growth-load gained per unit of energy eaten while already full

// Living food (see 13). Eat success = sigmoid(BITE_K*(bite - defense)); energy ∝ mass*nutrient.
const BITE_K: f32 = 8.0;
const BITE_COST: f32 = 1.5; // energy/sec maintenance cost of bite strength
const EAT_GAIN: f32 = 7.0; // energy per (mass * nutrient) consumed
const SEED_VIA_GUT: f32 = 0.5; // max chance (x quality) an eaten plant disperses an offspring (13)
const PLANT_START_MASS: f32 = 0.6;
const PLANT_REPRO_FRAC: f32 = 0.5; // fraction of mass kept after budding off a child

// Poison mode (--poison): signed eat reward so learners associate type -> approach/avoid.
const R_EAT_GOOD: f32 = 2.0;
const R_EAT_POISON: f32 = -2.0;
const POISON_COST: f32 = 6.0; // toxic energy hit: stings but survivable, so creature lives to learn

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
    pub poison: bool,    // two food types, nutritious/toxic flips each generation
    pub good_type: u8,   // which FoodKind is nutritious this generation
    pub diet: bool,      // epigenetic diet model (NFOOD types, expression, growth-load, disease)
    pub shift: bool,     // food availability changes each generation (non-stationary)
    pub avail: Vec<u8>,  // food types available this generation
    pub save: Option<String>, // --save=PATH: write survivors at headless run end (BACKLOG P2)
    pub load: Option<String>, // --load=PATH: resume from a saved population instead of random
}

impl GenState {
    // Number of distinct food types in play this mode.
    pub fn ntypes(&self) -> u8 {
        if self.diet { NFOOD as u8 } else if self.poison { 2 } else { 1 }
    }
}

fn rand_pos(rng: &mut Rng, y: f32) -> Vec3 {
    let x = rng.range(-WORLD_HALF, WORLD_HALF);
    let z = rng.range(-WORLD_HALF, WORLD_HALF);
    Vec3::new(x, y + crate::terrain::height(x, z), z) // sit on the terrain surface (P3)
}

fn diet_state(g: &Genome) -> DietState {
    DietState { expr: g.expr0, g: 0.0, age: 0 }
}

// Spawn one plant (living food). No render mesh; add_plant_visuals (render mode) gives it one.
fn spawn_plant(commands: &mut Commands, g: PlantGenome, mass: f32, pos: Vec3) {
    commands.spawn((
        Food,
        FoodKind(g.kind),
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
}

// --- plants: grow, reproduce (disperse mutated offspring), reseed if the web nearly collapses (13) ---
pub fn plant_step(
    mut commands: Commands,
    mut rng: ResMut<Rng>,
    gen: Res<GenState>,
    mut q: Query<(&mut PlantState, &PlantGenome, &Transform)>,
) {
    let mut count = q.iter().len();
    let mut births: Vec<(PlantGenome, Vec3)> = Vec::new();
    for (mut st, g, tf) in &mut q {
        st.mass += g.growth_rate() * DT;
        st.age += 1;
        if st.mass >= g.maturity && count + births.len() < PLANT_CAP && rng.f32() < P_REPRO {
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
    // reseed floor: keep a minimal seed bank so creatures can't drive food fully extinct
    let ntypes = gen.ntypes();
    while count + births.len() < PLANT_MIN {
        births.push((PlantGenome::random(&mut rng, ntypes), rand_pos(&mut rng, FOOD_Y)));
    }
    for (g, pos) in births {
        spawn_plant(&mut commands, g, PLANT_START_MASS, pos);
        count += 1;
    }
}

// --- per-tick life: sense -> think -> move -> eat -> metabolism -> learn ---
pub fn live_step(
    gen: Res<GenState>,
    mut rng: ResMut<Rng>,
    mut cq: Query<
        (&mut Transform, &mut Energy, &mut Fitness, &mut Heading, &mut Alive, &Genome, &mut Brain, &mut DietState, &mut Locomotion),
        (With<Creature>, Without<Food>),
    >,
    mut commands: Commands,
    fq: Query<(Entity, &Transform, &FoodKind, &PlantState, &PlantGenome), (With<Food>, Without<Creature>)>,
) {
    let dt = DT;
    let ntypes = gen.ntypes();
    // snapshot: (entity, pos, genome, mass). Genome carried so an eaten plant can disperse offspring (13).
    let foods: Vec<(Entity, Vec3, PlantGenome, f32)> = fq
        .iter()
        .map(|(e, t, _k, st, pg)| (e, t.translation, pg.clone(), st.mass))
        .collect();
    let mut eaten: HashSet<Entity> = HashSet::new();

    for (mut ct, mut energy, mut fit, mut head, mut alive, genome, mut brain, mut diet, mut loco) in &mut cq {
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

        // metabolism: basal + movement (convex in speed: rest cheap, sprint dear) + bite upkeep
        energy.0 -= (BASAL_COST + MOVE_COST * thrust * thrust + BITE_COST * genome.bite) * dt;

        // eat nearest plant on contact, IF bite beats its defense (arms race, see 13)
        let mut eat_reward = 0.0;
        if let Some((i, _)) = best {
            let (e, fp, mass) = (foods[i].0, foods[i].1, foods[i].3);
            let pg = foods[i].2.clone();
            if np.distance(fp) < EAT_RADIUS {
                let success = rng.f32() < sigmoid(BITE_K * (genome.bite - pg.defense));
                if success {
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
                    // overeating trade-off (12): energy is capped; eating while already full converts
                    // the excess into growth-load (harm) -> gorging shortens life. Eat best, in moderation.
                    if energy.0 > ENERGY_MAX {
                        let excess = energy.0 - ENERGY_MAX;
                        energy.0 = ENERGY_MAX;
                        diet.g += excess * OVEREAT_G;
                    }
                    commands.entity(e).despawn();
                    eaten.insert(e);
                    // endozoochory (13): an eaten plant may disperse a mutated offspring near the eater.
                    // Chance scales with quality -> tasty plants pay slower growth but win dispersal.
                    if foods.len() < PLANT_CAP && rng.f32() < pg.quality * SEED_VIA_GUT {
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
                // failed bite: the plant's defense held; it survives this contact
            }
        }

        // per-tick diet upkeep: aging, generalist overhead, growth-load decay, disease mortality (12).
        if gen.diet {
            diet.age += 1;
            let total_expr: f32 = diet.expr.iter().sum();
            energy.0 -= EXPR_OVERHEAD * total_expr * dt; // cost of keeping many genes expressed
            diet.g = (diet.g - G_DECAY).max(0.0);
            let age_frac = diet.age as f32 / AGE_SCALE;
            let aging = AGE_HAZARD * (age_frac / (age_frac + 1.0)); // rises then plateaus (~95 effect)
            let p_death = (aging + DISEASE_K * diet.g) * dt;
            if rng.f32() < p_death {
                alive.0 = false; // disease / old-age death
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
    pq: Query<(&PlantGenome, &PlantState)>,
    mut exit: MessageWriter<AppExit>,
) {
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
    let best = scored.first().map(|(f, _)| *f).unwrap_or(0.0);
    let avg_sensors: f32 = scored.iter().map(|(_, g)| g.n_sensors() as f32).sum::<f32>() / n as f32;
    let avg_bite: f32 = scored.iter().map(|(_, g)| g.bite).sum::<f32>() / n as f32;
    // plant (food) co-evolution stats: arms race = bite vs defense
    let plant_n = pq.iter().len().max(1);
    let avg_def: f32 = pq.iter().map(|(g, _)| g.defense).sum::<f32>() / plant_n as f32;
    let avg_nut: f32 = pq.iter().map(|(g, _)| g.nutrient).sum::<f32>() / plant_n as f32;
    let avg_qual: f32 = pq.iter().map(|(g, _)| g.quality).sum::<f32>() / plant_n as f32;
    if gen.diet {
        let avg_rig: f32 = scored.iter().map(|(_, g)| g.rigidity).sum::<f32>() / n as f32;
        info!("gen {:>3} | nutri {:>6.2} | sens {:.1} | rig {:.2} | bite {:.2} vs def {:.2} | plant-nut {:.2} qual {:.2} | roam {:.2} elev {:.1} | plants {}", gen.generation, avg, avg_sensors, avg_rig, avg_bite, avg_def, avg_nut, avg_qual, avg_roam, avg_elev, plant_n);
    } else {
        info!("gen {:>3} | food {:>6.2} | sens {:.1} | bite {:.2} vs def {:.2} | plant-nut {:.2} qual {:.2} | roam {:.2} elev {:.1} | plants {}", gen.generation, avg, avg_sensors, avg_bite, avg_def, avg_nut, avg_qual, avg_roam, avg_elev, plant_n);
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

    if gen.headless && gen.generation >= MAX_GEN_HEADLESS {
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
