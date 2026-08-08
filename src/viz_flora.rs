// Procedural flora geometry: soft multi-segment grass tufts + lobed tree canopies, with baked per-vertex
// color. Pure mesh/color math, no ECS, no sim state.
//
// Vertex color is a MULTIPLIER on StandardMaterial::base_color, so values >1 brighten. Grass tips push
// past 1 on purpose (straw highlight); bases stay well under 1 (thatch shade). That gradient is what
// stops a tuft reading as one flat silhouette.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

// Deterministic hash noise -> 0..1. No rand dep, no state, same seed = same tuft every run (meshes are
// baked once at startup, so a stable result is required for --capture diffs).
fn rnd(seed: u32, k: u32) -> f32 {
    let mut x = seed ^ k.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}

fn rng(seed: u32, k: u32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rnd(seed, k)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

// Blade quad segments. 3 -> 7 verts per blade (3 paired rows + 1 tip point), 5 tris. Enough bend to read
// as an arc; 2 kinks visibly, 4 blows the vertex budget below.
const BLADE_SEGS: usize = 3;

// Grass tuft: `blades` curved tapering blades fanned around a small footprint, unit height (~1.0 max) so
// the caller scales length per soil (same contract as viz::grass_tuft_mesh).
//
// VERTEX COUNT = blades * 7 (BLADE_SEGS=3). Instanced up to GRASS_CAP (8000), so keep blades <= 16
// (112 verts). blades=14 is the tuned default look.
//
// Shape notes: blade bends by theta(t) = bend * t^1.7, so the lower third stands near-vertical and the
// droop concentrates near the tip (a straight tilt reads as a spike, a tip-loaded arc reads as grass).
// Width tapers to zero at the tip via (1-t)^0.75, giving a fine point instead of a wide wedge.
//
// NORMALS lean 70% toward local up. The true ribbon normal is horizontal at the base, which is unlit
// under a high sun and is what makes thin blades go black; the up bias keeps every blade catching
// overhead light while the remaining 30% of the geometric normal preserves shape variation between
// blades. Pair with a material that renders both faces WITHOUT normal flipping (cull_mode: None,
// double_sided: FALSE), otherwise back-facing blades get their normal negated to point at the ground and
// go black again.
pub fn grass_blade_mesh(blades: usize, seed: u32) -> Mesh {
    let blades = blades.max(1);
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(blades * 7);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(blades * 7);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(blades * 7);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(blades * 7);
    let mut indices: Vec<u32> = Vec::with_capacity(blades * 15);

    // base = shaded thatch deep in the clump; tip = sun-bleached straw. R above G at the tip yellows the
    // green base_color; both channels under 1 at the base darken it without graying it.
    let base_tint = [0.30f32, 0.40, 0.28];
    let mid_tint = [0.80f32, 0.92, 0.62];
    let tip_tint = [1.34f32, 1.12, 0.52];

    for k in 0..blades {
        let t = k as u32;
        // golden angle spreads headings evenly for any blade count; jitter breaks the visible spiral
        let a = k as f32 * 2.39996 + rng(seed, t * 7 + 1, -0.35, 0.35);
        let (sa, ca) = a.sin_cos();
        let r = rng(seed, t * 7 + 2, 0.015, 0.13); // root offset from clump center (footprint radius)
        let (ox, oz) = (r * ca, r * sa);
        let h = rng(seed, t * 7 + 3, 0.55, 1.0); // per-blade length; longest hits the unit height
        // outer blades bend harder (droop away from clump), inner ones stay upright -> layered dome
        let bend = rng(seed, t * 7 + 4, 0.45, 1.15) + 2.2 * r;
        let w = rng(seed, t * 7 + 5, 0.008, 0.015); // half-width at base. thin: fat blades read as spikes
        let shade = rng(seed, t * 7 + 6, 0.86, 1.1); // per-blade brightness so a tuft is not one flat tone

        let vbase = positions.len() as u32;
        let dl = h / BLADE_SEGS as f32;
        let (mut py, mut pz) = (0.0f32, 0.0f32);
        for s in 0..=BLADE_SEGS {
            let ft = s as f32 / BLADE_SEGS as f32;
            let th = bend * ft.powf(1.7);
            let (sth, cth) = th.sin_cos();
            // ribbon normal is perpendicular to blade width (local x) and tangent; sign picked so the
            // CONVEX (upper) face of the arc is the front side
            let gn = Vec3::new(0.0, sth, -cth);
            let n = (gn * 0.3 + Vec3::Y * 0.7).normalize_or_zero();
            let hw = if s == BLADE_SEGS { 0.0 } else { w * (1.0 - ft).powf(0.75) };
            let col = if ft < 0.5 { lerp3(base_tint, mid_tint, ft * 2.0) } else { lerp3(mid_tint, tip_tint, (ft - 0.5) * 2.0) };
            let col = [col[0] * shade, col[1] * shade, col[2] * shade, 1.0];
            let n_end = if s == BLADE_SEGS { 1 } else { 2 };
            for side in 0..n_end {
                let lx = if side == 0 { -hw } else { hw };
                // rotate blade local (x width, y up, z bend) about Y by heading, then offset to its root
                positions.push([lx * ca + pz * sa + ox, py, -lx * sa + pz * ca + oz]);
                normals.push([n.x * ca + n.z * sa, n.y, -n.x * sa + n.z * ca]);
                uvs.push([side as f32, ft]);
                colors.push(col);
            }
            py += cth * dl;
            pz += sth * dl;
        }
        for s in 0..BLADE_SEGS as u32 {
            let lo = vbase + s * 2;
            if s + 1 == BLADE_SEGS as u32 {
                indices.extend_from_slice(&[lo, lo + 1, lo + 2]); // tip triangle (single apex vert)
            } else {
                indices.extend_from_slice(&[lo, lo + 1, lo + 3, lo, lo + 3, lo + 2]);
            }
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

// Per-tuft grass color, sRGB 0..1, feed straight to Color::srgb. Inputs: moisture 0..1 (sphere::moisture),
// temp 0..1 (0 = polar, 1 = equatorial hot), seed = per-tuft hash.
//
// Wet cool ground -> deep blue-green; dry ground -> olive; dry AND hot -> straw. Per-tuft hue jitter is
// the point of this function: 8000 tufts sharing one exact green is what makes a field read as astroturf.
pub fn grass_colour(moisture: f32, temp: f32, seed: u32) -> [f32; 3] {
    let m = moisture.clamp(0.0, 1.0);
    let t = temp.clamp(0.0, 1.0);
    let lush = [0.13f32, 0.46, 0.21]; // blue-green, well-watered
    let olive = [0.36f32, 0.40, 0.16]; // moisture-starved
    let straw = [0.58f32, 0.50, 0.20]; // dry + hot, cured on the stem
    let cold = [0.22f32, 0.38, 0.28]; // desaturated tundra green
    // dryness curve: only the bottom half of the moisture range browns out (grass stays green until it
    // is genuinely water-stressed)
    let dry = ((1.0 - m) * 1.6 - 0.25).clamp(0.0, 1.0);
    let mut c = lerp3(lush, lerp3(olive, straw, t), dry);
    c = lerp3(cold, c, (t * 2.2).clamp(0.0, 1.0)); // polar tufts pull toward the gray-green
    let j = 0.055; // hue jitter half-range. beyond ~0.08 individual tufts start reading as a wrong species
    [
        (c[0] + rng(seed, 11, -j, j)).clamp(0.02, 0.85),
        (c[1] + rng(seed, 23, -j * 0.7, j * 0.7)).clamp(0.05, 0.9),
        (c[2] + rng(seed, 37, -j * 0.6, j * 0.6)).clamp(0.02, 0.6),
    ]
}

// Tree canopy: `lobes` overlapping irregular blobs filling a crown of `radius`, centered on the origin
// (drop-in for the blob_cluster_mesh broadleaf crown, which is also origin-centered at radius ~1).
//
// VERTEX COUNT = lobes * 40 (rings 4 x sectors 7). Trees number in the hundreds, not thousands, so this
// is far cheaper per frame than the grass budget. lobes=6 -> 240 verts.
//
// Lobe radii get low-frequency trig displacement so the silhouette is dented instead of a stack of clean
// spheres. Vertex color darkens with downward-facing normals (undersides in self-shade) and with depth
// below the crown top, so the crown reads as layered foliage rather than one smooth ball.
pub fn tree_canopy_mesh(lobes: usize, radius: f32, seed: u32) -> Mesh {
    let lobes = lobes.max(1);
    let (rings, sectors) = (4usize, 7usize);
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let deep = [0.34f32, 0.44, 0.34]; // shaded interior/underside
    let lit = [1.12f32, 1.10, 0.78]; // sunlit top, slightly yellowed

    for l in 0..lobes {
        let li = l as u32;
        // lobe 0 = core fill at the crown center; the rest ride a golden-angle ring biased upward so the
        // crown is denser on top than underneath
        let (center, lr) = if l == 0 {
            (Vec3::ZERO, radius * 0.72)
        } else {
            let a = l as f32 * 2.39996 + rng(seed, li * 5 + 1, -0.3, 0.3);
            let off = radius * rng(seed, li * 5 + 2, 0.42, 0.72);
            let y = radius * rng(seed, li * 5 + 3, -0.28, 0.5);
            (Vec3::new(a.cos() * off, y, a.sin() * off), radius * rng(seed, li * 5 + 4, 0.42, 0.62))
        };
        let ph = rng(seed, li * 5 + 5, 0.0, 6.28); // noise phase, keeps lobes from denting identically
        let start = positions.len() as u32;
        for ri in 0..=rings {
            let phi = std::f32::consts::PI * ri as f32 / rings as f32;
            let (sp, cp) = phi.sin_cos();
            for si in 0..=sectors {
                let th = std::f32::consts::TAU * si as f32 / sectors as f32;
                let (st, ct) = th.sin_cos();
                let n = Vec3::new(sp * ct, cp, sp * st);
                let bump = 1.0 + 0.16 * ((3.0 * phi + ph).sin() * (2.0 * th + ph * 1.7).cos()); // clumped foliage dents
                let p = center + n * (lr * bump);
                // top-facing + high-in-crown = lit; underside + low = shade
                let up01 = 0.5 + 0.5 * n.y;
                let hi01 = ((p.y / radius) * 0.5 + 0.5).clamp(0.0, 1.0);
                let v = (0.35 * up01 + 0.35 * hi01 + 0.3 * rnd(seed, li * 97 + ri as u32 * 13 + si as u32)).clamp(0.0, 1.0);
                let c = lerp3(deep, lit, v);
                positions.push([p.x, p.y, p.z]);
                normals.push([n.x, n.y, n.z]);
                uvs.push([si as f32 / sectors as f32, ri as f32 / rings as f32]);
                colors.push([c[0], c[1], c[2], 1.0]);
            }
        }
        let cols = sectors + 1;
        for ri in 0..rings {
            for si in 0..sectors {
                let a = start + (ri * cols + si) as u32;
                let b = start + (ri * cols + si + 1) as u32;
                let c = start + ((ri + 1) * cols + si) as u32;
                let d = start + ((ri + 1) * cols + si + 1) as u32;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    // guards the stated vertex budgets + index bounds; grass is instanced GRASS_CAP times so a silent
    // budget blowup here is a frame-time regression
    #[test]
    fn flora_meshes_are_within_budget() {
        for (m, expect) in [(grass_blade_mesh(14, 7), 14 * 7), (grass_blade_mesh(1, 1), 7), (tree_canopy_mesh(6, 1.0, 3), 6 * 40)] {
            let n = m.count_vertices();
            assert_eq!(n, expect);
            let Some(Indices::U32(ix)) = m.indices() else { panic!("want u32 indices") };
            assert_eq!(ix.len() % 3, 0);
            assert!(ix.iter().all(|&i| (i as usize) < n));
            assert!(m.attribute(Mesh::ATTRIBUTE_NORMAL).is_some() && m.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
        }
        // unit height + tight footprint: caller scales length/girth per soil
        let g = grass_blade_mesh(14, 5);
        let p = g.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
        assert!(p.iter().map(|v| v[1]).fold(0.0f32, f32::max) <= 1.05);
        assert!(p.iter().map(|v| (v[0] * v[0] + v[2] * v[2]).sqrt()).fold(0.0f32, f32::max) < 0.4);
        assert!(grass_colour(0.0, 1.0, 42).iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
