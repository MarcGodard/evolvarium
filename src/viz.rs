// Render-only viz: make evolved variability visible. NEVER touches sim state.
//   creature COLOR = dominant diet (hue) + rigidity (sat: specialist=vivid)
//   creature SIZE  = sensor count (more eyes -> bigger body)
//   G = draw sensor rays (evolved eye layout)
use bevy::prelude::*;

use crate::components::{Alive, Creature, DietState, Energy, Fitness, Food, Grass, Heading, Rot, Seaweed, Seed, Tree};
use crate::genome::{master_expression, Genome, NUTRIENTS};
use crate::plant::{flower_color, form, plant_color, PlantGenome, PlantState};
use crate::sim::{grid_cell_surface, Fire, GenState, GroundWater, EYE_MIN, EYE_SPAN, LIMB_MIN, LIMB_SPAN, ROT_GONE};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

// Visual time-of-day offset (ticks) added to sun ONLY for lighting + sun/moon sky. Sim daylight
// (creature rest, plant growth) still reads raw tick. Lets walk snap to local noon + scrub sun for
// low-angle shadows without fast-forwarding sim. 0 = sky matches sim time (orbit default).
#[derive(Resource, Default)]
pub struct SunOffset(pub i64);

// Walk shadow toggle (O). Directional shadow range can black out ground receiver -> shadows opt-in.
// camera::update_shadow_mode reads this.
#[derive(Resource, Default)]
pub struct ShowShadows(pub bool);

// True while walk eye submerged below sea surface. Drives blue tint overlay + murky sky.
#[derive(Resource, Default)]
pub struct Underwater(pub bool);

// Ocean shell entity. animate_ocean breathes slow swell on it.
#[derive(Component)]
pub struct Ocean;

// A flapping bird wing (child of a flier). flap_wings rotates it about its root (the shoulder) on the
// forward (Z) axis. `side` (+1 right / -1 left) mirrors lift so both tips rise together; `freq` set from
// body size (small bird = fast flutter, big bird = slow beats); `rest` = base transform (rotation reapplied
// each frame so the wing returns to neutral between beats).
#[derive(Component)]
pub struct Wing {
    pub side: f32,
    pub freq: f32,
    pub amp: f32,
    pub rest: Transform,
}

// Planet globe entity. Casts shadow in BOTH camera modes (camera::update_planet_caster) -> planet
// shadows own night side (no sun through planet) in orbit, and in walk terrain past local horizon
// falls into planet shadow at dawn/dusk. Walk self-shadow acne held off by per-mode shadow_normal_bias
// (camera::update_shadow_mode).
#[derive(Component)]
pub struct Planet;

// Full-screen translucent blue node, shown only when eye underwater -> tints whole frame blue.
#[derive(Component)]
struct UnderwaterTint;

// Celestial body markers. Animated by day_night_lighting.
#[derive(Component)]
pub struct SunLight;
#[derive(Component)]
pub struct Moon;
#[derive(Component)]
pub struct SunDisc; // visible glowing sun (follows light direction)
#[derive(Component)]
pub struct SkyStars; // real BSC starfield shell; wheels with the day (rotate_sky_stars)
#[derive(Component)]
pub struct SkyPlanet {
    pub idx: usize, // orrery body index; positioned each frame against the stars (position_sky_planets)
}
// Auroral magnetic latitude (radians, ~66 deg): curtains sit at this |mag latitude|. Shared spawn + anim.
pub const AURORA_LAT: f32 = 1.15;
// Aurora base altitude above surface (world units; PLANET_R=80). Curtains rise CURTAIN_H tall from here
// -> tops reach high like real aurora (~100..300 km).
pub const AURORA_LIFT: f32 = 4.0;
// One dancing curtain segment of auroral oval. Many per pole, each own random phase/drift/hue -> band
// ripples + glides + flickers, not one uniform ring. Animated by update_aurora_curtains.
#[derive(Component)]
pub struct AuroraCurtain {
    pub pole: Vec3,  // magnetic pole this curtain rings
    pub ang: f32,    // base angle around oval
    pub drift: f32,  // sideways glide rate (random sign/speed) -> curtains travel along band
    pub phase: f32,  // flicker phase offset
    pub width: f32,  // tangential width (random per curtain -> uneven ray structure)
    pub freq: f32,   // flicker frequency
}

pub struct VizPlugin;

impl Plugin for VizPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowSensors>()
            .init_resource::<Selected>()
            .init_resource::<ShowLegend>()
            .init_resource::<SunOffset>()
            .init_resource::<Underwater>()
            .init_resource::<Phylogeny>()
            .init_resource::<ShowPhylo>()
            .insert_resource(ShowShadows(true)) // shadows on by default (O toggles)
            .add_systems(Startup, (log_viz_help, spawn_stats_ui, spawn_world_stats_ui, spawn_legend_ui, spawn_daycycle_ui, spawn_underwater_tint, spawn_clouds, set_initial_speed, spawn_minimap, spawn_phylo_ui))
            .add_systems(
                Update,
                (
                    restyle_creatures,
                    add_creature_visuals,
                    toggle_sensors,
                    draw_sensors,
                    (add_plant_visuals, size_plants, add_grass_visuals, add_seaweed_visuals, size_creatures, flap_wings),
                    (day_night_lighting, time_of_day, toggle_shadows, walk_ambient, update_daycycle, track_underwater, update_sky, toggle_underwater_tint, animate_ocean, update_globe_climate, update_aurora_curtains, rotate_sky_stars, position_sky_planets),
                    rain_visuals,
                    fire_visuals,
                    update_clouds,
                    hide_dead,
                    color_carrion,
                    pick_on_click,
                    update_stats,
                    update_world_stats,
                    time_controls,
                    toggle_legend,
                    (god_disturbances, crate::sim::save_world_key, crate::sim::save_on_window_close),
                    draw_selection,
                    (minimap_sync_cam, minimap_visibility, hud_visibility, planet_sky_visibility, minimap_input, minimap_rebuild, minimap_dynamic),
                    (phylogeny_classify, toggle_phylo, update_phylo_panel),
                ),
            );
    }
}

// --- Corner inspector minimap: a real 3D globe in the bottom-right that ROTATES WITH the main view, colored
// by a chosen FIELD overlay ('M' cycles static biome/heat/moisture/elevation + live soil/water/fire/life). A 2nd camera renders a field-colored
// globe (on RenderLayers 1, so the MAIN camera never sees it) into a corner viewport; its orbit is synced to
// OrbitCam each frame so the minimap shows the same face you're looking at. Render-only; never touches sim.
// Fields 0..MM_STATIC are STATIC (position-only -> sampled once per switch). Fields >= MM_STATIC are
// DYNAMIC live overlays (soil fertility / groundwater / fire / creature density) -> rebuilt each frame from
// sim resources so the minimap shows the world changing in real time.
const MM_FIELDS: [&str; 9] = ["biome", "heat", "moisture", "elevation", "soil", "water", "fire", "life", "wear"];
const MM_STATIC: usize = 4; // first dynamic field index
const MM_RES: usize = 64; // globe lat bands (small: minimap is tiny)
const MM_SIZE: f32 = 200.0; // viewport square, logical px
const MM_MARGIN: f32 = 10.0;
const MM_DIST: f32 = 215.0; // minimap cam distance from globe center (PLANET_R=80 -> whole globe framed)
const MM_DENSITY_FULL: f32 = 5.0; // creatures-per-cell that reads as full "life" brightness
const MM_SOIL_MAX: f32 = 2.5; // soil overlay normalizer (> FERT_CAP so death-spike fertility shows above baseline)

#[derive(Resource)]
struct Minimap {
    field: usize,
    dirty: bool,
    mesh: Handle<Mesh>,
}

// Optional startup override for the minimap field (--cap-mmfield): lets the capture tool open straight on a
// chosen overlay (e.g. wear) so a slow live field can be screenshotted. Absent -> minimap starts on biome.
#[derive(Resource, Default)]
pub struct MinimapInitField(pub usize);

#[derive(Component)]
struct MinimapCam;
#[derive(Component)]
struct MinimapLabel;

// field value at a UNIT surface dir -> linear RGB. Ramps chosen for legibility (cold=blue/hot=red, dry=tan/wet=blue).
fn minimap_color(field: usize, d: Vec3) -> [f32; 3] {
    match field {
        0 => crate::sphere::biome_color(d),
        1 => {
            let t = crate::sphere::base_temperature(d).clamp(0.0, 1.0); // heat: cold blue -> warm red
            [t, 0.15 + 0.4 * (1.0 - (t - 0.5).abs() * 2.0).max(0.0), 1.0 - t]
        }
        2 => {
            let m = crate::sphere::moisture(d).clamp(0.0, 1.0); // moisture: dry tan -> wet blue
            [0.75 * (1.0 - m) + 0.1, 0.45 * (1.0 - m) + 0.35 * m + 0.2, 0.2 + 0.8 * m]
        }
        _ => {
            let e = crate::sphere::elevation01(d); // elevation: ocean depth -> land green -> peak white
            let sl = crate::sphere::SEA_LEVEL;
            if e < sl {
                let depth = (e / sl).clamp(0.0, 1.0);
                [0.0, 0.1 + 0.35 * depth, 0.4 + 0.5 * depth]
            } else {
                let h = ((e - sl) / (1.0 - sl)).clamp(0.0, 1.0);
                [0.3 + 0.7 * h, 0.5 + 0.5 * h, 0.3 + 0.7 * h]
            }
        }
    }
}

// Dynamic overlay color: dim biome base (keeps continents legible) blended toward a hot color by the live
// per-cell value `v` (0..1). field index is the MM_FIELDS slot (>= MM_STATIC). d = unit surface dir.
fn minimap_dynamic_color(field: usize, d: Vec3, v: f32) -> [f32; 3] {
    let base = crate::sphere::biome_color(d);
    let dim = [base[0] * 0.3, base[1] * 0.3, base[2] * 0.3];
    let hot = match field {
        4 => [0.25, 0.95, 0.25], // soil fertility: barren -> lush green
        5 => [0.15, 0.45, 1.0],  // groundwater: dry -> blue
        6 => [1.0, 0.55, 0.1],   // fire: dark -> burning orange
        7 => [1.0, 0.95, 0.3],   // life (creature density): empty -> bright yellow
        _ => [0.62, 0.42, 0.24], // wear: untrodden -> bare dirt-brown trail
    };
    let t = v.clamp(0.0, 1.0);
    [dim[0] + (hot[0] - dim[0]) * t, dim[1] + (hot[1] - dim[1]) * t, dim[2] + (hot[2] - dim[2]) * t]
}

fn spawn_minimap(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    init_field: Option<Res<MinimapInitField>>,
) {
    use bevy::camera::visibility::RenderLayers;
    let field0 = init_field.map_or(0, |f| f.0.min(MM_FIELDS.len() - 1));
    // static start field renders its color; dynamic fields recolor next frame in minimap_dynamic
    let static_field = if field0 < MM_STATIC { field0 } else { 0 };
    let mesh = meshes.add(crate::terrain::build_globe_colored(MM_RES, |d| minimap_color(static_field, d)));
    // unlit -> vertex field colors show flat + vivid (no day/night shading on the inspector globe)
    let mat = materials.add(StandardMaterial { base_color: Color::WHITE, unlit: true, ..default() });
    // field globe at origin, ONLY on layer 1 (the main camera renders layer 0, so it never sees this copy)
    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat), Transform::default(), RenderLayers::layer(1)));
    // 2nd camera: renders layer 1 into a corner viewport, drawn AFTER the main camera (order 1). Viewport rect
    // set each frame from the window size in minimap_sync_cam (handles resize).
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.02, 0.02, 0.06)),
            ..default()
        },
        Projection::from(PerspectiveProjection { far: 2000.0, ..default() }),
        Transform::from_xyz(0.0, 0.0, MM_DIST).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(1),
        MinimapCam,
    ));
    commands.insert_resource(Minimap { field: field0, dirty: true, mesh }); // dirty -> frame 1 syncs label to field
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(MM_SIZE + MM_MARGIN + 4.0), // label just under the top-right globe
            right: Val::Px(MM_MARGIN),
            ..default()
        })
        .with_child((
            Text::new("map: biome  [M]"),
            TextFont { font_size: 12.0, ..default() },
            TextColor(Color::srgb(0.85, 0.9, 1.0)),
            MinimapLabel,
        ));
}

// Sync the minimap globe to the main view: orbit the inspector camera around the field-globe using OrbitCam's
// yaw/pitch (same dir formula as the main orbit cam) -> minimap shows the same face. Also keep the corner
// viewport sized to the window.
fn minimap_sync_cam(
    orbit: Query<&crate::camera::OrbitCam>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cam: Query<(&mut Transform, &mut Camera), With<MinimapCam>>,
) {
    let Ok(o) = orbit.single() else { return };
    let Ok((mut tf, mut camera)) = cam.single_mut() else { return };
    let dir = Vec3::new(o.pitch.cos() * o.yaw.cos(), o.pitch.sin(), o.pitch.cos() * o.yaw.sin());
    *tf = Transform::from_translation(dir * MM_DIST).looking_at(Vec3::ZERO, Vec3::Y);
    if let Ok(w) = windows.single() {
        let sf = w.scale_factor();
        let (pw, ph) = (w.physical_width(), w.physical_height());
        let s = (MM_SIZE * sf) as u32;
        let m = (MM_MARGIN * sf) as u32;
        if pw > s + m && ph > s + m {
            camera.viewport = Some(bevy::camera::Viewport {
                physical_position: UVec2::new(pw - s - m, m), // top-right (clear of the left-anchored HUD)
                physical_size: UVec2::new(s, s),
                ..default()
            });
        }
    }
}

// Minimap is an ORBIT-view aid only: deactivate its camera + hide its label in orrery/walk modes.
fn minimap_visibility(
    mode: Res<crate::camera::CameraMode>,
    mut cam: Query<&mut Camera, With<MinimapCam>>,
    mut label: Query<&mut Visibility, With<MinimapLabel>>,
) {
    let show = *mode == crate::camera::CameraMode::Orbit;
    if let Ok(mut c) = cam.single_mut() {
        if c.is_active != show {
            c.is_active = show;
        }
    }
    if let Ok(mut v) = label.single_mut() {
        *v = if show { Visibility::Inherited } else { Visibility::Hidden };
    }
}

