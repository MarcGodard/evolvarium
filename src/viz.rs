// Render-only visualization: make evolved variability visible.
//   - creature COLOR = dominant diet specialization (hue) + rigidity (saturation: specialist=vivid)
//   - creature SIZE  = sensor count (more eyes = bigger/more complex body)
//   - press G        = draw each creature's sensor rays (the evolved eye layout)
// All cosmetic; never touches sim state.
use bevy::prelude::*;

use crate::components::{Alive, Creature, DietState, Energy, Fitness, Food, Heading, Rot, Tree};
use crate::genome::{Genome, NFOOD};
use crate::plant::{plant_color, PlantGenome, PlantState};
use crate::sim::ROT_GONE;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

pub struct VizPlugin;

impl Plugin for VizPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowSensors>()
            .init_resource::<Selected>()
            .add_systems(Startup, (log_viz_help, spawn_stats_ui))
            .add_systems(
                Update,
                (
                    restyle_creatures,
                    toggle_sensors,
                    draw_sensors,
                    add_plant_visuals,
                    size_plants,
                    hide_dead,
                    color_carrion,
                    pick_on_click,
                    update_stats,
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

// Scale plants by accumulated mass so growth is visible. Trees render much bigger (tall canopy).
fn size_plants(mut q: Query<(&PlantState, &mut Transform, Option<&Tree>), With<Food>>) {
    for (st, mut tf, tree) in &mut q {
        tf.scale = if tree.is_some() {
            Vec3::splat((0.9 + 0.28 * st.mass).clamp(0.9, 4.5))
        } else {
            Vec3::splat((0.25 + 0.13 * st.mass).clamp(0.25, 1.6))
        };
    }
}

#[derive(Resource, Default)]
pub struct ShowSensors(pub bool);

fn log_viz_help() {
    info!("viz: creature hue=diet specialization, vividness=rigidity, size=sensor count | press G = show sensor rays");
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
        // rigidity -> saturation: pinned specialist = vivid, flexible generalist = washed out
        let sat = 0.2 + 0.7 * g.rigidity;
        if let Some(m) = mats.get_mut(&mm.0) {
            m.base_color = Color::hsl(type_hue(dom), sat, 0.55);
        }
        // body scale: girth from sensor count, vertical stretch from the height gene (tall reaches trees)
        let girth = 0.7 + 0.06 * g.n_sensors() as f32;
        tf.scale = Vec3::new(girth, girth * (0.7 + 1.6 * g.height), girth);
    }
}

fn toggle_sensors(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowSensors>) {
    if keys.just_pressed(KeyCode::KeyG) {
        show.0 = !show.0;
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
        text.0 = format!(
            "CREATURE  {}\nenergy   {:.1}\nfitness  {:.1}\nsensors  {}\nbite     {:.2}\nheight   {:.2}\nrigidity {:.2}\ndiet>type {} (eff {:.2})\nload(G)  {:.2}\nage      {}",
            if alive.0 { "alive" } else { "DEAD" },
            energy.0,
            fit.0,
            g.n_sensors(),
            g.bite,
            g.height,
            g.rigidity,
            dom,
            diet.expr[dom],
            diet.g,
            diet.age,
        );
    } else if let Ok((pg, st, rot, tree)) = foods.get(e) {
        if let Some(tree) = tree {
            text.0 = format!(
                "TREE  {}\nmass     {:.1}\nnutrient {:.2}\n{}",
                if tree.edible { "fruit (tall creatures eat)" } else { "evergreen (uneatable)" },
                st.mass,
                pg.nutrient,
                if tree.edible { "reach: needs height >= 0.6" } else { "pure structure / refuge" },
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
