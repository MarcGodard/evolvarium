//! Equal-angle cube-sphere cell grid for every per-cell world field (soil chemistry, ground water, climate,
//! fire, wear, crowding).
//!
//! Replaces the lon/lat SOIL_RES^2 grid, whose cells shrank to slivers at the poles while chem gave every
//! cell the SAME area: a polar sliver got an equatorial cell's NPP and soil stocks. Here cells differ in area
//! by <~1.5x max/min (equal-angle mapping) and `area_m2` is the EXACT solid angle x R^2, so per-cell physics
//! that scales with ground area stays honest at any resolution.
//!
//! Face mapping is `sphere::dir_face_uv`/`face_dir` (gnomonic), shared with the terrain atlas. Cell index
//! `face*n*n + j*n + i`, i/j uniform in ANGLE atan(u), atan(v).
use bevy::prelude::*;
use std::f32::consts::FRAC_PI_4;

/// Field grid cells per face edge. 32 per WORLD_SCALE -> ~13 m^2 (~3.6 m) cells at any planet size.
pub const FIELD_N: usize = (32.0 * crate::sphere::WORLD_SCALE) as usize;
/// Crowding grid: density-dependent grazing was tuned per ~78 m^2 cell (old 32x32 lon/lat mean). 13 -> 1014
/// cells keeps that spatial scale, so finer FIELDS do not silently strengthen or weaken crowding.
pub const CROWD_N: usize = (13.0 * crate::sphere::WORLD_SCALE) as usize;

pub struct CubeGrid {
    pub n: usize,
    center: Vec<Vec3>,
    area: Vec<f64>, // unit-sphere solid angle (sr); x R^2 for m^2
    nbr: Vec<[u32; 4]>,
    edges: Vec<f32>, // n+1 bin edges in gnomonic (tan) space: exact boundaries for the fast bin
}

fn edge(n: usize, k: usize) -> f32 {
    (-FRAC_PI_4 + std::f32::consts::FRAC_PI_2 * k as f32 / n as f32).tan()
}

// Solid angle of gnomonic rectangle [x0,x1]x[y0,y1] on the plane at distance 1 (closed form, exact).
fn rect_solid_angle(x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    let f = |x: f64, y: f64| (x * y / (1.0 + x * x + y * y).sqrt()).atan();
    f(x1, y1) - f(x0, y1) - f(x1, y0) + f(x0, y0)
}

impl CubeGrid {
    fn build(n: usize) -> Self {
        let cells = 6 * n * n;
        let mut center = Vec::with_capacity(cells);
        let mut area = Vec::with_capacity(cells);
        for face in 0..6 {
            for j in 0..n {
                for i in 0..n {
                    let mid = |k: usize| (-FRAC_PI_4 + std::f32::consts::FRAC_PI_2 * (k as f32 + 0.5) / n as f32).tan();
                    center.push(crate::sphere::face_dir(face, mid(i), mid(j)).normalize());
                    area.push(rect_solid_angle(
                        edge(n, i) as f64,
                        edge(n, i + 1) as f64,
                        edge(n, j) as f64,
                        edge(n, j + 1) as f64,
                    ));
                }
            }
        }
        // neighbours: step one cell along the FACE axes (u, v), not east/north (which are diagonal to the grid
        // off the equatorial faces). Past a face edge the extended gnomonic plane still maps to a real dir, and
        // re-binning it lands on the adjacent face's edge cell, so no per-face orientation tables are needed.
        let edges: Vec<f32> = (0..=n).map(|k| edge(n, k)).collect();
        let ang = |k: i32| (-FRAC_PI_4 + std::f32::consts::FRAC_PI_2 * (k as f32 + 0.5) / n as f32).tan();
        let mut nbr = Vec::with_capacity(cells);
        for face in 0..6 {
            for j in 0..n as i32 {
                for i in 0..n as i32 {
                    let at = |ii: i32, jj: i32| Self::bin(n, &edges, crate::sphere::face_dir(face, ang(ii), ang(jj))) as u32;
                    nbr.push([at(i + 1, j), at(i - 1, j), at(i, j + 1), at(i, j - 1)]);
                }
            }
        }
        CubeGrid { n, center, area, nbr, edges }
    }

