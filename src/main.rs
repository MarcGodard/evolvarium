// Evolvarium M1 — proof-of-life foraging sim (see 08-roadmap.md).
// Tiny NN brains (genome = weights) forage for food; generational GA selects by food eaten.
// Two modes, one binary:
//   cargo run                       -> render: watch creatures forage, fly camera
//   cargo run -- --headless         -> no window, fast, logs avg/best food per generation, exits
//   cargo run -- --headless --seed=7 -> reproducible run with a chosen seed
//   cargo run -- --headless --save=run.json -> write fitness-ranked survivors at run end
//   cargo run -- --headless --load=run.json -> resume from a saved population (BACKLOG P2)
//
// fields.rs (gravity/zone fields) returns at M4; not wired this milestone.
mod camera;
mod components;
mod genome;
mod persist;
mod plant;
mod rng;
mod sim;
mod terrain;
mod viz;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let learn = !args.iter().any(|a| a == "--nolearn"); // lifetime learning on by default
    let poison = args.iter().any(|a| a == "--poison"); // two food types, nutritious flips per gen
    let diet = args.iter().any(|a| a == "--diet"); // epigenetic diet model (NFOOD types)
    let shift = args.iter().any(|a| a == "--shift"); // food availability changes each generation
    let seed = args
        .iter()
        .find_map(|a| a.strip_prefix("--seed=").and_then(|s| s.parse::<u64>().ok()))
        .unwrap_or(1);
    // --save=PATH writes survivors at run end; --load=PATH resumes from a saved population.
    let save = args.iter().find_map(|a| a.strip_prefix("--save=").map(String::from));
    let load = args.iter().find_map(|a| a.strip_prefix("--load=").map(String::from));

    let ntypes: u8 = if diet { genome::NFOOD as u8 } else if poison { 2 } else { 1 };

    let mut app = App::new();
    app.insert_resource(rng::Rng::seed(seed));
    app.insert_resource(sim::Soil::new()); // dynamic soil-fertility grid (M5 nutrient loop)
    app.insert_resource(sim::GenState {
        generation: 0,
        ticks_left: sim::GEN_TICKS,
        headless,
        learn,
        poison,
        good_type: 0,
        diet,
        shift,
        avail: (0..ntypes).collect(), // gen 0: all types available
        save,
        load,
    });

    if headless {
        // No window/render. Spin flat-out; each Update = one constant-dt sim step (fast-forward).
        // LogPlugin separately (MinimalPlugins omits it).
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::ZERO)))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_systems(Startup, sim::spawn_world_headless)
            .add_systems(
                Update,
                (sim::live_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
            );
    } else {
        // Real-time visuals: step in FixedUpdate at the sim rate so sim-time = wall-time.
        app.add_plugins(DefaultPlugins)
            .insert_resource(Time::<Fixed>::from_hz((1.0 / sim::DT) as f64))
            .add_plugins(camera::FlyCameraPlugin)
            .add_plugins(viz::VizPlugin)
            .add_systems(Startup, (setup_scene, sim::spawn_world_render))
            .add_systems(
                FixedUpdate,
                (sim::live_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
            );
    }

    app.run();
}

// Render-only scene dressing: ground plane sized to the arena + a light.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let span = sim::WORLD_HALF * 2.0 + 4.0;
    // heightfield terrain (P3): elevation-shaded landscape (vertex colors), white base lets them show
    commands.spawn((
        Mesh3d(meshes.add(terrain::build_mesh(span, 140))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    commands.spawn((
        DirectionalLight { shadows_enabled: true, ..default() },
        Transform::from_xyz(20.0, 40.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
