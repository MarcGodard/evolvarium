// Two camera modes for the planet (TAB switches), render mode only.
//   ORBIT (default): drag right-mouse to rotate the globe, scroll/W,S to zoom, A/D spin, Q/E tilt. Left-
//     click selects a creature/plant; F follows it. Real shadows ON (O toggles): the old "eclipse disc" was
//     a self-shadow blackout, fixed by marking the globe + celestial bodies NotShadowCaster (see main.rs).
//   WALK: a true ground walk -- the eye rides a fixed height above the terrain (climbs hills, never flies).
//     WASD move (W/S forward+back, A/D strafe), arrows or right-drag look, Shift run. Walk into the sea to
//     swim (look + W to dive). Real shadows ON: tight cascade at eye level -> crisp tree/creature shadows.
use crate::sim::GenState;
use crate::viz::{noon_offset, Selected, SunLight, SunOffset};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

pub struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_systems(Startup, (spawn_camera, log_controls))
            .add_systems(
                Update,
                (
                    toggle_mode,
                    orbit_drag,
                    orbit_keys,
                    zoom,
                    walk_look,
                    walk_move,
                    apply_orbit,
                    apply_walk,
                    follow_camera,
                    update_shadow_mode,
                    update_shadow_cascade,
                    update_planet_caster,
                )
                    .chain(),
            );
    }
}

fn log_controls() {
    info!("camera: TAB = orbit/walk | ORBIT: right-drag rotate, scroll zoom, click select, F follow | WALK: WASD move, arrows/right-drag look, Shift run, swim into the sea (look + W to dive)");
}

// Which camera is active. Orbit = space view; Walk = ground view (with real shadows).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    #[default]
    Orbit,
    Walk,
}

// Orbit state: the camera sits on a sphere of radius `dist` around the planet center, aimed inward.
#[derive(Component)]
pub struct OrbitCam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

// Walk state: stand on a surface point (`dir`, unit), face compass `yaw`, look up/down with `pitch`. On
// land `eye_alt` is locked to WALK_EYE so you ride the terrain (walk over hills, never fly). Over ocean it
// becomes free (swim): look + W dives/rises, clamped between the seafloor and the sea surface.
#[derive(Component)]
pub struct WalkCam {
    pub dir: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub eye_alt: f32, // eye height above the terrain at `dir`; WALK_EYE on land, swim-controlled in water
}

const MIN_DIST: f32 = 95.0; // just above the surface (planet radius ~80 + terrain)
const MAX_DIST: f32 = 420.0;
const WALK_EYE: f32 = 2.5; // eye height above the terrain surface (true walk: rides elevation)
const WALK_SPEED: f32 = 14.0; // walk speed (units/sec); Shift runs
const WALK_TURN: f32 = 1.6; // keyboard look speed (rad/sec)
const PITCH_LIMIT: f32 = 1.3;
const SWIM_FLOOR: f32 = 0.8; // swimming: eye stays this far above the seafloor (no clip-through)
const SWIM_CEIL: f32 = 2.0; // swimming: head may breach this far above the sea surface

fn spawn_camera(mut commands: Commands) {
    // start framed on the homeland (where the founding population lives)
    let hl = crate::sim::homeland_center();
    let (lon, lat) = crate::sphere::dir_to_lonlat(hl);
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        // far clip pushed out so the distant sun + starfield render (they sit thousands of units away)
        Projection::from(PerspectiveProjection { far: 12000.0, ..default() }),
        // soft ambient (per-camera in 0.18) so the planet's night side is not pitch black
        AmbientLight { brightness: 220.0, ..default() },
        OrbitCam { yaw: lon, pitch: lat.clamp(-1.3, 1.3), dist: 230.0 },
        WalkCam { dir: hl.normalize_or_zero(), yaw: 0.0, pitch: 0.0, eye_alt: WALK_EYE },
        // shadow softness: swapped per mode in update_shadow_mode (orbit = soft Gaussian, walk = crisp)
        bevy::light::ShadowFilteringMethod::Hardware2x2,
    ));
}

