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
mod gym;
mod morph;
mod niche;
mod orrery;
mod orrery_view;
mod persist;
mod capture;
mod plant;
mod profile;
mod rng;
mod scenario;
mod sim;
mod snapshot;
mod sphere;
mod stars;
mod terrain;
mod viz;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

// CLI parse helpers. flag = bare `--name` present. val = raw str after `--key=`. parse_or = parsed w/ default.
fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
fn val<'a>(args: &'a [String], pfx: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(pfx))
}
fn parse_or<T: std::str::FromStr>(args: &[String], pfx: &str, default: T) -> T {
    val(args, pfx).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = flag(&args, "--headless");
    // --profile: time each hot system over the run, print cumulative ranking periodically (Phase 0,
    // PARALLELIZATION.md). Headless only (perf target). Near-free otherwise (scope() = atomic load when off).
    if flag(&args, "--profile") {
        profile::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let learn = !flag(&args, "--nolearn"); // lifetime learning default ON
    let poison = flag(&args, "--poison"); // legacy: 2 food types (ntypes=2)
    // Epigenetic diet (NFOOD types + instincts) default ON. --no-diet = simple single-food world.
    // --poison forces legacy 2-type mode when --no-diet set.
    let diet = !flag(&args, "--no-diet") && !poison;
    // Continuous reproduction (self-sustaining birth/death past warm-up) is DEFAULT.
    // --generational opts into discrete-generation GA instead.
    let continuous = !flag(&args, "--generational");
    let seed = parse_or(&args, "--seed=", 1u64);
    // --gens=N: headless run length in generations (default MAX_GEN_HEADLESS). Pair w/ --save for deep state.
    let max_gens = parse_or(&args, "--gens=", sim::MAX_GEN_HEADLESS);
    // --save=PATH writes survivors at run end. --load=PATH resumes saved population.
    let save = val(&args, "--save=").map(String::from);
    let mut load = val(&args, "--load=").map(String::from);
    // Render mode + no explicit --load: auto-load showcase seed if present so `cargo run` opens into
    // full breeding world (loaded continuous skips warm-up). --no-load forces fresh warm-up start.
    // Headless never auto-loads.
    // diverse co-evolved showcase: complex morph bodies (mean ~5.3 graph nodes) + 45 fliers + 55 swimmers + land,
    // genome-only so the world reseeds fresh + grows to carrying cap as you watch. Default opens WITH birds.
    // evolved-morph.json (curated 96, no fliers) kept as a --load alternative.
    const DEFAULT_SEED: &str = "evolved-showcase.json";
    if !headless && load.is_none() && !flag(&args, "--no-load") && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --shots[=PREFIX]: headless CPU snapshot of planet (several views -> PNG) then exit. Auto-loads
    // showcase seed if no --load. --shot-tick=N picks capture tick.
    let shots = flag(&args, "--shots") || args.iter().any(|a| a.starts_with("--shots="));
    let shot_prefix = val(&args, "--shots=").map(String::from).unwrap_or_else(|| "shot".into());
    let shot_tick = parse_or(&args, "--shot-tick=", 3000u32);
    if shots && headless && load.is_none() && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --diverse: hand-seed multi-niche showcase (swimmers/cold/warm/browsers in matching regions).
    // Auto-loads showcase seed for competent brains, then overrides trait genes + placement per niche.
    let diverse = flag(&args, "--diverse");
    // --capture=PREFIX: GPU screenshot of walk view -> PREFIX.png then exit. --cap-when = hour,
    // --cap-yaw = heading. Render mode, needs GPU. Inspect real lighting/shadows offline.
    let capture = val(&args, "--capture=").map(String::from);
    let cap_when = match val(&args, "--cap-when=").unwrap_or("morning") {
        "noon" => capture::CapWhen::Noon,
        "dusk" => capture::CapWhen::Dusk,
        "night" => capture::CapWhen::Night,
        _ => capture::CapWhen::Morning,
    };
    let cap_yaw = parse_or(&args, "--cap-yaw=", 0.0f32);
    let cap_off = parse_or(&args, "--cap-off=", 0i64);
    let cap_pitch = parse_or(&args, "--cap-pitch=", -0.35f32);
    // --cap-back=N: walk-shot side-vantage distance from homeland (default 22 = original wide framing). Small
    // (e.g. 4) = close-up on the founding creatures; large = wider establishing shot. Height scales with it.
    let cap_back = parse_or(&args, "--cap-back=", 22.0f32);
    // --cap-creature: aim the walk camera at the creature nearest homeland (close-up for creature visual work).
    let cap_creature = flag(&args, "--cap-creature");
    // --cap-lat=DEG: aim orbit camera straight down at this latitude (deg, + = north pole, - = south)
    // for top-down pole view. Implies orbit. Pair w/ --cap-dist to frame whole cap.
    let cap_lat = val(&args, "--cap-lat=").and_then(|s| s.parse::<f32>().ok());
    let cap_orbit = flag(&args, "--cap-orbit") || cap_lat.is_some();
    let cap_dist = parse_or(&args, "--cap-dist=", 140.0f32);
    // --cap-water: submerge capture camera in deep ocean (verify swim view + underwater tint).
    let cap_water = flag(&args, "--cap-water");
    // --cap-orrery: capture the TSN solar-system (orrery) view instead of the planet.
    let cap_orrery = flag(&args, "--cap-orrery");
    // --cap-warmup=N: sim frames before snapping (default 50). Raise for slow effects (fliers reaching cruise
    // altitude, land-wear trails forming). --cap-mmfield=N: open the minimap on overlay N (8 = wear) so a slow
    // live field can be screenshotted.
    let cap_warmup = parse_or(&args, "--cap-warmup=", capture::WARMUP);
    let cap_mmfield = val(&args, "--cap-mmfield=").and_then(|s| s.parse::<usize>().ok());
    if diverse && load.is_none() && std::path::Path::new(DEFAULT_SEED).exists() {
        load = Some(DEFAULT_SEED.to_string());
    }
    // --scenario=PATH / --out=PATH: tuning-harness mini-world runner (Layer 1). Runs one isolated cohort in
    // controlled env band, headless, writes result JSON + exits. See scenario.rs.
    let scenario_path = val(&args, "--scenario=").map(String::from);
    let out_path = val(&args, "--out=").map(String::from).unwrap_or_else(|| "result.json".into());
    // Plant seed-bank library: normal run seeds planet biome-matched FROM it when file exists.
    // --plant-lib=PATH overrides default path. --no-plant-lib forces archetype seeding.
    const DEFAULT_PLANT_LIB: &str = "plant-library.json";
    let plant_lib = if flag(&args, "--no-plant-lib") {
        None
    } else {
        Some(val(&args, "--plant-lib=").map(String::from).unwrap_or_else(|| DEFAULT_PLANT_LIB.into()))
    };
    let seed_given = args.iter().any(|a| a.starts_with("--seed="));
    // --merge=result.json --niche=NAME: fold scenario result's best genomes into plant library, then
    // exit (no sim). Harness synthesize stage calls this per tuned cohort. --lib-cap caps per-niche.
    if let Some(rp) = val(&args, "--merge=").map(String::from) {
        let niche = val(&args, "--niche=").map(String::from).unwrap_or_else(|| "default".into());
        let cap = parse_or(&args, "--lib-cap=", 8usize);
        let lib_path = plant_lib.clone().unwrap_or_else(|| DEFAULT_PLANT_LIB.into());
        scenario::merge_result_into_library(&rp, &niche, &lib_path, cap);
        return;
    }
    // --merge-snapshot=run.json: harvest whole-planet co-evolution run's surviving plants into library,
    // biome-labeled (+ --niche-suffix, default "-coevo"), then exit. Pairs w/ --headless --save run.
    if let Some(sp) = val(&args, "--merge-snapshot=").map(String::from) {
        let cap = parse_or(&args, "--lib-cap=", 8usize);
        let suffix = val(&args, "--niche-suffix=").map(String::from).unwrap_or_else(|| "-coevo".into());
        let lib_path = plant_lib.clone().unwrap_or_else(|| DEFAULT_PLANT_LIB.into());
        scenario::merge_snapshot_into_library(&sp, &lib_path, cap, &suffix);
        return;
    }
    // --merge-creatures=result.json --snap=PATH: harvest creature scenario result's best survivors into
    // population snapshot (showcase seed), accumulating across runs (--cap caps total), then exit.
    // Creature tuning workflow calls this per niche to build fresh evolved-continuous.json.
    if let Some(rp) = val(&args, "--merge-creatures=").map(String::from) {
        let snap_out = val(&args, "--snap=").map(String::from).unwrap_or_else(|| "evolved-continuous.json".into());
        let cap = parse_or(&args, "--cap=", 90usize);
        scenario::merge_creatures_into_snapshot(&rp, &snap_out, cap);
        return;
    }
    // --merge-snap=cohort.json --snap=seed.json: fold one gym cohort snapshot's creatures into a growing
    // creature seed (capped). Creature tune workflow synthesize stage calls this per niche cohort.
    if let Some(src) = val(&args, "--merge-snap=").map(String::from) {
        let snap_out = val(&args, "--snap=").map(String::from).unwrap_or_else(|| "evolved-morph.json".into());
        let cap = parse_or(&args, "--cap=", 96usize);
        scenario::merge_snapshot_creatures(&src, &snap_out, cap);
        return;
    }
    // --gym: P2 physics gym (avian). Drop one developed body into an isolated arena, step headless, report,
    // exit. --gym-seed picks the creature genome (default --seed); --gym-steps sets run length (120Hz).
    if flag(&args, "--gym") {
        let gym_seed = parse_or(&args, "--gym-seed=", seed);
        let gym_steps = parse_or(&args, "--gym-steps=", 600u32);
        if flag(&args, "--gym-evolve") {
            let pop = parse_or(&args, "--gym-pop=", 24usize);
            let gens = parse_or(&args, "--gym-gens=", 20u32);
            gym::evolve_gym(pop, gens, gym_steps, gym_seed, save.clone());
        } else {
            gym::run_gym(gym_seed, gym_steps);
        }
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
        mating: !flag(&args, "--no-mating"),
        // --garden: seed botanical showcase (one of every species in grid at homeland) to inspect flora.
        // Pair w/ --capture or `cargo run -- --garden` to walk garden.
        garden: flag(&args, "--garden"),
        plant_lib: plant_lib.clone(),
        // --until-sustain: run continuous headless until every niche holds itself up (no rescue for a window),
        // then save best snapshot + exit. Evolves a balanced, self-sustaining world to load in the visualizer.
        until_sustain: flag(&args, "--until-sustain"),
        // --metrics=PATH: write niche balance result JSON at run end (the balance harness scores config tweaks from it)
        metrics: val(&args, "--metrics=").map(String::from),
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
            .add_plugins(orrery_view::OrreryViewPlugin)
            .add_systems(Startup, (setup_scene, sim::spawn_world_render))
            .add_systems(
                FixedUpdate,
                (sim::weather_step, sim::fire_step, sim::live_step, sim::predation_step, sim::grass_step, sim::seaweed_step, sim::plant_step, sim::rot_step, niche::niche_step, sim::generation_step).chain(),
            );
        if let Some(field) = cap_mmfield {
            app.insert_resource(viz::MinimapInitField(field)); // open minimap on a chosen overlay for the shot
        }
        if let Some(prefix) = capture {
            app.insert_resource(capture::CaptureCfg { prefix, when: cap_when, yaw: cap_yaw, off: cap_off, pitch: cap_pitch, orbit: cap_orbit, dist: cap_dist, underwater: cap_water, lat: cap_lat, warmup: cap_warmup, orrery: cap_orrery, back: cap_back, focus_creature: cap_creature })
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
    mut images: ResMut<Assets<bevy::image::Image>>,
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
    // ocean: SINGLE OPAQUE sphere at sea level (land pokes above, basins flood below). One layer -> no
    // transparent-over-seabed sorting -> no shimmer (old translucent shell over the blue seabed double-layered
    // the water + flickered as the camera moved). Glossy (low roughness + high reflectance) for sun glint;
    // viz::animate_ocean breathes a slow swell. The depth gradient that used to live on the (now occluded)
    // seabed is baked onto THIS surface instead, so the ocean still reads shallow-cyan -> deep-navy.
    let sea_r = sphere::PLANET_R;
    // Per-vertex color baked once (no per-frame cost): ocean DEPTH gradient (matches sphere::biome_color ocean
    // branch so coast hue is continuous) + polar pack-ice white. Opaque now -> alpha unused (kept 1.0).
    let mut sea_mesh = Sphere::new(sea_r).mesh().ico(6).unwrap();
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
        sea_mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    {
        let shallow = Color::srgb(0.13, 0.40, 0.60).to_linear().to_f32_array();
        let deep = Color::srgb(0.02, 0.09, 0.28).to_linear().to_f32_array();
        let ice = Color::srgb(0.86, 0.90, 0.94).to_linear().to_f32_array();
        let cols: Vec<[f32; 4]> = pos
            .iter()
            .map(|p| {
                let d = Vec3::from_array(*p).normalize_or_zero();
                // depth 0 at coast .. 1 at abyssal center (elevation01 below SEA_LEVEL). Over land it clamps
                // to 0 (shallowest) but land pokes above -> those verts are hidden anyway.
                let depth = ((sphere::SEA_LEVEL - sphere::elevation01(d)) / sphere::SEA_LEVEL).clamp(0.0, 1.0);
                let mut c = [
                    shallow[0] + (deep[0] - shallow[0]) * depth,
                    shallow[1] + (deep[1] - shallow[1]) * depth,
                    shallow[2] + (deep[2] - shallow[2]) * depth,
                ];
                // pack ice: cold polar sea freezes (ramps below temp 0.30, matches biome_color ocean ice)
                let fi = ((0.30 - sphere::base_temperature(d)) / 0.30).clamp(0.0, 1.0);
                for k in 0..3 {
                    c[k] += (ice[k] - c[k]) * fi;
                }
                [c[0], c[1], c[2], 1.0]
            })
            .collect();
        sea_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cols);
    }
    commands.spawn((
        Mesh3d(meshes.add(sea_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colors carry the depth-graded water + ice
            alpha_mode: AlphaMode::Opaque, // single opaque layer -> cannot shimmer
            perceptual_roughness: 0.04, // glossy -> sharp sun specular glint
            reflectance: 0.6,
            ..default()
        })),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        viz::Ocean,
    ));
    // atmosphere rim: thin shell just above the surface, additive sky-blue, FRONT-culled (only far-side back
    // faces drawn). The opaque globe writes depth first, so the far shell is occluded EXCEPT the thin ring
    // peeking past the planet silhouette -> a soft blue limb halo, no tint over the disc. Orbit-view only
    // (viz::atmosphere_visibility). Per-VERTEX color (multiplies base) is set each frame by viz::update_atmosphere:
    // bright day-side limb, dim night airglow, warm twilight band -> a day-biased glow, not a uniform ring.
    let mut atmo_mesh = Sphere::new(sphere::PLANET_R * 1.055).mesh().ico(4).unwrap();
    let nverts = atmo_mesh.count_vertices();
    atmo_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.35f32, 0.55, 1.0, 1.0]; nverts]);
    commands.spawn((
        Mesh3d(meshes.add(atmo_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colors carry the day-biased blue glow
            unlit: true,
            alpha_mode: AlphaMode::Add,
            cull_mode: Some(bevy::render::render_resource::Face::Front),
            ..default()
        })),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        Visibility::Hidden, // shown only in orbit (atmosphere_visibility)
        viz::Atmosphere,
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
    // moon: UV sphere w/ procedural lunar texture, SUN-LIT for real PHASES (lit hemisphere bright, far side
    // dark). Faint emissive floor (carries the same texture) keeps the dark side a dim ghost, not a black void
    // nor a glowing ball -> when it crosses the sun it reads as a dark silhouette/eclipse, not a bright disc.
    // Tidally locked toward planet each frame (day_night_lighting look_at origin).
    let moon_tex = images.add(stars::moon_texture());
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sphere::MOON_R).mesh().uv(48, 24))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // texture carries albedo; full sun light -> bright lit crescent/face
            base_color_texture: Some(moon_tex.clone()),
            perceptual_roughness: 1.0,
            emissive: LinearRgba::rgb(0.06, 0.06, 0.07), // faint floor so dark side is a ghost, not black
            emissive_texture: Some(moon_tex),
            ..default()
        })),
        Transform::from_translation(sphere::moon_pos(0)),
        bevy::light::NotShadowCaster, // celestial body: must not cast (sits in light path)
        viz::Moon,
    ));
    // visible sun disc: bright emissive sphere far out along sun direction (moved each frame). Sized so
    // on-sky size ~matches the moon (real Earth coincidence + the eclipse geometry depends on it -> do NOT
    // enlarge). Bright warm-white emissive reads as a hot sun. Soft halo = the SunGlow billboard below (a
    // radial-gradient additive sprite), NOT solid additive shells (those render as flat discs that dwarf the
    // planet in orbit).
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(sphere::SUN_R).mesh().ico(3).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.98, 0.85),
            emissive: LinearRgba::rgb(20.0, 16.0, 9.0), // bright warm-white core
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
    // sun glow: camera-facing additive billboard (soft radial bloom) centered on the disc. Big quad, but the
    // texture's baked falloff keeps the transparent rim adding ~nothing -> a real halo, no flat-disc dwarfing.
    // Oriented + positioned each frame by viz::update_sun_glow. Quad lies in XY, normal +Z (billboarded there).
    let glow_tex = images.add(stars::sun_glow_texture());
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(sphere::SUN_R * 11.0, sphere::SUN_R * 11.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(glow_tex),
            unlit: true,
            alpha_mode: AlphaMode::Add, // additive bloom: brightens sky around sun, rim adds nothing
            cull_mode: None,
            ..default()
        })),
        Transform::from_translation(sphere::sun_dir(0) * (sphere::SUN_DIST * 0.985)),
        bevy::light::NotShadowCaster,
        viz::SunGlow,
    ));
    // starfield: the REAL Bright Star Catalog sky (same data as the orrery view), on a far shell. One mesh,
    // rotated each frame about the spin axis by viz::rotate_sky_stars so constellations wheel with the day.
    // Built in EQUATORIAL coords -> celestial pole = planet +Y; the ecliptic sun/moon/planets carry the
    // obliquity instead, so they drift against the fixed stars (= seasons + wandering planets).
    let sky_r = sphere::PLANET_R * 85.0; // ~6800, inside the 12k camera far clip
    let (sky_mesh, _hip) = stars::build_starfield(sky_r);
    commands.spawn((
        Mesh3d(meshes.add(sky_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colors carry per-star temperature; viz::fade_sky_stars dims at day
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add, // additive: stars glow over dark sky, vanish against bright day sky
            ..default()
        })),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        viz::SkyStars,
    ));
    // Milky Way: faint additive band along the real galactic plane, on a slightly farther shell. Tagged
    // SkyStars too -> wheels with the day + fades at midday like the stars.
    commands.spawn((
        Mesh3d(meshes.add(stars::build_milky_way(sky_r * 1.01))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        })),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        viz::SkyStars,
    ));
    // naked-eye wandering planets, positioned each frame by viz::position_sky_planets via the orrery model.
    let planet_mesh = meshes.add(Sphere::new(sky_r / 240.0).mesh().ico(3).unwrap());
    let sky_planets: [(usize, LinearRgba); 5] = [
        (orrery::MERCURY, LinearRgba::rgb(1.1, 1.0, 0.85)),
        (orrery::VENUS, LinearRgba::rgb(2.4, 2.2, 1.7)), // brightest "evening star"
        (orrery::MARS, LinearRgba::rgb(2.0, 0.7, 0.4)),
        (orrery::JUPITER, LinearRgba::rgb(1.8, 1.6, 1.25)),
        (orrery::SATURN, LinearRgba::rgb(1.7, 1.55, 1.1)),
    ];
    for (idx, col) in sky_planets {
        commands.spawn((
            Mesh3d(planet_mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                emissive: col,
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(0.0, sky_r * 0.97, 0.0),
            bevy::light::NotShadowCaster,
            viz::SkyPlanet { idx },
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
    // wildfire flames: a small pool of crossed-X flamelet clusters (additive emissive, aurora trick). Each slot
    // owns its OWN mesh so viz::fire_sheet_visuals can animate the tongues' lean per frame (only burning cells
    // pay the rebuild). Assigned to active fire cells each frame; hidden when fewer fires than slots.
    for i in 0..viz::FLAME_POOL {
        commands.spawn((
            Mesh3d(meshes.add(viz::flame_cluster_mesh())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::LinearRgba(LinearRgba::new(0.0, 0.0, 0.0, 0.0)),
                alpha_mode: AlphaMode::Add,
                unlit: true,
                double_sided: true, // both crossed sheets visible any side
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            viz::FlamePool(i),
        ));
    }
    // smoke: a matching pool of curling alpha-blend grey plumes (own mesh each so viz::smoke_visuals animates
    // the billow per frame). Assigned to the same active fire cells as the flames; hidden when fewer fires.
    for i in 0..viz::FLAME_POOL {
        commands.spawn((
            Mesh3d(meshes.add(viz::smoke_plume_mesh())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            viz::SmokePool(i),
        ));
    }
}
