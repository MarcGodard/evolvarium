// Two camera modes (TAB switches), render only.
//   ORBIT (default): right-drag rotate globe, scroll/W,S zoom, A/D spin, Q/E tilt. Left-click selects
//     creature/plant, F follows. Real shadows ON (O toggles). Old "eclipse disc" was self-shadow blackout,
//     fixed by marking globe + celestial bodies NotShadowCaster (see main.rs).
//   WALK: true ground walk, eye rides fixed height above terrain (climbs hills, never flies). WASD move
//     (W/S fwd+back, A/D strafe), arrows or right-drag look, Shift run. Walk into sea to swim (look + W
//     dive). Real shadows ON: tight cascade at eye level -> crisp tree/creature shadows.
use crate::viz::{Selected, SunLight, SunOffset};
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
                    orrery_drag,
                    orrery_zoom,
                    apply_orrery,
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

// Active camera. Orbit = planet space view; Orrery = TSN solar-system view; Walk = ground view (shadows).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    #[default]
    Orbit,
    Orrery,
    Walk,
}

// Orrery state: camera orbits the far solar-system center (orrery_view::ORRERY_CENTER), aimed inward.
#[derive(Component)]
pub struct OrreryCam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

// Orbit state: camera sits on sphere of radius `dist` around planet center, aimed inward.
#[derive(Component)]
pub struct OrbitCam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

// Walk state: stand on surface point (`dir`, unit), face compass `yaw`, look up/down with `pitch`. On land
// `eye_alt` locked to WALK_EYE, ride terrain (over hills, never fly). Over ocean free (swim): look + W
// dives/rises, clamped between seafloor + sea surface.
#[derive(Component)]
pub struct WalkCam {
    pub dir: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub eye_alt: f32, // eye height above terrain at `dir`; WALK_EYE on land, swim-controlled in water
}

const MIN_DIST: f32 = 95.0; // just above surface (planet radius ~80 + terrain)
const MAX_DIST: f32 = 420.0;
const WALK_EYE: f32 = 2.5; // eye height above terrain surface (rides elevation)
const WALK_SPEED: f32 = 14.0; // units/sec; Shift runs
const WALK_TURN: f32 = 1.6; // keyboard look speed (rad/sec)
const PITCH_LIMIT: f32 = 1.3;
const SWIM_FLOOR: f32 = 0.8; // swim: eye stays this far above seafloor (no clip-through)
const SWIM_CEIL: f32 = 2.0; // swim: head may breach this far above sea surface

fn spawn_camera(mut commands: Commands) {
    // start framed on homeland (founding population lives there)
    let hl = crate::sim::homeland_center();
    let (lon, lat) = crate::sphere::dir_to_lonlat(hl);
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        // Tonemapping: Bevy default (TonyMcMapface) is filmic path-to-white, desaturates bright areas toward
        // white, bleaches vivid plant/ground colors in daylight. ReinhardLuminance keeps hue + saturation in
        // highlights for punchy stylized look.
        bevy::core_pipeline::tonemapping::Tonemapping::ReinhardLuminance,
        // far clip pushed out so distant sun + starfield render (thousands of units away)
        Projection::from(PerspectiveProjection { far: 12000.0, ..default() }),
        // soft ambient (per-camera in 0.18) so night side not pitch black
        AmbientLight { brightness: 220.0, ..default() },
        OrbitCam { yaw: lon, pitch: lat.clamp(-1.3, 1.3), dist: 230.0 },
        OrreryCam { yaw: 0.6, pitch: 0.5, dist: 1500.0 }, // framed on the inner system (Sun..Jupiter)
        WalkCam { dir: hl.normalize_or_zero(), yaw: 0.0, pitch: 0.0, eye_alt: WALK_EYE },
        // shadow softness: swapped per mode in update_shadow_mode (orbit = soft Gaussian, walk = crisp)
        bevy::light::ShadowFilteringMethod::Hardware2x2,
        // all HUD/UI renders to THIS camera only -> the minimap's 2nd camera (viz.rs) shows just its globe,
        // not a duplicated HUD crammed into its viewport.
        bevy::ui::IsDefaultUiCamera,
    ));
}

