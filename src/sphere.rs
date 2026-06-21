//! Spherical world geometry. Creatures live on the SURFACE of a planet (radius PLANET_R) instead of a
//! flat x,z plane. A position is a 3D point; its surface direction `d = pos.normalize()` gives latitude,
//! longitude, the local tangent frame (east/north), and the surface normal. Movement is a great-circle
//! step in the tangent plane. Terrain/temperature/moisture are seamless 3D-noise fields on the sphere
//! (no edge, no wrap seam). The sun + moon orbit; day/night is positional (the lit half faces the sun).
//!
//! Earth-like proportions (stylized so the moon stays on-screen): moon radius ~0.27x planet, sun appears
//! ~same angular size as the moon (the real Earth coincidence), poles get glancing sun -> naturally cold.
#![allow(dead_code)] // wired into sim/viz incrementally; keep this module buildable + testable on its own
use crate::terrain::HEIGHT_MAX;
use bevy::prelude::*;

pub const PLANET_R: f32 = 80.0; // planet radius (world units). Matches old WORLD_HALF so creature scale + costs carry over.
pub const ELEV_MAX: f32 = HEIGHT_MAX; // max terrain elevation above the sea sphere (reuses the flat-world peak)
pub const SEA_LEVEL: f32 = 0.41; // normalized elevation (0..1) below which terrain floods (ocean) -> ~50% sea
pub const SEA_FLOOR_MAX: f32 = 9.0; // max ocean DEPTH below the sea surface at the abyssal center (world units)
// Founding homeland direction: a temperate mid-latitude spot, kept as gentle habitable lowland by a land
// landmark so founders never spawn on a peak or in the sea. Shared by terrain features + sim::homeland_center.
pub const HOMELAND_DIR: [f32; 3] = [0.30, 0.50, 0.40];
// Aquatic flora grows in the water column from here up to SEA_LEVEL (coast). Below = abyssal/barren. Wide
// band -> swimmers have a real sea-wide habitat, not a thin coastal ring (fish niche kept going extinct).
pub const AQUATIC_FLOOR: f32 = 0.12;
// Even bone-dry desert keeps a sliver of habitability so rare drought-tolerant flora (cacti/scrub/short
// grass) can persist, and a rain pulse (ground water -> effective moisture) blooms it briefly. Below the
// grass seed gate (GRASS_HAB_MIN) when dry, so desert stays sparse until it actually rains.
pub const DESERT_FLORA_FLOOR: f32 = 0.12;

// --- celestial bodies (relative Earth proportions, distances stylized down to stay visible) ---
pub const MOON_R: f32 = 0.27 * PLANET_R; // moon ~1/4 planet radius (Earth: 0.273)
pub const MOON_ORBIT: f32 = 6.0 * PLANET_R; // moon orbit radius (Earth ~60 R; compressed so it's framed)
pub const SUN_DIST: f32 = 60.0 * PLANET_R; // sun far away (directional light); billboard sized to match moon's angular size
pub const SUN_R: f32 = SUN_DIST / MOON_ORBIT * MOON_R; // sun billboard radius -> same on-sky size as the moon
pub const DAY_TICKS: u32 = 2400; // ticks per planet rotation (one day) -- same cadence as the old flat day
pub const MOON_PERIOD_DAYS: f32 = 8.0; // moon orbits once per 8 days (a visible monthly cycle, sped up)
pub const AXIAL_TILT: f32 = 0.41; // ~23.5 deg in radians: gives seasons + keeps poles cold

// ---------- lat/lon <-> 3D ----------

/// Unit surface direction -> (lon, lat) in radians. lat in [-pi/2, pi/2] (poles at +/-Y), lon in (-pi, pi].
pub fn dir_to_lonlat(d: Vec3) -> (f32, f32) {
    let lat = d.y.clamp(-1.0, 1.0).asin();
    let lon = d.z.atan2(d.x);
    (lon, lat)
}

/// (lon, lat, elevation) -> world position. elevation is height above the sea sphere (world units).
pub fn lonlat_to_pos(lon: f32, lat: f32, elevation: f32) -> Vec3 {
    let (cl, sl) = (lat.cos(), lat.sin());
    let d = Vec3::new(cl * lon.cos(), sl, cl * lon.sin());
    d * (PLANET_R + elevation)
}

