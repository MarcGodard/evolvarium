# Spherical world conversion (COMPLETE — historical)

The flat->sphere conversion is **done**: the sim, render, snapshot, climate, and camera all run on the
planet. This file is kept as the historical conversion record + design notes. The live roadmap (what is
done vs open going forward) is `BACKLOG.md`.

Goal: creatures live on the surface of a small planet, not a flat plane. User-chosen scope: **true
spherical sim** (not visual-only). Population starts **localized** (one region, spreads outward). Add
**poles with temperature** gradients. Drifting **clouds**; rain only under clouds (~10% chance), replacing
random storms. Earth-like proportions (stylized so moon stays framed).

## Status

- [x] **Geometry foundation** (`src/sphere.rs`, tested): lat/lon <-> 3D, tangent frame, great-circle `step`,
      `surface_dist`, seamless 3D-fBm terrain (`elevation`, `is_ocean`), `base_temperature` (cold poles +
      lapse rate), `moisture` (latitude belts + coastal), `sun_dir`/`moon_dir`/`daylight_at`. 5 unit tests green.
- [x] **Sim conversion** (`src/sim.rs`): DONE. Movement = great-circle (`sphere::step`); localized homeland
      spawn (`homeland_pos`, all initial life in one cap, spreads via `disperse_pos`); terrain/moisture/
      rockiness/habitability sampled on the sphere; positional day/night (`daylight_at` per creature+plant);
      grids (soil/water/fire/food) indexed by lon/lat (`grid_uv`, `cell_center` -> Vec3). Headless STABLE:
      pop ~70-90, energy ~35-42, no crash to the floor (the flat-world balance bug is gone on the sphere).
- [x] **Clouds + cloud-rain** (`sphere::cloud_cover`/`rain_at` + `weather_step`): rain is now local +
      cloud-driven (no global storms), ~10% of thick cloud is rain-bearing.
- [x] **Snapshot system** (`src/snapshot.rs`, `--shots`): headless CPU ray-tracer -> PNG (globe/homeland/
      farside/pole views). No GPU needed. Lets the world be inspected offline. VERIFIED (planet + oceans +
      localized life + herds visible).
- [x] **Live render** (`cargo run`): DONE. Globe mesh (`terrain::build_globe`, elevation-displaced +
      biome-vertex-colored) + translucent ocean sphere + orbiting sun (directional light follows
      `sphere::sun_dir`) + emissive moon on its orbit + drifting cloud puffs + cloud-driven rain streaks +
      surface fire glow, all on the sphere. Plants/trees orient outward (grow from the surface); creatures
      stand on the surface (sim sets it). Camera starts in space over the homeland, faster fly speed.
      Builds clean; render binary starts without panic.
- [x] **Thermal niche**: temp_pref gene + TEMP_COST (poles harsh) -> latitudinal spread (commit 80fcadc).
- [x] **Lifespans**: senescence was off in continuous (creatures immortal) -> enabled; real life cycle (ed493cc).
- [x] **Proportions + clouds**: trees shrunk (~2-4 units, no longer poke into clouds); clouds are now solid
      translucent puff entities on a shell above the trees, not wireframe gizmos (ed493cc).
- [x] **Navigation**: orbit camera (drag to rotate, scroll zoom) replaces free-fly (7ce9635).
- [x] **Cleanup**: the old flat `terrain.rs` helpers (build_mesh/height/moisture/rockiness/cloud_shade/
      WATER_LEVEL) were removed when the world became a planet; `terrain.rs` now only builds the globe mesh.
- [x] **More genes** (genetics review): added longevity, metab (tempo), parental (r/K), alpine (mountain
      niche), adiposity (fat strategy), and a full 10-nutrient regulatory diet genome. Reproductive r/K
      *cluster* (breed-threshold/fecundity/age-at-maturity as separate genes) still open in `BACKLOG.md`.
- [ ] **Bite arms race**: bite still pins ~1.0 (cosmetic now, pop stable) -- could pull to an interior
      optimum. Tracked as friction F2 in `clients/evolvarium/tuning-frictions.md` for the tuning harness.
- [~] **Polish**: star background + sun billboard + moon DONE; walk-mode real shadows + day/night DONE.
      Still open: axial-tilt seasons, nicer per-niche creature meshes, atmosphere rim/haze.

## Done build plans (collapsed)

These detailed conversion plans are all SHIPPED; kept as one-liners so they aren't recreated. See the
`## Status` markers above and the code for as-built detail.

- **Sim conversion** (position model, great-circle movement, localized spawn, sphere terrain sampling,
  lon/lat spatial grids, tangent-frame sensors, positional day/night, temperature pressure) — DONE in
  `sim.rs` + `sphere.rs`.
- **Clouds + rain** (scrolling 3D-fBm cloud field drifting with wind; local ~10% cloud-driven rain fills
  GroundWater; random `P_STORM` removed) — DONE (`sphere::cloud_cover`/`rain_at` + `weather_step`).
- **Render** (elevation-displaced biome globe mesh, translucent ocean sphere, sun directional light +
  billboard, emissive moon, soft cloud puffs, orbit camera) — DONE (`terrain::build_globe` + `viz.rs`).

## Earth proportions (in `sphere.rs`)

PLANET_R 80; MOON_R 0.27 R; MOON_ORBIT 6 R (compressed from ~60 to stay framed); SUN far, billboard sized
to match the moon's on-sky size (the real Earth coincidence); AXIAL_TILT ~23.5deg -> seasons + cold poles.

## Migration note

Old flat saves (`evolved-continuous.json`) are flat-world genomes; positions are meaningless on the sphere
but genomes still load (spawn them in the homeland cap). Regenerate the showcase seed after re-tuning.
