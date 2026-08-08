//! Procedural ground DETAIL for the planet surface: two tiling textures (albedo mottle + normal relief) and
//! a per-vertex tint. All three MODULATE `sphere::biome_color_with_moisture`, they never replace it: the
//! textures are multipliers centered near 1.0 and the tint is a per-channel factor, so the biome/climate
//! palette stays the single source of ground hue and this file only breaks up its flatness.
//!
//! Two noise sources on purpose:
//! - `sphere::fbm3` for everything at world scale (already seamless on the sphere) and, via `tile_fbm`, for
//!   the texture's clump/mid octaves.
//! - a local wrapped-lattice `pnoise2` for the finest texture grain, because exact periodicity at ~4 px
//!   costs one hash there and 128 hashes through `tile_fbm`.
#![allow(dead_code)]
use crate::sphere;
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Tile repeats around the equator. 128 tiles over 2*PI*80 m = ~3.9 m per tile (grass-clump scale on foot).
// Repeat count is a VISIBILITY constraint, not a detail one. One tile stamped 128x64 times put every
// feature of that tile on a regular lattice, and the eye integrates the repeats into lines: the planet read
// as a grid from orbit even with the aliasing fixed. Mipmaps cure the moire, NOT the periodicity, so the
// repeat itself has to come down. 32x16 puts a tile at ~16 m, sparse enough that the lattice stops being
// legible, and the per-vertex ground_tint carries the large-scale variation the finer repeat used to fake.
pub const GROUND_TILES_U: f32 = 32.0;
/// Tile repeats pole to pole. Half of U because a meridian is half a great circle -> square tiles at equator.
pub const GROUND_TILES_V: f32 = 16.0;

// ---------- noise helpers ----------

// Periodic 2D value noise on a wrapped lattice, exact period 1 in both u and v. `cells` must be a positive
// integer or the wrap seam reappears.
fn pnoise2(u: f32, v: f32, cells: f32) -> f32 {
    let n = (cells as i32).max(1);
    let (x, y) = (u * cells, v * cells);
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (i0, j0) = ((xi as i32).rem_euclid(n), (yi as i32).rem_euclid(n));
    let (i1, j1) = ((xi as i32 + 1).rem_euclid(n), (yi as i32 + 1).rem_euclid(n));
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (su, sv) = (s(fx), s(fy));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let a = lerp(hash2(i0, j0), hash2(i1, j0), su);
    let b = lerp(hash2(i0, j1), hash2(i1, j1), su);
    lerp(a, b, sv)
}

