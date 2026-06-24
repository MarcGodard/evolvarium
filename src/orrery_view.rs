//! Orrery view: a second "space" reproducing the TSN / tychos.space solar system. Tab cycles Orbit ->
//! Orrery -> Walk. Renders every visible TSN body (sun, planets, moons, Pluto, Halley, Eros) as a colored
//! sphere positioned each frame by the orrery model (orrery::body_positions). Lives at a FAR origin
//! (ORRERY_CENTER) so it never collides with the planet world: in orrery mode the camera flies out there and
//! the planet falls beyond the far clip; in planet modes the orrery is beyond the far clip. So no visibility
//! toggling needed. Orbit traces, BSC starfield, ecliptic grid, labels come in later passes.
//!
//! Runs on SIM time (sphere::t_years(gen.tick)) so the orrery, the planet sky, and the calendar always agree
//! on the moment. Toggleable overlays: orbit traces (T), ecliptic grid (G), zodiac (Z), labels (B),
//! constellations (L). Bodies use constant-angular-size rendering so even the moon/asteroids stay visible.
use crate::camera::CameraMode;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use std::f32::consts::TAU;

// Far parking spot for the orrery scene (30k units from the planet at origin). Camera far clip ~12k, so the
// two scenes are mutually beyond each other's clip = clean separation.
pub const ORRERY_CENTER: Vec3 = Vec3::new(0.0, 0.0, 30000.0);
// Star/constellation shell radius (units from ORRERY_CENTER). Inside the 12k far clip from a near camera.
const STAR_SHELL: f32 = 9000.0;

pub struct OrreryViewPlugin;
impl Plugin for OrreryViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowConstellations>()
            .init_resource::<Overlays>()
            .init_resource::<OrreryFocus>()
            .init_resource::<GeoCentric>()
            .add_systems(Startup, (spawn_orrery_bodies, spawn_starfield, spawn_overlays, spawn_sky_labels))
            .add_systems(
                Update,
                (
                    update_orrery_focus,
                    toggle_geocentric,
                    position_orrery_bodies,
                    size_sirius,
                    pick_orrery,
                    toggle_constellations,
                    orrery_scene_visibility,
                    constellation_visibility,
                    toggle_overlays,
                    overlay_visibility,
                    update_sky_labels,
                ),
            );
    }
}

// All orrery scenery (bodies + starfield). Visible ONLY in Orrery mode so the far solar-system scene never
// bleeds into the planet's orbit/walk sky (the camera far clip alone does not separate them reliably).
#[derive(Component)]
struct OrreryScenery;

#[derive(Resource, Default)]
struct ShowConstellations(bool); // L toggles; only meaningful in Orrery mode

