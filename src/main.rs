// Evolvarium: neuroevolution + ecology sim. Spec 08-roadmap.md. Per-creature NN brains
// (genome = weights + sensors + traits) forage/eat/fight/breed; GA + lifetime learning evolve
// vs living food web on heightfield world w/ day/night + rain cycle.
// Two modes, one binary:
//   cargo run                                -> render: watch the world, fly + follow camera, inspect
//   cargo run -- --headless                  -> no window, fast-forward, logs per-generation stats, exits
//   cargo run -- --headless --diet --gens=N  -> N generations of the epigenetic-diet model
//   cargo run -- --headless --seed=7         -> reproducible run with a chosen seed
//   cargo run -- --headless --save=run.json  -> write fitness-ranked survivors + food web at run end
//   cargo run -- --load=run.json             -> resume from a saved population
// Bevy ECS systems take many args + complex query tuples. Silence clippy noise.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
mod camera;
mod components;
mod config;
mod genome;
mod niche;
mod orrery;
mod persist;
mod capture;
mod plant;
mod profile;
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
    // --profile: time each hot system over the run, print cumulative ranking periodically (Phase 0,
    // PARALLELIZATION.md). Headless only (perf target). Near-free otherwise (scope() = atomic load when off).
    if args.iter().any(|a| a == "--profile") {
        profile::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let learn = !args.iter().any(|a| a == "--nolearn"); // lifetime learning default ON
    let poison = args.iter().any(|a| a == "--poison"); // legacy: 2 food types (ntypes=2)
    // Epigenetic diet (NFOOD types + instincts) default ON. --no-diet = simple single-food world.
    // --poison forces legacy 2-type mode when --no-diet set.
    let diet = !args.iter().any(|a| a == "--no-diet") && !poison;
    // Continuous reproduction (self-sustaining birth/death past warm-up) is DEFAULT.
    // --generational opts into discrete-generation GA instead.
    let continuous = !args.iter().any(|a| a == "--generational");
    let seed = args
        .iter()
        .find_map(|a| a.strip_prefix("--seed=").and_then(|s| s.parse::<u64>().ok()))
        .unwrap_or(1);
    // --gens=N: headless run length in generations (default MAX_GEN_HEADLESS). Pair w/ --save for deep state.
    let max_gens = args
        .iter()
        .find_map(|a| a.strip_prefix("--gens=").and_then(|s| s.parse::<u32>().ok()))
        .unwrap_or(sim::MAX_GEN_HEADLESS);
    // --save=PATH writes survivors at run end. --load=PATH resumes saved population.
    let save = args.iter().find_map(|a| a.strip_prefix("--save=").map(String::from));
    let mut load = args.iter().find_map(|a| a.strip_prefix("--load=").map(String::from));
    // Render mode + no explicit --load: auto-load showcase seed if present so `cargo run` opens into
    // full breeding world (loaded continuous skips warm-up). --no-load forces fresh warm-up start.
    // Headless never auto-loads.
    const DEFAULT_SEED: &str = "evolved-continuous.json";
    if !headless && load.is_none() && !args.iter().any(|a| a == "--no-load") && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --shots[=PREFIX]: headless CPU snapshot of planet (several views -> PNG) then exit. Auto-loads
    // showcase seed if no --load. --shot-tick=N picks capture tick.
    let shots = args.iter().any(|a| a == "--shots" || a.starts_with("--shots="));
    let shot_prefix = args.iter().find_map(|a| a.strip_prefix("--shots=").map(String::from)).unwrap_or_else(|| "shot".into());
    let shot_tick = args.iter().find_map(|a| a.strip_prefix("--shot-tick=").and_then(|s| s.parse::<u32>().ok())).unwrap_or(3000);
    if shots && headless && load.is_none() && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --diverse: hand-seed multi-niche showcase (swimmers/cold/warm/browsers in matching regions).
    // Auto-loads showcase seed for competent brains, then overrides trait genes + placement per niche.
    let diverse = args.iter().any(|a| a == "--diverse");
    // --capture=PREFIX: GPU screenshot of walk view -> PREFIX.png then exit. --cap-when = hour,
    // --cap-yaw = heading. Render mode, needs GPU. Inspect real lighting/shadows offline.
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
    // --cap-lat=DEG: aim orbit camera straight down at this latitude (deg, + = north pole, - = south)
    // for top-down pole view. Implies orbit. Pair w/ --cap-dist to frame whole cap.
    let cap_lat = args.iter().find_map(|a| a.strip_prefix("--cap-lat=").and_then(|s| s.parse::<f32>().ok()));
    let cap_orbit = args.iter().any(|a| a == "--cap-orbit") || cap_lat.is_some();
    let cap_dist = args.iter().find_map(|a| a.strip_prefix("--cap-dist=").and_then(|s| s.parse::<f32>().ok())).unwrap_or(140.0);
    // --cap-water: submerge capture camera in deep ocean (verify swim view + underwater tint).
    let cap_water = args.iter().any(|a| a == "--cap-water");
    // --cap-warmup=N: sim frames before snapping (default 50). Raise for slow effects (fliers reaching cruise
    // altitude, land-wear trails forming). --cap-mmfield=N: open the minimap on overlay N (8 = wear) so a slow
    // live field can be screenshotted.
    let cap_warmup = args.iter().find_map(|a| a.strip_prefix("--cap-warmup=").and_then(|s| s.parse::<u32>().ok())).unwrap_or(capture::WARMUP);
    let cap_mmfield = args.iter().find_map(|a| a.strip_prefix("--cap-mmfield=").and_then(|s| s.parse::<usize>().ok()));
    if diverse && load.is_none() && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --scenario=PATH / --out=PATH: tuning-harness mini-world runner (Layer 1). Runs one isolated cohort in
    // controlled env band, headless, writes result JSON + exits. See scenario.rs.
    let scenario_path = args.iter().find_map(|a| a.strip_prefix("--scenario=").map(String::from));
    let out_path = args.iter().find_map(|a| a.strip_prefix("--out=").map(String::from)).unwrap_or_else(|| "result.json".into());
    // Plant seed-bank library: normal run seeds planet biome-matched FROM it when file exists.
    // --plant-lib=PATH overrides default path. --no-plant-lib forces archetype seeding.
    const DEFAULT_PLANT_LIB: &str = "plant-library.json";
    let plant_lib = if args.iter().any(|a| a == "--no-plant-lib") {
        None
    } else {
        Some(args.iter().find_map(|a| a.strip_prefix("--plant-lib=").map(String::from)).unwrap_or_else(|| DEFAULT_PLANT_LIB.into()))
    };
    let seed_given = args.iter().any(|a| a.starts_with("--seed="));
    // --merge=result.json --niche=NAME: fold scenario result's best genomes into plant library, then
    // exit (no sim). Harness synthesize stage calls this per tuned cohort. --lib-cap caps per-niche.
    if let Some(rp) = args.iter().find_map(|a| a.strip_prefix("--merge=").map(String::from)) {
        let niche = args.iter().find_map(|a| a.strip_prefix("--niche=").map(String::from)).unwrap_or_else(|| "default".into());
        let cap = args.iter().find_map(|a| a.strip_prefix("--lib-cap=").and_then(|s| s.parse::<usize>().ok())).unwrap_or(8);
        let lib_path = plant_lib.clone().unwrap_or_else(|| DEFAULT_PLANT_LIB.into());
        scenario::merge_result_into_library(&rp, &niche, &lib_path, cap);
        return;
    }
    // --merge-snapshot=run.json: harvest whole-planet co-evolution run's surviving plants into library,
    // biome-labeled (+ --niche-suffix, default "-coevo"), then exit. Pairs w/ --headless --save run.
    if let Some(sp) = args.iter().find_map(|a| a.strip_prefix("--merge-snapshot=").map(String::from)) {
        let cap = args.iter().find_map(|a| a.strip_prefix("--lib-cap=").and_then(|s| s.parse::<usize>().ok())).unwrap_or(8);
        let suffix = args.iter().find_map(|a| a.strip_prefix("--niche-suffix=").map(String::from)).unwrap_or_else(|| "-coevo".into());
        let lib_path = plant_lib.clone().unwrap_or_else(|| DEFAULT_PLANT_LIB.into());
        scenario::merge_snapshot_into_library(&sp, &lib_path, cap, &suffix);
        return;
    }
    // --merge-creatures=result.json --snap=PATH: harvest creature scenario result's best survivors into
    // population snapshot (showcase seed), accumulating across runs (--cap caps total), then exit.
    // Creature tuning workflow calls this per niche to build fresh evolved-continuous.json.
    if let Some(rp) = args.iter().find_map(|a| a.strip_prefix("--merge-creatures=").map(String::from)) {
        let snap_out = args.iter().find_map(|a| a.strip_prefix("--snap=").map(String::from)).unwrap_or_else(|| "evolved-continuous.json".into());
        let cap = args.iter().find_map(|a| a.strip_prefix("--cap=").and_then(|s| s.parse::<usize>().ok())).unwrap_or(90);
        scenario::merge_creatures_into_snapshot(&rp, &snap_out, cap);
        return;
    }

    let mut app = App::new();
    // 4096 shadow map: default 2048 too soft at planet scale.
    app.insert_resource(bevy::light::DirectionalLightShadowMap { size: 4096 });
    app.insert_resource(rng::Rng::seed(seed));
    app.insert_resource(sim::Soil::new()); // soil-fertility grid (M5 nutrient loop)
    app.insert_resource(sim::GroundWater::new()); // rain-fed ground-water grid
    app.insert_resource(sim::Climate::new()); // slow climate-memory grid (geological desert/rainforest drift)
    app.insert_resource(sim::Fire::new()); // lightning-ignited wildfire grid
    app.insert_resource(sim::Wear::new()); // trampling/compaction grid: creatures wear paths, ground heals slowly
    app.init_resource::<sim::Weather>(); // rainfall intensity (storm onset + decay)
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
        seed,
        max_gens,
        save,
        load,
        diverse,
        // Two-parent breeding (assortative mate choice + crossover) for creatures AND plants is DEFAULT.
        // --no-mating opts out (single-parent budding, e.g. clean generational runs). --mating / --sexual
        // are no-op back-compat aliases (feature already on).
        mating: !args.iter().any(|a| a == "--no-mating"),
        // --garden: seed botanical showcase (one of every species in grid at homeland) to inspect flora.
        // Pair w/ --capture or `cargo run -- --garden` to walk garden.
        garden: args.iter().any(|a| a == "--garden"),
        plant_lib: plant_lib.clone(),
        // --until-sustain: run continuous headless until every niche holds itself up (no rescue for a window),
        // then save best snapshot + exit. Evolves a balanced, self-sustaining world to load in the visualizer.
        until_sustain: args.iter().any(|a| a == "--until-sustain"),
        // --metrics=PATH: write niche balance result JSON at run end (the balance harness scores config tweaks from it)
        metrics: args.iter().find_map(|a| a.strip_prefix("--metrics=").map(String::from)),
    });
    app.init_resource::<niche::NicheBanks>();
    app.init_resource::<niche::NicheTracker>();

    app.insert_resource(snapshot::ShotCfg { enabled: shots, at_tick: shot_tick, prefix: shot_prefix });

    // --scenario: tuning-harness branch. Parse scenario JSON, override RNG seed + GenState for isolated
    // controlled mini-world, seed ONLY the cohort, run minimal plant chain, write result, exit.
    if let Some(path) = scenario_path {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("scenario read failed ({}): {}", path, e));
        let mut scn: scenario::Scenario = serde_json::from_str(&text).unwrap_or_else(|e| panic!("scenario parse failed ({}): {}", path, e));
        let scen_seed = if seed_given { seed } else { scn.seed };
        scn.seed = scen_seed; // stamp EFFECTIVE seed so result JSON echoes real run seed, not file default
        let grazers = scn.world.grazers;
        let has_creatures = grazers > 0 || !scn.creature_cohort.is_empty();
        app.insert_resource(rng::Rng::seed(scen_seed));
        // scenario GenState: headless clock, no garden/library. creature_cohort runs CONTINUOUS (past
        // warm-up) so cohort can BREED + evolve + test self-sustaining (like plant arm grows toward
        // target). Global reseed floor gated off in scenario mode (live_step). Plant-only stays
        // continuous-off (grazers don't reseed). generation = WARMUP_GENS so live_continuous fires at once.
        app.insert_resource(sim::GenState {
            generation: if has_creatures { sim::WARMUP_GENS } else { 0 },
            ticks_left: sim::GEN_TICKS,
            headless: true,
            learn: true,
            poison: false,
            diet: true,
            continuous: has_creatures && !scn.creature_cohort.is_empty(),
            tick: 0,
            seed: scen_seed,
            max_gens: 1,
            save: None,
            load: None,
            diverse: false,
            mating: false,
            garden: false,
            plant_lib: None,
            until_sustain: false,
            metrics: None,
        });
        app.insert_resource(scenario::ScenarioCfg { scenario: scn, out: out_path });
        app.init_resource::<scenario::ScenarioStats>();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::ZERO)))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_systems(Startup, scenario::spawn_scenario_world);
        // creatures present (creature_cohort OR grazers) => include creature systems (forage + predation).
        if has_creatures {
            app.add_systems(Update, (sim::live_step, sim::predation_step, sim::plant_step, sim::rot_step, scenario::scenario_step).chain());
        } else {
            app.add_systems(Update, (sim::plant_step, sim::rot_step, scenario::scenario_step).chain());
        }
        app.run();
        return;
    }

    if headless {
        // No window/render. Spin flat-out: each Update = one constant-dt sim step (fast-forward).
        // LogPlugin added separately (MinimalPlugins omits it).
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::ZERO)))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_systems(Startup, sim::spawn_world_headless)
            .add_systems(
                Update,
                (snapshot::snapshot_capture, sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::grass_step, sim::seaweed_step, sim::plant_step, sim::rot_step, niche::niche_step, sim::generation_step, profile_report).chain(),
            );
    } else {
        // Real-time visuals: step in FixedUpdate at sim rate so sim-time = wall-time.
        app.add_plugins(DefaultPlugins)
            .insert_resource(Time::<Fixed>::from_hz((1.0 / sim::DT) as f64))
            .add_plugins(camera::OrbitCameraPlugin)
            .add_plugins(viz::VizPlugin)
            .add_systems(Startup, (setup_scene, sim::spawn_world_render))
            .add_systems(
                FixedUpdate,
                (sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::grass_step, sim::seaweed_step, sim::plant_step, sim::rot_step, niche::niche_step, sim::generation_step).chain(),
            );
        if let Some(field) = cap_mmfield {
            app.insert_resource(viz::MinimapInitField(field)); // open minimap on a chosen overlay for the shot
        }
        if let Some(prefix) = capture {
            app.insert_resource(capture::CaptureCfg { prefix, when: cap_when, yaw: cap_yaw, off: cap_off, pitch: cap_pitch, orbit: cap_orbit, dist: cap_dist, underwater: cap_water, lat: cap_lat, warmup: cap_warmup })
                .add_plugins(capture::CapturePlugin);
        }
    }

    app.run();
}

