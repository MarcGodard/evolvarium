# Evolvarium

A 3D artificial-life sim on a small **planet**. Tiny per-creature neural-net brains (genome = weights +
sensors + traits) forage, eat, fight, breed, and learn during their lives; a genetic algorithm plus
lifetime learning evolve them against a living, co-evolving food web. Creatures live on the surface of a
sphere with day/night, drifting clouds, cloud-driven rain, lightning wildfire, oceans, mountains, and
cold poles vs a warm equator. Design spec lives in `../clients/evolvarium/` (`00-concept.md` ..).

## Run it

```bash
cargo run                      # the planet visualizer (auto-loads the showcase seed -> a living world)
cargo run -- --headless        # no window, fast-forward, logs per-generation stats, exits
```

First build compiles Bevy (slow, minutes); later builds are fast. Linux needs the usual Bevy deps
(Vulkan/X11/Wayland, alsa, udev).

**Camera (orbit):** hold **right-mouse + drag** to rotate around the planet, **scroll** to zoom, **WASD/QE**
as a keyboard fallback. **Left-click** a creature/plant to inspect it; **F** follows the selection.

### Headless + flags

```bash
cargo run -- --headless --seed=9 --gens=500          # reproducible run, N generations
cargo run -- --headless --gens=400 --save=run.json   # evolve, then save the best healthy snapshot
cargo run -- --load=run.json                          # load a saved population (skips warm-up)
cargo run -- --headless --shots=planet --shot-tick=4000   # CPU-render PNG views of the planet (no GPU)
```

- `--generational` uses the discrete-generation GA instead of the default continuous reproduction.
- `--no-diet` runs the simple single-food world; `--no-load` forces a fresh warm-up start in render mode.
- `--shots[=PREFIX]` writes `PREFIX-globe/homeland/farside/pole.png` by ray-tracing the planet on the CPU
  (no GPU required) so the world can be inspected offline.

## The world

- **Spherical**: creatures move along great circles; positions are 3D points on the surface. Terrain
  elevation, oceans, moisture belts, and temperature are seamless 3D-noise fields (no seam, no pole pinch).
- **Localized start**: all founding life spawns in one homeland region and spreads across the globe.
- **Climate**: cold poles + high elevation, warm equator -> a latitudinal thermal niche (creatures evolve
  `temp_pref`). Day/night is positional (the lit half faces the orbiting sun; a moon orbits too).
- **Weather**: clouds drift; rain falls only from thick clouds (~10%), watering the ground; lightning can
  ignite wildfire that spreads through dry fuel, burns vegetation, and leaves fertile ash.
- **Food web**: plants, fruit trees (reach-gated, seed-dispersed when eaten), evergreens, carrion that
  rots to poison, and a soil-fertility loop (death feeds the ground).

## The genome (every trait is a trade-off)

Creatures evolve: directional **sensors** (angle + range), a variable-topology **brain** (weights +
per-connection plasticity + hidden-layer size), **diet** expression + **rigidity** (specialist vs
generalist), **bite**, **height**, **size**, **swim** (aquatic niche), **social** (kin herding), and
**temp_pref** (thermal/latitudinal niche). Plants/trees evolve nutrient, defense, quality, moisture
preference, height, light preference, regrow, branches, spread, and maturity.

Reproduction is **continuous** by default (self-sustaining birth/death after a short generational warm-up);
the population self-regulates to a stable carrying capacity (~70-90). Creatures age and die of old age.

## Layout

```
src/
  main.rs       app setup, CLI flags, render/headless wiring, the globe + sun + moon scene
  sphere.rs     planet geometry + climate: lat/lon<->3D, great-circle movement, terrain/temperature/
                moisture/ocean fields, sun/moon, clouds + cloud-driven rain, biome color
  sim.rs        the simulation: movement, sensing, metabolism, eating, predation, reproduction, weather,
                fire, soil, the nutrient loop, and per-generation/continuous logging
  genome.rs     creature genome + mutation + the tiny NN (forward + Hebbian/Oja lifetime learning)
  plant.rs      plant/tree genome, growth, coloring
  terrain.rs    build the globe render mesh from the sphere fields
  viz.rs        render-only: creature/plant styling, clouds, rain, fire, day/night, inspect panel
  camera.rs     orbit camera (drag rotate, scroll zoom, follow)
  snapshot.rs   headless CPU ray-tracer -> PNG snapshots of the planet
  persist.rs    save/load population snapshots (serde JSON)
  config.rs     all balance/tuning constants in one place
```

See `SPHERE-PLAN.md` for the in-progress conversion checklist and `BACKLOG.md` for further ideas.