// Planet HUD (world stats, inspector, day cycle, legend) is irrelevant in the orrery solar-system view: hide
// it there so the sky reads clean. (Minimap handled separately by minimap_visibility.)
fn hud_visibility(
    mode: Res<crate::camera::CameraMode>,
    mut q: Query<&mut Visibility, Or<(With<WorldStatsText>, With<StatsText>, With<DayCycleText>, With<LegendText>)>>,
) {
    let want = if *mode == crate::camera::CameraMode::Orrery { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

// Planet sky (real starfield + wandering planets) belongs to the orbit/walk views; hide it in the orrery
// solar-system view so the two skies never overlap (the camera far clip alone does not separate them).
fn planet_sky_visibility(
    mode: Res<crate::camera::CameraMode>,
    mut q: Query<&mut Visibility, Or<(With<SkyStars>, With<SkyPlanet>)>>,
) {
    let want = if *mode == crate::camera::CameraMode::Orrery { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

fn minimap_input(keys: Res<ButtonInput<KeyCode>>, mut mm: ResMut<Minimap>) {
    if keys.just_pressed(KeyCode::KeyM) {
        mm.field = (mm.field + 1) % MM_FIELDS.len();
        mm.dirty = true;
    }
}

// On field-switch: update the label (all fields) + rebuild the globe ONCE for STATIC fields. Dynamic fields
// are rebuilt every frame by minimap_dynamic instead, so skip them here.
fn minimap_rebuild(mut mm: ResMut<Minimap>, mut meshes: ResMut<Assets<Mesh>>, mut q_label: Query<&mut Text, With<MinimapLabel>>) {
    if !mm.dirty {
        return;
    }
    mm.dirty = false;
    let field = mm.field;
    if field < MM_STATIC {
        if let Some(m) = meshes.get_mut(&mm.mesh) {
            *m = crate::terrain::build_globe_colored(MM_RES, |d| minimap_color(field, d));
        }
    }
    if let Ok(mut t) = q_label.single_mut() {
        t.0 = format!("map: {}  [M]", MM_FIELDS[field]);
    }
}

// Live overlays: when a DYNAMIC field is active, rebuild the globe each frame from sim resources. Builds a
// normalized 0..1 per-cell value grid (SOIL_RES^2), then recolors the globe by sampling it per vertex dir.
fn minimap_dynamic(
    mm: Res<Minimap>,
    mut meshes: ResMut<Assets<Mesh>>,
    soil: Res<crate::sim::Soil>,
    gw: Res<crate::sim::GroundWater>,
    fire: Res<crate::sim::Fire>,
    wear: Res<crate::sim::Wear>,
    creatures: Query<&Transform, With<Creature>>,
) {
    let field = mm.field;
    if field < MM_STATIC {
        return;
    }
    let n = crate::config::SOIL_RES * crate::config::SOIL_RES;
    let vals: Vec<f32> = match field {
        4 => soil.cell.iter().map(|&f| (f / MM_SOIL_MAX).clamp(0.0, 1.0)).collect(), // fertility 0..MM_SOIL_MAX (past FERT_CAP so death spikes show)
        5 => gw.cell.iter().map(|&w| w.clamp(0.0, 1.0)).collect(),                                // groundwater already 0..1
        6 => fire.cell.iter().map(|&f| f.clamp(0.0, 1.0)).collect(),                              // fire already 0..1
        8 => wear.cell.iter().map(|&w| w.clamp(0.0, 1.0)).collect(),                              // wear already 0..WEAR_CAP(=1)
        _ => {
            let mut d = vec![0.0f32; n]; // creature density: count per cell / MM_DENSITY_FULL
            for t in creatures.iter() {
                d[crate::sim::grid_cell(t.translation)] += 1.0;
            }
            d.iter().map(|&c| (c / MM_DENSITY_FULL).min(1.0)).collect()
        }
    };
    if let Some(m) = meshes.get_mut(&mm.mesh) {
        *m = crate::terrain::build_globe_colored(MM_RES, |dir| {
            minimap_dynamic_color(field, dir, vals[crate::sim::grid_cell(dir)])
        });
    }
}

// Per-form plant mesh library (inserted by spawn_world_render): one silhouette per plant::form ->
// flora reads as botanical mix (ferns, reeds, cacti, lily pads, kelp...) not identical balls.
// `forms` indexed by genome `form` byte; flower/berry/cap are shared embellishment meshes.
#[derive(Resource)]
pub struct PlantForms {
    pub forms: Vec<Handle<Mesh>>, // indexed by plant::form::*
    pub flower: Handle<Mesh>,     // bloom blob
    pub berry: Handle<Mesh>,      // fruit blob
    pub cap: Handle<Mesh>,        // mushroom cap
}

// Shared creature capsule mesh (inserted by spawn_world_render). add_creature_visuals dresses creatures
// born mid-sim (spawn_creature adds no mesh) -> newborns + B-seeded creatures become visible.
#[derive(Resource)]
pub struct CreatureMesh(pub Handle<Mesh>);

// Genetic body-part meshes (M4): head + eyes + legs as child entities -> head size, eye count, leg
// count visible. Base sizes ~unit; add_creature_visuals scales per genome.
#[derive(Resource)]
pub struct CreatureParts {
    pub head: Handle<Mesh>,
    pub eye: Handle<Mesh>,
    pub leg: Handle<Mesh>,
    pub fin: Handle<Mesh>, // Y-axis cone: caudal tail fan + dorsal ridge (scaled flat per use)
    pub seg: Handle<Mesh>, // unit cuboid: rod/bushy tails + flat pectoral side-fins (axis-aligned, no rotation)
    pub wing: Handle<Mesh>, // flat swept tapered bird wing, root at x=0 extends +X (mirror via scale.x). Flaps about root.
}

// Skin color + body-plan scale from genome (M4). Shared by add_creature_visuals + restyle_creatures ->
// newborns look right immediately. Color from skin_hue/skin_sat genes; venom -> aposematic orange-red
// warning + vivid sat; pelt (fur) mutes + lightens; armor darkens; swimmers shift cyan + fish body plan.
// Body scale = size x height (+ armor bulk), narrower/longer for swimmers.
fn creature_look(g: &Genome) -> (Color, Vec3) {
    let warn = g.venom.clamp(0.0, 1.0);
    let mut hue = g.skin_hue * 360.0 * (1.0 - warn) + 25.0 * warn; // venom -> warning orange/red (25 deg)
    hue = hue * (1.0 - 0.45 * g.swim) + 200.0 * (0.45 * g.swim); // swimmers tinted toward cyan but KEEP hue variety (reef-bright fish)
    let sat = ((0.25 + 0.6 * g.skin_sat + 0.4 * warn) * (1.0 - 0.4 * g.pelt)).clamp(0.0, 1.0); // venom vivid, fur muted
    let light = (0.5 + 0.15 * g.pelt - 0.12 * g.armor).clamp(0.2, 0.8); // fur lighter, armor darker
    let girth = (0.7 + 0.06 * g.n_sensors() as f32) * (0.6 + 0.9 * g.size) * (1.0 + 0.2 * g.armor);
    let slim = 1.0 - 0.45 * g.elongate; // long bodies slim their cross-section (eel/snake noodle, not a giant log)
    let sx = girth * (1.0 - 0.25 * g.swim) * slim;
    let sy = girth * (0.7 + 1.6 * g.height) * (1.0 - 0.3 * g.swim) * slim;
    let sz = girth * (1.0 + 0.8 * g.swim + 2.4 * g.elongate); // swim flatter+longer; elongate -> snake/eel length
    (Color::hsl(hue.rem_euclid(360.0), sat, light), Vec3::new(sx, sy, sz))
}

const CREATURE_BORN_SCALE: f32 = 0.45; // newborns render at 45% adult size
const CREATURE_MATURE_TICKS: f32 = 220.0; // grow to full size by this age (ticks)

// Creatures GROW IN over early life (juvenile -> adult), no pop-in at full size. Visual only: rescales
// whole body per frame by age factor (parts are children, scale with it); never touches genome `size`
// or combat. Recomputes genome target scale x born->1.0 age lerp -> composes with restyle_creatures
// (sets full scale on genome change); this just shrinks juveniles.
fn size_creatures(mut q: Query<(&DietState, &Genome, &mut Transform), With<Creature>>) {
    for (diet, g, mut tf) in &mut q {
        let grow = (CREATURE_BORN_SCALE + (1.0 - CREATURE_BORN_SCALE) * diet.age as f32 / CREATURE_MATURE_TICKS).min(1.0);
        tf.scale = creature_look(g).1 * grow;
    }
}

// Flap bird wings each frame: rotate every Wing about the forward (Z) axis = its shoulder root. `side` mirrors
// the angle so both tips rise together. Reapplies the rest transform first so flap is relative to neutral
// (translation/scale constant; rotation oscillates). Real-time (Time), independent of sim speed/pause.
fn flap_wings(time: Res<Time>, mut q: Query<(&Wing, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (w, mut tf) in &mut q {
        let a = (t * w.freq).sin() * w.amp;
        tf.translation = w.rest.translation;
        tf.scale = w.rest.scale;
        tf.rotation = Quat::from_rotation_z(w.side * a) * w.rest.rotation;
    }
}

// Dress any creature lacking a mesh: shared capsule + genome-colored material. Covers creatures BORN
// mid-sim + B-seeded (spawn_creature adds no render mesh). Without this: invisible while alive, appear
// only once dead (carrion gets own mesh).
fn add_creature_visuals(
    mut commands: Commands,
    mesh: Option<Res<CreatureMesh>>,
    parts: Option<Res<CreatureParts>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &Genome, &mut Transform), (With<Creature>, Without<Mesh3d>)>,
) {
    let (Some(mesh), Some(parts)) = (mesh, parts) else { return };
    for (e, g, mut tf) in &mut q {
        let (color, scale) = creature_look(g);
        tf.scale = scale;
        // children inherit parent non-uniform body scale; cancel per-part so head/eyes/legs not distorted.
        // part_tf places part at WORLD offset `wo` with WORLD dims `dim` (base meshes ~unit).
        let inv = Vec3::new(1.0 / scale.x.max(0.01), 1.0 / scale.y.max(0.01), 1.0 / scale.z.max(0.01));
        let part_tf = |wo: Vec3, dim: Vec3| Transform { translation: wo * inv, scale: dim * inv, ..default() };
        let body = 0.6 + 0.9 * g.size; // overall build factor
        let srgb = |c: Color, k: f32| {
            let s = c.to_srgba();
            Color::srgb(s.red * k, s.green * k, s.blue * k)
        };

        commands.entity(e).insert((Mesh3d(mesh.0.clone()), MeshMaterial3d(materials.add(color))));

        // HEAD: sphere up front + on top (local +Z, +Y), two-toned by pattern gene (marking).
        let head_d = (0.45 + 0.55 * g.head) * body;
        let head_y = 0.7 * scale.y;
        let head_z = 0.35 * scale.z + 0.45 * head_d;
        let head = commands
            .spawn((
                Mesh3d(parts.head.clone()),
                MeshMaterial3d(materials.add(srgb(color, 1.0 - 0.45 * g.pattern))),
                part_tf(Vec3::new(0.0, head_y, head_z), Vec3::splat(head_d)),
            ))
            .id();
        commands.entity(e).add_child(head);

        // EYES: 1..6 bright spheres proud of head front face (second row above 3 eyes).
        let n_eyes = (EYE_MIN + EYE_SPAN * g.eyes).round().clamp(1.0, 6.0) as usize;
        let eye_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.97, 0.98, 1.0),
            emissive: LinearRgba::rgb(0.5, 0.52, 0.6), // glow -> eyes read at distance
            ..default()
        });
        let eye_d = 0.34 * head_d;
        for k in 0..n_eyes {
            let frac = if n_eyes <= 1 { 0.0 } else { (k as f32 / (n_eyes - 1) as f32) * 2.0 - 1.0 };
            let row = if k >= 3 { 1.0 } else { 0.0 };
            let ex = frac * 0.30 * head_d;
            let ey = head_y + 0.10 * head_d + row * 0.26 * head_d;
            let ez = head_z + 0.46 * head_d; // proud on front of head sphere
            let eye = commands
                .spawn((Mesh3d(parts.eye.clone()), MeshMaterial3d(eye_mat.clone()), part_tf(Vec3::new(ex, ey, ez), Vec3::splat(eye_d))))
                .id();
            commands.entity(e).add_child(eye);
        }

        // BEAK/SNOUT: forward cone off the head front, length from g.beak. Reads as a beak on birds, a snout on
        // others. Cone apex is +Y, so rotate +Y -> +Z (from_rotation_x(PI/2)) to point it forward.
        if g.beak > 0.3 {
            let beak_len = (0.35 + 0.75 * g.beak) * head_d;
            let beak_r = 0.22 * head_d;
            let beak = commands
                .spawn((Mesh3d(parts.fin.clone()), MeshMaterial3d(materials.add(srgb(color, 0.7))), Transform {
                    translation: (Vec3::new(0.0, head_y + 0.02 * head_d, head_z + 0.5 * head_d + 0.45 * beak_len)) * inv,
                    rotation: Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                    scale: (Vec3::new(beak_r, beak_len, beak_r)) * inv,
                }))
                .id();
            commands.entity(e).add_child(beak);
        }

        // LIMBS: body local axes -> +Y up, +Z forward (facing), creature stands vertical on surface. Limb FORM
        // forks on body plan so aquatics stop looking like land animals:
        //   land     -> 2..8 legs poking down (current).
        //   swimmer + many limbs (>=6) -> octopus: long thin tentacles drooping from front-under the mantle.
        //   swimmer + few limbs        -> fish: flat pectoral side-fins (tail fin does the propulsion read).
        //   very elongate (snake/eel)  -> legless: body + tail carry it, no limbs at all.
        let n_legs = (LIMB_MIN + LIMB_SPAN * g.limbs).round().clamp(2.0, 8.0) as usize;
        let swimmer = g.swim > 0.45;
        let flyer = g.flight > 0.5; // bird silhouette wins over swim/land fork
        let legless = g.elongate > 0.75; // serpent: limbs vanish
        let leg_mat = materials.add(srgb(color, 0.55));
        let fin_mat = materials.add(srgb(color, 0.85));
        if flyer {
            // BIRD: a pair of swept tapered wings that FLAP (flap_wings rotates them about the shoulder root).
            // Wing root sits at x=0 in the mesh and extends outward; placing the entity at the shoulder + scaling
            // by span makes rotation pivot at the shoulder. scale.x sign mirrors the mesh for the left wing.
            // Wider span at higher flight gene; flap FREQ from size -> hummingbird (small) flutters, hawk (big) beats.
            let span = (1.0 + 0.9 * g.flight) * body; // wing reach (each side)
            let flap_freq = 6.0 + 16.0 * (1.0 - g.size.clamp(0.0, 1.0)); // small = fast flutter, big = slow beats
            let wing_mat = materials.add(StandardMaterial { base_color: srgb(color, 0.9), double_sided: true, cull_mode: None, perceptual_roughness: 0.95, ..default() });
            for side in [1.0f32, -1.0] {
                let shoulder = Vec3::new(side * 0.35 * scale.x, 0.22 * scale.y, -0.05 * scale.z);
                let rest = Transform {
                    translation: shoulder * inv,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(side * span, 0.04 * body, 0.85 * span) * inv, // thin Y, span out X, chord Z
                };
                let wing = commands
                    .spawn((Mesh3d(parts.wing.clone()), MeshMaterial3d(wing_mat.clone()), rest, Wing { side, freq: flap_freq, amp: 0.6, rest }))
                    .id();
                commands.entity(e).add_child(wing);
            }
            // small pair of tucked legs under the body (perch/landing read)
            let leg_len = 0.3 * body;
            for side in [1.0f32, -1.0] {
                let leg = commands
                    .spawn((Mesh3d(parts.leg.clone()), MeshMaterial3d(leg_mat.clone()), part_tf(Vec3::new(side * 0.2 * scale.x, -0.35 * scale.y - 0.5 * leg_len, 0.0), Vec3::new(0.08 * body, leg_len, 0.08 * body))))
                    .id();
                commands.entity(e).add_child(leg);
            }
            // tail fan at the rear: flat horizontal plate, spread wide in X, extends back in Z (steering fan).
            let tl = (0.4 + 0.7 * g.tail) * body;
            let tail = commands
                .spawn((Mesh3d(parts.seg.clone()), MeshMaterial3d(wing_mat.clone()), part_tf(Vec3::new(0.0, 0.05 * scale.y, -(0.45 * scale.z + 0.5 * tl)), Vec3::new(1.0 * tl, 0.04 * body, tl))))
                .id();
            commands.entity(e).add_child(tail);
        } else if legless {
            // no limbs
        } else if swimmer && n_legs >= 6 {
            // OCTOPUS: tentacles ring the front-lower body, hang down + slightly splayed, long + thin.
            let t_len = 1.5 * body;
            let t_r = 0.07 * body;
            for k in 0..n_legs {
                let a = (k as f32 / n_legs as f32) * std::f32::consts::TAU;
                let tx = a.cos() * 0.42 * scale.x;
                let tz = 0.2 * scale.z + a.sin() * 0.42 * scale.z; // biased forward (under the mantle)
                let cy = -0.3 * scale.y - 0.5 * t_len;
                let tent = commands
                    .spawn((Mesh3d(parts.leg.clone()), MeshMaterial3d(leg_mat.clone()), part_tf(Vec3::new(tx, cy, tz), Vec3::new(t_r, t_len, t_r))))
                    .id();
                commands.entity(e).add_child(tent);
            }
        } else if swimmer {
            // FISH: a pair (up to 4) of flat pectoral fins out the lower sides, swept slightly back.
            let n_fins = n_legs.min(4);
            let fw = 0.55 * body; // fin reach out from body
            for k in 0..n_fins {
                let side = if k % 2 == 0 { 1.0 } else { -1.0 };
                let fx = side * (0.5 * scale.x + 0.5 * fw);
                let fz = 0.05 * scale.z - (k / 2) as f32 * 0.25 * scale.z; // pairs march toward the tail
                let fy = -0.15 * scale.y;
                let fin = commands
                    .spawn((Mesh3d(parts.seg.clone()), MeshMaterial3d(fin_mat.clone()), part_tf(Vec3::new(fx, fy, fz), Vec3::new(fw, 0.05 * body, 0.42 * body))))
                    .id();
                commands.entity(e).add_child(fin);
            }
        } else {
            // LAND LEGS: 2..8 thin legs ringed around lower body sides, poking down. Longer for climbers.
            let leg_len = (0.45 + 0.45 * g.climb) * body;
            let leg_r = 0.10 * body;
            for k in 0..n_legs {
                let a = (k as f32 / n_legs as f32) * std::f32::consts::TAU;
                let lx = a.cos() * 0.52 * scale.x; // just outside body silhouette
                let lz = a.sin() * 0.52 * scale.z;
                let cy = -0.35 * scale.y - 0.5 * leg_len; // hang from lower body
                let leg = commands
                    .spawn((Mesh3d(parts.leg.clone()), MeshMaterial3d(leg_mat.clone()), part_tf(Vec3::new(lx, cy, lz), Vec3::new(leg_r, leg_len, leg_r)))).id();
                commands.entity(e).add_child(leg);
            }
        }

        // TAIL: at rear (-Z), size from g.tail. Form by body plan: swimmer -> vertical caudal fan (cone);
        // furry land -> bushy upswept tail (box, lighter); else -> thin rod tail (box). Lizard/rat/squirrel/fish.
        // Fliers handled above (tail fan) -> skip here so no double tail.
        if g.tail > 0.12 && !flyer {
            let tl = (0.3 + 0.9 * g.tail) * body; // tail length
            if swimmer {
                // caudal fin: vertical fan, thin in X, tall in Y, short Z, at the very back
                let cf_y = 0.0;
                let cf_z = -(0.45 * scale.z + 0.4 * tl);
                let caudal = commands
                    .spawn((Mesh3d(parts.fin.clone()), MeshMaterial3d(fin_mat.clone()), part_tf(Vec3::new(0.0, cf_y, cf_z), Vec3::new(0.08 * body, 1.3 * tl, 0.7 * tl))))
                    .id();
                commands.entity(e).add_child(caudal);
            } else if g.pelt > 0.45 {
                // bushy upswept tail (squirrel): fat soft ellipsoid box, raised, lighter (fur)
                let bush_mat = materials.add(srgb(color, 1.15));
                let bz = -(0.4 * scale.z + 0.4 * tl);
                let by = 0.25 * scale.y + 0.3 * tl; // arcs up
                let bush = commands
                    .spawn((Mesh3d(parts.seg.clone()), MeshMaterial3d(bush_mat), part_tf(Vec3::new(0.0, by, bz), Vec3::new(0.5 * body, 0.9 * tl, 0.5 * body))))
                    .id();
                commands.entity(e).add_child(bush);
            } else {
                // rod tail (rat/lizard/snake tip): thin box pointing back
                let rz = -(0.45 * scale.z + 0.5 * tl);
                let rod = commands
                    .spawn((Mesh3d(parts.seg.clone()), MeshMaterial3d(leg_mat.clone()), part_tf(Vec3::new(0.0, -0.1 * scale.y, rz), Vec3::new(0.12 * body, 0.12 * body, tl))))
                    .id();
                commands.entity(e).add_child(rod);
            }
        }

        // DORSAL FIN: triangular ridge along the spine (top +Y, mid body), thin in X, swept in Z. Sailfin/shark.
        // Skip on fliers (a spine sail on a bird reads wrong).
        if g.fin > 0.45 && !flyer {
            let df = (0.3 + 0.7 * g.fin) * body;
            let dorsal = commands
                .spawn((Mesh3d(parts.fin.clone()), MeshMaterial3d(fin_mat.clone()), part_tf(Vec3::new(0.0, 0.45 * scale.y + 0.4 * df, 0.0), Vec3::new(0.07 * body, df, 0.9 * df))))
                .id();
            commands.entity(e).add_child(dorsal);
        }
    }
}

// Tree part meshes (inserted by spawn_world_render): trunk + two canopy shapes.
#[derive(Resource)]
pub struct TreeMeshes {
    pub trunk: Handle<Mesh>,
    pub broadleaf: Handle<Mesh>, // round canopy for fruit trees
    pub conifer: Handle<Mesh>,   // cone canopy for evergreens
    pub vine: Handle<Mesh>,      // helix vine spiraling up trunk (only some trees)
}

// Dress any plant lacking a mesh: form silhouette + genome-colored material (hue=kind,
// brightness=nutrient, warmth=defense). Covers initial plants AND new offspring.
fn add_plant_visuals(
    mut commands: Commands,
    forms: Option<Res<PlantForms>>,
    trees: Option<Res<TreeMeshes>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<(Entity, &PlantGenome, Option<&Tree>, Option<&Seed>), (With<Food>, Without<Mesh3d>, Without<Grass>, Without<Seaweed>)>, // grass + seaweed have own visuals
) {
    let Some(forms) = forms else { return };
    for (e, g, tree, seed) in &q {
        // tree = brown trunk (this entity) + canopy child. Fruit trees get round broadleaf crown
        // (greener + hint of genome leaf hue), evergreens a dark cone. Trees ignore `form`.
        if let (Some(t), Some(tm)) = (tree, &trees) {
            commands.entity(e).insert((
                Mesh3d(tm.trunk.clone()),
                MeshMaterial3d(materials.add(Color::srgb(0.40, 0.26, 0.13))),
            ));
            // broadleaf crown centered (sits high in canopy); stacked-cone conifer base at y=0 rests on
            // trunk top (lower attach). Trunk centered (half-height 1.0); canopies attach to envelop most
            // of trunk, leaving short bare-trunk stub -> a tree, not a hat on a pole.
            let (canopy, cmat, cy) = if t.edible {
                (tm.broadleaf.clone(), materials.add(plant_color(g)), 1.0)
            } else {
                // conifer cones are open shells: double-sided so hollow shows dark-green inner face (no
                // see-through to trunk/sky); spine cone fills core. Evergreen needle-green: brighter
                // blue-green reads as foliage not black blob; roughness 0.6 near foliage default (0.5) so
                // sun catches soft sheen like broadleaf. Old 0.9 matte + dark base: noon sun never lit it.
                let m = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.16, 0.52, 0.30),
                    perceptual_roughness: 0.6,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                (tm.conifer.clone(), m, -0.6)
            };
            let child = commands
                .spawn((Mesh3d(canopy), MeshMaterial3d(cmat), Transform::from_xyz(0.0, cy, 0.0)))
                .id();
            commands.entity(e).add_child(child);
            // flowering (blossom) fruit tree gets ring of bloom blobs in crown
            if t.edible && g.flower > 0.4 {
                let fmat = materials.add(StandardMaterial {
                    base_color: flower_color(g),
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                for k in 0..5 {
                    let a = k as f32 * 1.2566; // 72 deg apart
                    let c = commands
                        .spawn((
                            Mesh3d(forms.flower.clone()),
                            MeshMaterial3d(fmat.clone()),
                            Transform::from_xyz(1.0 * a.cos(), 1.4, 1.0 * a.sin()).with_scale(Vec3::splat(0.85)),
                        ))
                        .id();
                    commands.entity(e).add_child(c);
                }
            }
            // FRUIT in the crown (matches sim eat model: fruit_height = height - branches*BRANCH_REACH).
            // Bare tree hangs fruit HIGH (top of canopy, only tall/climbers/fliers reach); branchy tree drops
            // it LOW (short ground creatures reach). Local y maps that drop; ring sits in the broadleaf crown.
            // Ripe red/orange, toxic fruit warns violet (fruit_toxicity, decoupled from body toxicity).
            if t.edible && g.fruiting > 0.3 {
                let (fruit, fem) = if g.fruit_toxicity > 0.5 {
                    (Color::srgb(0.62, 0.05, 0.78), LinearRgba::rgb(0.20, 0.0, 0.28)) // toxic: violet warning
                } else {
                    (Color::srgb(0.95, 0.30, 0.06), LinearRgba::rgb(0.32, 0.08, 0.0)) // ripe: orange-red, slight glow
                };
                let frmat = materials.add(StandardMaterial { base_color: fruit, emissive: fem, ..default() });
                let fy = 1.4 - 0.95 * g.branches.clamp(0.0, 1.0); // bare ~1.4 (top), full branches ~0.45 (low in branches)
                let n = 6;
                for k in 0..n {
                    let a = k as f32 * std::f32::consts::TAU / n as f32;
                    let r = 0.85;
                    let c = commands
                        .spawn((
                            Mesh3d(forms.berry.clone()),
                            MeshMaterial3d(frmat.clone()),
                            Transform::from_xyz(r * a.cos(), fy, r * a.sin()).with_scale(Vec3::splat(1.4)),
                        ))
                        .id();
                    commands.entity(e).add_child(c);
                }
            }
            // some trees host climbing vine up trunk (vine appears only WITH a tree). Presence keyed off
            // flower_hue gene > 0.58 -> deterministic + varied (~40% of trees).
            if g.flower_hue > 0.58 {
                let vmat = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.16, 0.45, 0.12),
                    perceptual_roughness: 0.85,
                    ..default()
                });
                let vine = commands.spawn((Mesh3d(tm.vine.clone()), MeshMaterial3d(vmat), Transform::IDENTITY)).id();
                commands.entity(e).add_child(vine);
            }
            continue;
        }
        // --- regular plant: silhouette by form, foliage color by genome ---
        let fi = (g.form as usize).min(forms.forms.len().saturating_sub(1));
        let leafy = matches!(
            g.form,
            form::FERN | form::REED | form::KELP | form::ROSETTE | form::LILYPAD | form::GROUNDCOVER | form::MOSS
        );
        // fallen FRUIT (carries Seed) renders as BRIGHT fruity blob (ripe colors) -> pops on ground +
        // reads as food, not foliage. Living plants use aposematic foliage color (toxicity).
        let body_color = if seed.is_some() {
            let c = flower_color(g).to_srgba(); // reuse bright genetic palette for varied ripe-fruit color
            Color::srgb(c.red, c.green, c.blue)
        } else {
            plant_color(g)
        };
        let mat = materials.add(StandardMaterial {
            base_color: body_color,
            perceptual_roughness: 0.9,
            emissive: if seed.is_some() { LinearRgba::rgb(0.10, 0.06, 0.0) } else { LinearRgba::BLACK }, // fruit glows a touch
            double_sided: leafy, // thin leaf/frond/disc meshes need both faces
            cull_mode: if leafy { None } else { Some(bevy::render::render_resource::Face::Back) },
            ..default()
        });
        commands.entity(e).insert((Mesh3d(forms.forms[fi].clone()), MeshMaterial3d(mat)));
        // bloom child for flowering plant (near top of unit mesh, local space)
        if g.flower > 0.25 && !matches!(g.form, form::KELP | form::MOSS) {
            // local-y of each mesh TOP -> bloom sits ON plant (no float). Flower stalk = CENTERED cylinder
            // (top at y=0.5); clumps/cactus ~1.0; clusters ~0.5; flat forms hug ground.
            let top = match g.form {
                form::FLOWER_STALK => 0.5, // centered cylinder, top at half-height
                form::REED | form::SUCCULENT | form::FERN => 0.85,
                form::HERB => 0.4,
                form::SHRUB => 0.6,
                form::LILYPAD | form::GROUNDCOVER | form::ROSETTE => 0.12,
                _ => 0.5,
            };
            let child = commands
                .spawn((
                    Mesh3d(forms.flower.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: flower_color(g),
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, top, 0.0).with_scale(Vec3::splat(0.28 + 0.45 * g.flower)),
                ))
                .id();
            commands.entity(e).add_child(child);
            // bright YELLOW center disc on petals (classic flower: glowing center + colorful petals).
            // Reuses small berry sphere; emissive -> reads as sunny eye even in shade.
            let bloom = 0.28 + 0.45 * g.flower;
            let center = commands
                .spawn((
                    Mesh3d(forms.berry.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(1.0, 0.86, 0.12),
                        emissive: LinearRgba::rgb(0.55, 0.42, 0.0),
                        ..default()
                    })),
                    Transform::from_xyz(0.0, top + 0.16 * bloom, 0.0).with_scale(Vec3::splat(0.5 * bloom)),
                ))
                .id();
            commands.entity(e).add_child(center);
        }
        // berry children for fruiting land bush; skip aquatic/flat forms. Toxic berries warn deep
        // magenta/violet, edible glow ripe red/orange (slight emissive sheen).
        if g.fruiting > 0.3 && matches!(g.form, form::SHRUB | form::HERB | form::FLOWER_STALK) {
            let (berry, bem) = if g.toxicity > 0.5 {
                (Color::srgb(0.62, 0.05, 0.78), LinearRgba::rgb(0.20, 0.0, 0.28)) // toxic: vivid violet warning
            } else {
                (Color::srgb(0.95, 0.12, 0.18), LinearRgba::rgb(0.30, 0.02, 0.0)) // ripe: bright red
            };
            let bmat = materials.add(StandardMaterial { base_color: berry, emissive: bem, ..default() });
            for k in 0..3 {
                let a = k as f32 * 2.0944; // 120 deg
                let c = commands
                    .spawn((
                        Mesh3d(forms.berry.clone()),
                        MeshMaterial3d(bmat.clone()),
                        Transform::from_xyz(0.22 * a.cos(), 0.45, 0.22 * a.sin()),
                    ))
                    .id();
                commands.entity(e).add_child(c);
            }
        }
        // mushroom cap (domed) on stem top
        if g.form == form::MUSHROOM {
            let c = commands
                .spawn((
                    Mesh3d(forms.cap.clone()),
                    MeshMaterial3d(materials.add(plant_color(g))),
                    Transform::from_xyz(0.0, 0.24, 0.0).with_scale(Vec3::new(0.5, 0.45, 0.5)),
                ))
                .id();
            commands.entity(e).add_child(c);
        }
    }
}