// --profile: print cumulative per-system ranking every 600 ticks (~1/8 gen). Last tick before exit not
// guaranteed (exit fires in generation_step), but 600-tick cadence + flat-out headless gives a stable ranking.
fn profile_report(gen: Res<sim::GenState>) {
    if !profile::ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if gen.tick > 0 && gen.tick % 600 == 0 {
        profile::report(gen.tick);
    }
}

// Render-only scene dressing: planet globe, translucent ocean shell, sun light + moon.
// Sun direction + moon position animated each frame by viz::day_night_lighting.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // planet: elevation-displaced, biome-vertex-colored globe. White base_color lets vertex colors show.
    // Globe is shadow caster in BOTH views (camera::update_planet_caster) -> shadows own night side /
    // terrain past horizon at dawn-dusk. Also RECEIVES -> trees/creatures drop crisp shadows on land.
    // Walk's curved-terrain self-shadow acne held off by higher per-mode shadow_normal_bias
    // (camera::update_shadow_mode) + 4096 shadow map.
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
    // ocean shell: translucent blue sphere at sea level (land pokes above, basins flood below). Glossy
    // (low roughness + high reflectance) for sun glint. viz::animate_ocean breathes slow swell.
    // sea surface = waterline reference at PLANET_R: land (elevation >= 0) pokes above, ocean floor
    // (elevation < 0, signed bathymetry) sinks below -> shell meets globe exactly at coast.
    let sea_r = sphere::PLANET_R;
    // Sea surface w/ baked per-vertex ice: cold polar ocean freezes to OPAQUE white pack ice, warmer seas
    // stay translucent blue. base_color = WHITE so each vertex color carries BOTH hue + alpha (ice opaque,
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
                // freeze ramps below temp 0.30 (sea holds heat -> onset warmer than land ice),
                // solid pack ice by ~0.12. smoothstep -> soft floe edge, not hard ring.
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
    // sun (directional light; direction set per-frame by day_night_lighting). shadows_enabled toggled by
    // camera::update_shadow_mode: OFF in orbit (shadow-range boundary showed as "eclipse" disc when zoomed),
    // ON in walk (eye-level horizon close so range covers whole view -> real shadows, no disc). Cascade tuned
    // TIGHT for ground scale (creatures ~0.5, trees ~3 units): default config spreads shadow map over ~1000-
    // unit frustum, so at ground level depth precision collapses + tree/creature shadows vanish. Pack 4
    // cascades within 130 units (first 12) so close-up shadows stay crisp. globe+ocean are NotShadowCaster
    // so only trees/creatures cast. shadow_normal_bias trims acne on curved terrain receiver.
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 64_000.0, // daylight, ~36% below full-noon 100k (two 20% cuts) for softer sun
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
        // keep light ViewVisible so shadow cascades keep building as day_night rotates it
        bevy::camera::visibility::NoFrustumCulling,
        viz::SunLight,
    ));
    // moon: small emissive sphere; position set per-frame by day_night_lighting.
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sphere::MOON_R).mesh().ico(3).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.85, 0.9),
            emissive: LinearRgba::rgb(0.5, 0.5, 0.55),
            ..default()
        })),
        Transform::from_translation(sphere::moon_pos(0)),
        bevy::light::NotShadowCaster, // celestial body: must not cast (sits in light path)
        viz::Moon,
    ));
    // visible sun disc: bright emissive sphere far out along sun direction (moved each frame). Sized so
    // on-sky size ~matches moon (real Earth coincidence). Light source made visible.
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sphere::SUN_R).mesh().ico(3).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.95, 0.65),
            emissive: LinearRgba::rgb(9.0, 8.0, 4.0),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(sphere::sun_dir(0) * sphere::SUN_DIST),
        // CRITICAL: sun disc sits far out ALONG sun direction = directly between directional light + planet.
        // If it casts, huge shadow blankets whole lit hemisphere -> total walk-mode blackout (no light = no
        // drop shadows). NotShadowCaster makes ground shadows work.
        bevy::light::NotShadowCaster,
        viz::SunDisc,
    ));
    // starfield: evenly-spread points on far shell (deterministic Fibonacci sphere), shared emissive mesh.
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
    // aurora: high above each MAGNETIC pole at auroral magnetic latitude (~66 deg), oriented to
    // sphere::mag_pole_dir() (tilted off spin axis -> ovals sit OFF geographic poles). DIM base ring
    // (viz::update_aurora) glows under band of independently dancing CURTAIN segments
    // (viz::AuroraCurtain + update_aurora_curtains) that flicker, glide, sway, shift color randomly.
    let curtain_mesh = meshes.add(viz::aurora_curtain_mesh()); // crossed gradient curtain (green->crimson, soft)
    // deterministic per-curtain pseudo-random (no RNG state): hash int -> 0..1
    let h = |k: u32| {
        let x = (k.wrapping_mul(2_654_435_761) ^ 0x9e37_79b9) as f32;
        (x.sin() * 43_758.5453).fract().abs()
    };
    for (pi, s) in [1.0_f32, -1.0].into_iter().enumerate() {
        let axis = sphere::mag_pole_dir() * s; // magnetic pole (BOTH north + south get curtains)
        // dancing curtain segments around oval (transform set per-frame by update_aurora_curtains).
        // Dense + overlapping (soft-edged) so they blend into continuous folded sheet w/ ray structure.
        let n_cur = 72u32;
        for i in 0..n_cur {
            let k = i + pi as u32 * 977;
            let base = i as f32 / n_cur as f32 * std::f32::consts::TAU + h(k * 7) * 0.12;
            commands.spawn((
                Mesh3d(curtain_mesh.clone()),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::new(0.6, 0.6, 0.6, 0.5)),
                    alpha_mode: AlphaMode::Add,
                    unlit: true,
                    double_sided: true, // both crossed sheets visible any side
                    cull_mode: None,
                    ..default()
                })),
                Transform::default(),
                bevy::light::NotShadowCaster,
                viz::AuroraCurtain {
                    pole: axis,
                    ang: base,
                    drift: (h(k * 13) - 0.5) * 0.0008, // slow sideways glide, random dir + speed
                    phase: h(k * 17) * std::f32::consts::TAU,
                    width: 4.5 + h(k * 23) * 4.0, // uneven widths -> ray structure
                    freq: 0.02 + h(k * 29) * 0.05,
                },
            ));
        }
    }
}
