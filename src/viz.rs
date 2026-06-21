// Render-only visualization: make evolved variability visible.
//   - creature COLOR = dominant diet specialization (hue) + rigidity (saturation: specialist=vivid)
//   - creature SIZE  = sensor count (more eyes = bigger/more complex body)
//   - press G        = draw each creature's sensor rays (the evolved eye layout)
// All cosmetic; never touches sim state.
use bevy::prelude::*;

use crate::components::{Alive, Creature, DietState, Energy, Fitness, Food, Heading, Rot, Tree};
use crate::genome::{Genome, NFOOD};
use crate::plant::{plant_color, PlantGenome, PlantState};
use crate::sim::{grid_cell_surface, Fire, GenState, GroundWater, ROT_GONE};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

// Visual time-of-day offset (ticks) added to the sun ONLY for lighting + the sun/moon sky. Sim daylight
// (creature rest, plant growth) still reads raw tick. Lets walk mode snap to local noon + scrub the sun
// for nice low-angle shadows without fast-forwarding the sim. 0 = sky matches sim time (orbit default).
#[derive(Resource, Default)]
pub struct SunOffset(pub i64);

// Walk-mode shadow toggle (O). Default OFF so the sunlit world is always visible; the directional shadow
// range can black out the ground receiver, so shadows are opt-in. camera::update_shadow_mode reads this.
#[derive(Resource, Default)]
pub struct ShowShadows(pub bool);

// Markers for the celestial bodies (animated by day_night_lighting).
#[derive(Component)]
pub struct SunLight;
#[derive(Component)]
pub struct Moon;
#[derive(Component)]
pub struct SunDisc; // the visible glowing sun (follows the light direction)

pub struct VizPlugin;

impl Plugin for VizPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowSensors>()
            .init_resource::<Selected>()
            .init_resource::<ShowLegend>()
            .init_resource::<SunOffset>()
            .insert_resource(ShowShadows(true)) // walk shadows on by default (O toggles)
            .add_systems(Startup, (log_viz_help, spawn_stats_ui, spawn_world_stats_ui, spawn_legend_ui, spawn_daycycle_ui, spawn_clouds, set_initial_speed))
            .add_systems(
                Update,
                (
                    restyle_creatures,
                    add_creature_visuals,
                    toggle_sensors,
                    draw_sensors,
                    add_plant_visuals,
                    size_plants,
                    (day_night_lighting, time_of_day, toggle_shadows, walk_ambient, update_daycycle),
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
                    god_disturbances,
                    draw_selection,
                ),
            );
    }
}

// Shared sphere mesh for plants (inserted by spawn_world_render).
#[derive(Resource)]
pub struct PlantMesh(pub Handle<Mesh>);

// Shared creature capsule mesh (inserted by spawn_world_render) so add_creature_visuals can dress
// creatures born mid-sim (spawn_creature adds no mesh) -> newborns + B-seeded creatures become visible.
#[derive(Resource)]
pub struct CreatureMesh(pub Handle<Mesh>);


// Color + body-plan scale from a genome. Shared by add_creature_visuals (initial look) and
// restyle_creatures (on genome change) so newborns look right immediately, not default-orange.
fn creature_look(g: &Genome) -> (Color, Vec3) {
    let mut dom = 0;
    let mut best = g.expr0[0];
    for t in 1..NFOOD {
        if g.expr0[t] > best {
            best = g.expr0[t];
            dom = t;
        }
    }
    let sat = 0.2 + 0.7 * g.rigidity; // pinned specialist = vivid, generalist = washed out
    let hue = type_hue(dom) * (1.0 - g.swim) + 200.0 * g.swim; // swim shifts toward cyan
    let girth = (0.7 + 0.06 * g.n_sensors() as f32) * (0.6 + 0.9 * g.size);
    let sx = girth * (1.0 - 0.25 * g.swim);
    let sy = girth * (0.7 + 1.6 * g.height) * (1.0 - 0.3 * g.swim);
    let sz = girth * (1.0 + 0.8 * g.swim); // swim = flatter + longer (fish shape)
    (Color::hsl(hue, sat, 0.55), Vec3::new(sx, sy, sz))
}

