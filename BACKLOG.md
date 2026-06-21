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
      cloud-only rain (no global storms); lightning -> wildfire.

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

## Open

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
      or right-drag look, Shift run. Real shadows ON only in walk (close horizon -> range covers view, no
      eclipse disc); OFF in orbit. Ground-tuned cascade (max 130, first bound 12); globe+ocean NotShadowCaster
      so only trees/creatures cast. camera::CameraMode + WalkCam + update_shadow_mode.
- [~] Orbit-view real shadows: ABANDONED. A directional shadow map only covers maximum_distance around the
      camera, so the boundary showed as a dark "eclipse" disc that grew zooming in; widening the range blacked
      the whole hemisphere (big smooth globe self-shadows). Orbit uses normal-based lambert day/night only.
      Real shadows live in walk mode instead (above).
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
