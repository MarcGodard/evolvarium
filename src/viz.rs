// Render-only visualization: make evolved variability visible.
//   - creature COLOR = dominant diet specialization (hue) + rigidity (saturation: specialist=vivid)
//   - creature SIZE  = sensor count (more eyes = bigger/more complex body)
//   - press G        = draw each creature's sensor rays (the evolved eye layout)
// All cosmetic; never touches sim state.
use bevy::prelude::*;

use crate::components::{Alive, Creature, Food, Heading, Rot};
use crate::genome::{Genome, NFOOD};
use crate::plant::{plant_color, PlantGenome, PlantState};
use crate::sim::ROT_GONE;

pub struct VizPlugin;

impl Plugin for VizPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowSensors>()
            .add_systems(Startup, log_viz_help)
            .add_systems(
                Update,
                (restyle_creatures, toggle_sensors, draw_sensors, add_plant_visuals, size_plants, hide_dead, color_carrion),
            );
    }
}

// Shared sphere mesh for plants (inserted by spawn_world_render).
#[derive(Resource)]
pub struct PlantMesh(pub Handle<Mesh>);

// Give any plant lacking a mesh its visuals: shared sphere + a material colored by its genome
// (hue=kind, brightness=nutrient, warmth=defense). Covers initial plants AND new offspring.
fn add_plant_visuals(
    mut commands: Commands,
    mesh: Option<Res<PlantMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<(Entity, &PlantGenome), (With<Food>, Without<Mesh3d>)>,
) {
    let Some(mesh) = mesh else { return };
    for (e, g) in &q {
        commands
            .entity(e)
            .insert((Mesh3d(mesh.0.clone()), MeshMaterial3d(materials.add(plant_color(g)))));
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

// Scale plants by accumulated mass so growth is visible.
fn size_plants(mut q: Query<(&PlantState, &mut Transform), With<Food>>) {
    for (st, mut tf) in &mut q {
        tf.scale = Vec3::splat((0.25 + 0.13 * st.mass).clamp(0.25, 1.6));
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
        // body scale reflects sensor count
        tf.scale = Vec3::splat(0.8 + 0.07 * g.n_sensors() as f32);
    }
}

fn toggle_sensors(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowSensors>) {
    if keys.just_pressed(KeyCode::KeyG) {
        show.0 = !show.0;
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
