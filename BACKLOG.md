# Backlog / roadmap

Single worktree (`evolvarium`, branch `main`); commit here, backup `git push origin main:build`.
One coherent item per change: implement, keep the build green, verify headless, commit. Balance constants
live in `config.rs`; the live conversion plan is `SPHERE-PLAN.md`.

## Done

### Solar system + sky: real Tychos model (2026-06-24)
Full design `~/Documents/Github/clients/evolvarium/15-solar-system-tychos.md`. Data copied from
pholmq/TSN (GPL-2.0) @ commit 49fd49c (pinned in `orrery.rs` + `stars.rs` comments).
- [x] **Shooting stars (2026-06-24)**: occasional meteors streak the night sky (`viz::meteor_visuals`,
  gizmo gradient streaks). Stateless + tick-deterministic like rain; 7 sparse slots; night-only in walk,
  always in orbit, off in orrery. Render-only.
- [x] **`orrery.rs`** — byte-exact TSN body table (celestial-settings.json: sun, planets, moons, Pluto,
  Halley, Eros via nested deferent/epicycle chain) + geocentric sky dirs. Pure, unit-tested.
- [x] **Sirius-binary precession** — Earth's PVP orbit made elliptical (period 24000 yr, e=0.0404, apsides
  aimed at Sirius). One ellipse reconciles: ~26000-yr *apparent* precession (apoapsis), 360-day mean year
  breathing ~332..390 d with Sirius distance, whole-system rate k=omega/n.
- [x] **Earth calendar + real sun/moon** (`sphere.rs`): `t_years`, `ecliptic_to_sky`, `fmt_date`/
  `fmt_age_days`; `sun_dir`/`moon_dir` delegate to the model. Seasons now a real ~360-day year (sub-solar
  latitude drift); climate band + pop preserved. Wet/dry season phase-locked to the year.
- [x] **Orrery view** (`orrery_view.rs`, `camera.rs`): TAB cycles Orbit -> Orrery -> Walk. Renders all TSN
  bodies at model positions; own lively clock; `--cap-orrery` for offline shots. Minimap orbit-only.
- [x] **Real starfield** (`stars.rs`): TSN Bright Star Catalog (8227 stars, BSC.json) colored by temperature
  + constellation lines (off by default, L toggles). One combined mesh. Used by BOTH the orrery and the
  planet sky (orbit + walk) — stars wheel with the day; naked-eye planets (`SkyPlanet`) drift along the zodiac.
- [x] **Eclipses**: sun disc sized to the moon's angular size, so the moon transits the sun; solar eclipse
  dims the sun toward twilight, lunar eclipse reddens the moon (blood moon). Analytic, tested geometry.
- [x] **Orrery overlays** (orrery-mode only, mode-gated keys): T orbit/deferent traces, G ecliptic grid,
  Z zodiac, B body+bright-star labels (on by default), L constellations (off by default). Legend moved to J;
  H is the master HUD toggle; planet HUD hidden in orrery.
- [x] **HUD calendar**: WORLD panel shows the full Earth date + clock (Yr/Mon/Day HH:MM), season, and the
  next solar/lunar eclipse countdown; creature age in days. (Dropped the sirius/year-length line.)
- [x] **Fluffier clouds (2026-06-24)**: cloud puffs are now multi-lobe cumulus CLUSTERS (merged sphere
  lobes, `viz::cloud_puff_mesh`) that self-shade into 3D billows instead of flat lozenges, matte, with
  daylight-aware albedo (bright white high sun, golden where the sun grazes them at sunrise/sunset, dim
  moonlit floor at night).
- [x] **Sky polish (2026-06-24)**: stars + Milky Way are additive and fade with local daylight (midday ->
  only the brightest survive; orbit/orrery keep full); procedural Milky Way band on the real galactic plane
  (`stars::build_milky_way`, galactic->equatorial transform). Moon got a procedural cratered texture
  (`stars::moon_texture`) on a UV sphere, tidally locked toward the planet, SUN-LIT for real phases (lit
  crescent + dark-side ghost; reads as a silhouette when transiting the sun). Sun disc brightened/whiter.
- [x] **Sun glow + night-cloud fix (2026-06-24)**: real soft sun halo via a camera-facing additive
  radial-gradient billboard (`stars::sun_glow_texture` + `viz::SunGlow`/`update_sun_glow`) that blooms warm
  at dawn/dusk and dims during eclipses, replacing the removed solid corona shells (which dwarfed the planet
  in orbit). Cloud night side no longer paints brown discs from orbit: warmth now gates to the lit hemisphere
  only and night clouds fade to dim cool grey + more transparent.
- [x] **Bodies visible at any zoom**: constant-angular-size rendering so the moon/Phobos/Deimos/asteroids
  stay visible; orrery min-zoom 6 so you can fly in and resolve the moon by Evolvarium.
- [x] **Sirius as a body** on the precession apsidal line (bright blue-white), labeled + click-identifiable.
- [x] **Click-to-identify** sky objects (bodies, sun, moon, Sirius, nearest catalog star with HR/mag/temp)
  in orrery + planet sky; ground creatures/plants keep the inspector. (Fixed: minimap 2nd camera broke
  cam.single() in pick_on_click + capture force_cam.)
- [x] **Geocentric by default**: orrery centers on the home planet, renamed **Evolvarium** (display only);
  C toggles to the system center.
- [x] **Orrery time = sim time** so orrery, planet sky, and calendar agree on the moment.
- [ ] **Open polish**: free-look/pan in the orrery (vs orbit-locked); richer identity panels (constellation
  membership, distance). (Home planet now reads "Evolvarium" in the identity panel too, not just the orrery.)

### The planet (spherical world)
- [x] Spherical sim: great-circle movement, localized homeland start that spreads, lon/lat spatial grids,
      positional day/night, sphere terrain/ocean/temperature/moisture fields (`sphere.rs`).
- [x] Live globe render: elevation-displaced biome globe, ocean shell, orbiting sun (visible disc) + moon,
      drifting cloud puffs, cloud-driven rain streaks, surface fire, starfield, orbit camera.
- [x] Headless CPU snapshot renderer (`--shots`) -> PNG views (no GPU); creatures colored by thermal gene.
- [x] Climate: cold poles / warm equator, latitude moisture belts; weather = drifting clouds + ~10%
      cloud-only rain (no global storms). Lightning -> wildfire (re-enabled, flammability fixed: see the
      Wildfire fix entry under World/visuals).

### Biology
- [x] Continuous reproduction (default, stable cross-seed ~70-90 carrying capacity); generational GA via
      `--generational`. Warm-up then emergent birth/death; density-dependent; reseed floor.
- [x] Senescence: creatures age and die of old age (was disabled in continuous -> fixed).
- [x] Epigenetic diet model (NFOOD types, expression, rigidity, growth-load disease), arms-race bite vs
      plant defense, predation (needs a real combat edge), kin-based social/herding, scavenging on carrion.
- [x] Food web: plants, fruit trees (reach-gated, dispersed when eaten), evergreens, carrion->poison rot
      chain, soil-fertility nutrient loop.
- [x] Save/load populations (`--save`/`--load`, best-healthy-snapshot); render auto-loads the showcase seed.
- [x] **Full top-to-bottom world save/load (2026-06-23)**: `--save` now writes the EXACT world (every
      creature + plant where it stands: pos/alt/heading/energy/diet/fitness; trees/carrion/ferment/fruit;
      the dynamic field grids soil/groundwater/climate/fire/WEAR; seed bank; weather; tick) under a new
      `Snapshot.world`. `--load` RESTORES it verbatim (continues from saved tick, wear/fields intact) instead
      of the lossy scatter. Grass/seaweed carpets regenerate on load (wear re-bares trodden cells at once).
      Backward compatible: old seeds (no `world`) fall back to the legacy scatter path. So a worn world can be
      saved + reloaded for an instant screenshot (no long warmup). persist::WorldState + sim::{collect_full_snapshot,
      restore_full_world}. Verified: save@tick9600 -> reload resumes @9601 with wear 0.048 (vs fresh 0.001), pop 1100 exact.