// TAB toggles orbit <-> walk. Enter walk: drop onto surface point orbit camera was over, facing north,
// level. Entering either mode cancels follow.
fn toggle_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<CameraMode>,
    mut selected: ResMut<Selected>,
    mut sun_offset: ResMut<SunOffset>,
    mut q: Query<(&OrbitCam, &OrreryCam, &mut WalkCam)>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }
    // TAB cycles Orbit -> Orrery -> Walk -> Orbit.
    *mode = match *mode {
        CameraMode::Orbit => {
            selected.follow = false;
            info!("camera: ORRERY mode (TSN solar system: right-drag rotate, scroll zoom, TAB to walk)");
            CameraMode::Orrery
        }
        CameraMode::Orrery => {
            if let Ok((orbit, _orrery, mut walk)) = q.single_mut() {
                // drop walk onto the surface point the ORBIT camera was over (orbit dir), facing north.
                walk.dir = Vec3::new(
                    orbit.pitch.cos() * orbit.yaw.cos(),
                    orbit.pitch.sin(),
                    orbit.pitch.cos() * orbit.yaw.sin(),
                )
                .normalize_or_zero();
                walk.yaw = 0.0;
                walk.pitch = 0.0;
                walk.eye_alt = WALK_EYE;
                sun_offset.0 = 0; // true sim time on arrival (walk + orbit agree on time-of-day)
            }
            selected.follow = false;
            info!("camera: WALK mode (WASD move, arrows/right-drag look, Shift run, swim: look + W to dive, [ ] scrub time, \\ noon, TAB to orbit)");
            CameraMode::Walk
        }
        CameraMode::Walk => {
            sun_offset.0 = 0; // back to true sim time for orbit view
            info!("camera: ORBIT mode");
            CameraMode::Orbit
        }
    };
}

// --- orbit mode ---

// Hold right mouse + move to orbit. No cursor lock (globe orbit reads better as drag).
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

