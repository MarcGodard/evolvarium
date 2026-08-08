//! Procedural sky + cloud shading. Pure functions, no ECS, no per-frame state: a baked dome gradient
//! texture, a per-direction sky color, and a per-puff cloud color. Complements `viz::update_sky` (which only
//! paints ONE flat ClearColor, hence the slab sky with no horizon) and `viz::update_clouds` (grey soap
//! bubbles).
//!
//! COLOR SPACE: every output is sRGB display 0..1, the same space `viz::update_sky` stops already live in.
//! Feed straight into `Color::srgb`/`Color::srgba`, or `*255` into `Rgba8UnormSrgb` bytes. No linear
//! conversion at the call site.
//!
//! ANGLES: radians. elevation 0 = horizon, PI/2 = zenith, negative = below horizon.
#![allow(dead_code)]
use bevy::asset::RenderAssetUsages;
use std::f32::consts::FRAC_PI_2;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn gauss(x: f32, sigma: f32) -> f32 {
    let k = x / sigma;
    (-(k * k)).exp()
}

// ---------- palette ----------
// Kept as named consts so a tuning pass edits one block instead of hunting literals inside the mixer.

const ZENITH_DAY: [f32; 3] = [0.16, 0.38, 0.86]; // deep rayleigh blue overhead
const HORIZON_DAY: [f32; 3] = [0.66, 0.78, 0.90]; // long air path -> desaturated pale blue-white
const ZENITH_DUSK: [f32; 3] = [0.10, 0.16, 0.42]; // sun grazing: overhead stays deep, horizon does the work
const HORIZON_GOLD: [f32; 3] = [0.98, 0.55, 0.28];
const ZENITH_NIGHT: [f32; 3] = [0.015, 0.025, 0.065];
const HORIZON_NIGHT: [f32; 3] = [0.055, 0.075, 0.14]; // never equal to zenith: a visible night horizon line
                                                      // is what stops terrain ending in a void

/// Sky color looking along `elevation_angle` (rad above horizon; PI/2 = zenith, negative = below horizon)
/// with the sun at `sun_elevation` (rad, negative = set) and `sun_angle_delta` rad of angular separation
/// between the view direction and the sun. sRGB 0..1.
///
/// Model is a fit, not physics: air-mass proxy drives zenith->horizon desaturation, a narrow gaussian adds
/// the horizon haze band, `golden` warms the horizon while the sun grazes, and a second gaussian on
/// `sun_angle_delta` paints the sun's own halo (wide + amber at sunset, tight + white at noon).
pub fn sky_color_at(elevation_angle: f32, sun_elevation: f32, sun_angle_delta: f32) -> [f32; 3] {
    let e = elevation_angle.clamp(-FRAC_PI_2, FRAC_PI_2);
    let up = e.max(0.0);

    // Air-mass proxy 0 zenith..1 horizon. Exponent 1.7 keeps the upper ~2/3 of the dome saturated blue and
    // squeezes the pale band into the last ~30 deg; linear (1.0) washes the whole sky out.
    let m = (1.0 - up / FRAC_PI_2).powf(1.7);
    // Haze band hugging the horizon, sigma ~8 deg. This is the piece that dissolves the terrain edge.
    let haze = gauss(up, 0.14);

    // Daylight 0..1 from sun height: -0.21 rad (~-12 deg, nautical twilight) .. 0.16 rad (~9 deg, full day).
    let day = smoothstep(-0.21, 0.16, sun_elevation);
    // Golden window: on from ~-11 deg (afterglow, sun already set) through ~+3 deg, off by ~18 deg.
    let golden =
        smoothstep(-0.20, -0.02, sun_elevation) * (1.0 - smoothstep(0.05, 0.32, sun_elevation));

    let zen_lit = mix3(ZENITH_DAY, ZENITH_DUSK, golden * 0.75);
    let hor_lit = mix3(HORIZON_DAY, HORIZON_GOLD, golden * 0.85);
    let zen = mix3(ZENITH_NIGHT, zen_lit, day);
    let hor = mix3(HORIZON_NIGHT, hor_lit, day);
    let mut c = mix3(zen, hor, m);

    // Haze pushes the last degrees toward a bright grey by day; at night `day`=0 collapses this to `hor` so
    // the night horizon does not turn grey.
    let hazed = mix3(hor, [0.86, 0.88, 0.90], 0.35 * day);
    c = mix3(c, hazed, 0.45 * haze);

    // Sun halo. `low` widens + reddens it: at sunset the line of sight to the sun runs through ~38 air
    // masses, so the glow smears across half the sky.
    let low = 1.0 - smoothstep(0.0, 0.50, sun_elevation.max(0.0));
    let sigma = 0.20 + 0.70 * low; // ~11 deg at noon .. ~52 deg at sunset
    let g = gauss(sun_angle_delta.abs(), sigma);
    let glow_col = mix3([1.0, 0.96, 0.86], [1.0, 0.62, 0.30], low);
    let glow = (g * (0.22 + 0.55 * low) * day).clamp(0.0, 0.85);
    c = mix3(c, glow_col, glow);

    // Below the horizon keep falling into a dark ground haze, so a caller mapping this over a FULL sphere
    // gets no bright seam under the terrain. `below` reaches 1 by ~-14 deg.
    let below = smoothstep(0.0, -0.25, e);
    let ground = [c[0] * 0.30, c[1] * 0.30, c[2] * 0.34];
    c = mix3(c, ground, below);

    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]
}

