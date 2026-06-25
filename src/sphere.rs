//! Spherical world geometry. Creatures live on planet SURFACE (radius PLANET_R), not flat x,z plane.
//! Position = 3D point; surface direction `d = pos.normalize()` gives lat, lon, tangent frame (east/north),
//! surface normal. Movement = great-circle step in tangent plane. Terrain/temperature/moisture = seamless
//! 3D-noise fields on sphere (no edge, no wrap seam). Sun + moon orbit; day/night positional (lit half
//! faces sun). Earth-like proportions, stylized so moon stays on-screen: moon radius ~0.27x planet, sun
//! ~same angular size as moon (real Earth coincidence), poles get glancing sun -> cold.
#![allow(dead_code)] // wired into sim/viz incrementally; keep module buildable + testable standalone
use crate::terrain::HEIGHT_MAX;
use bevy::prelude::*;

pub const PLANET_R: f32 = 80.0; // radius (world units). Matches old WORLD_HALF so creature scale + costs carry over.
pub const ELEV_MAX: f32 = HEIGHT_MAX; // max terrain elevation above sea sphere (reuses flat-world peak)
pub const SEA_LEVEL: f32 = 0.41; // normalized elev (0..1) below this floods (ocean) -> ~50% sea
pub const SEA_FLOOR_MAX: f32 = 9.0; // max ocean DEPTH below sea surface at abyssal center (world units)
// Homeland founding dir: temperate mid-latitude, kept gentle habitable lowland by a land landmark so
// founders never spawn on peak or in sea. Shared by terrain features + sim::homeland_center.
pub const HOMELAND_DIR: [f32; 3] = [0.30, 0.50, 0.40];
// Aquatic flora grows in water column from here up to SEA_LEVEL (coast). Below = abyssal/barren. Wide band
// -> swimmers get sea-wide habitat, not thin coastal ring (fish niche kept going extinct).
pub const AQUATIC_FLOOR: f32 = 0.12;
// Bone-dry desert keeps sliver of habitability so rare drought-tolerant flora (cacti/scrub/short grass)
// persists; rain pulse (ground water -> effective moisture) blooms it briefly. Below grass seed gate
// (GRASS_HAB_MIN) when dry, so desert stays sparse until it rains.
pub const DESERT_FLORA_FLOOR: f32 = 0.12;

// --- celestial bodies (Earth proportions, distances stylized down to stay visible) ---
pub const MOON_R: f32 = 0.27 * PLANET_R; // moon ~1/4 planet radius (Earth: 0.273)
pub const MOON_ORBIT: f32 = 6.0 * PLANET_R; // orbit radius (Earth ~60 R; compressed so framed)
pub const SUN_DIST: f32 = 60.0 * PLANET_R; // sun far (directional light); billboard sized to match moon angular size
pub const SUN_R: f32 = SUN_DIST / MOON_ORBIT * MOON_R; // billboard radius -> same on-sky size as moon
pub const DAY_TICKS: u32 = 2400; // ticks per planet rotation (one day). Same cadence as old flat day.
pub const AXIAL_TILT: f32 = 0.41; // ~23.5 deg radians (matches TSN Earth -23.439): seasons + keeps poles cold
// Calendar: mean year = orrery::MEAN_YEAR_DAYS = 360 d = MONTHS_PER_YEAR x MONTH_DAYS. Actual year length
// breathes with Sirius distance (orrery::year_len_days, ~332..390); the calendar GRID uses the clean mean so
// dates stay monotonic, the live variation surfaces as an astronomical readout (viz). Day = DAY_TICKS ticks.
pub const MONTH_DAYS: u32 = 30;
pub const MONTHS_PER_YEAR: u32 = 12;

// --- planet magnetic field (tilted geomagnetic dipole) ---
// Magnetic north tilted off spin axis (+Y). Offset -> nonzero compass declination + auroral oval off
// geographic pole (Earth: ~11 deg). Field fed to creatures via `magneto` sense gene.
pub const MAG_TILT: f32 = 0.20; // dipole tilt off rotation axis (~11.5 deg)

// ---------- lat/lon <-> 3D ----------

/// Unit surface dir -> (lon, lat) radians. lat in [-pi/2, pi/2] (poles at +/-Y), lon in (-pi, pi].
pub fn dir_to_lonlat(d: Vec3) -> (f32, f32) {
    let lat = d.y.clamp(-1.0, 1.0).asin();
    let lon = d.z.atan2(d.x);
    (lon, lat)
}

