//! Render-only: creature-built structures (build.rs) made visible. Woven nests as straw bowls, earthworks as
//! earthen berms with the pond they hold back. One entity per built field cell, diffed against the sim every
//! REFRESH_FRAMES so thousands of cells cost nothing between refreshes. No sim coupling beyond reads.
use bevy::prelude::*;
use std::collections::HashMap;

const REFRESH_FRAMES: u32 = 20;
/// Nest mass below which nothing is drawn (a few twigs read as noise from orbit).
const NEST_VIS_MIN_KG: f64 = 0.05;
const DAM_VIS_MIN: f32 = 0.08;
/// Pond shows when the dammed cell's ground water is above this (world mean sits ~0.02-0.05).
const POND_GW_MIN: f32 = 0.04;

pub struct BuildVizPlugin;

impl Plugin for BuildVizPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_build_viz).add_systems(Update, refresh_structures);
    }
}

#[derive(Resource)]
struct BuildViz {
    nest_mesh: Handle<Mesh>,
    nest_mat: Handle<StandardMaterial>,
    dam_mesh: Handle<Mesh>,
    dam_mat: Handle<StandardMaterial>,
    pond_mesh: Handle<Mesh>,
    pond_mat: Handle<StandardMaterial>,
    nests: HashMap<usize, Entity>,
    dams: HashMap<usize, Entity>,
    ponds: HashMap<usize, Entity>,
    frame: u32,
}

#[derive(Component)]
pub struct Structure;

fn setup_build_viz(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(BuildViz {
        // woven bowl (lathed cup, hollow, raised rim). A torus read as an egg/rock from ground level.
        nest_mesh: meshes.add(nest_bowl_mesh()),
        nest_mat: mats.add(StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.95, ..default() }),
        // berm: a squashed half-buried sphere = wide low mound of packed earth. NOT a capsule: a capsule on its
        // side reads as a fallen log (viz::Log) from orbit. Ochre clay, lighter than bark.
        dam_mesh: meshes.add(Sphere::new(1.0).mesh().ico(3).unwrap()),
        dam_mat: mats.add(StandardMaterial { base_color: Color::srgb(0.58, 0.45, 0.27), perceptual_roughness: 1.0, ..default() }),
        pond_mesh: meshes.add(Cylinder::new(1.0, 0.04)),
        pond_mat: mats.add(StandardMaterial {
            base_color: Color::srgba(0.18, 0.36, 0.52, 0.75),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.08,
            ..default()
        }),
        nests: HashMap::new(),
        dams: HashMap::new(),
        ponds: HashMap::new(),
        frame: 0,
    });
}

// Lathed bowl, unit size (rim radius ~0.6, height ~0.4), +Y up. Profile runs outside-bottom -> rim -> inside
// floor, so the hollow is visible from above and the rim silhouette from the side. Vertex colours: straw base
// with darker twig streaks (angle hash) and a paler rim, so it reads WOVEN without a texture.
fn nest_bowl_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    // (radius, height) profile
    let prof: [(f32, f32); 8] = [(0.05, 0.0), (0.38, 0.02), (0.55, 0.16), (0.62, 0.34), (0.58, 0.40), (0.50, 0.36), (0.38, 0.20), (0.05, 0.14)];
    let segs = 28usize;
    let rows = prof.len();
    let mut pos = Vec::with_capacity(rows * (segs + 1));
    let mut nrm = Vec::with_capacity(rows * (segs + 1));
    let mut col = Vec::with_capacity(rows * (segs + 1));
    for (r, &(rad, h)) in prof.iter().enumerate() {
        // profile tangent -> outward normal in the (radial, y) plane
        let (a, b) = (prof[r.saturating_sub(1)], prof[(r + 1).min(rows - 1)]);
        let (dr, dy) = (b.0 - a.0, b.1 - a.1);
        // (dy, -dr) already faces OUT on the outer wall and INTO the cavity on the inner rows (profile turns
        // back on itself), matching the (a,c,b) winding; flipping the inner rows shaded the hollow as facing down
        let (nr, ny) = (dy, -dr);
        let len = (nr * nr + ny * ny).sqrt().max(1e-6);
        for k in 0..=segs {
            let th = k as f32 / segs as f32 * std::f32::consts::TAU;
            let (s, c) = th.sin_cos();
            pos.push([rad * c, h, rad * s]);
            nrm.push([nr / len * c, ny / len, nr / len * s]);
            let strand = ((k as u32 * 2654435761u32 ^ (r as u32 * 40503)) >> 24) as f32 / 255.0;
            let rim = if (3..=4).contains(&r) { 0.12 } else { 0.0 };
            let shade = 0.78 + 0.22 * strand + rim;
            col.push([0.66 * shade, 0.53 * shade, 0.30 * shade, 1.0]);
        }
    }
    let mut idx = Vec::new();
    let w = (segs + 1) as u32;
    for r in 0..(rows as u32 - 1) {
        for k in 0..segs as u32 {
            let (a, b, c, d) = (r * w + k, r * w + k + 1, (r + 1) * w + k, (r + 1) * w + k + 1);
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(idx))
}

