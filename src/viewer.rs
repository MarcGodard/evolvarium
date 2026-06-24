// Creature viewer (--viewer[=genome.json]): load ONE creature onto an empty planet stage, orbit it up
// close, drag egui sliders to edit its trait genes (body + eyes rebuild live), then press T to seed a
// world around it and watch it live (release into sim). Inspection tool for marc + the agent.
//
// Wiring: main.rs render branch inserts ViewerMode (-> spawn_world_render skips seeding, sim.rs) + ViewerCfg,
// adds EguiPlugin + ViewerPlugin. Sim held PAUSED (Time<Virtual>) until release, so editing is frame-driven
// (Update/PostUpdate run while paused; only the FixedUpdate sim chain is frozen).

use crate::components::Creature;
use crate::genome::Genome;
use crate::viz::{EyeVis, Selected};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

// Presence = viewer mode. sim::spawn_world_render checks it to skip the population/plant seed.
#[derive(Resource)]
pub struct ViewerMode;

// --viewer=PATH: optional snapshot to load the inspected creature from (else a random genome).
#[derive(Resource)]
pub struct ViewerCfg {
    pub path: Option<String>,
}

#[derive(Resource)]
pub struct ViewerState {
    pub creature: Entity,
    pub released: bool, // T pressed -> world seeded, sim running, normal camera takes over
}

// Self-owned orbit-around-creature (close inspection). Reuses nothing from the planet orbit (tuned for
// dist 95..420 + mode-switch zoom), so the viewer drives its own yaw/pitch/dist here.
#[derive(Resource)]
pub struct ViewerCam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}
impl Default for ViewerCam {
    fn default() -> Self {
        ViewerCam { yaw: 0.6, pitch: 0.35, dist: 9.0 }
    }
}

pub struct ViewerPlugin;
impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewerCam>()
            .add_systems(Startup, spawn_viewer_creature)
            .add_systems(Update, (hold_pause, rebuild_on_edit, release_into_sim))
            .add_systems(EguiPrimaryContextPass, viewer_panel)
            // PostUpdate: run AFTER the planet camera systems so the close-orbit transform wins.
            .add_systems(PostUpdate, viewer_camera);
    }
}

// Spawn the single inspected creature at the homeland surface. Genome from --viewer=PATH (snapshot best) or
// random. Sets Selected.follow=true so the planet camera systems (orbit_drag/zoom/apply_orbit) all yield;
// viewer_camera owns the view.
fn spawn_viewer_creature(mut commands: Commands, mut rng: ResMut<crate::rng::Rng>, cfg: Res<ViewerCfg>, mut selected: ResMut<Selected>) {
    let g = cfg
        .path
        .as_deref()
        .and_then(crate::persist::load_snapshot)
        .and_then(|s| s.creatures.into_iter().next())
        .unwrap_or_else(|| Genome::random(&mut rng));
    let pos = crate::sphere::surface_pos(crate::sim::homeland_center(), crate::config::CREATURE_Y);
    let e = crate::sim::spawn_creature(&mut commands, g, pos, &mut rng, crate::config::BIRTH_ENERGY);
    selected.entity = Some(e);
    selected.follow = true; // disables planet orbit/zoom/apply_orbit (they early-return on follow)
    commands.insert_resource(ViewerState { creature: e, released: false });
    info!("viewer: inspecting 1 creature. Right-drag orbit, scroll zoom, sliders edit genes, T = release into sim.");
}

// Hold the sim paused while inspecting (before release). Update runs regardless of pause, so the creature
// still renders + edits apply.
fn hold_pause(state: Option<Res<ViewerState>>, mut vtime: ResMut<Time<Virtual>>) {
    let Some(state) = state else { return };
    if !state.released && !vtime.is_paused() {
        vtime.pause();
    }
}

// egui panel: a 0..1 slider per scalar trait gene. Editing writes back only on actual change (Slider.changed)
// -> fires Changed<Genome> just for that edit (drives restyle + rebuild_on_edit). Skips net/plast/body/
// sensors/uptake (not single-scalar; body-graph genes aren't hand-editable here).
fn viewer_panel(mut contexts: EguiContexts, state: Option<Res<ViewerState>>, mut q: Query<&mut Genome, With<Creature>>) {
    let Some(state) = state else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let Ok(mut g) = q.get_mut(state.creature) else { return };

    macro_rules! gene {
        ($ui:expr, $field:ident) => {{
            let mut v = g.$field;
            if $ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text(stringify!($field))).changed() {
                g.$field = v; // DerefMut -> Changed<Genome> only when the value actually moved
            }
        }};
    }

    egui::Window::new("Genome").default_width(260.0).show(ctx, |ui| {
        if state.released {
            ui.label("released into sim (editing still rebuilds this creature)");
        } else {
            ui.label("T = release into sim");
        }
        ui.separator();
        ui.label("body / size");
        gene!(ui, size);
        gene!(ui, height);
        gene!(ui, head);
        gene!(ui, eyes);
        gene!(ui, limbs);
        ui.separator();
        ui.label("locomotion / niche");
        gene!(ui, swim);
        gene!(ui, flight);
        gene!(ui, climb);
        gene!(ui, alpine);
        ui.separator();
        ui.label("defense / diet");
        gene!(ui, armor);
        gene!(ui, pelt);
        gene!(ui, venom);
        gene!(ui, bite);
        gene!(ui, carnivory);
        ui.separator();
        ui.label("look (cosmetic)");
        gene!(ui, skin_hue);
        gene!(ui, skin_sat);
        gene!(ui, pattern);
        gene!(ui, elongate);
        gene!(ui, tail);
        gene!(ui, fin);
        gene!(ui, beak);
    });
}

