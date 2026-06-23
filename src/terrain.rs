// Planet globe mesh (render only). World fields (terrain elevation, oceans, climate, noise) live in
// `sphere.rs`; this module turns them into a render mesh. No flat-heightfield path: world is a planet.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

pub const HEIGHT_MAX: f32 = 12.0; // peak terrain elev above sea sphere (world units). sphere::ELEV_MAX aliases this

// UV sphere displaced by terrain elevation, vertex-colored by biome (oceans blue, land green/sand/rock,
// polar ice). `res` = latitude bands. longitude uses 2*res.
pub fn build_globe(res: usize) -> Mesh {
    build_globe_colored(res, |d| crate::sphere::biome_color(d))
}

// As build_globe but vertex colors come from `color` (any field -> rgb) -> reused for the inspector minimap's
// field overlays (heat/moisture/elevation) on a small synced globe. `color` gets a UNIT surface dir.
pub fn build_globe_colored(res: usize, color: impl Fn(Vec3) -> [f32; 3]) -> Mesh {
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
            normals.push([d.x, d.y, d.z]); // radial normal, not geometric. smooth shading, ignores elev slope
            let c = color(d);
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
