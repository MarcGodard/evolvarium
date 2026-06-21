// Evolvarium — neuroevolution + ecology sim (see 08-roadmap.md). Tiny per-creature NN brains
// (genome = weights + sensors + traits) forage, eat, fight, breed; a GA + lifetime learning evolve
// them against a living, co-evolving food web on a heightfield world with day/night + a rain cycle.
// Two modes, one binary:
//   cargo run                                -> render: watch the world, fly + follow camera, inspect
//   cargo run -- --headless                  -> no window, fast-forward, logs per-generation stats, exits
//   cargo run -- --headless --diet --gens=N  -> N generations of the epigenetic-diet model
//   cargo run -- --headless --seed=7         -> reproducible run with a chosen seed
//   cargo run -- --headless --save=run.json  -> write fitness-ranked survivors + food web at run end
//   cargo run -- --load=run.json             -> resume from a saved population
mod camera;
mod components;
mod config;
mod genome;
mod persist;
mod plant;
mod rng;
mod sim;
mod sphere;
mod terrain;
mod viz;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let learn = !args.iter().any(|a| a == "--nolearn"); // lifetime learning on by default
    let poison = args.iter().any(|a| a == "--poison"); // legacy: two food types (sets ntypes=2)
    // Epigenetic diet model (NFOOD food types + instincts) is ON by default; --no-diet for the simple
    // single-food world. (--poison still forces the legacy 2-type mode when --no-diet is set.)
    let diet = !args.iter().any(|a| a == "--no-diet") && !poison;
    // Continuous reproduction (self-sustaining birth/death after a short warm-up) is the DEFAULT now
    // that it is stable + robust cross-seed. --generational opts into the discrete-generation GA instead.
    let continuous = !args.iter().any(|a| a == "--generational");
    let seed = args
        .iter()
        .find_map(|a| a.strip_prefix("--seed=").and_then(|s| s.parse::<u64>().ok()))
        .unwrap_or(1);
    // --gens=N: headless run length in generations (default 40). Use with --save to evolve a deep state.
    let max_gens = args
        .iter()
        .find_map(|a| a.strip_prefix("--gens=").and_then(|s| s.parse::<u32>().ok()))
        .unwrap_or(sim::MAX_GEN_HEADLESS);
    // --save=PATH writes survivors at run end; --load=PATH resumes from a saved population.
    let save = args.iter().find_map(|a| a.strip_prefix("--save=").map(String::from));
    let mut load = args.iter().find_map(|a| a.strip_prefix("--load=").map(String::from));
    // Render mode with no explicit --load: auto-load the showcase seed if present, so `cargo run` opens
    // straight into a full, breeding world (loaded continuous skips the warm-up) instead of replaying the
    // generational warm-up first. --no-load forces a fresh warm-up start; headless never auto-loads.
    const DEFAULT_SEED: &str = "evolved-continuous.json";
    if !headless && load.is_none() && !args.iter().any(|a| a == "--no-load") && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }

    let mut app = App::new();
    app.insert_resource(rng::Rng::seed(seed));
    app.insert_resource(sim::Soil::new()); // dynamic soil-fertility grid (M5 nutrient loop)
    app.insert_resource(sim::GroundWater::new()); // dynamic rain-fed ground-water grid (rain cycle)
    app.insert_resource(sim::Fire::new()); // lightning-ignited wildfire grid
    app.init_resource::<sim::Weather>(); // current rainfall intensity (storms onset + decay)
    app.init_resource::<sim::TreeBites>(); // per-tick fruit-tree grazing accumulator
    app.insert_resource(sim::GenState {
        generation: 0,
        ticks_left: sim::GEN_TICKS,
        headless,
        learn,
        poison,
        diet,
        continuous,
        tick: 0,
        max_gens,
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
                (sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
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
                (sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
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
        Mesh3d(meshes.add(terrain::build_mesh(span, 200))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    // translucent water surface: floods low basins below WATER_LEVEL (valleys/desert sinks)
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(span, span))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.18, 0.42, 0.62, 0.6),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.1, // calm, reflective
            ..default()
        })),
        Transform::from_xyz(0.0, terrain::WATER_LEVEL, 0.0),
    ));
    commands.spawn((
        DirectionalLight { shadows_enabled: true, ..default() },
        Transform::from_xyz(20.0, 40.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
