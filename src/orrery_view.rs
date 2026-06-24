//! Orrery view: a second "space" reproducing the TSN / tychos.space solar system. Tab cycles Orbit ->
//! Orrery -> Walk. Renders every visible TSN body (sun, planets, moons, Pluto, Halley, Eros) as a colored
//! sphere positioned each frame by the orrery model (orrery::body_positions). Lives at a FAR origin
//! (ORRERY_CENTER) so it never collides with the planet world: in orrery mode the camera flies out there and
//! the planet falls beyond the far clip; in planet modes the orrery is beyond the far clip. So no visibility
//! toggling needed. Orbit traces, BSC starfield, ecliptic grid, labels come in later passes.
//!
//! Has its OWN clock (OrreryClock) so the system is lively (TSN-style time control) without speeding the sim.
use crate::camera::CameraMode;
use bevy::prelude::*;

// Far parking spot for the orrery scene (30k units from the planet at origin). Camera far clip ~12k, so the
// two scenes are mutually beyond each other's clip = clean separation.
pub const ORRERY_CENTER: Vec3 = Vec3::new(0.0, 0.0, 30000.0);
// View time rate: years of model time per real second. ~0.05 -> Mercury orbits in ~5 s, Earth in ~20 s.
const VIEW_YEARS_PER_SEC: f32 = 0.05;

pub struct OrreryViewPlugin;
impl Plugin for OrreryViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OrreryClock>()
            .add_systems(Startup, spawn_orrery_bodies)
            .add_systems(Update, (advance_clock, position_orrery_bodies).chain());
    }
}

/// Independent model time for the orrery view (years). Advances on real time so it runs even when sim paused.
#[derive(Resource, Default)]
pub struct OrreryClock {
    pub years: f32,
}

#[derive(Component)]
pub struct OrreryBody {
    pub idx: usize,
}

// TSN-ish body colors (name -> emissive RGB). Bodies render self-lit so they read as orrery markers.
fn body_color(name: &str) -> LinearRgba {
    match name {
        "Sun" => LinearRgba::rgb(7.0, 5.6, 2.0),
        "Mercury" => LinearRgba::rgb(0.55, 0.5, 0.45),
        "Venus" => LinearRgba::rgb(1.0, 0.9, 0.65),
        "Earth" => LinearRgba::rgb(0.25, 0.5, 1.0),
        "Moon" => LinearRgba::rgb(0.55, 0.55, 0.58),
        "Mars" => LinearRgba::rgb(0.9, 0.35, 0.2),
        "Phobos" | "Deimos" | "Eros" => LinearRgba::rgb(0.45, 0.4, 0.35),
        "Jupiter" => LinearRgba::rgb(0.85, 0.72, 0.55),
        "Saturn" => LinearRgba::rgb(0.9, 0.82, 0.6),
        "Uranus" => LinearRgba::rgb(0.6, 0.9, 0.95),
        "Neptune" => LinearRgba::rgb(0.3, 0.45, 0.95),
        "Pluto" => LinearRgba::rgb(0.75, 0.68, 0.6),
        "Halleys" => LinearRgba::rgb(0.8, 0.85, 0.95),
        _ => LinearRgba::rgb(0.7, 0.7, 0.7),
    }
}

// Render only real bodies, not the invisible deferent/center pivots.
fn is_visible_body(name: &str) -> bool {
    !name.contains("deferent") && name != "SystemCenter"
}

fn spawn_orrery_bodies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let unit = meshes.add(Sphere::new(1.0).mesh().ico(3).unwrap());
    for idx in 0..crate::orrery::body_count() {
        let (name, size, _tilt) = crate::orrery::body_meta(idx);
        if !is_visible_body(name) {
            continue;
        }
        let r = size.max(0.6); // tiny bodies (Eros 0.1) still readable as a dot
        let col = body_color(name);
        commands.spawn((
            OrreryBody { idx },
            Mesh3d(unit.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::from(col),
                emissive: col,
                unlit: true,
                ..default()
            })),
            Transform::from_translation(ORRERY_CENTER).with_scale(Vec3::splat(r)),
            bevy::light::NotShadowCaster,
        ));
    }
    info!("orrery view: spawned TSN bodies at {:?}; press TAB to reach it (Orbit -> Orrery -> Walk)", ORRERY_CENTER);
}

fn advance_clock(mode: Res<CameraMode>, time: Res<Time<bevy::time::Real>>, mut clock: ResMut<OrreryClock>) {
    if *mode == CameraMode::Orrery {
        clock.years += time.delta_secs() * VIEW_YEARS_PER_SEC;
    }
}

fn position_orrery_bodies(
    mode: Res<CameraMode>,
    clock: Res<OrreryClock>,
    mut q: Query<(&OrreryBody, &mut Transform)>,
) {
    if *mode != CameraMode::Orrery {
        return; // off-screen in other modes; skip the work
    }
    let pos = crate::orrery::body_positions(clock.years);
    for (b, mut tf) in &mut q {
        tf.translation = ORRERY_CENTER + pos[b.idx];
    }
}