fn orrery_scene_visibility(mode: Res<CameraMode>, mut q: Query<&mut Visibility, With<OrreryScenery>>) {
    let want = if *mode == CameraMode::Orrery { Visibility::Visible } else { Visibility::Hidden };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

fn constellation_visibility(
    mode: Res<CameraMode>,
    show: Res<ShowConstellations>,
    mut q: Query<&mut Visibility, With<ConstellationLines>>,
) {
    let want = if *mode == CameraMode::Orrery && show.0 { Visibility::Visible } else { Visibility::Hidden };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

// --- toggleable orrery overlays: orbit traces (T), ecliptic grid (G), zodiac (Z), labels (B) ---

#[derive(Resource)]
struct Overlays {
    traces: bool,
    grid: bool,
    zodiac: bool,
    labels: bool,
}
impl Default for Overlays {
    fn default() -> Self {
        // labels ON by default so bodies + bright named stars are identifiable on arrival (B toggles).
        Overlays { traces: false, grid: false, zodiac: false, labels: true }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum OverlayKind {
    Traces,
    Grid,
    Zodiac,
}
#[derive(Component)]
struct OrreryOverlay(OverlayKind);
#[derive(Component)]
enum SkyLabel {
    Body(usize),
    Zodiac(usize),
    Star(usize), // index into StarCatalog: bright named stars (Capella, Pollux...)
    Sirius,      // the binary companion body (on the precession orbit), not a catalog dot
}

// Sirius (Earth's binary companion driving the 24,000-yr precession) rendered as a bright body far out along
// the apsidal line (sirius_dir). Distinct from the faint catalog Sirius; this is the modeled companion.
#[derive(Component)]
struct SiriusBody;
const D_SIRIUS: f32 = 7000.0; // render distance from the system center (inside the 9000 star shell)

const ZODIAC: [&str; 12] = [
    "Aries", "Taurus", "Gemini", "Cancer", "Leo", "Virgo", "Libra", "Scorpio", "Sagittarius", "Capricorn", "Aquarius", "Pisces",
];

fn line_mesh(positions: Vec<[f32; 3]>) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m
}
fn a3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

// Orbit/deferent traces: sample each visible body's path over ~its own period (capped) -> the looping
// epicyclic curves TSN/tychos.space show. Static (precession ~const within a session), built once.
fn build_orbit_traces() -> Mesh {
    let mut pos = Vec::new();
    for idx in 0..crate::orrery::body_count() {
        let (name, _, _) = crate::orrery::body_meta(idx);
        if !is_visible_body(name) {
            continue;
        }
        let speed = crate::orrery::body_speed(idx).abs();
        if speed < 1e-4 {
            continue;
        }
        let window = (TAU / speed).clamp(2.0, 30.0);
        let steps = 220;
        let mut prev: Option<Vec3> = None;
        for s in 0..=steps {
            let tau = window * s as f32 / steps as f32;
            let p = crate::orrery::body_positions(tau)[idx];
            if let Some(pp) = prev {
                pos.push(a3(pp));
                pos.push(a3(p));
            }
            prev = Some(p);
        }
    }
    line_mesh(pos)
}

// Ecliptic grid: the ecliptic circle (lat 0) + parallels at +/-30,60 deg + meridians every 30 deg longitude.
fn build_ecliptic_grid(r: f32) -> Mesh {
    let mut pos = Vec::new();
    let dir = |lon: f32, lat: f32| Vec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin()) * r;
    let segs = 128;
    for &lat in &[0.0f32, 30f32.to_radians(), -30f32.to_radians(), 60f32.to_radians(), -60f32.to_radians()] {
        for i in 0..segs {
            let a0 = TAU * i as f32 / segs as f32;
            let a1 = TAU * (i + 1) as f32 / segs as f32;
            pos.push(a3(dir(a0, lat)));
            pos.push(a3(dir(a1, lat)));
        }
    }
    for k in 0..12 {
        let lon = TAU * k as f32 / 12.0;
        let m = 48;
        for i in 0..m {
            let l0 = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / m as f32;
            let l1 = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * (i + 1) as f32 / m as f32;
            pos.push(a3(dir(lon, l0)));
            pos.push(a3(dir(lon, l1)));
        }
    }
    line_mesh(pos)
}

// Zodiac: the ecliptic band (lat +/-8 deg) divided into 12 signs by radial ticks at 30-deg longitudes.
fn build_zodiac_marks(r: f32) -> Mesh {
    let mut pos = Vec::new();
    let band = 8f32.to_radians();
    let dir = |lon: f32, lat: f32| Vec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin()) * r;
    // two band edges
    for &lat in &[band, -band] {
        let segs = 96;
        for i in 0..segs {
            let a0 = TAU * i as f32 / segs as f32;
            let a1 = TAU * (i + 1) as f32 / segs as f32;
            pos.push(a3(dir(a0, lat)));
            pos.push(a3(dir(a1, lat)));
        }
    }
    // division ticks
    for k in 0..12 {
        let lon = TAU * k as f32 / 12.0;
        pos.push(a3(dir(lon, band)));
        pos.push(a3(dir(lon, -band)));
    }
    line_mesh(pos)
}

fn spawn_overlays(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let r = STAR_SHELL * 0.9;
    let mat = |c: Color, materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial { base_color: c, unlit: true, ..default() })
    };
    let over = |mesh: Mesh, kind: OverlayKind, c: Color, commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>| {
        commands.spawn((
            OrreryOverlay(kind),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mat(c, materials)),
            Transform::from_translation(ORRERY_CENTER),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
        ));
    };
    over(build_orbit_traces(), OverlayKind::Traces, Color::srgb(0.35, 0.4, 0.55), &mut commands, &mut meshes, &mut materials);
    over(build_ecliptic_grid(r), OverlayKind::Grid, Color::srgb(0.2, 0.3, 0.28), &mut commands, &mut meshes, &mut materials);
    over(build_zodiac_marks(r * 0.99), OverlayKind::Zodiac, Color::srgb(0.45, 0.35, 0.5), &mut commands, &mut meshes, &mut materials);
}

