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
#[cfg(test)]
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

// --- trees -------------------------------------------------------------------------------------
//
// Three meshes make a tree: tree_trunk_mesh (bole + limbs), tree_canopy_mesh (broadleaf crown) or
// conifer_mesh (evergreen). All three are seeded, so a forest is not a row of clones.
//
// LOCAL FRAME (do not change without fixing viz::add_plant_visuals + size_plants): the trunk axis runs
// y = -0.7 (foot, sits on terrain) .. +0.7. Broadleaf crown is origin-centered radius ~1 and is parented
// at y = +1.0; conifer is base-at-y=0 parented at y = -0.6. Fruit/blossom children hang at y 0.45..1.4,
// radius ~0.9, so the crown must stay wide enough to hold them.

// Shared vertex buffer for the tree meshes. Winding is decided per triangle from the vertex normal (see
// `tri`), so the shape closures below can use whatever parametrization handedness reads clearest.
struct Buf {
    pos: Vec<[f32; 3]>,
    nor: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

impl Buf {
    fn new() -> Self {
        Buf { pos: Vec::new(), nor: Vec::new(), uv: Vec::new(), col: Vec::new(), idx: Vec::new() }
    }
    fn len(&self) -> u32 {
        self.pos.len() as u32
    }
    fn vert(&mut self, p: Vec3, n: Vec3, uv: [f32; 2], c: [f32; 3]) {
        let n = n.normalize_or_zero();
        self.pos.push([p.x, p.y, p.z]);
        self.nor.push([n.x, n.y, n.z]);
        self.uv.push(uv);
        self.col.push([c[0], c[1], c[2], 1.0]);
    }
    // Emit with the winding whose geometric normal agrees with vertex normal at `a`. The broadleaf canopy
    // material culls back faces, so a tri wound the wrong way is a hole straight through the crown.
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        let (pa, pb, pc) = (Vec3::from(self.pos[a as usize]), Vec3::from(self.pos[b as usize]), Vec3::from(self.pos[c as usize]));
        let g = (pb - pa).cross(pc - pa);
        if g.dot(Vec3::from(self.nor[a as usize])) >= 0.0 {
            self.idx.extend_from_slice(&[a, b, c]);
        } else {
            self.idx.extend_from_slice(&[a, c, b]);
        }
    }
    // quad corners: a=(row,col) b=(row,col+1) c=(row+1,col) d=(row+1,col+1)
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.tri(a, b, c);
        self.tri(b, d, c);
    }
    fn finish(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nor);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        mesh.insert_indices(Indices::U32(self.idx));
        mesh
    }
}

// Central-difference normal of a parametric surface p(u, th). `outref` only fixes the sign (the cross
// product's direction depends on parametrization handedness); pass any vector roughly pointing out of
// the solid.
fn surf_normal(p: &impl Fn(f32, f32) -> Vec3, u: f32, th: f32, outref: Vec3) -> Vec3 {
    let d = 1.0e-3;
    let tu = p((u + d).min(1.0), th) - p((u - d).max(0.0), th);
    let tt = p(u, th + d) - p(u, th - d);
    let n = tt.cross(tu).normalize_or_zero();
    if n.dot(outref) < 0.0 { -n } else { n }
}

const TRUNK_BOT: f32 = -0.7; // foot; entity transform puts this on the terrain
const TRUNK_TOP: f32 = 0.7;

