// Static terrain heightfield (BACKLOG P3). Rolling hills give real 3D range of motion: creatures
// ride the surface, climbing costs energy and descending refunds less (dissipative, see 10).
// Deterministic pure function of (x,z) so headless + render agree and runs stay reproducible.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

pub const HEIGHT_MAX: f32 = 12.0; // peak terrain elevation (world units)
pub const WATER_LEVEL: f32 = 2.8; // y below which low basins flood (render water plane sits here)
const ROCK_START: f32 = 0.48; // normalized height where rocky highland begins (lower = more rock visible)

// Rockiness 0..1 at (x,z): 0 below ROCK_START, ramps to 1 at the peaks. Rocky = hard to cross + barely
// any plants grow (see sim movement cost + plant_habitability).
pub fn rockiness(x: f32, z: f32) -> f32 {
    ((height(x, z) / HEIGHT_MAX - ROCK_START) / (1.0 - ROCK_START)).clamp(0.0, 1.0)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

// Terrain color = four DEFINED biomes (sharp transitions): wet valley + grass (green), desert (sand,
// low moisture), rocky highland (grey, high elevation). Reads the landscape + biomes at a glance.
fn terrain_color(h01: f32, moist: f32) -> [f32; 4] {
    let wet = [0.13, 0.44, 0.22]; // lush shoreline / wet valley
    let grass = [0.22, 0.64, 0.18]; // vivid grassland (clearly green)
    let sand = [0.86, 0.76, 0.48]; // desert
    let rock = [0.50, 0.46, 0.40]; // warm grey rock (clearly distinct from green grass)
    let peak = [0.82, 0.80, 0.78]; // pale bare peak
    // grass band: darker green near water, brighter grass higher (narrow blend = defined shoreline)
    let land = lerp3(wet, grass, ((h01 - 0.18) / 0.18).clamp(0.0, 1.0));
    let arid = ((0.42 - moist) / 0.12).clamp(0.0, 1.0); // sharp desert edge around moisture 0.42
    let land = lerp3(land, sand, arid);
    let rk = ((h01 - ROCK_START) / 0.12).clamp(0.0, 1.0); // sharp rock edge at ROCK_START
    let c = lerp3(land, lerp3(rock, peak, ((h01 - ROCK_START) / (1.0 - ROCK_START)).clamp(0.0, 1.0)), rk);
    [c[0], c[1], c[2], 1.0]
}

// --- value-noise fBm for organic terrain (deterministic, no external crate) ---
fn hash2(i: i32, j: i32) -> f32 {
    let mut h = (i.wrapping_mul(374761393).wrapping_add(j.wrapping_mul(668265263))) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}
fn value_noise(x: f32, z: f32) -> f32 {
    let (xi, zi) = (x.floor(), z.floor());
    let (xf, zf) = (x - xi, z - zi);
    let (i, j) = (xi as i32, zi as i32);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t); // smoothstep
    let (u, v) = (smooth(xf), smooth(zf));
    let n00 = hash2(i, j);
    let n10 = hash2(i + 1, j);
    let n01 = hash2(i, j + 1);
    let n11 = hash2(i + 1, j + 1);
    let nx0 = n00 + (n10 - n00) * u;
    let nx1 = n01 + (n11 - n01) * u;
    nx0 + (nx1 - nx0) * v
}
// fractional Brownian motion: sum octaves of value noise -> natural rolling terrain, ~0..1.
fn fbm(x: f32, z: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for _ in 0..4 {
        sum += amp * value_noise(x * freq, z * freq);
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / 0.9375 // normalize (0.5+0.25+0.125+0.0625) to ~0..1
}

// Drifting clouds: a scrolling noise field of patchy shade 0..1 (0 full sun, up to CLOUD_MAX_SHADE under
// a cloud). Deterministic fn of (x,z,tick) so headless + render agree. Dims LOCAL light -> couples to
// plant + creature light sensitivity (shade-lovers gain under clouds, sun-lovers lose), and is drawn in viz.
const CLOUD_SCALE: f32 = 0.016; // spatial frequency of cloud patches
const CLOUD_SPEED: f32 = 0.03;  // wind drift in world units per tick
const CLOUD_COVER: f32 = 0.6;   // noise threshold (higher = sparser clouds)
pub const CLOUD_MAX_SHADE: f32 = 0.2; // max local light reduction under a cloud (gentle: stronger dimming cut plant growth enough to lower carrying capacity + tip marginal populations)
pub fn cloud_shade(x: f32, z: f32, tick: u32) -> f32 {
    let t = tick as f32 * CLOUD_SPEED;
    let n = fbm((x + t) * CLOUD_SCALE, z * CLOUD_SCALE); // scrolls with the wind
    ((n - CLOUD_COVER) / (1.0 - CLOUD_COVER)).clamp(0.0, 1.0) * CLOUD_MAX_SHADE
}

// A meandering river: 1 along a winding centerline, falling to 0 past the bank. Carves the channel low
// so water flows as a river rather than round pools. Centerline winds in world coords (z varies x).
fn river(x: f32, z: f32) -> f32 {
    let center_x = 34.0 * (z * 0.018).sin() + 16.0 * (z * 0.047 + 1.3).sin();
    let bank = 9.0;
    1.0 - ((x - center_x).abs() / bank).min(1.0)
}

// Surface height at (x,z): organic fBm hills, with a winding river channel carved in.
pub fn height(x: f32, z: f32) -> f32 {
    let base = fbm(x * 0.022, z * 0.022); // broad natural hills/valleys (~0..1)
    let h = base * (1.0 - 0.92 * river(x, z)); // carve the river channel down toward the floor
    h * HEIGHT_MAX
}

// Soil moisture 0..1 at (x,z) for a given season phase (-1 dry .. +1 wet). Lowlands hold water, plus
// a spatial patch pattern; season shifts the whole map wetter/drier over time. Drives plant mortality
// (P3): a plant whose moisture preference is far from local moisture is stressed and may die.
pub fn moisture(x: f32, z: f32, season: f32) -> f32 {
    let low = 1.0 - height(x, z) / HEIGHT_MAX; // low ground = wet
    let patch = 0.5 + 0.5 * (x * 0.08).sin() * (z * 0.06).cos();
    let base = 0.6 * low + 0.4 * patch;
    // broad arid zones (deserts): large-scale dry basins that suppress moisture hard -> only
    // drought-tolerant (low `wet`) plants survive there, giving sparse desert flora + sand color.
    let desert = (0.5 + 0.5 * (x * 0.035 + 1.7).sin() * (z * 0.03 - 0.9).sin()).powi(2); // 0..1, high=arid
    let m = base * (1.0 - 0.85 * desert);
    (m + 0.2 * season).clamp(0.0, 1.0)
}

// Plant habitability 0..1 at (x,z): near 0 in deep water (land flora drowns) and in arid desert
// (desiccation), high on moderate-moisture land. Limits food growth + survival in both extremes (P3).
pub fn plant_habitability(x: f32, z: f32, season: f32) -> f32 {
    let submerged = ((WATER_LEVEL - height(x, z)) / 2.0).clamp(0.0, 1.0); // depth below water, ~2 units to 0
    let water_ok = 1.0 - submerged;
    let arid = (1.0 - moisture(x, z, season) / 0.35).clamp(0.0, 1.0); // moisture < 0.35 -> increasingly arid
    let dry_ok = 1.0 - arid;
    let rock_ok = 1.0 - 0.9 * rockiness(x, z); // rocky highland: very few plants grow
    (water_ok * dry_ok * rock_ok).clamp(0.0, 1.0)
}

// Build the planet globe mesh: a UV sphere displaced by terrain elevation + vertex-colored by biome
// (oceans blue, land green/sand/rock, polar ice). `res` = latitude bands (longitude uses 2*res). Render only.
pub fn build_globe(res: usize) -> Mesh {
    use crate::sphere;
    let (rows, cols) = (res, res * 2);
    let mut positions = Vec::with_capacity((rows + 1) * (cols + 1));
    let mut normals = Vec::with_capacity((rows + 1) * (cols + 1));
    let mut colors = Vec::with_capacity((rows + 1) * (cols + 1));
    for j in 0..=rows {
        let lat = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * j as f32 / rows as f32;
        for i in 0..=cols {
            let lon = -std::f32::consts::PI + std::f32::consts::TAU * i as f32 / cols as f32;
            let d = sphere::lonlat_to_pos(lon, lat, 0.0).normalize();
            let pos = d * (sphere::PLANET_R + sphere::elevation(d));
            positions.push([pos.x, pos.y, pos.z]);
            normals.push([d.x, d.y, d.z]); // smooth sphere normal (good enough for shading)
            let c = sphere::biome_color(d);
            colors.push([c[0], c[1], c[2], 1.0]);
        }
    }
    let stride = (cols + 1) as u32;
    let mut indices = Vec::with_capacity(rows * cols * 6);
    for j in 0..rows as u32 {
        for i in 0..cols as u32 {
            let a = j * stride + i;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

// Build a render mesh of the heightfield over [-span/2, span/2]^2 (render mode only).
pub fn build_mesh(span: f32, res: usize) -> Mesh {
    let half = span / 2.0;
    let step = span / (res - 1) as f32;
    let mut positions = Vec::with_capacity(res * res);
    let mut uvs = Vec::with_capacity(res * res);
    let mut colors = Vec::with_capacity(res * res);
    for j in 0..res {
        for i in 0..res {
            let x = -half + i as f32 * step;
            let z = -half + j as f32 * step;
            let h = height(x, z);
            positions.push([x, h, z]);
            uvs.push([i as f32 / (res - 1) as f32, j as f32 / (res - 1) as f32]);
            colors.push(terrain_color(h / HEIGHT_MAX, moisture(x, z, 0.0))); // shade by elevation + biome
        }
    }
    let mut indices = Vec::with_capacity((res - 1) * (res - 1) * 6);
    for j in 0..res - 1 {
        for i in 0..res - 1 {
            let a = (j * res + i) as u32;
            let b = a + 1;
            let c = a + res as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]); // CCW from above -> up-facing normals
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh.compute_normals();
    mesh
}
