// P2 morphology gym (--gym): evolve creatures that move themselves in an isolated avian-physics arena.
// Each part is its own rigid body, linked to its parent by a joint from the graph's JointSpec; hinge limbs
// carry an AngularMotor driven by an evolvable CPG (per-joint gait_phase + amplitude from the genome, left/
// right anti-phase by body side). Fitness = horizontal COM travel (minus an airborne penalty so it rewards
// walking, not launching). `--gym-evolve` runs a GA over body+gait toward better movers. Builds its own
// PhysicsPlugins App per eval -- the live planet (kinematic) never touches avian. Brain/HyperNEAT real-time
// control is P3; this is the evolvable-CPG controller the plan flagged as the robust path.
use avian3d::prelude::*;
use bevy::prelude::*;
use std::time::Duration;

use crate::config::{MUT_RATE, MUT_STD};
use crate::genome::Genome;
use crate::morph::{develop, BodyGraph, JointKind, PlacedPart, ShapeKind};
use crate::rng::Rng;

const HZ: f64 = 120.0; // physics rate (fixed dt); enhanced-determinism + fixed step = reproducible
const DROP_H: f32 = 0.4; // start the body just above ground so it lands fast + spends the run moving
const CPG_FREQ: f32 = 2.2; // gait oscillation (Hz) of the limb-motor target angle
const MOTOR_STIFF: f32 = 40.0; // AccelerationBased motor: position-tracking gain (mass-independent)
const MOTOR_DAMP: f32 = 8.0; // motor velocity damping
const MOTOR_MAX_TORQUE: f32 = 30.0; // clamp so a motor can't fling the body apart
const L_GROUND: u32 = 1 << 0; // parts collide only with ground, never each other (ragdoll self-collision
const L_BODY: u32 = 1 << 1; //   at overlapping joints diverges the solver). Refine w/ selective later.

#[derive(Resource)]
struct GymCfg {
    body: BodyGraph,
}

#[derive(Resource, Default)]
struct Actuators(Vec<Actuator>);
struct Actuator {
    joint: Entity,
    center: f32,
    amp: f32,
    phase: f32,
}

#[derive(Resource, Default)]
struct GymClock(f32);

#[derive(Component)]
struct GymPart;

fn part_body(p: &PlacedPart) -> (Vec3, Quat, Collider) {
    let axis = p.tf.rotation * Vec3::Y;
    match p.shape {
        ShapeKind::Segment => (p.tf.translation + axis * (p.length * 0.5), p.tf.rotation, Collider::capsule(p.radius.max(0.04), p.length.max(0.05))),
        ShapeKind::Sphere => (p.tf.translation + axis * p.radius, p.tf.rotation, Collider::sphere(p.radius.max(0.04))),
        ShapeKind::Plate => {
            let hz = (0.18 * p.radius).max(0.02);
            (p.tf.translation + axis * (p.length * 0.5), p.tf.rotation, Collider::cuboid((2.0 * p.radius).max(0.05), p.length.max(0.05), 2.0 * hz))
        }
    }
}

fn local_axis(a: u8) -> Vec3 {
    match a {
        1 => Vec3::Y,
        2 => Vec3::Z,
        _ => Vec3::X,
    }
}

