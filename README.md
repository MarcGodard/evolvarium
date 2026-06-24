# Evolvarium

A 3D artificial-life sim on a small **planet**. Tiny per-creature neural-net brains (genome = weights +
sensors + traits) forage, eat, fight, breed, and learn during their lives; a genetic algorithm plus
lifetime learning evolve them against a living, co-evolving food web. Creatures live on the surface of a
sphere (birds fly above it, swimmers dive through its oceans) with day/night, drifting clouds, cloud-driven
rain, lightning wildfire, oceans, mountains, a tilted magnetic field with polar auroras, and cold poles vs a
warm equator. Design spec lives in `../clients/evolvarium/` (`00-concept.md` ..).

## Setup

Need Rust (stable, edition 2021). Install via [rustup](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # then restart shell or: source "$HOME/.cargo/env"
rustc --version                                                  # confirm toolchain
```

No pinned toolchain file: latest stable works. Builds the GPU visualizer (needs Vulkan + a display) and a
headless mode (no GPU). Linux needs the usual Bevy system deps:

```bash
# Debian/Ubuntu
sudo apt install build-essential pkg-config libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev vulkan-tools
# Fedora
sudo dnf install gcc pkg-config alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel vulkan-tools
# Arch
sudo pacman -S base-devel pkg-config alsa-lib systemd-libs wayland libxkbcommon vulkan-tools
```

macOS + Windows need no extra system packages (Metal / DirectX ship with the OS).

## Run it

```bash
cargo run                      # the planet visualizer (auto-loads the showcase seed -> a living world)
cargo run -- --headless        # no window, fast-forward, logs per-generation stats, exits
```

First build compiles Bevy (slow, minutes); later builds are fast. No GPU/display (CI, servers)? Use
`--headless` or `--shots` (CPU ray-tracer) instead of the windowed visualizer.

**Camera:** **TAB** toggles ORBIT (in space) and WALK (on the ground). In orbit, hold **right-mouse + drag**
to rotate around the planet, **scroll** to zoom, **WASD/QE** as a keyboard fallback. In walk, the eye rides a
fixed height above the terrain: **WASD** move, arrows or right-drag to look, **Shift** to run, **[ / ]** scrub
time-of-day, **\\** jump to noon. **Left-click** a creature/plant to inspect it; **F** follows the selection.
Real sun shadows render in walk mode; **H** opens a legend of every control and HUD field. **M** cycles a
corner minimap globe through field overlays (biome / heat / moisture / elevation, plus live soil / groundwater
/ fire / creature-density / wear); **Y** opens a live phylogeny (a species tree of the population). God controls:
**B** seed creatures, **P** populate the whole planet, **L** lightning, **K** cull.

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
  ignite wildfire that spreads through dry vegetation (oceans + the polar ice cap are firebreaks; spread
  scales with fuel density), burns plants/trees, and leaves fertile ash so burned ground regrows richer.
- **Food web**: plants, fruit trees (reach-gated, seed-dispersed when eaten, dropping fallen fruit),
  evergreens, fermenting fruit/detritus, carrion that rots to poison, and a soil-fertility loop where
  fertility follows water (poor dry interior, rich wetlands/coasts) and spikes where life dies and decomposes.
- **Land wear**: grounded creatures trample the ground they cross; heavily trodden cells compact, grow less,
  and shed their grass, so busy niches wear visible bare clearings while idle ground slowly heals.

## The genome (every trait is a trade-off)

Creatures evolve: directional **sensors** (angle + range), a variable-topology **brain** (weights +
per-connection plasticity + hidden-layer size), a 10-nutrient **diet** genome (per-nutrient `uptake` +
**rigidity** specialist-vs-generalist), **bite**, **height**, **size**, **swim** (aquatic niche),
**alpine** (mountain niche), **social** (kin herding), **temp_pref** (thermal/latitudinal niche),
**longevity** (long life vs upkeep), **metab** (frugal/sluggish vs fast/costly), **parental** (r/K
investment), and **adiposity** (lean/nimble vs fatty/buffered). They also evolve **flight** (birds cruise
above the world; high flight excludes high swim, and a shared vertical axis lets swimmers dive through the
water column), **magneto** (magnetoreception for navigation under the tilted field), and a defense/morphology
cluster: **detox**, **carnivory**, **pelt**, **armor**, **venom**, **limbs**, **climb**, **eyes**, **head**,
plus cosmetic appearance genes. The brain's action outputs (move, turn, attack, defend, eat, sprint, climb)
mean predation, active defense, what to eat, and whether to fly are all learned in-life and selected across
generations. Plants/trees evolve a 10-nutrient profile plus defense, quality, moisture preference, height,
light preference, regrow, branches, spread, and maturity.

**Metabolism**: three energy stores (fast / sugar / fat) burn fast->sugar->fat; fast leaks even at rest,
fat mobilizes slowly and carries upkeep. Plants give sugar, meat gives fat, fruit and fermenting detritus
give the volatile fast store. A regulatory `master_expression` (reserves vs uptake demand) gates how much
energy each food yields, so diet breadth is a real trade-off.

Reproduction is **continuous** by default (self-sustaining birth/death after a short generational warm-up);
the population self-regulates to a stable carrying capacity of ~1000+ creatures (the sim tick is
multi-threaded to carry it; see `PARALLELIZATION.md`). Creatures age and die of old age. `--mating` enables
two-parent assortative reproduction (crossover + mate choice) for stronger speciation.

## Layout

```
src/
  main.rs       app setup, CLI flags, render/headless wiring, the globe + sun + moon scene
  sphere.rs     planet geometry + climate: lat/lon<->3D, great-circle movement, terrain/temperature/
                moisture/ocean fields, tilted magnetic field, sun/moon, clouds + cloud-driven rain, biome color
  sim.rs        the simulation: movement, sensing, metabolism, eating, predation, reproduction, weather,
                fire, soil, the nutrient loop, and per-generation/continuous logging
  genome.rs     creature genome + mutation + the tiny NN (forward + Hebbian/Oja lifetime learning)
  plant.rs      plant/tree genome, growth, coloring
  terrain.rs    build the globe render mesh from the sphere fields
  viz.rs        render-only: creature/plant styling, clouds, rain, fire, aurora, day/night, inspect panel,
                corner minimap globe, phylogeny view
  camera.rs     orbit + walk cameras (drag rotate, scroll zoom, follow, ground walk, swim), per-mode shadows
  snapshot.rs   headless CPU ray-tracer -> PNG snapshots of the planet
  persist.rs    save/load population snapshots (serde JSON)
  config.rs     all balance/tuning constants in one place
```

See `BACKLOG.md` for the roadmap + done items and `PARALLELIZATION.md` for the multi-core tick; the
`../clients/evolvarium/` specs hold the design detail. `SPHERE-PLAN.md` is the (completed) sphere-conversion history.