// Give any creature lacking a mesh its visuals (shared capsule + own genome-colored material). Covers
// creatures BORN mid-sim and B-seeded ones (spawn_creature adds no render mesh). Without this they are
// invisible while alive and only appear once dead (carrion gets its own mesh).
fn add_creature_visuals(
    mut commands: Commands,
    mesh: Option<Res<CreatureMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &Genome, &mut Transform), (With<Creature>, Without<Mesh3d>)>,
) {
    let Some(mesh) = mesh else { return };
    for (e, g, mut tf) in &mut q {
        let (color, scale) = creature_look(g);
        tf.scale = scale;
        commands
            .entity(e)
            .insert((Mesh3d(mesh.0.clone()), MeshMaterial3d(materials.add(color))));
    }
}

// Tree part meshes (inserted by spawn_world_render): a trunk + two canopy shapes.
#[derive(Resource)]
pub struct TreeMeshes {
    pub trunk: Handle<Mesh>,
    pub broadleaf: Handle<Mesh>, // round canopy for fruit trees
    pub conifer: Handle<Mesh>,   // cone canopy for evergreens
}

// Give any plant lacking a mesh its visuals: shared sphere + a material colored by its genome
// (hue=kind, brightness=nutrient, warmth=defense). Covers initial plants AND new offspring.
fn add_plant_visuals(
    mut commands: Commands,
    mesh: Option<Res<PlantMesh>>,
    trees: Option<Res<TreeMeshes>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<(Entity, &PlantGenome, Option<&Tree>), (With<Food>, Without<Mesh3d>)>,
) {
    let Some(mesh) = mesh else { return };
    for (e, g, tree) in &q {
        match (tree, &trees) {
            // tree = a brown trunk (this entity) + a green canopy child (cone evergreen / round fruit)
            (Some(t), Some(tm)) => {
                commands.entity(e).insert((
                    Mesh3d(tm.trunk.clone()),
                    MeshMaterial3d(materials.add(Color::srgb(0.40, 0.26, 0.13))),
                ));
                let (canopy, color) = if t.edible {
                    (tm.broadleaf.clone(), Color::srgb(0.20, 0.60, 0.16))
                } else {
                    (tm.conifer.clone(), Color::srgb(0.06, 0.30, 0.18))
                };
                let child = commands
                    .spawn((
                        Mesh3d(canopy),
                        MeshMaterial3d(materials.add(color)),
                        Transform::from_xyz(0.0, 2.2, 0.0),
                    ))
                    .id();
                commands.entity(e).add_child(child);
            }
            // plain plant: shared sphere, colored by genome
            _ => {
                commands
                    .entity(e)
                    .insert((Mesh3d(mesh.0.clone()), MeshMaterial3d(materials.add(plant_color(g)))));
            }
        }
    }
}

