//! GPU screenshot capture (--capture). Renders REAL Bevy scene (true directional light + shadows + ambient)
//! from walk camera at chosen sun phase, saves PNG, exits. Primary offline render verifier; snapshot.rs is
//! the CPU proxy. Needs GPU + display, render mode only.
use crate::camera::{CameraMode, WalkCam};
use crate::viz::SunOffset;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

// --cap-when: sun phase for walk shot, relative to homeland camera stands on.
#[derive(Clone, Copy)]
pub enum CapWhen {
    Morning, // sun ~45deg up east -> long shadows
    Noon,    // sun overhead -> flat light, tiny shadows
    Dusk,    // sun near horizon -> very long shadows
    Night,   // sun far side -> tests no light leaks through planet
}

#[derive(Resource)]
pub struct CaptureCfg {
    pub prefix: String,
    pub when: CapWhen,
    pub yaw: f32,     // --cap-yaw: walk heading (look dir around surface)
    pub off: i64,     // --cap-off: raw sun-tick offset, overrides `when` when nonzero, dials sun angle
    pub pitch: f32,   // --cap-pitch: cam pitch. negative = look down
    pub orbit: bool,  // --cap-orbit: capture from orbit (space) not walk (surface)
    pub dist: f32,    // --cap-dist: orbit distance from planet center (95..420). zoom test for eclipse-disc regression
    pub underwater: bool, // --cap-water: submerge in deep ocean. verifies swim view + blue tint
    pub lat: Option<f32>, // --cap-lat: top-down orbit view at this latitude (deg, +90 = north pole, -90 = south)
    pub warmup: u32,      // --cap-warmup: sim frames before the shot (default WARMUP). Raise to let fliers rise off the ground + land-wear trails accumulate before snapping.
    pub back: f32,        // --cap-back: walk side-vantage distance from homeland (default 22 = original framing). Small = wide-area close-up.
    pub orrery: bool,     // --cap-orrery: capture the TSN solar-system view instead of the planet
    pub focus_creature: bool, // --cap-creature: aim at the creature nearest homeland from cap-back units away (creature close-up)
}

// Deepest-ocean surface dir, found by scanning a Fibonacci sphere (2000 samples). Robust to noise seed
// vs a hardcoded dir. --cap-water uses it to stand camera in real deep water.
fn ocean_dir() -> Vec3 {
    let n = 2000usize;
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let mut best = Vec3::Z;
    let mut lowest = f32::INFINITY;
    for i in 0..n {
        let y = 1.0 - (i as f32 + 0.5) / n as f32 * 2.0;
        let r = (1.0 - y * y).max(0.0).sqrt();
        let theta = golden * i as f32;
        let d = Vec3::new(theta.cos() * r, y, theta.sin() * r);
        let e = crate::sphere::elevation01(d);
        if e < lowest {
            lowest = e;
            best = d;
        }
    }
    best
}

// Shallow flora-band ocean point (~5u deep): where seaweed grows + swimmers forage, vs the barren abyss
// ocean_dir picks. Better swim view (fish + kelp in frame), still well submerged (eye 2u off floor stays
// below the surface). Scans a Fibonacci sphere for the submerged point closest to TARGET_DEPTH.
fn shallow_swim_dir() -> Vec3 {
    const TARGET_DEPTH: f32 = 5.0; // below surface (flora band spans ~0..9u); deep enough to stay submerged
    let n = 2000usize;
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let mut best = ocean_dir();
    let mut best_err = f32::INFINITY;
    for i in 0..n {
        let y = 1.0 - (i as f32 + 0.5) / n as f32 * 2.0;
        let r = (1.0 - y * y).max(0.0).sqrt();
        let theta = golden * i as f32;
        let d = Vec3::new(theta.cos() * r, y, theta.sin() * r);
        let depth = -crate::sphere::elevation(d); // >0 in ocean
        if depth <= 0.0 {
            continue; // land
        }
        let err = (depth - TARGET_DEPTH).abs();
        if err < best_err {
            best_err = err;
            best = d;
        }
    }
    best
}

// Default frames to wait before shot so assets load + sim settles (materials, dressed entities). Override
// with --cap-warmup for slow effects (fliers climbing to cruise alt, land-wear trails forming).
pub const WARMUP: u32 = 50;

pub struct CapturePlugin;
impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        // PostStartup not Startup: spawn_camera (camera plugin Startup) must run first so WalkCam entity
        // exists when we drop it into ocean for --cap-water.
        app.add_systems(PostStartup, setup_capture_view)
            .add_systems(Update, (capture_tick, quit_countdown))
            // deterministic framing: own cam transform in PostUpdate, after walk/orbit ran
            .add_systems(PostUpdate, force_cam.before(bevy::transform::TransformSystems::Propagate));
    }
}