/// (lon, lat, elevation) -> world pos. elevation = height above sea sphere (world units).
pub fn lonlat_to_pos(lon: f32, lat: f32, elevation: f32) -> Vec3 {
    let (cl, sl) = (lat.cos(), lat.sin());
    let d = Vec3::new(cl * lon.cos(), sl, cl * lon.sin());
    d * (PLANET_R + elevation)
}

/// Outward surface normal at world pos.
pub fn normal(pos: Vec3) -> Vec3 {
    pos.normalize_or_zero()
}

/// Local tangent frame at surface dir `d`: (east, north), both unit + perpendicular to `d`. north -> +Y
/// pole; east -> increasing lon. Degenerate near poles (east -> 0); at a pole callers get arbitrary but
/// stable frame.
pub fn tangent_frame(d: Vec3) -> (Vec3, Vec3) {
    let axis = Vec3::Y;
    let mut east = axis.cross(d);
    if east.length_squared() < 1e-8 {
        east = Vec3::X; // at pole: pick any consistent tangent
    }
    let east = east.normalize();
    let north = d.cross(east).normalize();
    (east, north)
}

/// Tangent dir for compass heading at `d`: heading 0 = north, +pi/2 = east.
pub fn heading_tangent(d: Vec3, heading: f32) -> Vec3 {
    let (east, north) = tangent_frame(d);
    (north * heading.cos() + east * heading.sin()).normalize_or_zero()
}

/// Great-circle step: from world `pos`, move `dist` (world units) along compass `heading`. Returns new
/// surface dir (unit) + new heading (parallel-transported so "forward" stays consistent).
pub fn step(pos: Vec3, heading: f32, dist: f32) -> (Vec3, f32) {
    let d = pos.normalize_or_zero();
    let t = heading_tangent(d, heading);
    let ang = dist / PLANET_R; // arc angle = arc length / radius
    let (s, c) = (ang.sin(), ang.cos());
    let new_d = (d * c + t * s).normalize();
    // recompute heading in new tangent frame from transported forward vector
    let new_t = (-d * s + t * c).normalize_or_zero();
    let (east, north) = tangent_frame(new_d);
    let new_heading = new_t.dot(east).atan2(new_t.dot(north));
    (new_d, new_heading)
}

/// Great-circle dist (along surface, not chord) between two world positions.
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

/// Fractal Brownian motion 3D, ~0..1. Sampled on unit sphere -> seamless terrain (no seam/poles).
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

// Guaranteed landmarks blended onto fbm base: (center dir, angular radius rad, amplitude). +amp pushes up
// mountain massif, -amp carves deep ocean basin. Ensures planet always has >=2 mountain ranges + >=1 deep
// ocean regardless of noise seed. fbm fills rest (coasts, hills, smaller seas).
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
        let g = (-(ang / r) * (ang / r)).exp(); // gaussian falloff from landmark center
        sum += a * g;
    }
    sum
}

/// Normalized terrain elevation 0..1 at surface dir `d` (continents, oceans, mountains).
pub fn elevation01(d: Vec3) -> f32 {
    (fbm3(d * TERRAIN_FREQ + Vec3::splat(11.3)) + terrain_features(d)).clamp(0.0, 1.0)
}

/// Terrain height world units RELATIVE TO SEA SURFACE (waterline ref at radius PLANET_R). Positive on land
/// (0 at coast .. ELEV_MAX on peaks); NEGATIVE under ocean (0 at coast .. -SEA_FLOOR_MAX at abyssal center)
/// = one continuous signed bathymetry. INVARIANT: signed so seafloor sinks below render shell + waterline
/// sits exactly at coast -> render + sim agree on what is underwater. History: earlier clamped ocean to 0
/// (flat seafloor at PLANET_R) while render shell floated SEA_LEVEL*ELEV_MAX above it, so coastal LAND band
/// (elev01 SEA_LEVEL..~0.65) sat UNDER shell: visually flooded yet classed dry (no swim/tint, plants +
/// walkers stranded "in" sea).
pub fn elevation(d: Vec3) -> f32 {
    let e = elevation01(d);
    if e >= SEA_LEVEL {
        (e - SEA_LEVEL) / (1.0 - SEA_LEVEL) * ELEV_MAX // land: 0 at coast .. ELEV_MAX on peaks
    } else {
        -((SEA_LEVEL - e) / SEA_LEVEL) * SEA_FLOOR_MAX // ocean: 0 at coast .. -SEA_FLOOR_MAX at abyss
    }
}

