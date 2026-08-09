// Planet globe mesh (render only). World fields (terrain elevation, oceans, climate, noise) live in
// `sphere.rs`; this module turns them into a render mesh. No flat-heightfield path: world is a planet.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

pub const HEIGHT_MAX: f32 = 12.0; // peak terrain elev above sea sphere (world units). sphere::ELEV_MAX aliases this
// Render-only ocean-floor drop: shallow shelf (elevation ~0 at coast) would otherwise sit a hair below the
// translucent ocean shell (~PLANET_R+0.16) -> z-fight shimmer seen through the water in orbit. Drop the whole
// ocean floor by this so the shelf clears the shell with margin. Sim bathymetry (sphere::elevation) untouched.
pub const OCEAN_RENDER_DROP: f32 = 0.8;

/// Surface point AS RENDERED. Differs from `sphere::surface_pos` under water, where the globe mesh sinks the
/// floor by OCEAN_RENDER_DROP to clear the ocean shell. Anything seated on the seabed must use this: sampling
/// the sim bathymetry instead leaves it hovering exactly that far above the visible floor, which is what the
/// kelp was doing.
pub fn render_surface_pos(d: Vec3, offset: f32) -> Vec3 {
    let drop = if crate::sphere::elevation(d) < 0.0 { OCEAN_RENDER_DROP } else { 0.0 };
    crate::sphere::surface_pos(d, offset - drop)
}

// UV sphere displaced by terrain elevation, vertex-colored by biome (oceans blue, land green/sand/rock,
// polar ice). `res` = latitude bands. longitude uses 2*res.
pub fn build_globe(res: usize) -> Mesh {
    // ground_tint MULTIPLIES the biome hue so large areas stop reading as one flat wash. Planet only: the
    // minimap reuses build_globe_colored for field overlays where mottling would corrupt the reading.
    build_globe_colored(res, |d| {
        let m = crate::sphere::moisture(d);
        let c = crate::sphere::biome_color_with_moisture(d, m);
        let t = crate::viz_ground::ground_tint(d, m, crate::sphere::base_temperature(d), crate::sphere::rockiness(d));
        [c[0] * t[0], c[1] * t[1], c[2] * t[2]]
    })
}

// As build_globe but vertex colors come from `color` (any field -> rgb) -> reused for the inspector minimap's
// field overlays (heat/moisture/elevation) on a small synced globe. `color` gets a UNIT surface dir.
pub fn build_globe_colored(res: usize, color: impl Fn(Vec3) -> [f32; 3]) -> Mesh {
    use crate::sphere;
    let (rows, cols) = (res, res * 2);
    let mut positions = Vec::with_capacity((rows + 1) * (cols + 1));
    let mut normals = Vec::with_capacity((rows + 1) * (cols + 1));
    let mut colors = Vec::with_capacity((rows + 1) * (cols + 1));
    let mut uvs = Vec::with_capacity((rows + 1) * (cols + 1));
    for j in 0..=rows {
        let lat = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * j as f32 / rows as f32;
        for i in 0..=cols {
            let lon = -std::f32::consts::PI + std::f32::consts::TAU * i as f32 / cols as f32;
            let d = sphere::lonlat_to_pos(lon, lat, 0.0).normalize();
            let mut elev = sphere::elevation(d);
            if elev < 0.0 {
                elev -= OCEAN_RENDER_DROP; // sink ocean floor clear of the shell (render only)
            }
            let pos = d * (sphere::PLANET_R + elev);
            positions.push([pos.x, pos.y, pos.z]);
            normals.push([d.x, d.y, d.z]); // radial normal, not geometric. smooth shading, ignores elev slope
            let c = color(d);
            colors.push([c[0], c[1], c[2], 1.0]);
            uvs.push(crate::viz_ground::ground_uv(d));
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
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    let _ = mesh.generate_tangents(); // normal map is unusable without tangents; mesh had none
    mesh
}
