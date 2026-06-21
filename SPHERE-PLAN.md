# Spherical world conversion (in progress)

Goal: creatures live on the surface of a small planet, not a flat plane. User-chosen scope: **true
spherical sim** (not visual-only). Population starts **localized** (one region, spreads outward). Add
**poles with temperature** gradients. Drifting **clouds**; rain only under clouds (~10% chance), replacing
random storms. Earth-like proportions (stylized so moon stays framed).

## Status

- [x] **Geometry foundation** (`src/sphere.rs`, tested): lat/lon <-> 3D, tangent frame, great-circle `step`,
      `surface_dist`, seamless 3D-fBm terrain (`elevation`, `is_ocean`), `base_temperature` (cold poles +
      lapse rate), `moisture` (latitude belts + coastal), `sun_dir`/`moon_dir`/`daylight_at`. 5 unit tests green.
- [ ] **Sim conversion** (`src/sim.rs`): the big one.
- [ ] **Render** (`src/viz.rs`, `src/terrain.rs`, `src/camera.rs`).
- [ ] **Clouds + cloud-rain** (`src/sphere.rs` cloud field + `src/sim.rs` weather).
- [ ] **Re-tune balance** on the new geometry (carrying capacity changes).

## Sim conversion checklist (next ticks)

1. **Position model**: keep Bevy `Transform.translation` as the 3D surface point (length ~ PLANET_R + elevation).
   `Heading(f32)` becomes a COMPASS heading (0 = north) interpreted via `sphere::heading_tangent`.
2. **Movement** (`live_step` ~L840): replace `dir = (sin,0,cos)` + box clamp with `sphere::step(pos, heading, dist)`;
   set translation = new_dir * (PLANET_R + elevation(new_dir)); rotation = look along tangent, up = surface normal.
   Drop all `clamp(-WORLD_HALF, WORLD_HALF)` (a sphere has no edge).
3. **Spawning** (`spawn_world_*`, `rand_pos`): localized start = sample positions in a cap around a chosen
   "homeland" direction (small angular radius). Plants/trees seed globally on land only (`!is_ocean`).
4. **Terrain sampling**: replace `terrain::height/moisture/rockiness(x,z)` calls with `sphere::*(dir)`.
   Ocean = `is_ocean`; swim niche keyed to ocean/coast. Temperature is new -> feeds metabolism + plant growth.
5. **Spatial grid** (`fcell`, `Soil`, `GroundWater`, `Fire`): index by 3D cell or by lat/lon bins. Neighbor
   queries use 3D chord distance (~geodesic at perception range). Simplest: a 3D hash grid over the surface.
6. **Sensors** (~L796): bearing computed in the local tangent frame (project neighbor offset onto east/north).
7. **Day/night**: `daylight()` becomes positional `sphere::daylight_at(dir, tick)`; light_pref niche now also
   varies by where on the globe you are (terminator sweeps). LIGHT_COST uses local daylight.
8. **Temperature pressure** (new): each creature has a thermal comfort (could reuse light_pref or add a gene);
   being far from comfort costs energy. Poles = harsh, equator = mild. Trade-off teeth for the new climate axis.

## Clouds + rain

- Cloud field = scrolling 3D-fBm over the sphere (`sphere::cloud_cover(dir, tick)` 0..1), drifts with "wind"
  (rotate sample point over time). Visualized as soft white patches above the surface.
- Rain: a cell rains when cloud cover over it is high AND a per-cell roll < ~10%. GroundWater fills under
  rain only. Remove random `P_STORM`. Lightning/fire still tied to rain cells.

## Render

- Globe mesh: icosphere or lat/lon sphere, vertex-colored by `elevation`/`moisture`/temperature biome.
  Translucent ocean sphere at `PLANET_R + SEA_LEVEL*ELEV_MAX`.
- Sun: directional light from `sun_dir(tick)` + a far billboard (`SUN_R`). Moon: emissive sphere at `moon_pos(tick)`.
- Camera: orbit/arcball around the planet (replaces fly-cam), or a surface-follow cam.
- Clouds: instanced soft quads / a translucent noisy shell.

## Earth proportions (in `sphere.rs`)

PLANET_R 80; MOON_R 0.27 R; MOON_ORBIT 6 R (compressed from ~60 to stay framed); SUN far, billboard sized
to match the moon's on-sky size (the real Earth coincidence); AXIAL_TILT ~23.5deg -> seasons + cold poles.

## Migration note

Old flat saves (`evolved-continuous.json`) are flat-world genomes; positions are meaningless on the sphere
but genomes still load (spawn them in the homeland cap). Regenerate the showcase seed after re-tuning.