/// Surface point under ocean?
pub fn is_ocean(d: Vec3) -> bool {
    elevation01(d) < SEA_LEVEL
}

/// Temperature 0..1 at `d`: warm at equator, cold at poles + high elevation. Sub-solar point also warms
/// locally (day side warmer) once a tick supplied via `solar_warmth`.
pub fn base_temperature(d: Vec3) -> f32 {
    let (_lon, lat) = dir_to_lonlat(d);
    let c = lat.cos(); // 1 at equator, 0 at poles
    // Extra polar chill: ramps in ONLY at high lat (cos < ~0.55, i.e. |lat| > ~57 deg) so temperate +
    // tropical band stays as-warm (population unaffected), high lat drops faster -> deeper, wider frozen
    // zone. Quadratic: gentle at edge, strongest at pole.
    let polar = ((0.55 - c) / 0.55).clamp(0.0, 1.0); // 0 below ~57 deg .. 1 at pole
    let by_lat = c - 0.45 * polar * polar;
    let lapse = elevation(d).max(0.0) / ELEV_MAX * 0.4; // high ground colder (ocean depth: no lapse)
    (by_lat - lapse).clamp(0.0, 1.0)
}

/// Moisture 0..1 at `d`: oceans + low ground wet, noise patch pattern on top (deserts emerge in dry patches
/// away from coast). Latitude bands (wet tropics/poles, dry subtropics) add Earth-like belts.
pub fn moisture(d: Vec3) -> f32 {
    let (_lon, lat) = dir_to_lonlat(d);
    let coastal = 1.0 - (elevation(d).max(0.0) / ELEV_MAX).min(1.0); // low/coastal = wetter (ocean = fully wet)
    let patch = fbm3(d * 3.7 - Vec3::splat(5.0));
    // dry subtropical belts ~ +/-30 deg, wetter equator + poles
    let belt = 0.5 + 0.5 * (lat * 3.0).cos();
    (0.45 * coastal + 0.35 * patch + 0.20 * belt).clamp(0.0, 1.0)
}

/// Magnetic north pole dir (unit): spin axis +Y tilted by MAG_TILT toward +X (fixed lon).
pub fn mag_pole_dir() -> Vec3 {
    Vec3::new(MAG_TILT.sin(), MAG_TILT.cos(), 0.0) // already unit (sin^2+cos^2=1)
}

/// Geomagnetic dipole field VECTOR at unit surface dir `d` (world frame, magnitude in dipole units).
/// B = 3(m.d)d - m, m = magnetic moment along tilted pole. Stronger + more vertical near poles.
pub fn mag_field(d: Vec3) -> Vec3 {
    let m = mag_pole_dir();
    3.0 * m.dot(d) * d - m
}

/// Magnetic latitude proxy at `d`: sin(mag lat), -1 (mag south pole) .. +1 (mag north pole), 0 on mag
/// equator. Inclination/"map" cue: how far creature is from magnetic poles.
pub fn mag_latitude(d: Vec3) -> f32 {
    d.dot(mag_pole_dir()).clamp(-1.0, 1.0)
}

/// Bearing of horizontal field component toward magnetic north, radians rel GEOGRAPHIC north ("compass"
/// cue). Nonzero declination under MAG_TILT -> carries real direction info. 0 at a pole.
pub fn mag_north_bearing(d: Vec3) -> f32 {
    let b = mag_field(d);
    let bh = b - d * b.dot(d); // horizontal component (project out radial/vertical part)
    if bh.length_squared() < 1e-8 {
        return 0.0; // near mag pole field is vertical -> no horizontal heading
    }
    let (east, north) = tangent_frame(d);
    bh.dot(east).atan2(bh.dot(north))
}

/// Field intensity proxy at `d` (closed-form dipole magnitude): ~1 at mag equator, ~2 at poles. Brightens
/// aurora; not a brain input in v1.
pub fn mag_intensity(d: Vec3) -> f32 {
    let md = d.dot(mag_pole_dir());
    (1.0 + 3.0 * md * md).sqrt()
}

