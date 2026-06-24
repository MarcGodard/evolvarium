// P2 morphology gym (--gym): drop one creature's developed body into an isolated avian-physics arena, step
// it headless + deterministically, report whether it falls + settles. SCAFFOLD: the creature is ONE compound
// rigid body (all parts fused) so this proves the physics integration end-to-end; articulated joints +
// actuators + a learned gait come in the next steps. Builds its own PhysicsPlugins App -- the live planet
// (kinematic) never touches avian.
use avian3d::prelude::*;
use bevy::prelude::*;
use std::time::Duration;

use crate::morph::{develop, BodyGraph, PlacedPart, ShapeKind};
use crate::rng::Rng;

const HZ: f64 = 120.0; // physics rate (fixed dt = 1/HZ); enhanced-determinism + fixed step = reproducible
const DROP_H: f32 = 2.0; // spawn the body this far above the ground so it falls + lands (proves contact)

#[derive(Resource)]
struct GymCfg {
    seed: u64,
}

#[derive(Component)]
struct Subject; // the creature body (single compound rigid body in the scaffold)

// avian capsule spans its local Y; `length` here = the part's axial extent (cylinder portion). Sub-shapes are
// placed in the body's local frame: a part's base sits at tf.translation and its axis is tf.rotation * +Y, so
// the shape CENTER is half a length up that axis.
fn part_subshape(p: &PlacedPart) -> (Vec3, Quat, Collider) {
    let axis = p.tf.rotation * Vec3::Y;
    match p.shape {
        ShapeKind::Segment => {
            let c = p.tf.translation + axis * (p.length * 0.5);
            (c, p.tf.rotation, Collider::capsule(p.radius.max(0.03), p.length.max(0.03)))
        }
        ShapeKind::Sphere => {
            // build_body_mesh centers the sphere at y=radius up the part axis
            let c = p.tf.translation + axis * p.radius;
            (c, p.tf.rotation, Collider::sphere(p.radius.max(0.03)))
        }
        ShapeKind::Plate => {
            let c = p.tf.translation + axis * (p.length * 0.5);
            let hz = (0.18 * p.radius).max(0.02); // PLATE_THICK * r (see morph)
            (c, p.tf.rotation, Collider::cuboid((2.0 * p.radius).max(0.03), p.length.max(0.03), 2.0 * hz))
        }
    }
}

fn setup(mut commands: Commands, cfg: Res<GymCfg>) {
    // static ground at y=0 (top face), thick slab below
    commands.spawn((RigidBody::Static, Collider::cuboid(400.0, 2.0, 400.0), Transform::from_xyz(0.0, -1.0, 0.0)));
    // evolve nothing: just develop a random body and fuse its parts into one compound collider
    let mut rng = Rng::seed(cfg.seed);
    let body = BodyGraph::random(&mut rng);
    let pheno = develop(&body);
    let shapes: Vec<(Vec3, Quat, Collider)> = pheno.parts.iter().map(part_subshape).collect();
    // lift so the lowest point starts above the ground -> it falls + lands
    let min_y = shapes.iter().map(|(c, _, _)| c.y).fold(f32::INFINITY, f32::min);
    let lift = DROP_H - min_y + 1.0;
    commands.spawn((
        RigidBody::Dynamic,
        Collider::compound(shapes),
        Transform::from_xyz(0.0, lift, 0.0),
        Subject,
    ));
}

// Headless deterministic run: build a MinimalPlugins + PhysicsPlugins app, run Startup once, then advance the
// PhysicsSchedule by a fixed dt N times (avian's documented manual-stepping pattern). Returns final body
// center height + total settle displacement.
pub fn run_gym(seed: u64, steps: u32) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(TransformPlugin)
        .add_plugins(PhysicsPlugins::default())
        .insert_resource(Gravity(Vec3::NEG_Y * 9.81))
        .insert_resource(GymCfg { seed })
        .add_systems(Startup, setup);
    app.finish();
    app.cleanup();
    app.update(); // run Startup (spawn arena + subject); physics not auto-stepped (no FixedUpdate time yet)

    let dt = Duration::from_secs_f64(1.0 / HZ);
    let world = app.world_mut();
    let start_y = subject_pos(world).y;
    let mut last = subject_pos(world);
    let mut path = 0.0f32;
    for _ in 0..steps {
        world.resource_mut::<Time<Physics>>().advance_by(dt);
        world.run_schedule(PhysicsSchedule);
        let p = subject_pos(world);
        path += p.distance(last);
        last = p;
    }
    let end = subject_pos(world);
    println!(
        "gym: seed={} steps={} ({:.2}s @ {}Hz) | start_y={:.2} end_y={:.2} drop={:.2} | end=({:.2},{:.2},{:.2}) | path={:.2} | finite={}",
        seed,
        steps,
        steps as f64 / HZ,
        HZ as u32,
        start_y,
        end.y,
        start_y - end.y,
        end.x,
        end.y,
        end.z,
        path,
        end.is_finite(),
    );
}

// Read avian's Position (physics source of truth); Transform isn't synced when we run PhysicsSchedule alone.
fn subject_pos(world: &mut World) -> Vec3 {
    let mut q = world.query_filtered::<&Position, With<Subject>>();
    q.iter(world).next().map(|p| p.0).unwrap_or(Vec3::ZERO)
}
