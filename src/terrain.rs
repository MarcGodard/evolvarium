// Static terrain heightfield (BACKLOG P3). Rolling hills give real 3D range of motion: creatures
// ride the surface, climbing costs energy and descending refunds less (dissipative, see 10).
// Deterministic pure function of (x,z) so headless + render agree and runs stay reproducible.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

pub const HEIGHT_MAX: f32 = 12.0; // peak terrain elevation (world units)
pub const WATER_LEVEL: f32 = 2.8; // y below which low basins flood (render water plane sits here)
const ROCK_START: f32 = 0.6; // normalized height where rocky highland begins

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
    let rock = [0.38, 0.37, 0.36]; // dark rock (distinct from grass)
    let peak = [0.88, 0.88, 0.92]; // bright bare peak
    // grass band: darker green near water, brighter grass higher (narrow blend = defined shoreline)
    let land = lerp3(wet, grass, ((h01 - 0.18) / 0.18).clamp(0.0, 1.0));
    let arid = ((0.42 - moist) / 0.12).clamp(0.0, 1.0); // sharp desert edge around moisture 0.42
    let land = lerp3(land, sand, arid);
    let rk = ((h01 - ROCK_START) / 0.12).clamp(0.0, 1.0); // sharp rock edge at ROCK_START
    let c = lerp3(land, lerp3(rock, peak, ((h01 - ROCK_START) / (1.0 - ROCK_START)).clamp(0.0, 1.0)), rk);
    [c[0], c[1], c[2], 1.0]
}

// Surface height at (x,z). Two-frequency sinusoid hills normalized to 0..HEIGHT_MAX.
pub fn height(x: f32, z: f32) -> f32 {
    let a = (x * 0.12).sin() * (z * 0.10).cos();
    let b = 0.5 * (x * 0.05 + 1.3).sin() * (z * 0.07 - 0.7).sin();
    let n = ((a + b) * 0.5 + 0.5).clamp(0.0, 1.0); // ~0..1
    n * HEIGHT_MAX
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