// Hide a creature's mesh when it dies (P1.4); restore on rebirth at the generation boundary
// (Alive flips back true). Its carrion (a separate Food entity) appears in its place.
fn hide_dead(mut q: Query<(&Alive, &mut Visibility), With<Creature>>) {
    for (alive, mut vis) in &mut q {
        let want = if alive.0 { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

// Carrion/detritus (Rot) color tells its rot stage: fresh = meaty red, rotten = dark muddy green.
// So the rot chain (P3) reads at a glance: bright red corpse -> darkening -> gone.
fn color_carrion(mut mats: ResMut<Assets<StandardMaterial>>, q: Query<(&Rot, &MeshMaterial3d<StandardMaterial>)>) {
    for (rot, mm) in &q {
        let f = (rot.age as f32 / ROT_GONE as f32).clamp(0.0, 1.0); // 0 fresh .. 1 rotten
        if let Some(m) = mats.get_mut(&mm.0) {
            m.base_color = Color::hsl(10.0 + 90.0 * f, 0.6, 0.5 - 0.35 * f); // red->sick-green, darkening
        }
    }
}

// Scale plants by mass (growth visible) AND root them on the terrain. The height gene STRETCHES a
// plant vertically (taller plant) rather than lifting it into the air -> tall plants read as tall but
// their base stays on the ground (no floating). Trees render much bigger (tall trunk + canopy).
fn size_plants(mut q: Query<(&PlantState, &PlantGenome, &mut Transform, Option<&Tree>), With<Food>>) {
    for (st, g, mut tf, tree) in &mut q {
        let up = tf.translation.normalize_or_zero(); // outward surface normal at this spot
        let base = crate::sphere::surface_pos(up, 0.0); // foot on the terrain surface
        let rot = Quat::from_rotation_arc(Vec3::Y, up); // grow outward from the planet, not world-up
        if tree.is_some() {
            // trees stay small relative to the planet (was up to ~13 units on an 80-radius world, which
            // poked into the clouds). Now a tree is ~2-4 units tall.
            let s = (0.35 + 0.12 * st.mass).clamp(0.35, 1.1);
            tf.scale = Vec3::splat(s);
            tf.rotation = rot;
            tf.translation = base + up * (1.5 * s); // trunk base rests on the surface
        } else {
            // sphere mesh radius 0.35; girth from mass, modest vertical stretch from the height gene
            let girth = (0.2 + 0.1 * st.mass).clamp(0.2, 1.0);
            let tall = 1.0 + 1.4 * g.height; // taller plants = harder for short creatures to reach
            tf.scale = Vec3::new(girth, girth * tall, girth);
            tf.rotation = rot;
            tf.translation = base + up * (0.35 * girth * tall); // base rooted on the surface (no float)
        }
    }
}

// Orbit the sun + moon: the directional light comes FROM the sun's current direction (so the lit half of
// the planet + the terminator sweep as it spins), and the moon sphere rides its orbit. The globe self-
// shades via surface normals, so illuminance stays constant; ambient (set in setup) lifts the night side.
fn day_night_lighting(
    gen: Res<GenState>,
    offset: Res<SunOffset>,
    mut suns: Query<&mut Transform, (With<SunLight>, Without<Moon>, Without<SunDisc>)>,
    mut moons: Query<&mut Transform, (With<Moon>, Without<SunLight>, Without<SunDisc>)>,
    mut discs: Query<&mut Transform, (With<SunDisc>, Without<SunLight>, Without<Moon>)>,
) {
    // visual sky time = sim tick + offset (offset lets walk mode pick a sunny hour without moving the sim)
    let vtick = (gen.tick as i64 + offset.0).rem_euclid(crate::sphere::DAY_TICKS as i64) as u32;
    let sd = crate::sphere::sun_dir(vtick);
    for mut tf in &mut suns {
        // ROTATE the directional light in place (only direction matters). The light carries NoFrustumCulling
        // so it stays ViewVisible -> Bevy keeps building its shadow cascades. (Teleporting it far / to the
        // planet core got it frustum-culled to invisible, which silently disabled shadows.)
        *tf = Transform::IDENTITY.looking_to(-sd, Vec3::Y);
    }
    for mut tf in &mut discs {
        tf.translation = sd * crate::sphere::SUN_DIST; // the visible sun rides the same direction, far out
    }
    let mtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let mp = crate::sphere::moon_pos(mtick);
    for mut tf in &mut moons {
        tf.translation = mp;
    }
}

// Scrub time-of-day in walk mode: [ winds the sun back, ] pushes it forward (golden-hour shadows), \
// snaps to local noon overhead the walker. Adjusts the visual SunOffset only (sim time untouched).
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

// In walk mode, make the camera's ambient fill track the local daylight so NIGHT GOES DARK (flat high
// ambient made everything glow like light from the ground). Night keeps a low moonlit floor so silhouettes
// read; noon gets a bright sky fill. Orbit ambient is left to update_shadow_mode (steady 220 for a crisp
// terminator). Uses the same visual sky time (sim tick + SunOffset) as the sun.
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
    let vtick = (gen.tick as i64 + offset.0).rem_euclid(crate::sphere::DAY_TICKS as i64) as u32;
    let day = crate::sphere::daylight_at(w.dir.normalize_or_zero(), vtick); // 0 night .. 1 noon overhead
    let b = 35.0 + 175.0 * day; // dim moonlit ~35 at night, soft day fill ~210 (low so shadows still read)
    for mut a in &mut ambient {
        a.brightness = b;
    }
}

// Toggle walk-mode shadows (O). Off by default so the sunlit world is always visible; turn on to add real
// tree/creature shadows. If turning them on darkens the whole view, that's the directional-shadow blackout.
fn toggle_shadows(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<crate::camera::CameraMode>,
    mut show: ResMut<ShowShadows>,
) {
    if *mode == crate::camera::CameraMode::Walk && keys.just_pressed(KeyCode::KeyO) {
        show.0 = !show.0;
        info!("walk shadows: {}", if show.0 { "ON" } else { "OFF" });
    }
}

// Tick offset that puts the sun overhead surface dir `d` (local noon). Sun sweeps longitude: its ground
// track angle = 2*PI*tick/DAY_TICKS in the x-z plane, so match d's longitude angle.
pub fn noon_offset(d: Vec3, tick: u32) -> i64 {
    use std::f32::consts::TAU;
    let target = d.z.atan2(d.x).rem_euclid(TAU); // longitude of the walk point
    let want = (target / TAU * crate::sphere::DAY_TICKS as f32).round() as i64;
    let have = (tick as i64).rem_euclid(crate::sphere::DAY_TICKS as i64);
    (want - have).rem_euclid(crate::sphere::DAY_TICKS as i64)
}

// Rain streaks: when a storm is active, draw falling streaks via gizmos (immediate-mode, no entities).
// Count + opacity scale with Weather.rain. Positions are a deterministic scatter that falls + wraps on
// the tick clock (no per-frame RNG -> stays reproducible). Cosmetic; reads the rain cycle at a glance.
// Rain streaks: short radial lines hanging under thick clouds wherever it is currently raining (cloud-
// driven). Sampled on a lat/lon grid; rain is sparse (~10% of thick cloud) so streaks are scattered.
fn rain_visuals(gen: Res<GenState>, mut gizmos: Gizmos) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let (rows, cols) = (44, 88);
    let col = Color::srgba(0.6, 0.7, 0.95, 0.55);
    for j in 0..rows {
        for i in 0..cols {
            let lat = -FRAC_PI_2 + PI * (j as f32 + 0.5) / rows as f32;
            let lon = -PI + TAU * (i as f32 + 0.5) / cols as f32;
            let d = crate::sphere::lonlat_to_pos(lon, lat, 0.0).normalize();
            if crate::sphere::rain_at(d, gen.tick) > 0.0 {
                let base = crate::sphere::surface_pos(d, 0.0);
                gizmos.line(base + d * 9.0, base + d * 3.0, col); // a drop falling toward the surface
            }
        }
    }
}

// Drifting clouds as solid translucent puffs (not wireframe). A fixed grid of flattened white spheres
// rides a shell well above the tallest trees; each frame its opacity + size track the cloud field, which
// scrolls with the wind -> clouds form, drift, and dissolve. CLOUD_ALT clears the terrain + trees.
#[derive(Component)]
struct CloudPuff {
    dir: Vec3,
}

fn cloud_alt() -> f32 {
    crate::sphere::PLANET_R + crate::sphere::ELEV_MAX + 10.0
}

fn spawn_clouds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let mesh = meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap());
    let alt = cloud_alt();
    let (rows, cols) = (16, 32);
    for j in 0..rows {
        for i in 0..cols {
            let lat = -FRAC_PI_2 * 0.92 + (PI * 0.92) * (j as f32 + 0.5) / rows as f32; // skip the exact poles
            let lon = -PI + TAU * (i as f32 + 0.5) / cols as f32;
            let dir = crate::sphere::lonlat_to_pos(lon, lat, 0.0).normalize();
            let mat = materials.add(StandardMaterial {
                base_color: Color::srgba(0.95, 0.96, 1.0, 0.0),
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(dir * alt),
                Visibility::Hidden,
                CloudPuff { dir },
            ));
        }
    }
}

