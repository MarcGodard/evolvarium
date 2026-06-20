// Free-fly camera. Click window to capture mouse + look; Esc to release. WASD move, Q/E down/up, Shift faster.
// Render mode only. Lets you fly through the world and watch blobs (see 01).
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

pub struct FlyCameraPlugin;

impl Plugin for FlyCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_camera, log_controls))
            .add_systems(Update, (cursor_capture, look, movement));
    }
}

fn log_controls() {
    info!("camera: CLICK window to capture mouse + look | ESC release | WASD move | Q/E down/up | Shift faster");
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
        Self { speed: 12.0, sensitivity: 0.003, yaw: 0.0, pitch: 0.0 }
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 8.0, 22.0).looking_at(Vec3::new(0.0, 4.0, 0.0), Vec3::Y),
        FlyCam::default(),
    ));
}

// Click to capture (lock + hide cursor); Esc to release. Standard FPS-style.
// 0.18: CursorOptions is its own component on the window entity, not a Window field.
fn cursor_capture(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    if buttons.just_pressed(MouseButton::Left) {
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
    mut q: Query<(&mut Transform, &mut FlyCam)>,
) {
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
    mut q: Query<(&mut Transform, &FlyCam)>,
) {
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