// T traces, G grid, Z zodiac, B labels. Orrery-mode only (these keys do planet things otherwise).
fn toggle_overlays(keys: Res<ButtonInput<KeyCode>>, mode: Res<CameraMode>, mut ov: ResMut<Overlays>) {
    if *mode != CameraMode::Orrery {
        return;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        ov.traces = !ov.traces;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        ov.grid = !ov.grid;
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        ov.zodiac = !ov.zodiac;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        ov.labels = !ov.labels;
    }
}

fn overlay_visibility(mode: Res<CameraMode>, ov: Res<Overlays>, mut q: Query<(&OrreryOverlay, &mut Visibility)>) {
    let on = *mode == CameraMode::Orrery;
    for (o, mut v) in &mut q {
        let want = if on && match o.0 {
            OverlayKind::Traces => ov.traces,
            OverlayKind::Grid => ov.grid,
            OverlayKind::Zodiac => ov.zodiac,
        } {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
}

// World-space text labels (body names + zodiac signs) drawn as screen-projected UI. Hidden unless in Orrery
// mode with the matching overlay on (labels for bodies, zodiac toggle for signs).
fn spawn_sky_labels(mut commands: Commands) {
    let mk = |commands: &mut Commands, text: String, label: SkyLabel| {
        commands.spawn((
            label,
            Text::new(text),
            TextFont { font_size: 12.0, ..default() },
            TextColor(Color::srgb(0.8, 0.85, 0.95)),
            Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
        ));
    };
    for idx in 0..crate::orrery::body_count() {
        let (name, _, _) = crate::orrery::body_meta(idx);
        if is_visible_body(name) {
            mk(&mut commands, display_name(name).to_string(), SkyLabel::Body(idx));
        }
    }
    for (k, name) in ZODIAC.iter().enumerate() {
        mk(&mut commands, name.to_string(), SkyLabel::Zodiac(k));
    }
    mk(&mut commands, "Sirius".to_string(), SkyLabel::Sirius);
    // bright named stars (Capella, Pollux, Sirius's neighbors...): label the brightest with proper names.
    for (i, st) in crate::stars::star_catalog().iter().enumerate() {
        if st.name.is_some() && st.mag < 2.0 {
            mk(&mut commands, st.label(), SkyLabel::Star(i));
        }
    }
}

fn update_sky_labels(
    mode: Res<CameraMode>,
    ov: Res<Overlays>,
    gen: Res<crate::sim::GenState>,
    catalog: Option<Res<crate::viz::StarCatalog>>,
    cam: Query<(&Camera, &GlobalTransform), With<crate::camera::OrreryCam>>,
    mut q: Query<(&SkyLabel, &mut Node, &mut Visibility)>,
) {
    let on = *mode == CameraMode::Orrery;
    let Ok((camera, cam_tf)) = cam.single() else { return };
    let positions = if on { crate::orrery::body_positions(orrery_tau(gen.tick)) } else { Vec::new() };
    let r = STAR_SHELL * 0.9;
    for (label, mut node, mut vis) in &mut q {
        let (world, show) = match label {
            SkyLabel::Body(i) => {
                let p = if *i < positions.len() { body_render_pos(*i, &positions) } else { Vec3::ZERO };
                (ORRERY_CENTER + p, on && ov.labels)
            }
            SkyLabel::Zodiac(k) => {
                let lon = TAU * *k as f32 / 12.0 + TAU / 24.0; // center of each 30-deg sign
                (ORRERY_CENTER + Vec3::new(lon.cos(), 0.0, lon.sin()) * r, on && ov.zodiac)
            }
            SkyLabel::Sirius => (ORRERY_CENTER + crate::orrery::sirius_dir() * D_SIRIUS, on && ov.labels),
            SkyLabel::Star(i) => {
                let dir = catalog.as_ref().and_then(|c| c.0.get(*i)).map(|s| s.dir).unwrap_or(Vec3::Z);
                (ORRERY_CENTER + dir * STAR_SHELL, on && ov.labels)
            }
        };
        let mut visible = false;
        if show {
            if let Ok(screen) = camera.world_to_viewport(cam_tf, world) {
                node.left = Val::Px(screen.x + 6.0);
                node.top = Val::Px(screen.y);
                visible = true;
            }
        }
        let want = if visible { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

// Real Bright Star Catalog sky + constellation lines on a far shell around the solar system (TSN data).
fn spawn_starfield(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (star_mesh, hip_dir) = crate::stars::build_starfield(STAR_SHELL);
    let n_stars = star_mesh.count_vertices() / 4;
    commands.spawn((
        OrreryScenery,
        Mesh3d(meshes.add(star_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colors carry per-star temperature
            unlit: true,
            cull_mode: None, // star quads sit on a shell facing outward; camera looks out from inside -> no cull
            ..default()
        })),
        Transform::from_translation(ORRERY_CENTER),
        Visibility::Hidden, // shown only in Orrery mode (orrery_scene_visibility)
        bevy::light::NotShadowCaster,
    ));
    // Milky Way band, faint additive, on a slightly farther shell (galactic plane, real orientation).
    commands.spawn((
        OrreryScenery,
        Mesh3d(meshes.add(crate::stars::build_milky_way(STAR_SHELL * 1.01))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        })),
        Transform::from_translation(ORRERY_CENTER),
        Visibility::Hidden,
        bevy::light::NotShadowCaster,
    ));
    if let Some(lines) = crate::stars::build_constellation_lines(&hip_dir, STAR_SHELL) {
        commands.spawn((
            Mesh3d(meshes.add(lines)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.16, 0.22, 0.38), // faint blue constellation lines (subtle vs stars)
                unlit: true,
                ..default()
            })),
            Transform::from_translation(ORRERY_CENTER),
            Visibility::Hidden, // OFF by default; toggle with L (toggle_constellations)
            bevy::light::NotShadowCaster,
            ConstellationLines,
        ));
    }
    info!("orrery view: starfield {} BSC stars + constellation lines on shell r={}", n_stars, STAR_SHELL);
}

