// Generative morphology: Karl-Sims directed PART-GRAPH genome. A compact graph (nodes = parametric parts,
// edges = attachments w/ recursion + reflection) GROWS an open-ended body. Recursion -> segmented chains /
// multi-bone limbs; reflection -> bilateral pairs for free. So a few bytes -> worm, crab, six-legged browser,
// or non-Earth shapes. Costs derive from the developed geometry (mass/area/reach), NOT hand-tuned per gene.
//
// PURE module (no ECS systems): develop() + Morphometrics shared by render (viz) + sim (derived stats) +
// later the physics gym. Long axis of every part = local +Y (base at origin, extends +Y by `length`).
use crate::rng::Rng;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const MAX_PARTS: usize = 32; // developed-part ceiling: bounds mesh + metric cost regardless of graph
pub const MAX_NODES: usize = 8; // graph node-template ceiling (mutation bound)
pub const MAX_EDGES: usize = 10; // graph edge ceiling (mutation bound)

// Parametric primitive. Segment = tapered tube (body/limb/neck/tail). Sphere = head/blob. Plate = flat
// fin/wing/fan (thin in local Z, spans Y x X). Shape only sets the MESH + volume; placement is the edge's job.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum ShapeKind {
    Segment,
    Sphere,
    Plate,
}
impl Default for ShapeKind {
    fn default() -> Self {
        ShapeKind::Segment
    }
}