/// World pos `offset` above terrain surface at dir `d` (d need not be unit).
pub fn surface_pos(d: Vec3, offset: f32) -> Vec3 {
    let d = d.normalize_or_zero();
    d * (PLANET_R + elevation(d) + offset)
}

/// Rockiness 0..1 at `d`: 0 on low/mid ground, ramps to 1 on highest peaks (hard to cross, few plants).
pub fn rockiness(d: Vec3) -> f32 {
    const ROCK_START: f32 = 0.72;
    ((elevation01(d) - ROCK_START) / (1.0 - ROCK_START)).clamp(0.0, 1.0)
}

/// Plant habitability 0..1 at `d`: 0 in ocean, reduced on rock, drought, cold (poles). Land flora thrives
/// warm/moist/low -> plants + eaters cluster temperate/tropical.
pub fn plant_habitability(d: Vec3) -> f32 {
    plant_habitability_with_moisture(d, moisture(d))
}

/// Plant habitability with EXTERNALLY supplied land moisture, so dynamic climate grid can override static
/// `moisture(d)` -> deserts + rainforests form as climate drifts. Ocean/aquatic + thermal branches
/// moisture-independent (unchanged). `plant_habitability` = this with static moisture.
pub fn plant_habitability_with_moisture(d: Vec3, moist: f32) -> f32 {
    let e = elevation01(d);
    if e < AQUATIC_FLOOR {
        return 0.0; // abyssal deep ocean: barren (no light reaches bottom)
    }
    let warm_ok = 0.45 + 0.55 * base_temperature(d); // poles support hardy cold-tolerant flora, not barren
    if e < SEA_LEVEL {
        // Water column grows aquatic flora (plankton/algae/seagrass) -> food base for swimmers sea-wide,
        // richest in shallows (coastal seagrass), thinning toward open water (plankton). Water moderates
        // temp, so aquatic flora less polar-sensitive than land. Trade-off: open water feeds less than rich
        // land, swimmers pay swim gene + slow on land.
        let shallow = ((e - AQUATIC_FLOOR) / (SEA_LEVEL - AQUATIC_FLOOR)).clamp(0.0, 1.0); // 0 open .. 1 coast
        let water_warm = 0.7 + 0.3 * base_temperature(d);
        return ((0.55 + 0.30 * shallow) * water_warm).clamp(0.0, 1.0);
    }
    let rock_ok = 1.0 - 0.9 * rockiness(d);
    // Dry-ground habitability with desert floor: bone-dry land keeps DESERT_FLORA_FLOOR (rare scrub).
    // Callers pass effective moisture (static + rain ground water), so downpour lifts this -> bloom.
    let moist_ok = (moist / 0.35).clamp(0.0, 1.0).max(DESERT_FLORA_FLOOR);
    (rock_ok * moist_ok * warm_ok).clamp(0.0, 1.0)
}

/// Flammable fuel 0..1 at `d`: how readily spot can BURN = dry vegetation it carries. 0 over any water
/// (oceans never burn, even shallow seagrass), 0 on bare rock + barren desert. Rises with land plant
/// habitability (grass/forest = fuel). Wildfire gates on this so only vegetated land burns; dryness (ground
/// water) is SEPARATE gate (wet vegetation resists fire) applied by fire sim.
pub fn fuel(d: Vec3) -> f32 {
    if is_ocean(d) {
        return 0.0; // water carries no burnable fuel (plant_habitability high in shallow seas -> exclude)
    }
    // Frozen ground (polar ice cap) carries no DRY fuel -> never burns. plant_habitability keeps floor at
    // poles (cold-niche flora alive), but snow tundra carries no fire, so gate fuel to 0 across deep-cold
    // core (temp < 0.25). Biome frosts wider (temp < 0.34), so frost edge (0.25..0.34) = tundra with sparse
    // fuel; solid ice core stays firebreak.
    let cold_ok = (base_temperature(d) / 0.25).clamp(0.0, 1.0); // 0 at frozen pole .. 1 by ice edge
    plant_habitability(d) * cold_ok // land only: ~0 on rock/desert/ice, high in lush temperate/tropical
}

/// Unlit biome color (RGB 0..1) at outward dir `d`: ocean by depth, land by elevation/moisture, polar ice.
/// Shared by globe mesh (viz) + snapshot renderer so they match.
pub fn biome_color(d: Vec3) -> [f32; 3] {
    biome_color_with_moisture(d, moisture(d))
}