// Orrery time = SIM time so the orrery, planet sky, and calendar all show the SAME moment (motion is slow,
// at sim pace; use the in-sim speed controls to advance faster).
fn orrery_tau(tick: u32) -> f32 {
    crate::sphere::t_years(tick)
}

#[derive(Component)]
pub struct OrreryBody {
    pub idx: usize,
}

#[derive(Component)]
struct ConstellationLines;

// Toggle constellation lines with L (orrery-mode only; L is "lightning fire" on the planet).
fn toggle_constellations(keys: Res<ButtonInput<KeyCode>>, mode: Res<CameraMode>, mut show: ResMut<ShowConstellations>) {
    if *mode == CameraMode::Orrery && keys.just_pressed(KeyCode::KeyL) {
        show.0 = !show.0;
    }
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

// Home planet (TSN "Earth") is displayed as "Evolvarium".
pub fn display_name(name: &str) -> &str {
    if name == "Earth" {
        "Evolvarium"
    } else {
        name
    }
}

// --- view focus: TSN is geocentric, so center on Evolvarium by default (C toggles to the system center) ---
#[derive(Resource)]
pub struct OrreryFocus(pub Vec3); // world look-at point the orrery camera orbits
impl Default for OrreryFocus {
    fn default() -> Self {
        OrreryFocus(ORRERY_CENTER)
    }
}
#[derive(Resource)]
struct GeoCentric(bool);
impl Default for GeoCentric {
    fn default() -> Self {
        GeoCentric(true) // Earth/Evolvarium-centered by default, like TSN
    }
}

fn update_orrery_focus(mode: Res<CameraMode>, geo: Res<GeoCentric>, gen: Res<crate::sim::GenState>, mut focus: ResMut<OrreryFocus>) {
    if *mode != CameraMode::Orrery {
        return;
    }
    focus.0 = if geo.0 {
        ORRERY_CENTER + crate::orrery::body_positions(orrery_tau(gen.tick))[crate::orrery::EARTH]
    } else {
        ORRERY_CENTER
    };
}

fn toggle_geocentric(keys: Res<ButtonInput<KeyCode>>, mode: Res<CameraMode>, mut geo: ResMut<GeoCentric>) {
    if *mode == CameraMode::Orrery && keys.just_pressed(KeyCode::KeyC) {
        geo.0 = !geo.0;
    }
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
            OrreryScenery,
            Mesh3d(unit.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::from(col),
                emissive: col,
                unlit: true,
                ..default()
            })),
            Transform::from_translation(ORRERY_CENTER).with_scale(Vec3::splat(r)),
            Visibility::Hidden, // shown only in Orrery mode (orrery_scene_visibility)
            bevy::light::NotShadowCaster,
        ));
    }
    // Sirius: the binary companion, far out along the apsidal line. Bright blue-white (Sirius A is blue-white).
    commands.spawn((
        SiriusBody,
        OrreryScenery,
        Mesh3d(unit.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            emissive: LinearRgba::rgb(3.2, 3.5, 4.4),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(ORRERY_CENTER + crate::orrery::sirius_dir() * D_SIRIUS).with_scale(Vec3::splat(20.0)),
        Visibility::Hidden,
        bevy::light::NotShadowCaster,
    ));
    info!("orrery view: spawned TSN bodies + Sirius; press TAB to reach it (Orbit -> Orrery -> Walk)");
}

