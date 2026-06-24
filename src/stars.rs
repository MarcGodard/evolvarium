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
use bevy::mesh::{Indices, PrimitiveTopology};
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
        let s = 22.0 * (0.32 + lum) * (r / 9000.0); // quad half-size, scaled so angular size is constant at any shell r
        let rgb = temp_to_rgb(k);
        let i = (0.45 + lum * 0.55).min(1.0);
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
    fn temp_color_warm_vs_hot() {
        let cool = temp_to_rgb(3000.0); // reddish: red > blue
        let hot = temp_to_rgb(20000.0); // bluish: blue >= red
        assert!(cool[0] > cool[2], "3000K should be red-dominant");
        assert!(hot[2] >= hot[0] * 0.9, "20000K should be blue-rich");
    }
}
