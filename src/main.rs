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
mod snapshot;
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
    // --shots[=PREFIX]: headless CPU snapshot of the planet (several views -> PNG) then exit. Auto-loads
    // the showcase seed (a living world) if no --load given. --shot-tick=N picks when to capture.
    let shots = args.iter().any(|a| a == "--shots" || a.starts_with("--shots="));
    let shot_prefix = args.iter().find_map(|a| a.strip_prefix("--shots=").map(String::from)).unwrap_or_else(|| "shot".into());
    let shot_tick = args.iter().find_map(|a| a.strip_prefix("--shot-tick=").and_then(|s| s.parse::<u32>().ok())).unwrap_or(3000);
    if shots && headless && load.is_none() && std::path::Path::new(DEFAULT_SEED).exists() {
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

    app.insert_resource(snapshot::ShotCfg { enabled: shots, at_tick: shot_tick, prefix: shot_prefix });

    if headless {
        // No window/render. Spin flat-out; each Update = one constant-dt sim step (fast-forward).
        // LogPlugin separately (MinimalPlugins omits it).
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::ZERO)))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_systems(Startup, sim::spawn_world_headless)
            .add_systems(
                Update,
                (snapshot::snapshot_capture, sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
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

// Render-only scene dressing: the planet globe, a translucent ocean shell, the sun light + a moon. The
// sun direction + moon position are animated each frame by viz::day_night_lighting.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // planet: elevation-displaced, biome-vertex-colored globe. White base_color lets vertex colors show.
    commands.spawn((
        Mesh3d(meshes.add(terrain::build_globe(160))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::IDENTITY,
    ));
    // ocean shell: a translucent blue sphere at sea level (land pokes above it, basins flood below)
    let sea_r = sphere::PLANET_R + sphere::SEA_LEVEL * sphere::ELEV_MAX;
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sea_r).mesh().ico(6).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.10, 0.32, 0.52, 0.55),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.1,
            ..default()
        })),
        Transform::IDENTITY,
    ));
    // sun (directional light; direction set per-frame by day_night_lighting) + soft ambient so the night
    // side is not pitch black.
    commands.spawn((
        DirectionalLight { shadows_enabled: true, illuminance: 11000.0, ..default() },
        Transform::from_xyz(1.0, 0.5, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
        viz::SunLight,
    ));
    // moon: a small emissive sphere; position set per-frame by day_night_lighting.
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sphere::MOON_R).mesh().ico(3).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.85, 0.9),
            emissive: LinearRgba::rgb(0.5, 0.5, 0.55),
            ..default()
        })),
        Transform::from_translation(sphere::moon_pos(0)),
        viz::Moon,
    ));
}