/// Cloud puff color + opacity. `altitude01` = 0 at the puff base, 1 at its crown; `sun_dot` = dot(puff or
/// lobe normal, sun dir), -1 anti-sun .. 1 sub-solar. Returns sRGB RGBA.
///
/// Alpha is a PROFILE, not a final value: caller multiplies by growth/cover (see `viz::update_clouds`, which
/// caps at ~0.55).
pub fn cloud_puff_shading(altitude01: f32, sun_dot: f32) -> [f32; 4] {
    let c = cloud_sun_tint(sun_dot);
    let d = cloud_depth_tint(altitude01);
    [
        (c[0] * d[0]).clamp(0.0, 1.0),
        (c[1] * d[1]).clamp(0.0, 1.0),
        (c[2] * d[2]).clamp(0.0, 1.0),
        d[3],
    ]
}

/// Blue-grey, not neutral grey: a shaded cloud is lit by the sky dome, and neutral grey is exactly what
/// makes a puff read as a dirty soap bubble.
const CLOUD_SHADOW: [f32; 3] = [0.42, 0.47, 0.60];

/// Whole-puff sun response. Split from the altitude profile so a renderer can drive this per frame while
/// the profile bakes into static vertex colours (see `viz::update_clouds` / `viz::cloud_puff_mesh`).
pub fn cloud_sun_tint(sun_dot: f32) -> [f32; 3] {
    let s = sun_dot.clamp(-1.0, 1.0);
    // Terminator sits past the geometric one (-0.35, not 0): clouds forward-scatter light around into their
    // own shadow side, so a hard N.L cut reads as a lit rock, not a cloud.
    let lit = smoothstep(-0.35, 0.45, s);
    // Grazing band around s=0 -> the golden rim on dawn/dusk clouds and on every puff's limb.
    let graze = (1.0 - (s.abs() / 0.45).min(1.0)).powf(1.5);

    const SUN_WHITE: [f32; 3] = [1.0, 0.985, 0.955];
    const WARM: [f32; 3] = [1.0, 0.88, 0.72];

    let c = mix3(CLOUD_SHADOW, SUN_WHITE, lit);
    // 0.7 not 0.45: the blue-grey base is cold enough that a weaker warm mix leaves the grazing rim reading
    // neutral instead of golden.
    mix3(c, WARM, graze * 0.7 * lit)
}