fn hash2(i: i32, j: i32) -> f32 {
    let mut h = (i.wrapping_mul(374_761_393)).wrapping_add(j.wrapping_mul(668_265_263)) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

// Seamlessly tiling fBm from the NON-tiling `sphere::fbm3`: cross-blend the four unit-shifted copies. The
// blend weights make f(0,v) == f(1,v) and f(u,0) == f(u,1) exactly. Dividing the deviation by
// sqrt(sum w^2) restores the variance the blend eats in the tile interior (weights there are 0.25 each, so
// contrast would otherwise halve mid-tile and the texture would read as a soft vignette grid).
// `cells` = lattice cells across one tile; keep it >= 3 so the four shifted samples are decorrelated.
fn tile_fbm(u: f32, v: f32, cells: f32, z: f32) -> f32 {
    let s = |x: f32, y: f32| sphere::fbm3(Vec3::new(x * cells, y * cells, z));
    let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
    let f = [s(u, v), s(u - 1.0, v), s(u, v - 1.0), s(u - 1.0, v - 1.0)];
    let mixed: f32 = w.iter().zip(f).map(|(w, f)| w * f).sum();
    let norm = w.iter().map(|w| w * w).sum::<f32>().sqrt().max(1e-4);
    (0.5 + (mixed - 0.5) / norm).clamp(0.0, 1.0)
}

// Contrast stretch about 0.5. value-noise fBm piles up near its mean (4 averaged octaves), so raw fbm3
// deviates only ~+/-0.12 and reads as a flat wash; k pulls that back out to a usable spread.
fn stretch(f: f32, k: f32) -> f32 {
    (0.5 + (f - 0.5) * k).clamp(0.0, 1.0)
}

fn smooth01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

// Linear 0..1 -> sRGB 0..1. The albedo image is Rgba8UnormSrgb, so the shader decodes it; without this
// encode the stored multiplier would come back as roughly its 2.2 power (mottling ~2x too contrasty, ground
// far too dark).
fn lin_to_srgb(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

// The three detail octaves at one texel. Shared by albedo + normal so the bumps sit under the mottling
// instead of drifting against it.
fn fields(u: f32, v: f32, grain_cells: f32) -> (f32, f32, f32) {
    let clump = stretch(tile_fbm(u, v, 3.0, 11.3), 2.8); // patch scale: a few clumps per ~4 m tile
    let mid = stretch(tile_fbm(u, v, 11.0, 41.7), 2.8); // tussock scale
    let grain = pnoise2(u, v, grain_cells); // ~4 px features, kills the plastic sheen at close range
    (clump, mid, grain)
}

// Grain lattice: size/4 -> ~4 px per cell. Below that it aliases into shimmer when the ground is oblique.
fn grain_cells(size: usize) -> f32 {
    ((size / 4).max(4)) as f32
}

// ---------- textures ----------

/// Seamlessly tiling ground albedo MULTIPLIER (soil/grass mottling, 3 octaves, hue shifts as well as
/// value). Wire as `base_color_texture` with a repeating sampler; mean is ~0.85 linear so it darkens the
/// surface slightly on purpose (the untextured ground reads blown out). 256 is enough at ~4 m per tile.
pub fn ground_detail_texture(size: usize) -> Image {
    let size = size.max(8);
    let gc = grain_cells(size);
    // Hue anchors, all near-neutral so the biome color still dominates: lush turf leans blue-green, dry
    // sward leans straw-yellow, bare patches lean red-brown.
    const LUSH: [f32; 3] = [0.88, 1.03, 0.93];
    const STRAW: [f32; 3] = [1.06, 0.99, 0.80];
    const EARTH: [f32; 3] = [1.05, 0.90, 0.75];
    let mut data = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
            let (clump, mid, grain) = fields(u, v, gc);
            // Cap at 0.94 so the brightest hue anchor (1.06) still lands under 1.0 and survives 8-bit.
            let shade = (0.85 + 0.26 * (clump - 0.5) + 0.17 * (mid - 0.5) + 0.08 * (grain - 0.5)).clamp(0.55, 0.94);
            let dry = smooth01((clump - 0.40) / 0.40);
            let bare = smooth01((mid - 0.66) / 0.24);
            let hue = lerp3(lerp3(LUSH, STRAW, dry), EARTH, bare * 0.55);
            let i = (y * size + x) * 4;
            for c in 0..3 {
                data[i + c] = (lin_to_srgb(hue[c] * shade) * 255.0) as u8;
            }
            data[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Matching tiling tangent-space normal map (fine relief under `ground_detail_texture`). Format is LINEAR
/// Rgba8Unorm: an sRGB format would bend the encoded vectors and tilt every normal. Green is up (OpenGL
/// convention); negate the G channel if the relief reads inverted.
pub fn ground_normal_texture(size: usize) -> Image {
    let size = size.max(8);
    let gc = grain_cells(size);
    // Bump height, grain-heavy: the clump octave is an albedo patch, it carries almost no relief.
    let h: Vec<f32> = (0..size * size)
        .map(|k| {
            let (x, y) = (k % size, k / size);
            let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
            let (clump, mid, grain) = fields(u, v, gc);
            0.18 * clump + 0.47 * mid + 0.35 * grain
        })
        .collect();
    // Slope gain. Higher = harsher; 2.6 keeps the surface reading as soil under a low sun without the
    // stucco look that shows up past ~5.
    const RELIEF: f32 = 2.6;
    let at = |x: i32, y: i32| {
        let n = size as i32;
        h[(y.rem_euclid(n) * n + x.rem_euclid(n)) as usize]
    };
    let mut data = vec![0u8; size * size * 4];
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            // Central differences wrap through rem_euclid -> the normal map tiles as exactly as the height.
            let du = (at(x + 1, y) - at(x - 1, y)) * 0.5;
            let dv = (at(x, y + 1) - at(x, y - 1)) * 0.5;
            let n = Vec3::new(-du * RELIEF, -dv * RELIEF, 1.0).normalize();
            let i = ((y * size as i32 + x) * 4) as usize;
            data[i] = ((n.x * 0.5 + 0.5) * 255.0) as u8;
            data[i + 1] = ((n.y * 0.5 + 0.5) * 255.0) as u8;
            data[i + 2] = ((n.z * 0.5 + 0.5) * 255.0) as u8;
            data[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

// ---------- per-vertex tint ----------

/// Per-channel MULTIPLIER (mean ~1.0) for the globe's vertex color, applied on top of
/// `sphere::biome_color_with_moisture`. Breaks up the large uniform biome plates at 40 m / 9 m / 3 m scales
/// and rotates HUE, not just value: dry ground swings yellow-brown, damp ground swings blue-green.
///
/// `d` = unit outward dir. `moisture` = effective moisture (static + rain ground water, same value fed to
/// `biome_color_with_moisture`). `temp` = `sphere::base_temperature`, `rockiness` = `sphere::rockiness`,
/// both 0..1.
pub fn ground_tint(d: Vec3, moisture: f32, temp: f32, rockiness: f32) -> [f32; 3] {
    // Frequencies are in units of 1/radian, so cell arc = PLANET_R / freq: ~30 m, ~9 m, ~3 m patches.
    // fbm3 adds 3 more octaves under each, so the fine field reaches sub-metre.
    let big = stretch(sphere::fbm3(d * 2.7 + Vec3::new(13.0, 5.0, 21.0)), 2.8);
    let mid = stretch(sphere::fbm3(d * 9.0 + Vec3::new(57.0, 31.0, 3.0)), 2.8);
    let fine = stretch(sphere::fbm3(d * 26.0 + Vec3::new(101.0, 67.0, 45.0)), 2.8);

    // Ice reads uniform in reality and mottling on snow looks like dirt, so fade out over the same
    // temp < 0.34 ramp where biome_color_with_moisture starts whitening.
    let frost = ((0.34 - temp) / 0.34).clamp(0.0, 1.0);
    let rock = rockiness.clamp(0.0, 1.0);
    // Seabed is viewed through water tint + an opaque ocean shell, so most of this would be wasted.
    let sub = if sphere::is_ocean(d) { 0.35 } else { 1.0 };

    // Stone gains value contrast (shadow + lichen between boulders) and loses hue swing (grey is grey).
    let val_amp = sub * (1.0 - 0.55 * frost) * (1.0 + 0.35 * rock);
    let value = 1.0 + val_amp * (0.13 * (big - 0.5) + 0.10 * (mid - 0.5) + 0.05 * (fine - 0.5));

    let wet = (moisture / 0.45).clamp(0.0, 1.0);
    // Lush turf is already saturated green, so pushing hue there just looks sick; dry ground carries the
    // widest real hue range (straw, ochre, bare soil).
    let swing = 0.16 * sub * (1.0 - 0.45 * wet) * (1.0 - frost) * (1.0 - 0.60 * rock);
    let h = 0.62 * big + 0.38 * mid - 0.5; // -0.5 blue-green .. +0.5 yellow-brown
    [
        (value * (1.0 + h * swing)).clamp(0.70, 1.25),
        (value * (1.0 + h * swing * 0.25)).clamp(0.70, 1.25), // green moves least -> hue rotates, luminance holds
        (value * (1.0 - h * swing * 0.85)).clamp(0.70, 1.25),
    ]
}

/// Tiling UV for a globe vertex, matching `terrain::build_globe_colored`'s lon/lat lattice. Tile counts are
/// integers so u = 0 and u = GROUND_TILES_U land on the same texel at the +/-PI seam.
pub fn ground_uv(d: Vec3) -> [f32; 2] {
    let lat = d.y.clamp(-1.0, 1.0).asin();
    let lon = d.z.atan2(d.x);
    let u = (lon + std::f32::consts::PI) / std::f32::consts::TAU;
    let v = (lat + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI;
    [u * GROUND_TILES_U, v * GROUND_TILES_V]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_fbm_wraps_exactly() {
        for k in 0..17 {
            let t = k as f32 / 16.0;
            assert!((tile_fbm(0.0, t, 3.0, 11.3) - tile_fbm(1.0, t, 3.0, 11.3)).abs() < 1e-5, "u seam at v={t}");
            assert!((tile_fbm(t, 0.0, 11.0, 41.7) - tile_fbm(t, 1.0, 11.0, 41.7)).abs() < 1e-5, "v seam at u={t}");
        }
    }

    #[test]
    fn pnoise2_wraps_exactly() {
        for k in 0..17 {
            let t = k as f32 / 16.0;
            assert!((pnoise2(0.0, t, 32.0) - pnoise2(1.0, t, 32.0)).abs() < 1e-6);
            assert!((pnoise2(t, 0.0, 32.0) - pnoise2(t, 1.0, 32.0)).abs() < 1e-6);
        }
    }

    #[test]
    fn detail_texture_has_contrast_and_hue_range() {
        let size = 64;
        let img = ground_detail_texture(size);
        assert_eq!(img.texture_descriptor.size.width, size as u32);
        let data = img.data.as_ref().unwrap();
        assert_eq!(data.len(), size * size * 4);
        let (mut lo, mut hi) = (255u8, 0u8);
        let mut hue_spread = 0u8; // max |R-B| across texels: proves color varies, not only brightness
        for px in data.chunks(4) {
            lo = lo.min(px[1]);
            hi = hi.max(px[1]);
            hue_spread = hue_spread.max(px[0].abs_diff(px[2]));
        }
        assert!(hi - lo > 40, "flat albedo, range {lo}..{hi}");
        assert!(hue_spread > 15, "no hue variation, max |R-B| = {hue_spread}");
    }

    #[test]
    fn normal_texture_is_unit_ish_and_z_up() {
        let size = 64;
        let img = ground_normal_texture(size);
        assert_eq!(img.texture_descriptor.format, TextureFormat::Rgba8Unorm, "normal map must stay linear");
        let data = img.data.as_ref().unwrap();
        let mut tilted = 0;
        for px in data.chunks(4) {
            let n = Vec3::new(
                px[0] as f32 / 255.0 * 2.0 - 1.0,
                px[1] as f32 / 255.0 * 2.0 - 1.0,
                px[2] as f32 / 255.0 * 2.0 - 1.0,
            );
            assert!((n.length() - 1.0).abs() < 0.05, "not unit: {n:?}");
            assert!(n.z > 0.0, "normal must point out of the surface");
            if n.x.abs() > 0.05 || n.y.abs() > 0.05 {
                tilted += 1;
            }
        }
        assert!(tilted > size * size / 10, "normal map is nearly flat ({tilted} tilted texels)");
    }

    #[test]
    fn tint_centers_on_one_and_shifts_hue() {
        let mut sum = [0.0f32; 3];
        let n = 400;
        let mut hue_max = 0.0f32;
        for k in 0..n {
            // spiral over the sphere so samples cover every biome
            let t = (k as f32 + 0.5) / n as f32;
            let y = 1.0 - 2.0 * t;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = k as f32 * 2.399_963;
            let d = Vec3::new(r * a.cos(), y, r * a.sin());
            let c = ground_tint(d, sphere::moisture(d), sphere::base_temperature(d), sphere::rockiness(d));
            for i in 0..3 {
                sum[i] += c[i];
            }
            hue_max = hue_max.max((c[0] - c[2]).abs());
        }
        for i in 0..3 {
            let mean = sum[i] / n as f32;
            assert!((mean - 1.0).abs() < 0.04, "channel {i} mean {mean} should stay near 1 (tint must not shift overall brightness)");
        }
        assert!(hue_max > 0.05, "tint only changes value, not hue ({hue_max})");
    }

    #[test]
    fn tint_is_calm_on_ice() {
        // polar/frozen ground: mottling must nearly vanish so snow does not read as dirt
        let d = Vec3::new(0.02, 0.999, 0.03).normalize();
        let c = ground_tint(d, sphere::moisture(d), 0.02, 0.0);
        for v in c {
            assert!((v - 1.0).abs() < 0.06, "ice tint too strong: {c:?}");
        }
    }

    #[test]
    fn ground_uv_seam_matches() {
        // the +/-PI meridian is a duplicated vertex column in build_globe_colored; both copies must land on
        // the same texel, which holds only while the tile counts are whole numbers
        assert_eq!(GROUND_TILES_U.fract(), 0.0);
        assert_eq!(GROUND_TILES_V.fract(), 0.0);
        // sample just inside each side: lon = +/-PI itself lands on whichever branch atan2 rounds to
        let pi = std::f32::consts::PI;
        let west = ground_uv(sphere::lonlat_to_pos(-pi + 1e-4, 0.0, 0.0).normalize());
        let east = ground_uv(sphere::lonlat_to_pos(pi - 1e-4, 0.0, 0.0).normalize());
        assert!(west[0] < 0.01, "west seam u {}", west[0]);
        assert!(east[0] > GROUND_TILES_U - 0.01, "east seam u {}", east[0]);
        // poles clamp to the v range ends, no wrap past them
        let np = ground_uv(Vec3::Y);
        assert!((np[1] - GROUND_TILES_V).abs() < 1e-2, "north pole v {}", np[1]);
    }
}

// ---------- mipmaps ----------

// Bevy 0.18 does NOT build mip chains. An unmipped tiling texture at GROUND_TILES_U repeats is undersampled
// the moment a texel is smaller than a pixel, and the aliasing beats against the lat/lon tile lattice into
// visible diagonal banding: from orbit the planet reads as a GRID rather than as ground. Filtering cannot
// fix it at sample time, so the chain has to exist.
//
// Layout contract: wgpu wants every mip level concatenated after the base, each half the previous size
// (floored, min 1), same row-major RGBA8 as the base. mip_level_count must match the number appended or
// upload panics.
pub fn with_mipmaps(mut img: Image) -> Image {
    let w0 = img.texture_descriptor.size.width as usize;
    let h0 = img.texture_descriptor.size.height as usize;
    let Some(base) = img.data.clone() else { return img };
    let mut out = base.clone();
    let (mut w, mut h, mut prev) = (w0, h0, base);
    let mut levels = 1u32;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                // box filter over the 2x2 parent block. Averaging in 8-bit sRGB is not gamma correct, but the
                // texture is a near-neutral multiplier centred on ~0.85, so the error stays far below a
                // quantization step and is not worth an encode/decode per texel.
                for c in 0..4 {
                    let mut sum = 0u32;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = (x * 2 + dx).min(w - 1);
                            let sy = (y * 2 + dy).min(h - 1);
                            sum += prev[(sy * w + sx) * 4 + c] as u32;
                        }
                    }
                    next[(y * nw + x) * 4 + c] = (sum / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        w = nw;
        h = nh;
        levels += 1;
    }
    img.texture_descriptor.mip_level_count = levels;
    img.data = Some(out);
    img
}
