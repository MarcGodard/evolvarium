// Render-only visualization: make evolved variability visible.
//   - creature COLOR = dominant diet specialization (hue) + rigidity (saturation: specialist=vivid)
//   - creature SIZE  = sensor count (more eyes = bigger/more complex body)
//   - press G        = draw each creature's sensor rays (the evolved eye layout)
// All cosmetic; never touches sim state.
use bevy::prelude::*;

use crate::components::{Alive, Creature, DietState, Energy, Fitness, Food, Heading, Rot, Tree};
use crate::genome::{Genome, NFOOD};
use crate::plant::{plant_color, PlantGenome, PlantState};
use crate::sim::{grid_cell_surface, Fire, GenState, ROT_GONE};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

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
            .add_systems(Startup, (log_viz_help, spawn_stats_ui, spawn_world_stats_ui, spawn_clouds))
            .add_systems(
                Update,
                (
                    restyle_creatures,
                    toggle_sensors,
                    draw_sensors,
                    add_plant_visuals,
                    size_plants,
                    day_night_lighting,
                    rain_visuals,
                    fire_visuals,
                    update_clouds,
                    hide_dead,
                    color_carrion,
                    pick_on_click,
                    update_stats,
                    update_world_stats,
                    time_controls,
                    draw_selection,
                ),
            );
    }
}

// Shared sphere mesh for plants (inserted by spawn_world_render).
#[derive(Resource)]
pub struct PlantMesh(pub Handle<Mesh>);

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
    mut suns: Query<&mut Transform, (With<SunLight>, Without<Moon>, Without<SunDisc>)>,
    mut moons: Query<&mut Transform, (With<Moon>, Without<SunLight>, Without<SunDisc>)>,
    mut discs: Query<&mut Transform, (With<SunDisc>, Without<SunLight>, Without<Moon>)>,
) {
    let sd = crate::sphere::sun_dir(gen.tick);
    for mut tf in &mut suns {
        // place the light source out along the sun direction, aimed at the planet center
        *tf = Transform::from_translation(sd * (crate::sphere::PLANET_R * 4.0)).looking_at(Vec3::ZERO, Vec3::Y);
    }
    for mut tf in &mut discs {
        tf.translation = sd * crate::sphere::SUN_DIST; // the visible sun rides the same direction, far out
    }
    let mp = crate::sphere::moon_pos(gen.tick);
    for mut tf in &mut moons {
        tf.translation = mp;
    }
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
    info!("viz: creature hue=diet specialization, vividness=rigidity, size=sensor count | G=sensor rays | SPACE=pause | +/-=sim speed");
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
        // dominant diet specialization -> hue
        let mut dom = 0;
        let mut best = g.expr0[0];
        for t in 1..NFOOD {
            if g.expr0[t] > best {
                best = g.expr0[t];
                dom = t;
            }
        }
        // rigidity -> saturation: pinned specialist = vivid, flexible generalist = washed out.
        // swim -> hue shifts toward cyan/blue (aquatic look); diet hue otherwise.
        let sat = 0.2 + 0.7 * g.rigidity;
        let hue = type_hue(dom) * (1.0 - g.swim) + 200.0 * g.swim;
        if let Some(m) = mats.get_mut(&mm.0) {
            m.base_color = Color::hsl(hue, sat, 0.55);
        }
        // body plan: girth from sensors, overall bulk from size, vertical stretch from height; swim
        // flattens + elongates the body into a fish shape (longer along travel, lower + narrower).
        let girth = (0.7 + 0.06 * g.n_sensors() as f32) * (0.6 + 0.9 * g.size);
        let sx = girth * (1.0 - 0.25 * g.swim);
        let sy = girth * (0.7 + 1.6 * g.height) * (1.0 - 0.3 * g.swim);
        let sz = girth * (1.0 + 0.8 * g.swim);
        tf.scale = Vec3::new(sx, sy, sz);
    }
}

fn toggle_sensors(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowSensors>) {
    if keys.just_pressed(KeyCode::KeyG) {
        show.0 = !show.0;
    }
}

// God-controls: SPACE pauses/resumes the sim, +/- fast-forward / slow down. Drives Bevy's virtual clock,
// which FixedUpdate advances from -> pausing/speeding it pauses/speeds the whole simulation, no sim change.
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
    let cur = vtime.relative_speed();
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        let s = (cur * 2.0).min(16.0);
        vtime.set_relative_speed(s);
        info!("sim speed {s:.2}x");
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        let s = (cur * 0.5).max(0.25);
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

// Recompute the world dashboard each frame from the living population (cheap aggregate over creatures).
fn update_world_stats(
    gen: Res<GenState>,
    creatures: Query<(&Genome, &Alive), With<Creature>>,
    mut text: Query<&mut Text, With<WorldStatsText>>,
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
    t.0 = format!(
        "WORLD\npop        {n}\nday        {day}\ntemp avg   {:.2}  (cold {cold} / warm {warm})\nlongevity  {:.2}\nmetab      {:.2}\nr/K        {:.2}\nhabitat    aquatic {aq} / land {land}\nspecialists {spec}",
        temp / nf, lng / nf, met / nf, par / nf
    );
}

fn spawn_stats_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("left-click a creature or plant to inspect"),
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
            if best.map_or(true, |(bt, _)| t < bt) {
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