/// Optical-depth profile down a puff, as an RGB MULTIPLIER plus an alpha. `altitude01` = 0 at the base, 1 at
/// the crown. Multiplier form (rather than a mix toward shadow) is what lets it live in vertex colours; the
/// blue-preserving ratio below keeps the base going bluer instead of merely darker.
pub fn cloud_depth_tint(altitude01: f32) -> [f32; 4] {
    let depth = 1.0 - altitude01.clamp(0.0, 1.0);
    // 0.35 toward CLOUD_SHADOW expressed against a ~white lit cloud, times a flat 0.18 dim.
    let mix = 0.35 * depth;
    let dim = 1.0 - 0.18 * depth;
    let ratio = [0.420, 0.477, 0.628]; // CLOUD_SHADOW / SUN_WHITE
    // Solid base, slightly thinner crown. Narrow range on purpose: these lobes OVERLAP, and anything much
    // below opaque makes every intersection ring visible, so the puff reads as a stack of circles.
    let alpha = 1.0 - 0.22 * altitude01.clamp(0.0, 1.0);
    [
        (1.0 - mix + mix * ratio[0]) * dim,
        (1.0 - mix + mix * ratio[1]) * dim,
        (1.0 - mix + mix * ratio[2]) * dim,
        alpha,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn horizon_is_paler_than_zenith_by_day() {
        let noon = 1.2_f32;
        let zen = sky_color_at(FRAC_PI_2, noon, FRAC_PI_2);
        let hor = sky_color_at(0.0, noon, FRAC_PI_2);
        // pale = higher red + smaller blue-red spread
        assert!(hor[0] > zen[0], "horizon should be paler: {hor:?} vs {zen:?}");
        assert!(hor[2] - hor[0] < zen[2] - zen[0], "horizon should be less saturated");
    }

    #[test]
    fn sunset_horizon_warms_and_night_is_dark() {
        let set = sky_color_at(0.02, 0.01, 0.05); // looking at the setting sun
        assert!(set[0] > set[2], "sunset horizon must be red-dominant: {set:?}");
        let night = sky_color_at(FRAC_PI_2, -0.6, PI);
        assert!(night[0] + night[1] + night[2] < 0.25, "night zenith too bright: {night:?}");
        assert!(night[2] > night[0], "night sky stays blue-ish: {night:?}");
    }

    #[test]
    fn sun_halo_brightens_toward_the_sun() {
        let near = sky_color_at(0.1, 0.05, 0.0);
        let far = sky_color_at(0.1, 0.05, PI);
        let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
        assert!(sum(near) > sum(far) + 0.15, "halo missing: {near:?} vs {far:?}");
    }

    #[test]
    fn below_horizon_darkens_monotonically() {
        let a = sky_color_at(-0.02, 0.8, PI);
        let b = sky_color_at(-0.4, 0.8, PI);
        let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
        assert!(sum(b) < sum(a), "under-horizon must fade: {b:?} vs {a:?}");
    }

    #[test]
    fn cloud_lit_side_bright_shadow_side_blue_base_dense() {
        let lit_top = cloud_puff_shading(1.0, 0.9);
        let shade_top = cloud_puff_shading(1.0, -0.9);
        assert!(lit_top[0] > shade_top[0] + 0.3, "sunlit side must be far brighter");
        assert!(shade_top[2] > shade_top[0], "shadow side must be blue-grey: {shade_top:?}");
        let base = cloud_puff_shading(0.0, 0.9);
        assert!(base[0] < lit_top[0], "base must be darker than crown");
        assert!(base[3] > lit_top[3], "base must be denser than crown");
        // grazing sun tints warm, not neutral
        let graze = cloud_puff_shading(1.0, 0.05);
        assert!(graze[0] > graze[2], "grazing puff should be warm: {graze:?}");
    }
}

// ---------- sky dome ----------

use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::Mesh;

/// Inward-facing UV sphere for the sky. Colours are per-VERTEX rather than a baked ramp because a 1D
/// gradient cannot carry sun-direction warmth: the glow has to sit where the sun actually is, which needs an
/// angle per vertex. `update_sky_dome` rewrites ATTRIBUTE_COLOR as the sun moves.
/// Local +Y is zenith; the dome is oriented to the walker's surface normal, not world up.
pub fn sky_dome_mesh(rings: usize, sectors: usize, radius: f32) -> Mesh {
    let (rings, sectors) = (rings.max(3), sectors.max(3));
    let mut positions = Vec::with_capacity((rings + 1) * (sectors + 1));
    let mut normals = Vec::with_capacity((rings + 1) * (sectors + 1));
    let mut colors = Vec::with_capacity((rings + 1) * (sectors + 1));
    for j in 0..=rings {
        // full sphere, not a hemisphere: below-horizon verts get the ground-haze end of the ramp so the
        // terrain never meets an unpainted edge at the silhouette
        let phi = std::f32::consts::PI * j as f32 / rings as f32; // 0 = zenith
        for i in 0..=sectors {
            let th = std::f32::consts::TAU * i as f32 / sectors as f32;
            let d = bevy::math::Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
            positions.push([d.x * radius, d.y * radius, d.z * radius]);
            normals.push([-d.x, -d.y, -d.z]); // inward: the viewer is inside
            colors.push([0.5, 0.6, 0.9, 1.0]);
        }
    }
    let stride = (sectors + 1) as u32;
    let mut indices = Vec::with_capacity(rings * sectors * 6);
    for j in 0..rings as u32 {
        for i in 0..sectors as u32 {
            let (a, b) = (j * stride + i, j * stride + i + 1);
            let (c, d) = (a + stride, b + stride);
            indices.extend_from_slice(&[a, b, c, b, d, c]); // wound for inside viewing
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Repaint dome vertices for a sun at `sun_local` (unit, in the dome's local frame where +Y is zenith).
/// Cheap enough to run on a throttle: a 24x48 dome is ~1200 verts.
pub fn paint_sky_dome(mesh: &mut Mesh, sun_local: bevy::math::Vec3) {
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return;
    };
    let sun_elev = sun_local.y.clamp(-1.0, 1.0).asin();
    let colors: Vec<[f32; 4]> = pos
        .iter()
        .map(|p| {
            let d = bevy::math::Vec3::new(p[0], p[1], p[2]).normalize_or_zero();
            let elev = d.y.clamp(-1.0, 1.0).asin();
            let delta = d.dot(sun_local).clamp(-1.0, 1.0).acos();
            let c = sky_color_at(elev, sun_elev, delta);
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}

#[cfg(test)]
mod dome_tests {
    use super::*;

    #[test]
    fn zenith_is_bluer_than_horizon_at_midday() {
        // The dome only ever shows its near-horizon band in a normal walk view, which reads pale and grey.
        // That is correct sky, not a washed-out bug, and this pins the distinction: zenith [0.16,0.38,0.86],
        // horizon [0.67,0.78,0.90]. Checked numerically because --cap-pitch does not actually change the
        // capture vantage, so the zenith cannot be inspected by screenshot.
        let sun_elev = 1.2_f32; // high sun
        let z = sky_color_at(std::f32::consts::FRAC_PI_2, sun_elev, 1.0);
        let h = sky_color_at(0.05, sun_elev, 1.5);
        // blue dominance = b - r. Real sky: strong at zenith, weak (pale) at horizon.
        let zb = z[2] - z[0];
        let hb = h[2] - h[0];
        assert!(zb > hb, "zenith must be bluer than horizon: {zb} vs {hb}");
        assert!(zb > 0.15, "zenith too desaturated to read as sky: blue-red = {zb}");
    }
}