fn setup(mut commands: Commands, cfg: Res<GymCfg>) {
    commands.spawn((RigidBody::Static, Collider::cuboid(400.0, 2.0, 400.0), CollisionLayers::new(L_GROUND, L_BODY), Transform::from_xyz(0.0, -1.0, 0.0)));
    let pheno = develop(&cfg.body);
    let bodies: Vec<(Vec3, Quat, Collider)> = pheno.parts.iter().map(part_body).collect();
    let min_y = bodies.iter().map(|(c, _, _)| c.y).fold(f32::INFINITY, f32::min);
    let lift = DROP_H - min_y;

    let mut ents: Vec<Entity> = Vec::with_capacity(pheno.parts.len());
    for (c, r, col) in &bodies {
        let e = commands
            .spawn((RigidBody::Dynamic, col.clone(), CollisionLayers::new(L_BODY, L_GROUND), Transform::from_translation(*c + Vec3::Y * lift).with_rotation(*r), GymPart))
            .id();
        ents.push(e);
    }

    let mut acts: Vec<Actuator> = Vec::new();
    for (i, part) in pheno.parts.iter().enumerate() {
        let Some(p) = part.parent else { continue };
        let (cc, cr, _) = bodies[i];
        let (pc, pr, _) = bodies[p];
        let attach = part.tf.translation; // child base (lift cancels in the anchor deltas)
        let anchor_child = cr.inverse() * (attach - cc);
        let anchor_parent = pr.inverse() * (attach - pc);
        match part.joint.kind {
            JointKind::Hinge => {
                let ax = local_axis(part.joint.axis);
                let motor = AngularMotor::new(MotorModel::AccelerationBased { stiffness: MOTOR_STIFF, damping: MOTOR_DAMP }).with_max_torque(MOTOR_MAX_TORQUE);
                let je = commands
                    .spawn(
                        RevoluteJoint::new(ents[p], ents[i])
                            .with_local_anchor1(anchor_parent)
                            .with_local_anchor2(anchor_child)
                            .with_hinge_axis(ax)
                            .with_angle_limits(part.joint.lo, part.joint.hi)
                            .with_motor(motor),
                    )
                    .id();
                let center = 0.5 * (part.joint.lo + part.joint.hi);
                let amp = 0.85 * 0.5 * (part.joint.hi - part.joint.lo) * part.joint.motor; // genetic swing scale
                // genetic phase + left/right anti-phase (by body side) -> alternating gait for free
                let side = if part.tf.translation.x < -0.05 { std::f32::consts::PI } else { 0.0 };
                acts.push(Actuator { joint: je, center, amp, phase: part.joint.gait_phase + side });
            }
            JointKind::Universal => {
                commands.spawn(SphericalJoint::new(ents[p], ents[i]).with_local_anchor1(anchor_parent).with_local_anchor2(anchor_child));
            }
            JointKind::Fixed => {
                commands.spawn(FixedJoint::new(ents[p], ents[i]).with_local_anchor1(anchor_parent).with_local_anchor2(anchor_child));
            }
        }
    }
    commands.insert_resource(Actuators(acts));
}

struct GymResult {
    horiz: f32,
    end_y: f32,
    start_y: f32,
    finite: bool,
}

// Run ONE body in a fresh physics arena for `steps` and return locomotion metrics. Deterministic.
fn eval(body: &BodyGraph, steps: u32) -> GymResult {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(TransformPlugin)
        .add_plugins(PhysicsPlugins::default())
        .insert_resource(Gravity(Vec3::NEG_Y * 9.81))
        .insert_resource(GymCfg { body: body.clone() })
        .init_resource::<Actuators>()
        .init_resource::<GymClock>()
        .add_systems(Startup, setup);
    app.finish();
    app.cleanup();
    app.update(); // Startup: spawn arena + articulated body

    let dt = Duration::from_secs_f64(1.0 / HZ);
    let world = app.world_mut();
    let start = com(world);
    for _ in 0..steps {
        drive_motors(world);
        world.resource_mut::<Time<Physics>>().advance_by(dt);
        world.run_schedule(PhysicsSchedule);
    }
    let end = com(world);
    GymResult {
        horiz: ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt(),
        end_y: end.y,
        start_y: start.y,
        finite: end.is_finite(),
    }
}

// Fitness: reward horizontal travel, penalize ending well above the start height (launched/flipped, not
// walking); cull non-finite (blown-up) bodies.
fn fitness(r: &GymResult) -> f32 {
    if !r.finite {
        return -1000.0;
    }
    r.horiz - 0.5 * (r.end_y - r.start_y).max(0.0)
}

