//! Real starfield for the orrery view: Yale Bright Star Catalog (~8200 stars) + constellation lines,
//! imported from TYCHOSIUM (pholmq/TSN, GPL-2.0). BSC.json: per star RA/Dec (sexagesimal strings), V
//! (visual magnitude), K (color temperature). constellations.json: polylines of HIP star ids.
//!
//! DATA SOURCE: pholmq/TSN @ commit 49fd49c5f3e902b907ab3788c8fa484f88dd5937 (2026-05-27),
//! files src/settings/{BSC.json, constellations.json}. To refresh, re-pull from that repo + bump this SHA.
//!
//! Build ONE combined mesh (a tiny camera-facing quad per star, vertex-colored by temperature, sized by
//! magnitude) -> single draw call for thousands of stars. Constellation lines = one LineList mesh. Both are
//! placed on a far shell around the orrery center so the solar system sits inside a real sky.
#![allow(dead_code)]
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

const BSC_JSON: &str = include_str!("../assets/stars/BSC.json");
const CONSTELLATIONS_JSON: &str = include_str!("../assets/stars/constellations.json");

// Field getters tolerant of string-or-number JSON (BSC mixes "6.29" and 6.29 across entries).
fn f32_field(o: &serde_json::Value, key: &str) -> Option<f32> {
    let v = o.get(key)?;
    v.as_f64().map(|x| x as f32).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}