/// Surface normal (outward) at a world position.
pub fn normal(pos: Vec3) -> Vec3 {
    pos.normalize_or_zero()
}

/// Local tangent frame at surface direction `d`: (east, north), both unit + perpendicular to `d`.
/// north points toward +Y pole; east points toward increasing longitude. Degenerate near the poles
/// (east -> 0); callers moving exactly at a pole get an arbitrary but stable frame.
pub fn tangent_frame(d: Vec3) -> (Vec3, Vec3) {
    let axis = Vec3::Y;
    let mut east = axis.cross(d);
    if east.length_squared() < 1e-8 {
        east = Vec3::X; // at a pole: pick any consistent tangent
    }
    let east = east.normalize();
    let north = d.cross(east).normalize();
    (east, north)
}

/// Tangent direction for a compass heading at `d`: heading 0 = north, +pi/2 = east.
pub fn heading_tangent(d: Vec3, heading: f32) -> Vec3 {
    let (east, north) = tangent_frame(d);
    (north * heading.cos() + east * heading.sin()).normalize_or_zero()
}

/// Great-circle step: from world `pos`, move `dist` (world units) along compass `heading`. Returns the new
/// surface direction (unit) + the new heading (parallel-transported so "forward" stays consistent).
pub fn step(pos: Vec3, heading: f32, dist: f32) -> (Vec3, f32) {
    let d = pos.normalize_or_zero();
    let t = heading_tangent(d, heading);
    let ang = dist / PLANET_R; // arc angle = arc length / radius
    let (s, c) = (ang.sin(), ang.cos());
    let new_d = (d * c + t * s).normalize();
    // recompute heading in the new tangent frame from the transported forward vector
    let new_t = (-d * s + t * c).normalize_or_zero();
    let (east, north) = tangent_frame(new_d);
    let new_heading = new_t.dot(east).atan2(new_t.dot(north));
    (new_d, new_heading)
}

/// Great-circle distance (along the surface) between two world positions.
pub fn surface_dist(a: Vec3, b: Vec3) -> f32 {
    let da = a.normalize_or_zero();
    let db = b.normalize_or_zero();
    da.dot(db).clamp(-1.0, 1.0).acos() * PLANET_R
}

// ---------- 3D value-noise fBm (seamless on the sphere) ----------

