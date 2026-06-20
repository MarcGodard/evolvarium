// Static terrain heightfield (BACKLOG P3). Rolling hills give real 3D range of motion: creatures
// ride the surface, climbing costs energy and descending refunds less (dissipative, see 10).
// Deterministic pure function of (x,z) so headless + render agree and runs stay reproducible.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

pub const HEIGHT_MAX: f32 = 6.0; // peak terrain elevation (world units)

// Surface height at (x,z). Two-frequency sinusoid hills normalized to 0..HEIGHT_MAX.
pub fn height(x: f32, z: f32) -> f32 {
    let a = (x * 0.12).sin() * (z * 0.10).cos();
    let b = 0.5 * (x * 0.05 + 1.3).sin() * (z * 0.07 - 0.7).sin();
    let n = ((a + b) * 0.5 + 0.5).clamp(0.0, 1.0); // ~0..1
    n * HEIGHT_MAX
}

// Build a render mesh of the heightfield over [-span/2, span/2]^2 (render mode only).
pub fn build_mesh(span: f32, res: usize) -> Mesh {
    let half = span / 2.0;
    let step = span / (res - 1) as f32;
    let mut positions = Vec::with_capacity(res * res);
    let mut uvs = Vec::with_capacity(res * res);
    for j in 0..res {
        for i in 0..res {
            let x = -half + i as f32 * step;
            let z = -half + j as f32 * step;
            positions.push([x, height(x, z), z]);
            uvs.push([i as f32 / (res - 1) as f32, j as f32 / (res - 1) as f32]);
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
    mesh.insert_indices(Indices::U32(indices));
    mesh.compute_normals();
    mesh
}