fn u32_field(o: &serde_json::Value, key: &str) -> Option<u32> {
    let v = o.get(key)?;
    v.as_u64().map(|x| x as u32).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// A catalog star with identity fields, for click-to-identify. `dir` is the equatorial unit direction.
pub struct StarInfo {
    pub dir: Vec3,
    pub hr: String,
    pub hip: Option<u32>,
    pub name: Option<String>,        // proper name (N)
    pub bayer: Option<String>,       // Bayer/Flamsteed designation (B)
    pub constellation: Option<String>, // 3-letter abbr (C)
    pub mag: f32,
    pub temp: f32,
}

impl StarInfo {
    /// Human label: proper name if any, else Bayer+constellation, else HR number.
    pub fn label(&self) -> String {
        if let Some(n) = &self.name {
            n.clone()
        } else if let (Some(b), Some(c)) = (&self.bayer, &self.constellation) {
            format!("{b} {c}")
        } else if let Some(c) = &self.constellation {
            format!("HR {} ({c})", self.hr)
        } else {
            format!("HR {}", self.hr)
        }
    }
}

fn str_field(o: &serde_json::Value, key: &str) -> Option<String> {
    o.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Parse the BSC catalog into queryable star records (for picking). Equatorial dirs (celestial pole +Y).
pub fn star_catalog() -> Vec<StarInfo> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(BSC_JSON).unwrap_or_default();
    let mut out = Vec::with_capacity(raw.len());
    for st in &raw {
        let (Some(ras), Some(decs)) = (st.get("RA").and_then(|x| x.as_str()), st.get("Dec").and_then(|x| x.as_str())) else { continue };
        let (Some(ra), Some(dec)) = (parse_ra(ras), parse_dec(decs)) else { continue };
        out.push(StarInfo {
            dir: radec_to_dir(ra, dec),
            hr: str_field(st, "HR").unwrap_or_default(),
            hip: u32_field(st, "HIP"),
            name: str_field(st, "N"),
            bayer: str_field(st, "B"),
            constellation: str_field(st, "C"),
            mag: f32_field(st, "V").unwrap_or(6.0),
            temp: f32_field(st, "K").unwrap_or(5500.0),
        });
    }
    out
}

// "00h 05m 03.8s" -> radians (hours*15 deg). Tolerant: strip unit letters, take up to 3 numbers.
fn parse_ra(s: &str) -> Option<f32> {
    let cleaned: String = s.chars().map(|c| if c.is_ascii_digit() || c == '.' || c == '-' { c } else { ' ' }).collect();
    let p: Vec<f32> = cleaned.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    if p.is_empty() {
        return None;
    }
    let hours = p[0] + p.get(1).copied().unwrap_or(0.0) / 60.0 + p.get(2).copied().unwrap_or(0.0) / 3600.0;
    Some((hours * 15.0).to_radians())
}

// "-00° 30′ 10″" -> radians. Sign read from the raw string ("-00" loses sign under parse).
fn parse_dec(s: &str) -> Option<f32> {
    let sign = if s.trim_start().starts_with('-') { -1.0 } else { 1.0 };
    let cleaned: String = s.chars().map(|c| if c.is_ascii_digit() || c == '.' { c } else { ' ' }).collect();
    let p: Vec<f32> = cleaned.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    if p.is_empty() {
        return None;
    }
    let deg = p[0] + p.get(1).copied().unwrap_or(0.0) / 60.0 + p.get(2).copied().unwrap_or(0.0) / 3600.0;
    Some((sign * deg).to_radians())
}

// Equatorial RA/Dec -> unit direction. RA about +Y (celestial poles at +/-Y); good enough for a backdrop sky.
fn radec_to_dir(ra: f32, dec: f32) -> Vec3 {
    Vec3::new(dec.cos() * ra.cos(), dec.sin(), dec.cos() * ra.sin())
}

// Color temperature (Kelvin) -> linear RGB 0..1 (Tanner Helland approximation, as TSN uses).
fn temp_to_rgb(k: f32) -> [f32; 3] {
    let t = (k / 100.0).clamp(10.0, 400.0);
    let r = if t <= 66.0 { 255.0 } else { 329.698_73 * (t - 60.0).powf(-0.133_204_76) };
    let g = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_17 * (t - 60.0).powf(-0.075_514_85)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    [(r / 255.0).clamp(0.0, 1.0), (g / 255.0).clamp(0.0, 1.0), (b / 255.0).clamp(0.0, 1.0)]
}

/// Build the starfield mesh (one camera-facing quad per star) on a shell of radius `r`. Also returns the
/// HIP -> direction map (for constellation lines). Mesh is centered at origin; place the entity at the sky
/// center. Quads face the shell center (where the camera sits looking out), vertex-colored by temperature.
pub fn build_starfield(r: f32) -> (Mesh, HashMap<u32, Vec3>) {
    let stars: Vec<serde_json::Value> = serde_json::from_str(BSC_JSON).unwrap_or_default();
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(stars.len() * 4);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(stars.len() * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(stars.len() * 6);
    let mut hip_dir: HashMap<u32, Vec3> = HashMap::with_capacity(stars.len());

    for st in &stars {
        let (Some(ras), Some(decs)) = (st.get("RA").and_then(|x| x.as_str()), st.get("Dec").and_then(|x| x.as_str())) else { continue };
        let (Some(ra), Some(dec)) = (parse_ra(ras), parse_dec(decs)) else { continue };
        let dir = radec_to_dir(ra, dec);
        if let Some(hip) = u32_field(st, "HIP") {
            hip_dir.insert(hip, dir);
        }
        let v: f32 = f32_field(st, "V").unwrap_or(6.0);
        let k: f32 = f32_field(st, "K").unwrap_or(5500.0);
        // brightness 0.08..1 from magnitude (bright = low V). Size + color intensity scale with it.
        let lum = ((6.5 - v) / 8.0).clamp(0.06, 1.0);
        let s = 14.0 * (0.28 + lum) * (r / 9000.0); // quad half-size, scaled so angular size is constant at any shell r
        let rgb = temp_to_rgb(k);
        let i = (0.30 + lum * 0.70).min(1.0); // dim faint stars harder so the field reads as points, not a wall
        let col = [rgb[0] * i, rgb[1] * i, rgb[2] * i, 1.0];

        let pos = dir * r;
        // basis perpendicular to dir -> quad in the tangent plane (faces the center/camera).
        let up_ref = if dir.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
        let right = dir.cross(up_ref).normalize_or_zero();
        let up = dir.cross(right).normalize_or_zero();
        let base = positions.len() as u32;
        for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = pos + right * (du * s) + up * (dv * s);
            positions.push([p.x, p.y, p.z]);
            colors.push(col);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    (mesh, hip_dir)
}

// Galactic (l,b) -> equatorial unit dir matching radec_to_dir convention. Bakes J2000 galactic->equatorial
// rotation so the band lands on the real RA/Dec (galactic center in Sagittarius, NGP in Coma). Lets the
// Milky Way sit correctly against the BSC stars. Matrix rows = standard equatorial (Z=NCP) basis; we recover
// RA/Dec then reuse radec_to_dir so axis convention matches the catalog exactly.
fn gal_to_dir(l: f32, b: f32) -> Vec3 {
    let (sl, cl) = l.sin_cos();
    let (sb, cb) = b.sin_cos();
    let g = [cb * cl, cb * sl, sb]; // galactic: x->center, z->NGP
    // J2000 galactic->equatorial rotation (standard, Z=NCP).
    let ex = -0.054_875_56 * g[0] + 0.494_109_43 * g[1] - 0.867_666_15 * g[2];
    let ey = -0.873_437_10 * g[0] - 0.444_829_63 * g[1] - 0.198_076_37 * g[2];
    let ez = -0.483_835_02 * g[0] + 0.746_982_25 * g[1] + 0.455_983_78 * g[2];
    let ra = ey.atan2(ex);
    let dec = ez.clamp(-1.0, 1.0).asin();
    radec_to_dir(ra, dec)
}

// Tiny deterministic LCG -> uniforms in 0..1. No std::rand dep; reproducible band per build.
fn lcg(state: &mut u64) -> f32 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
    ((*state >> 40) as f32) / ((1u64 << 24) as f32)
}

/// Build a faint Milky Way band on a shell of radius `r`: thousands of soft additive patches scattered along
/// the galactic plane (Gaussian falloff in galactic latitude), brightened toward the galactic center and the
/// bulge. Positioned via real galactic->equatorial transform so it aligns with the catalog stars and wheels
/// with them. Use an ADDITIVE material so it glows over the dark sky and vanishes against bright day sky.
pub fn build_milky_way(r: f32) -> Mesh {
    const N: usize = 2600;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(N * 4);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(N * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(N * 6);
    let mut rng: u64 = 0x5EED_1234_ABCD_0001;
    for _ in 0..N {
        let l = lcg(&mut rng) * std::f32::consts::TAU;
        // galactic latitude: Box-Muller Gaussian, sigma wider toward the central bulge (l near 0).
        let center = ((l + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs(); // 0 at GC..PI at anticenter
        let centerness = (1.0 - center / std::f32::consts::PI).powf(1.5); // bright bulge near GC
        let sigma = 0.045 + 0.08 * centerness; // rad: ~2.5deg disk, ~7deg bulge
        let u1 = lcg(&mut rng).max(1e-6);
        let u2 = lcg(&mut rng);
        let b = sigma * (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
        let dir = gal_to_dir(l, b);

        // faint patch: brighter near plane + center; jittered so the band looks mottled, not uniform.
        let lat_fall = (-(b / sigma) * (b / sigma) * 0.5).exp();
        let mottle = 0.35 + 0.65 * lcg(&mut rng);
        let bright = (0.022 + 0.06 * centerness) * lat_fall * mottle; // faint: subtle glow band, not squares
        let col = [bright * 0.82, bright * 0.84, bright, 1.0]; // pale blue-white

        let s = (12.0 + 14.0 * lcg(&mut rng)) * (r / 9000.0); // small soft patches; overlap -> smooth glow
        let pos = dir * r;
        let up_ref = if dir.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
        let right = dir.cross(up_ref).normalize_or_zero();
        let up = dir.cross(right).normalize_or_zero();
        let base = positions.len() as u32;
        for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = pos + right * (du * s) + up * (dv * s);
            positions.push([p.x, p.y, p.z]);
            colors.push(col);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Procedural equirectangular lunar texture (grey highlands + dark maria + cratered). For the planet's moon,
/// mapped onto a UV sphere and tidally locked so the same face points at the world (no real photo asset, so
/// we synthesize a moon-like surface). u = longitude, v = latitude; near-side maria roughly placed.
pub fn moon_texture() -> Image {
    const W: usize = 512;
    const H: usize = 256;
    // (u, v, radius, depth): dark mare basins clustered on the near face (centered ~u=0.5).
    let maria = [
        (0.46_f32, 0.40_f32, 0.15_f32, 0.24_f32),
        (0.40, 0.34, 0.11, 0.20),
        (0.55, 0.46, 0.13, 0.18),
        (0.50, 0.55, 0.10, 0.16),
        (0.61, 0.38, 0.08, 0.14),
        (0.36, 0.50, 0.07, 0.12),
    ];
    let mut rng: u64 = 0xC0FF_EE12_3400_0777;
    let craters: Vec<(f32, f32, f32)> = (0..110)
        .map(|_| {
            let u = lcg(&mut rng);
            let v = lcg(&mut rng);
            let r = 0.005 + 0.028 * lcg(&mut rng).powi(2); // many small, few large
            (u, v, r)
        })
        .collect();
    // shortest longitudinal distance (texture wraps in u).
    let du_wrap = |a: f32, b: f32| {
        let mut d = a - b;
        if d > 0.5 {
            d -= 1.0;
        } else if d < -0.5 {
            d += 1.0;
        }
        d
    };
    let mut data = vec![0u8; W * H * 4];
    for y in 0..H {
        for x in 0..W {
            let u = x as f32 / W as f32;
            let v = y as f32 / H as f32;
            let mut g = 0.60 + 0.04 * ((u * 41.0).sin() * (v * 33.0).cos()); // faint highland mottle
            for (mu, mv, mr, md) in maria {
                let du = du_wrap(u, mu);
                let dv = v - mv;
                let d2 = (du * du + dv * dv) / (mr * mr);
                g -= md * (-d2).exp();
            }
            for (cu, cv, cr) in &craters {
                let du = du_wrap(u, *cu);
                let dv = v - *cv;
                let d = (du * du + dv * dv).sqrt();
                if d < *cr {
                    g -= 0.12 * (1.0 - d / cr); // dark floor
                } else if d < cr * 1.4 {
                    g += 0.12 * (1.0 - (d - cr) / (cr * 0.4)); // bright rim
                }
            }
            let g = g.clamp(0.06, 0.94);
            let i = (y * W + x) * 4;
            data[i] = (g * 252.0) as u8;
            data[i + 1] = (g * 250.0) as u8;
            data[i + 2] = (g * 255.0) as u8; // faint cool tint
            data[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d { width: W as u32, height: H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Build constellation lines as a single LineList mesh from TSN constellations.json (polylines of HIP ids),
/// looked up in `hip_dir`. Lines sit just inside the star shell (`r`). None if nothing resolved.
pub fn build_constellation_lines(hip_dir: &HashMap<u32, Vec3>, r: f32) -> Option<Mesh> {
    let root: serde_json::Value = serde_json::from_str(CONSTELLATIONS_JSON).ok()?;
    let cons = root.get("constellations")?.as_array()?;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    for con in cons {
        let Some(lines) = con.get("lines").and_then(|l| l.as_array()) else { continue };
        for line in lines {
            let Some(ids) = line.as_array() else { continue };
            // polyline: connect consecutive resolvable stars as LineList segment pairs
            let mut prev: Option<Vec3> = None;
            for id in ids {
                let dir = id.as_u64().and_then(|h| hip_dir.get(&(h as u32)).copied());
                if let (Some(a), Some(b)) = (prev, dir) {
                    let pa = a * (r * 0.999);
                    let pb = b * (r * 0.999);
                    positions.push([pa.x, pa.y, pa.z]);
                    positions.push([pb.x, pb.y, pb.z]);
                }
                prev = dir.or(prev);
            }
        }
    }
    if positions.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ra_dec() {
        // 00h -> 0 rad; 06h -> 90 deg; "-00 30" -> small negative dec (sign preserved)
        assert!(parse_ra("00h 00m 00.0s").unwrap().abs() < 1e-4);
        assert!((parse_ra("06h 00m 00s").unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        assert!(parse_dec("-00° 30′ 00″").unwrap() < 0.0, "southern dec sign must survive");
        assert!((parse_dec("+90° 00′ 00″").unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
    }

    #[test]
    fn catalog_parses_thousands() {
        let (mesh, hip) = build_starfield(9000.0);
        assert!(hip.len() > 3000, "expected thousands of stars, got {}", hip.len());
        assert!(mesh.count_vertices() > 12000, "verts {}", mesh.count_vertices());
    }

    #[test]
    fn constellations_resolve() {
        let (_m, hip) = build_starfield(9000.0);
        let lines = build_constellation_lines(&hip, 9000.0);
        assert!(lines.is_some(), "constellation lines should resolve from HIP map");
    }

    #[test]
    fn catalog_has_named_stars() {
        let cat = star_catalog();
        assert!(cat.len() > 3000, "catalog size {}", cat.len());
        let named = cat.iter().filter(|s| s.name.is_some()).count();
        assert!(named > 10, "expected proper-named stars, got {named}");
        // every record has a usable label + unit direction
        for s in cat.iter().take(50) {
            assert!(!s.label().is_empty());
            assert!((s.dir.length() - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn milky_way_aligns_with_galactic_center() {
        // galactic center (l=0,b=0) must land at RA ~266.4 deg, Dec ~-28.9 deg (Sagittarius).
        let gc = gal_to_dir(0.0, 0.0);
        let dec = gc.y.asin().to_degrees();
        let ra = gc.z.atan2(gc.x).rem_euclid(std::f32::consts::TAU).to_degrees();
        assert!((dec - (-28.9)).abs() < 1.5, "GC dec {dec}");
        assert!((ra - 266.4).abs() < 2.0, "GC ra {ra}");
        // band mesh has thousands of patches
        let m = build_milky_way(9000.0);
        assert!(m.count_vertices() > 8000, "milky way verts {}", m.count_vertices());
    }

    #[test]
    fn moon_texture_is_well_formed() {
        let img = moon_texture();
        let sz = img.texture_descriptor.size;
        assert_eq!(sz.width, 512);
        assert_eq!(sz.height, 256);
        assert_eq!(img.data.as_ref().map(|d| d.len()), Some(512 * 256 * 4));
    }

    #[test]
    fn temp_color_warm_vs_hot() {
        let cool = temp_to_rgb(3000.0); // reddish: red > blue
        let hot = temp_to_rgb(20000.0); // bluish: blue >= red
        assert!(cool[0] > cool[2], "3000K should be red-dominant");
        assert!(hot[2] >= hot[0] * 0.9, "20000K should be blue-rich");
    }
}