// Upright on the sphere at cell centre, `yaw` around the local normal, lifted `lift` above terrain.
fn placed(dir: Vec3, yaw: f32, lift: f32, scale: Vec3) -> Transform {
    let up = dir.normalize_or_zero();
    let (east, _) = crate::sphere::tangent_frame(up);
    let base = Quat::from_rotation_arc(Vec3::Y, up);
    let east_local = base.inverse() * east;
    let align = Quat::from_rotation_arc(Vec3::X, Vec3::new(east_local.x, 0.0, east_local.z).normalize_or(Vec3::X));
    Transform {
        translation: crate::sphere::surface_pos(up, lift),
        rotation: base * align * Quat::from_rotation_y(yaw),
        scale,
    }
}

// Per-cell decorative yaw: stable across refreshes (hash of the cell index), so structures do not spin.
fn cell_yaw(c: usize) -> f32 {
    let h = (c as u32).wrapping_mul(2654435761);
    (h >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU
}

fn refresh_structures(
    mut commands: Commands,
    mut viz: ResMut<BuildViz>,
    bio: Res<crate::chem::Biosphere>,
    earth: Res<crate::build::Earthworks>,
    gw: Res<crate::sim::GroundWater>,
) {
    viz.frame = viz.frame.wrapping_add(1);
    if viz.frame % REFRESH_FRAMES != 0 {
        return;
    }
    let g = crate::grid::field();
    let viz = &mut *viz;
    for c in 0..g.len() {
        let dir = g.center(c);
        // nests: size grows as cube root of mass (volume), so a 10x heavier nest is ~2x wider
        let kg = bio.nest_kg(c);
        if kg >= NEST_VIS_MIN_KG {
            // size the bowl to ~one occupant (cube root of mass): a cell's whole stock drawn as ONE bowl at full
            // cbrt scale dwarfed the animals that use it, so big stocks read as a broader, not taller, bowl
            let s = (kg / 0.4).cbrt().clamp(0.35, 1.4) as f32;
            let tf = placed(dir, cell_yaw(c), 0.0, Vec3::new(s, 0.8 * s, s));
            match viz.nests.get(&c) {
                Some(&e) => {
                    commands.entity(e).insert(tf);
                }
                None => {
                    let e = commands.spawn((Mesh3d(viz.nest_mesh.clone()), MeshMaterial3d(viz.nest_mat.clone()), tf, Structure)).id();
                    viz.nests.insert(c, e);
                }
            }
        } else if let Some(e) = viz.nests.remove(&c) {
            commands.entity(e).despawn();
        }
        // earthworks: a berm whose height tracks the level, plus the pond it holds when the ground is wet
        let lvl = earth.level[c];
        if lvl >= DAM_VIS_MIN {
            let yaw = cell_yaw(c) + 1.3;
            let tf = placed(dir, yaw, -0.1, Vec3::new(2.4, 0.15 + 0.45 * lvl, 0.7 + 0.3 * lvl));
            match viz.dams.get(&c) {
                Some(&e) => {
                    commands.entity(e).insert(tf);
                }
                None => {
                    let e = commands.spawn((Mesh3d(viz.dam_mesh.clone()), MeshMaterial3d(viz.dam_mat.clone()), tf, Structure)).id();
                    viz.dams.insert(c, e);
                }
            }
            let wet = gw.cell[c];
            if wet >= POND_GW_MIN {
                let r = 0.8 + 1.8 * lvl * (wet / 0.2).min(1.0);
                // the water pools on one side of the berm, not under it (centred, the mound hides it)
                let (e, n) = crate::sphere::tangent_frame(dir);
                let side = (e * yaw.cos() + n * yaw.sin()).normalize_or_zero();
                let pond_dir = (dir + side * (1.2 + r) / crate::sphere::PLANET_R).normalize();
                let ptf = placed(pond_dir, 0.0, 0.03, Vec3::new(r, 1.0, r * 0.8));
                match viz.ponds.get(&c) {
                    Some(&e) => {
                        commands.entity(e).insert(ptf);
                    }
                    None => {
                        let e = commands.spawn((Mesh3d(viz.pond_mesh.clone()), MeshMaterial3d(viz.pond_mat.clone()), ptf, Structure)).id();
                        viz.ponds.insert(c, e);
                    }
                }
            } else if let Some(e) = viz.ponds.remove(&c) {
                commands.entity(e).despawn();
            }
        } else {
            if let Some(e) = viz.dams.remove(&c) {
                commands.entity(e).despawn();
            }
            if let Some(e) = viz.ponds.remove(&c) {
                commands.entity(e).despawn();
            }
        }
    }
}