// Keep Sirius a visible size at any zoom (constant angular size, like the planets).
fn size_sirius(mode: Res<CameraMode>, cam: Query<&crate::camera::OrreryCam>, mut q: Query<&mut Transform, With<SiriusBody>>) {
    if *mode != CameraMode::Orrery {
        return;
    }
    let dist = cam.single().map(|c| c.dist).unwrap_or(1800.0);
    let r = body_render_radius(6.0, dist);
    for mut tf in &mut q {
        tf.scale = Vec3::splat(r);
    }
}

// Render radius for a body. Two regimes blended by max():
//  - far: a constant-ANGULAR floor (scales with distance) so even the tiny moon/asteroids stay visible dots;
//  - close: clamped to the body's TRUE size, so flying in makes it GROW on screen (real geometry) -> you can
//    actually zoom into Evolvarium until it fills the view, instead of it staying a fixed speck.
fn body_render_radius(size: f32, cam_dist: f32) -> f32 {
    let angular_floor = cam_dist * 0.0026 * size.clamp(0.25, 8.0).sqrt();
    angular_floor.max(size)
}

// The Moon's true orbit (0.255) is tiny vs body render sizes, so it would sit INSIDE Evolvarium. For the
// orrery VIEW we push it out so you can watch it circle the planet (render-only; the sky's moon direction
// in sim/planet views is unchanged). Earth-relative offset is scaled, keeping the orbital phase.
const MOON_VIS_EXAG: f32 = 40.0;
fn body_render_pos(idx: usize, pos: &[Vec3]) -> Vec3 {
    if idx == crate::orrery::MOON {
        let earth = pos[crate::orrery::EARTH];
        earth + (pos[idx] - earth) * MOON_VIS_EXAG
    } else {
        pos[idx]
    }
}