/// As `biome_color` but EXTERNALLY supplied land moisture, so globe render recolors land from live climate
/// grid (dry -> sand/desert, wet -> green) as climate drifts. Ocean depth + polar ice branches
/// moisture-independent (unchanged). `biome_color` = this with static moisture.
pub fn biome_color_with_moisture(d: Vec3, m: f32) -> [f32; 3] {
    let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    let temp = base_temperature(d);
    if is_ocean(d) {
        let depth = ((SEA_LEVEL - elevation01(d)) / SEA_LEVEL).clamp(0.0, 1.0);
        let mut c = lerp3([0.13, 0.40, 0.60], [0.02, 0.09, 0.28], depth);
        // Sea ice: cold polar ocean freezes to pale pack ice, thickening toward pole. This colors the SEABED
        // (seen when swimming); the opaque ocean surface bakes its own matching depth+ice gradient in main.rs.
        if temp < 0.30 {
            c = lerp3(c, [0.86, 0.90, 0.94], (0.30 - temp) / 0.30);
        }
        return c;
    }
    let elev = (elevation(d) / ELEV_MAX).clamp(0.0, 1.0);
    let mut c = lerp3([0.20, 0.55, 0.22], [0.48, 0.40, 0.26], elev);
    // Bare gray rock on rockiest highland (rockiness ramps above ROCK_START) -> rocky land reads as stone,
    // not dark soil. Stops short of full gray so grass between rocks still shows green.
    let rock = rockiness(d);
    if rock > 0.0 {
        c = lerp3(c, [0.44, 0.42, 0.40], rock * 0.85);
    }
    if m < 0.35 {
        c = lerp3(c, [0.80, 0.72, 0.45], (0.35 - m) / 0.35);
    }
    // Polar ice cap: wider onset (temp < 0.34) + bright snow white. Frosts at edge, full ice at pole.
    if temp < 0.34 {
        c = lerp3(c, [0.95, 0.96, 0.98], (0.34 - temp) / 0.34);
    }
    c
}

/// Random surface dir inside "homeland" cap: within `cap_rad` radians of `center`. Starts population
/// LOCALIZED in one region (then spreads). cap_rad = PI = whole globe.
pub fn random_dir_in_cap(rng: &mut crate::rng::Rng, center: Vec3, cap_rad: f32) -> Vec3 {
    let center = center.normalize_or_zero();
    // uniform in spherical cap: cos(theta) in [cos(cap_rad), 1], azimuth uniform
    let cos_min = cap_rad.cos();
    let cos_t = cos_min + (1.0 - cos_min) * rng.f32();
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = rng.f32() * std::f32::consts::TAU;
    let (east, north) = tangent_frame(center);
    (center * cos_t + (east * phi.cos() + north * phi.sin()) * sin_t).normalize()
}

// ---------- sun + moon ----------

/// Elapsed time in mean-years (real time; drives orrery precession + body angles). 1 = one 360-day year.
pub fn t_years(tick: u32) -> f32 {
    tick as f32 / (crate::orrery::MEAN_YEAR_DAYS * DAY_TICKS as f32)
}

/// Map a geocentric ECLIPTIC dir (orrery output, ~in ecliptic plane) into the planet-fixed sky: tilt the
/// ecliptic by the axial tilt (so the sun's sub-solar point drifts +/-tilt in latitude over the year =
/// real seasons), then spin once per DAY_TICKS (day/night). Sun/moon/planets all go through this.
pub fn ecliptic_to_sky(ecl: Vec3, tick: u32) -> Vec3 {
    let daily = (tick as f32 / DAY_TICKS as f32) * std::f32::consts::TAU;
    (Quat::from_rotation_y(daily) * Quat::from_rotation_x(AXIAL_TILT) * ecl).normalize_or_zero()
}

/// Sun dir (unit) at `tick`. Delegates to the Tychos model: yearly ecliptic sun dir (seasons via tilt) +
/// daily spin. Sub-solar latitude band identical to the old fake (+/-sin AXIAL_TILT) so climate is preserved;
/// only the season CADENCE changes (was a ~7.7-day wobble, now one ~360-day year).
pub fn sun_dir(tick: u32) -> Vec3 {
    ecliptic_to_sky(crate::orrery::sun_ecliptic_dir(t_years(tick)), tick)
}

