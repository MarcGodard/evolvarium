# Backlog / roadmap

Single worktree (`evolvarium`, branch `main`); commit here, backup `git push origin main:build`.
One coherent item per change: implement, keep the build green, verify headless, commit. Balance constants
live in `config.rs`; the live conversion plan is `SPHERE-PLAN.md`.

## Done

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

### Evolvable genes (creature)
- [x] sensors (angle+range, add/remove), brain (weights + per-connection plasticity + hidden-layer size),
      diet expr0, rigidity, bite, height, size, swim, social, **temp_pref** (thermal niche),
      **longevity** (lifespan vs upkeep), **metab** (frugal/sluggish vs fast/costly), **parental** (r/K),
      **alpine** (mountain niche: cheap rock-crossing vs heavy-build flat-ground penalty; mirror of swim;
      self-limiting since mountains ~5% of world so won't peg like armor; serde default 0 = neutral).
- Plants/trees: kind, nutrient, defense, quality, wet, height, light_pref, regrow, branches, spread, maturity.
      **wet now gates water survival** (drown mortality x submersion x (1-wet)): land flora drowns in the
      sea, aquatic flora (high wet) thrives -> wet splits land vs aquatic plants. Trees are land-only.

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
- [ ] Layer 1 CREATURE arm: `creature_cohort` is parsed-but-inert; wire reflex presets + creature
      death-cause tallies + objectives (doc 14) when creatures are tackled. First job = friction F1.

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
- [ ] Stronger diversity: now that aquatic has a real food base + swimmers can be seeded, run a --mating long
      seed on the NEW world (50% ocean + mountains) so the showcase holds aquatic + alpine + land niches.
      Consider making --mating the default once seeds are re-evolved under it.
- [ ] Visual polish: nicer creature meshes per niche, axial-tilt seasons, atmosphere rim/haze.
- [ ] Scale-up toward thousands of creatures (original spec headline) via density-invariant rebalance —
      big, do cautiously in validated steps.

### Bigger
- [ ] Sexual reproduction + speciation (mate choice, genetic-distance species) — spec M6.
- [ ] God-panel UI + live charts (population/traits over time) — spec 07.

## Not automated here
- Genome/learning ARCHITECTURE changes (NN input shape, learning rule) — confirm with the human first.