// Hide creature mesh on death (P1.4); restore on rebirth at generation boundary (Alive flips back true).
// Carrion (separate Food entity) appears in its place.
fn hide_dead(mut q: Query<(&Alive, &mut Visibility), With<Creature>>) {
    for (alive, mut vis) in &mut q {
        let want = if alive.0 { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

// Carrion/detritus (Rot) color = rot stage: fresh = meaty red, rotten = dark muddy green. Rot chain (P3)
// reads at a glance: bright red corpse -> darkening -> gone.
fn color_carrion(mut mats: ResMut<Assets<StandardMaterial>>, q: Query<(&Rot, &MeshMaterial3d<StandardMaterial>)>) {
    for (rot, mm) in &q {
        let f = (rot.age as f32 / ROT_GONE as f32).clamp(0.0, 1.0); // 0 fresh .. 1 rotten
        if let Some(m) = mats.get_mut(&mm.0) {
            m.base_color = Color::hsl(10.0 + 90.0 * f, 0.6, 0.5 - 0.35 * f); // red->sick-green, darkening
        }
    }
}

// Scale plants by mass (growth visible) AND root on terrain. height gene STRETCHES plant vertically,
// not lifts into air -> tall plants read tall but base stays grounded (no float). Trees render much
// bigger (tall trunk + canopy).
fn size_plants(mut q: Query<(&PlantState, &PlantGenome, &mut Transform, Option<&Tree>, Option<&Rot>), (With<Food>, Without<Grass>)>) {
    for (st, g, mut tf, tree, rot) in &mut q {
        let up = tf.translation.normalize_or_zero(); // outward surface normal
        let base = crate::sphere::surface_pos(up, 0.0); // foot on terrain surface
        let rot_q = Quat::from_rotation_arc(Vec3::Y, up); // grow outward from planet, not world-up
        // `life` = overall size factor. LIVING plant/tree: grows with mass toward maturity (seedling small,
        // mature full) -> render size tracks mass. DEAD (carrion/fallen fruit/litter, has Rot): SHRINKS as
        // it decomposes -> corpse fades to nothing by ROT_GONE, despawns clean.
        let life = match rot {
            Some(r) => (1.0 - r.age as f32 / ROT_GONE as f32).clamp(0.0, 1.0),
            None => 0.4 + 0.6 * (st.mass / g.maturity.max(0.1)).clamp(0.0, 1.0),
        };
        if tree.is_some() {
            // tree size tracks MASS -> tree on good soil (bigger mass, see plant_step soil response) renders
            // larger than poor-soil one. Mature tree reads as real TREE (~2.4 at maturity ~14, up to ~3.8
            // full) not shrub; cap keeps tallest clear of clouds. height gene tweaks +/- for canopy variety.
            let s = (0.6 + 0.13 * st.mass).clamp(0.6, 3.8) * life * (0.85 + 0.3 * g.height);
            tf.scale = Vec3::splat(s);
            tf.rotation = rot_q;
            tf.translation = base + up * (0.7 * s); // trunk base on surface (trunk half-height = 0.7)
            continue;
        }
        // per-form scale (girth, height) + lift so each silhouette sits on surface. girth grows with mass +
        // bushiness; height from height gene. Custom clump/disc meshes base at y=0 (lift=0); centered
        // primitives lift half their height. droop squashes height.
        let girth = (0.2 + 0.12 * st.mass).clamp(0.2, 1.1);
        let bushy = 0.7 + 0.6 * g.bushiness;
        let tall = (1.0 + 1.4 * g.height) * (1.0 - 0.3 * g.droop);
        // (sx, sy, sz, lift_local) per form. lift = local distance origin -> mesh base. Most forms are
        // base-at-y=0 multi-blob/clump/cactus meshes (lift=0); only centered cylinder stems (flower stalk,
        // mushroom) lift half their height.
        let (sx, sy, sz, lift) = match g.form {
            form::SHRUB => (girth * bushy * 1.2, girth * bushy * 1.1, girth * bushy * 1.2, 0.0),
            form::GROUNDCOVER => (girth * bushy * 1.6, girth * 0.6, girth * bushy * 1.6, 0.0),
            form::MOSS => (girth * bushy * 1.4, girth * 0.45, girth * bushy * 1.4, 0.0),
            form::FERN => (0.55 + 0.45 * bushy, 0.6 + 0.7 * g.height, 0.55 + 0.45 * bushy, 0.0),
            form::SUCCULENT => (0.6 + 0.4 * girth, 0.7 + 0.6 * g.height, 0.6 + 0.4 * girth, 0.0),
            form::REED => (0.3 + 0.25 * bushy, 1.0 + 1.6 * g.height, 0.3 + 0.25 * bushy, 0.0),
            form::FLOWER_STALK => (0.6 + 0.4 * girth, 0.6 + 1.0 * g.height, 0.6 + 0.4 * girth, 0.5),
            form::ROSETTE => (0.6 + 0.5 * bushy, 0.35 + 0.35 * g.height, 0.6 + 0.5 * bushy, 0.0),
            form::LILYPAD => (0.6 + 0.7 * girth, 1.0, 0.6 + 0.7 * girth, 0.0),
            form::KELP => (0.4 + 0.3 * bushy, 1.4 + 2.2 * g.height, 0.4 + 0.3 * bushy, 0.0),
            form::MUSHROOM => (0.5 + 0.4 * girth, 0.5 + 0.6 * g.height, 0.5 + 0.4 * girth, 0.25),
            // HERB + fallback: small bushy clump, stretched by height gene
            _ => (girth * bushy, girth * bushy * tall, girth * bushy, 0.0),
        };
        tf.scale = Vec3::new(sx, sy, sz) * life;
        tf.rotation = rot_q;
        // lily pad floats ON water surface (~PLANET_R), not on seabed below.
        if g.form == form::LILYPAD {
            tf.translation = up * (crate::sphere::PLANET_R + 0.08);
        } else {
            tf.translation = base + up * (lift * sy * life); // mesh base rooted on terrain (no float)
        }
    }
}

// Shared grass tuft mesh + material (inserted by spawn_world_render). One mesh + one material for ALL
// tufts (cheap: grass ubiquitous), unlike per-genome plant spheres.
#[derive(Resource)]
pub struct GrassMesh(pub Handle<Mesh>);
#[derive(Resource)]
pub struct GrassMaterial(pub Handle<StandardMaterial>);
// Shared kelp-frond mesh + brown-green material for ALL seaweed. add_seaweed_visuals scales each by mass/depth.
#[derive(Resource)]
pub struct SeaweedMesh(pub Handle<Mesh>);
#[derive(Resource)]
pub struct SeaweedMaterial(pub Handle<StandardMaterial>);

// Build one grass tuft as clump of BLADES: each blade tall thin pointed strip tapering to tip, arcing
// over (curved, not flat sliver), spread over small footprint + fanned around clump. Unit height (1.0) so
// caller scales real length per soil. Double-sided material renders both faces.
pub fn grass_tuft_mesh() -> Mesh {
    const BLADES: usize = 11;
    let w = 0.022; // blade half-width at base (thin)
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for k in 0..BLADES {
        let t = k as f32;
        let a = t * 2.39996; // golden angle: even blade heading spread around clump
        let (sa, ca) = a.sin_cos();
        let r = 0.04 + 0.16 * ((t * 1.7).sin().abs()); // root offset from clump center (footprint)
        let (ox, oz) = (r * ca, r * sa);
        let h = 0.7 + 0.45 * ((t * 0.9).cos().abs()); // per-blade height variation
        let curve = 0.18 * h; // tip arcs over in blade local +z -> bent blade, not flat spike
        // blade profile local (x width, y up, z bend): base -> mid -> pointed tip
        let prof = [
            [-w, 0.0, 0.0],
            [w, 0.0, 0.0],
            [-w * 0.55, 0.55 * h, curve * 0.45],
            [w * 0.55, 0.55 * h, curve * 0.45],
            [0.0, h, curve], // tip (a point)
        ];
        let base = positions.len() as u32;
        for (vi, p) in prof.iter().enumerate() {
            // rotate blade about Y by heading `a`, then offset to its clump root
            let x = p[0] * ca + p[2] * sa + ox;
            let z = -p[0] * sa + p[2] * ca + oz;
            positions.push([x, p[1], z]);
            normals.push([0.0, 1.0, 0.0]); // up-facing -> blades catch overhead sun (bright green)
            uvs.push([if vi % 2 == 0 { 0.0 } else { 1.0 }, p[1] / h.max(0.001)]);
        }
        // two body quads + pointed tip triangle
        indices.extend_from_slice(&[
            base, base + 1, base + 3, base, base + 3, base + 2, // lower quad
            base + 2, base + 3, base + 4, // tip triangle
        ]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

// Clump of BROAD blades/fronds rooted at y=0, arcing to pointed tip (~unit height). One generator reused
// for fern/reed/kelp/rosette forms (size_plants stretches each differently). Up-facing normals catch
// overhead sun; material double-sided so both faces show. Params: blades=count, hw=base half-width,
// foot=clump footprint radius, curve=tip arc-over, lean=outward lean (rosette flat near ground, reed
// straight up).
pub fn frond_clump_mesh(blades: usize, hw: f32, foot: f32, curve: f32, lean: f32) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for k in 0..blades {
        let t = k as f32;
        let a = t * 2.39996; // golden angle: even heading spread around clump
        let (sa, ca) = a.sin_cos();
        let r = foot * (0.3 + 0.7 * ((t * 1.7).sin().abs())); // root offset from center (footprint)
        let (ox, oz) = (r * ca, r * sa);
        let h = 0.8 + 0.4 * ((t * 0.9).cos().abs()); // per-frond height variation
        // profile: base -> mid -> tip, leaning outward (lean) + arcing over (curve)
        let prof = [
            [-hw, 0.0, 0.0],
            [hw, 0.0, 0.0],
            [-hw * 0.6, 0.55 * h, lean * 0.5 + curve * 0.45],
            [hw * 0.6, 0.55 * h, lean * 0.5 + curve * 0.45],
            [0.0, h, lean + curve],
        ];
        let base = positions.len() as u32;
        for (vi, p) in prof.iter().enumerate() {
            let x = p[0] * ca + p[2] * sa + ox;
            let z = -p[0] * sa + p[2] * ca + oz;
            positions.push([x, p[1], z]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([if vi % 2 == 0 { 0.0 } else { 1.0 }, p[1] / h.max(0.001)]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 3, base, base + 3, base + 2, base + 2, base + 3, base + 4]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

// One aurora curtain: CROSSED PAIR of vertical sheets (X in plan view: one in local XY facing +Z, one in
// local ZY facing +X) -> broad area from ANY horizontal angle, never looks thin. Both sheets carry the
// Earth-aurora vertical color profile baked into vertex colors (additive, falloff lives in RGB): faint
// magenta lower fringe -> bright green body (557.7nm oxygen) -> fading crimson/red top (630nm) ->
// transparent. Soft feathered edges (cosine) -> overlapping curtains blend into continuous rippling sheet
// with ray structure. Brightness/on-off multiplied per frame via base_color. Local: X,Z = width
// (-0.5..0.5), Y = height (0..1, base 0). Transform scales X,Z by width, Y by height.
pub fn aurora_curtain_mesh() -> Mesh {
    let cols = 6usize; // horizontal subdivisions (edge feather)
    let rows = 14usize; // vertical subdivisions (smooth color gradient)
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    // vertical color stops (fy 0=bottom .. 1=top)
    let stop = |fy: f32| -> [f32; 3] {
        let pink = [1.1, 0.25, 0.85]; // lower nitrogen fringe (bright so faint fringe reads)
        let green = [0.30, 1.0, 0.45]; // main oxygen band (557.7nm, slightly yellow-green)
        let red = [1.7, 0.30, 0.40]; // high-altitude oxygen red (boosted so top stays visible past falloff)
        if fy < 0.12 {
            let t = fy / 0.12;
            [pink[0] + (green[0] - pink[0]) * t, pink[1] + (green[1] - pink[1]) * t, pink[2] + (green[2] - pink[2]) * t]
        } else if fy < 0.45 {
            green
        } else {
            let t = (fy - 0.45) / 0.55;
            [green[0] + (red[0] - green[0]) * t, green[1] + (red[1] - green[1]) * t, green[2] + (red[2] - green[2]) * t]
        }
    };
    let stride = (cols + 1) as u32;
    for plane in 0..2 {
        let vbase = positions.len() as u32;
        for r in 0..=rows {
            let fy = r as f32 / rows as f32;
            // vertical brightness falloff: solid through green body, fade to 0 at top, soft bottom.
            let va = (1.0 - fy).powf(1.15) * (fy / 0.04).clamp(0.0, 1.0);
            let c = stop(fy);
            for cc in 0..=cols {
                let fx = cc as f32 / cols as f32 - 0.5; // -0.5..0.5
                let hf = (fx * std::f32::consts::PI).cos().max(0.0); // 1 center -> 0 edges (soft sides)
                let a = va * hf;
                // plane 0: sheet XY (width along X); plane 1: crossed sheet ZY (width along Z)
                let pos = if plane == 0 { [fx, fy, 0.0] } else { [0.0, fy, fx] };
                let nrm = if plane == 0 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
                positions.push(pos);
                normals.push(nrm);
                uvs.push([fx + 0.5, 1.0 - fy]);
                colors.push([c[0] * a, c[1] * a, c[2] * a, a]); // RGB carries the falloff (additive); alpha mirrors it
            }
        }
        for r in 0..rows as u32 {
            for cc in 0..cols as u32 {
                let i = vbase + r * stride + cc;
                indices.extend_from_slice(&[i, i + 1, i + stride + 1, i, i + stride + 1, i + stride]);
            }
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

// Flat round pad, unit radius on XZ plane (y=0), normals up. Lily pad / flat mat. Missing wedge (notch)
// gives lily-pad silhouette.
pub fn disc_mesh(segs: usize) -> Mesh {
    let mut positions: Vec<[f32; 3]> = vec![[0.0, 0.0, 0.0]];
    let mut normals: Vec<[f32; 3]> = vec![[0.0, 1.0, 0.0]];
    let mut uvs: Vec<[f32; 2]> = vec![[0.5, 0.5]];
    let mut indices: Vec<u32> = Vec::new();
    let notch = 0.5_f32; // radians of missing wedge (lily-pad cleft)
    let span = std::f32::consts::TAU - notch;
    for i in 0..=segs {
        let a = notch * 0.5 + span * (i as f32 / segs as f32);
        let (s, c) = a.sin_cos();
        positions.push([c, 0.0, s]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.5 + 0.5 * c, 0.5 + 0.5 * s]);
        if i < segs {
            indices.extend_from_slice(&[0, (i + 1) as u32, (i + 2) as u32]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

// --- richer plant geometry. All baked into SHARED per-form meshes (zero per-plant cost): fuller
// multi-blob bushes/canopies, petalled flowers, domed mushroom caps, armed cactus, stacked-cone conifers.
// Each carries per-vertex grayscale shade (ATTRIBUTE_COLOR) that StandardMaterial multiplies into genome
// color -> soft depth/AO (darker base, brighter crown) not flat blobs. ---

// Buffers for geometry appenders below.
struct MeshBuf {
    pos: Vec<[f32; 3]>,
    nor: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>, // grayscale shade (rgb=v), multiplied into material color
}
// Baked shade -> vertex color. Input brightness 0..1 -> multiplier that only DARKENS (never past ~0.92)
// so shading reads as soft depth/AO into genome color, not washed toward white. Low end keeps crevices
// dark for volume.
fn shade_col(v: f32) -> [f32; 4] {
    let s = (0.45 + 0.45 * v).clamp(0.4, 0.92);
    [s, s, s, 1.0]
}
impl MeshBuf {
    fn new() -> Self {
        MeshBuf { pos: Vec::new(), nor: Vec::new(), col: Vec::new() }
    }
    fn finish(self, indices: Vec<u32>) -> Mesh {
        let n = self.pos.len();
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nor);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        mesh.insert_indices(Indices::U32(indices));
        mesh
    }
}

// Append low-poly UV sphere at `center` (radius r), flat shade `v` on every vertex.
fn push_sphere(b: &mut MeshBuf, idx: &mut Vec<u32>, center: Vec3, r: f32, rings: usize, sectors: usize, v: f32) {
    let start = b.pos.len() as u32;
    for ri in 0..=rings {
        let phi = std::f32::consts::PI * ri as f32 / rings as f32;
        let (sp, cp) = phi.sin_cos();
        for si in 0..=sectors {
            let th = std::f32::consts::TAU * si as f32 / sectors as f32;
            let (st, ct) = th.sin_cos();
            let n = Vec3::new(sp * ct, cp, sp * st);
            let p = center + n * r;
            b.pos.push([p.x, p.y, p.z]);
            b.nor.push([n.x, n.y, n.z]);
            b.col.push(shade_col(v));
        }
    }
    let cols = sectors + 1;
    for ri in 0..rings {
        for si in 0..sectors {
            let a = start + (ri * cols + si) as u32;
            let bb = start + (ri * cols + si + 1) as u32;
            let c = start + ((ri + 1) * cols + si) as u32;
            let d = start + ((ri + 1) * cols + si + 1) as u32;
            idx.extend_from_slice(&[a, c, bb, bb, c, d]);
        }
    }
}

// Append SMOOTH cone (base at `base`, up `height`, base radius r). Ring vertices carry radial slant
// normals (not flat per-face) -> smooth rounded gradient, reads as 3D cone not cardboard triangle. Shade
// fades base->apex (dark->light).
fn push_cone(b: &mut MeshBuf, idx: &mut Vec<u32>, base: Vec3, r: f32, height: f32, seg: usize, v: f32) {
    let apex = base + Vec3::Y * height;
    // cone slant normal at angle th: outward (scaled by height) + up (scaled by r)
    let slant = |th: f32| Vec3::new(th.cos() * height, r, th.sin() * height).normalize_or_zero();
    let ring0 = b.pos.len() as u32;
    for si in 0..=seg {
        let th = std::f32::consts::TAU * si as f32 / seg as f32;
        let p = base + Vec3::new(th.cos() * r, 0.0, th.sin() * r);
        let n = slant(th);
        b.pos.push([p.x, p.y, p.z]);
        b.nor.push([n.x, n.y, n.z]);
        b.col.push(shade_col(v * 0.85));
    }
    let apex0 = b.pos.len() as u32;
    for si in 0..seg {
        let th = std::f32::consts::TAU * (si as f32 + 0.5) / seg as f32; // apex normal aimed at face center
        let n = slant(th);
        b.pos.push([apex.x, apex.y, apex.z]);
        b.nor.push([n.x, n.y, n.z]);
        b.col.push(shade_col(v * 1.05));
    }
    for si in 0..seg as u32 {
        // wind ring->ring->apex so OUTER cone surface is FRONT face. Was reversed: outer face came out
        // back-facing -> double_sided canopy flipped normal (out+up -> in+down) -> cone lit from below
        // (bright undersides, dark sunlit tops). Front-facing outward = lit from above.
        idx.extend_from_slice(&[ring0 + si + 1, ring0 + si, apex0 + si]);
    }
}

// Bushy clump of overlapping blobs. Each blob = (center, radius, shade). Used for herbs, shrubs, ground
// cover, moss bumps, broadleaf tree canopies -> full foliage, not one ball.
pub fn blob_cluster_mesh(blobs: &[(Vec3, f32, f32)]) -> Mesh {
    let mut b = MeshBuf::new();
    let mut idx = Vec::new();
    for &(c, r, v) in blobs {
        push_sphere(&mut b, &mut idx, c, r, 5, 7, v);
    }
    b.finish(idx)
}

// Bird wing: flat swept tapered planform in the X-Z plane (y~0), ROOT at x=0 extending to a pointed tip at
// x=1 (swept back, -Z). Drawn double-sided (material cull None) so the underside lights. Root at origin so a
// flap = rotation about the wing entity's origin (the shoulder). Unit span; viz scales per genome.
pub fn wing_mesh() -> Mesh {
    let mut b = MeshBuf::new();
    // outline, root(0) fanning out to the tip(2); leading edge sweeps back, trailing edge tapers in.
    let pts = [
        Vec3::new(0.0, 0.0, 0.18),   // 0 root leading
        Vec3::new(0.55, 0.0, 0.02),  // 1 mid leading (swept)
        Vec3::new(1.0, 0.0, -0.42),  // 2 pointed tip
        Vec3::new(0.5, 0.0, -0.52),  // 3 mid trailing
        Vec3::new(0.0, 0.0, -0.40),  // 4 root trailing
    ];
    for p in pts {
        b.pos.push([p.x, p.y, p.z]);
        b.nor.push([0.0, 1.0, 0.0]);
        b.col.push([1.0, 1.0, 1.0, 1.0]);
    }
    b.finish(vec![0, 1, 2, 0, 2, 3, 0, 3, 4]) // fan from root
}

// Petalled flower: shallow CUP of petals around raised center button. Petals tilt up-and-out (tips raised)
// with real slant normals -> catch light + read 3D; each petal slightly different height (staggered) so
// coplanar petals don't z-fight. Rendered double-sided (see add_plant_visuals) so undersides not black.
// Base at y=0.
pub fn flower_mesh(petals: usize) -> Mesh {
    let mut b = MeshBuf::new();
    let mut idx = Vec::new();
    push_sphere(&mut b, &mut idx, Vec3::new(0.0, 0.14, 0.0), 0.16, 5, 7, 0.6); // raised center button
    for k in 0..petals {
        let a = std::f32::consts::TAU * k as f32 / petals as f32;
        let (s, c) = a.sin_cos();
        let dir = Vec3::new(c, 0.0, s);
        let side = Vec3::new(-s, 0.0, c) * 0.16;
        let stagger = 0.012 * (k % 3) as f32; // per-petal height offset -> no coplanar z-fight
        let base_l = dir * 0.12 - side + Vec3::Y * (0.02 + stagger);
        let base_r = dir * 0.12 + side + Vec3::Y * (0.02 + stagger);
        let tip = dir * 0.5 + Vec3::Y * (0.2 + stagger); // tip raised -> cupped petal
        let n = (tip - base_l).cross(base_r - base_l).normalize_or_zero(); // slant normal
        let n = [n.x, n.y, n.z];
        let start = b.pos.len() as u32;
        for (p, v) in [(base_l, 0.9), (base_r, 0.9), (tip, 1.0)] {
            b.pos.push([p.x, p.y, p.z]);
            b.nor.push(n);
            b.col.push([v, v, v, 1.0]);
        }
        idx.extend_from_slice(&[start, start + 1, start + 2]);
    }
    b.finish(idx)
}

// Mushroom cap: rounded dome (top half of squashed sphere). Base at y=0.
pub fn dome_mesh() -> Mesh {
    let mut b = MeshBuf::new();
    let mut idx = Vec::new();
    let (rings, sectors) = (4usize, 9usize);
    let start = b.pos.len() as u32;
    for ri in 0..=rings {
        let phi = std::f32::consts::FRAC_PI_2 * ri as f32 / rings as f32; // 0 (top) .. PI/2 (rim)
        let (sp, cp) = phi.sin_cos();
        for si in 0..=sectors {
            let th = std::f32::consts::TAU * si as f32 / sectors as f32;
            let (st, ct) = th.sin_cos();
            let n = Vec3::new(sp * ct, cp, sp * st).normalize_or_zero();
            let p = Vec3::new(sp * ct, cp * 0.7, sp * st); // 0.7 = squashed dome
            b.pos.push([p.x, p.y, p.z]);
            b.nor.push([n.x, n.y, n.z]);
            b.col.push(shade_col(0.75 + 0.25 * cp));
        }
    }
    let cols = sectors + 1;
    for ri in 0..rings {
        for si in 0..sectors {
            let a = start + (ri * cols + si) as u32;
            let bb = start + (ri * cols + si + 1) as u32;
            let c = start + ((ri + 1) * cols + si) as u32;
            let d = start + ((ri + 1) * cols + si + 1) as u32;
            idx.extend_from_slice(&[a, bb, c, bb, d, c]);
        }
    }
    b.finish(idx)
}

// Cactus: tall rounded column + couple stubby up-curved arms (saguaro silhouette). Base at y=0.
pub fn cactus_mesh() -> Mesh {
    let mut b = MeshBuf::new();
    let mut idx = Vec::new();
    // main column: stacked blobs base -> rounded top
    for k in 0..5 {
        let y = 0.12 + 0.2 * k as f32;
        let r = 0.2 - 0.015 * k as f32;
        push_sphere(&mut b, &mut idx, Vec3::new(0.0, y, 0.0), r, 5, 7, 0.7 + 0.05 * k as f32);
    }
    // two arms: horizontal stub turning up
    for &(sx, h) in &[(0.18f32, 0.55f32), (-0.16f32, 0.7f32)] {
        push_sphere(&mut b, &mut idx, Vec3::new(sx, h, 0.0), 0.1, 4, 6, 0.78);
        push_sphere(&mut b, &mut idx, Vec3::new(sx * 1.4, h + 0.12, 0.0), 0.09, 4, 6, 0.82);
        push_sphere(&mut b, &mut idx, Vec3::new(sx * 1.5, h + 0.28, 0.0), 0.085, 4, 6, 0.9);
    }
    b.finish(idx)
}

// Conifer: stacked cones OVERLAPPING heavily (each tier base well inside cone below) -> merge into ONE
// solid Christmas-tree silhouette, not three floating cones. Base at y=0; each apex rises past next cone
// base, closing gaps.
pub fn conifer_mesh() -> Mesh {
    let mut b = MeshBuf::new();
    let mut idx = Vec::new();
    // TIERED fir: smooth cones (no cardboard facets) as drooping branch skirts, stacked so each rim pokes
    // out below the next. Narrow central SPINE cone fills core; canopy material double-sided (see
    // add_plant_visuals) -> no hollow see-through, trunk stays hidden.
    push_cone(&mut b, &mut idx, Vec3::new(0.0, 0.0, 0.0), 0.35, 2.55, 12, 0.85); // central spine (core fill)
    // (base_y, base_radius, height, shade) skirts: wide bottom -> narrow top, moderate overlap = visible
    // tiers. Shade floor raised (was 0.68) so lower tiers keep light, not near-black.
    let skirts = [(0.1_f32, 1.5_f32, 1.0_f32, 0.82_f32), (0.7, 1.2, 1.0, 0.88), (1.3, 0.9, 1.0, 0.94), (1.85, 0.55, 0.95, 1.0)];
    for (y, r, h, shade) in skirts {
        push_cone(&mut b, &mut idx, Vec3::new(0.0, y, 0.0), r, h, 16, shade);
    }
    b.finish(idx)
}

// Climbing vine: helix of small leaf-blobs spiraling up trunk (local space matching centered trunk, y in
// ~[-0.95, 0.85], hugging radius `rad`). Shared mesh attached to vine-bearing trees -> vine appears only
// WITH a tree, costs nothing per-tree (one shared mesh + material).
pub fn vine_mesh(rad: f32) -> Mesh {
    let mut b = MeshBuf::new();
    let mut idx = Vec::new();
    let turns = 3.0;
    let n = 26;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let y = -0.95 + 1.8 * t;
        let a = turns * std::f32::consts::TAU * t;
        let c = Vec3::new(a.cos() * rad, y, a.sin() * rad);
        push_sphere(&mut b, &mut idx, c, 0.05, 3, 5, 0.8); // vine strand bead
        if i % 3 == 0 {
            // leaf blob poking outward every few beads
            let out = Vec3::new(a.cos(), 0.0, a.sin()) * (rad + 0.07);
            push_sphere(&mut b, &mut idx, Vec3::new(out.x, y, out.z), 0.07, 3, 5, 1.0);
        }
    }
    b.finish(idx)
}

// Give any grass tuft lacking a mesh the shared tuft mesh + green material, set transform ONCE (grass
// static, so size at attach not per-frame -> 8000 tufts cost nothing per frame). LENGTH + girth vary with
// SOIL (habitability + moisture): lush tall on rich ground, short on marginal. Rooted on surface, stood
// on sphere normal.
fn add_grass_visuals(
    mut commands: Commands,
    mesh: Option<Res<GrassMesh>>,
    mat: Option<Res<GrassMaterial>>,
    mut q: Query<(Entity, &PlantState, &mut Transform), (With<Grass>, Without<Mesh3d>)>,
) {
    let (Some(mesh), Some(mat)) = (mesh, mat) else { return };
    for (e, st, mut tf) in &mut q {
        let up = tf.translation.normalize_or_zero();
        let base = crate::sphere::surface_pos(up, 0.0);
        // WATER drives height + thickness: wet ground (coasts/edges, tropics) grows tall lush; dry interior
        // short + thin. plant_habitability = secondary viability factor (sparse on marginal land). moisture
        // high near sea edge (coastal) -> tall thick grass by water.
        let wet = crate::sphere::moisture(up).clamp(0.0, 1.0);
        let viable = crate::sphere::plant_habitability(up).clamp(0.0, 1.0);
        let mass_f = 0.7 + 0.3 * st.mass.min(1.0);
        // rocky ground: rockiness thins length + girth -> few sparse blades among stones, not a lawn.
        let thin = 1.0 - 0.65 * crate::sphere::rockiness(up);
        let len = (0.35 + 2.2 * wet) * (0.55 + 0.45 * viable) * mass_f * thin; // water-dominated height
        let girth = (0.9 + 1.9 * wet) * (0.6 + 0.4 * viable) * (0.45 + 0.55 * thin); // wetter = thicker clump
        tf.scale = Vec3::new(girth, len, girth);
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, up);
        tf.translation = base + up * 0.02; // roots on surface
        commands
            .entity(e)
            .insert((Mesh3d(mesh.0.clone()), MeshMaterial3d(mat.0.clone())));
    }
}

// Seaweed = ocean grass: attach shared kelp-frond mesh, size each by mass + DEPTH. Deeper kelp grows
// taller (long stipes reaching light) -> underwater forest; shallow fronds stubby.
fn add_seaweed_visuals(
    mut commands: Commands,
    mesh: Option<Res<SeaweedMesh>>,
    mat: Option<Res<SeaweedMaterial>>,
    mut q: Query<(Entity, &PlantState, &mut Transform), (With<Seaweed>, Without<Mesh3d>)>,
) {
    let (Some(mesh), Some(mat)) = (mesh, mat) else { return };
    for (e, st, mut tf) in &mut q {
        let up = tf.translation.normalize_or_zero();
        let base = crate::sphere::surface_pos(up, 0.0); // holdfast anchored on seabed
        let e01 = crate::sphere::elevation01(up);
        let depth = ((crate::sphere::SEA_LEVEL - e01) / crate::sphere::SEA_LEVEL).clamp(0.0, 1.0);
        let mass_f = 0.6 + 0.4 * st.mass.min(1.2);
        let len = (1.4 + 3.0 * depth) * mass_f; // tall stipes deep, stubby shallow
        let girth = 0.6 + 0.5 * mass_f; // ribbon fronds, not bushes
        tf.scale = Vec3::new(girth, len, girth);
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, up);
        tf.translation = base + up * 0.02;
        commands
            .entity(e)
            .insert((Mesh3d(mesh.0.clone()), MeshMaterial3d(mat.0.clone())));
    }
}

// Orbit sun + moon: directional light comes FROM sun current direction (lit half + terminator sweep as it
// spins); moon sphere rides its orbit. Globe self-shades via surface normals -> illuminance constant;
// ambient (set in setup) lifts night side.
// Base directional sun illuminance (matches main.rs spawn). Eclipse dims this toward twilight.
const SUN_ILLUM: f32 = 64_000.0;

fn day_night_lighting(
    gen: Res<GenState>,
    offset: Res<SunOffset>,
    mut suns: Query<(&mut Transform, &mut DirectionalLight), (With<SunLight>, Without<Moon>, Without<SunDisc>)>,
    mut moons: Query<(&mut Transform, &MeshMaterial3d<StandardMaterial>), (With<Moon>, Without<SunLight>, Without<SunDisc>)>,
    mut discs: Query<&mut Transform, (With<SunDisc>, Without<SunLight>, Without<Moon>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // visual sky time = sim tick + offset (offset lets walk pick sunny hour without moving sim). FULL tick
    // (not mod DAY_TICKS): the Tychos sun carries a yearly ecliptic term, so reducing to one day would erase
    // seasons from the lit sky.
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let sd = crate::sphere::sun_dir(vtick);
    let md = crate::sphere::moon_dir(vtick); // unit geocentric moon dir

    // Eclipses (sun disc + moon disc share angular radius `ar` here by design, see sphere::SUN_R):
    // SOLAR = moon disc over sun disc (new moon, sep small) -> dim the sun (moon's shadow on the world).
    // LUNAR = moon in the planet's umbra (full moon, moon opposite sun within planet's angular radius) -> blood moon.
    let ar = crate::sphere::MOON_R / crate::sphere::MOON_ORBIT; // ~0.045 rad (~2.6 deg), stylized large
    let sep = sd.dot(md).clamp(-1.0, 1.0).acos();
    let solar = ((2.0 * ar - sep) / (2.0 * ar)).clamp(0.0, 1.0); // 0 none .. 1 total
    let par = crate::sphere::PLANET_R / crate::sphere::MOON_ORBIT; // planet angular radius seen from moon
    let lang = (-sd).dot(md).clamp(-1.0, 1.0).acos();
    let lunar = ((par - lang) / par).clamp(0.0, 1.0);

    for (mut tf, mut light) in &mut suns {
        // ROTATE directional light in place (only direction matters). Light carries NoFrustumCulling -> stays
        // ViewVisible -> Bevy keeps building shadow cascades. GOTCHA: teleporting it far / to planet core
        // frustum-culled it to invisible, silently disabling shadows.
        *tf = Transform::IDENTITY.looking_to(-sd, Vec3::Y);
        light.illuminance = SUN_ILLUM * (1.0 - 0.96 * solar); // total eclipse -> ~4% (deep twilight)
    }
    for mut tf in &mut discs {
        tf.translation = sd * crate::sphere::SUN_DIST; // visible sun rides same direction, far out
    }
    let mtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let mp = crate::sphere::moon_pos(mtick);
    for (mut tf, mat) in &mut moons {
        tf.translation = mp;
        if let Some(m) = materials.get_mut(&mat.0) {
            // blood moon: lerp the moon toward dark red as it enters the umbra
            let e = Vec3::new(0.5, 0.5, 0.55).lerp(Vec3::new(0.35, 0.05, 0.03), lunar);
            m.emissive = LinearRgba::rgb(e.x, e.y, e.z);
            let bc = Vec3::new(0.85, 0.85, 0.9).lerp(Vec3::new(0.45, 0.12, 0.09), lunar);
            m.base_color = Color::srgb(bc.x, bc.y, bc.z);
        }
    }
}

// Wheel the real starfield with the planet's daily spin. Equatorial coords -> rotate about +Y (celestial
// pole). FULL tick keeps it in step with sun/moon/planets (which carry the obliquity via ecliptic_to_sky).
fn rotate_sky_stars(gen: Res<GenState>, offset: Res<SunOffset>, mut q: Query<&mut Transform, With<SkyStars>>) {
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let daily = (vtick as f32 / crate::sphere::DAY_TICKS as f32) * std::f32::consts::TAU;
    for mut tf in &mut q {
        tf.rotation = Quat::from_rotation_y(daily);
    }
}

// Place each wandering planet on the sky shell at its geocentric ecliptic dir (orrery), through the same
// ecliptic_to_sky transform as sun/moon, so it drifts correctly against the fixed stars along the zodiac.
fn position_sky_planets(gen: Res<GenState>, offset: Res<SunOffset>, mut q: Query<(&SkyPlanet, &mut Transform)>) {
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let tau = crate::sphere::t_years(vtick);
    let r = crate::sphere::PLANET_R * 85.0 * 0.97; // just inside the star shell
    for (p, mut tf) in &mut q {
        let ecl = crate::orrery::geocentric_dir(p.idx, tau);
        tf.translation = crate::sphere::ecliptic_to_sky(ecl, vtick) * r;
    }
}

// Scrub time-of-day in walk mode: [ winds sun back, ] forward (golden-hour shadows), \ snaps to local
// noon overhead walker. Adjusts visual SunOffset only (sim time untouched).
fn time_of_day(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<crate::camera::CameraMode>,
    gen: Res<GenState>,
    mut offset: ResMut<SunOffset>,
    walkers: Query<&crate::camera::WalkCam>,
) {
    if *mode != crate::camera::CameraMode::Walk {
        return;
    }
    let step = crate::sphere::DAY_TICKS as i64 / 48; // ~1 sky-hour per tap
    if keys.just_pressed(KeyCode::BracketRight) {
        offset.0 += step;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        offset.0 -= step;
    }
    if keys.just_pressed(KeyCode::Backslash) {
        if let Ok(w) = walkers.single() {
            offset.0 = noon_offset(w.dir, gen.tick);
        }
    }
}

// Walk mode: camera ambient fill tracks local daylight so NIGHT GOES DARK (flat high ambient made
// everything glow from the ground). Night keeps low moonlit floor so silhouettes read; noon bright sky
// fill. Orbit ambient left to update_shadow_mode (steady 220 for crisp terminator). Uses same visual sky
// time (sim tick + SunOffset) as sun.
fn walk_ambient(
    mode: Res<crate::camera::CameraMode>,
    gen: Res<GenState>,
    offset: Res<SunOffset>,
    walkers: Query<&crate::camera::WalkCam>,
    mut ambient: Query<&mut AmbientLight>,
) {
    if *mode != crate::camera::CameraMode::Walk {
        return;
    }
    let Ok(w) = walkers.single() else { return };
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32; // full tick: keep seasons (see day_night_lighting)
    let day = crate::sphere::daylight_at(w.dir.normalize_or_zero(), vtick); // 0 night .. 1 noon overhead
    // low fill so strong directional sun (100k lux) keeps shadows + 3D shading; lit surfaces stay bright.
    // High fill washed shadows flat.
    let b = 45.0 + 230.0 * day; // moonlit ~45 night, soft day fill ~275 (shadows survive)
    for mut a in &mut ambient {
        a.brightness = b;
    }
}

// Toggle real shadows (O), both walk + orbit. On by default; off = flat always-sunlit look.
fn toggle_shadows(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowShadows>) {
    if keys.just_pressed(KeyCode::KeyO) {
        show.0 = !show.0; // both walk + orbit (camera::update_shadow_mode reads it)
        info!("shadows: {}", if show.0 { "ON" } else { "OFF" });
    }
}

// Tick offset putting sun overhead surface dir `d` (local noon). Derived from the actual sun dir (Tychos
// model): daily spin advances the sun's horizontal bearing by -TAU/DAY_TICKS per tick, so rotate the
// current sun bearing onto d's bearing. Model-agnostic (reads sun_dir, not a hardcoded longitude formula).
pub fn noon_offset(d: Vec3, tick: u32) -> i64 {
    use std::f32::consts::TAU;
    let s = crate::sphere::sun_dir(tick);
    let sun_bear = s.z.atan2(s.x);
    let d_bear = d.z.atan2(d.x);
    let off = ((sun_bear - d_bear) / TAU * crate::sphere::DAY_TICKS as f32).round() as i64;
    off.rem_euclid(crate::sphere::DAY_TICKS as i64)
}

// Precip streaks (immediate-mode gizmos, no entities). Where raining (cloud-driven, sampled on lat/lon
// grid), draw scatter that FALLS: height cycles down tick clock + wraps -> animates, not static. WARM
// cells -> blue rain streaks (gradient: faded motion-tail top, bright drop head bottom; heavier = more,
// longer, brighter). COLD cells (below snow line) -> white snow: slower fall, lazy sway, soft dots, no
// tail. Deterministic (tick clock + hashed jitter, no per-frame RNG).
fn rain_visuals(gen: Res<GenState>, mut gizmos: Gizmos) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let (rows, cols) = (44, 88);
    // Fall span + speed CONSTANT (not from rain intensity). rain_at animates every tick (clouds drift), so
    // intensity-derived span made modulus + jitter jump each tick -> drops bounced instead of falling.
    // FALL_SPEED*FALL_SPAN cycle = whole ticks, tick wrapped to exact multiple of cycle -> seamless wrap,
    // no f32 precision drift on long runs.
    const FALL_SPAN: f32 = 9.0; // drop travel distance (surface .. top of streak), world units
    const FALL_SPEED: f32 = 0.25; // rain: units per tick
    const SNOW_SPEED: f32 = 0.10; // snow drifts ~2.5x slower than rain
    // Snow line: precip renders as snow where GROUND temp < this. ABOVE permanent ice-cap onset (temp<0.34)
    // because air aloft much colder than surface -> snow falls well outside year-round ice (snow line
    // ~50deg lat). Tuned vs rain field: 0.34 never snowed (rain never that cold); 0.65 -> snow ~60% of ticks
    // (see sphere test snow_cells_exist).
    const SNOW_TEMP: f32 = 0.65;
    // 180000 = 5000*(FALL_SPAN/FALL_SPEED) = 2000*(FALL_SPAN/SNOW_SPEED): exact multiple of BOTH fall
    // cycles -> rain AND snow phase identically across wrap (seamless), no f32 drift on long runs.
    let t = (gen.tick % 180_000) as f32;
    for j in 0..rows {
        for i in 0..cols {
            let lat = -FRAC_PI_2 + PI * (j as f32 + 0.5) / rows as f32;
            let lon = -PI + TAU * (i as f32 + 0.5) / cols as f32;
            let d = crate::sphere::lonlat_to_pos(lon, lat, 0.0).normalize();
            let r = crate::sphere::rain_at(d, gen.tick);
            if r <= 0.0 {
                continue;
            }
            let (east, north) = crate::sphere::tangent_frame(d);
            let base = crate::sphere::surface_pos(d, 0.0);
            // cold cells get snow: white drifting flakes (slow fall + side-sway, soft dots, no tail); warmer
            // keep bright blue rain. Threshold tracks visible ice cap.
            if crate::sphere::base_temperature(d) < SNOW_TEMP {
                let flakes = 2 + (r * 5.0) as usize; // heavier snow = denser scatter (2..7)
                for k in 0..flakes {
                    let seed = ((j * cols + i) * 8 + k) as u32;
                    let ph = hash01(seed ^ 0xC3) * TAU; // per-flake sway phase
                    // slow descent on shared seamless clock + lazy horizontal drift
                    let fall =
                        FALL_SPAN - ((t * SNOW_SPEED + hash01(seed ^ 0xAA) * FALL_SPAN) % FALL_SPAN);
                    let sway = east * (t * 0.04 + ph).sin() * 1.2 + north * (t * 0.03 + ph).cos() * 0.8;
                    let foot = base
                        + east * (hash01(seed) - 0.5) * 4.0
                        + north * (hash01(seed ^ 0x55) - 0.5) * 4.0
                        + sway;
                    let p = foot + d * (fall + 0.5);
                    let flake = Color::srgba(0.93, 0.95, 1.0, (0.40 + 0.40 * r).min(0.85));
                    // tiny soft dot: short segment along up (no tail -> reads as flake, not streak)
                    gizmos.line(p, p + d * 0.25, flake);
                }
                continue;
            }
            let drops = 1 + (r * 4.0) as usize; // heavier downpour = more drops (1..5)
            for k in 0..drops {
                let seed = ((j * cols + i) * 8 + k) as u32;
                // spread drop across cell footprint in tangent plane
                let foot =
                    base + east * (hash01(seed) - 0.5) * 4.0 + north * (hash01(seed ^ 0x55) - 0.5) * 4.0;
                // fall: streak bottom slides FALL_SPAN..0 then wraps back to top
                let fall =
                    FALL_SPAN - ((t * FALL_SPEED + hash01(seed ^ 0xAA) * FALL_SPAN) % FALL_SPAN);
                let len = 1.2 + 1.5 * r; // heavier rain = longer streaks
                let head = Color::srgba(0.70, 0.80, 1.0, (0.35 + 0.5 * r).min(0.85)); // bright drop head
                let tail = Color::srgba(0.70, 0.80, 1.0, 0.0); // fades into motion tail
                gizmos.line_gradient(foot + d * (fall + len + 0.5), foot + d * (fall + 0.5), tail, head);
            }
        }
    }
}

// Drifting clouds as solid translucent puffs (not wireframe). Fixed grid of flattened white spheres rides
// shell well above tallest trees; each frame opacity + size track cloud field which scrolls with wind ->
// clouds form, drift, dissolve. cloud_alt() clears terrain + trees.
#[derive(Component)]
struct CloudPuff {
    anchor: Vec3,   // fixed field-space home; each frame rides this latitude wind to drifted pos
    moist: f32,     // ground moisture below anchor; wetter -> bigger taller cloud
    grow: f32,      // 0..1 smoothed fullness; ramps toward target -> build + dissolve (no pop-in)
    scale_var: f32, // per-puff size mult (~0.6..1.6) -> cloud lumpy, not grid of clones
    flat: f32,      // per-puff vertical squash (thinner = wispier); flattens along local up
    hbias: f32,     // per-puff altitude offset -> puffs layer instead of one shell
}

fn cloud_alt() -> f32 {
    crate::sphere::PLANET_R + crate::sphere::ELEV_MAX + 10.0
}

// Cheap deterministic 0..1 from integer seed (jitter + per-puff variation; spawn reproducible).
fn hash01(n: u32) -> f32 {
    let x = n.wrapping_mul(2654435761) ^ (n >> 15);
    (x.wrapping_mul(40503) & 0xffff) as f32 / 65535.0
}

fn spawn_clouds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let mesh = meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap());
    let alt = cloud_alt();
    // Dense grid + per-puff jitter -> cloudy regions read as clusters of varied puffs, not a lattice.
    let (rows, cols) = (22, 44);
    let (dlat, dlon) = (PI * 0.92 / rows as f32, TAU / cols as f32);
    for j in 0..rows {
        for i in 0..cols {
            let seed = (j as u32) * cols + i as u32;
            // jitter each puff off exact grid (up to ~0.6 cell) -> organic pattern
            let lat = -FRAC_PI_2 * 0.92 + (PI * 0.92) * (j as f32 + 0.5) / rows as f32
                + (hash01(seed) - 0.5) * dlat * 1.2;
            let lon = -PI + TAU * (i as f32 + 0.5) / cols as f32 + (hash01(seed ^ 0x9e37) - 0.5) * dlon * 1.2;
            let anchor = crate::sphere::lonlat_to_pos(lon, lat, 0.0).normalize();
            let mat = materials.add(StandardMaterial {
                base_color: Color::srgba(0.95, 0.96, 1.0, 0.0),
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(anchor * alt),
                Visibility::Hidden,
                // Clouds don't cast (for now): GOTCHA: translucent Blend mesh casts FULL OPAQUE shadow in
                // Bevy (alpha ignored in shadow pass) -> big overlapping puffs = hard black blobs. True
                // ~50% soft cloud shadow needs alpha-HASHED (dithered) shadows = small custom shadow shader;
                // until then clouds stay non-casting so they read soft + transparent.
                bevy::light::NotShadowCaster,
                CloudPuff {
                    anchor,
                    moist: crate::sphere::moisture(anchor),
                    grow: 0.0,
                    scale_var: 0.6 + hash01(seed ^ 0x1234) * 1.0,
                    flat: 0.35 + hash01(seed ^ 0x5678) * 0.18,
                    hbias: (hash01(seed ^ 0xabcd) - 0.5) * 6.0,
                },
            ));
        }
    }
}

fn update_clouds(
    time: Res<Time>,
    gen: Res<GenState>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&mut CloudPuff, &MeshMaterial3d<StandardMaterial>, &mut Visibility, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let alt = cloud_alt();
    for (mut puff, mm, mut vis, mut tf) in &mut q {
        // Glide: cloud PATTERN drifts at -wind per tick (features move opposite sample rotation), so riding
        // anchor by -a keeps each puff on its own cloud as it sweeps across sky.
        let a = -(gen.tick as f32) * crate::sphere::zonal_wind(puff.anchor);
        let (s, c) = (a.sin(), a.cos());
        let an = puff.anchor;
        let dir = Vec3::new(c * an.x - s * an.z, an.y, s * an.x + c * an.z);
        // Cover sampled at drifted pos -> rain (same field) falls under visible cloud.
        let cov = crate::sphere::cloud_cover(dir, gen.tick);
        // Target fullness: cloud cover x ground moisture below (wet -> taller). Below cover threshold 0.18
        // target = 0 -> puff shrinks + fades, not vanishes.
        let target = if cov < 0.18 { 0.0 } else { cov * (0.55 + 0.45 * puff.moist) };
        // Ramp grow toward target over seconds -> smooth build + dissolve (no pop-in). Build slow,
        // dissipate faster, like real cumulus.
        let rate = if target > puff.grow { 0.6 } else { 1.1 };
        puff.grow += (target - puff.grow) * (dt * rate).min(1.0);
        if puff.grow < 0.02 {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        // Move + orient: sit at drifted pos, lie flat against sky shell (squash along local up).
        tf.translation = dir * (alt + puff.hbias);
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
        if let Some(m) = mats.get_mut(&mm.0) {
            // Opacity tracks grow -> forming cloud fades in from clear; thickest cap ~0.5. Thicker cover
            // greys puff (rain clouds darker underneath); thin cover stays bright white.
            let shade = 1.0 - 0.28 * cov;
            m.base_color = Color::srgba(0.96 * shade, 0.97 * shade, 1.0 * shade, (0.45 * puff.grow).min(0.5));
        }
        // Start small wisp, grow to full puff; per-puff size + squash -> clouds lumpy not uniform.
        let s = (2.0 + 16.0 * puff.grow) * puff.scale_var;
        tf.scale = Vec3::new(s, s * puff.flat, s); // squash along local up (set by tf.rotation) = flat cloud
    }
}

// Wildfire (immediate-mode gizmos). Each burning cell = teardrop cluster of flickering flame tongues (hot
// yellow-white base -> dim red swaying tips) + a few rising cooling embers. Tongue count, body radius +
// height scale with burn intensity. Tight to land cell so coarse-grid coastal cells don't spill onto sea.
// Deterministic (tick clock + hashed jitter).
fn fire_visuals(fire: Res<Fire>, gen: Res<GenState>, mut gizmos: Gizmos) {
    let t = gen.tick as f32;
    for c in 0..fire.cell.len() {
        let f = fire.cell[c];
        if f < 0.1 {
            continue;
        }
        let surf = grid_cell_surface(c);
        let up = surf.normalize_or_zero();
        // safety: never draw flame over water (sim won't ignite ocean cells; guards coarse-grid coastal
        // cells whose center reads as sea -> no flame on waves).
        if crate::sphere::is_ocean(up) {
            continue;
        }
        let (east, north) = crate::sphere::tangent_frame(up);
        use std::f32::consts::TAU;
        // flame body: many short tongues packed in small disk, fanning into teardrop converging to point.
        // Hot bright at base, cooling to dim red at flickering tips.
        let tongues = 10 + (f * 16.0) as usize; // hotter = fuller flame body (10..26)
        for k in 0..tongues {
            let seed = (c * 32 + k) as u32;
            // base point in disk, denser toward center (sqrt) -> solid core
            let ang = hash01(seed) * TAU;
            let rad = hash01(seed ^ 0x9e3);
            let rr = rad.sqrt() * (0.4 + 0.6 * f); // body radius grows with intensity (~ up to 1.0)
            let (bx, bz) = (ang.cos() * rr, ang.sin() * rr);
            let foot = surf + up * 0.05 + east * bx + north * bz;
            // height: center-tall teardrop (edge tongues short), pulsing on tick clock
            let flick = 0.5 + 0.5 * (t * 0.30 + seed as f32 * 1.7).sin();
            let h = (0.6 + 1.8 * f) * (1.0 - 0.6 * rad) * (0.55 + 0.7 * flick); // ~ up to 2.4
            let sway = (t * 0.22 + seed as f32 * 2.3).sin() * 0.3 * f; // tips wander as it flickers
            // tips pull back toward center as they rise -> flame comes to a point
            let tip = surf + up * (0.05 + h) + east * (bx * 0.2 + sway) + north * (bz * 0.2 + sway * 0.5);
            let hot = Color::srgb(1.0, 0.92, 0.55); // near yellow-white, hottest at base
            let cool = Color::srgba(0.85, 0.12, 0.02, 0.55); // dim red, fading at cooling tip
            gizmos.line_gradient(foot, tip, hot, cool);
        }
        // embers: small bright motes rise, drift wider, fade as they cool
        let embers = (f * 4.0) as usize;
        for k in 0..embers {
            let seed = (c * 16 + 100 + k) as u32;
            let span = 4.0 + 3.0 * f;
            let rise = (t * 0.2 + hash01(seed) * span) % span;
            let frac = rise / span;
            let p = surf
                + up * (1.0 + rise)
                + east * (hash01(seed ^ 0x3) - 0.5) * 2.0 * frac
                + north * (hash01(seed ^ 0x7) - 0.5) * 2.0 * frac;
            let lit = Color::srgba(1.0, 0.6, 0.2, (1.0 - frac) * 0.9); // bright, dims as cools
            let gone = Color::srgba(1.0, 0.3, 0.05, 0.0);
            gizmos.line_gradient(p, p + up * 0.4, lit, gone);
        }
    }
}

#[derive(Resource, Default)]
pub struct ShowSensors(pub bool);

fn log_viz_help() {
    info!("viz: TAB=orbit/walk (keeps true sim time; [ ] scrub time, \\ noon; swim into the sea: look + W to dive) | hue=diet, vividness=rigidity, size=sensors | color=skin genes (venom=warning tint), head/eyes/legs=genome, size=body | G=sensor rays | SPACE=pause | 1-5=speed +/-=fine | B=seed life P=populate planet L=lightning K=cull | M=minimap field Y=phylogeny | O=save world | H=legend");
}


// Recolor + rescale creature on genome change (spawn + every generation boundary).
fn restyle_creatures(
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&Genome, &MeshMaterial3d<StandardMaterial>, &mut Transform), Changed<Genome>>,
) {
    for (g, mm, mut tf) in &mut q {
        let (color, scale) = creature_look(g); // skin_hue/sat, venom warning, fur/armor, fish body plan
        if let Some(m) = mats.get_mut(&mm.0) {
            m.base_color = color;
        }
        tf.scale = scale;
    }
}

fn toggle_sensors(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowSensors>) {
    if keys.just_pressed(KeyCode::KeyG) {
        show.0 = !show.0;
    }
}

// God-controls (M6): live disturbances to steer ecosystem + watch response. L = lightning (ignite
// wildfire in driest land cell -> spreads via fire_step). K = mass-mortality (kill ~1/3 creatures).
// Pokes sim resources/state transiently; no balance constants changed. Uses no sim RNG (deterministic-safe).
fn god_disturbances(
    keys: Res<ButtonInput<KeyCode>>,
    gen: Res<GenState>,
    mut fire: ResMut<Fire>,
    gw: Res<GroundWater>,
    mut creatures: Query<(&Genome, &mut Alive), With<Creature>>,
    mut commands: Commands,
    mut rng: ResMut<crate::rng::Rng>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        // seed burst of creatures cloned from living pop (competent brains)
        const BURST: usize = 200;
        let parents: Vec<Genome> = creatures.iter().filter(|(_, a)| a.0).map(|(g, _)| g.clone()).collect();
        crate::sim::seed_burst(&mut commands, &mut rng, &parents, BURST);
        info!("god: seeded {BURST} new creatures (clones of the living)");
    }
    if keys.just_pressed(KeyCode::KeyP) {
        // populate WHOLE planet: plants + trees + creatures, each in survivable habitat (aquatic in sea,
        // alpine in mountains, climate-matched). Fills every region instead of waiting for spread.
        let parents: Vec<Genome> = creatures.iter().filter(|(_, a)| a.0).map(|(g, _)| g.clone()).collect();
        crate::sim::seed_planet(&mut commands, &mut rng, &parents, gen.ntypes(), 300, 600, 120);
        info!("god: seeded the whole planet (300 creatures, 600 plants, 120 trees)");
    }
    if keys.just_pressed(KeyCode::KeyL) {
        // ignite driest non-ocean grid cell (most flammable fuel)
        let mut best = 0usize;
        let mut driest = f32::INFINITY;
        for c in 0..fire.cell.len() {
            let surf = grid_cell_surface(c);
            if crate::sphere::is_ocean(surf.normalize_or_zero()) {
                continue;
            }
            let w = gw.get(surf);
            if w < driest {
                driest = w;
                best = c;
            }
        }
        fire.cell[best] = 1.0;
        info!("god: lightning strike -> wildfire ignited");
    }
    if keys.just_pressed(KeyCode::KeyK) {
        let mut i = 0u32;
        let mut killed = 0u32;
        for (_, mut alive) in &mut creatures {
            if alive.0 {
                i += 1;
                if i.is_multiple_of(3) {
                    alive.0 = false; // sim turns into carrion + despawns next step
                    killed += 1;
                }
            }
        }
        info!("god: mass-mortality event -> culled {killed} creatures");
    }
}

// Start visualizer at calm pace so day/night + creature motion read clearly. Sim unchanged (same ticks);
// only how fast virtual clock feeds FixedUpdate. Speed up with +/- keys.
const VIEW_SPEED_DEFAULT: f32 = 0.35;
fn set_initial_speed(mut vtime: ResMut<Time<Virtual>>) {
    vtime.set_relative_speed(VIEW_SPEED_DEFAULT);
}

// Time god-controls: SPACE pause/resume, +/- halve/double, 1-5 jump to preset speed. Drives Bevy virtual
// clock FixedUpdate advances from -> pause/speed scales whole sim, no sim change. Range 0.1x (study one
// creature) .. 16x (fast-forward evolution).
fn time_controls(keys: Res<ButtonInput<KeyCode>>, mut vtime: ResMut<Time<Virtual>>) {
    if keys.just_pressed(KeyCode::Space) {
        if vtime.is_paused() {
            vtime.unpause();
            info!("sim resumed");
        } else {
            vtime.pause();
            info!("sim PAUSED");
        }
    }
    // preset speeds on number row (1=slowest .. 5=fast)
    let preset = if keys.just_pressed(KeyCode::Digit1) {
        Some(0.1)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(0.35)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(1.0)
    } else if keys.just_pressed(KeyCode::Digit4) {
        Some(4.0)
    } else if keys.just_pressed(KeyCode::Digit5) {
        Some(16.0)
    } else {
        None
    };
    if let Some(s) = preset {
        if vtime.is_paused() {
            vtime.unpause();
        }
        vtime.set_relative_speed(s);
        info!("sim speed {s:.2}x");
    }
    let cur = vtime.relative_speed();
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        let s = (cur * 2.0).min(16.0);
        vtime.set_relative_speed(s);
        info!("sim speed {s:.2}x");
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        let s = (cur * 0.5).max(0.1);
        vtime.set_relative_speed(s);
        info!("sim speed {s:.2}x");
    }
}

// --- click-to-inspect (left-click selects creature/plant; on-screen panel shows stats) ---

#[derive(Resource, Default)]
pub struct Selected {
    pub entity: Option<Entity>,
    pub follow: bool,        // camera tracks selected entity (toggle F)
    pub follow_offset: Vec3, // camera offset from target, captured when follow engaged
}

#[derive(Component)]
struct StatsText;

// Live world dashboard (bottom-left): population, day, avg evolved genes + niche counts. Render-only.
#[derive(Component)]
struct WorldStatsText;

// Sky color tracks sun in walk mode (black sky made midday look like night). Dark night -> warm
// dawn/dusk -> blue midday, by local daylight at walker. Orbit keeps near-black space.
fn update_sky(
    gen: Res<GenState>,
    offset: Res<SunOffset>,
    mode: Res<crate::camera::CameraMode>,
    underwater: Res<Underwater>,
    walkers: Query<&crate::camera::WalkCam>,
    mut clear: ResMut<ClearColor>,
) {
    let space = Vec3::new(0.015, 0.02, 0.05);
    let c = if *mode != crate::camera::CameraMode::Walk {
        space
    } else {
        let dir = walkers.single().map(|w| w.dir.normalize_or_zero()).unwrap_or(Vec3::Y);
        let vtick = (gen.tick as i64 + offset.0).max(0) as u32; // full tick: keep seasons
        let d = crate::sphere::daylight_at(dir, vtick);
        if underwater.0 {
            // submerged: murky blue-green horizon, darker than open sky + dims with daylight
            Vec3::new(0.02, 0.12, 0.20) * (0.35 + 0.65 * d)
        } else {
            let night = Vec3::new(0.02, 0.03, 0.07);
            let warm = Vec3::new(0.75, 0.45, 0.32); // dawn/dusk horizon glow
            let blue = Vec3::new(0.50, 0.70, 1.0); // clear bright midday
            if d < 0.25 {
                night.lerp(warm, (d / 0.25).clamp(0.0, 1.0))
            } else {
                warm.lerp(blue, ((d - 0.25) / 0.75).clamp(0.0, 1.0))
            }
        }
    };
    clear.0 = Color::srgb(c.x, c.y, c.z);
}

// Flag whether walk eye below sea surface (walk mode + over ocean only). Others read Underwater to tint
// frame blue + murk sky. Cleared in orbit.
fn track_underwater(
    mode: Res<crate::camera::CameraMode>,
    walkers: Query<&crate::camera::WalkCam>,
    mut underwater: ResMut<Underwater>,
) {
    let sub = *mode == crate::camera::CameraMode::Walk
        && walkers.single().is_ok_and(|w| {
            let depth = (-crate::sphere::elevation(w.dir)).max(0.0); // local water depth (sea surface above seafloor)
            crate::sphere::is_ocean(w.dir) && w.eye_alt < depth
        });
    if underwater.0 != sub {
        underwater.0 = sub;
    }
}

// Spawn full-screen blue tint overlay (hidden until underwater). GlobalZIndex(-1) keeps it above 3D scene
// but below HUD text -> world tints blue, dashboard stays readable.
fn spawn_underwater_tint(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.06, 0.30, 0.52, 0.34)),
        GlobalZIndex(-1),
        Visibility::Hidden,
        UnderwaterTint,
    ));
}

// Show blue overlay only while submerged.
fn toggle_underwater_tint(
    underwater: Res<Underwater>,
    mut q: Query<&mut Visibility, With<UnderwaterTint>>,
) {
    let want = if underwater.0 { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

// Breathe slow swell on ocean shell (subtle radial scale wobble = living tide). Cosmetic.
fn animate_ocean(gen: Res<GenState>, mut q: Query<&mut Transform, With<Ocean>>) {
    let s = 1.0 + 0.004 * (gen.tick as f32 * 0.03).sin();
    for mut tf in &mut q {
        tf.scale = Vec3::splat(s);
    }
}

// Aurora curtains: each segment flickers organically (layered sines + global substorm), glides sideways
// around oval, sways, pulses height -> restless dancing band. Color green, tips surge toward
// magenta/violet during active bursts. Brighter on night side (daylight washes it out).
fn update_aurora_curtains(
    gen: Res<GenState>,
    offset: Res<SunOffset>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&AuroraCurtain, &MeshMaterial3d<StandardMaterial>, &mut Transform)>,
) {
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let t = gen.tick as f32;
    let base_r = crate::sphere::PLANET_R + AURORA_LIFT;
    let substorm = ((t * 0.0009).sin() * 0.5 + 0.5).powf(3.0); // shared planet-wide activity surge
    const CURTAIN_H: f32 = 16.0; // curtain height (mesh unit-tall; this scales it)
    const FOLD_AMP: f32 = 0.10; // mag-latitude wave amplitude (radians) -> serpentine draperies
    for (c, mm, mut tf) in &mut q {
        let night = 1.0 - crate::sphere::daylight_at(c.pole, vtick);
        let f = c.freq;
        // organic flicker: incommensurate sines + substorm burst
        let flick = (0.45
            + 0.30 * (t * f + c.phase).sin()
            + 0.15 * (t * f * 2.3 + c.phase * 1.7).sin()
            + 0.25 * substorm)
            .clamp(0.0, 1.5);
        // drift around oval + folded draperies: band waves north/south (two frequencies) like real curtains
        let ang = c.ang + c.drift * t + 0.04 * (t * f * 0.7 + c.phase).sin();
        let lat = AURORA_LAT
            + FOLD_AMP * (ang * 5.0 + t * 0.004 + c.phase).sin()
            + 0.5 * FOLD_AMP * (ang * 11.0 - t * 0.006).sin();
        // basis perpendicular to the magnetic pole
        let pole = c.pole;
        let a = if pole.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let u = pole.cross(a).normalize();
        let v = pole.cross(u);
        let circ = u * ang.cos() + v * ang.sin();
        let dirp = (pole * lat.sin() + circ * lat.cos()).normalize();
        let n = dirp; // radial up (curtain rises along this = local Y)
        let tang = pole.cross(dirp).normalize(); // around-oval tangent (curtain width = local X)
        let bin = tang.cross(n); // local Z, RIGHT-HANDED (X x Y = Z) so quaternion is real rotation, not mirror
        let height = 0.55 + 0.9 * flick; // curtains grow tall when active
        let sway = Quat::from_axis_angle(tang, 0.10 * (t * f * 1.3 + c.phase).sin());
        tf.translation = dirp * base_r; // base on band; mesh (local Y) rises outward
        tf.rotation = sway * Quat::from_mat3(&Mat3::from_cols(tang, n, bin));
        let w = c.width * (0.85 + 0.3 * flick);
        tf.scale = Vec3::new(w, CURTAIN_H * height, w); // X and Z both = width so crossed sheets match
        // brightness + overall fade; green->red gradient lives in mesh vertex colors. Kept soft (cores
        // don't blow to white) for ethereal look, not laser beams.
        let i = (night * (0.30 + 0.7 * flick)).clamp(0.0, 1.25);
        let alpha = (night * (0.2 + 0.7 * flick)).clamp(0.0, 0.85);
        if let Some(mat) = mats.get_mut(&mm.0) {
            mat.base_color = Color::LinearRgba(LinearRgba::new(i, i, i, alpha));
        }
    }
}

// Globe climate recolor: as slow Climate grid drifts, repaint planet LAND vertices (dry -> sand, wet ->
// green) -> deserts + rainforests form/migrate over time. Throttled (climate geological): repaints every
// GLOBE_RECOLOR_TICKS sim-ticks. Cheap: one pass rewriting ATTRIBUTE_COLOR from bilinear-sampled climate
// moisture (ocean depth + polar ice branches moisture-independent -> stable).
const GLOBE_RECOLOR_TICKS: u32 = 600; // ~10 sim-seconds between repaints (51k verts, negligible cost)
fn update_globe_climate(
    gen: Res<GenState>,
    climate: Res<crate::sim::Climate>,
    planet: Query<&Mesh3d, With<Planet>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut next: Local<u32>,
) {
    if gen.tick < *next {
        return; // not time yet (also paints once at startup: tick 0 >= next 0)
    }
    *next = gen.tick + GLOBE_RECOLOR_TICKS;
    let Ok(h) = planet.single() else { return };
    let Some(mesh) = meshes.get_mut(&h.0) else { return };
    // clone positions so immutable borrow ends before re-inserting color attribute
    let positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
        _ => return,
    };
    let colors: Vec<[f32; 4]> = positions
        .iter()
        .map(|p| {
            // vertex pos = d * (R + elevation) -> normalize recovers surface direction
            let d = Vec3::new(p[0], p[1], p[2]).normalize_or_zero();
            let m = climate.sample(d);
            let c = crate::sphere::biome_color_with_moisture(d, m);
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}

// Top-center day/night phase readout (walk mode). Shows where in cycle you are.
#[derive(Component)]
struct DayCycleText;

fn spawn_daycycle_ui(mut commands: Commands) {
    // full-width centered row -> child text sits middle-top
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((
            Text::new(""),
            TextFont { font_size: 22.0, ..default() },
            TextColor(Color::WHITE),
            DayCycleText,
        ));
}

// Set phase label + color from local daylight at walker (visual sky time). Hidden in orbit (you see whole
// terminator there). rising vs falling splits dawn/dusk + morning/afternoon.
fn update_daycycle(
    gen: Res<GenState>,
    offset: Res<SunOffset>,
    mode: Res<crate::camera::CameraMode>,
    walkers: Query<&crate::camera::WalkCam>,
    mut q: Query<(&mut Text, &mut TextColor), With<DayCycleText>>,
) {
    let Ok((mut text, mut color)) = q.single_mut() else { return };
    if *mode != crate::camera::CameraMode::Walk {
        text.0.clear(); // orbit: no single time-of-day
        return;
    }
    let dir = walkers.single().map(|w| w.dir.normalize_or_zero()).unwrap_or(Vec3::Y);
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32; // full tick: keep seasons
    let d = crate::sphere::daylight_at(dir, vtick);
    let ahead = crate::sphere::daylight_at(dir, vtick + 30);
    let rising = ahead > d;
    let (label, c) = if d < 0.04 {
        ("NIGHT", Color::srgb(0.55, 0.62, 0.95))
    } else if d < 0.22 {
        if rising { ("DAWN", Color::srgb(1.0, 0.6, 0.35)) } else { ("DUSK", Color::srgb(1.0, 0.45, 0.3)) }
    } else if d < 0.5 {
        if rising { ("MORNING", Color::srgb(1.0, 0.85, 0.5)) } else { ("AFTERNOON", Color::srgb(1.0, 0.8, 0.45)) }
    } else {
        ("MIDDAY", Color::srgb(1.0, 0.95, 0.6))
    };
    if text.0 != label {
        text.0 = label.into();
    }
    color.0 = c;
}

fn spawn_world_stats_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("world..."),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgb(0.78, 0.9, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
        WorldStatsText,
    ));
}

// --- legend overlay (H toggles full panel explaining every HUD field + control) ---

#[derive(Resource, Default)]
struct ShowLegend(bool);

#[derive(Component)]
struct LegendText;

const LEGEND: &str = "\
EVOLVARIUM  -  legend   (press H to close)

TOP-CENTER (walk mode)
  time-of-day phase where you stand: NIGHT / DAWN /
  MORNING / MIDDAY / AFTERNOON / DUSK (colored by sun).

DASHBOARD (bottom-left)
  speed       sim pace; 1x = real-time. PAUSED = stopped.
  pop         creatures alive right now.
  day         days elapsed (one sun rotation = a day).
  trend       population over recent time (mini graph;
              taller = more, scaled to the carrying cap).
  temp avg    average heat preference. cold = polar-loving,
              warm = equator-loving creatures.
  longevity   average lifespan gene (high = long-lived).
  metab       metabolism gene (high = frugal/slow,
              low = fast but costly).
  r/K         breeding style: low = many cheap young fast,
              high = few well-provisioned young.
  habitat     aquatic = swimmers, flying = fliers, land = land-dwellers.
  specialists creatures locked to one food type.

HOW CREATURES LOOK
  hue         what they eat (diet specialization).
  vividness   how specialized (vivid) vs generalist (pale).
  cyan + flat  swimmers (fish-shaped body).
  size        bigger = more sensors / body size genes.

CONTROLS
  TAB         switch ORBIT (space) <-> WALK (ground)
  O           toggle real sun shadows (on) -- both modes
  ORBIT: right-drag rotate, scroll/W,S zoom, A/D spin,
         Q/E tilt, left-click inspect, F follow
  WALK:  WASD move, arrows or right-drag look,
         Shift run (eye walks over the hills)
         swim into the SEA: look + W to dive/rise
         (view tints blue while underwater)
         [ / ]  scrub time-of-day   \\  jump to noon
         (walk keeps true sim time; scrub [ ] for
          low sun + long shadows; night goes dark)
  G  sensor rays   SPACE  pause/resume
  1-5  speed presets (slow..fast)   + / -  fine speed
  B  seed creatures    P  populate whole planet
  L  lightning fire    K  cull    H  this legend
  M  cycle minimap field    Y  phylogeny (species tree)
  O  save full world -> savestate.json (reload: --load=savestate.json)
  (P seeds plants+trees+creatures in every habitat)";

fn spawn_legend_ui(mut commands: Commands) {
    commands.spawn((
        Text::new(LEGEND),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgb(0.92, 0.96, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            left: Val::Px(40.0),
            padding: UiRect::all(Val::Px(14.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.04, 0.08, 0.86)),
        Visibility::Hidden,
        LegendText,
    ));
}

// H toggles legend panel. Starts hidden; top-left hint tells player it exists.
fn toggle_legend(
    keys: Res<ButtonInput<KeyCode>>,
    mut show: ResMut<ShowLegend>,
    mut q: Query<&mut Visibility, With<LegendText>>,
) {
    if keys.just_pressed(KeyCode::KeyH) {
        show.0 = !show.0;
        for mut v in &mut q {
            *v = if show.0 { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
}

// --- Lineage / phylogeny view (M7 data, spec 09 open-q #7). Render-only: online genetic clustering builds a
// live SPECIES tree from the living population, no sim/spawn changes (keeps determinism + shared worktree safe).
// Works because offspring resemble parents -> cluster-by-similarity tracks descent; a lineage that drifts past
// PHY_THRESH from every existing species BUDS a new species whose parent = its nearest relative (its ancestor).
// 'Y' toggles a panel drawing the species tree (indented by ancestry, colored by clade, live pop + peak + tags).
const PHY_K: usize = 10; // trait-vector dims (niche-defining heritable genes)
const PHY_W: [f32; PHY_K] = [2.2, 2.4, 2.0, 1.2, 1.6, 1.4, 1.0, 0.8, 0.9, 0.7]; // weight niche-defining genes up
const PHY_THRESH: f32 = 0.9; // weighted distance beyond which a creature founds a NEW species
const PHY_EMA: f32 = 0.03; // centroid drift per member per pass (species slowly tracks its members)
const PHY_INTERVAL: u32 = 180; // ticks between classification passes (cheap: few species)
const PHY_MAX_SPECIES: usize = 80; // cap new-species creation (beyond -> force nearest) to bound memory + panel
const PHY_ROOT: u32 = u32::MAX; // sentinel parent for a founding (root) species

// Niche-defining heritable genes -> trait vector for clustering. Order matches PHY_W.
fn phy_traits(g: &Genome) -> [f32; PHY_K] {
    [g.swim, g.flight, g.alpine, g.size, g.carnivory, g.temp_pref, g.bite, g.light_pref, g.height, g.social]
}
fn phy_dist(a: &[f32; PHY_K], b: &[f32; PHY_K]) -> f32 {
    let mut s = 0.0;
    for i in 0..PHY_K {
        let d = (a[i] - b[i]) * PHY_W[i];
        s += d * d;
    }
    s.sqrt()
}
// Short clade label from a species centroid (what KIND of creature this lineage is).
fn phy_tags(c: &[f32; PHY_K]) -> String {
    let (swim, flight, alpine, size, carn, temp) = (c[0], c[1], c[2], c[3], c[4], c[5]);
    let mut t: Vec<&str> = Vec::new();
    t.push(if flight > 0.5 { "flier" } else if swim > 0.5 { "swimmer" } else if alpine > 0.5 { "alpine" } else { "land" });
    t.push(if carn > 0.6 { "carnivore" } else if carn < 0.3 { "herbivore" } else { "omnivore" });
    if size > 0.62 {
        t.push("large");
    } else if size < 0.3 {
        t.push("small");
    }
    if temp < 0.35 {
        t.push("cold");
    } else if temp > 0.65 {
        t.push("warm");
    }
    t.join(" ")
}

struct Species {
    id: u32,
    parent: u32, // parent species id; PHY_ROOT = founding/root species
    centroid: [f32; PHY_K],
    color: Color,
    alive: u32, // recomputed each classification pass
    peak: u32,
}

#[derive(Resource, Default)]
struct Phylogeny {
    species: Vec<Species>,
    next_id: u32,
    last_tick: u32,
    revision: u32,      // bump when species set/counts change -> panel rebuilds
    lines: Vec<Entity>, // current panel line-entities (despawned + rebuilt on revision change)
}

#[derive(Resource, Default)]
struct ShowPhylo(bool);
#[derive(Component)]
struct PhyloPanel;
#[derive(Component)]
struct PhyloTitle;

// Classify the living population into species every PHY_INTERVAL ticks: assign each creature to its nearest
// species (drifts the centroid), or bud a NEW species (parent = nearest) when it's past PHY_THRESH from all.
fn phylogeny_classify(gen: Res<GenState>, creatures: Query<&Genome, With<Creature>>, mut phy: ResMut<Phylogeny>) {
    let first = phy.last_tick == 0 && phy.species.is_empty();
    if !first && gen.tick.saturating_sub(phy.last_tick) < PHY_INTERVAL {
        return;
    }
    phy.last_tick = gen.tick.max(1);
    let tick = gen.tick;
    for s in &mut phy.species {
        s.alive = 0;
    }
    for g in creatures.iter() {
        let v = phy_traits(g);
        let (mut best, mut bi) = (f32::INFINITY, usize::MAX);
        for (i, s) in phy.species.iter().enumerate() {
            let d = phy_dist(&v, &s.centroid);
            if d < best {
                best = d;
                bi = i;
            }
        }
        if bi != usize::MAX && (best < PHY_THRESH || phy.species.len() >= PHY_MAX_SPECIES) {
            let s = &mut phy.species[bi];
            for k in 0..PHY_K {
                s.centroid[k] += (v[k] - s.centroid[k]) * PHY_EMA;
            }
            s.alive += 1;
        } else {
            let parent = if bi == usize::MAX { PHY_ROOT } else { phy.species[bi].id };
            let id = phy.next_id;
            phy.next_id += 1;
            let color = creature_look(g).0;
            phy.species.push(Species { id, parent, centroid: v, color, alive: 1, peak: 1 });
        }
    }
    for s in &mut phy.species {
        if s.alive > s.peak {
            s.peak = s.alive;
        }
    }
    let _ = tick;
    phy.revision = phy.revision.wrapping_add(1);
}

// Phylogeny panel sits under the minimap (top-right), hidden until 'Y'. Title child stays; species lines are
// rebuilt as colored children on revision change.
fn spawn_phylo_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(MM_SIZE + MM_MARGIN + 26.0), // below the minimap globe + its label
                right: Val::Px(MM_MARGIN),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                max_width: Val::Px(320.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.04, 0.08, 0.86)),
            Visibility::Hidden,
            PhyloPanel,
        ))
        .with_child((
            Text::new("PHYLOGENY  [Y]"),
            TextFont { font_size: 13.0, ..default() },
            TextColor(Color::srgb(0.9, 0.95, 1.0)),
            PhyloTitle,
        ));
}

// Y toggles the phylogeny panel. Bump revision on open so it rebuilds immediately with current species.
fn toggle_phylo(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowPhylo>, mut phy: ResMut<Phylogeny>, mut q: Query<&mut Visibility, With<PhyloPanel>>) {
    if keys.just_pressed(KeyCode::KeyY) {
        show.0 = !show.0;
        if show.0 {
            phy.revision = phy.revision.wrapping_add(1);
        }
        for mut v in &mut q {
            *v = if show.0 { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
}

// Rebuild the species-tree lines when visible + the species set/counts changed. Shows living species + the
// ancestor nodes that connect them to a root (extinct dead-end branches hidden), depth-first, indented by
// ancestry, each line colored by clade.
fn update_phylo_panel(
    show: Res<ShowPhylo>,
    mut phy: ResMut<Phylogeny>,
    mut commands: Commands,
    panel: Query<Entity, With<PhyloPanel>>,
    mut title: Query<&mut Text, With<PhyloTitle>>,
    mut last_rev: Local<u32>,
) {
    if !show.0 || phy.revision == *last_rev {
        return;
    }
    *last_rev = phy.revision;
    let Ok(panel_e) = panel.single() else { return };

    let n = phy.species.len();
    // keep = alive, or an ancestor of an alive species (connect the living tree to its roots)
    let mut keep = vec![false; n];
    let pos_of = |id: u32| phy.species.iter().position(|s| s.id == id);
    for i in 0..n {
        if phy.species[i].alive > 0 {
            keep[i] = true;
            let mut p = phy.species[i].parent;
            while let Some(pi) = pos_of(p) {
                if keep[pi] {
                    break;
                }
                keep[pi] = true;
                p = phy.species[pi].parent;
            }
        }
    }
    // effective parent: ROOT if the real parent isn't kept (orphan -> a root)
    let kept_ids: std::collections::HashSet<u32> = (0..n).filter(|&i| keep[i]).map(|i| phy.species[i].id).collect();
    let eff: Vec<u32> = (0..n)
        .map(|i| {
            let p = phy.species[i].parent;
            if p != PHY_ROOT && kept_ids.contains(&p) { p } else { PHY_ROOT }
        })
        .collect();
    // pre-order DFS from roots (parent id PHY_ROOT); children ordered by id (= birth order) -> indented tree
    let mut rows: Vec<(usize, usize)> = Vec::new(); // (species index, depth)
    fn preorder(species: &[Species], keep: &[bool], eff: &[u32], pid: u32, depth: usize, rows: &mut Vec<(usize, usize)>) {
        let mut kids: Vec<usize> = (0..species.len()).filter(|&i| keep[i] && eff[i] == pid).collect();
        kids.sort_by_key(|&i| species[i].id);
        for i in kids {
            rows.push((i, depth));
            preorder(species, keep, eff, species[i].id, depth + 1, rows);
        }
    }
    preorder(&phy.species, &keep, &eff, PHY_ROOT, 0, &mut rows);

    // snapshot display data (release phy borrow before spawning)
    let alive_total: u32 = phy.species.iter().map(|s| s.alive).sum();
    let alive_species = phy.species.iter().filter(|s| s.alive > 0).count();
    let total_species = phy.species.len();
    let display: Vec<(String, Color)> = rows
        .iter()
        .map(|&(i, depth)| {
            let s = &phy.species[i];
            let mark = if s.alive > 0 { "●" } else { "·" };
            let line = format!("{}{} sp{} {}  x{} (pk {})", "  ".repeat(depth), mark, s.id, phy_tags(&s.centroid), s.alive, s.peak);
            let col = if s.alive > 0 { s.color } else { s.color.with_alpha(0.45) };
            (line, col)
        })
        .collect();

    if let Ok(mut t) = title.single_mut() {
        t.0 = format!("PHYLOGENY  {} alive / {} ever, {} creatures  [Y]", alive_species, total_species, alive_total);
    }
    for e in phy.lines.drain(..) {
        commands.entity(e).despawn();
    }
    let mut new_lines = Vec::with_capacity(display.len());
    for (line, col) in display {
        let e = commands
            .spawn((Text::new(line), TextFont { font_size: 12.0, ..default() }, TextColor(col), ChildOf(panel_e)))
            .id();
        new_lines.push(e);
    }
    phy.lines = new_lines;
}

// Unicode sparkline of history series, scaled 0..max.
fn sparkline(hist: &[u16], max: f32) -> String {
    const B: [char; 8] = ['\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    hist.iter()
        .map(|&v| {
            let i = ((v as f32 / max.max(1.0)) * 7.0).round().clamp(0.0, 7.0) as usize;
            B[i]
        })
        .collect()
}

// Recompute world dashboard each frame from living population (cheap aggregate over creatures). Keeps
// rolling population history (sampled ~1x/sec), renders as sparkline -> lightweight "population over
// time" chart (M7) in HUD.
fn update_world_stats(
    gen: Res<GenState>,
    vtime: Res<Time<Virtual>>,
    creatures: Query<(&Genome, &Alive), With<Creature>>,
    mut text: Query<&mut Text, With<WorldStatsText>>,
    mut hist: Local<Vec<u16>>,
    mut frame: Local<u32>,
) {
    let Ok(mut t) = text.single_mut() else { return };
    let (mut n, mut temp, mut lng, mut met, mut par) = (0u32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let (mut cold, mut warm, mut aq, mut fly, mut land, mut spec) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    for (g, alive) in &creatures {
        if !alive.0 {
            continue;
        }
        n += 1;
        temp += g.temp_pref;
        lng += g.longevity;
        met += g.metab;
        par += g.parental;
        if g.temp_pref < 0.4 { cold += 1; } else if g.temp_pref > 0.6 { warm += 1; }
        // mutually exclusive, niche-priority order (mirror niche::niche_of): aquatic, then aerial, then land
        if g.swim > 0.6 { aq += 1; } else if g.flight >= crate::sim::FLIGHT_KNEE { fly += 1; } else if g.swim < 0.3 { land += 1; }
        if g.rigidity > 0.6 { spec += 1; }
    }
    let nf = n.max(1) as f32;
    // calendar date + Sirius-precession readout (the year breathes with Sirius distance)
    let date = crate::sphere::fmt_date(gen.tick);
    let ty = crate::sphere::t_years(gen.tick);
    let yrlen = crate::orrery::year_len_days(ty).round() as i32;
    let sdist = crate::orrery::sirius_dist01(ty); // 0 near .. 1 far
    let prox = if sdist > 0.66 { "far" } else if sdist < 0.33 { "near" } else { "mid" };
    let prec = ((crate::orrery::apparent_precession_years(ty) / 100.0).round() as i32) * 100;
    // sample population ~1x/sec into rolling history (~48 samples) for trend sparkline
    *frame += 1;
    if (*frame).is_multiple_of(60) {
        hist.push(n as u16);
        if hist.len() > 48 {
            hist.remove(0);
        }
    }
    let trend = sparkline(&hist, crate::sim::CREATURE_CAP as f32);
    let speed = if vtime.is_paused() {
        "PAUSED".to_string()
    } else {
        format!("{:.2}x", vtime.relative_speed())
    };
    t.0 = format!(
        "WORLD\nspeed      {speed}\npop        {n}\ndate       {date}\nsky        yr {yrlen}d  sirius {prox}  prec ~{prec}\ntrend      {trend}\ntemp avg   {:.2}  (cold {cold} / warm {warm})\nlongevity  {:.2}\nmetab      {:.2}\nr/K        {:.2}\nhabitat    aquatic {aq} / flying {fly} / land {land}\nspecialists {spec}",
        temp / nf, lng / nf, met / nf, par / nf
    );
}

fn spawn_stats_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("press H for legend  -  left-click a creature or plant to inspect"),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
        StatsText,
    ));
}

// Ray-sphere hit: nearest positive t along (origin + t*dir) intersecting sphere, else None.
fn ray_hit(origin: Vec3, dir: Vec3, center: Vec3, r: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let disc = b * b - (oc.dot(oc) - r * r);
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let t = -b - s;
    if t >= 0.0 {
        Some(t)
    } else {
        let t2 = -b + s;
        (t2 >= 0.0).then_some(t2)
    }
}

// Left-click picks nearest creature/plant under cursor (only when not in look mode).
fn pick_on_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
    cam: Query<(&Camera, &GlobalTransform)>,
    creatures: Query<(Entity, &GlobalTransform), With<Creature>>,
    foods: Query<(Entity, &GlobalTransform, Option<&Tree>), With<Food>>,
    mut selected: ResMut<Selected>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok((window, cursor_opts)) = windows.single() else { return };
    if cursor_opts.grab_mode != CursorGrabMode::None {
        return; // look mode (right-click captured): cursor not usable for picking
    }
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, cam_tf)) = cam.single() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else { return };
    let (o, d) = (ray.origin, *ray.direction);
    let mut best: Option<(f32, Entity)> = None;
    let consider = |e: Entity, pos: Vec3, r: f32, best: &mut Option<(f32, Entity)>| {
        if let Some(t) = ray_hit(o, d, pos, r) {
            if best.is_none_or(|(bt, _)| t < bt) {
                *best = Some((t, e));
            }
        }
    };
    for (e, t) in &creatures {
        consider(e, t.translation(), 1.0, &mut best);
    }
    for (e, t, tree) in &foods {
        // trees big -> generous pick radius scaled by size; plants small
        let r = if tree.is_some() { 2.0 * t.compute_transform().scale.max_element() } else { 0.8 };
        consider(e, t.translation(), r, &mut best);
    }
    if let Some((_, e)) = best {
        selected.entity = Some(e);
    }
    // miss keeps current selection (so follow not lost by stray click)
}

// Draw yellow ring around selected entity each frame -> shows what's picked.
fn draw_selection(selected: Res<Selected>, q: Query<&GlobalTransform>, mut gizmos: Gizmos) {
    if let Some(e) = selected.entity {
        if let Ok(tf) = q.get(e) {
            // ring scales with entity size -> reads around big things (trees) too
            let r = 1.0 + 1.4 * tf.compute_transform().scale.max_element();
            gizmos.sphere(tf.translation(), r, Color::srgb(1.0, 1.0, 0.2));
        }
    }
}

// Update on-screen panel with selected entity live stats (creature or plant/carrion).
fn update_stats(
    selected: Res<Selected>,
    creatures: Query<(&Energy, &Fitness, &Genome, &DietState, &Alive)>,
    foods: Query<(&PlantGenome, &PlantState, Option<&Rot>, Option<&Tree>)>,
    mut text: Query<&mut Text, With<StatsText>>,
) {
    let Ok(mut text) = text.single_mut() else { return };
    let Some(e) = selected.entity else {
        text.0 = "left-click a creature or plant to inspect".into();
        return;
    };
    if let Ok((energy, fit, g, diet, alive)) = creatures.get(e) {
        // dominant nutrient gut targets (highest uptake gene) + current master digestion expression
        let mut dom = 0;
        for t in 1..NUTRIENTS {
            if g.uptake[t] > g.uptake[dom] {
                dom = t;
            }
        }
        let breadth = g.uptake.iter().filter(|u| **u > 0.4).count(); // nutrients actively absorbed (uptake > 0.4)
        let master = master_expression(&g.uptake, &diet.reserves, crate::config::RESERVE_REQ, crate::config::MASTER_FLOOR);
        let mode = if g.light_pref > 0.6 { "diurnal" } else if g.light_pref < 0.4 { "nocturnal" } else { "cathemeral" };
        // niche-priority (mirror niche::niche_of + the WORLD habitat tally): aquatic, then flying, then land
        let habitat = if g.swim > 0.6 { "aquatic" }
            else if g.flight >= crate::sim::FLIGHT_KNEE { "flying" }
            else if g.swim < 0.3 { "land" }
            else { "amphibious" };
        let clime = if g.temp_pref > 0.6 { "warm" } else if g.temp_pref < 0.4 { "cold" } else { "temperate" };
        text.0 = format!(
            "CREATURE  {}\nenergy   {:.1}  f{:.0}/s{:.0}/fat{:.0}\nadiposity {:.2}\nfitness  {:.1}\nsensors  {}\nbite     {:.2}\nheight   {:.2}\nsize     {:.2}\nswim     {:.2} ({})\nflight   {:.2}\nsocial   {:.2}\ntemp     {:.2} ({})\nlongevity {:.2}\nmetab    {:.2}\nparental {:.2}\nrigidity {:.2}\nlight    {:.2} ({})\nfatigue  {:.2}\ngut>top n{} (master {:.2})\nbreadth  {}\nload(G)  {:.2}\nage      {}",
            if alive.0 { "alive" } else { "DEAD" },
            energy.total(),
            energy.fast,
            energy.sugar,
            energy.fat,
            g.adiposity,
            fit.0,
            g.n_sensors(),
            g.bite,
            g.height,
            g.size,
            g.swim,
            habitat,
            g.flight,
            g.social,
            g.temp_pref,
            clime,
            g.longevity,
            g.metab,
            g.parental,
            g.rigidity,
            g.light_pref,
            mode,
            diet.fatigue,
            dom,
            master,
            breadth,
            diet.g,
            crate::sphere::fmt_age_days(diet.age),
        );
    } else if let Ok((pg, st, rot, tree)) = foods.get(e) {
        if let Some(tree) = tree {
            // creature height to feed = tree height - base margin - branch reach (branches hang fruit low)
            let reach = (pg.height - crate::sim::TREE_REACH_MARGIN - pg.branches * crate::sim::BRANCH_REACH).max(0.0);
            text.0 = format!(
                "TREE  {}\nheight   {:.2}\nbranches {:.2}\nmass     {:.1}\nnutrient {:.2}\n{}",
                if tree.edible { "fruit (branches lower reach)" } else { "evergreen (uneatable)" },
                pg.height,
                pg.branches,
                st.mass,
                pg.nutrient,
                if tree.edible { format!("reach: creature height >= {:.2}", reach) } else { "pure structure / refuge".into() },
            );
        } else if let Some(rot) = rot {
            let f = (rot.age as f32 / ROT_GONE as f32 * 100.0).min(100.0);
            text.0 = format!(
                "CARRION / DETRITUS\nrotted   {:.0}%\nmass     {:.1}\nnutrient {:.2}",
                f, st.mass, pg.nutrient
            );
        } else {
            text.0 = format!(
                "PLANT  type {}\nmass     {:.1}\nnutrient {:.2}\ndefense  {:.2}\nquality  {:.2}\nwet-pref {:.2}",
                pg.kind, st.mass, pg.nutrient, pg.defense, pg.quality, pg.wet
            );
        }
    } else {
        text.0 = "(selection gone: eaten or died)".into();
    }
}

// Draw each sensor as ray from creature along (heading + angle), length = range.
fn draw_sensors(
    show: Res<ShowSensors>,
    mut gizmos: Gizmos,
    q: Query<(&Transform, &Heading, &Genome), With<Creature>>,
) {
    if !show.0 {
        return;
    }
    for (tf, head, g) in &q {
        let p = tf.translation;
        for s in &g.sensors {
            let a = head.0 + s.angle;
            let dir = Vec3::new(a.sin(), 0.0, a.cos());
            gizmos.line(p, p + dir * s.range, Color::srgb(1.0, 1.0, 0.3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::noon_offset;
    use bevy::prelude::Vec3;

    // noon_offset must put the sun on the walk point's meridian = the BRIGHTEST moment of that day. (With
    // real seasons the noon sun can be low at high latitude in winter, so it is no longer ~overhead; the
    // robust invariant is that noon is the daily daylight peak and the point is lit.)
    #[test]
    fn noon_offset_lights_the_walk_point() {
        let dt = crate::sphere::DAY_TICKS as i64;
        for d in [
            Vec3::new(0.30, 0.50, 0.40),   // homeland
            Vec3::new(0.95, 0.30, -0.05),  // mountain A
            Vec3::new(-0.10, -0.30, 0.95), // deep ocean
            Vec3::new(-0.65, 0.20, -0.75), // mountain B
        ] {
            let d = d.normalize();
            for tick in [0u32, 600, 1234, 9_000_000] {
                let off = noon_offset(d, tick);
                // noon_offset applies to the FULL tick (Tychos sun has a yearly term; mod-DAY_TICKS samples
                // a different season). vtick = tick + off.
                let base = (tick as i64 + off).max(0);
                let noon = crate::sphere::daylight_at(d, base as u32);
                assert!(noon > 0.2, "walk point should be daylit at noon, got {noon} for d={d:?} tick={tick}");
                // noon is the daily peak: >= daylight at every other hour of the same day.
                for h in 1..24 {
                    let other = crate::sphere::daylight_at(d, (base + h * dt / 24).max(0) as u32);
                    assert!(noon + 2e-3 >= other, "noon {noon} < hour {h} {other} for d={d:?} tick={tick}");
                }
            }
        }
    }
}