/// Moon dir (unit) at `tick`: real Tychos geocentric moon (~monthly, was a fixed 8-day circle), through the
/// same sky transform so it rises/sets with the day and shows phases as it drifts against the sun.
pub fn moon_dir(tick: u32) -> Vec3 {
    ecliptic_to_sky(crate::orrery::moon_ecliptic_dir(t_years(tick)), tick)
}

pub fn moon_pos(tick: u32) -> Vec3 {
    moon_dir(tick) * MOON_ORBIT
}

/// Calendar date string from `tick`. Calendar grid uses the clean mean year (12 mo x 30 d); Sirius-driven
/// year-length variation is shown separately as an astronomical readout (viz).
pub fn fmt_date(tick: u32) -> String {
    let year_days = MONTH_DAYS * MONTHS_PER_YEAR; // 360
    let d = tick / DAY_TICKS; // whole days since epoch
    let yr = d / year_days;
    let rem = d % year_days;
    format!("Yr {} Mon {} Day {}", yr + 1, rem / MONTH_DAYS + 1, rem % MONTH_DAYS + 1)
}

/// Age (ticks) as days, for HUD. Internal age stays in ticks; display only.
pub fn fmt_age_days(ticks: u32) -> String {
    format!("{:.1}d", ticks as f32 / DAY_TICKS as f32)
}

/// Wall-clock HH:MM within the local day (tick 0 = 00:00 midnight). Display only.
pub fn fmt_clock(tick: u32) -> String {
    let mins = ((tick % DAY_TICKS) as f32 / DAY_TICKS as f32 * 24.0 * 60.0) as u32;
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

/// Season from the sub-solar latitude (sun_dir.y = sin of solar declination; daily spin leaves y unchanged).
/// Northern-hemisphere framing. Spring/autumn split by whether the sun is climbing or falling in declination.
pub fn season_label(tick: u32) -> &'static str {
    let decl = sun_dir(tick).y;
    let rising = sun_dir(tick + DAY_TICKS * 5).y > decl;
    if decl > 0.2 {
        "N summer"
    } else if decl < -0.2 {
        "N winter"
    } else if rising {
        "N spring"
    } else {
        "N autumn"
    }
}

/// Scan forward for the next solar + lunar eclipse, as days from `tick` (-1 if none within the horizon).
/// Conditions mirror viz::day_night_lighting: solar when sun/moon discs overlap (sep < 2*angular radius),
/// lunar when the moon enters the planet's umbra (anti-sun within planet angular radius). Geocentric
/// separation is invariant under the daily spin, so this depends only on the orrery's monthly drift. Coarse
/// step is fine: the alignment windows (~5 deg solar, ~10 deg lunar) are far wider than per-step moon motion.
pub fn next_eclipse(tick: u32) -> (f32, f32) {
    let solar_thr = 2.0 * MOON_R / MOON_ORBIT; // overlapping discs
    let lunar_thr = PLANET_R / MOON_ORBIT; // planet umbra half-angle seen from moon
    let horizon = 400 * DAY_TICKS;
    let step = 120u32;
    let (mut solar, mut lunar) = (-1.0f32, -1.0f32);
    let mut tk = tick + step;
    while tk < tick + horizon && (solar < 0.0 || lunar < 0.0) {
        let sd = sun_dir(tk);
        let md = moon_dir(tk);
        if solar < 0.0 && sd.dot(md).clamp(-1.0, 1.0).acos() < solar_thr {
            solar = (tk - tick) as f32 / DAY_TICKS as f32;
        }
        if lunar < 0.0 && (-sd).dot(md).clamp(-1.0, 1.0).acos() < lunar_thr {
            lunar = (tk - tick) as f32 / DAY_TICKS as f32;
        }
        tk += step;
    }
    (solar, lunar)
}

/// Local daylight 0..1 at surface dir `d` for tick: how much point faces sun.
pub fn daylight_at(d: Vec3, tick: u32) -> f32 {
    d.dot(sun_dir(tick)).clamp(0.0, 1.0)
}

// ---------- clouds + cloud-driven rain ----------