// Trunk + limbs, seeded. Bole is a swept tube: taper toward the top, exponential ROOT FLARE over the
// bottom ~15% (a straight frustum reads as a dowel), a slight lean, and low-amplitude flutes around the
// circumference so bark catches a rim of light instead of shading like a cylinder.
//
// Limbs fork off the upper bole and curve outward-then-up into the crown volume. They start below the
// crown underside (~y 0.2 once the canopy is parented at +1.0) so the fork is visible from the ground,
// which is most of what separates "tree" from "lollipop" at a distance.
//
// VERTEX COUNT ~ 180 (bole 7x9 grid + 2 caps + 3..4 limbs of 4x6). Hundreds of trees on screen, so this
// stays in the same order as the old frustum+blob pair.
pub fn tree_trunk_mesh(seed: u32) -> Mesh {
    let mut b = Buf::new();
    let lean = Vec3::new(rng(seed, 1, -0.12, 0.12), 0.0, rng(seed, 2, -0.12, 0.12));
    let ph = rng(seed, 3, 0.0, 6.28);
    let flutes = 3.0 + (rnd(seed, 4) * 3.0).floor(); // 3..5 bark ridges
    let axis = |t: f32| Vec3::new(0.0, TRUNK_BOT + (TRUNK_TOP - TRUNK_BOT) * t, 0.0) + lean * t.powf(1.6);
    let rad = |t: f32, th: f32| {
        (0.085 + 0.085 * (1.0 - t).powf(1.4) + 0.175 * (-7.0 * t).exp()) * (1.0 + 0.09 * (flutes * th + ph).sin())
    };
    let bole = |t: f32, th: f32| axis(t) + Vec3::new(th.cos(), 0.0, th.sin()) * rad(t, th);
    // bark tone: flute ridges catch light, crevices and the ground-damp base stay dark
    let bark = |t: f32, th: f32, k: u32| {
        let ridge = 0.5 + 0.5 * (flutes * th + ph).sin();
        let v = 0.58 + 0.50 * ridge - 0.22 * (1.0 - t) + rng(seed, 300 + k, -0.07, 0.07);
        let v = v.clamp(0.42, 1.25);
        [v, v * 0.98, v * 0.94] // faint warm bias: bark reads brown, not gray
    };

    let (rings, sectors) = (6usize, 8usize);
    let start = b.len();
    for ri in 0..=rings {
        let t = (ri as f32 / rings as f32).powf(1.5); // sample dense near the flared foot
        for si in 0..=sectors {
            let th = std::f32::consts::TAU * si as f32 / sectors as f32;
            let p = bole(t, th);
            let n = surf_normal(&bole, t, th, Vec3::new(th.cos(), 0.0, th.sin()));
            b.vert(p, n, [si as f32 / sectors as f32, t], bark(t, th, ri as u32 * 17 + si as u32));
        }
    }
    let cols = (sectors + 1) as u32;
    for ri in 0..rings as u32 {
        for si in 0..sectors as u32 {
            b.quad(start + ri * cols + si, start + ri * cols + si + 1, start + (ri + 1) * cols + si, start + (ri + 1) * cols + si + 1);
        }
    }
    // caps close the shell: bole material culls back faces, so an open end is a see-through hole
    for (t, up) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
        let c = b.len();
        b.vert(axis(t), Vec3::Y * up, [0.5, 0.5], [0.5, 0.49, 0.47]);
        for si in 0..=sectors {
            let th = std::f32::consts::TAU * si as f32 / sectors as f32;
            b.vert(bole(t, th), Vec3::Y * up, [0.5, 0.5], [0.5, 0.49, 0.47]);
        }
        for si in 0..sectors as u32 {
            b.tri(c, c + 1 + si, c + 2 + si);
        }
    }

    let limbs = 3 + (seed % 2) as usize;
    for k in 0..limbs {
        let ki = k as u32;
        let a = k as f32 * 2.39996 + rng(seed, 40 + ki, -0.45, 0.45);
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        let t0 = (0.40 + 0.17 * k as f32 + rng(seed, 60 + ki, -0.05, 0.05)).min(0.93);
        let reach = rng(seed, 80 + ki, 0.30, 0.60);
        let rise = rng(seed, 100 + ki, 0.55, 0.95);
        let r0 = 0.058 * (1.0 - 0.35 * t0);
        // outward first, up later: u^0.75 spends the reach early, u^1.5 the rise late -> an arching limb,
        // not a diagonal spar
        let curve = |u: f32| axis(t0) + dir * (reach * u.powf(0.75)) + Vec3::Y * (rise * u.powf(1.5));
        let (segs, sec) = (3usize, 5usize);
        let lstart = b.len();
        for si in 0..=segs {
            let u = si as f32 / segs as f32;
            let tan = (curve((u + 0.05).min(1.0)) - curve((u - 0.05).max(0.0))).normalize_or_zero();
            let side = tan.cross(Vec3::Y).normalize_or_zero();
            let upn = side.cross(tan).normalize_or_zero();
            let r = r0 * (1.0 - u).powf(0.8) + 0.008;
            let c = curve(u);
            for sj in 0..=sec {
                let th = std::f32::consts::TAU * sj as f32 / sec as f32;
                let n = side * th.cos() + upn * th.sin();
                let v = (0.72 + 0.38 * u + rng(seed, 500 + ki * 31 + sj as u32, -0.06, 0.06)).clamp(0.5, 1.25);
                b.vert(c + n * r, n, [sj as f32 / sec as f32, u], [v, v * 0.98, v * 0.94]);
            }
        }
        let lc = (sec + 1) as u32;
        for si in 0..segs as u32 {
            for sj in 0..sec as u32 {
                b.quad(lstart + si * lc + sj, lstart + si * lc + sj + 1, lstart + (si + 1) * lc + sj, lstart + (si + 1) * lc + sj + 1);
            }
        }
    }
    b.finish()
}