// Joint to parent. Fixed in P1 (rigid body for render + derived stats); Hinge/Universal get actuated in the
// P2 physics gym (motor drives the angle, creature learns gaits).
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum JointKind {
    Fixed,
    Hinge,
    Universal,
}
impl Default for JointKind {
    fn default() -> Self {
        JointKind::Fixed
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub struct JointSpec {
    pub kind: JointKind,
    pub axis: u8,       // 0=x 1=y 2=z local hinge axis
    pub lo: f32,        // angle limit lo (rad)
    pub hi: f32,        // angle limit hi (rad)
    pub motor: f32,     // 0..1 actuator strength (P2 gym)
}
impl Default for JointSpec {
    fn default() -> Self {
        JointSpec { kind: JointKind::Fixed, axis: 0, lo: -0.6, hi: 0.6, motor: 0.0 }
    }
}

// A part TEMPLATE (referenced by node index; one template can be reused by many edges + recursions).
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PartGene {
    pub shape: ShapeKind,
    pub length: f32, // extent along local +Y
    pub radius: f32, // half-thickness (Segment/Sphere radius; Plate half-width in X)
    pub taper: f32,  // tip radius fraction 0..1 (1 = no taper, 0 = point)
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

const PLATE_THICK: f32 = 0.18; // plate Z-thickness as a fraction of its half-width (visual + volume)

// One attachment in the graph: child `to` hangs off parent `from`. recursion + reflection live here.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct EdgeGene {
    pub from: usize,   // parent node index
    pub to: usize,     // child node index (may equal from -> self-chain)
    pub along: f32,    // 0..1 attach height up the parent's +Y axis
    pub around: f32,   // angle around parent +Y (rad) -> which side
    pub pitch: f32,    // child axis tilt: 0 = straight out radial, + = up toward parent tip, - = down
    pub roll: f32,     // child spin about its own axis
    pub scale: f32,    // child size mult, compounding per recursion (limbs taper down a chain)
    pub reflect: bool, // also emit a bilaterally-mirrored copy (the OTHER side)
    pub recurse: u8,   // times this edge is followed along a path (0 = inactive, N = chain of N)
    pub joint: JointSpec,
}

// Whole body: node templates + edges + root index.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BodyGraph {
    pub nodes: Vec<PartGene>,
    pub edges: Vec<EdgeGene>,
    pub root: usize,
}

// A developed part placed in body-local space (root at origin, +Y up, +Z forward). tf has unit scale: size
// lives in `length`/`radius` (already scale-baked) so meshes stay clean + metrics read true dimensions.
#[derive(Clone, Debug)]
pub struct PlacedPart {
    pub tf: Transform,
    pub shape: ShapeKind,
    pub length: f32,
    pub radius: f32,
    pub taper: f32,
    pub color: [f32; 3],
    pub joint: JointSpec,
    pub depth: u8, // graph depth (0 = root); render/metrics hints
    pub parent: Option<usize>, // index into Phenotype.parts of the part this hangs off (None = root). Gym joints.
}

pub struct Phenotype {
    pub parts: Vec<PlacedPart>,
}

// Default body = one capsule-equivalent Segment, no edges. Old saves (no `body` field) develop to this, so
// they render + behave like today's single-capsule creature. Founders/evolved override with real graphs.
pub fn default_body() -> BodyGraph {
    BodyGraph {
        nodes: vec![PartGene { shape: ShapeKind::Segment, length: 1.4, radius: 0.5, taper: 0.85, r: 0.6, g: 0.6, b: 0.62 }],
        edges: vec![],
        root: 0,
    }
}

// Child base transform RELATIVE to the parent's local frame. Uses the parent's ALREADY-SCALED length/radius
// so the attach point sits on the parent surface. chirality (+/-1) mirrors lateral angles for reflected sides.
fn child_local(parent_len: f32, parent_radius: f32, parent_taper: f32, e: &EdgeGene, chirality: f32) -> Transform {
    let around = e.around * chirality;
    let r_at = parent_radius * (1.0 - (1.0 - parent_taper.clamp(0.0, 1.0)) * e.along.clamp(0.0, 1.0));
    // `around` measured from FORWARD (+Z); X = lateral. chirality flips X -> bilateral (left/right) pairs.
    let radial = Vec3::new(around.sin(), 0.0, around.cos());
    let pos = Vec3::Y * (e.along.clamp(0.0, 1.0) * parent_len) + radial * r_at;
    // child +Y axis points outward (radial) tilted by pitch toward parent +Y
    let dir = (radial * e.pitch.cos() + Vec3::Y * e.pitch.sin()).normalize_or_zero();
    let dir = if dir.length_squared() > 1e-6 { dir } else { radial };
    let rot = Quat::from_rotation_arc(Vec3::Y, dir) * Quat::from_rotation_y(e.roll * chirality);
    Transform { translation: pos, rotation: rot, scale: Vec3::ONE }
}

// Grow the graph into placed parts. DFS; per-edge recursion budget decremented along each path (cloned per
// branch so siblings get independent budgets); global MAX_PARTS hard cap. Reflected edges emit a mirrored
// sibling subtree (chirality flips, propagating symmetry down the limb).
pub fn develop(g: &BodyGraph) -> Phenotype {
    let mut parts = Vec::new();
    if g.nodes.is_empty() {
        return Phenotype { parts };
    }
    let root = g.root.min(g.nodes.len() - 1);
    let remaining: Vec<u8> = g.edges.iter().map(|e| e.recurse).collect();
    expand(g, root, Transform::IDENTITY, 1.0, 1.0, 0, None, &remaining, &mut parts);
    Phenotype { parts }
}

#[allow(clippy::too_many_arguments)]
fn expand(g: &BodyGraph, node: usize, world: Transform, scale: f32, chirality: f32, depth: u8, parent: Option<usize>, remaining: &[u8], parts: &mut Vec<PlacedPart>) {
    if parts.len() >= MAX_PARTS || node >= g.nodes.len() {
        return;
    }
    let n = &g.nodes[node];
    let my_idx = parts.len();
    parts.push(PlacedPart {
        tf: world,
        shape: n.shape,
        length: (n.length * scale).max(0.02),
        radius: (n.radius * scale).max(0.02),
        taper: n.taper.clamp(0.05, 1.0),
        color: [n.r, n.g, n.b],
        joint: JointSpec::default(), // overwritten below for non-root by the incoming edge's joint
        depth,
        parent,
    });
    let plen = (n.length * scale).max(0.02);
    let prad = (n.radius * scale).max(0.02);
    for (ei, e) in g.edges.iter().enumerate() {
        if e.from != node || remaining[ei] == 0 || e.to >= g.nodes.len() {
            continue;
        }
        if parts.len() >= MAX_PARTS {
            break;
        }
        let mut rem2 = remaining.to_vec();
        rem2[ei] -= 1;
        let signs: &[f32] = if e.reflect { &[1.0, -1.0] } else { &[1.0] };
        for &s in signs {
            if parts.len() >= MAX_PARTS {
                break;
            }
            let ch = chirality * s;
            let cworld = world.mul_transform(child_local(plen, prad, n.taper, e, ch));
            let before = parts.len();
            expand(g, e.to, cworld, scale * e.scale.clamp(0.2, 1.0), ch, depth.saturating_add(1), Some(my_idx), &rem2, parts);
            if before < parts.len() {
                parts[before].joint = e.joint; // the child just placed carries its incoming joint
            }
        }
    }
}

// Geometry-derived stats (the honest costs/benefits). Computed ONCE at spawn/mutation, cached as a component.
// All approximate (this is a stylized sim, not CFD): good enough to make shape pay rent.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // plan_area/limb_count/limb_length: computed stats reserved for the P2 gym + audits
pub struct Morphometrics {
    pub mass: f32,          // Σ part volume (basal + move cost, energy store)
    pub part_count: u32,    // tissue upkeep
    pub reach: f32,         // max +Y extent (browse height / neck)
    pub frontal_area: f32,  // X-width x Y-height (drag facing forward)
    pub plan_area: f32,     // X-width x Z-length (footprint)
    pub wing_area: f32,     // ~horizontal plate area (lift)
    pub fin_area: f32,      // ~vertical/sagittal plate area (swim thrust + steering)
    pub limb_count: u32,    // downward-reaching segments (legs)
    pub limb_length: f32,   // mean leg length
    pub ground_contacts: u32, // leg tips near the lowest point (stance)
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

impl Morphometrics {
    pub fn of(g: &BodyGraph) -> Self {
        Self::from_phenotype(&develop(g))
    }