// Keyboard fallback: A/D orbit longitude, W/S zoom, Q/E tilt latitude. Shift = faster.
// Time<Real> (wall-clock), NOT Res<Time> (virtual): camera control must keep working while the sim is PAUSED
// (paused Time<Virtual> -> delta 0 -> frozen camera). User looks/walks around a paused world.
fn orbit_keys(mode: Res<CameraMode>, keys: Res<ButtonInput<KeyCode>>, time: Res<Time<bevy::time::Real>>, selected: Res<Selected>, mut q: Query<&mut OrbitCam>) {
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

fn zoom(mode: Res<CameraMode>, scroll: Res<AccumulatedMouseScroll>, selected: Res<Selected>, mut q: Query<&mut OrbitCam>) {
    if *mode != CameraMode::Orbit || selected.follow || scroll.delta.y == 0.0 {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    cam.dist = (cam.dist - scroll.delta.y * 12.0).clamp(MIN_DIST, MAX_DIST);
}

// Place camera from (yaw, pitch, dist) around planet center, looking inward.
fn apply_orbit(mode: Res<CameraMode>, selected: Res<Selected>, mut q: Query<(&mut Transform, &OrbitCam)>) {
    if *mode != CameraMode::Orbit || selected.follow {
        return; // walk owns transform in walk mode; follow_camera owns it while following
    }
    let Ok((mut t, cam)) = q.single_mut() else { return };
    let dir = Vec3::new(cam.pitch.cos() * cam.yaw.cos(), cam.pitch.sin(), cam.pitch.cos() * cam.yaw.sin());
    t.translation = dir * cam.dist;
    t.look_at(Vec3::ZERO, Vec3::Y);
}

// --- orrery mode (TSN solar system at the far ORRERY_CENTER) ---

const ORRERY_MIN_DIST: f32 = 60.0;   // close in on the inner bodies
const ORRERY_MAX_DIST: f32 = 8000.0; // whole system incl. outer planets (kept under the 12k far clip)

fn orrery_drag(
    mode: Res<CameraMode>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<bevy::time::Real>>,
    mut q: Query<&mut OrreryCam>,
) {
    if *mode != CameraMode::Orrery {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    if buttons.pressed(MouseButton::Right) {
        cam.yaw -= motion.delta.x * 0.005;
        cam.pitch = (cam.pitch + motion.delta.y * 0.005).clamp(-1.45, 1.45);
    }
    // keyboard fallback (A/D spin, Q/E tilt, W/S zoom)
    let dt = time.delta_secs();
    let boost = if keys.pressed(KeyCode::ShiftLeft) { 3.0 } else { 1.0 };
    if keys.pressed(KeyCode::KeyA) { cam.yaw += 0.8 * dt * boost; }
    if keys.pressed(KeyCode::KeyD) { cam.yaw -= 0.8 * dt * boost; }
    if keys.pressed(KeyCode::KeyQ) { cam.pitch = (cam.pitch + 0.8 * dt * boost).clamp(-1.45, 1.45); }
    if keys.pressed(KeyCode::KeyE) { cam.pitch = (cam.pitch - 0.8 * dt * boost).clamp(-1.45, 1.45); }
    let zk = 400.0 * dt * boost;
    if keys.pressed(KeyCode::KeyW) { cam.dist = (cam.dist - zk).clamp(ORRERY_MIN_DIST, ORRERY_MAX_DIST); }
    if keys.pressed(KeyCode::KeyS) { cam.dist = (cam.dist + zk).clamp(ORRERY_MIN_DIST, ORRERY_MAX_DIST); }
}

fn orrery_zoom(mode: Res<CameraMode>, scroll: Res<AccumulatedMouseScroll>, mut q: Query<&mut OrreryCam>) {
    if *mode != CameraMode::Orrery || scroll.delta.y == 0.0 {
        return;
    }
    let Ok(mut cam) = q.single_mut() else { return };
    // scroll zoom proportional to distance -> smooth from inner bodies out to Pluto
    cam.dist = (cam.dist * (1.0 - scroll.delta.y * 0.1)).clamp(ORRERY_MIN_DIST, ORRERY_MAX_DIST);
}

// Place camera around ORRERY_CENTER, looking inward at the solar system.
fn apply_orrery(mode: Res<CameraMode>, mut q: Query<(&mut Transform, &OrreryCam)>) {
    if *mode != CameraMode::Orrery {
        return;
    }
    let Ok((mut t, cam)) = q.single_mut() else { return };
    let dir = Vec3::new(cam.pitch.cos() * cam.yaw.cos(), cam.pitch.sin(), cam.pitch.cos() * cam.yaw.sin());
    let center = crate::orrery_view::ORRERY_CENTER;
    t.translation = center + dir * cam.dist;
    t.look_at(center, Vec3::Y);
}

// --- walk mode ---

// Move over surface: on land W/S fwd+back along heading, A/D strafe (great-circle steps glued to planet,
// never up/down). Over OCEAN swim/fly: W/S follow look direction (pitch down + W dives, pitch up + W rises),
// A/D strafe horizontally, eye_alt clamped between seafloor and just above sea surface. Arrows turn/look
// both modes, Shift sprints.
fn walk_move(
    mode: Res<CameraMode>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<bevy::time::Real>>, // wall-clock: walk must work while sim PAUSED (virtual time -> dt 0)
    trees: Query<&GlobalTransform, With<crate::components::Tree>>,
    mut q: Query<&mut WalkCam>,
) {
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
        // swim: split forward into horizontal (advances along great circle) + vertical (changes depth)
        let (ch, sv) = (w.pitch.cos(), w.pitch.sin());
        if keys.pressed(KeyCode::KeyW) { w.dir = crate::sphere::step(w.dir, w.yaw, dist * ch).0; w.eye_alt += dist * sv; }
        if keys.pressed(KeyCode::KeyS) { w.dir = crate::sphere::step(w.dir, w.yaw, -dist * ch).0; w.eye_alt -= dist * sv; }
        if keys.pressed(KeyCode::KeyA) { w.dir = crate::sphere::step(w.dir, w.yaw - FRAC_PI_2, dist).0; }
        if keys.pressed(KeyCode::KeyD) { w.dir = crate::sphere::step(w.dir, w.yaw + FRAC_PI_2, dist).0; }
        let water_top = (-crate::sphere::elevation(w.dir)).max(0.0); // local water depth: sea surface above seafloor
        w.eye_alt = w.eye_alt.clamp(SWIM_FLOOR, water_top + SWIM_CEIL);
    } else {
        w.eye_alt = WALK_EYE; // on land (or wading out of sea): fixed eye height
        if keys.pressed(KeyCode::KeyW) { w.dir = crate::sphere::step(w.dir, w.yaw, dist).0; }
        if keys.pressed(KeyCode::KeyS) { w.dir = crate::sphere::step(w.dir, w.yaw, -dist).0; }
        if keys.pressed(KeyCode::KeyA) { w.dir = crate::sphere::step(w.dir, w.yaw - FRAC_PI_2, dist).0; }
        if keys.pressed(KeyCode::KeyD) { w.dir = crate::sphere::step(w.dir, w.yaw + FRAC_PI_2, dist).0; }
    }
    // solid-world collision (land only): push out of tree trunks -> can't walk through them
    for gt in &trees {
        let tpos = gt.translation();
        let r = tpos.length();
        let tdir = tpos / r.max(1e-4);
        let off = w.dir - tdir;
        let d = off.length() * r; // tangent dist from trunk, world units
        let trunk_r = (0.5 + 0.22 * gt.scale().x).clamp(0.5, 1.6); // thicker block for bigger trees
        if d > 1e-4 && d < trunk_r {
            w.dir = (tdir + off / off.length() * (trunk_r / r)).normalize_or_zero(); // push to trunk edge
        }
    }
}

// Right-drag look (yaw + pitch), walk mode.
fn walk_look(mode: Res<CameraMode>, buttons: Res<ButtonInput<MouseButton>>, motion: Res<AccumulatedMouseMotion>, mut q: Query<&mut WalkCam>) {
    if *mode != CameraMode::Walk || !buttons.pressed(MouseButton::Right) {
        return;
    }
    let Ok(mut w) = q.single_mut() else { return };
    w.yaw += motion.delta.x * 0.005;
    w.pitch = (w.pitch - motion.delta.y * 0.005).clamp(-PITCH_LIMIT, PITCH_LIMIT);
}

// Build walk transform: eye = fixed height above terrain at `dir` (local up = radial), looking along
// heading tangent tilted by pitch. Riding `surface_pos` = eye climbs/descends with hills.
fn apply_walk(mode: Res<CameraMode>, mut q: Query<(&mut Transform, &WalkCam)>) {
    if *mode != CameraMode::Walk {
        return;
    }
    let Ok((mut t, w)) = q.single_mut() else { return };
    let d = w.dir.normalize_or_zero();
    let up = d; // local up = straight away from planet center
    let eye = crate::sphere::surface_pos(d, w.eye_alt); // eye_alt = WALK_EYE on land, swim depth in water
    let tangent = crate::sphere::heading_tangent(d, w.yaw);
    let forward = (tangent * w.pitch.cos() + up * w.pitch.sin()).normalize_or_zero();
    t.translation = eye;
    t.look_to(forward, up);
}

// --- shared ---

// Lighting + shadows per mode. Shadows work in BOTH modes (O toggles). Old "eclipse disc" in orbit was
// globe/celestial self-shadow blackout bounded by cascade range -> fixed by marking globe, ocean, sun disc,
// moon + stars NotShadowCaster, so only trees/creatures cast, no dark disc. Cascade swapped per mode (see
// update_shadow_cascade): walk = 4 tight cascades for crisp ground shadows at eye level; orbit = wide
// cascade so casters across whole near hemisphere covered from far camera. Walk ambient tracks daylight in
// viz::walk_ambient; orbit keeps steady fill.
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
        l.shadows_enabled = show_shadows.0; // real shadows in walk AND orbit (no eclipse disc)
        // planet casts in both modes. On surface curved globe self-shadows ground eye stands on -> needs
        // heftier normal bias to push receiver off own caster (no acne). Orbit (large-scale terminator,
        // distant view) keeps lighter bias for crisp object shadows.
        l.shadow_normal_bias = if walk { 3.2 } else { 1.8 };
    }
    // orbit shadows coarse (far camera, wide cascade) -> soften w/ multi-tap Gaussian PCF; walk stays crisp
    // (Hardware2x2) since tight eye-level cascade already resolves sharp ground shadows.
    for mut f in &mut filter {
        *f = if walk { bevy::light::ShadowFilteringMethod::Hardware2x2 } else { bevy::light::ShadowFilteringMethod::Gaussian };
    }
    // orbit gets steady ambient for crisp terminator; walk ambient tracks daylight in viz::walk_ambient
    if !walk {
        for mut a in &mut ambient {
            a.brightness = 220.0;
        }
    }
}