fn update_clouds(
    gen: Res<GenState>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&CloudPuff, &MeshMaterial3d<StandardMaterial>, &mut Visibility, &mut Transform)>,
) {
    for (puff, mm, mut vis, mut tf) in &mut q {
        let c = crate::sphere::cloud_cover(puff.dir, gen.tick);
        if c < 0.18 {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        if let Some(m) = mats.get_mut(&mm.0) {
            m.base_color = Color::srgba(0.95, 0.96, 1.0, (0.18 + 0.5 * c).min(0.7));
        }
        let s = 7.0 + 11.0 * c; // thicker cloud = bigger puff
        tf.scale = Vec3::new(s, s * 0.45, s); // flattened like a cloud
    }
}

// Wildfire glow: an orange flame sphere at each burning fire-cell on the surface, sized by intensity.
fn fire_visuals(fire: Res<Fire>, mut gizmos: Gizmos) {
    for c in 0..fire.cell.len() {
        let f = fire.cell[c];
        if f < 0.1 {
            continue;
        }
        let surf = grid_cell_surface(c);
        let up = surf.normalize_or_zero();
        let col = Color::srgb(1.0, 0.35 + 0.25 * f, 0.05); // deep orange .. yellow-hot
        gizmos.sphere(surf + up * (0.6 + 2.0 * f), 0.6 + 1.8 * f, col);
    }
}

#[derive(Resource, Default)]
pub struct ShowSensors(pub bool);

fn log_viz_help() {
    info!("viz: TAB=orbit/walk (walk arrives at noon; [ ] scrub time, \\ noon) | hue=diet, vividness=rigidity, size=sensors | G=sensor rays | SPACE=pause | 1-5=speed +/-=fine | B=seed life P=populate planet L=lightning K=cull | H=legend");
}

// Hue per dominant food/diet type, matching the food palette (green/purple/gold/cyan).
fn type_hue(t: usize) -> f32 {
    match t {
        0 => 130.0,
        1 => 285.0,
        2 => 45.0,
        _ => 190.0,
    }
}

// Recolor + rescale a creature when its genome changes (spawn + every generation boundary).
fn restyle_creatures(
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&Genome, &MeshMaterial3d<StandardMaterial>, &mut Transform), Changed<Genome>>,
) {
    for (g, mm, mut tf) in &mut q {
        let (color, scale) = creature_look(g); // hue=diet, sat=rigidity, swim=cyan + fish body plan
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

// God-controls (M6): live disturbances to steer the ecosystem + watch it respond. L = lightning strike
// (ignite a wildfire in the driest land cell -> it spreads/burns via fire_step). K = mass-mortality event
// (kill ~a third of creatures -> watch the population recover). Pokes sim resources/state transiently;
// no balance constants changed. Uses no sim RNG (stays deterministic-safe).
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
        // "make more life!" -> seed a burst of creatures cloned from the living pop (competent brains)
        const BURST: usize = 200;
        let parents: Vec<Genome> = creatures.iter().filter(|(_, a)| a.0).map(|(g, _)| g.clone()).collect();
        crate::sim::seed_burst(&mut commands, &mut rng, &parents, BURST);
        info!("god: seeded {BURST} new creatures (clones of the living)");
    }
    if keys.just_pressed(KeyCode::KeyP) {
        // populate the WHOLE planet: plants + trees + creatures, each in habitat it can survive (aquatic in
        // sea, alpine in mountains, climate-matched). Fills every region instead of waiting for spread.
        let parents: Vec<Genome> = creatures.iter().filter(|(_, a)| a.0).map(|(g, _)| g.clone()).collect();
        crate::sim::seed_planet(&mut commands, &mut rng, &parents, gen.ntypes(), 300, 600, 120);
        info!("god: seeded the whole planet (300 creatures, 600 plants, 120 trees)");
    }
    if keys.just_pressed(KeyCode::KeyL) {
        // ignite the driest non-ocean grid cell (most flammable fuel)
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
                    alive.0 = false; // sim turns it into carrion + despawns next step
                    killed += 1;
                }
            }
        }
        info!("god: mass-mortality event -> culled {killed} creatures");
    }
}