    // Hot path (every plant/creature/deposit, many times a tick). Polynomial atan guess (|err| < 0.004 rad, under
    // a quarter cell for n <= 96) lands within one bin; exact tan-space edges settle it. No atan call.
    fn bin(n: usize, edges: &[f32], d: Vec3) -> usize {
        let (face, u, v) = crate::sphere::dir_face_uv(d);
        let k = |w: f32| {
            let a = FRAC_PI_4 * w + 0.273 * w * (1.0 - w.abs());
            let mut k = (((a / FRAC_PI_4 + 1.0) * 0.5 * n as f32) as usize).min(n - 1);
            if k > 0 && w < edges[k] {
                k -= 1;
            } else if k + 1 < n && w >= edges[k + 1] {
                k += 1;
            }
            k
        };
        face * n * n + k(v) * n + k(u)
    }

    pub fn len(&self) -> usize {
        self.center.len()
    }
    /// Cell holding surface dir or world pos `p` (any nonzero length). Zero/NaN maps to cell 0.
    pub fn cell(&self, p: Vec3) -> usize {
        if !(p.length_squared() > 1e-12) {
            return 0;
        }
        Self::bin(self.n, &self.edges, p)
    }
    /// Unit surface dir at cell centre.
    pub fn center(&self, c: usize) -> Vec3 {
        self.center[c]
    }
    /// Ground area of cell `c`, m^2, on the PLANET_R sphere.
    pub fn area_m2(&self, c: usize) -> f64 {
        let r = crate::sphere::PLANET_R as f64;
        self.area[c] * r * r
    }
    /// Mean cell area, m^2: total surface over cell count.
    pub fn mean_area_m2(&self) -> f64 {
        let r = crate::sphere::PLANET_R as f64;
        4.0 * std::f64::consts::PI * r * r / self.len() as f64
    }
    /// The 4 edge neighbours (+u, -u, +v, -v on the cell's face; across an edge, the adjacent face's cell).
    pub fn neighbors(&self, c: usize) -> [usize; 4] {
        self.nbr[c].map(|x| x as usize)
    }
    /// Cell `c` plus neighbours-of-neighbours (deduped, first `len` entries valid): the 3x3 block around `c`
    /// plus the 4 cells two steps out along the axes. Covers every point within one cell width of `c`, except
    /// near cube corners where 3 faces meet.
    pub fn ring2(&self, c: usize) -> ([usize; 21], usize) {
        let mut cand = [usize::MAX; 21];
        let mut k = 0;
        let mut push = |x: usize| {
            if !cand[..k].contains(&x) {
                cand[k] = x;
                k += 1;
            }
        };
        push(c);
        for a in self.neighbors(c) {
            push(a);
            for b in self.neighbors(a) {
                push(b);
            }
        }
        (cand, k)
    }
    /// Smooth sample of a per-cell field at dir `d`: compact radial kernel (radius one cell width) over the
    /// 2-ring neighbourhood. Continuous everywhere, face edges included: a cell's weight reaches 0 before it
    /// can leave the candidate set (any centre within one width of `d` is at most a diagonal away from the
    /// containing cell, and diagonals are neighbours-of-neighbours).
    #[cfg(test)] // production callers cache sample_weights (globe vertices never move)
    pub fn sample(&self, field: &[f32], d: Vec3) -> f32 {
        let w = self.sample_weights(d);
        let wsum: f32 = w.iter().map(|x| x.1).sum();
        if wsum > 0.0 { w.iter().map(|&(c, k)| k * field[c as usize]).sum::<f32>() / wsum } else { field[self.cell(d)] }
    }

    /// The (cell, weight) kernel `sample` blends at `d`, unnormalized. For points that never move (globe
    /// vertices) cache this once and blend any number of fields with it.
    pub fn sample_weights(&self, d: Vec3) -> Vec<(u32, f32)> {
        let c = self.cell(d);
        let dn = d.normalize_or_zero();
        let r = std::f32::consts::FRAC_PI_2 / self.n as f32; // < 1 width: near cube corners (3 faces meet) a 2-ring misses some cells within a full width
        let inv_r2 = 1.0 / (r * r);
        let (cand, k) = self.ring2(c);
        let mut out = Vec::with_capacity(k);
        for &x in &cand[..k] {
            let ang2 = 2.0 * (1.0 - self.center[x].dot(dn)).max(0.0); // chord^2 ~ angle^2
            let q = 1.0 - ang2 * inv_r2;
            if q > 0.0 {
                out.push((x as u32, q * q));
            }
        }
        if out.is_empty() {
            out.push((c as u32, 1.0));
        }
        out
    }
}