- [x] **In-game save key + save-on-exit (2026-06-23)**: press **O** in the windowed sim to write the full
      current world to `savestate.json` (or the `--save` path), reload with `--load=savestate.json`. Closing
      the window with `--save=PATH` set ALSO writes the full world (sim::save_on_window_close). Headless
      `--save` already writes on programmatic run-end (--gens done / extinction). Read-only god-controls
      reusing the headless --save builders (sim::do_full_save). Legend + help updated. (Caveat: headless
      Ctrl-C/SIGINT kills the process before the save runs; use --gens for a clean save-on-exit.)

### Evolvable genes (creature)
- [x] sensors (angle+range, add/remove), brain (weights + per-connection plasticity + hidden-layer size),
      diet expr0, rigidity, bite, height, size, swim, social, **temp_pref** (thermal niche),
      **longevity** (lifespan vs upkeep), **metab** (frugal/sluggish vs fast/costly), **parental** (r/K),
      **alpine** (mountain niche: cheap rock-crossing vs heavy-build flat-ground penalty; mirror of swim;
      self-limiting since mountains ~5% of world so won't peg like armor; serde default 0 = neutral).
- Plants/trees: kind, nutrient, defense, quality, wet, height, light_pref, regrow, branches, spread, maturity.
      **wet now gates water survival** (drown mortality x submersion x (1-wet)): land flora drowns in the
      sea, aquatic flora (high wet) thrives -> wet splits land vs aquatic plants. Trees are land-only.

### Combat brain outputs M6 (2026-06-22)
- [x] **Brain 2 -> 6 outputs**: `[thrust, turn, attack, defend, eat, sprint]`. attack/defend/eat/sprint are
      0..1 sigmoid intents learned in-life (reward-modulated) + selected across generations. `forward()`
      activates them with a loop; `learn()` already loops ho generically.
- [x] **NN-driven predation**: removed the hunger gate; a creature hunts iff its brain raises `attack` past
      `ATTACK_INTENT_THRESH`. Committing costs `ATTACK_COST` land-or-miss; a kill grants `R_KILL`, a whiff
      `R_WASTE` (so pointless aggression is selected against -- the stabilizer that replaced the well-fed skip).
- [x] **Active defense**: `defend` (brace) adds `BRACE_DEF` to effective defense in predation but immobilizes
      (`BRACE_DRAG` on movement); surviving an attack while braced earns `R_DEFEND`. Passive evasion softened
      (`CLIMB_EVADE` 0.5->0.35, `SOCIAL_SAFETY` 0.7->0.5) so fighting/defending competes with flee/hide.
- [x] **Eat-gate** (`eat`): ingestion is now a choice (gate 0.3, below the 0.5 founder baseline so founders
      still feed) -> the brain can refuse toxic/unripe/spoiled food. **Sprint**: burst speed for chase OR flee,
      paid in energy + fatigue.
- [x] **Seed migration**: old 2-output nets pad to 6 ho rows on load (`pad_net_ho`/`pad_plast_ho`), new
      outputs biased to safe defaults (combat/sprint OFF, eat ON) so a loaded pre-combat seed behaves as
      before; unit-tested. Verified: scenario w/ predator cohort is stable (no cannibalism crash) + carnivory
      drifts up (predation emerging). Retune via `tools/retune-combat.workflow.js` (predator pressure per niche).

### Flight + depth: vertical DOF (birds + diving fish) (2026-06-22)
- [x] **Unified vertical axis** (`Locomotion.alt`): one signed-free offset above terrain serves BOTH media.
      `surface_pos` rides terrain + alt; since `elevation()` is signed bathymetry (waterline at PLANET_R),
      a swimmer sits above the SEAFLOOR, so depth = rising off the seafloor through the water column is the
      SAME positive axis as a flier rising into the sky. Ceiling by medium: fliers `MAX_FLIGHT_ALT*flight`,
      swimmers the local water column `-elevation`.
- [x] **`flight` gene** (`#[serde(default)]`, ~15% of founders are true fliers so the niche is visible gen 0).
      Above `FLIGHT_KNEE` the creature leaves the ground; mirror of swim (fast aloft, clumsy grounded).
- [x] **7th brain output** `[...,climb]`: out[6] = rise/sink intent. `OUTPUT_MIGRATE_BIAS` extended (climb
      biased negative) so old saved nets migrate grounded. New brain INPUT `altitude` (GLOBAL_INPUTS 11->12,
      appended last so `pad_ih_inputs` aligns old nets). Both migrations unit-tested.
- [x] **Neutral buoyancy** (`FLIGHT_BUOYANCY`/`FLIGHT_CRUISE`): fliers hover aloft + fish hover mid-water even
      with a neutral brain (visible birds/fish); brain climbs/descends around cruise, landing-to-eat = sustained
      descend. Costs: `FLIGHT_ALT_COST` (hold altitude) + `FLIGHT_GROUND_COST` (clumsy wings grounded).
- [x] **No vertical gates needed**: eating/predation/collision all key off 3D translation, so an airborne flier
      is auto > EAT_RADIUS/ATTACK_RADIUS from ground stuff (flight = real escape valve, fruit reach from above).
      Only drowning (distance-independent hard kill) gated by altitude (fliers cross open ocean unharmed).
- [x] **Render** (`viz.rs`): bird body-plan branch (`flight>0.5`) = swept flat wings + tucked legs; altitude
      renders for free (sim writes Transform.translation). Verified `--capture` (wings + birds aloft + fish).
      Pop stable: fresh fliered world `--gens=3` and migrated saved snapshot both hold ~70-90.
- [x] **Showcase seed predates flight** (DONE 2026-06-24): default `cargo run` now loads `evolved-showcase.json`
      -- a `--diverse` gens=8 co-evolve thinned to a genome-only seed (200 = 45 fliers up to flight 0.92 +
      55 swimmers + 100 land), complex morph bodies (mean ~5.3 graph nodes), reseeds a fresh world that grows
      to carrying cap. Default world opens WITH birds. `evolved-morph.json` kept as a curated `--load` alt.
- [x] **Bird looks (2026-06-22)**: flapping wings (`Wing` comp + `flap_wings` rotates each about the shoulder
      root on the forward axis; freq from body `size` -> hummingbird flutter vs hawk beats), dedicated swept
      tapered wing mesh (`wing_mesh`, double-sided), bird tail fan, new cosmetic `beak` gene (forward cone =
      beak on birds / snout on others, backfilled by `ensure_cosmetic`). Wingspan scales with flight gene + size.
- [x] **Vertical-medium invariant locked (2026-06-23)**: extracted the inline ceiling logic into pure
      `sim::vertical_envelope(flight, swim, elev)` + `sim::can_drown(flight, swim, alt)`, unit-tested. Guarantees
      fliers reach the sky over ANY terrain (never trapped on ground/water), swimmers are ground-pinned over dry
      land (ceil 0 -> can't rise into air), walkers grounded everywhere, and the drown kill spares fliers
      (float like a duck) + swimmers. Refactor only, behavior identical.
- [ ] **Balance-phase follow-ups** (visuals-first now): dive-hunting tuning, flier predator niche, flock/school
      cohesion at altitude, HUD flier count.
- [x] **FLAGSHIP: generative morphology + embodied evolution (Karl-Sims part-graph). P1+P2 DONE + MERGED to
      `main`** (DEFAULT_SEED = evolved-morph.json). Only P3 deferred (below). Plan:
      `~/.claude/plans/cuddly-coalescing-bachman.md`. Replaces the per-scalar
      "make elongate/tail/fin pay rent" idea with a far bigger move the user signed off on: an indirect
      part-graph genome that GROWS open-ended bodies (recursion + reflection + repetition) so morphology is
      discovered, not declared. Two-tier: physics + learned gaits in an isolated harness gym (P2); the live
      1000-creature planet renders the evolved bodies + derives movement stats from geometry (no live physics).
      - [x] **P1.1** `src/morph.rs`: `BodyGraph` (nodes=parts, edges=attach/recurse/reflect/joint), `develop()`,
        `Morphometrics` (mass/reach/areas/limbs), bounded by `MAX_PARTS`. Pure + tested.
      - [x] **P1.2** `Genome.body: BodyGraph` (serde default = capsule -> old saves load unchanged); founders
        random, mutate drifts, crossover inherits.
      - [x] **P1.3** generative merged mesh per body (`build_body_mesh`, hash-keyed LRU cache `BodyMeshCache`)
        replaces the fixed-part assembler; emissive eyes from the body bbox. Creatures render as varied bodies.
      - [x] **P1.4** geometry-derived stats coupled to fitness (gentle, no-free-lunch): legs->land traction,
        fins->swim thrust, frontal area->water drag, wings->flight lift relief, part-count+mass->basal/move
        cost, body reach->browse height. Carrying capacity holds (~1100 stable).
      - [x] **P1.5** seed round-trip verified: evolved a fresh pop (96k ticks, pop ~1070 stable) -> saved
        `evolved-morph.json` -> loaded -> body graphs develop + render (novel bodies, e.g. radial-spiked).
        Seed gitignored (41MB full-world save; reflects UNTUNED balance) + DEFAULT_SEED swap DEFERRED until
        after P2 balance retune, then commit the tuned showcase seed.
      - [x] **P2.1** avian3d gym (`--gym`): body drops into an isolated physics arena, deterministic fixed
        step, settles. (avian 0.6 = bevy 0.18; 0.7 needs bevy 0.19.)
      - [x] **P2.2** articulated body: per-part rigid bodies + joints from JointSpec; hinge limbs driven by an
        AngularMotor CPG; self-collision filtered; horizontal-COM locomotion score.
      - [x] **P2.3** gym evolution loop (`--gym-evolve`): evolvable gait genes (phase/amp per joint), GA selects
        for locomotion (elitism), best mover saved. Reward-hacking guard: cull divergent bodies.
      - [x] **P2.4** harness + tuned showcase: `--gym-evolve --save` exports a planet-loadable cohort;
        `--merge-snap` folds cohorts into a creature seed; `tools/tune-locomotion.workflow.js` (agent per
        body-niche). Showcase = gym-evolved COMPLEX movers -> seed planet -> short ecological tuning (8 gens,
        bodies stay rich: sz 0.42 vs 0.20 for pure-30-gen) -> `evolved-morph.json`, now the DEFAULT_SEED. Pop
        sustains healthy (~1043); creatures render varied + complex (limbs, eyes, herding).
      - [ ] **P3** HyperNEAT-style CPPN brain scaling controller to body; optional live spotlight physics.
        (Deferred: not needed for working tuned creatures; the planet is kinematic + uses geometry stats.)
      NOTE (user 2026-06-24): "as much if not more variety than earth", "go full out". MERGED to `main`
      (sign-off given); metaball skin then wraps the evolved bodies in one tight surface (see World/visuals).

### Magnetic field + magnetoreception (2026-06-22)
- [x] **Tilted geomagnetic dipole** (`sphere.rs`): `MAG_TILT` + `mag_pole_dir` (magnetic north ~11.5 deg off
      the spin axis); `mag_field` / `mag_latitude` (inclination "map" cue) / `mag_north_bearing` (compass,
      nonzero declination under the tilt) / `mag_intensity`. Pure + unit-tested.
- [x] **`magneto` sense gene** (serde default 0 = off; old saves unchanged): a soft-knee switch
      (`mag_expression`, smoothstep 0.2..0.6) gating 2 new brain inputs (magnetic latitude + compass heading)
      for navigation/homing. `GLOBAL_INPUTS` 9 -> 11; old nets zero-padded on load (migration unit-tested,
      seed still loads). No free lunch: `MAG_COST` basal upkeep scaled by expression -> sense kept only where
      it pays off. Selection is emergent (a fresh harness seed could exploit it later).
- [x] **Aurora** (`main.rs`/`viz.rs`): an emissive `Aurora` ring around each magnetic pole at ~66 deg magnetic
      latitude, oriented to the tilted pole (sits OFF the geographic pole), shimmering green<->magenta +
      brighter on the night side (`update_aurora`). Verified via `--capture` (oval offset from the geo pole).

### Creature overhaul M4 (2026-06-22)
- [x] **12 new genes** (all serde-default, save-safe): detox, carnivory, pelt, armor, venom (physiology/
      defense); limbs, climb, eyes, head (morphology); skin_hue, skin_sat, pattern (appearance). Each has a
      benefit + a cost (no free lunch) and, where it makes sense, a visible phenotype.
- [x] **Toxic load** (`DietState.toxic_load`): ingested toxins (toxic plants, rotten meat, fermented
      spoilage, venomous prey) accumulate + linger, draining energy + driving disease + a death hazard,
      cleared slowly (faster with `detox`). Replaces the old instant-only toxin hit.
- [x] **Rabbit starvation**: meat energy gated by the `carnivory` gut; processing protein with an empty carb
      buffer makes metabolic toxic load -> an all-meat creature with no plant carbs poisons + starves.
- [x] **size = energy use**: basal upkeep scales allometrically with body size (size^1.5).
- [x] Gene effects wired: pelt insulation/heat/water, armor predation defense, venom predator deterrent,
      limbs land traction, climb predator-evasion + fruit-tree reach, eyes detection bonus, head cheaper brain.
- [x] **New brain inputs** (4 -> 9 globals): own toxic_load, canopy shade, nearest-threat dist+bearing, in-
      water. Need-for-shade has teeth (open-sun heat cost relieved by canopy). Old saved nets zero-padded on
      load (`Genome::ensure_net_shape`); migration unit-tested.
- [x] **Genetic creature visuals**: composed body (skin color from genes, venom warning tint, fur/armor
      shading), head sphere (size + pattern two-tone), 1..6 eyes, 2..8 splayed legs — all from the genome.
- [x] **Tuning-harness creature arm**: `creature_cohort` (overrides + reflex presets), patch placement,
      continuous in-scenario breeding, survival/master/trait-drift metrics + best survivors;
      `--merge-creatures` harvests winners into a population snapshot. `tools/tune-creatures.workflow.js`
      fans out one tuner per niche. Fresh 120-creature, 10-biome `evolved-continuous.json` built from it.

### Metabolism overhaul (2026-06-21, a42d7aa)
- [x] **Three energy stores** (`Energy{fast,sugar,fat}`): burn order fast->sugar->fat; fast leaks even at
      rest (volatile, can't bank); fat mobilizes slow (`power()` caps thrust -> fat-only is sluggish) +
      carries upkeep; sugar overflow converts to fat at a loss. New `adiposity` gene (lean/nimble vs
      fatty/buffered). Plants give sugar; meat -> fat. Verified: pop stable, adiposity under selection.
- [x] **Fruit + fermentation food web** (forageable `fast`): mature fruit trees drop fallen fruit; fruit +
      dead-plant detritus ferment over the Rot clock (fresh->sugar, fermenting->fast/ethanol+toxin,
      spoiled->toxic/gone). `Ferment{toxic}` marker splits plant matter from animal carrion. Fruit ferments
      richly + low-tox, detritus scraps + high-tox. DEFERRED: viz fruit-on-crown + falling-fruit render.
- [x] **10 nutrients + regulatory diet genome** ("10 genes feed 1"): `uptake[10]` genes feed a computed
      `master_expression` (reserves vs uptake demand) that gates energy extraction; plants produce
      `nutrients[10]`+`toxicity` (x soil fertility), growth pays for both; `DietState.reserves[10]` top up
      on eat (x uptake) + deplete with use -> deficiency raises growth-load (soft). `UPTAKE_OVERHEAD` taxes
      broad guts. NFOOD=4 kept as plant-FAMILY axis (sensing/color) only. Verified: pop 50-86, diet breadth
      evolves 9.7->6.5. KNOWN-SOFT: master expr pegs ~0.99 (reserve gradient too gentle) -> friction F1
      (`clients/evolvarium/tuning-frictions.md`) for the tuning harness.

### Grass: whole-planet ground cover + whole-planet trees (2026-06-21)
- [x] **Grass = render-only ground cover** (`Grass` marker + `PlantGenome::grass`): a "lesser plant"
      (one nutrient vs ~3-4, low energy, defenseless, flat, high-regrow turf). Carries NO `Food`, so it
      stays OUT of the per-tick food clone/sensing entirely -> 8000 tufts cost ~nothing in the sim
      (putting grass in the food scan both crushed perf and crashed foraging). Own lifecycle
      (`grass_step`) + own cap (`GRASS_CAP`); seeds/persists only where `plant_habitability >
      GRASS_HAB_MIN` ("soil capable of plants"), dies on fire/drown/poor-soil, refills each tick.
      WHOLE-PLANET: `grass_pos` samples the full sphere (not homeland).
- [x] **Edible as a thin POSITION-based fallback** (`GRASS_GRAZE`): since grass is not an entity in
      the food scan, a HUNGRY creature (`energy < START_ENERGY`) standing on grass-bearing soil nibbles
      a small sugar trickle (x local habitability). Hunger-gated so it neither distracts foraging (grass
      is never sensed) nor force-feeds the full. NOTE: fine population balance deferred (per owner); the
      sim currently runs hot -- revisit `GRASS_GRAZE` + tree caps when tuning.
- [x] **Visualized as 3D blades** (`viz::grass_tuft_mesh`): one shared clump of 11 thin, tall, pointed,
      curved blade strips + one green double-sided material for ALL tufts. `add_grass_visuals` sizes each
      tuft ONCE at attach (static -> no per-frame cost): LENGTH + thickness are WATER-driven (moisture,
      so coastal/edge grass grows tall + lush; dry interior short), gated by habitability; rooted + stood
      on the surface normal. ROOT-CAUSE fix: `add_plant_visuals` was grabbing grass (it has no `Food`
      now, plus a `Without<Grass>` guard) and rendering it as plant domes -> grass looked like "blobs."
- [x] **Trees now whole-planet**: `spawn_trees` always scatters worldwide (`rand_pos`); `N_TREES`
      240, `TREE_CAP` 480 so forests fill the globe (ambient reproduction tops up).
- [ ] **Rebalance population** for the heavier whole-planet world (grass graze + tree caps); deferred.

## Fundamentals retrofit (chemistry + physics) — IN PROGRESS

Plan: `~/.claude/plans/steady-wishing-swan.md`. Principle: **matter is closed, energy flows through**.
Roadmap: fundamentals -> living ocean -> parasites. Scale decision: 1 world unit = 1 metre, SI throughout.

- [x] **Phase 0**: baseline captured (seeds 1/5/9); pre-retrofit `plants` sat pinned at `PLANT_CAP`.
- [x] **Phase 1 chemistry** (`src/chem.rs`): conserved C/N/P reservoirs (soil mineral/organic, atmosphere,
      buried sediment, rock), Liebig's law of the minimum replacing the fertility multiplier, bacteria+fungi
      as a Q10 decomposition rate law, real N fixation / P weathering / burial / uplift. Plants now DRAW from
      the budget; death, fire (real combustion, N volatilizes and P stays as ash), grazing, and reproduction
      all route through reservoirs. Standing biomass reaches 0.40 kg/m2 against the real temperate grassland
      band of 0.5-1, and nitrogen is the limiting element as in real temperate systems.
- [x] `PLANT_CAP` 4000 -> 12000, redefined as a PERF ceiling. Counts sitting exactly on it mean the chemistry
      is not limiting, which is a bug rather than a reason to raise it.
- [~] **Phase 2 fauna** (in progress): creature mass calibrated from the ecology (`BODY_MASS_PER_MORPH`),
      tissue accounted at its own composition (`tissue_comp`: carrion is animal flesh, ~7x the N of litter),
      and a world-level `Biosphere.fauna_pool` holding assimilated-but-unbuilt matter. Eating now SPLITS at
      `ASSIMILATION` (0.2): a small share is retained as animal tissue, the rest respired and excreted. That
      split IS the trophic step and is what keeps the pyramid upright without a cap. Births DRAW from the
      pool, so when assimilated nitrogen runs out reproduction fails: animal carrying capacity becomes a
      consequence of the food web rather than of `CREATURE_CAP`. Death now returns the creature's ACTUAL mass
      (was a `CARRION_MASS` constant). Chose a single shared pool over per-creature body stocks deliberately:
      per-individual stores would need a component, a grant system, and claim dedup on every eating path, to
      buy condition that `Energy` already models, while the constraint that matters is population-level.
      Full food-web circuit asserted matter-neutral end to end in `chem.rs` tests.
- [ ] **Phase 2 remaining**: verify drift in CONTINUOUS mode (all runs so far sat inside the 12-gen
      generational warm-up, where a dead creature keeps its entity as a recyclable placeholder AND spawns
      carrion, so the sim double-counts by design and the ledger cannot read clean). Then thermodynamics
      (Kleiber, body-temperature heat balance), mechanics (drag, Archimedes, lift), stellar radiation.

- [x] **Phase 3 thermodynamics** (`src/thermo.rs`): Kleiber `B = a*M^0.75` and a real heat balance
      (Stefan-Boltzmann radiation, conductance with insulation in series, Meeh surface area) replace the
      invented `SIZE_BASAL_EXP`, `TEMP_COST * mismatch`, and `PELT_COLD_RELIEF`. Thermal performance curves
      make endothermy buy BREADTH, not speed. Diurnal temperature (~14 K land, ~2 K sea, 2 h lag) gave that
      breadth something to be worth: mean endothermy 0.11 -> 0.43.
- [x] **Carrying capacity stopped being a constant (2026-08-09)**: intake now scales as `M^0.75`, the same
      exponent as Kleiber, so a bite and the metabolism it funds move together. Before, cost scaled with mass
      while a bite paid the same at any size, so small was strictly cheaper with nothing pushing back.
      Population went from pinned at `CREATURE_CAP` (5000) to ~150 set by the food web, and mean body mass
      0.032 -> 0.24 kg. Applied to bite MASS, not just energy, so a small grazer also stops stripping a whole
      plant per bite.
- [x] **`NICHE_FLOOR` 40 -> 8**: 6 niches x 40 exceeded the whole post-retrofit population, so every niche
      sat permanently under its floor and the world ran on continuous rescue. A floor above the equilibrium
      is a pump, not a backstop. It had been masking the true equilibrium, which is lower than the rescued
      numbers suggested.
- [x] **Structural-mutation ratchet**: add was gated on node room AND edge room, removal on neither, so a
      body at `MAX_NODES` could only ever lose edges. Developed part count fell 6.25 -> 5.30 over 40
      generations of PURE DRIFT; it now holds. Found with `morph::drift_probe` (kept, `--ignored`).
- [x] **Two uncoupled notions of body size, and the exploit between them**: `Morphometrics::of` takes only
      `&BodyGraph`, so `morph.mass` is independent of the `size` gene. Metabolism and intake key off
      `morph.mass` while `fat_cap` keyed off the gene, letting a lineage shrink the body that costs energy
      and keep the gene that grants storage. Measured directly: `sz` held 0.59-0.61 while mean body mass fell
      1.73 -> 0.24 kg. Fat storage now scales with real body mass, so storage goes as M^1 against metabolism
      at M^0.75 and fasting endurance ~ M^0.25 becomes the force selecting FOR size. Deleted `SIZE_ENERGY`.

### Energy budget made coherent, then found to be the wrong suspect (2026-08-09)

Every law in the creature budget now keys off the SAME body mass on consistent exponents: metabolism
`M^0.75` (Kleiber), intake `M^0.75`, locomotion `M^0.75`, fat storage `M^1` so fasting endurance is
`M^0.25`. No brain output is free any more either: turning was ungated AND uncharged (a cold or starving
creature could pivot forever, which is the observed spin-in-place death), and vertical altitude gain was
free while walking uphill was charged.

- [x] **Behaviour is measured, not inferred**: `path`, `net` (displacement from birthplace), `straight`
      (net/path) and `still %` in the continuous log. `Locomotion` already carried `start` and `path` and
      nothing ever read them. First numbers: path ~16, net ~4.3, straight 0.32-0.43, still 2-9%. Creatures
      are local FORAGERS, not spinners and not migrators.
- [x] **Energy budget is measured**: `Energy.taken`/`spent` per life -> `E in`, `out`, `ratio`. Also
      `foodCV` (plant patchiness per habitable grid cell) and `noP-births` (births refused for want of
      matter).
- [x] **Two of my own hypotheses refuted by those instruments**, both recorded so they are not retried:
      food is strongly CLUMPED (`foodCV` 2.40), not uniform, so travel was never unprofitable for lack of
      patchiness; and the income/expenditure ratio is an EMERGENT equilibrium of a density-dependent system,
      not a knob. Raising `EAT_GAIN` 19 -> 26 moved the ratio 0.94 -> 0.94 and population 58 -> 50, and was
      reverted.
- [x] **The actual limiter: matter, not energy.** `ASSIMILATION` 0.2 was applied uniformly to C, N and P,
      but animal tissue carries ~6.7x the N and ~10x the P of plant tissue per kg, so phosphorus demanded
      50 kg of plant per kg of animal against a real ~10:1. Split into `ASSIMILATION` (carbon, the energy
      trophic step) and `NUTRIENT_RETENTION` 0.70 (N and P, which animals conserve hard because those limit
      them too). N now 9.5:1, P 14:1, P still tightest.

### Open in this phase
- [ ] Aerial and highland niches are still not self-sustaining (89 rescues each in a 22-gen run, vs 24 cold
      and 46 land). Highland is only 4.17% of the surface (`niche::habitat_probe`), so a uniform floor is
      probably the wrong shape for it.
- [ ] **Creatures live ~1 day against a 360-day year** (`BIO_ACCEL` 2000), so seasonal migration is
      structurally unreachable: an animal cannot follow a season it never sees, and no affordable run even
      covers one year (a 22-gen run is 12% of a year). Needs either much longer lifespans relative to the
      day, or a shorter year. The year is tied to the real Tychos calendar, so this is a user call.
- [x] **Fauna standing stock fixed by the assimilation split** (verified, seed 1, 22 gens): population
      47 -> 1180 -> 2112 with approach shaping, fauna 0.001 -> 0.013 kg/m2, fauna/flora 0.1% -> 3.3% against
      a real 1-10% band, and the rescue subsidy from 20x the standing population down to ~1x. Flora fell
      0.97 -> 0.40 because animals finally graze for real.
- [x] **Approach shaping restored** (gated by usable fraction): population 1180 -> 2112, distance walked
      34 -> 45, displacement from birth 7.5 -> 9.8. Teaching creatures to navigate toward food they can
      digest roughly doubled the world's animal population.
- [x] **`still %` was measuring the BIRTH RATE, not stillness** (fixed). Only creatures past the age-200 gate
      counted as having moved, but the figure divided by the whole population, so every young creature scored
      as motionless. The 15% -> 31% rise I read as creatures getting stuck was the opposite: more births.
      Denominator is now the mature count, applied to still%, energy income and expenditure alike. Lesson:
      when a metric moves in a surprising direction, suspect the metric before building a story on it.
- [ ] **Population growth is SLOW on hard seeds and run length changes the conclusion.** Seed 5 reads 48 at
      gen 16 and ~336 by gen 21, so a 16-gen run and a 22-gen run tell different stories about the same
      build. Long runs are also being killed part way (not memory; ~14 GB free), so several results this
      phase rest on partial logs. Prefer gens>=20 for any population claim, and check for the
      "headless run done" line before trusting a number.
- [ ] Energy ratio sits at 0.92 and `noP-births` still fires 36-65 per interval. At ~2000 creatures both are
      small relative rates consistent with a healthy density-dependent equilibrium, but neither has been
      checked on a second seed.
- [ ] Legacy `Soil` grid still runs UNCONSERVED alongside the `Biosphere`, feeding plant regrowth and
      `SOIL_NUTRI`. Phase 1 lists it for deletion.
- [ ] Body temperature as STATE (`thermo::net_heat_watts`/`step_body_temp`) is written and unit-tested but
      not wired; the live path uses the steady-state pair. Wiring it buys thermal inertia, a second
      mass-scaled advantage for large bodies.
- [ ] Carbon drift ~0.2-0.3% per run is the largest remaining ledger term.

### VERDICT: the conserved world is self-sustaining (40-gen run)

The question that decided whether this retrofit produced a living world or a dying one. Every time a
subsidy was removed the population fell (~1100 -> ~500 -> ~160 -> 113), which read like a slide toward
extinction. It is not: those were transients. Over 40 generations population RECOVERS and climbs to
**723 -> 1347**, above where the old subsidised world sat, and holds.

The ledger holds while it does: C oscillates in a bounded band (6922 -> 6297 -> 6874 ppm, not
accumulating), N sits at 1.9 ppm, P creeps 1050 -> 1096 across the whole run. So carrying capacity is now
a consequence of the element budget rather than of `CREATURE_CAP`, and the budget balances.

Lesson for future balance work: do NOT judge a conservation change on a 16-gen run. The trough is deep and
the recovery only shows past ~20 gens.

### Findings that only became visible once matter was measured

- **INVERTED TROPHIC PYRAMID (open, biggest one).** Fauna standing stock is 1.95-2.12 kg/m2 against flora
  0.12-0.21: animals outweigh plants 10-16x. Real ecosystems run the other way round, with animal stock at
  1-10% of plant stock, because each trophic step loses ~90%. Mean creature body is 1.74 m^3 of geometric
  volume, which at flesh density is ~1700 kg, a rhinoceros. So the world is roughly 100-1000x overstocked
  for its area. Not fixable by tuning: either the planet is far too small for its population (PLANET_R 80 m
  would need to be ~700 m for 1100 sheep-sized animals), or `Morphometrics.mass` is not metre-scale volume
  and needs an explicit calibration. DECIDE BEFORE Phase 2, since element-backed bodies make it binding.
  Diagnostic lives in `chem::trophic_ratio` + the CHEM log line.
- **Two clocks disagree.** Astronomical (DAY_TICKS*DT = 40 s per calendar day vs 86400 real) vs biological
  (a creature lives ~1 calendar day vs ~1.5 real years). Confined to `chem::BIO_ACCEL`; all biotic rates
  carry the same factor so their ratios stay real, which is what preserves emergent behaviour.
- **Soil ratios are not free parameters.** NPP, soil organic stock, and decomposition rate are mutually
  determined at steady state, and the organic pool's C:N must come from LITTER (30), not a humus table (12),
  or nitrogen is over-released 2.5x and nothing is ever nutrient-limited. Locked by a test across all three
  elements; checking carbon alone is what let N run away.
- **NPP is area-limited, not per-plant.** Light falls on ground, so a cell's plant community shares one
  budget. Previously productivity scaled with entity count.
- Residual ledger drift is +4794 ppm over 5 gens, entirely from fauna not being element-backed yet.

## Open

### Agent tuning harness (see clients/evolvarium/14-tuning-harness.md)
- [x] Layer 1 PLANT/TREE arm (engine): `--scenario=cohort.json --out=result.json` deterministic mini-world
      runner (`scenario.rs`). Isolated cohort of 5-30 plants/trees in a controlled environment band
      (lat_band, wetness, aquatic/rocky/fire/grazers, second_band for MIXED). Result JSON: survival,
      peak/target, mean mass/age, births/deaths/R, deaths_by_cause, trait_drift, health_score, best_genomes.
      GENE-AGNOSTIC: genome overrides + trait_drift go through serde generically, so a new PlantGenome gene
      is tunable with zero harness edits. Reseed floor auto-disabled + death causes counted only in
      scenario mode (normal runs pay nothing).
- [x] Plant seed-bank library (`persist.rs`): `plant-library.json` of tuned genomes; `--merge=result.json
      --niche=NAME` folds a run's winners in (accumulates across runs, dedupes, per-niche cap). A normal
      `cargo run` seeds the planet biome-matched FROM it (archetype fallback where unmatched; `--no-plant-lib`
      to disable). Forward-compatible (old libraries load after a gene is added).
- [x] Layer 2 (Workflow): `tools/tune-plants.workflow.js` — one tuner agent per niche (core land, aquatic,
      trees, mixed pairs) iterates the runner toward survival+growth, synthesize merges winners into the
      library + smokes the seeded planet + logs frictions. Run on demand (opt-in).
- [x] Layer 1 CREATURE arm (DONE, M4): `creature_cohort` runner live in `scenario.rs` — genome overrides +
      named reflex priors (`reflex_brain`: approach-food / flee-predator / rest-at-night / wander), continuous
      in-scenario breeding, death-cause tallies (`deaths_by_cause`) + survival/master/trait-drift metrics + best
      survivors. `--merge-creatures` harvests winners into a population snapshot; `tools/tune-creatures.workflow.js`
      fans out one tuner per niche. (OPEN follow-up: friction F1 = soften the master-expression gradient.)

### Genes (each: real trade-off, serde-default balance-neutral, verify headless before commit)
- [ ] Reproductive r/K cluster (breed-threshold / offspring-investment / fecundity / age-at-maturity as
      genes). HIGH value (drives life-history speciation) but touches reproduction balance -> do it when no
      long seed is mid-run, and re-tune carefully.
- [~] Armor (predation defense vs speed/upkeep) — TRIED + REVERTED (2026-06-21). Across 3 cost/protection
      settings armor always pegged high (0.7-0.93): avoiding predation death outweighs the cost, so it's a
      near-free defense (violates "everything a trade-off"), it suppresses the carnivore niche, and it
      shifts the equilibrium up to ~100-117 (toward the cap). Revisit only with a fundamentally different
      design (e.g. armor type-specific, or predation made a much larger/uncapped mortality source).
- [ ] Smell sensor (long-range, no type info, cheap). NOTE: changes the NN input shape -> invalidates saved
      brains; gate it / migrate, and regenerate the seed.
- [ ] Metabolic-efficiency on digestion, vision acuity, etc. (lower priority).

### World / visuals
- [x] Hand-seeded diverse showcase (`--diverse`, evolved-diverse.json @ gens=0): loads a multi-niche world
      (cold 52 / warm 50, aquatic 52 / land 75). `cargo run -- --load=evolved-diverse.json` to view it.
- [x] Feed all niches: polar flora floor raised (66b1d03) + shallow-water aquatic flora (this commit), so
      cold + coastal/aquatic regions now have food. Base stays stable (~70). NECESSARY but NOT sufficient.
- [x] REPRODUCTIVE ISOLATION via two-parent mating (`--mating`, default off; `--sexual` kept as alias):
      offspring = crossover of two nearby genetically-similar parents (assortative mate choice) + mutation,
      single-parent budding fallback if no mate. Genome::crossover (structure from one parent, scalar traits
      + diet uniform-crossed). VERIFIED: sustains a stable pop (seed5 min39/mean71/max98) AND assortative
      mating slows reconvergence -> cold niche held 25 (vs 6 budding) over 10 gens. Speciation payoff is real
      for cold; aquatic still 0 (coastal food too sparse -> niche-area problem, not a mating one).
      `cargo run -- --mating` to watch speciation.
- [x] Render god-control: B = seed a burst of 200 creatures ("make more life!" button, kid-friendly). Seeds
      CLONES of the living pop (competent brains + light mutation), not random flailers, so they survive +
      behave. viz::god_disturbances -> sim::seed_burst(parents). Joins L=lightning, K=cull.
- [x] Render BUG fixed: creatures born mid-sim (and B-seeded) never got a render mesh (the referenced
      viz::add_creature_visuals was never written) -> live newborns invisible, only visible once dead as
      carrion; world looked empty as the seed pop aged out. Added CreatureMesh resource + add_creature_visuals
      (dresses meshless creatures w/ genome color via shared creature_look helper, also used by restyle).
- [x] Watchable visualizer: default view speed 0.35x (sim unchanged, just the virtual clock); speed presets
      1-5 (0.1/0.35/1/4/16x) + +/- fine + SPACE pause; speed shown in HUD. Legend overlay (H) explains every
      HUD field (incl. trend) + color/shape encoding + all controls. Top-left hint points to H.
- [x] WALK MODE (camera): TAB toggles ORBIT (space) <-> WALK (ground). True walk -- eye rides a fixed height
      above terrain (climbs hills, no fly): WASD move (W/S fwd+back, A/D strafe, great-circle steps), arrows
      or right-drag look, Shift run. Real shadows ON in BOTH modes now (orbit eclipse-disc fixed). Ground-tuned
      walk cascade (max 130, first bound 12), zoom-scaled orbit cascade. camera::CameraMode + WalkCam +
      update_shadow_mode + update_shadow_cascade.
- [x] WALK lighting fix: walk used to look like permanent night (positional day/night -> spawned on the
      dark hemisphere half the time; a day is only ~2400 ticks). Entering walk now snaps the sky to local
      noon via a visual SunOffset (lights + sun/moon only; sim daylight/rest untouched). [ / ] scrub
      time-of-day, \ jump to noon (golden-hour shadows on demand). Ambient lifts 220->550 in walk.
- [x] Orbit-view real shadows: FIXED (was abandoned). The old "eclipse" disc was a self-shadow blackout from
      the celestial bodies + globe casting into the shadow map; fixed by marking sun-disc/moon/stars
      NotShadowCaster + a zoom-scaled orbit cascade (near/far tracks camera dist). Orbit now shows a real
      terminator shadow.
- [x] Planet casts a shadow in BOTH views: the globe is now a shadow caster in orbit AND walk (was orbit-only;
      walk forced NotShadowCaster to avoid curved-terrain self-shadow acne). Re-enabled in walk with a higher
      per-mode shadow_normal_bias (3.2 walk / 1.8 orbit) so the terrain just past the horizon falls into the
      planet's shadow at dawn/dusk without acne. camera::update_planet_caster + update_shadow_mode. Verified
      live by the user.
- [x] Wildfire flammability fix + re-enable (`FIRE_ENABLED=true`): the polar ice cap no longer burns
      (`sphere::fuel` gates to 0 across the ice-temperature band, like ocean already did), so ocean + ice are
      firebreaks. Fires spread far less easily: `FIRE_SPREAD` 0.5->0.18 and spread now SCALES with the
      neighbor's fuel density (lush forest carries fire, sparse scrub barely does), `FUEL_MIN` 0.30->0.45,
      `FIRE_DECAY` 0.12->0.18. Burning ALSO enriches soil where it burns: per-cell ash (`FIRE_ASH`) plus a new
      `FIRE_BURN_ASH` deposit when a plant/tree/grass burns up (its biomass -> ash, x mass; trees ~3x) so
      burned ground regrows richer. Glow shrunk + ocean-guarded so coarse coastal cells don't spill flame onto
      the sea. Verified headless: pop stable ~49-63, fire avg 0.006-0.012 (contained), trees hold ~150.
- [x] Earth-like geography (terrain features in sphere.rs): ~50% ocean (SEA_LEVEL 0.41), one great deep
      ocean + a second basin, two mountain ranges -- all GUARANTEED via placed gaussian landmarks (not left
      to noise). report_geography test confirms ocean ~47% / deep ~13% / mountain ~5%. Homeland moved to a
      verified temperate lowland (gentle land landmark) so founders don't spawn on a peak or in the sea.
- [x] Aquatic habitat widened: flora grows through the water column (AQUATIC_FLOOR 0.12..SEA_LEVEL), richest
      in shallows, thinning to open water; less polar-sensitive (water moderates temp). Gives swimmers a
      sea-wide food base. Paired with wet-gated drown so only aquatic plants live there.
- [x] God-control P = populate the WHOLE planet: plants + trees + creatures scattered globally, each placed
      in habitat it can survive (swimmers in sea w/ high swim, alpine in mountains, climate-matched temp,
      aquatic flora in water, trees on habitable land). sim::seed_planet. Joins B (creature clone burst).
- [x] Stronger diversity (2026-06-24): the new default `evolved-showcase.json` is a multi-niche co-evolved
      world (fliers + swimmers + alpine + land) holding across the 50%-ocean + mountains planet. See the
      showcase-seed item above. (--mating-as-default still open to consider.)
- [x] **Niche x climate diversity seed (2026-06-25)**: `tools/tune-diversity.workflow.js` fanned out 12 agents
      (flying/land/water/highland x warm/temperate/cold), each evolving 50 hardy survivors in a COMPETITIVE
      mini-world (finite food + grazers + a predator sub-cohort), holding niche+climate identity + showing
      brain development (evolvable sensors/hidden + recurrent memory under predation). All 12 hit 50; merged ->
      `evolved-diversity.json` (600 = exactly 50/combo; all 4 niches + 3 climates each >30; brains hidden 2-7,
      sensors 1-7). Live planet co-evolution (`--gens=10 --load`) grew it to cap 2000 and held with NO niche
      rescue in steady state (self-sustaining, unlike the showcase which needs aerial rescue): land 1044,
      aquatic 525, highland 288, flying 143; cold 233/temp 887/warm 880. Under planet competition brains got
      LEANER (hidden 3.2, sensors 1.4) = no-free-lunch efficiency vs the abundant mini-worlds. Survivors saved
      to `evolved-diversity-coevo.json` (local, 67MB full-world, not committed). Harness change: best_creatures
      harvest cap 12 -> 50. Friction logged: flying-temperate couldn't hold flight identity under competition.
- [x] **Visual lightning during storms (2026-06-24)**: `viz::lightning_visuals` draws jagged blue-white bolts
      (cloud-base -> ground channel + a fork + a ground starburst flash, bright at onset, fades over FLASH_TICKS)
      over heavy WARM-storm cells (rain > 0.55, mirrors the rain field where sim lightning also ignites fire).
      Render-only immediate gizmos like rain_visuals, tick-bucketed hashes -> deterministic, sparse, no entity
      churn, no sim coupling. (Follow-up if wanted: an ambient light POP on strike; fire ignition stays the
      sim's own roll in fire_step.)
- [x] **Atmosphere rim/haze (2026-06-24)**: additive sky-blue shell just above the surface, FRONT-culled so
  the opaque globe occludes all but the thin ring past its silhouette -> soft blue limb halo in orbit view
  (`viz::Atmosphere` + `atmosphere_visibility`, orbit-only). Axial-tilt seasons already landed (Tychos sun).
- [x] **Day-biased atmosphere glow (2026-06-24)**: rim shell now carries per-vertex color driven by the sun
  (`viz::update_atmosphere`) -> bright blue on the sunlit limb, dim airglow on the night side, warm sliver at
  the terminator. Replaces the uniform ring.
- [x] **Visual polish: nicer creature meshes (2026-06-24)**: per-niche bodies now come from the evolved
      part-graph (morphology flagship), THEN a metaball SDF skin (`morph.rs` build_body_mesh) wraps each in
      one tight surface — smooth-union closes gaps + fillets seams (no more blocky stacked primitives).
      Graph-connectivity bridges keep far/thin-necked heads attached; sub-cell-thin parts floored so fins/
      limbs don't shred; load cost hidden via grid-sampled normals + a per-frame build budget. Eyes anchor to
      the head surface (floating-eyes fix). Verified via `--capture` across body plans.
- [x] **Corner inspector minimap** (DONE): a real 3D globe in the top-right that ROTATES WITH the view (2nd
      camera on RenderLayers 1, corner viewport, synced to OrbitCam), colored by a chosen FIELD. 'M' cycles
      biome / heat / moisture / elevation. (UI fixed to the main camera via IsDefaultUiCamera so the HUD doesn't
      duplicate into the minimap viewport.) terrain::build_globe_colored + viz::minimap_*.
  - [x] DYNAMIC overlays (2026-06-23): 'M' now cycles 8 fields = 4 static (biome/heat/moisture/elevation)
        + 4 LIVE (soil fertility / groundwater / fire / creature-density), rebuilt each frame from sim
        resources via grid_cell sampling + build_globe_colored. viz::minimap_dynamic.
  - [x] Follow-up (2026-06-24): walk-mode aims the minimap at the walker (centers on `WalkCam.dir`)
        instead of OrbitCam; minimap now shows in Orbit + Walk, hidden only in the orrery.
- [x] **Land wear / soil compaction (2026-06-23)**: `Wear` grid (SOIL_RES, mirrors Soil/GroundWater).
      Grounded LAND creatures add wear along their path (`bat.wear_adds` in live_step decide, x 0.5+size;
      fliers aloft + swimmers in water don't trample), applied + decayed (slow heal, ~250-tick half-life) in
      the serial apply. High wear CUTS plant/grass/tree growth (`trample = 1 - WEAR_GROWTH_PENALTY*wear`) and
      CULLS grass tufts on over-worn cells (WEAR_GRASS_CULL + per-tick WEAR_CULL_PROB) -> busy niches go
      visibly BARE (emergent dirt clearings) + grazing-pressure feedback. New minimap field 'wear' (dirt-brown
      overlay, 9 fields now). Verified at full pop (`--load --gens=3-4`): pop steady 1100, plants ~4230,
      ~25-30 land cells trail-level bare + self-limiting (no desertification). Continuous log adds `wear`/`bare`.
- [x] **Dead-tree logs — visual half (2026-06-24)**: when a tree dies it drops a fallen log on the ground
      (`viz::Log`, observed via `RemovedComponents<Tree>` + a position cache; render-only, windowed app, no
      sim/balance/seed coupling; bulk-despawn guard skips world resets). Logs vary per-instance (length,
      thickness, yaw, bark color incl. mossy + weathered-grey) and sink into the ground at end of a long life.
- [ ] **Dead-tree logs — gameplay half** (balance phase): small creatures HIDE in/behind a log -> predation
      cover (ties into threat/flee + predation); big creatures can't; slow-rotting logs return soil nutrients.
      Deferred (balance-adjacent) until the visuals-first phase ends.
- [ ] Scale-up toward thousands of creatures (original spec headline) via density-invariant rebalance —
      big, do cautiously in validated steps. (Carrying cap now ~1100 w/ parallel live_step; see PARALLELIZATION.md Phase 6.)

### Perf findings (2026-06-23 session) — for future tuning
- [x] Headless skips render-only grass + seaweed (`gen.headless` early-return in grass_step/seaweed_step):
      ~1.37x faster headless tick (5785->4223 us @ pop 1100). Windowed/capture/shots keep the carpets.
- [ ] **Creature spatial grid: NOT worth it at ~1100** (tried + reverted). The O(n^2) social/threat/collision
      scans are already parallel, so at 16 cores they're only ~225 us -- the grid's per-tick build + per-creature
      candidate-gather overhead exceeded the saving (tick got SLOWER). Revisit ONLY at ~10k+ creatures where
      O(n^2) re-dominates. Don't re-attempt below that without a profile showing the scans dominate.
- [ ] **Grass cost is now a WINDOWED-only concern** (headless skips it). Windowed is render-bound, not sim-bound
      (PARALLELIZATION.md), so the 1814 us grass_step sim cost rarely gates frame rate. If ever needed: cache the
      STATIC per-tuft field samples (base_temperature/moisture/rockiness/elevation are position-only -> compute
      once at spawn, store on the tuft; re-sample only the dynamic groundwater/fire/daylight/soil) for a
      behavior-preserving ~30% grass cut. Lower priority given the headless skip + render-bound windowed.
- [ ] **live_step now dominates the tick at 1100** (~2300 us, top system). Cost is per-creature food-sensing +
      brain forward()/learn(), all O(n) + already parallel. Next real lever would be cheaper brains (the evolved
      nets grew ~2x -> bird seed is 12MB); a soft brain-size cost/cap could trim both perf + seed size, but it's
      balance-sensitive (changes what evolves) -> needs the multi-seed equivalence fan-out before committing.

### M7+ enrichment (reconciled from spec 08-roadmap.md M7+ + 09-open-questions.md, 2026-06-23)
Spec-folder ideas that were NOT line-itemed here. Pulled in so BACKLOG is the single task list. "Done"
items from M0-M6 stay in the specs (status notes inline there); only the still-OPEN enrichment lands here.
- [ ] **More environment fields** (M7+): water CURRENTS (drift vector that pushes swimmers + disperses
      seeds/plankton) + standing TOXINS (volcanic/anoxic patches that poison on contact). Each = a `sphere.rs`
      pure field + a sim hook + (ideally) a minimap overlay; low-risk UNLESS wired as a new brain INPUT
      (then gate, see below). NOTE already covered, do NOT re-add: temperature, moisture, magnetic field,
      and LIGHT (positional daylight + water-depth attenuation `×(1-0.6·depth)` + canopy-shade brain input).
- [ ] **Essential / trace nutrients** (M7+): make a few of the 10 nutrients ESSENTIAL (deficiency becomes
      lethal, not just the current soft growth-load via `DEFICIT_G`). Deepens the diet model; balance-
      sensitive -> tune via the harness. NOTE carrion-TIMING already done (Rot{age} + Ferment chain: fresh ->
      sugar -> toxic fast -> gone), so a fresh kill is already richer than an old one; don't re-add that.
- [x] **Lineage / phylogeny view** (M7 data + open-q #7, DONE 2026-06-23): 'Y' toggles a live SPECIES TREE
      panel (under the minimap). Render-only online genetic clustering (`viz.rs` phylogeny_classify +
      Phylogeny resource): every PHY_INTERVAL ticks each creature joins its nearest species (centroid EMA) or
      BUDS a new species (parent = nearest relative) when past PHY_THRESH on a niche-gene trait vector. Works
      as descent because offspring resemble parents. Panel draws the tree indented by ancestry, colored per
      clade, with live pop + peak + clade tags (flier/swimmer/alpine/land, herbivore/omnivore/carnivore,
      size, thermal). Verified --capture: 10 clades from the bird seed, sensible nesting. NO sim changes
      (determinism + shared worktree safe). FOLLOW-UP: trait-distribution-over-time charts (the other half of
      M7 data) still open; pairs with the god-panel UI item below (shared charting surface).
- [x] **Recurrent / memory brains (2026-06-25)**: MEM_CELLS=3 memory registers. Net WRITES them as extra
      outputs (out[8..8+MEM_CELLS]) + READS last tick's values back as extra globals via `Brain.memory`
      (1-tick delay, same pattern as voice/prev_dist) -> path memory, hysteresis, keep pursuing hidden prey.
      Bounded (sigmoid 0..1); MEM_CELLS=0 -> pure feedforward ("gate behind a setting", spec 04). Migrates
      gracefully (read-back inputs pad 0, memory outputs biased -4 = off via OUTPUT_MIGRATE_BIAS const fn) ->
      showcase loads unchanged, pop holds ~1000-1175. forward()/learn() unchanged (loop generically).
- [x] **CPPN / HyperNEAT indirect encoding (2026-06-25)**: opt-in `BrainEncoding::Cppn` (--cppn founders;
      default planet stays 100% Direct, untouched). A small fixed-topology CPPN (`src/cppn.rs`: 7 coord inputs
      -> 2x10 mixed-activation hidden -> 1 weight) PAINTS net.ih/net.ho at spawn (`develop_brain`) from a
      FULL body-geometry substrate: input neurons on a sensor-angle ring, output motors at body anchors
      (head/COM/dorsal from `Morphometrics`), so a mutated body/sensor inherits CPPN-painted (not random-pad)
      control = the morphology-coupling fix (P3). forward()/learn() run UNCHANGED on the painted net; lifetime
      Hebbian fine-tunes, GA evolves the cppn (mutate/crossover cell-aligned). Encoding set at founding +
      inherited (no mid-lineage flip; can't invert a learned net into a cppn). Verified: --cppn pop evolves
      5 gens (fitness 0.5->3.8), saves/loads/renders. Deferred: per-weight CPPN plasticity (flat 0.2 v1);
      per-limb motor transfer (gym-only, planet brain is kinematic). NEXT: evolve + save a tuned CPPN seed.
- [ ] **GPU for brains/fields** (M7+ perf, opt-in): offload brain forward()/learn() or field sampling to the
      GPU. Only worth it at much higher pop than today (live_step is already parallel + fast at 1100); revisit
      with a profile that shows CPU brains dominating at ~10k+. See PARALLELIZATION.md perf notes.
- [ ] **Articulated bodies** (open-q #1, M7+ opt-in): jointed bodies + a motor/joint solver as an opt-in mode
      (v0 is non-articulated blobs by decision). Big; population-budget-sensitive -> design with the human.

### Bigger
- [x] Multi-core sim tick (Phases 0-5 DONE 2026-06-23, full plan + report in `PARALLELIZATION.md`).
      Parallelized grass/plant/seaweed (snapshot->par decide->serial apply, per-entity deterministic RNG) +
      weather (grid chunked over ComputeTaskPool, byte-identical). Whole tick ~16.5->3.48 ms = **4.7x** on 16
      cores (61->~290 ticks/s). Deterministic; equivalent (flora <3% + traits within +-10% pooled; seaweed +
      weather byte-identical). `--profile` flag + `Rng::for_entity` infra added. System unchaining still
      deferred (unsafe: systems share Soil/gw/fire within a tick). Helps headless runs + fast-forward viz only;
      normal viz/campaign unchanged by design.
- [x] live_step parallelized + ~1000-creature world (Phase 6 DONE 2026-06-23, report in `PARALLELIZATION.md`).
      Creature pop grown ~60->1100 (CREATURE_CAP 130->1100, NICHE_CAP ~7x; food caps untouched, web already
      carried it). live_step got the same snapshot->par decide->serial apply pattern (LiveBatch intents:
      eat-despawn dedup, tree-bite/soil/birth/carrion, running caps in apply). At pop 1100: live_step **5.0x**
      (11.3->2.3 ms), whole tick **2.6x** (15.3->5.8 ms = 66->173 ticks/s) -- the gain grows with pop since
      live is O(n^2) in the social/threat/collision scans. Deterministic (same-seed byte-identical); equivalent
      (pop/energy/plants within ~1% pooled over 4 seeds); stable (pop holds 1100 to 48k ticks, no boom-bust).
- [ ] Solar system: real Tychos orbital model drives the sky (sun/moon + wandering planets) — spec
      `clients/evolvarium/15-solar-system-tychos.md`. Literal Tychos geometry (TSN deferent/epicycle data),
      drives existing sky, real orbital proportions. New `orrery.rs`; `sun_dir`/`moon_pos` delegate.
- [ ] Sexual reproduction + speciation (mate choice, genetic-distance species) — spec M6.
- [ ] God-panel UI + live charts (population/traits over time) — spec 07.

## Not automated here
- Genome/learning ARCHITECTURE changes (NN input shape, learning rule) — confirm with the human first.