// Start the visualizer at a calm, watchable pace so day/night + creature motion read clearly. The sim is
// unchanged (same ticks); only how fast the virtual clock feeds FixedUpdate. Speed up with +/keys anytime.
const VIEW_SPEED_DEFAULT: f32 = 0.35;
fn set_initial_speed(mut vtime: ResMut<Time<Virtual>>) {
    vtime.set_relative_speed(VIEW_SPEED_DEFAULT);
}

// Time god-controls: SPACE pause/resume, +/- halve/double, number keys 1-5 jump to a preset speed. Drives
// Bevy's virtual clock that FixedUpdate advances from -> pausing/speeding it scales the whole sim, no sim
// change. Range 0.1x (study a single creature) .. 16x (fast-forward evolution).
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
    // preset speeds on the number row (1=slowest .. 5=fast)
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

// --- click-to-inspect (left-click selects a creature/plant; an on-screen panel shows its stats) ---

#[derive(Resource, Default)]
pub struct Selected {
    pub entity: Option<Entity>,
    pub follow: bool,        // camera tracks the selected entity (toggle with F)
    pub follow_offset: Vec3, // camera offset from target captured when follow engaged
}

#[derive(Component)]
struct StatsText;

// Live world dashboard (bottom-left): population, day, average evolved genes + niche counts. Render-only.
#[derive(Component)]
struct WorldStatsText;

