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
      **longevity** (lifespan vs upkeep), **metab** (frugal/sluggish vs fast/costly).
- Plants/trees: kind, nutrient, defense, quality, wet, height, light_pref, regrow, branches, spread, maturity.

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
- [ ] Visual polish: nicer creature meshes per niche, axial-tilt seasons, atmosphere rim/haze.
- [ ] Scale-up toward thousands of creatures (original spec headline) via density-invariant rebalance —
      big, do cautiously in validated steps.

### Bigger
- [ ] Sexual reproduction + speciation (mate choice, genetic-distance species) — spec M6.
- [ ] God-panel UI + live charts (population/traits over time) — spec 07.

## Not automated here
- Genome/learning ARCHITECTURE changes (NN input shape, learning rule) — confirm with the human first.