// Aim camera at homeland from fixed side+elevated vantage, ignoring walk/orbit. Deterministic so test
// objects + shadows always framed.
fn force_cam(
    cfg: Res<CaptureCfg>,
    mut q: Query<&mut Transform, (With<Camera3d>, With<crate::camera::OrbitCam>)>,
    creatures: Query<&Transform, (With<crate::components::Creature>, Without<Camera3d>)>,
) {
    if cfg.orrery {
        return; // OrreryCam set in setup_capture_view; apply_orrery frames it from the focus point
    }
    // --cap-creature: lock onto the creature nearest homeland, frame it from cap-back units away (slightly up
    // + along the cap-yaw tangent). Repeated each frame so it tracks the chosen creature through warmup.
    if cfg.focus_creature {
        let home = crate::sim::homeland_center();
        let target = creatures
            .iter()
            .map(|t| t.translation)
            .min_by(|a, b| {
                let da = (a.normalize_or_zero() - home).length_squared();
                let db = (b.normalize_or_zero() - home).length_squared();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            });
        if let (Some(target), Ok(mut t)) = (target, q.single_mut()) {
            let up = target.normalize_or_zero(); // planet-up at the creature
            let tangent = crate::sphere::heading_tangent(up, cfg.yaw);
            let dist = cfg.back.max(2.0);
            let eye = target + tangent * dist + up * (0.7 * dist); // back + well above to clear grass (3/4 down view)
            *t = Transform::from_translation(eye).looking_at(target, up);
        }
        return;
    }
    if cfg.orbit {
        // --cap-lat: aim orbit cam straight down at chosen latitude on homeland meridian, top-down pole view.
        // Own transform here (not apply_orbit) to pick stable up: look_at with up=Y collapses at poles (view
        // axis ~parallel Y), so near a pole use Z as up instead.
        if let Some(lat_deg) = cfg.lat {
            use std::f32::consts::FRAC_PI_2;
            let lat = lat_deg.to_radians().clamp(-FRAC_PI_2, FRAC_PI_2);
            let (lon, _) = crate::sphere::dir_to_lonlat(crate::sim::homeland_center());
            let dir = Vec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin());
            let eye = dir * cfg.dist;
            let up = if dir.y.abs() > 0.9 { Vec3::Z } else { Vec3::Y }; // near pole: avoid Y up collapse
            if let Ok(mut t) = q.single_mut() {
                *t = Transform::from_translation(eye).looking_at(Vec3::ZERO, up);
            }
        }
        return; // plain orbit framing owned by apply_orbit (Update). don't override here
    }
    if cfg.underwater {
        // submerged shallow flora band: eye 2u off seafloor, level + slightly up at sunlit surface. shot shows
        // blue tint + kelp + foraging swimmers (vs the empty abyss).
        let d = shallow_swim_dir();
        let eye = crate::sphere::surface_pos(d, 2.0);
        // look along heading tilted by cap-pitch (negative = down at lit seafloor through water)
        let tangent = crate::sphere::heading_tangent(d, cfg.yaw);
        let fwd = (tangent * cfg.pitch.cos() + d * cfg.pitch.sin()).normalize();
        if let Ok(mut t) = q.single_mut() {
            *t = Transform::from_translation(eye).looking_to(fwd, d);
        }
        return;
    }
    let home = crate::sim::homeland_center();
    let side = crate::sphere::heading_tangent(home, cfg.yaw);
    // elevated side vantage looking at homeland. Scales with --cap-back: back=22 = original wide framing,
    // small back (e.g. 4) = close-up on the founding creatures. Height tracks back so the look-down angle holds.
    let f = cfg.back / 22.0;
    let eye = crate::sphere::surface_pos(home, 10.0 * f) + side * 22.0 * f;
    let target = crate::sphere::surface_pos(home, 2.0 * f);
    if let Ok(mut t) = q.single_mut() {
        *t = Transform::from_translation(eye).looking_at(target, home);
    }
}