// Top-center day/night phase readout (walk mode). Tells you where in the cycle you are at a glance.
#[derive(Component)]
struct DayCycleText;

fn spawn_daycycle_ui(mut commands: Commands) {
    // full-width row, centered -> child text sits middle-top
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

// Set the phase label + color from local daylight at the walker (visual sky time). Hidden in orbit (there
// you see the whole terminator anyway). rising vs falling splits dawn/dusk + morning/afternoon.
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
    let day = crate::sphere::DAY_TICKS as i64;
    let vtick = (gen.tick as i64 + offset.0).rem_euclid(day) as u32;
    let d = crate::sphere::daylight_at(dir, vtick);
    let ahead = crate::sphere::daylight_at(dir, ((vtick as i64 + 30).rem_euclid(day)) as u32);
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

// --- legend overlay (H toggles a full panel explaining every HUD field + control) ---

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
  habitat     aquatic = swimmers, land = land-dwellers.
  specialists creatures locked to one food type.

HOW CREATURES LOOK
  hue         what they eat (diet specialization).
  vividness   how specialized (vivid) vs generalist (pale).
  cyan + flat  swimmers (fish-shaped body).
  size        bigger = more sensors / body size genes.

CONTROLS
  TAB         switch ORBIT (space) <-> WALK (ground)
  ORBIT: right-drag rotate, scroll/W,S zoom, A/D spin,
         Q/E tilt, left-click inspect, F follow
  WALK:  WASD move, arrows or right-drag look,
         Shift run (eye walks over the hills)
         [ / ]  scrub time-of-day   \\  jump to noon
         O  toggle shadows (on; off = always sunlit)
         (walk arrives at local noon; scrub [ ] for
          low sun + long shadows; night goes dark)
  G  sensor rays   SPACE  pause/resume
  1-5  speed presets (slow..fast)   + / -  fine speed
  B  seed creatures    P  populate whole planet
  L  lightning fire    K  cull    H  this legend
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

// H toggles the legend panel. Starts hidden; the top-left hint tells the player it exists.
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

// A unicode sparkline of a history series, scaled 0..max.
fn sparkline(hist: &[u16], max: f32) -> String {
    const B: [char; 8] = ['\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    hist.iter()
        .map(|&v| {
            let i = ((v as f32 / max.max(1.0)) * 7.0).round().clamp(0.0, 7.0) as usize;
            B[i]
        })
        .collect()
}

// Recompute the world dashboard each frame from the living population (cheap aggregate over creatures).
// Keeps a rolling population history (sampled ~1x/sec) and renders it as a sparkline -> a lightweight
// "population over time" chart (M7) right in the HUD.
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
    let (mut cold, mut warm, mut aq, mut land, mut spec) = (0u32, 0u32, 0u32, 0u32, 0u32);
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
        if g.swim > 0.6 { aq += 1; } else if g.swim < 0.3 { land += 1; }
        if g.rigidity > 0.6 { spec += 1; }
    }
    let nf = n.max(1) as f32;
    let day = gen.tick / crate::sphere::DAY_TICKS;
    // sample population ~once a second into a rolling history (~48 samples) for the trend sparkline
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
        "WORLD\nspeed      {speed}\npop        {n}\nday        {day}\ntrend      {trend}\ntemp avg   {:.2}  (cold {cold} / warm {warm})\nlongevity  {:.2}\nmetab      {:.2}\nr/K        {:.2}\nhabitat    aquatic {aq} / land {land}\nspecialists {spec}",
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

// Ray-sphere hit: nearest positive t along (origin + t*dir) intersecting the sphere, else None.
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

// Left-click picks the nearest creature/plant under the cursor (only when not in look mode).
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
        return; // in look mode (right-click captured): cursor not usable for picking
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
        // trees are big -> use a generous pick radius scaled by their size; plants are small
        let r = if tree.is_some() { 2.0 * t.compute_transform().scale.max_element() } else { 0.8 };
        consider(e, t.translation(), r, &mut best);
    }
    if let Some((_, e)) = best {
        selected.entity = Some(e);
    }
    // miss keeps the current selection (so follow isn't lost by a stray click)
}

