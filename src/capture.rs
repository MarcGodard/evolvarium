//! GPU screenshot capture (--capture). Renders the REAL Bevy scene (true directional light + shadows +
//! ambient) from the walk camera at a chosen sun phase, saves a PNG, exits. Lets the world be inspected
//! offline (unlike snapshot.rs which is a CPU proxy). Needs a GPU + display; render mode only.
use crate::camera::{CameraMode, WalkCam};
use crate::viz::SunOffset;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

// Which time-of-day to frame the walk shot at (sun relative to the homeland the camera stands on).
#[derive(Clone, Copy)]
pub enum CapWhen {
    Morning, // sun ~45deg up in the east -> long visible shadows
    Noon,    // sun overhead -> flat light, tiny shadows
    Dusk,    // sun near the horizon -> very long shadows
    Night,   // sun on the far side -> tests "no light through the planet"
}

#[derive(Resource)]
pub struct CaptureCfg {
    pub prefix: String,
    pub when: CapWhen,
    pub yaw: f32,     // walk heading (look direction around the surface)
    pub off: i64,     // raw extra sun-tick offset (overrides `when` when nonzero) for dialing sun angle
    pub pitch: f32,   // camera pitch (negative = look down at the ground)
    pub orbit: bool,  // capture from orbit (far) instead of walk (surface)
}

// Frames to let assets load + the sim settle before grabbing the shot (materials, dressed entities).
const WARMUP: u32 = 50;

pub struct CapturePlugin;
impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_capture_view)
            .add_systems(Update, (capture_tick, quit_countdown))
            // deterministic framing: own the camera transform in PostUpdate, after walk/orbit ran
            .add_systems(PostUpdate, force_cam.before(bevy::transform::TransformSystems::Propagate));
    }
}

// Point the camera at the homeland from a fixed side+elevated vantage, ignoring walk/orbit. Deterministic
// so test objects + their shadows are always framed.
fn force_cam(cfg: Res<CaptureCfg>, mut q: Query<&mut Transform, With<Camera3d>>) {
    let home = crate::sim::homeland_center();
    let side = crate::sphere::heading_tangent(home, cfg.yaw);
    let eye = crate::sphere::surface_pos(home, 10.0) + side * 22.0;
    let target = crate::sphere::surface_pos(home, 2.0);
    if let Ok(mut t) = q.single_mut() {
        *t = Transform::from_translation(eye).looking_at(target, home);
    }
}

// Stand the walk camera on the homeland, face `yaw`, and set the sun to the requested phase.
fn setup_capture_view(
    cfg: Res<CaptureCfg>,
    mut mode: ResMut<CameraMode>,
    mut offset: ResMut<SunOffset>,
    mut q: Query<&mut WalkCam>,
    mut orbit_q: Query<&mut crate::camera::OrbitCam>,
) {
    let home = crate::sim::homeland_center();
    if cfg.orbit {
        *mode = CameraMode::Orbit;
        let (lon, lat) = crate::sphere::dir_to_lonlat(home);
        if let Ok(mut o) = orbit_q.single_mut() {
            o.yaw = lon;
            o.pitch = lat.clamp(-1.3, 1.3);
            o.dist = 140.0;
        }
    } else {
        *mode = CameraMode::Walk;
        if let Ok(mut w) = q.single_mut() {
            // stand BACK from the homeland along the heading so the homeland (entities) is in front
            w.dir = crate::sphere::step(home, cfg.yaw, -16.0).0;
            w.yaw = cfg.yaw;
            w.pitch = cfg.pitch;
        }
    }
    // noon_offset puts the sun overhead home; shift it for the requested hour (or raw --cap-off).
    let day = crate::sphere::DAY_TICKS as i64;
    let base = crate::viz::noon_offset(home, 0);
    offset.0 = base
        + if cfg.off != 0 {
            cfg.off
        } else {
            match cfg.when {
                CapWhen::Morning => -day / 8,
                CapWhen::Noon => 0,
                CapWhen::Dusk => -day * 5 / 32,
                CapWhen::Night => day / 2,
            }
        };
}

// Wait WARMUP frames, snap the window to PNG, exit once written.
fn capture_tick(
    mut frames: Local<u32>,
    mut shot: Local<bool>,
    cfg: Res<CaptureCfg>,
    gen: Res<crate::sim::GenState>,
    offset: Res<SunOffset>,
    walkers: Query<&WalkCam>,
    lights: Query<(&DirectionalLight, &GlobalTransform, &ViewVisibility), With<crate::viz::SunLight>>,
    mut commands: Commands,
) {
    *frames += 1;
    if *frames < WARMUP || *shot {
        return;
    }
    *shot = true;
    // diagnostics: where is the sun vs the camera, and are shadows on?
    let vtick = (gen.tick as i64 + offset.0).rem_euclid(crate::sphere::DAY_TICKS as i64) as u32;
    let sd = crate::sphere::sun_dir(vtick);
    let home = walkers.single().map(|w| w.dir.normalize_or_zero()).unwrap_or(Vec3::Y);
    let day = home.dot(sd);
    let (shadows, fwd, vis) = lights
        .single()
        .map(|(l, gt, v)| (l.shadows_enabled, gt.forward().as_vec3(), v.get()))
        .unwrap_or((false, Vec3::ZERO, false));
    info!(
        "capture diag: vtick={} -forward={:?} home.dot(sd)={:.2} shadows_enabled={} light_view_visible={}",
        vtick, -fwd, day, shadows, vis
    );
    let _ = sd;
    let path = format!("{}.png", cfg.prefix);
    info!("capture: writing {path}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    // give the save one extra frame, then quit
    commands.spawn(QuitAfter(3));
}

#[derive(Component)]
struct QuitAfter(u32);

// Count down spawned quit timers; exit when any reaches zero (screenshot flushed to disk).
fn quit_countdown(mut q: Query<&mut QuitAfter>, mut exit: MessageWriter<AppExit>) {
    for mut t in &mut q {
        if t.0 == 0 {
            exit.write(AppExit::Success);
        } else {
            t.0 -= 1;
        }
    }
}
