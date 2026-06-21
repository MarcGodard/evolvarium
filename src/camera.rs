// Orbit camera for the planet. DRAG (hold right mouse) to rotate around the globe, SCROLL to zoom, WASD/QE
// as a keyboard fallback. LEFT-click selects a creature/plant; F follows the selection. Render mode only.
use crate::viz::Selected;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

pub struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_camera, log_controls))
            .add_systems(Update, (orbit_drag, orbit_keys, zoom, apply_orbit, follow_camera).chain());
    }
}

fn log_controls() {
    info!("camera: DRAG (right-mouse) orbit | SCROLL zoom | WASD/QE orbit+zoom | LEFT-CLICK select | F follow | ESC release");
}

// Orbit state: the camera sits on a sphere of radius `dist` around the planet center, aimed inward.
#[derive(Component)]
pub struct OrbitCam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

const MIN_DIST: f32 = 95.0; // just above the surface (planet radius ~80 + terrain)
const MAX_DIST: f32 = 420.0;

fn spawn_camera(mut commands: Commands) {
    // start framed on the homeland (where the founding population lives)
    let (lon, lat) = crate::sphere::dir_to_lonlat(crate::sim::homeland_center());
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        // soft ambient (per-camera in 0.18) so the planet's night side is not pitch black
        AmbientLight { brightness: 220.0, ..default() },
        OrbitCam { yaw: lon, pitch: lat.clamp(-1.3, 1.3), dist: 230.0 },
    ));
}

// Hold right mouse + move to orbit. No cursor lock (a globe orbit reads better as a drag).
fn orbit_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    selected: Res<Selected>,
    mut q: Query<&mut OrbitCam>,
) {
    if selected.follow || !buttons.pressed(MouseButton::Right) {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    cam.yaw -= motion.delta.x * 0.005;
    cam.pitch = (cam.pitch + motion.delta.y * 0.005).clamp(-1.45, 1.45);
}

// Keyboard fallback: A/D orbit longitude, W/S zoom in/out, Q/E tilt latitude. Shift = faster.
fn orbit_keys(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, selected: Res<Selected>, mut q: Query<&mut OrbitCam>) {
    if selected.follow {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    let dt = time.delta_secs();
    let boost = if keys.pressed(KeyCode::ShiftLeft) { 3.0 } else { 1.0 };
    if keys.pressed(KeyCode::KeyA) { cam.yaw += 0.8 * dt * boost; }
    if keys.pressed(KeyCode::KeyD) { cam.yaw -= 0.8 * dt * boost; }
    if keys.pressed(KeyCode::KeyQ) { cam.pitch = (cam.pitch + 0.8 * dt * boost).clamp(-1.45, 1.45); }
    if keys.pressed(KeyCode::KeyE) { cam.pitch = (cam.pitch - 0.8 * dt * boost).clamp(-1.45, 1.45); }
    if keys.pressed(KeyCode::KeyW) { cam.dist = (cam.dist - 60.0 * dt * boost).clamp(MIN_DIST, MAX_DIST); }
    if keys.pressed(KeyCode::KeyS) { cam.dist = (cam.dist + 60.0 * dt * boost).clamp(MIN_DIST, MAX_DIST); }
}

// Scroll wheel zooms in/out.
fn zoom(scroll: Res<AccumulatedMouseScroll>, selected: Res<Selected>, mut q: Query<&mut OrbitCam>) {
    if selected.follow || scroll.delta.y == 0.0 {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    cam.dist = (cam.dist - scroll.delta.y * 12.0).clamp(MIN_DIST, MAX_DIST);
}

// Place the camera from (yaw, pitch, dist) around the planet center, looking inward.
fn apply_orbit(selected: Res<Selected>, mut q: Query<(&mut Transform, &OrbitCam)>) {
    if selected.follow {
        return; // follow_camera owns the transform while following
    }
    let Ok((mut t, cam)) = q.single_mut() else { return };
    let dir = Vec3::new(cam.pitch.cos() * cam.yaw.cos(), cam.pitch.sin(), cam.pitch.cos() * cam.yaw.sin());
    t.translation = dir * cam.dist;
    t.look_at(Vec3::ZERO, Vec3::Y);
}

// Follow the selected entity (toggle with F): keep a fixed offset and track it. Stops if the target dies.
fn follow_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mut selected: ResMut<Selected>,
    targets: Query<&GlobalTransform>,
    mut cam: Query<&mut Transform, With<OrbitCam>>,
) {
    let Ok(mut cam_tf) = cam.single_mut() else { return };
    if keys.just_pressed(KeyCode::KeyF) {
        if let Some(e) = selected.entity {
            selected.follow = !selected.follow;
            if selected.follow {
                if let Ok(t) = targets.get(e) {
                    // sit a little above + outside the target along its radial (a third-person planet view)
                    let n = t.translation().normalize_or_zero();
                    selected.follow_offset = n * 14.0 + Vec3::Y * 4.0;
                }
            }
        }
    }
    if !selected.follow {
        return;
    }
    match selected.entity.and_then(|e| targets.get(e).ok()) {
        Some(t) => {
            let tp = t.translation();
            cam_tf.translation = tp + selected.follow_offset;
            cam_tf.look_at(tp, Vec3::Y);
        }
        None => selected.follow = false, // target gone
    }
}