// Broadleaf crown: `lobes` overlapping irregular ELLIPSOID clusters filling a crown of `radius`, centered
// on the origin.
//
// VERTEX COUNT = lobes * 35 (rings 4 x sectors 6). lobes=8 -> 280 verts.
//
// MANY SMALL LOBES beat few big ones: at 6 sectors a lobe silhouette is a hexagon, so a crown of 4 fat
// lobes reads as cut gems, while 8 small ones union into a bumpy foliage edge. Lobes are wider than tall
// and the core lobe is sunk, so the crown spreads instead of balling up. Two trig frequencies dent each
// lobe.
//
// Vertex color carries three signals a single flat green cannot: downward normals = self-shaded
// underside, low in the crown = shade, near the crown center = interior AO. Variation is mostly SMOOTH
// trig; hash noise stays small because on a 35-vert lobe a hash lands as a blotch the size of a face.
pub fn tree_canopy_mesh(lobes: usize, radius: f32, seed: u32) -> Mesh {
    let lobes = lobes.max(1);
    let (rings, sectors) = (4usize, 6usize);
    let mut b = Buf::new();

    let deep = [0.22f32, 0.30, 0.22]; // shaded interior/underside
    let lit = [1.22f32, 1.16, 0.74]; // sunlit top, yellowed

    for l in 0..lobes {
        let li = l as u32;
        // lobe 0 = core fill, dropped slightly and flattened; the rest ride a golden-angle spiral whose
        // reach grows with index, so lobes land at mixed depths instead of one shell
        let (center, sc) = if l == 0 {
            (Vec3::new(0.0, -0.10 * radius, 0.0), Vec3::new(0.72, 0.56, 0.72) * radius)
        } else {
            let f = l as f32 / (lobes - 1).max(1) as f32;
            let a = l as f32 * 2.39996 + rng(seed, li * 9 + 1, -0.3, 0.3);
            // reach shrinks as the lobe rides higher: a constant reach builds a flat-topped broccoli cap,
            // the taper builds a dome. Outer lobes (high f) also sag, so the crown skirt droops toward the
            // limb tips instead of ending in a flat brim.
            let y = radius * (rng(seed, li * 9 + 3, -0.12, 0.70) - 0.42 * f);
            let off = radius * (0.26 + 0.44 * f) * rng(seed, li * 9 + 2, 0.85, 1.15) * (1.0 - 0.45 * (y / radius).max(0.0));
            let s = Vec3::new(rng(seed, li * 9 + 4, 0.34, 0.50), rng(seed, li * 9 + 5, 0.28, 0.44), rng(seed, li * 9 + 6, 0.34, 0.50));
            (Vec3::new(a.cos() * off, y, a.sin() * off), s * radius)
        };
        let ph = rng(seed, li * 9 + 7, 0.0, 6.28); // noise phase, keeps lobes from denting identically
        let shade = rng(seed, li * 9 + 8, 0.86, 1.12); // per-lobe brightness
        let start = b.len();
        for ri in 0..=rings {
            let phi = std::f32::consts::PI * ri as f32 / rings as f32;
            let (sp, cp) = phi.sin_cos();
            for si in 0..=sectors {
                let th = std::f32::consts::TAU * si as f32 / sectors as f32;
                let (st, ct) = th.sin_cos();
                let d = Vec3::new(sp * ct, cp, sp * st);
                let bump = 1.0 + 0.14 * ((3.0 * phi + ph).sin() * (2.0 * th + ph * 1.7).cos()) + 0.08 * ((5.0 * th + ph * 2.3).sin() * (2.0 * phi).sin());
                let p = center + Vec3::new(d.x * sc.x, d.y * sc.y, d.z * sc.z) * bump;
                let n = Vec3::new(d.x / sc.x, d.y / sc.y, d.z / sc.z); // ellipsoid normal, not the sphere direction
                let up01 = 0.5 + 0.5 * n.normalize_or_zero().y;
                let hi01 = ((p.y / radius) * 0.5 + 0.5).clamp(0.0, 1.0);
                let out01 = (p.length() / (1.15 * radius)).clamp(0.0, 1.0); // buried verts sit in shade
                // dapple frequency stays UNDER the sector count: 4*th over 6 sectors aliases into per-vertex
                // noise, which on a 35-vert lobe lands as a dark blotch the size of a face
                let dapple = 0.5 + 0.5 * (2.0 * th + ph).sin() * (2.0 * phi + ph * 0.6).cos();
                let v = ((0.34 * up01 + 0.24 * hi01 + 0.24 * out01 + 0.10 * dapple + 0.08 * rnd(seed, li * 97 + ri as u32 * 13 + si as u32)) * shade).clamp(0.0, 1.05);
                b.vert(p, n, [si as f32 / sectors as f32, ri as f32 / rings as f32], lerp3(deep, lit, v));
            }
        }
        let cols = (sectors + 1) as u32;
        for ri in 0..rings as u32 {
            for si in 0..sectors as u32 {
                b.quad(start + ri * cols + si, start + ri * cols + si + 1, start + (ri + 1) * cols + si, start + (ri + 1) * cols + si + 1);
            }
        }
    }
    b.finish()
}