/// Mean cell area of the retired 32x32 lon/lat grid on the 80 m planet. Point-deposit amounts (wear per footfall,
/// fertility per corpse) were tuned against it; `legacy_area_ratio` carries them to any grid. Fixed m^2, not
/// planet area / 1024: a bigger planet must not inflate every deposit.
const LEGACY_CELL_M2: f64 = 4.0 * std::f64::consts::PI * 80.0 * 80.0 / 1024.0;

/// legacy mean cell area / area of field cell `c`: multiply a per-legacy-cell point deposit by this.
pub fn legacy_area_ratio(c: usize) -> f32 {
    (LEGACY_CELL_M2 / field().area_m2(c)) as f32
}

static FIELD: std::sync::OnceLock<CubeGrid> = std::sync::OnceLock::new();
static CROWD: std::sync::OnceLock<CubeGrid> = std::sync::OnceLock::new();

/// The world field grid (built once per process).
pub fn field() -> &'static CubeGrid {
    FIELD.get_or_init(|| CubeGrid::build(FIELD_N))
}
/// Coarse grid for creature crowding (see CROWD_N).
pub fn crowd() -> &'static CubeGrid {
    CROWD.get_or_init(|| CubeGrid::build(CROWD_N))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_tile_the_sphere_and_stay_near_equal() {
        for g in [field(), crowd()] {
            let total: f64 = (0..g.len()).map(|c| g.area[c]).sum();
            assert!((total - 4.0 * std::f64::consts::PI).abs() < 1e-6, "solid angle sums to {total}");
            let (lo, hi) = (0..g.len()).fold((f64::MAX, 0.0f64), |(lo, hi), c| (lo.min(g.area[c]), hi.max(g.area[c])));
            assert!(hi / lo < 1.5, "area spread {}", hi / lo);
        }
    }

    #[test]
    fn fast_bin_matches_exact_atan_bin() {
        let mut rng = crate::rng::Rng::seed(7);
        for g in [field(), crowd()] {
            let n = g.n;
            let exact = |d: Vec3| {
                let (face, u, v) = crate::sphere::dir_face_uv(d);
                let k = |w: f32| (((w.atan() / FRAC_PI_4 + 1.0) * 0.5 * n as f32) as usize).min(n - 1);
                face * n * n + k(v) * n + k(u)
            };
            let mut off = 0;
            for _ in 0..200_000 {
                let d = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                if g.cell(d) != exact(d) {
                    off += 1; // only f32 ties exactly on an edge may differ
                }
            }
            assert!(off <= 2, "{off} of 200k dirs binned differently at n={n}");
        }
    }

    #[test]
    fn centers_bin_to_their_own_cell() {
        let g = field();
        for c in 0..g.len() {
            assert_eq!(g.cell(g.center(c)), c);
        }
    }

    #[test]
    fn neighbors_are_adjacent_and_distinct_from_self() {
        let g = field();
        let width = std::f32::consts::FRAC_PI_2 / g.n as f32;
        for c in 0..g.len() {
            for nb in g.neighbors(c) {
                assert_ne!(nb, c, "cell {c} lists itself");
                let ang = g.center(c).dot(g.center(nb)).clamp(-1.0, 1.0).acos();
                assert!(ang < 1.6 * width, "cell {c} neighbour {nb} {ang} rad away (width {width})");
                assert!(g.neighbors(nb).contains(&c), "neighbour relation not symmetric: {c} -> {nb}");
            }
            let nb = g.neighbors(c);
            for x in 0..4 {
                for y in x + 1..4 {
                    assert_ne!(nb[x], nb[y], "cell {c} has a repeated neighbour");
                }
            }
        }
    }

    #[test]
    fn sample_is_continuous_across_face_edges() {
        let g = field();
        let field: Vec<f32> = (0..g.len()).map(|c| g.center(c).x + 2.0 * g.center(c).y).collect();
        let mut rng = crate::rng::Rng::seed(3);
        for i in 0..4000 {
            let a = rng.range(-1.0, 1.0);
            // half on a cube edge, half anywhere (interior cell boundaries must be seamless too)
            let d = if i % 2 == 0 { Vec3::new(1.0, 1.0, a) } else { Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)).normalize_or(Vec3::X) };
            let e = 1e-4;
            let lo = g.sample(&field, d + Vec3::new(e, -e, 0.0));
            let hi = g.sample(&field, d + Vec3::new(-e, e, 0.0));
            assert!((lo - hi).abs() < 0.005, "jump {} at {d:?}", (lo - hi).abs());
        }
    }
}