// TAB toggles orbit <-> walk. Entering walk: drop onto the surface point the orbit camera was over, facing
// north, level. Entering either mode cancels follow.
fn toggle_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<CameraMode>,
    mut selected: ResMut<Selected>,
    gen: Res<GenState>,
    mut sun_offset: ResMut<SunOffset>,
    mut q: Query<(&OrbitCam, &mut WalkCam)>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }
    *mode = match *mode {
        CameraMode::Orbit => {
            if let Ok((orbit, mut walk)) = q.single_mut() {
                // the surface point under the orbit camera = its position direction
                walk.dir = Vec3::new(
                    orbit.pitch.cos() * orbit.yaw.cos(),
                    orbit.pitch.sin(),
                    orbit.pitch.cos() * orbit.yaw.sin(),
                )
                .normalize_or_zero();
                walk.yaw = 0.0;
                walk.pitch = 0.0;
                walk.eye_alt = WALK_EYE;
                // arrive in good light: snap the sky to ~mid-morning (sun ~45deg up, not overhead) so shadows
                // are immediately visible (overhead noon casts them straight down = invisible). [ ] \ to scrub.
                sun_offset.0 = noon_offset(walk.dir, gen.tick) - (crate::sphere::DAY_TICKS as i64) / 8;
            }
            selected.follow = false;
            info!("camera: WALK mode (WASD move, arrows/right-drag look, Shift run, swim into the sea: look + W to dive, [ ] scrub time, \\ noon, TAB to orbit)");
            CameraMode::Walk
        }
        CameraMode::Walk => {
            sun_offset.0 = 0; // back to true sim time for the orbit view
            info!("camera: ORBIT mode");
            CameraMode::Orbit
        }
    };
}

// --- orbit mode ---