// Conifer, seeded. Base at y=0, apex ~2.5, widest ~1.5: same envelope the stacked-cone version had, so
// the y=-0.6 parent offset in add_plant_visuals still lands the skirt on the trunk foot.
//
// Each tier is a DROOPING SCALLOPED skirt, not a cone: radius is modulated around the circumference
// (branch whorls) and the rim curves down past the straight-cone line, which is what gives a fir its
// ragged edge. Tier phases differ so the scallops do not stack into vertical ribs.
//
// Rendered with no_cull_mat (see viz.rs): the skirts are open shells, and a double-sided material would
// negate the back-face normal and drop the whole tree to ambient-only black.
//
// VERTEX COUNT ~ 290 (5 tiers of 3x17 + spine).
pub fn conifer_mesh(seed: u32) -> Mesh {
    let mut b = Buf::new();
    // spine: narrow tapered core so the axis is never see-through between skirts
    {
        let (rows, sec) = (3usize, 8usize);
        let spine = |u: f32, th: f32| Vec3::new(th.cos(), 0.0, th.sin()) * (0.28 * (1.0 - u).powf(0.9) + 0.035) + Vec3::Y * (0.05 + 2.40 * u);
        let start = b.len();
        for ri in 0..=rows {
            let u = ri as f32 / rows as f32;
            for si in 0..=sec {
                let th = std::f32::consts::TAU * si as f32 / sec as f32;
                let v = 0.42 + 0.20 * u;
                b.vert(spine(u, th), surf_normal(&spine, u, th, Vec3::new(th.cos(), 0.0, th.sin())), [0.0, u], [v, v, v]);
            }
        }
        let cols = (sec + 1) as u32;
        for ri in 0..rows as u32 {
            for si in 0..sec as u32 {
                b.quad(start + ri * cols + si, start + ri * cols + si + 1, start + (ri + 1) * cols + si, start + (ri + 1) * cols + si + 1);
            }
        }
    }
    let tiers = 5usize;
    for i in 0..tiers {
        let ii = i as u32;
        // stack offsets keep the LOWEST rim at y >= 0 once droop is subtracted: the mesh is parented 0.1
        // above the trunk foot, so a rim below 0 sinks into the terrain
        let top_y = 0.90 + 0.42 * i as f32;
        let h = 0.62 - 0.05 * i as f32;
        let big = 0.22 + 1.28 * (1.0 - i as f32 / tiers as f32).powf(0.9); // 1.5 at the ground tier
        let droop = rng(seed, 10 + ii, 0.14, 0.26);
        let whorls = 5.0 + (rnd(seed, 30 + ii) * 2.0).floor(); // 5..6 branch bulges; sectors=16 resolves that
        let tph = rng(seed, 50 + ii, 0.0, 6.28);
        // two incommensurate frequencies: a single cos gives an evenly-lobed flower, real branch tips are
        // uneven. Amplitude is what makes the rim ragged at distance, which is the whole fir read.
        let scallop = move |th: f32| 0.70 + 0.22 * (whorls * th + tph).cos() + 0.10 * (2.0 * whorls * th + tph * 1.7).sin();
        // u = 0 small ring at the tier top (hidden by the tier above), u = 1 the drooping rim
        let skirt = move |u: f32, th: f32| {
            let uu = 0.18 + 0.82 * u;
            Vec3::new(th.cos(), 0.0, th.sin()) * (big * uu.powf(0.75) * scallop(th)) + Vec3::Y * (top_y - h * uu - droop * uu.powi(3))
        };
        let (rows, sec) = (2usize, 16usize);
        let start = b.len();
        for ri in 0..=rows {
            let u = ri as f32 / rows as f32;
            for si in 0..=sec {
                let th = std::f32::consts::TAU * si as f32 / sec as f32;
                let p = skirt(u, th);
                let n = surf_normal(&skirt, u, th, Vec3::new(th.cos(), 0.0, th.sin()) * 0.5 + Vec3::Y);
                // needle tips (rim + scallop crests) catch sun; the tier's inner shelf stays dark
                let v = (0.58 + 0.40 * u + 0.55 * (scallop(th) - 0.70) * u + rng(seed, 700 + ii * 41 + si as u32, -0.06, 0.06)).clamp(0.45, 1.25);
                b.vert(p, n, [si as f32 / sec as f32, u], [v * 0.92, v, v * 0.88]);
            }
        }
        let cols = (sec + 1) as u32;
        for ri in 0..rows as u32 {
            for si in 0..sec as u32 {
                b.quad(start + ri * cols + si, start + ri * cols + si + 1, start + (ri + 1) * cols + si, start + (ri + 1) * cols + si + 1);
            }
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    // guards the stated vertex budgets + index bounds; grass is instanced GRASS_CAP times so a silent
    // budget blowup here is a frame-time regression
    #[test]
    fn flora_meshes_are_within_budget() {
        for (m, expect) in [(grass_blade_mesh(14, 7), 14 * 7), (grass_blade_mesh(1, 1), 7), (tree_canopy_mesh(8, 1.0, 3), 8 * 35)] {
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

    // trunk/conifer are per-tree meshes drawn in the hundreds: guard the budget, the index bounds, and
    // the local frame add_plant_visuals + size_plants hard-code (trunk foot at -0.7, conifer base at 0).
    #[test]
    fn tree_meshes_are_within_budget_and_frame() {
        for s in [0u32, 1, 977, 4242] {
            let trunk = tree_trunk_mesh(s);
            let cone = conifer_mesh(s);
            for (m, cap) in [(&trunk, 220usize), (&cone, 340)] {
                let n = m.count_vertices();
                assert!(n <= cap, "vert budget: {n} > {cap}");
                let Some(Indices::U32(ix)) = m.indices() else { panic!("want u32 indices") };
                assert_eq!(ix.len() % 3, 0);
                assert!(ix.iter().all(|&i| (i as usize) < n));
                assert!(m.attribute(Mesh::ATTRIBUTE_NORMAL).is_some() && m.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
            }
            let tp = trunk.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
            let (lo, hi) = tp.iter().fold((9.0f32, -9.0f32), |(l, h), v| (l.min(v[1]), h.max(v[1])));
            assert!((lo - TRUNK_BOT).abs() < 1.0e-4, "trunk foot moved: {lo}");
            assert!(hi > TRUNK_TOP && hi < 1.7, "limbs must reach the crown without overshooting it: {hi}");
            let cp = cone.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
            let cy = cp.iter().fold((9.0f32, -9.0f32), |(l, h), v| (l.min(v[1]), h.max(v[1])));
            assert!(cy.0 > -0.1 && cy.0 < 0.15, "conifer base off y=0: {}", cy.0);
            assert!(cy.1 > 2.2 && cy.1 < 2.8, "conifer height drifted: {}", cy.1);
        }
    }
}