    pub fn from_phenotype(p: &Phenotype) -> Self {
        let mut mass = 0.0;
        let mut wing_area = 0.0;
        let mut fin_area = 0.0;
        let mut limb_count = 0u32;
        let mut limb_len_sum = 0.0;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut lowest = f32::INFINITY;
        // first pass: bbox + aggregates; track each part's tip-y for the stance pass
        let mut tip_ys: Vec<f32> = Vec::with_capacity(p.parts.len());
        for part in &p.parts {
            let base = part.tf.translation;
            let tip = part.tf.transform_point(Vec3::Y * part.length);
            tip_ys.push(tip.y);
            for v in [base, tip] {
                min = min.min(v - Vec3::splat(part.radius));
                max = max.max(v + Vec3::splat(part.radius));
            }
            lowest = lowest.min(base.y.min(tip.y));
            // volume -> mass. (scale already baked into length/radius)
            mass += part_volume(part);
            match part.shape {
                ShapeKind::Plate => {
                    let area = part.length * 2.0 * part.radius;
                    let normal = (part.tf.rotation * Vec3::Z).normalize_or_zero();
                    if normal.y.abs() > 0.6 {
                        wing_area += area; // roughly horizontal -> lift surface
                    } else {
                        fin_area += area; // roughly vertical -> swim/steer surface
                    }
                }
                ShapeKind::Segment => {
                    // a "leg" = a non-root segment whose tip points below its base (downward reach)
                    if part.depth > 0 && tip.y < base.y - 0.05 {
                        limb_count += 1;
                        limb_len_sum += part.length;
                    }
                }
                ShapeKind::Sphere => {}
            }
        }
        if !min.is_finite() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        // stance: leg-ish tips within a small band of the lowest point = ground contacts
        let ground_contacts = tip_ys.iter().filter(|&&y| y <= lowest + 0.25).count() as u32;
        let dim = (max - min).max(Vec3::ZERO);
        Morphometrics {
            mass: mass.max(1e-3),
            part_count: p.parts.len() as u32,
            reach: max.y.max(0.0),
            frontal_area: (dim.x * dim.y).max(1e-3),
            plan_area: (dim.x * dim.z).max(1e-3),
            wing_area,
            fin_area,
            limb_count,
            limb_length: if limb_count > 0 { limb_len_sum / limb_count as f32 } else { 0.0 },
            ground_contacts,
            bbox_min: min,
            bbox_max: max,
        }
    }
}

fn part_volume(p: &PlacedPart) -> f32 {
    let l = p.length.max(0.0);
    let r = p.radius.max(0.0);
    match p.shape {
        ShapeKind::Segment => {
            let rm = r * (1.0 + p.taper) * 0.5;
            std::f32::consts::PI * rm * rm * l
        }
        ShapeKind::Sphere => 4.0 / 3.0 * std::f32::consts::PI * r * r * r,
        ShapeKind::Plate => l * (2.0 * r) * (PLATE_THICK * r),
    }
}

// ---- founder + mutation (heritable, mirrors genome.rs scalar+structural pattern) ----

impl BodyGraph {
    // A varied-but-viable founder body: torso root + head + a reflected appendage set (legs/fins/wings) +
    // optional tail. Random params span body plans so selection has morphological variation from gen 0.
    pub fn random(rng: &mut Rng) -> Self {
        // part colors are per-part SHADES (grey multipliers); the genome hue comes from the entity material,
        // so vertex color just darkens/lightens each part (belly lighter, limbs darker, etc.).
        let torso_sh = rng.range(0.85, 1.0);
        let head_sh = rng.range(0.78, 0.95);
        let app_sh = rng.range(0.7, 0.92);
        let mut nodes = vec![
            // 0: torso
            PartGene { shape: ShapeKind::Segment, length: rng.range(1.0, 2.2), radius: rng.range(0.4, 0.75), taper: rng.range(0.6, 0.95), r: torso_sh, g: torso_sh, b: torso_sh },
            // 1: head
            PartGene { shape: ShapeKind::Sphere, length: rng.range(0.4, 0.8), radius: rng.range(0.3, 0.55), taper: 1.0, r: head_sh, g: head_sh, b: head_sh },
        ];
        let mut edges = vec![
            // head on the front-top of the torso
            EdgeGene { from: 0, to: 1, along: rng.range(0.8, 1.0), around: 0.0, pitch: rng.range(0.2, 0.9), roll: 0.0, scale: 1.0, reflect: false, recurse: 1, joint: JointSpec::default() },
        ];
        // appendage template (legs / fins / wings) chosen by a body-plan roll
        let plan = rng.f32();
        let (shape, pitch, count_along) = if plan < 0.45 {
            (ShapeKind::Segment, rng.range(-1.3, -0.5), true) // legs (point down/out)
        } else if plan < 0.75 {
            (ShapeKind::Plate, rng.range(-0.2, 0.2), false) // fins (lateral plates)
        } else {
            (ShapeKind::Plate, rng.range(0.3, 0.9), false) // wings (upper plates)
        };
        let app = nodes.len();
        nodes.push(PartGene { shape, length: rng.range(0.6, 1.4), radius: rng.range(0.12, 0.4), taper: rng.range(0.4, 0.9), r: app_sh, g: app_sh, b: app_sh });
        let pairs = 1 + (rng.f32() * 2.5) as usize; // 1..3 reflected pairs
        for k in 0..pairs {
            let along = if count_along { 0.2 + 0.6 * (k as f32 / pairs.max(1) as f32) } else { rng.range(0.3, 0.7) };
            edges.push(EdgeGene {
                from: 0,
                to: app,
                along,
                around: rng.range(0.6, 1.5),
                pitch,
                roll: 0.0,
                scale: rng.range(0.7, 0.95),
                reflect: true,
                recurse: 1 + (rng.f32() * 2.0) as u8, // 1..2 -> jointed limb chain
                joint: JointSpec { kind: JointKind::Hinge, axis: 0, lo: -0.8, hi: 0.8, motor: rng.f32() },
            });
        }
        // optional tail
        if rng.f32() < 0.5 {
            let tail = nodes.len();
            nodes.push(PartGene { shape: ShapeKind::Segment, length: rng.range(0.5, 1.5), radius: rng.range(0.1, 0.3), taper: rng.range(0.2, 0.6), r: app_sh, g: app_sh, b: app_sh });
            edges.push(EdgeGene { from: 0, to: tail, along: 0.0, around: std::f32::consts::PI, pitch: rng.range(-0.3, 0.3), roll: 0.0, scale: 0.9, reflect: false, recurse: 1 + (rng.f32() * 2.0) as u8, joint: JointSpec::default() });
        }
        BodyGraph { nodes, edges, root: 0 }
    }