// Stand walk camera on homeland, face `yaw`, set sun to requested phase.
fn setup_capture_view(
    cfg: Res<CaptureCfg>,
    mut mode: ResMut<CameraMode>,
    mut offset: ResMut<SunOffset>,
    mut q: Query<&mut WalkCam>,
    mut orbit_q: Query<&mut crate::camera::OrbitCam>,
    mut orrery_q: Query<&mut crate::camera::OrreryCam>,
) {
    if cfg.orrery {
        *mode = CameraMode::Orrery;
        // drive the OrreryCam from cfg so apply_orrery frames it AND body sizing (reads dist) matches
        if let Ok(mut o) = orrery_q.single_mut() {
            o.yaw = cfg.yaw;
            o.pitch = if cfg.pitch != 0.0 { cfg.pitch } else { 0.45 };
            o.dist = if cfg.dist > 1.0 { cfg.dist } else { 1500.0 };
        }
        return;
    }
    let home = crate::sim::homeland_center();
    // sun anchor: overhead ocean point for --cap-water, else overhead homeland.
    let sun_anchor = if cfg.underwater { shallow_swim_dir() } else { home };
    if cfg.underwater {
        // submerged swim view: drop walk eye into deep ocean so track_underwater flags it (tint overlay +
        // murky sky then show in shot). force_cam owns final transform.
        *mode = CameraMode::Walk;
        if let Ok(mut w) = q.single_mut() {
            w.dir = shallow_swim_dir();
            w.yaw = cfg.yaw;
            w.pitch = cfg.pitch;
            w.eye_alt = 2.0; // 2u above seafloor. flora-band depth ~5u -> eye ~3u below surface -> underwater
        }
    } else if cfg.orbit {
        *mode = CameraMode::Orbit;
        let (lon, lat) = crate::sphere::dir_to_lonlat(home);
        if let Ok(mut o) = orbit_q.single_mut() {
            o.yaw = lon;
            o.pitch = lat.clamp(-1.3, 1.3);
            o.dist = cfg.dist;
        }
    } else {
        *mode = CameraMode::Walk;
        if let Ok(mut w) = q.single_mut() {
            // stand back cfg.back u from homeland along heading so homeland entities are in front
            w.dir = crate::sphere::step(home, cfg.yaw, -cfg.back).0;
            w.yaw = cfg.yaw;
            w.pitch = cfg.pitch;
        }
    }
    // noon_offset puts sun overhead anchor. shift for requested hour, or raw --cap-off. offsets in day-ticks.
    let day = crate::sphere::DAY_TICKS as i64;
    let base = crate::viz::noon_offset(sun_anchor, 0);
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

// Wait WARMUP frames, snap window to PNG, exit once written.
fn capture_tick(
    mut frames: Local<u32>,
    mut shot: Local<bool>,
    cfg: Res<CaptureCfg>,
    gen: Res<crate::sim::GenState>,
    offset: Res<SunOffset>,
    walkers: Query<&WalkCam>,
    lights: Query<(&DirectionalLight, &GlobalTransform, &ViewVisibility), With<crate::viz::SunLight>>,
    underwater: Res<crate::viz::Underwater>,
    mut commands: Commands,
) {
    *frames += 1;
    if *frames < cfg.warmup || *shot {
        return;
    }
    *shot = true;
    // diag: sun vs camera, shadows on? home.dot(sd) > 0 = day side.
    let vtick = (gen.tick as i64 + offset.0).rem_euclid(crate::sphere::DAY_TICKS as i64) as u32;
    let sd = crate::sphere::sun_dir(vtick);
    let home = walkers.single().map(|w| w.dir.normalize_or_zero()).unwrap_or(Vec3::Y);
    let day = home.dot(sd);
    let (shadows, fwd, vis) = lights
        .single()
        .map(|(l, gt, v)| (l.shadows_enabled, gt.forward().as_vec3(), v.get()))
        .unwrap_or((false, Vec3::ZERO, false));
    let weye = walkers.single().map(|w| (crate::sphere::is_ocean(w.dir), w.eye_alt)).unwrap_or((false, 0.0));
    info!(
        "capture diag: vtick={} -forward={:?} home.dot(sd)={:.2} shadows_enabled={} light_view_visible={} underwater={} is_ocean={} eye_alt={:.2}",
        vtick, -fwd, day, shadows, vis, underwater.0, weye.0, weye.1
    );
    let _ = sd;
    let path = format!("{}.png", cfg.prefix);
    info!("capture: writing {path}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    // QuitAfter(3): hold a few frames so screenshot flushes to disk before exit
    commands.spawn(QuitAfter(3));
}

#[derive(Component)]
struct QuitAfter(u32);

// Count down quit timers. exit when any hits zero (screenshot flushed).
fn quit_countdown(mut q: Query<&mut QuitAfter>, mut exit: MessageWriter<AppExit>) {
    for mut t in &mut q {
        if t.0 == 0 {
            exit.write(AppExit::Success);
        } else {
            t.0 -= 1;
        }
    }
}
