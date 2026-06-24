// P2 morphology gym (--gym): drop one creature's developed body into an isolated avian-physics arena, step
// it headless + deterministically, and score locomotion. P2.2: the body is ARTICULATED -- each part is its
// own rigid body linked to its parent by a joint from the graph's JointSpec; hinge (limb) joints carry an
// AngularMotor whose target angle oscillates (a CPG gait), so the creature attempts to move itself. Score =
// horizontal COM travel. Builds its own PhysicsPlugins App -- the live planet (kinematic) never touches avian.
// Brain-driven control + lifetime learning come next (P2.3); this proves articulation + actuation + a scorable
// locomotion signal.
use avian3d::prelude::*;
use bevy::prelude::*;
use std::time::Duration;

use crate::morph::{develop, BodyGraph, JointKind, PlacedPart, ShapeKind};
use crate::rng::Rng;

const HZ: f64 = 120.0; // physics rate (fixed dt); enhanced-determinism + fixed step = reproducible
const DROP_H: f32 = 0.4; // start the body just above ground so it lands fast + spends the run moving
const CPG_FREQ: f32 = 2.2; // gait oscillation (Hz) of the limb-motor target angle
const MOTOR_STIFF: f32 = 40.0; // AccelerationBased motor: position tracking gain (mass-independent)
const MOTOR_DAMP: f32 = 8.0; // motor velocity damping
const MOTOR_MAX_TORQUE: f32 = 30.0; // clamp so a motor can't fling the body apart
// collision layers: creature parts collide ONLY with the ground, never with each other (ragdoll self-
// collision at the overlapping joints diverges the solver -> NaN). Limbs may interpenetrate; acceptable for
// a first gait, refine with selective self-collision later.
const L_GROUND: u32 = 1 << 0;
const L_BODY: u32 = 1 << 1;

#[derive(Resource)]
struct GymCfg {
    seed: u64,
}

// Per-hinge actuator: drive this joint's motor target angle as center + amp*sin(t). Phase offset per joint
// staggers limbs into a gait.
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
struct GymPart; // any creature body part (COM scoring)

// Body center + rotation + collider for a part, in body-local space. avian capsule/cuboid span local Y; a
// part's base sits at tf.translation with axis tf.rotation*+Y, so the shape CENTER is half a length up it.
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
    let mut rng = Rng::seed(cfg.seed);
    let body = BodyGraph::random(&mut rng);
    let pheno = develop(&body);

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

    // joints from each part to its parent, anchored at the child's base (the attach point on the parent)
    let mut acts: Vec<Actuator> = Vec::new();
    for (i, part) in pheno.parts.iter().enumerate() {
        let Some(p) = part.parent else { continue };
        let (cc, cr, _) = bodies[i];
        let (pc, pr, _) = bodies[p];
        let attach = part.tf.translation; // child base, body-local (lift cancels out in anchor deltas below)
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
                let amp = 0.85 * 0.5 * (part.joint.hi - part.joint.lo);
                acts.push(Actuator { joint: je, center, amp, phase: i as f32 * 0.7 });
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

pub fn run_gym(seed: u64, steps: u32) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(TransformPlugin)
        .add_plugins(PhysicsPlugins::default())
        .insert_resource(Gravity(Vec3::NEG_Y * 9.81))
        .insert_resource(GymCfg { seed })
        .init_resource::<Actuators>()
        .init_resource::<GymClock>()
        .add_systems(Startup, setup);
    app.finish();
    app.cleanup();
    app.update(); // run Startup (spawn arena + articulated body)

    let dt = Duration::from_secs_f64(1.0 / HZ);
    let world = app.world_mut();
    let start = com(world);
    for _ in 0..steps {
        drive_motors(world);
        world.resource_mut::<Time<Physics>>().advance_by(dt);
        world.run_schedule(PhysicsSchedule);
    }
    let end = com(world);
    let horiz = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
    println!(
        "gym: seed={} steps={} ({:.2}s) | start=({:.2},{:.2},{:.2}) end=({:.2},{:.2},{:.2}) | horiz_travel={:.2} | finite={}",
        seed, steps, steps as f64 / HZ, start.x, start.y, start.z, end.x, end.y, end.z, horiz, end.is_finite(),
    );
}

// Update each hinge motor's target angle for this step (CPG gait): center + amp*sin(t*2pi*freq + phase).
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

// Center of mass of all parts (avian Position = physics source of truth).
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