// Draw a yellow ring around the selected entity each frame so you can see what's picked.
fn draw_selection(selected: Res<Selected>, q: Query<&GlobalTransform>, mut gizmos: Gizmos) {
    if let Some(e) = selected.entity {
        if let Ok(tf) = q.get(e) {
            // ring scales with the entity's size so it reads around big things (trees) too
            let r = 1.0 + 1.4 * tf.compute_transform().scale.max_element();
            gizmos.sphere(tf.translation(), r, Color::srgb(1.0, 1.0, 0.2));
        }
    }
}

// Update the on-screen panel with the selected entity's live stats (creature or plant/carrion).
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
        // dominant diet type (highest current expression)
        let mut dom = 0;
        for t in 1..NFOOD {
            if diet.expr[t] > diet.expr[dom] {
                dom = t;
            }
        }
        let mode = if g.light_pref > 0.6 { "diurnal" } else if g.light_pref < 0.4 { "nocturnal" } else { "cathemeral" };
        let habitat = if g.swim > 0.6 { "aquatic" } else if g.swim < 0.3 { "land" } else { "amphibious" };
        let clime = if g.temp_pref > 0.6 { "warm" } else if g.temp_pref < 0.4 { "cold" } else { "temperate" };
        text.0 = format!(
            "CREATURE  {}\nenergy   {:.1}\nfitness  {:.1}\nsensors  {}\nbite     {:.2}\nheight   {:.2}\nsize     {:.2}\nswim     {:.2} ({})\nsocial   {:.2}\ntemp     {:.2} ({})\nlongevity {:.2}\nmetab    {:.2}\nparental {:.2}\nrigidity {:.2}\nlight    {:.2} ({})\nfatigue  {:.2}\ndiet>type {} (eff {:.2})\nload(G)  {:.2}\nage      {}",
            if alive.0 { "alive" } else { "DEAD" },
            energy.0,
            fit.0,
            g.n_sensors(),
            g.bite,
            g.height,
            g.size,
            g.swim,
            habitat,
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
            diet.expr[dom],
            diet.g,
            diet.age,
        );
    } else if let Ok((pg, st, rot, tree)) = foods.get(e) {
        if let Some(tree) = tree {
            // creature height needed to feed = tree height - base margin - branch reach (branches hang fruit low)
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

// Draw each sensor as a ray from the creature along its (heading + angle), length = range.
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

    // noon_offset must put the sun (nearly) overhead the walk point -> daylight ~ |horizontal| of d.
    #[test]
    fn noon_offset_lights_the_walk_point() {
        for d in [
            Vec3::new(0.30, 0.50, 0.40),   // homeland
            Vec3::new(0.95, 0.30, -0.05),  // mountain A
            Vec3::new(-0.10, -0.30, 0.95), // deep ocean
            Vec3::new(-0.65, 0.20, -0.75), // mountain B
        ] {
            let d = d.normalize();
            for tick in [0u32, 600, 1234, 9_000_000] {
                let off = noon_offset(d, tick);
                let vtick = (tick as i64 + off).rem_euclid(crate::sphere::DAY_TICKS as i64) as u32;
                let day = crate::sphere::daylight_at(d, vtick);
                // sun overhead -> daylight ~= horizontal extent of d (sqrt(x^2+z^2)); must be clearly lit.
                let horiz = (d.x * d.x + d.z * d.z).sqrt();
                assert!(day > horiz - 0.05, "d={d:?} tick={tick} day={day} horiz={horiz}");
                assert!(day > 0.25, "walk point should be daylit at noon, got {day} for d={d:?}");
            }
        }
    }
}