fn hash3(i: i32, j: i32, k: i32) -> f32 {
    let mut h = (i.wrapping_mul(374761393))
        .wrapping_add(j.wrapping_mul(668265263))
        .wrapping_add(k.wrapping_mul(2147483647)) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn value_noise3(p: Vec3) -> f32 {
    let (xi, yi, zi) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (xf, yf, zf) = (p.x - xi, p.y - yi, p.z - zi);
    let (i, j, k) = (xi as i32, yi as i32, zi as i32);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v, w) = (smooth(xf), smooth(yf), smooth(zf));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c000 = hash3(i, j, k);
    let c100 = hash3(i + 1, j, k);
    let c010 = hash3(i, j + 1, k);
    let c110 = hash3(i + 1, j + 1, k);
    let c001 = hash3(i, j, k + 1);
    let c101 = hash3(i + 1, j, k + 1);
    let c011 = hash3(i, j + 1, k + 1);
    let c111 = hash3(i + 1, j + 1, k + 1);
    let x00 = lerp(c000, c100, u);
    let x10 = lerp(c010, c110, u);
    let x01 = lerp(c001, c101, u);
    let x11 = lerp(c011, c111, u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    lerp(y0, y1, w)
}

/// Fractal Brownian motion in 3D, ~0..1. Sampled on the unit sphere -> seamless terrain (no seam/poles).
pub fn fbm3(p: Vec3) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for _ in 0..4 {
        sum += amp * value_noise3(p * freq);
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / 0.9375
}

// ---------- terrain + climate fields on the sphere ----------

const TERRAIN_FREQ: f32 = 1.9; // continents/oceans scale (lower = bigger landmasses)

// Guaranteed landmarks blended onto the fbm base: (center dir, angular radius rad, amplitude). +amp pushes
// up a mountain massif, -amp carves a deep ocean basin. Ensures the planet always has >=2 mountain ranges
// and >=1 deep ocean regardless of the noise seed. fbm fills in the rest (coasts, hills, smaller seas).
const LANDMARKS: [([f32; 3], f32, f32); 5] = [
    ([0.95, 0.30, -0.05], 0.46, 0.46),   // mountain range A
    ([-0.65, 0.20, -0.75], 0.42, 0.46),  // mountain range B (opposite hemisphere)
    ([-0.10, -0.30, 0.95], 0.90, -0.50), // great deep ocean (large, abyssal center)
    ([0.55, -0.55, -0.62], 0.70, -0.34), // second ocean basin
    (HOMELAND_DIR, 0.50, 0.16),          // gentle homeland lowland (habitable founding ground)
];

fn terrain_features(d: Vec3) -> f32 {
    let dn = d.normalize_or_zero();
    let mut sum = 0.0;
    for (c, r, a) in LANDMARKS {
        let cc = Vec3::new(c[0], c[1], c[2]).normalize();
        let ang = dn.dot(cc).clamp(-1.0, 1.0).acos();
        let g = (-(ang / r) * (ang / r)).exp(); // gaussian falloff from the landmark center
        sum += a * g;
    }
    sum
}

/// Normalized terrain elevation 0..1 at surface direction `d` (continents, oceans, mountains).
pub fn elevation01(d: Vec3) -> f32 {
    (fbm3(d * TERRAIN_FREQ + Vec3::splat(11.3)) + terrain_features(d)).clamp(0.0, 1.0)
}

/// Terrain height in world units RELATIVE TO THE SEA SURFACE (the waterline reference at radius PLANET_R).
/// Positive on land (0 at the coast up to ELEV_MAX on peaks); NEGATIVE under the ocean (0 at the coast down
/// to -SEA_FLOOR_MAX at the abyssal center) -> one continuous signed bathymetry. Earlier this clamped ocean
/// to 0 (a flat seafloor at PLANET_R) while the render shell floated SEA_LEVEL*ELEV_MAX above it, so a wide
/// coastal band of LAND (elev01 SEA_LEVEL..~0.65) sat UNDER the shell: visually flooded yet classed as dry
/// land (no swim/tint, land plants + walkers stranded "in" the sea). Signing the field sinks the seafloor
/// below the shell and puts the waterline exactly at the coast, so render + sim agree on what is underwater.
pub fn elevation(d: Vec3) -> f32 {
    let e = elevation01(d);
    if e >= SEA_LEVEL {
        (e - SEA_LEVEL) / (1.0 - SEA_LEVEL) * ELEV_MAX // land: 0 at the coast .. ELEV_MAX on peaks
    } else {
        -((SEA_LEVEL - e) / SEA_LEVEL) * SEA_FLOOR_MAX // ocean: 0 at the coast .. -SEA_FLOOR_MAX at the abyss
    }
}

/// Is this surface point under the ocean?
pub fn is_ocean(d: Vec3) -> bool {
    elevation01(d) < SEA_LEVEL
}

/// Temperature 0..1 at `d`: warm at the equator, cold at the poles + at high elevation. The sub-solar
/// point also warms locally (day side warmer) once a tick is supplied via `solar_warmth`.
pub fn base_temperature(d: Vec3) -> f32 {
    let (_lon, lat) = dir_to_lonlat(d);
    let c = lat.cos(); // 1 at equator, 0 at poles
    // Extra polar chill: ramps in ONLY at high latitude (cos < ~0.55, i.e. |lat| > ~57 deg) so the temperate
    // + tropical band stays as-warm (population unaffected), but the high latitudes drop faster -> a deeper,
    // wider frozen zone. Quadratic: gentle at the edge, strongest at the pole.
    let polar = ((0.55 - c) / 0.55).clamp(0.0, 1.0); // 0 below ~57 deg .. 1 at the pole
    let by_lat = c - 0.45 * polar * polar;
    let lapse = elevation(d).max(0.0) / ELEV_MAX * 0.4; // high ground is colder (ocean depth: no lapse)
    (by_lat - lapse).clamp(0.0, 1.0)
}

/// Moisture 0..1 at `d`: oceans + low ground wet, a noise patch pattern on top (deserts emerge in dry
/// patches away from the coast). Latitude bands (wet tropics/poles, dry subtropics) add Earth-like belts.
pub fn moisture(d: Vec3) -> f32 {
    let (_lon, lat) = dir_to_lonlat(d);
    let coastal = 1.0 - (elevation(d).max(0.0) / ELEV_MAX).min(1.0); // low/coastal = wetter (ocean = fully wet)
    let patch = fbm3(d * 3.7 - Vec3::splat(5.0));
    // dry subtropical belts ~ +/-30 deg, wetter equator + poles
    let belt = 0.5 + 0.5 * (lat * 3.0).cos();
    (0.45 * coastal + 0.35 * patch + 0.20 * belt).clamp(0.0, 1.0)
}

/// World position sitting `offset` above the terrain surface at direction `d` (d need not be unit).
pub fn surface_pos(d: Vec3, offset: f32) -> Vec3 {
    let d = d.normalize_or_zero();
    d * (PLANET_R + elevation(d) + offset)
}

/// Rockiness 0..1 at `d`: 0 on low/mid ground, ramps to 1 on the highest peaks (hard to cross, few plants).
pub fn rockiness(d: Vec3) -> f32 {
    const ROCK_START: f32 = 0.72;
    ((elevation01(d) - ROCK_START) / (1.0 - ROCK_START)).clamp(0.0, 1.0)
}

/// Plant habitability 0..1 at `d`: 0 in ocean, reduced on rock, in drought, and in the cold (poles). Land
/// flora thrives in warm, moist, low ground -> plants + the creatures that eat them cluster temperate/tropical.
pub fn plant_habitability(d: Vec3) -> f32 {
    plant_habitability_with_moisture(d, moisture(d))
}

/// Plant habitability with an EXTERNALLY supplied land moisture, so the dynamic climate grid can override
/// the static `moisture(d)` field -> deserts + rainforests form as climate drifts. Ocean/aquatic + thermal
/// branches are moisture-independent (unchanged). `plant_habitability` = this with the static moisture.
pub fn plant_habitability_with_moisture(d: Vec3, moist: f32) -> f32 {
    let e = elevation01(d);
    if e < AQUATIC_FLOOR {
        return 0.0; // abyssal deep ocean: barren (no light reaches the bottom)
    }
    let warm_ok = 0.45 + 0.55 * base_temperature(d); // poles support hardy (cold-tolerant) flora, not barren
    if e < SEA_LEVEL {
        // water column grows aquatic flora (plankton/algae/seagrass) -> a real food base for swimmers across
        // the seas, richest in the shallows (coastal seagrass), thinning toward open water (plankton). Water
        // moderates temperature, so aquatic flora is less polar-sensitive than land flora. Trade-off: open
        // water feeds less than rich land, and swimmers pay the swim gene + are slow on land.
        let shallow = ((e - AQUATIC_FLOOR) / (SEA_LEVEL - AQUATIC_FLOOR)).clamp(0.0, 1.0); // 0 open .. 1 coast
        let water_warm = 0.7 + 0.3 * base_temperature(d);
        return ((0.55 + 0.30 * shallow) * water_warm).clamp(0.0, 1.0);
    }
    let rock_ok = 1.0 - 0.9 * rockiness(d);
    // dry-ground habitability with a desert floor: bone-dry land keeps DESERT_FLORA_FLOOR (rare scrub), and
    // since callers pass an effective moisture (static + rain ground water), a downpour lifts this -> bloom.
    let moist_ok = (moist / 0.35).clamp(0.0, 1.0).max(DESERT_FLORA_FLOOR);
    (rock_ok * moist_ok * warm_ok).clamp(0.0, 1.0)
}

/// Flammable fuel 0..1 at `d`: how readily this spot can BURN = how much dry vegetation it carries. 0 over
/// any water (oceans never burn, even shallow seagrass), 0 on bare rock + barren desert (no fuel). Rises
/// with land plant habitability (grass/forest = fuel). Wildfire gates on this so only vegetated land burns;
/// dryness (ground water) is a SEPARATE gate (wet vegetation resists fire) applied by the fire sim.
pub fn fuel(d: Vec3) -> f32 {
    if is_ocean(d) {
        return 0.0; // water carries no burnable fuel (plant_habitability is high in shallow seas -> exclude)
    }
    // frozen ground (the polar ice cap) carries no DRY fuel -> never burns. plant_habitability keeps a floor
    // at the poles (cold-niche flora stays alive), but snow-covered tundra does not carry fire, so gate fuel
    // to 0 across the deep-cold core (temp < 0.25). The biome frosts a bit wider (temp < 0.34), so the frost
    // edge (0.25..0.34) is tundra that still carries sparse fuel; the solid ice core stays a firebreak.
    let cold_ok = (base_temperature(d) / 0.25).clamp(0.0, 1.0); // 0 at the frozen pole .. 1 by the ice edge
    plant_habitability(d) * cold_ok // land only: ~0 on rock/desert/ice, high in lush temperate/tropical
}

/// Unlit biome color (RGB 0..1) at outward direction `d`: ocean by depth, land by elevation/moisture,
/// polar ice. Shared by the globe mesh (viz) + the snapshot renderer so they look the same.
pub fn biome_color(d: Vec3) -> [f32; 3] {
    biome_color_with_moisture(d, moisture(d))
}

/// As `biome_color` but with an EXTERNALLY supplied land moisture, so the globe render can recolor land
/// from the live climate grid (dry -> sand/desert, wet -> green) as climate drifts. Ocean depth + polar
/// ice branches are moisture-independent (unchanged). `biome_color` = this with the static moisture.
pub fn biome_color_with_moisture(d: Vec3, m: f32) -> [f32; 3] {
    let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    let temp = base_temperature(d);
    if is_ocean(d) {
        let depth = ((SEA_LEVEL - elevation01(d)) / SEA_LEVEL).clamp(0.0, 1.0);
        let mut c = lerp3([0.13, 0.40, 0.60], [0.02, 0.09, 0.28], depth);
        // sea ice: cold polar ocean freezes to pale pack ice, thickening toward the pole. The translucent
        // ocean shell tints over this, so a pale seabed reads as icy pale-blue polar water.
        if temp < 0.30 {
            c = lerp3(c, [0.86, 0.90, 0.94], (0.30 - temp) / 0.30);
        }
        return c;
    }
    let elev = (elevation(d) / ELEV_MAX).clamp(0.0, 1.0);
    let mut c = lerp3([0.20, 0.55, 0.22], [0.48, 0.40, 0.26], elev);
    // bare gray rock on the rockiest highland (rockiness ramps in above ROCK_START) -> rocky land reads as
    // stone, not just dark soil. Stops short of full gray so grass between the rocks still shows green.
    let rock = rockiness(d);
    if rock > 0.0 {
        c = lerp3(c, [0.44, 0.42, 0.40], rock * 0.85);
    }
    if m < 0.35 {
        c = lerp3(c, [0.80, 0.72, 0.45], (0.35 - m) / 0.35);
    }
    // polar ice cap: wider onset (temp < 0.34) + bright snow white. Frosts in at the edge, full ice at the pole.
    if temp < 0.34 {
        c = lerp3(c, [0.95, 0.96, 0.98], (0.34 - temp) / 0.34);
    }
    c
}

/// Sample a random surface direction inside a "homeland" cap: within `cap_rad` radians of `center`.
/// Used to start the population LOCALIZED in one region (it then spreads). cap_rad = PI = whole globe.
pub fn random_dir_in_cap(rng: &mut crate::rng::Rng, center: Vec3, cap_rad: f32) -> Vec3 {
    let center = center.normalize_or_zero();
    // uniform in a spherical cap: cos(theta) in [cos(cap_rad), 1], azimuth uniform
    let cos_min = cap_rad.cos();
    let cos_t = cos_min + (1.0 - cos_min) * rng.f32();
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = rng.f32() * std::f32::consts::TAU;
    let (east, north) = tangent_frame(center);
    (center * cos_t + (east * phi.cos() + north * phi.sin()) * sin_t).normalize()
}

// ---------- sun + moon ----------

/// Sun direction (unit) at `tick`: the planet spins about its tilted axis, so the sun sweeps longitudes
/// once per DAY_TICKS. Tilt keeps the sub-solar latitude near the equator, so the poles stay cold.
pub fn sun_dir(tick: u32) -> Vec3 {
    let a = (tick as f32 / DAY_TICKS as f32) * std::f32::consts::TAU;
    // sun in the equatorial plane, lifted by axial tilt so high latitudes get glancing rays
    Vec3::new(a.cos(), AXIAL_TILT.sin() * (a * 0.13).sin(), a.sin()).normalize()
}

/// Moon direction (unit) at `tick`: orbits slower than the day + on a slightly inclined plane, so it
/// drifts against the day/night cycle (visible phases as it catches up to / falls behind the sun).
pub fn moon_dir(tick: u32) -> Vec3 {
    let period = DAY_TICKS as f32 * MOON_PERIOD_DAYS;
    let a = (tick as f32 / period) * std::f32::consts::TAU;
    Vec3::new(a.cos(), 0.18 * a.sin(), a.sin() * 0.98).normalize()
}

pub fn moon_pos(tick: u32) -> Vec3 {
    moon_dir(tick) * MOON_ORBIT
}

/// Local daylight 0..1 at surface direction `d` for the given tick: how much the point faces the sun.
pub fn daylight_at(d: Vec3, tick: u32) -> f32 {
    d.dot(sun_dir(tick)).clamp(0.0, 1.0)
}

// ---------- clouds + cloud-driven rain ----------

const CLOUD_FREQ: f32 = 3.0;     // cloud patch size (higher = smaller, more patches)
// Wind is now LATITUDE-BANDED, not one uniform spin: see `zonal_wind`. WIND_PEAK = the strongest band
// (rad/tick); bands shear past each other -> clouds at different latitudes drift at different speeds + dirs.
const WIND_PEAK: f32 = 0.0011;
// Slow secondary morph: nudge the noise through its 3rd axis over time so the cloud PATTERN itself evolves
// (forms + dissolves) instead of rigidly circling forever. ~1 unit of fbm space per ~25k ticks = gentle.
const CLOUD_MORPH: f32 = 0.00004;
// Climate-drift field (geological): the SLOW regional wet/dry anomaly the climate memory chases. Same
// rotate-the-sample-point trick as clouds, but ~500x slower so wet belts migrate over years, not days.
const CLIMATE_DRIFT: f32 = 0.45;     // anomaly amplitude (0 = static climate, 1 = strong wet-belt migration)
const CLIMATE_SPEED: f32 = 0.0000019; // rad/tick the anomaly rotates: full sweep TAU/speed ~3.3M ticks ~1380 days
const CLIMATE_FREQ: f32 = 1.3;       // anomaly patch size (low = continent-scale wet/dry zones, not speckle)
const CLOUD_COVER: f32 = 0.55;   // noise threshold: above this is cloudy (higher = sparser clouds)
pub const CLOUD_RAIN_MIN: f32 = 0.45; // cloud cover above which rain can fall (thick-ish cloud)
// Rain-mask threshold on a second fbm field: rain falls only where the mask exceeds this. fbm3 clusters
// mid-range + rarely tops ~0.85, so the old `1.0 - 0.10 = 0.90` gate was UNREACHABLE -> rain never fell.
// 0.60 sits in the field's upper band -> scattered, drifting rain cells under the thicker clouds.
pub const RAIN_MASK_MIN: f32 = 0.60;

/// Signed zonal (east-west) wind at direction `d`, rad/tick. Earth-like 3-band flow: gentle equatorial
/// easterlies (-), strong mid-latitude westerlies (+), tapering toward the poles. + drifts west->east, -
/// east->west. Rotation about the spin axis preserves latitude, so each cloud keeps its band -> the bands
/// shear past each other and clouds move at visibly different speeds + directions by latitude.
pub fn zonal_wind(d: Vec3) -> f32 {
    let (_lon, lat) = dir_to_lonlat(d);
    // 0.35 - 0.65*cos(3*lat): equator ~ -0.30 (mild easterly), ~30deg ~ +0.35, ~60deg ~ +1.0 (jet), pole
    // ~ +0.35. Most of the populated mid-latitudes drift west->east; only the deep tropics reverse.
    WIND_PEAK * (0.35 - 0.65 * (3.0 * lat).cos())
}

/// Cloud cover 0..1 at surface direction `d` and `tick`: a scrolling 3D-fBm field. 0 = clear sky, 1 = thick
/// overcast. Drift is latitude-banded (`zonal_wind`) so clouds move at different speeds per band, and a slow
/// morph evolves the pattern so cloud systems form + dissolve, not just circle. Deterministic -> headless +
/// render agree. Drives local shade (visual + plant light) and is the ONLY source of rain (see `rain_at`).
pub fn cloud_cover(d: Vec3, tick: u32) -> f32 {
    // rotate the sample point about the spin axis by this latitude's wind so the pattern drifts per band
    let a = tick as f32 * zonal_wind(d);
    let (s, c) = (a.sin(), a.cos());
    let rot = Vec3::new(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    // walk the 3rd noise axis slowly -> the cloud pattern itself slowly reshapes (form + dissolve)
    let morph = tick as f32 * CLOUD_MORPH;
    let n = fbm3(rot * CLOUD_FREQ + Vec3::new(31.7, morph, 7.0));
    ((n - CLOUD_COVER) / (1.0 - CLOUD_COVER)).clamp(0.0, 1.0)
}

/// Rain intensity 0..1 at `d`,`tick`. Rain comes ONLY from clouds: it can rain solely where cloud cover is
/// thick (> CLOUD_RAIN_MIN), and within that only where a separate slow-drifting mask field is high
/// (> RAIN_MASK_MIN) -> rain falls in scattered, moving cells under the thicker clouds, not everywhere.
pub fn rain_at(d: Vec3, tick: u32) -> f32 {
    let cover = cloud_cover(d, tick);
    if cover <= CLOUD_RAIN_MIN {
        return 0.0;
    }
    let a = tick as f32 * zonal_wind(d) * 0.7; // rain bands drift with this band's wind, a touch slower
    let (s, c) = (a.sin(), a.cos());
    let rot = Vec3::new(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    let mask = fbm3(rot * (CLOUD_FREQ * 1.7) + Vec3::splat(71.2));
    if mask < RAIN_MASK_MIN {
        return 0.0; // cloudy but not raining here
    }
    cover // rain as heavy as the cloud is thick
}

/// Long-run climate moisture target 0..1 at `d` for `tick`: the value this cell's slow climate memory
/// drifts toward. Static moisture baseline + a VERY slowly rotating regional anomaly (continent-scale wet
/// vs dry patches) so some regions stay rainier and the wet/dry belts migrate over years -> deserts +
/// rainforests form, persist, move. Pure + deterministic -> headless + render agree. The slow `Climate`
/// grid (sim.rs) low-pass-filters this; sampling it directly = the instantaneous target, not the climate.
pub fn climate_target(d: Vec3, tick: u32) -> f32 {
    let base = moisture(d);
    // rotate the sample point slowly about the spin axis -> the anomaly pattern migrates across the surface
    let a = tick as f32 * CLIMATE_SPEED;
    let (s, c) = (a.sin(), a.cos());
    let rot = Vec3::new(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    let anomaly = fbm3(rot * CLIMATE_FREQ + Vec3::splat(57.4)); // ~0..0.9, mean ~0.5
    let bias = CLIMATE_DRIFT * (anomaly - 0.5); // center -> push some regions wetter, others drier than baseline
    (base + bias).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Diagnostic (run: cargo test report_geography -- --nocapture): reports ocean/deep/mountain coverage so
    // SEA_LEVEL can be tuned to ~50% ocean. The median elevation is the SEA_LEVEL giving exactly 50% ocean.
    #[test]
    fn report_geography() {
        let n = 40000usize;
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        let dir = |i: usize| {
            let y = 1.0 - (i as f32 + 0.5) / n as f32 * 2.0;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let th = golden * i as f32;
            Vec3::new(th.cos() * r, y, th.sin() * r)
        };
        let mut elev: Vec<f32> = (0..n).map(|i| elevation01(dir(i))).collect();
        elev.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = elev[n / 2];
        let frac_below = |t: f32| elev.iter().filter(|&&e| e < t).count() as f32 / n as f32;
        let mountain = (0..n).filter(|&i| rockiness(dir(i)) > 0.2).count() as f32 / n as f32;
        let hl = Vec3::from(HOMELAND_DIR).normalize();
        println!(
            "GEO: median={:.3} | ocean@sea{:.2}={:.1}% | deep(<{:.2})={:.1}% | mountain(rocky)={:.1}%",
            median, SEA_LEVEL, 100.0 * frac_below(SEA_LEVEL), AQUATIC_FLOOR, 100.0 * frac_below(AQUATIC_FLOOR), 100.0 * mountain
        );
        println!(
            "HOMELAND: elev01={:.3} (sea {:.2}, rock 0.72) temp={:.2} habitability={:.2} -> {}",
            elevation01(hl), SEA_LEVEL, base_temperature(hl), plant_habitability(hl),
            if elevation01(hl) > SEA_LEVEL && elevation01(hl) < 0.72 && plant_habitability(hl) > 0.4 { "GOOD land" } else { "BAD" }
        );
    }

    #[test]
    fn lonlat_roundtrip() {
        for &(lon, lat) in &[(0.3f32, 0.4f32), (-2.1, -0.9), (1.7, 0.0)] {
            let p = lonlat_to_pos(lon, lat, 0.0);
            let (lon2, lat2) = dir_to_lonlat(p.normalize());
            assert!((lon - lon2).abs() < 1e-3, "lon {lon} vs {lon2}");
            assert!((lat - lat2).abs() < 1e-3, "lat {lat} vs {lat2}");
        }
    }

    #[test]
    fn step_stays_on_sphere_and_conserves_distance() {
        let start = lonlat_to_pos(0.0, 0.1, 0.0);
        let (d, _h) = step(start, 1.0, 5.0);
        assert!((d.length() - 1.0).abs() < 1e-4, "step result must be a unit direction");
        // a great-circle step of length L moves the surface point ~L along the surface
        let moved = surface_dist(start, d * PLANET_R);
        assert!((moved - 5.0).abs() < 0.2, "moved {moved}, expected ~5");
    }

    #[test]
    fn tangent_is_perpendicular() {
        let d = lonlat_to_pos(0.5, 0.6, 0.0).normalize();
        let (east, north) = tangent_frame(d);
        assert!(east.dot(d).abs() < 1e-4);
        assert!(north.dot(d).abs() < 1e-4);
        assert!(east.dot(north).abs() < 1e-3, "east/north should be orthogonal");
    }

    #[test]
    fn poles_colder_than_equator() {
        let eq = lonlat_to_pos(0.0, 0.0, 0.0).normalize();
        let pole = lonlat_to_pos(0.0, 1.55, 0.0).normalize();
        assert!(base_temperature(eq) > base_temperature(pole));
    }

    #[test]
    fn clouds_vary_and_drift() {
        // the cloud field spans clear..cloudy across the globe, and a fixed point changes over time (drift)
        let mut min = 1.0f32;
        let mut max = 0.0f32;
        for i in 0..200 {
            let lon = i as f32 * 0.3;
            let lat = (i as f32 * 0.11).sin();
            let cover = cloud_cover(lonlat_to_pos(lon, lat, 0.0).normalize(), 100);
            min = min.min(cover);
            max = max.max(cover);
        }
        assert!(min < 0.05 && max > 0.5, "clouds should range clear..thick (min {min}, max {max})");
        let p = lonlat_to_pos(0.4, 0.2, 0.0).normalize();
        let a = cloud_cover(p, 0);
        let b = cloud_cover(p, 4000);
        assert!((a - b).abs() > 1e-3, "clouds should drift over time");
    }

    #[test]
    fn day_and_night_exist() {
        // at any tick, some longitude faces the sun (day) and the antipode does not (night)
        let s = sun_dir(600);
        let day = (s * PLANET_R).normalize();
        let night = -day;
        assert!(daylight_at(day, 600) > 0.5);
        assert!(daylight_at(night, 600) < 0.01);
    }
}