// Hold right mouse + move to orbit. No cursor lock (a globe orbit reads better as a drag).
fn orbit_drag(
    mode: Res<CameraMode>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    selected: Res<Selected>,
    mut q: Query<&mut OrbitCam>,
) {
    if *mode != CameraMode::Orbit || selected.follow || !buttons.pressed(MouseButton::Right) {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    cam.yaw -= motion.delta.x * 0.005;
    cam.pitch = (cam.pitch + motion.delta.y * 0.005).clamp(-1.45, 1.45);
}

// Keyboard fallback: A/D orbit longitude, W/S zoom in/out, Q/E tilt latitude. Shift = faster.
fn orbit_keys(mode: Res<CameraMode>, keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, selected: Res<Selected>, mut q: Query<&mut OrbitCam>) {
    if *mode != CameraMode::Orbit || selected.follow {
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
fn zoom(mode: Res<CameraMode>, scroll: Res<AccumulatedMouseScroll>, selected: Res<Selected>, mut q: Query<&mut OrbitCam>) {
    if *mode != CameraMode::Orbit || selected.follow || scroll.delta.y == 0.0 {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    cam.dist = (cam.dist - scroll.delta.y * 12.0).clamp(MIN_DIST, MAX_DIST);
}

// Place the camera from (yaw, pitch, dist) around the planet center, looking inward.
fn apply_orbit(mode: Res<CameraMode>, selected: Res<Selected>, mut q: Query<(&mut Transform, &OrbitCam)>) {
    if *mode != CameraMode::Orbit || selected.follow {
        return; // walk owns the transform in walk mode; follow_camera owns it while following
    }
    let Ok((mut t, cam)) = q.single_mut() else { return };
    let dir = Vec3::new(cam.pitch.cos() * cam.yaw.cos(), cam.pitch.sin(), cam.pitch.cos() * cam.yaw.sin());
    t.translation = dir * cam.dist;
    t.look_at(Vec3::ZERO, Vec3::Y);
}

// --- walk mode ---

// Move over the surface: on land W/S forward+back along heading, A/D strafe (great-circle steps glued to
// the planet, never up/down). Over OCEAN you SWIM/fly: W/S follow the look direction so pitching down +
// W dives and pitching up + W rises; A/D strafe horizontally; eye_alt is clamped between the seafloor and
// just above the sea surface. Arrows turn/look in both modes, Shift runs/sprints.
fn walk_move(mode: Res<CameraMode>, keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut q: Query<&mut WalkCam>) {
    if *mode != CameraMode::Walk {
        return;
    }
    let Ok(mut w) = q.single_mut() else { return };
    let dt = time.delta_secs();
    let boost = if keys.pressed(KeyCode::ShiftLeft) { 2.5 } else { 1.0 };
    let dist = WALK_SPEED * dt * boost;
    use std::f32::consts::FRAC_PI_2;
    // look (both modes)
    if keys.pressed(KeyCode::ArrowLeft) { w.yaw -= WALK_TURN * dt; }
    if keys.pressed(KeyCode::ArrowRight) { w.yaw += WALK_TURN * dt; }
    if keys.pressed(KeyCode::ArrowUp) { w.pitch = (w.pitch + WALK_TURN * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT); }
    if keys.pressed(KeyCode::ArrowDown) { w.pitch = (w.pitch - WALK_TURN * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT); }
    if crate::sphere::is_ocean(w.dir) {
        // swim: split forward into horizontal (advances along the great circle) + vertical (changes depth)
        let (ch, sv) = (w.pitch.cos(), w.pitch.sin());
        if keys.pressed(KeyCode::KeyW) { w.dir = crate::sphere::step(w.dir, w.yaw, dist * ch).0; w.eye_alt += dist * sv; }
        if keys.pressed(KeyCode::KeyS) { w.dir = crate::sphere::step(w.dir, w.yaw, -dist * ch).0; w.eye_alt -= dist * sv; }
        if keys.pressed(KeyCode::KeyA) { w.dir = crate::sphere::step(w.dir, w.yaw - FRAC_PI_2, dist).0; }
        if keys.pressed(KeyCode::KeyD) { w.dir = crate::sphere::step(w.dir, w.yaw + FRAC_PI_2, dist).0; }
        let water_top = crate::sphere::SEA_LEVEL * crate::sphere::ELEV_MAX; // sea surface height over the ocean floor
        w.eye_alt = w.eye_alt.clamp(SWIM_FLOOR, water_top + SWIM_CEIL);
    } else {
        w.eye_alt = WALK_EYE; // on land (or wading out of the sea): stand at a fixed eye height
        if keys.pressed(KeyCode::KeyW) { w.dir = crate::sphere::step(w.dir, w.yaw, dist).0; }
        if keys.pressed(KeyCode::KeyS) { w.dir = crate::sphere::step(w.dir, w.yaw, -dist).0; }
        if keys.pressed(KeyCode::KeyA) { w.dir = crate::sphere::step(w.dir, w.yaw - FRAC_PI_2, dist).0; }
        if keys.pressed(KeyCode::KeyD) { w.dir = crate::sphere::step(w.dir, w.yaw + FRAC_PI_2, dist).0; }
    }
}

// Right-drag to look around (yaw + pitch) in walk mode.
fn walk_look(mode: Res<CameraMode>, buttons: Res<ButtonInput<MouseButton>>, motion: Res<AccumulatedMouseMotion>, mut q: Query<&mut WalkCam>) {
    if *mode != CameraMode::Walk || !buttons.pressed(MouseButton::Right) {
        return;
    }
    let Ok(mut w) = q.single_mut() else { return };
    w.yaw += motion.delta.x * 0.005;
    w.pitch = (w.pitch - motion.delta.y * 0.005).clamp(-PITCH_LIMIT, PITCH_LIMIT);
}

// Build the walk transform: eye = a fixed height above the terrain at `dir` (local up = radial), looking
// along the heading tangent tilted by pitch. Riding `surface_pos` means the eye climbs/descends with hills.
fn apply_walk(mode: Res<CameraMode>, mut q: Query<(&mut Transform, &WalkCam)>) {
    if *mode != CameraMode::Walk {
        return;
    }
    let Ok((mut t, w)) = q.single_mut() else { return };
    let d = w.dir.normalize_or_zero();
    let up = d; // local up = straight away from the planet center
    let eye = crate::sphere::surface_pos(d, w.eye_alt); // eye_alt = WALK_EYE on land, swim depth in water
    let tangent = crate::sphere::heading_tangent(d, w.yaw);
    let forward = (tangent * w.pitch.cos() + up * w.pitch.sin()).normalize_or_zero();
    t.translation = eye;
    t.look_to(forward, up);
}

// --- shared ---

// Lighting + shadows per mode. Shadows now work in BOTH modes (O toggles): the old "eclipse disc" in orbit
// was the globe/celestial self-shadow blackout bounded by the cascade range -> fixed by marking the globe,
// ocean, sun disc, moon + stars NotShadowCaster, so only trees/creatures cast and there's no dark disc.
// The cascade is swapped per mode: walk packs 4 tight cascades (max 130) for crisp ground shadows at eye
// level; orbit uses a wide cascade (max 900) so casters across the whole near hemisphere are covered from
// the far camera. Walk ambient tracks daylight in viz::walk_ambient; orbit keeps a steady fill.
fn update_shadow_mode(
    mode: Res<CameraMode>,
    show_shadows: Res<crate::viz::ShowShadows>,
    mut lights: Query<&mut DirectionalLight, With<SunLight>>,
    mut ambient: Query<&mut AmbientLight>,
    mut filter: Query<&mut bevy::light::ShadowFilteringMethod>,
) {
    if !mode.is_changed() && !show_shadows.is_changed() {
        return;
    }
    let walk = *mode == CameraMode::Walk;
    for mut l in &mut lights {
        l.shadows_enabled = show_shadows.0; // real shadows in walk AND orbit now (no more eclipse disc)
        // the planet now casts in both modes; on the surface the curved globe self-shadows the very ground
        // the eye stands on -> needs a heftier normal bias to push the receiver off its own caster (no acne),
        // while orbit (large-scale terminator, distant view) keeps the lighter bias for crisp object shadows.
        l.shadow_normal_bias = if walk { 3.2 } else { 1.8 };
    }
    // orbit shadows are coarse (far camera, wide cascade) -> soften with multi-tap Gaussian PCF; walk stays
    // crisp (Hardware2x2) since the tight eye-level cascade already resolves sharp ground shadows.
    for mut f in &mut filter {
        *f = if walk { bevy::light::ShadowFilteringMethod::Hardware2x2 } else { bevy::light::ShadowFilteringMethod::Gaussian };
    }
    // orbit gets a steady ambient for a crisp terminator; walk's ambient tracks daylight in viz::walk_ambient
    if !walk {
        for mut a in &mut ambient {
            a.brightness = 220.0;
        }
    }
}

// Size the sun's shadow cascade to the active view each frame. Walk packs 4 tight cascades (max 130) for
// crisp ground shadows at eye level. Orbit scales the cascade to the zoom: the camera sits `dist` from the
// planet center, so the visible near hemisphere spans up to ~dist+R in view depth -> set max to track it
// so shadows stay sharp zoomed-in AND still cover the globe zoomed-out (no fixed coarse cascade).
fn update_shadow_cascade(
    mode: Res<CameraMode>,
    orbit: Query<&OrbitCam>,
    mut lights: Query<&mut bevy::light::CascadeShadowConfig, With<SunLight>>,
) {
    let builder = if *mode == CameraMode::Walk {
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: 0.3,
            maximum_distance: 130.0,
            first_cascade_far_bound: 12.0,
            overlap_proportion: 0.2,
        }
    } else {
        let dist = orbit.single().map(|o| o.dist).unwrap_or(230.0);
        let r = crate::sphere::PLANET_R;
        let near = (dist - r - 30.0).max(0.5); // shadows start just in front of the near surface
        let far = dist + r + 20.0; // reach the far edge of the visible near hemisphere
        // first split must sit strictly between near + far (bevy asserts minimum_distance < first bound)
        let first = near + (far - near) * 0.4;
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: near,
            maximum_distance: far,
            first_cascade_far_bound: first,
            overlap_proportion: 0.2,
        }
    };
    let cascade = builder.build();
    for mut c in &mut lights {
        *c = cascade.clone();
    }
}

// Toggle the planet globe's shadow-caster status (runs on mode/shadow change). The globe CASTS in BOTH
// orbit AND walk now (user: both views need the planet casting), so it shadows its own night side -- the
// far hemisphere + the terrain just past the local horizon at dawn/dusk fall into the planet's own shadow
// instead of catching the sun "through" the planet. Walk's curved-terrain self-shadow acne (the old reason
// this was orbit-only) is countered by a higher shadow_normal_bias set per-mode in update_shadow_mode.
fn update_planet_caster(
    mode: Res<CameraMode>,
    show_shadows: Res<crate::viz::ShowShadows>,
    planet: Query<Entity, With<crate::viz::Planet>>,
    mut commands: Commands,
) {
    if !mode.is_changed() && !show_shadows.is_changed() {
        return;
    }
    let _ = mode; // cast in both modes; only gated by the master shadow toggle
    let cast = show_shadows.0;
    for e in &planet {
        if cast {
            commands.entity(e).remove::<bevy::light::NotShadowCaster>();
        } else {
            commands.entity(e).insert(bevy::light::NotShadowCaster);
        }
    }
}

// Follow the selected entity (toggle with F, orbit mode): keep a fixed offset and track it. Stops if dead.
fn follow_camera(
    mode: Res<CameraMode>,
    keys: Res<ButtonInput<KeyCode>>,
    mut selected: ResMut<Selected>,
    targets: Query<&GlobalTransform>,
    mut cam: Query<&mut Transform, With<OrbitCam>>,
) {
    if *mode != CameraMode::Orbit {
        return;
    }
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
