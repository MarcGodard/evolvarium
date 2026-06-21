// Free-fly camera. RIGHT-click to capture mouse + look; Esc to release. WASD move, Q/E down/up, Shift
// faster. Left-click is left free for selecting creatures/plants (see viz::pick_on_click).
// Render mode only. Lets you fly through the world and watch blobs (see 01).
use crate::viz::Selected;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

pub struct FlyCameraPlugin;

impl Plugin for FlyCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_camera, log_controls))
            .add_systems(Update, (cursor_capture, look, movement, follow_camera));
    }
}

fn log_controls() {
    info!("camera: RIGHT-CLICK look | LEFT-CLICK select | F follow selected | ESC release | WASD move | Q/E down/up | Shift faster");
}

#[derive(Component)]
pub struct FlyCam {
    pub speed: f32,
    pub sensitivity: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for FlyCam {
    fn default() -> Self {
        Self { speed: 45.0, sensitivity: 0.003, yaw: 0.0, pitch: 0.0 } // faster: the planet is ~80-unit radius
    }
}

fn spawn_camera(mut commands: Commands) {
    // start out in space looking at the planet (radius ~80) from over the homeland
    let eye = crate::sim::homeland_center() * 230.0 + Vec3::Y * 40.0;
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y),
        // soft ambient (per-camera in 0.18) so the planet's night side is not pitch black
        AmbientLight { brightness: 220.0, ..default() },
        FlyCam::default(),
    ));
}

// Right-click to capture (lock + hide cursor); Esc to release. Left-click stays free for selection.
// 0.18: CursorOptions is its own component on the window entity, not a Window field.
fn cursor_capture(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    if buttons.just_pressed(MouseButton::Right) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

// Mouse motion -> yaw/pitch. Only while cursor captured (locked).
fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    selected: Res<Selected>,
    mut q: Query<(&mut Transform, &mut FlyCam)>,
) {
    if selected.follow {
        return; // follow_camera owns the camera while following
    }
    let Ok(cursor) = cursor.single() else { return };
    if cursor.grab_mode == CursorGrabMode::None {
        return; // not captured: don't steal mouse
    }
    let Ok((mut t, mut cam)) = q.single_mut() else { return };
    cam.yaw -= motion.delta.x * cam.sensitivity;
    cam.pitch -= motion.delta.y * cam.sensitivity;
    cam.pitch = cam.pitch.clamp(-1.54, 1.54); // ~+-88 deg, avoid flip
    t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
}

// WASD + Q/E fly, relative to facing. Shift to sprint.
fn movement(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    selected: Res<Selected>,
    mut q: Query<(&mut Transform, &FlyCam)>,
) {
    if selected.follow {
        return; // follow_camera owns the camera while following
    }
    let Ok((mut t, cam)) = q.single_mut() else { return };
    let mut dir = Vec3::ZERO;
    let fwd = *t.forward();
    let right = *t.right();
    if keys.pressed(KeyCode::KeyW) { dir += fwd; }
    if keys.pressed(KeyCode::KeyS) { dir -= fwd; }
    if keys.pressed(KeyCode::KeyD) { dir += right; }
    if keys.pressed(KeyCode::KeyA) { dir -= right; }
    if keys.pressed(KeyCode::KeyE) { dir += Vec3::Y; }
    if keys.pressed(KeyCode::KeyQ) { dir -= Vec3::Y; }

    if dir != Vec3::ZERO {
        let boost = if keys.pressed(KeyCode::ShiftLeft) { 3.0 } else { 1.0 };
        t.translation += dir.normalize() * cam.speed * boost * time.delta_secs();
    }
}

// Follow the selected entity (toggle with F). Captures the current camera offset when engaged, then
// keeps that offset while tracking the target each frame. Stops if the target dies/despawns.
fn follow_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mut selected: ResMut<Selected>,
    targets: Query<&GlobalTransform>,
    mut cam: Query<&mut Transform, With<FlyCam>>,
) {
    let Ok(mut cam_tf) = cam.single_mut() else { return };
    if keys.just_pressed(KeyCode::KeyF) {
        if let Some(e) = selected.entity {
            selected.follow = !selected.follow;
            if selected.follow {
                if let Ok(t) = targets.get(e) {
                    let off = cam_tf.translation - t.translation();
                    // avoid a degenerate zero offset (camera sitting on the target)
                    selected.follow_offset = if off.length() < 1.0 { Vec3::new(0.0, 6.0, 12.0) } else { off };
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