const CLOUD_FREQ: f32 = 3.0;     // cloud patch size (higher = smaller, more patches)
// Wind LATITUDE-BANDED, not uniform spin: see `zonal_wind`. WIND_PEAK = strongest band (rad/tick); bands
// shear past each other -> clouds at different latitudes drift at different speeds + dirs.
const WIND_PEAK: f32 = 0.0011;
// Slow secondary morph: nudge noise through 3rd axis over time so cloud PATTERN evolves (forms + dissolves)
// not rigidly circling. ~1 unit fbm space per ~25k ticks = gentle.
const CLOUD_MORPH: f32 = 0.00004;
// Climate-drift field (geological): SLOW regional wet/dry anomaly climate memory chases. Same
// rotate-sample-point trick as clouds but ~500x slower so wet belts migrate over years, not days.
const CLIMATE_DRIFT: f32 = 0.45;     // anomaly amplitude (0 = static climate, 1 = strong wet-belt migration)
const CLIMATE_SPEED: f32 = 0.0000019; // rad/tick anomaly rotates: full sweep TAU/speed ~3.3M ticks ~1380 days
const CLIMATE_FREQ: f32 = 1.3;       // anomaly patch size (low = continent-scale wet/dry zones, not speckle)
const CLOUD_COVER: f32 = 0.55;   // noise threshold: above = cloudy (higher = sparser clouds)
pub const CLOUD_RAIN_MIN: f32 = 0.45; // cloud cover above which rain can fall (thick-ish cloud)
// Rain-mask threshold on second fbm field: rain falls only where mask exceeds this. fbm3 clusters mid-range
// + rarely tops ~0.85, so old `1.0 - 0.10 = 0.90` gate was UNREACHABLE -> rain never fell. 0.60 sits in
// field's upper band -> scattered, drifting rain cells under thicker clouds.
pub const RAIN_MASK_MIN: f32 = 0.60;

/// Signed zonal (east-west) wind at dir `d`, rad/tick. Earth-like 3-band flow: gentle equatorial easterlies
/// (-), strong mid-latitude westerlies (+), tapering toward poles. + drifts west->east, - east->west.
/// Rotation about spin axis preserves latitude, so each cloud keeps its band -> bands shear past each other,
/// clouds move at visibly different speeds + dirs by latitude.
pub fn zonal_wind(d: Vec3) -> f32 {
    let (_lon, lat) = dir_to_lonlat(d);
    // 0.35 - 0.65*cos(3*lat): equator ~ -0.30 (mild easterly), ~30deg ~ +0.35, ~60deg ~ +1.0 (jet), pole
    // ~ +0.35. Most populated mid-latitudes drift west->east; only deep tropics reverse.
    WIND_PEAK * (0.35 - 0.65 * (3.0 * lat).cos())
}