// Size sun shadow cascade to active view each frame. Walk = 4 tight cascades (max 130) for crisp ground
// shadows at eye level. Orbit scales cascade to zoom: camera sits `dist` from planet center, visible near
// hemisphere spans up to ~dist+R in view depth -> set max to track it so shadows stay sharp zoomed-in AND
// cover globe zoomed-out (no fixed coarse cascade).
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
        let near = (dist - r - 30.0).max(0.5); // shadows start just in front of near surface
        let far = dist + r + 20.0; // reach far edge of visible near hemisphere
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

// Toggle planet globe shadow-caster status (runs on mode/shadow change). Globe CASTS in BOTH orbit AND walk
// (both views need planet casting), so it shadows own night side: far hemisphere + terrain just past local
// horizon at dawn/dusk fall into planet's own shadow instead of catching sun "through" planet. Walk
// curved-terrain self-shadow acne (old reason this was orbit-only) countered by higher shadow_normal_bias
// set per-mode in update_shadow_mode.
fn update_planet_caster(
    mode: Res<CameraMode>,
    show_shadows: Res<crate::viz::ShowShadows>,
    planet: Query<Entity, With<crate::viz::Planet>>,
    mut commands: Commands,
) {
    if !mode.is_changed() && !show_shadows.is_changed() {
        return;
    }
    let _ = mode; // cast in both modes; only gated by master shadow toggle
    let cast = show_shadows.0;
    for e in &planet {
        if cast {
            commands.entity(e).remove::<bevy::light::NotShadowCaster>();
        } else {
            commands.entity(e).insert(bevy::light::NotShadowCaster);
        }
    }
}

// Follow selected entity (F toggles, orbit mode): keep fixed offset + track. Stops if target dead.
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
                    // sit above + outside target along its radial (third-person planet view)
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