    // Drift part params, edge params, and (rarely) topology. rate/std mirror genome.mutate scalars.
    pub fn mutate(&mut self, rng: &mut Rng, rate: f32) {
        for n in &mut self.nodes {
            if rng.f32() < rate {
                n.length = (n.length + rng.normal() * 0.15).clamp(0.15, 3.0);
            }
            if rng.f32() < rate {
                n.radius = (n.radius + rng.normal() * 0.08).clamp(0.05, 1.2);
            }
            if rng.f32() < rate {
                n.taper = (n.taper + rng.normal() * 0.1).clamp(0.05, 1.0);
            }
            if rng.f32() < rate {
                n.r = (n.r + rng.normal() * 0.08).clamp(0.0, 1.0);
                n.g = (n.g + rng.normal() * 0.08).clamp(0.0, 1.0);
                n.b = (n.b + rng.normal() * 0.08).clamp(0.0, 1.0);
            }
            if rng.f32() < rate * 0.5 {
                n.shape = match (rng.f32() * 3.0) as u8 {
                    0 => ShapeKind::Segment,
                    1 => ShapeKind::Sphere,
                    _ => ShapeKind::Plate,
                };
            }
        }
        for e in &mut self.edges {
            if rng.f32() < rate {
                e.along = (e.along + rng.normal() * 0.1).clamp(0.0, 1.0);
            }
            if rng.f32() < rate {
                e.around = e.around + rng.normal() * 0.3;
            }
            if rng.f32() < rate {
                e.pitch = (e.pitch + rng.normal() * 0.2).clamp(-1.5, 1.5);
            }
            if rng.f32() < rate {
                e.roll = e.roll + rng.normal() * 0.2;
            }
            if rng.f32() < rate {
                e.scale = (e.scale + rng.normal() * 0.08).clamp(0.2, 1.0);
            }
            if rng.f32() < rate * 0.4 {
                e.reflect = !e.reflect;
            }
            if rng.f32() < rate * 0.4 {
                e.recurse = (e.recurse as i32 + if rng.f32() < 0.5 { -1 } else { 1 }).clamp(0, 5) as u8;
            }
        }
        // structural: add a new appendage (node + reflected edge off the torso)
        if rng.f32() < 0.04 && self.nodes.len() < MAX_NODES && self.edges.len() < MAX_EDGES {
            let idx = self.nodes.len();
            let base = &self.nodes[0];
            self.nodes.push(PartGene {
                shape: if rng.f32() < 0.5 { ShapeKind::Segment } else { ShapeKind::Plate },
                length: rng.range(0.4, 1.2),
                radius: rng.range(0.1, 0.35),
                taper: rng.range(0.3, 0.9),
                r: base.r,
                g: base.g,
                b: base.b,
            });
            self.edges.push(EdgeGene {
                from: 0,
                to: idx,
                along: rng.range(0.2, 0.8),
                around: rng.range(0.4, 1.6),
                pitch: rng.range(-1.0, 0.8),
                roll: 0.0,
                scale: rng.range(0.7, 0.95),
                reflect: rng.f32() < 0.7,
                recurse: 1 + (rng.f32() * 2.0) as u8,
                joint: JointSpec { kind: JointKind::Hinge, axis: 0, lo: -0.8, hi: 0.8, motor: rng.f32() },
            });
        }
        // structural: drop a random non-head edge (keep root + at least the head edge)
        if rng.f32() < 0.03 && self.edges.len() > 1 {
            let i = 1 + (rng.f32() * (self.edges.len() - 1) as f32) as usize;
            if i < self.edges.len() {
                self.edges.remove(i);
            }
        }
    }
}

// ---- generative mesh: one merged, vertex-colored mesh per developed body ----

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};

const RADIAL: usize = 8; // radial segments for tubes/spheres (low = cheap; bodies are small on screen)

struct MeshBuf {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u32>,
    y_off: f32, // shift all verts down by this so the body is vertically centered on the entity origin
}
impl MeshBuf {
    fn new() -> Self {
        MeshBuf { pos: vec![], nrm: vec![], col: vec![], idx: vec![], y_off: 0.0 }
    }
    fn vert(&mut self, tf: &Transform, p: Vec3, n: Vec3, c: [f32; 4]) -> u32 {
        let mut wp = tf.transform_point(p);
        wp.y -= self.y_off;
        let wn = (tf.rotation * n).normalize_or_zero();
        let i = self.pos.len() as u32;
        self.pos.push([wp.x, wp.y, wp.z]);
        self.nrm.push([wn.x, wn.y, wn.z]);
        self.col.push(c);
        i
    }
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend_from_slice(&[a, b, c]);
    }
}