// Single inspection run: print metrics for the body grown from `seed`.
pub fn run_gym(seed: u64, steps: u32) {
    let mut rng = Rng::seed(seed);
    let g = Genome::random(&mut rng);
    let r = eval(&g.body, steps);
    println!(
        "gym: seed={} steps={} ({:.2}s) | horiz_travel={:.2} | start_y={:.2} end_y={:.2} | fitness={:.2} | finite={}",
        seed, steps, steps as f64 / HZ, r.horiz, r.start_y, r.end_y, fitness(&r), r.finite,
    );
}

// GA over body+gait: evaluate locomotion, keep elites, refill by clone+mutate. Best fitness should climb.
// Saves the best genome (serde) to `save` if given (a locomotion-specialist seed for the planet / further GA).
pub fn evolve_gym(pop_n: usize, gens: u32, steps: u32, seed: u64, save: Option<String>) {
    let mut rng = Rng::seed(seed);
    let mut pop: Vec<Genome> = (0..pop_n).map(|_| Genome::random(&mut rng)).collect();
    let elite_n = (pop_n / 4).max(1);
    for gen in 0..gens {
        let mut scored: Vec<(f32, usize)> = pop.iter().enumerate().map(|(i, g)| (fitness(&eval(&g.body, steps)), i)).collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let best = scored[0].0;
        let mean = scored.iter().map(|s| s.0).sum::<f32>() / pop_n as f32;
        println!("gym-evolve gen {gen}: best={best:.2} mean={mean:.2}");
        let elite: Vec<Genome> = scored.iter().take(elite_n).map(|(_, i)| pop[*i].clone()).collect();
        let mut next = elite.clone(); // elitism: best survive unmutated (next[0] = current best)
        while next.len() < pop_n {
            let p = &elite[(rng.f32() * elite_n as f32) as usize % elite_n];
            let mut c = p.clone();
            c.mutate(&mut rng, MUT_RATE, MUT_STD); // drifts body + gait (+ net/traits, used on the planet)
            next.push(c);
        }
        pop = next;
    }
    if let Some(path) = save {
        // export the top-K movers as a planet-loadable creatures-only snapshot (world None -> legacy load
        // scatters them by niche, regenerates plants). Re-score final pop to rank.
        let mut scored: Vec<(f32, usize)> = pop.iter().enumerate().map(|(i, g)| (fitness(&eval(&g.body, steps)), i)).collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let k = pop_n.min(16);
        let creatures: Vec<Genome> = scored.iter().take(k).map(|(_, i)| pop[*i].clone()).collect();
        let snap = crate::persist::Snapshot { generation: 0, creatures, plants: Vec::new(), world: None };
        crate::persist::save_snapshot(&path, &snap);
        println!("gym-evolve: saved top {k} movers -> {path}");
    }
}

fn drive_motors(world: &mut World) {
    let t = {
        let mut c = world.resource_mut::<GymClock>();
        c.0 += 1.0 / HZ as f32;
        c.0
    };
    let targets: Vec<(Entity, f32)> = {
        let acts = world.resource::<Actuators>();
        acts.0.iter().map(|a| (a.joint, a.center + a.amp * (t * std::f32::consts::TAU * CPG_FREQ + a.phase).sin())).collect()
    };
    for (je, target) in targets {
        if let Some(mut j) = world.get_mut::<RevoluteJoint>(je) {
            j.motor.target_position = target;
        }
    }
}

fn com(world: &mut World) -> Vec3 {
    let mut q = world.query_filtered::<&Position, With<GymPart>>();
    let mut sum = Vec3::ZERO;
    let mut n = 0u32;
    for p in q.iter(world) {
        sum += p.0;
        n += 1;
    }
    if n > 0 {
        sum / n as f32
    } else {
        Vec3::ZERO
    }
}