// On a gene edit: despawn the inspected creature's eye children + drop its Mesh3d so viz::add_creature_visuals
// re-dresses it next frame (new eye count/anchor/scale). Body mesh is cached by body_hash (scalar genes don't
// touch body), so this is cheap; color/scale refresh comes free from viz::restyle_creatures. Scoped to the
// inspected entity -> editing after release never strips eyes off the rest of the population.
fn rebuild_on_edit(
    mut commands: Commands,
    state: Option<Res<ViewerState>>,
    changed: Query<&Children, (With<Creature>, Changed<Genome>)>,
    eyes: Query<(), With<EyeVis>>,
) {
    let Some(state) = state else { return };
    let Ok(children) = changed.get(state.creature) else { return };
    for c in children.iter() {
        if eyes.get(c).is_ok() {
            commands.entity(c).despawn();
        }
    }
    commands.entity(state.creature).remove::<Mesh3d>();
}

// T: seed a full world (plants + trees + creatures, habitat-matched) around the inspected creature, clone its
// genome as the founder stock, unpause. Hands the camera back to the normal planet controls.
fn release_into_sim(
    keys: Res<ButtonInput<KeyCode>>,
    state: Option<ResMut<ViewerState>>,
    mut commands: Commands,
    mut rng: ResMut<crate::rng::Rng>,
    gen: Res<crate::sim::GenState>,
    genomes: Query<&Genome, With<Creature>>,
    mut vtime: ResMut<Time<Virtual>>,
    mut selected: ResMut<Selected>,
) {
    let Some(mut state) = state else { return };
    if state.released || !keys.just_pressed(KeyCode::KeyT) {
        return;
    }
    let parents: Vec<Genome> = genomes.get(state.creature).cloned().into_iter().collect();
    crate::sim::seed_planet(&mut commands, &mut rng, &parents, gen.ntypes(), 300, 600, 120);
    state.released = true;
    selected.follow = false; // planet camera resumes
    vtime.unpause();
    info!("viewer: released into sim. Normal camera: TAB orbit/walk, scroll zoom, F follow.");
}

// Close orbit around the inspected creature, in its LOCAL tangent frame (yaw spins around the creature's up
// axis, pitch lifts over it, up = radial so the horizon stays level). Right-drag = yaw/pitch, scroll = dist.
// Runs only before release; after that the normal planet camera owns the transform.
fn viewer_camera(
    state: Option<Res<ViewerState>>,
    mut cam: ResMut<ViewerCam>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    target: Query<&GlobalTransform, With<Creature>>,
    mut tf: Query<&mut Transform, With<crate::camera::OrbitCam>>,
) {
    let Some(state) = state else { return };
    if state.released {
        return;
    }
    if buttons.pressed(MouseButton::Right) {
        cam.yaw -= motion.delta.x * 0.006;
        cam.pitch = (cam.pitch + motion.delta.y * 0.006).clamp(-1.4, 1.4);
    }
    if scroll.delta.y != 0.0 {
        cam.dist = (cam.dist * (1.0 - scroll.delta.y * 0.12)).clamp(2.5, 60.0);
    }
    let Ok(focus) = target.get(state.creature).map(|g| g.translation()) else { return };
    let Ok(mut t) = tf.single_mut() else { return };
    // local tangent basis around the creature's radial up
    let up = focus.normalize_or_zero();
    let t1 = up.cross(Vec3::X).normalize_or_zero();
    let t1 = if t1.length_squared() < 1e-4 { up.cross(Vec3::Z).normalize_or_zero() } else { t1 };
    let t2 = up.cross(t1).normalize_or_zero();
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let dir = cp * (cy * t1 + sy * t2) + sp * up;
    t.translation = focus + dir.normalize_or_zero() * cam.dist;
    t.look_at(focus, up);
}
