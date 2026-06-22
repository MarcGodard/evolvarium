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
// Bevy ECS systems legitimately take many args + use complex query tuple types; silence that clippy noise.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
mod camera;
mod components;
mod config;
mod genome;
mod persist;
mod capture;
mod plant;
mod rng;
mod scenario;
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
    // --diverse: hand-seed a multi-niche showcase (swimmers/cold/warm/browsers placed in matching regions).
    // Auto-loads the showcase seed for COMPETENT brains, then overrides trait genes + placement per niche.
    let diverse = args.iter().any(|a| a == "--diverse");
    // --capture=PREFIX: GPU screenshot of the walk view -> PREFIX.png then exit. --cap-when picks the hour,
    // --cap-yaw the heading. Render mode (needs a GPU). Lets the real lighting/shadows be inspected offline.
    let capture = args.iter().find_map(|a| a.strip_prefix("--capture=").map(String::from));
    let cap_when = match args.iter().find_map(|a| a.strip_prefix("--cap-when=")).unwrap_or("morning") {
        "noon" => capture::CapWhen::Noon,
        "dusk" => capture::CapWhen::Dusk,
        "night" => capture::CapWhen::Night,
        _ => capture::CapWhen::Morning,
    };
    let cap_yaw = args.iter().find_map(|a| a.strip_prefix("--cap-yaw=").and_then(|s| s.parse::<f32>().ok())).unwrap_or(0.0);
    let cap_off = args.iter().find_map(|a| a.strip_prefix("--cap-off=").and_then(|s| s.parse::<i64>().ok())).unwrap_or(0);
    let cap_pitch = args.iter().find_map(|a| a.strip_prefix("--cap-pitch=").and_then(|s| s.parse::<f32>().ok())).unwrap_or(-0.35);
    // --cap-lat=DEG: aim the orbit camera straight down at this latitude (deg, + = north pole, - = south)
    // for a top-down pole view. Implies orbit. Pair with --cap-dist to frame the whole cap.
    let cap_lat = args.iter().find_map(|a| a.strip_prefix("--cap-lat=").and_then(|s| s.parse::<f32>().ok()));
    let cap_orbit = args.iter().any(|a| a == "--cap-orbit") || cap_lat.is_some();
    let cap_dist = args.iter().find_map(|a| a.strip_prefix("--cap-dist=").and_then(|s| s.parse::<f32>().ok())).unwrap_or(140.0);
    // --cap-water: stand the capture camera submerged in a deep ocean (verify swim view + underwater tint).
    let cap_water = args.iter().any(|a| a == "--cap-water");
    if diverse && load.is_none() && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --scenario=PATH / --out=PATH: tuning-harness mini-world runner (Layer 1). Runs one isolated cohort in a
    // controlled environment band, headless, then writes the result JSON + exits. See scenario.rs.
    let scenario_path = args.iter().find_map(|a| a.strip_prefix("--scenario=").map(String::from));
    let out_path = args.iter().find_map(|a| a.strip_prefix("--out=").map(String::from)).unwrap_or_else(|| "result.json".into());
    // Plant seed-bank library: a normal run seeds the planet biome-matched FROM it when the file exists.
    // --plant-lib=PATH overrides the default path; --no-plant-lib forces today's archetype seeding.
    const DEFAULT_PLANT_LIB: &str = "plant-library.json";
    let plant_lib = if args.iter().any(|a| a == "--no-plant-lib") {
        None
    } else {
        Some(args.iter().find_map(|a| a.strip_prefix("--plant-lib=").map(String::from)).unwrap_or_else(|| DEFAULT_PLANT_LIB.into()))
    };
    let seed_given = args.iter().any(|a| a.starts_with("--seed="));
    // --merge=result.json --niche=NAME: fold a scenario result's best genomes into the plant library, then
    // exit (no sim). The tuning-harness synthesize stage calls this per tuned cohort. --lib-cap caps per-niche.
    if let Some(rp) = args.iter().find_map(|a| a.strip_prefix("--merge=").map(String::from)) {
        let niche = args.iter().find_map(|a| a.strip_prefix("--niche=").map(String::from)).unwrap_or_else(|| "default".into());
        let cap = args.iter().find_map(|a| a.strip_prefix("--lib-cap=").and_then(|s| s.parse::<usize>().ok())).unwrap_or(8);
        let lib_path = plant_lib.clone().unwrap_or_else(|| DEFAULT_PLANT_LIB.into());
        scenario::merge_result_into_library(&rp, &niche, &lib_path, cap);
        return;
    }

    let mut app = App::new();
    // crisp directional shadows (default 2048 is soft at planet scale)
    app.insert_resource(bevy::light::DirectionalLightShadowMap { size: 4096 });
    app.insert_resource(rng::Rng::seed(seed));
    app.insert_resource(sim::Soil::new()); // dynamic soil-fertility grid (M5 nutrient loop)
    app.insert_resource(sim::GroundWater::new()); // dynamic rain-fed ground-water grid (rain cycle)
    app.insert_resource(sim::Climate::new()); // slow climate-memory grid (geological desert/rainforest drift)
    app.insert_resource(sim::Fire::new()); // lightning-ignited wildfire grid
    app.init_resource::<sim::Weather>(); // current rainfall intensity (storms onset + decay)
    app.init_resource::<sim::TreeBites>(); // per-tick fruit-tree grazing accumulator
    app.init_resource::<sim::SeedBank>(); // dormancy seed bank: buried seeds germinate later (plant_step)
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
        diverse,
        // Two-parent breeding (assortative mate choice + crossover) for creatures AND plants is the DEFAULT
        // now: --no-mating opts out (single-parent budding, e.g. clean generational runs). --mating / --sexual
        // kept as no-op back-compat aliases (the feature they named is already on).
        mating: !args.iter().any(|a| a == "--no-mating"),
        // --garden: seed a botanical showcase (one of every species in a grid at the homeland) for
        // inspecting the flora. Pair with --capture or just `cargo run -- --garden` to walk the garden.
        garden: args.iter().any(|a| a == "--garden"),
        plant_lib: plant_lib.clone(),
    });

    app.insert_resource(snapshot::ShotCfg { enabled: shots, at_tick: shot_tick, prefix: shot_prefix });

    // --scenario: tuning-harness branch. Parse the scenario JSON, override the RNG seed + GenState for an
    // isolated controlled mini-world, seed ONLY the cohort, run the minimal plant chain, write result, exit.
    if let Some(path) = scenario_path {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("scenario read failed ({}): {}", path, e));
        let scn: scenario::Scenario = serde_json::from_str(&text).unwrap_or_else(|e| panic!("scenario parse failed ({}): {}", path, e));
        let scen_seed = if seed_given { seed } else { scn.seed };
        let grazers = scn.world.grazers;
        app.insert_resource(rng::Rng::seed(scen_seed));
        // scenario GenState: headless clock, continuous OFF (so grazers don't reseed), no garden/library.
        app.insert_resource(sim::GenState {
            generation: 0,
            ticks_left: sim::GEN_TICKS,
            headless: true,
            learn: true,
            poison: false,
            diet: true,
            continuous: false,
            tick: 0,
            max_gens: 1,
            save: None,
            load: None,
            diverse: false,
            mating: false,
            garden: false,
            plant_lib: None,
        });
        app.insert_resource(scenario::ScenarioCfg { scenario: scn, out: out_path });
        app.init_resource::<scenario::ScenarioStats>();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::ZERO)))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_systems(Startup, scenario::spawn_scenario_world);
        // grazers => include the creature systems (forage + predation) so they actually graze the cohort.
        if grazers > 0 {
            app.add_systems(Update, (sim::live_step, sim::predation_step, sim::plant_step, sim::rot_step, scenario::scenario_step).chain());
        } else {
            app.add_systems(Update, (sim::plant_step, sim::rot_step, scenario::scenario_step).chain());
        }
        app.run();
        return;
    }

    if headless {
        // No window/render. Spin flat-out; each Update = one constant-dt sim step (fast-forward).
        // LogPlugin separately (MinimalPlugins omits it).
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::ZERO)))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_systems(Startup, sim::spawn_world_headless)
            .add_systems(
                Update,
                (snapshot::snapshot_capture, sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::grass_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
            );
    } else {
        // Real-time visuals: step in FixedUpdate at the sim rate so sim-time = wall-time.
        app.add_plugins(DefaultPlugins)
            .insert_resource(Time::<Fixed>::from_hz((1.0 / sim::DT) as f64))
            .add_plugins(camera::OrbitCameraPlugin)
            .add_plugins(viz::VizPlugin)
            .add_systems(Startup, (setup_scene, sim::spawn_world_render))
            .add_systems(
                FixedUpdate,
                (sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::grass_step, sim::plant_step, sim::rot_step, sim::generation_step).chain(),
            );
        if let Some(prefix) = capture {
            app.insert_resource(capture::CaptureCfg { prefix, when: cap_when, yaw: cap_yaw, off: cap_off, pitch: cap_pitch, orbit: cap_orbit, dist: cap_dist, underwater: cap_water, lat: cap_lat })
                .add_plugins(capture::CapturePlugin);
        }
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
    // The globe is a shadow caster in BOTH views (camera::update_planet_caster), so it shadows its own night
    // side / the terrain just past the horizon at dawn-dusk. It also RECEIVES, so trees/creatures drop crisp
    // real shadows on the land. Walk's curved-terrain self-shadow acne is held off by a higher per-mode
    // shadow_normal_bias (camera::update_shadow_mode) + the 4096 shadow map.
    commands.spawn((
        Mesh3d(meshes.add(terrain::build_globe(160))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::IDENTITY,
        viz::Planet, // shadow caster in both orbit + walk (night-side / terminator shadow)
    ));
    // ocean shell: a translucent blue sphere at sea level (land pokes above it, basins flood below). Glossy
    // (low roughness + high reflectance) so the sun glints off it; viz::animate_ocean breathes a slow swell.
    // sea surface = the waterline reference at PLANET_R: land (elevation >= 0) pokes above, ocean floor
    // (elevation < 0, signed bathymetry) sinks below -> the shell meets the globe exactly at the coast.
    let sea_r = sphere::PLANET_R;
    // Sea surface with baked per-vertex ice: cold polar ocean freezes to OPAQUE white pack ice, warmer seas
    // stay translucent blue. base_color = WHITE so each vertex color carries BOTH hue and alpha (ice opaque,
    // open water see-through). Latitude-driven (base_temperature), baked once at spawn -> no per-frame cost.
    let mut sea_mesh = Sphere::new(sea_r).mesh().ico(6).unwrap();
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
        sea_mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    {
        let water = Color::srgba(0.07, 0.26, 0.44, 0.62).to_linear().to_f32_array();
        let ice = Color::srgba(0.90, 0.93, 0.97, 1.0).to_linear().to_f32_array();
        let cols: Vec<[f32; 4]> = pos
            .iter()
            .map(|p| {
                let d = Vec3::from_array(*p).normalize_or_zero();
                // freeze ramps in below temp 0.30 (sea holds heat -> onset a touch warmer than land ice),
                // solid pack ice by ~0.12. smoothstep -> soft floe edge, not a hard ring.
                let f = ((0.30 - sphere::base_temperature(d)) / 0.18).clamp(0.0, 1.0);
                let f = f * f * (3.0 - 2.0 * f);
                [
                    water[0] + (ice[0] - water[0]) * f,
                    water[1] + (ice[1] - water[1]) * f,
                    water[2] + (ice[2] - water[2]) * f,
                    water[3] + (ice[3] - water[3]) * f,
                ]
            })
            .collect();
        sea_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cols);
    }
    commands.spawn((
        Mesh3d(meshes.add(sea_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colors carry water blue / ice white (+ alpha)
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.04, // glossy -> sharp sun specular glint
            reflectance: 0.6,
            ..default()
        })),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        viz::Ocean,
    ));
    // sun (directional light; direction set per-frame by day_night_lighting) + soft ambient so the night
    // side is not pitch black. shadows_enabled is toggled by camera::update_shadow_mode: OFF in orbit (the
    // shadow-range boundary showed as an "eclipse" disc when zoomed), ON in walk mode (eye-level horizon is
    // close so the range covers the whole view -> real shadows, no disc). Cascade tuned TIGHT for ground
    // scale (creatures ~0.5, trees ~3 units): the default config spreads its shadow map over a ~1000-unit
    // frustum, so at ground level the depth precision collapses and tree/creature shadows vanish. Pack 4
    // cascades within 130 units (first 12) so close-up shadows stay crisp; globe+ocean are NotShadowCaster
    // so only trees/creatures cast. shadow_normal_bias trims acne on the curved terrain receiver.
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 64_000.0, // daylight, dialed down ~36% from full-noon 100k (two 20% cuts) for a softer sun
            shadow_depth_bias: 0.04,
            shadow_normal_bias: 1.8,
            ..default()
        },
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: 0.3,
            maximum_distance: 130.0,
            first_cascade_far_bound: 12.0,
            overlap_proportion: 0.2,
        }
        .build(),
        Transform::from_xyz(1.0, 0.5, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
        // keep the light ViewVisible so its shadow cascades keep building even as day_night rotates it
        bevy::camera::visibility::NoFrustumCulling,
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
        bevy::light::NotShadowCaster, // celestial body: must not cast (it sits in the light path)
        viz::Moon,
    ));
    // visible sun disc: a bright emissive sphere far out along the sun direction (moved each frame). Sized
    // so its on-sky size ~matches the moon (the real Earth coincidence). Just the light source made visible.
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sphere::SUN_R).mesh().ico(3).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.95, 0.65),
            emissive: LinearRgba::rgb(9.0, 8.0, 4.0),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(sphere::sun_dir(0) * sphere::SUN_DIST),
        // CRITICAL: the sun disc sits far out ALONG the sun direction = directly between the directional
        // light and the planet. If it casts, its huge shadow blankets the whole lit hemisphere -> total
        // walk-mode blackout (no light = no drop shadows). NotShadowCaster is what makes ground shadows work.
        bevy::light::NotShadowCaster,
        viz::SunDisc,
    ));
    // starfield: evenly-spread points on a far shell (deterministic Fibonacci sphere), shared emissive mesh.
    let star_mesh = meshes.add(Sphere::new(9.0).mesh().ico(1).unwrap());
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(2.0, 2.0, 2.3),
        unlit: true,
        ..default()
    });
    let n_stars = 700usize;
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    for i in 0..n_stars {
        let y = 1.0 - (i as f32 + 0.5) / n_stars as f32 * 2.0;
        let r = (1.0 - y * y).max(0.0).sqrt();
        let theta = golden * i as f32;
        let dir = Vec3::new(theta.cos() * r, y, theta.sin() * r);
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_translation(dir * 7000.0),
            bevy::light::NotShadowCaster, // background stars never cast
        ));
    }
}