/// Cloud cover 0..1 at surface dir `d` and `tick`: scrolling 3D-fBm field. 0 = clear, 1 = thick overcast.
/// Drift latitude-banded (`zonal_wind`) so clouds move at different speeds per band; slow morph evolves
/// pattern so cloud systems form + dissolve, not just circle. Deterministic -> headless + render agree.
/// Drives local shade (visual + plant light); ONLY source of rain (see `rain_at`).
pub fn cloud_cover(d: Vec3, tick: u32) -> f32 {
    // rotate sample point about spin axis by this latitude's wind so pattern drifts per band
    let a = tick as f32 * zonal_wind(d);
    let (s, c) = (a.sin(), a.cos());
    let rot = Vec3::new(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    // walk 3rd noise axis slowly -> cloud pattern slowly reshapes (form + dissolve)
    let morph = tick as f32 * CLOUD_MORPH;
    let n = fbm3(rot * CLOUD_FREQ + Vec3::new(31.7, morph, 7.0));
    ((n - CLOUD_COVER) / (1.0 - CLOUD_COVER)).clamp(0.0, 1.0)
}

/// Rain intensity 0..1 at `d`,`tick`. Rain comes ONLY from clouds: rains solely where cloud cover thick
/// (> CLOUD_RAIN_MIN), and within that only where separate slow-drifting mask field high (> RAIN_MASK_MIN)
/// -> scattered, moving cells under thicker clouds, not everywhere.
pub fn rain_at(d: Vec3, tick: u32) -> f32 {
    let cover = cloud_cover(d, tick);
    if cover <= CLOUD_RAIN_MIN {
        return 0.0;
    }
    let a = tick as f32 * zonal_wind(d) * 0.7; // rain bands drift with this band's wind, touch slower
    let (s, c) = (a.sin(), a.cos());
    let rot = Vec3::new(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    let mask = fbm3(rot * (CLOUD_FREQ * 1.7) + Vec3::splat(71.2));
    if mask < RAIN_MASK_MIN {
        return 0.0; // cloudy but not raining here
    }
    cover // rain as heavy as cloud is thick
}

/// Long-run climate moisture target 0..1 at `d` for `tick`: value this cell's slow climate memory drifts
/// toward. Static moisture baseline + VERY slowly rotating regional anomaly (continent-scale wet vs dry
/// patches) so some regions stay rainier + wet/dry belts migrate over years -> deserts + rainforests form,
/// persist, move. Pure + deterministic -> headless + render agree. Slow `Climate` grid (sim.rs)
/// low-pass-filters this; sampling directly = instantaneous target, not the climate.
pub fn climate_target(d: Vec3, tick: u32) -> f32 {
    let base = moisture(d);
    // rotate sample point slowly about spin axis -> anomaly pattern migrates across surface
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

    // Diagnostic (run: cargo test report_geography -- --nocapture): reports ocean/deep/mountain coverage to
    // tune SEA_LEVEL to ~50% ocean. Median elevation = SEA_LEVEL giving exactly 50% ocean.
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
        // great-circle step of length L moves surface point ~L along surface
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
    fn magnetic_field_cues() {
        // latitude cue: +1 at mag pole, ~0 on mag equator (dir perpendicular to pole)
        assert!((mag_latitude(mag_pole_dir()) - 1.0).abs() < 1e-5, "mag lat at pole should be ~1");
        assert!(mag_latitude(Vec3::Z).abs() < 1e-5, "mag lat on the magnetic equator should be ~0");
        // intensity: stronger (more vertical) at pole than equator
        assert!(mag_intensity(mag_pole_dir()) > mag_intensity(Vec3::Z) + 0.5);
        // compass: finite + bounded everywhere; nonzero declination somewhere (tilted dipole)
        let mut max_dec = 0.0f32;
        for &(lon, lat) in &[(0.0f32, 0.3f32), (1.2, -0.5), (-2.0, 0.7), (2.5, 0.1)] {
            let d = lonlat_to_pos(lon, lat, 0.0).normalize();
            let bear = mag_north_bearing(d);
            assert!(bear.is_finite() && bear.abs() <= std::f32::consts::PI + 1e-4, "bearing out of range: {bear}");
            max_dec = max_dec.max(bear.abs());
        }
        assert!(max_dec > 1e-3, "tilted dipole should produce a nonzero declination somewhere");
    }

    #[test]
    fn poles_colder_than_equator() {
        let eq = lonlat_to_pos(0.0, 0.0, 0.0).normalize();
        let pole = lonlat_to_pos(0.0, 1.55, 0.0).normalize();
        assert!(base_temperature(eq) > base_temperature(pole));
    }

    #[test]
    fn clouds_vary_and_drift() {
        // cloud field spans clear..cloudy across globe, fixed point changes over time (drift)
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
        // at any tick, some longitude faces sun (day) + antipode does not (night)
        let s = sun_dir(600);
        let day = (s * PLANET_R).normalize();
        let night = -day;
        assert!(daylight_at(day, 600) > 0.5);
        assert!(daylight_at(night, 600) < 0.01);
    }

    #[test]
    fn eclipses_occur() {
        // sun + moon discs share angular radius `ar`; a solar eclipse needs their sky dirs within 2*ar.
        // Scan a synodic month: the moon laps the sun, so min separation must reach ~0 (new-moon alignment).
        let ar = MOON_R / MOON_ORBIT;
        let mut min_sep = f32::MAX;
        for t in (0..90_000).step_by(150) {
            let sep = sun_dir(t).dot(moon_dir(t)).clamp(-1.0, 1.0).acos();
            min_sep = min_sep.min(sep);
        }
        assert!(min_sep < 2.0 * ar, "moon should occlude sun within a month (min_sep {min_sep}, 2ar {})", 2.0 * ar);
    }

    #[test]
    fn next_eclipse_predicts_within_a_month() {
        // both a solar and a lunar alignment must be found, and within ~a synodic month (monthly cadence).
        let (s, l) = next_eclipse(0);
        assert!(s > 0.0 && s < 45.0, "next solar {s}d");
        assert!(l > 0.0 && l < 45.0, "next lunar {l}d");
        // clock + season helpers produce well-formed strings
        assert_eq!(fmt_clock(0), "00:00");
        assert_eq!(fmt_clock(DAY_TICKS / 2), "12:00");
        assert!(season_label(0).starts_with('N'));
    }
}