// Build ONE merged mesh for a whole developed body. Each part emits a parametric primitive in its local
// frame (base at origin, +Y axis), transformed by its placement, with the part color as vertex color (the
// StandardMaterial base_color multiplies it -> genome-level tints still apply).
pub fn build_body_mesh(p: &Phenotype, center_y: f32) -> Mesh {
    let mut b = MeshBuf::new();
    b.y_off = center_y;
    for part in &p.parts {
        let c = [part.color[0], part.color[1], part.color[2], 1.0];
        match part.shape {
            ShapeKind::Segment => push_frustum(&mut b, &part.tf, part.radius, part.radius * part.taper, part.length, c),
            ShapeKind::Sphere => push_sphere(&mut b, &part.tf, part.radius, part.length, c),
            ShapeKind::Plate => push_plate(&mut b, &part.tf, part.radius, part.length, c),
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, b.pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, b.nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, b.col);
    mesh.insert_indices(Indices::U32(b.idx));
    mesh
}

// Tapered tube along +Y from y=0 (radius r0) to y=len (radius r1), with end caps. Side normals ~radial.
fn push_frustum(b: &mut MeshBuf, tf: &Transform, r0: f32, r1: f32, len: f32, c: [f32; 4]) {
    let tau = std::f32::consts::TAU;
    let mut ring0 = [0u32; RADIAL];
    let mut ring1 = [0u32; RADIAL];
    for k in 0..RADIAL {
        let a = tau * k as f32 / RADIAL as f32;
        let (s, co) = a.sin_cos();
        let n = Vec3::new(s, 0.0, co);
        ring0[k] = b.vert(tf, Vec3::new(s * r0, 0.0, co * r0), n, c);
        ring1[k] = b.vert(tf, Vec3::new(s * r1, len, co * r1), n, c);
    }
    for k in 0..RADIAL {
        let kn = (k + 1) % RADIAL;
        b.tri(ring0[k], ring0[kn], ring1[kn]);
        b.tri(ring0[k], ring1[kn], ring1[k]);
    }
    // caps (fans)
    let cap0 = b.vert(tf, Vec3::ZERO, Vec3::NEG_Y, c);
    let cap1 = b.vert(tf, Vec3::Y * len, Vec3::Y, c);
    for k in 0..RADIAL {
        let kn = (k + 1) % RADIAL;
        b.tri(cap0, ring0[kn], ring0[k]);
        b.tri(cap1, ring1[k], ring1[kn]);
    }
}

// UV sphere of radius r, centered at y=r (rests on the attach). `len` unused except to keep the call uniform.
fn push_sphere(b: &mut MeshBuf, tf: &Transform, r: f32, _len: f32, c: [f32; 4]) {
    let stacks = RADIAL.max(6);
    let slices = RADIAL;
    let center = Vec3::Y * r;
    let pi = std::f32::consts::PI;
    let tau = std::f32::consts::TAU;
    let mut grid = vec![0u32; (stacks + 1) * (slices + 1)];
    for i in 0..=stacks {
        let phi = pi * i as f32 / stacks as f32; // 0..pi
        let (sp, cp) = phi.sin_cos();
        for j in 0..=slices {
            let th = tau * j as f32 / slices as f32;
            let (st, ct) = th.sin_cos();
            let n = Vec3::new(sp * st, cp, sp * ct);
            grid[i * (slices + 1) + j] = b.vert(tf, center + n * r, n, c);
        }
    }
    for i in 0..stacks {
        for j in 0..slices {
            let a = grid[i * (slices + 1) + j];
            let d = grid[i * (slices + 1) + j + 1];
            let e = grid[(i + 1) * (slices + 1) + j];
            let f = grid[(i + 1) * (slices + 1) + j + 1];
            b.tri(a, e, f);
            b.tri(a, f, d);
        }
    }
}

// Flat plate (fin/wing): spans Y in [0,len], X in [-hw,hw], thin Z. hw = r (half-width).
fn push_plate(b: &mut MeshBuf, tf: &Transform, r: f32, len: f32, c: [f32; 4]) {
    let hw = r;
    let hz = (PLATE_THICK * r).max(0.01);
    let corners = [
        // (x, y, z, nx, ny, nz) faces: front/back dominate
        Vec3::new(-hw, 0.0, hz),
        Vec3::new(hw, 0.0, hz),
        Vec3::new(hw, len, hz),
        Vec3::new(-hw, len, hz),
        Vec3::new(-hw, 0.0, -hz),
        Vec3::new(hw, 0.0, -hz),
        Vec3::new(hw, len, -hz),
        Vec3::new(-hw, len, -hz),
    ];
    let face = |b: &mut MeshBuf, q: [usize; 4], n: Vec3| {
        let v0 = b.vert(tf, corners[q[0]], n, c);
        let v1 = b.vert(tf, corners[q[1]], n, c);
        let v2 = b.vert(tf, corners[q[2]], n, c);
        let v3 = b.vert(tf, corners[q[3]], n, c);
        b.tri(v0, v1, v2);
        b.tri(v0, v2, v3);
    };
    face(b, [0, 1, 2, 3], Vec3::Z); // front
    face(b, [5, 4, 7, 6], Vec3::NEG_Z); // back
    face(b, [4, 0, 3, 7], Vec3::NEG_X); // left
    face(b, [1, 5, 6, 2], Vec3::X); // right
    face(b, [3, 2, 6, 7], Vec3::Y); // top
    face(b, [4, 5, 1, 0], Vec3::NEG_Y); // bottom
}

// Stable hash of a body graph for the render mesh cache (f32 bit patterns + enum tags + indices, FNV-1a).
pub fn body_hash(g: &BodyGraph) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    let f = |v: f32| v.to_bits() as u64;
    mix(g.root as u64);
    for n in &g.nodes {
        mix(n.shape as u64);
        mix(f(n.length));
        mix(f(n.radius));
        mix(f(n.taper));
        mix(f(n.r) ^ (f(n.g) << 1) ^ (f(n.b) << 2));
    }
    for e in &g.edges {
        mix((e.from as u64) ^ ((e.to as u64) << 8) ^ ((e.recurse as u64) << 16) ^ ((e.reflect as u64) << 24));
        mix(f(e.along) ^ (f(e.around) << 1));
        mix(f(e.pitch) ^ (f(e.roll) << 1) ^ (f(e.scale) << 2));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_mesh_is_nonempty_and_finite() {
        let mut rng = Rng::seed(5);
        let g = BodyGraph::random(&mut rng);
        let mesh = build_body_mesh(&develop(&g), 0.0);
        let n = mesh.count_vertices();
        assert!(n > 0, "mesh has vertices");
        assert!(mesh.indices().map(|i| i.len() > 0).unwrap_or(false), "mesh has indices");
    }

    #[test]
    fn body_hash_stable_and_sensitive() {
        let mut rng = Rng::seed(11);
        let g = BodyGraph::random(&mut rng);
        assert_eq!(body_hash(&g), body_hash(&g.clone()), "same graph -> same hash");
        let mut g2 = g.clone();
        g2.nodes[0].length += 0.5;
        assert_ne!(body_hash(&g), body_hash(&g2), "changed graph -> different hash");
    }

    #[test]
    fn default_body_is_one_capsule() {
        let p = develop(&default_body());
        assert_eq!(p.parts.len(), 1);
        assert_eq!(p.parts[0].shape, ShapeKind::Segment);
        let m = Morphometrics::of(&default_body());
        assert!(m.mass > 0.0, "capsule has mass");
        assert_eq!(m.part_count, 1);
        assert!(m.reach > 0.0);
    }

    #[test]
    fn develop_is_deterministic() {
        let mut rng = Rng::seed(123);
        let g = BodyGraph::random(&mut rng);
        let a = develop(&g).parts.len();
        let b = develop(&g).parts.len();
        assert_eq!(a, b, "same graph -> same part count");
        // exact transforms reproduce
        let pa = develop(&g);
        let pb = develop(&g);
        for (x, y) in pa.parts.iter().zip(&pb.parts) {
            assert_eq!(x.tf.translation, y.tf.translation);
        }
    }

    #[test]
    fn reflect_makes_bilateral_pairs() {
        // torso + one reflected leg edge -> torso + 2 legs
        let g = BodyGraph {
            nodes: vec![
                PartGene { shape: ShapeKind::Segment, length: 1.5, radius: 0.5, taper: 0.8, r: 0.5, g: 0.5, b: 0.5 },
                PartGene { shape: ShapeKind::Segment, length: 0.8, radius: 0.15, taper: 0.6, r: 0.4, g: 0.4, b: 0.4 },
            ],
            edges: vec![EdgeGene { from: 0, to: 1, along: 0.5, around: 1.2, pitch: -1.0, roll: 0.0, scale: 0.9, reflect: true, recurse: 1, joint: JointSpec::default() }],
            root: 0,
        };
        let p = develop(&g);
        assert_eq!(p.parts.len(), 3, "torso + 2 mirrored legs");
        // the two legs sit on opposite sides (mirrored X)
        let legs: Vec<&PlacedPart> = p.parts.iter().filter(|q| q.depth == 1).collect();
        assert_eq!(legs.len(), 2);
        assert!((legs[0].tf.translation.x + legs[1].tf.translation.x).abs() < 1e-3, "legs mirror across X");
    }

    #[test]
    fn recurse_makes_a_chain() {
        // self-chaining edge (to == from of a limb node) builds a multi-segment limb bounded by recurse
        let g = BodyGraph {
            nodes: vec![
                PartGene { shape: ShapeKind::Segment, length: 1.5, radius: 0.5, taper: 0.8, r: 0.5, g: 0.5, b: 0.5 },
                PartGene { shape: ShapeKind::Segment, length: 0.6, radius: 0.12, taper: 0.7, r: 0.4, g: 0.4, b: 0.4 },
            ],
            edges: vec![
                EdgeGene { from: 0, to: 1, along: 0.5, around: 1.0, pitch: -0.8, roll: 0.0, scale: 0.85, reflect: false, recurse: 1, joint: JointSpec::default() },
                EdgeGene { from: 1, to: 1, along: 0.95, around: 0.0, pitch: 0.0, roll: 0.0, scale: 0.8, reflect: false, recurse: 3, joint: JointSpec::default() },
            ],
            root: 0,
        };
        let p = develop(&g);
        // torso(1) + first limb(1) + 3 chained = 5
        assert_eq!(p.parts.len(), 5);
    }

    #[test]
    fn max_parts_bounds_runaway_graph() {
        // a fully self-recursive, reflected, high-recurse graph must still terminate at MAX_PARTS
        let g = BodyGraph {
            nodes: vec![PartGene { shape: ShapeKind::Segment, length: 1.0, radius: 0.5, taper: 0.9, r: 0.5, g: 0.5, b: 0.5 }],
            edges: vec![EdgeGene { from: 0, to: 0, along: 0.9, around: 0.5, pitch: 0.1, roll: 0.0, scale: 0.95, reflect: true, recurse: 5, joint: JointSpec::default() }],
            root: 0,
        };
        let p = develop(&g);
        assert!(p.parts.len() <= MAX_PARTS, "got {} > MAX_PARTS", p.parts.len());
    }

    #[test]
    fn random_bodies_are_varied_and_viable() {
        let mut rng = Rng::seed(99);
        for _ in 0..50 {
            let g = BodyGraph::random(&mut rng);
            let m = Morphometrics::of(&g);
            assert!(m.part_count >= 2, "has torso + appendages");
            assert!(m.part_count <= MAX_PARTS as u32);
            assert!(m.mass > 0.0 && m.mass.is_finite());
            assert!(m.reach > 0.0);
            assert!(m.bbox_max.x >= m.bbox_min.x);
        }
    }

    #[test]
    fn mutate_keeps_graph_bounded() {
        let mut rng = Rng::seed(7);
        let mut g = BodyGraph::random(&mut rng);
        for _ in 0..500 {
            g.mutate(&mut rng, 0.3);
            assert!(g.nodes.len() <= MAX_NODES);
            assert!(g.edges.len() <= MAX_EDGES);
            let m = Morphometrics::of(&g);
            assert!(m.mass.is_finite() && m.mass > 0.0);
            assert!(m.part_count <= MAX_PARTS as u32);
        }
    }
}