// Click-to-identify in the orrery: a body (precise sphere hit), else Sirius / nearest star (angular).
#[allow(clippy::too_many_arguments)]
fn pick_orrery(
    mouse: Res<ButtonInput<MouseButton>>,
    mode: Res<CameraMode>,
    gen: Res<crate::sim::GenState>,
    windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
    cam: Query<(&Camera, &GlobalTransform), With<crate::camera::OrreryCam>>,
    camd: Query<&crate::camera::OrreryCam>,
    bodies: Query<&OrreryBody>,
    catalog: Option<Res<crate::viz::StarCatalog>>,
    mut id: ResMut<crate::viz::Identified>,
) {
    if *mode != CameraMode::Orrery || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok((window, cursor_opts)) = windows.single() else { return };
    if cursor_opts.grab_mode != CursorGrabMode::None {
        return;
    }
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, cam_tf)) = cam.single() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else { return };
    let (o, d) = (ray.origin, *ray.direction);
    let pos = crate::orrery::body_positions(orrery_tau(gen.tick));
    let dist = camd.single().map(|c| c.dist).unwrap_or(1800.0);
    let mut best_body: Option<(f32, usize)> = None;
    for b in &bodies {
        let (_, size, _) = crate::orrery::body_meta(b.idx);
        let r = body_render_radius(size, dist).max(dist * 0.012); // generous pick disk
        if let Some(t) = crate::viz::ray_hit(o, d, ORRERY_CENTER + body_render_pos(b.idx, &pos), r) {
            if best_body.is_none_or(|(bt, _)| t < bt) {
                best_body = Some((t, b.idx));
            }
        }
    }
    if let Some((_, idx)) = best_body {
        *id = crate::viz::Identified::Body(idx);
        return;
    }
    // Sirius (usize::MAX sentinel) + nearest catalog star by angular proximity.
    let mut top = (d.dot((ORRERY_CENTER + crate::orrery::sirius_dir() * STAR_SHELL - o).normalize_or_zero()), usize::MAX);
    if let Some(cat) = &catalog {
        for (i, st) in cat.0.iter().enumerate() {
            let dot = d.dot((ORRERY_CENTER + st.dir * STAR_SHELL - o).normalize_or_zero());
            if dot > top.0 {
                top = (dot, i);
            }
        }
    }
    *id = if top.0 > 0.9994 {
        if top.1 == usize::MAX {
            crate::viz::Identified::Sirius
        } else {
            crate::viz::Identified::Star(top.1)
        }
    } else {
        crate::viz::Identified::None
    };
}

fn position_orrery_bodies(
    mode: Res<CameraMode>,
    gen: Res<crate::sim::GenState>,
    cam: Query<&crate::camera::OrreryCam>,
    mut q: Query<(&OrreryBody, &mut Transform)>,
) {
    if *mode != CameraMode::Orrery {
        return; // off-screen in other modes; skip the work
    }
    let pos = crate::orrery::body_positions(orrery_tau(gen.tick));
    let dist = cam.single().map(|c| c.dist).unwrap_or(1800.0);
    for (b, mut tf) in &mut q {
        tf.translation = ORRERY_CENTER + body_render_pos(b.idx, &pos);
        let (_, size, _) = crate::orrery::body_meta(b.idx);
        tf.scale = Vec3::splat(body_render_radius(size, dist));
    }
}
