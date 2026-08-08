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
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::{FRAC_PI_2, PI};

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

/// Baked vertical sky strip for a sky-dome mesh: `size` rows tall, 4 px wide (gradient is vertical only;
/// 4 px keeps rows tiny while staying wider than a 1-px edge case in any sampler).
///
/// UV CONTRACT: matches a Bevy UV-sphere directly. v=0 (top row) = zenith, v=0.5 = horizon, v=1 = nadir. A
/// hemisphere dome must remap its v into 0..0.5.
///
/// Sun-relative terms are deliberately NOT baked (sun frozen at ~35 deg, halo suppressed by sampling 180 deg
/// away): time of day belongs on the material's base_color tint or on a per-frame rebuild, not in the strip.
pub fn sky_gradient_texture(size: usize) -> Image {
    const W: usize = 4;
    let h = size.max(4);
    let mut data = vec![0u8; W * h * 4];
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let e = (0.5 - v) * PI; // v=0 -> +PI/2 zenith, v=0.5 -> 0 horizon, v=1 -> -PI/2 nadir
        let c = sky_color_at(e, 0.61, PI);
        for x in 0..W {
            let i = (y * W + x) * 4;
            data[i] = (c[0] * 255.0) as u8;
            data[i + 1] = (c[1] * 255.0) as u8;
            data[i + 2] = (c[2] * 255.0) as u8;
            data[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d { width: W as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Cloud puff color + opacity. `altitude01` = 0 at the puff base, 1 at its crown; `sun_dot` = dot(puff or
/// lobe normal, sun dir), -1 anti-sun .. 1 sub-solar. Returns sRGB RGBA.
///
/// Alpha is a PROFILE, not a final value: caller multiplies by growth/cover (see `viz::update_clouds`, which
/// caps at ~0.55).
pub fn cloud_puff_shading(altitude01: f32, sun_dot: f32) -> [f32; 4] {
    let a01 = altitude01.clamp(0.0, 1.0);
    let s = sun_dot.clamp(-1.0, 1.0);

    // Terminator sits past the geometric one (-0.35, not 0): clouds forward-scatter light around into their
    // own shadow side, so a hard N.L cut reads as a lit rock, not a cloud.
    let lit = smoothstep(-0.35, 0.45, s);
    // Grazing band around s=0 -> the golden rim on dawn/dusk clouds and on every puff's limb.
    let graze = (1.0 - (s.abs() / 0.45).min(1.0)).powf(1.5);

    const SUN_WHITE: [f32; 3] = [1.0, 0.985, 0.955];
    const WARM: [f32; 3] = [1.0, 0.88, 0.72];
    // Blue-grey, not neutral grey: a shaded cloud is lit by the sky dome, and neutral grey is exactly what
    // made the old puffs read as dirty soap bubbles.
    const SHADOW: [f32; 3] = [0.42, 0.47, 0.60];

    let mut c = mix3(SHADOW, SUN_WHITE, lit);
    // 0.7 not 0.45: the blue-grey base is cold enough that a weaker warm mix leaves the grazing rim reading
    // neutral instead of golden.
    c = mix3(c, WARM, graze * 0.7 * lit);

    // Optical depth: the base sees light only after it crossed the whole cloud -> darker AND bluer. Crown
    // keeps full value. Two terms because a plain multiply desaturates toward black instead of toward sky.
    let depth = 1.0 - a01;
    c = mix3(c, SHADOW, 0.35 * depth);
    let dim = 1.0 - 0.18 * depth;
    let c = [c[0] * dim, c[1] * dim, c[2] * dim];

    // Solid base, wispy crown: the silhouette edge is where a cloud stops looking like a balloon.
    let alpha = 0.95 - 0.50 * a01;

    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), alpha.clamp(0.0, 1.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn gradient_texture_is_well_formed_and_darkens_downward() {
        let img = sky_gradient_texture(128);
        let sz = img.texture_descriptor.size;
        assert_eq!((sz.width, sz.height), (4, 128));
        assert_eq!(img.data.as_ref().map(|d| d.len()), Some(4 * 128 * 4));
        let d = img.data.as_ref().unwrap();
        let row = |y: usize| {
            let i = y * 4 * 4;
            (d[i] as u32, d[i + 1] as u32, d[i + 2] as u32)
        };
        let zen = row(0);
        let hor = row(63); // v ~ 0.5
        let nadir = row(127);
        assert!(hor.0 > zen.0, "horizon row should be paler than zenith");
        assert!(nadir.2 < hor.2, "below-horizon rows should darken");
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
