// Creature viewer (--viewer[=genome.json]): load ONE creature onto an empty planet stage, orbit it up
// close, drag egui sliders to edit its trait genes (body + eyes rebuild live), then press T to seed a
// world around it and watch it live (release into sim). Inspection tool for marc + the agent.
//
// Wiring: main.rs render branch inserts ViewerMode (-> spawn_world_render skips seeding, sim.rs) + ViewerCfg,
// adds EguiPlugin + ViewerPlugin. Sim held PAUSED (Time<Virtual>) until release, so editing is frame-driven
// (Update/PostUpdate run while paused; only the FixedUpdate sim chain is frozen).

use crate::components::Creature;
use crate::genome::Genome;
use crate::morph::ShapeKind;
use crate::viz::{BodyMeshCache, CreatureParts, EyeVis, Selected};
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

// Set by viewer_panel (which owns the egui ctx), read by viewer_camera -> mouse over the panel never drives
// the world camera. Kept as a resource so only ONE system touches EguiContexts (accessing egui outside the
// EguiPrimaryContextPass in multipass mode stops the panel rendering).
#[derive(Resource, Default)]
pub struct PointerOverUi(pub bool);

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
            .init_resource::<PointerOverUi>()
            .add_systems(Startup, spawn_viewer_creature)
            .add_systems(Update, (hold_pause, rebuild_on_edit, release_into_sim, bind_egui_to_main))
            .add_systems(EguiPrimaryContextPass, viewer_panel)
            // PostUpdate: run AFTER the planet camera systems so the close-orbit transform wins.
            .add_systems(PostUpdate, viewer_camera);
    }
}

// Pin the PRIMARY egui context to the MAIN camera (full window). With auto_create_primary_context off
// (main.rs), nothing else claims it, so the minimap's small 2nd camera can't hijack the panel rect. Runs each
// frame until the main camera has it, then the Without<> filter makes it a no-op.
fn bind_egui_to_main(mut commands: Commands, cam: Query<Entity, (With<crate::camera::OrbitCam>, Without<bevy_egui::PrimaryEguiContext>)>) {
    for e in &cam {
        commands.entity(e).insert((bevy_egui::EguiContext::default(), bevy_egui::PrimaryEguiContext));
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

// Full-height LEFT side panel: every editable gene, grouped + scrollable. Writes back only on actual change
// (Slider/Checkbox .changed()) -> fires Changed<Genome> just for that edit (drives restyle + rebuild_on_edit).
// Covers all scalar trait genes, the 10 uptake nutrients, the vision sensors, and the body-graph shape
// (per-node primitive/size, per-edge placement/recursion). Skips only the NN weights (net/plast).
fn viewer_panel(mut contexts: EguiContexts, state: Option<Res<ViewerState>>, mut over_ui: ResMut<PointerOverUi>, mut q: Query<&mut Genome, With<Creature>>) {
    let Some(state) = state else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    over_ui.0 = ctx.is_pointer_over_area() || ctx.wants_pointer_input(); // gate viewer_camera so UI scroll/drag stays in the panel
    let Ok(mut g) = q.get_mut(state.creature) else { return };
    let g = &mut g; // Mut<Genome>; macro/checkbox write through it ONLY on real change -> Changed<Genome> stays precise (no per-frame rebuild)
    let pi = std::f32::consts::PI;

    // slider over any f32 lvalue ($place): read a copy, write back ONLY when egui reports the value changed.
    macro_rules! sl {
        ($ui:expr, $label:expr, $lo:expr, $hi:expr, $place:expr) => {{
            let mut v = $place;
            if $ui.add(egui::Slider::new(&mut v, $lo..=$hi).text($label)).changed() {
                $place = v;
            }
        }};
    }

    egui::SidePanel::left("genome_panel").resizable(true).default_width(360.0).width_range(320.0..=680.0).show(ctx, |ui| {
        ui.set_min_height(ui.available_height()); // fill full window height, not shrink-wrap to content
        ui.heading("Genome");
        ui.label(if state.released { "released into sim (edits still rebuild this creature)" } else { "T = release into sim" });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // SHAPE comes from the generative body graph, not these scalars -> reshape via "Body graph (shape)".
            egui::CollapsingHeader::new("Appearance").default_open(true).show(ui, |ui| {
                sl!(ui, "size (whole-body scale)", 0.0, 1.0, g.size);
                sl!(ui, "eyes (count 1..6)", 0.0, 1.0, g.eyes);
                sl!(ui, "head (eye size)", 0.0, 1.0, g.head);
                sl!(ui, "skin_hue", 0.0, 1.0, g.skin_hue);
                sl!(ui, "skin_sat", 0.0, 1.0, g.skin_sat);
                ui.label("↓ to reshape the BODY, use 'Body graph (shape)' below");
            });
            egui::CollapsingHeader::new("Traits (stats, no mesh change)").show(ui, |ui| {
                sl!(ui, "height (reach)", 0.0, 1.0, g.height);
                sl!(ui, "limbs (leg count)", 0.0, 1.0, g.limbs);
            });
            egui::CollapsingHeader::new("Locomotion & niche").show(ui, |ui| {
                sl!(ui, "swim", 0.0, 1.0, g.swim);
                sl!(ui, "flight", 0.0, 1.0, g.flight);
                sl!(ui, "climb", 0.0, 1.0, g.climb);
                sl!(ui, "alpine", 0.0, 1.0, g.alpine);
                sl!(ui, "light_pref", 0.0, 1.0, g.light_pref);
                sl!(ui, "temp_pref", 0.0, 1.0, g.temp_pref);
            });
            egui::CollapsingHeader::new("Defense").show(ui, |ui| {
                sl!(ui, "armor", 0.0, 1.0, g.armor);
                sl!(ui, "pelt", 0.0, 1.0, g.pelt);
                sl!(ui, "venom", 0.0, 1.0, g.venom);
            });
            egui::CollapsingHeader::new("Diet & metabolism").show(ui, |ui| {
                sl!(ui, "bite", 0.0, 1.0, g.bite);
                sl!(ui, "carnivory", 0.0, 1.0, g.carnivory);
                sl!(ui, "detox", 0.0, 1.0, g.detox);
                sl!(ui, "rigidity", 0.0, 1.0, g.rigidity);
                sl!(ui, "metab", 0.0, 1.0, g.metab);
                sl!(ui, "adiposity", 0.0, 1.0, g.adiposity);
                sl!(ui, "longevity", 0.0, 1.0, g.longevity);
                sl!(ui, "parental", 0.0, 1.0, g.parental);
            });
            egui::CollapsingHeader::new("Behavior & senses").show(ui, |ui| {
                sl!(ui, "social", 0.0, 1.0, g.social);
                sl!(ui, "magneto", 0.0, 1.0, g.magneto);
            });
            egui::CollapsingHeader::new("Look (cosmetic, legacy)").show(ui, |ui| {
                sl!(ui, "pattern", 0.0, 1.0, g.pattern);
                sl!(ui, "elongate", 0.0, 1.0, g.elongate);
                sl!(ui, "tail", 0.0, 1.0, g.tail);
                sl!(ui, "fin", 0.0, 1.0, g.fin);
                sl!(ui, "beak", 0.0, 1.0, g.beak);
            });
            egui::CollapsingHeader::new("Uptake (nutrient affinities)").show(ui, |ui| {
                for i in 0..crate::genome::NUTRIENTS {
                    sl!(ui, format!("nutrient {i}"), 0.0, 2.0, g.uptake[i]);
                }
            });
            egui::CollapsingHeader::new("Sensors (vision)").show(ui, |ui| {
                let mut remove = None;
                for i in 0..g.sensors.len() {
                    ui.horizontal(|ui| {
                        ui.label(format!("#{i}"));
                        if g.sensors.len() > 1 && ui.small_button("remove").clicked() {
                            remove = Some(i);
                        }
                    });
                    sl!(ui, "angle", -pi, pi, g.sensors[i].angle);
                    sl!(ui, "range", 0.0, 40.0, g.sensors[i].range);
                }
                if let Some(i) = remove {
                    g.sensors.remove(i);
                }
                if ui.button("+ add sensor").clicked() {
                    g.sensors.push(crate::genome::Sensor { angle: 0.0, range: 15.0 });
                }
            });
            egui::CollapsingHeader::new("Body graph (shape)").show(ui, |ui| {
                ui.label("nodes (body parts)");
                for i in 0..g.body.nodes.len() {
                    ui.separator();
                    let mut shape = g.body.nodes[i].shape;
                    egui::ComboBox::from_id_salt(("node_shape", i)).selected_text(format!("node {i}: {shape:?}")).show_ui(ui, |ui| {
                        ui.selectable_value(&mut shape, ShapeKind::Segment, "Segment");
                        ui.selectable_value(&mut shape, ShapeKind::Sphere, "Sphere");
                        ui.selectable_value(&mut shape, ShapeKind::Plate, "Plate");
                    });
                    if shape != g.body.nodes[i].shape {
                        g.body.nodes[i].shape = shape;
                    }
                    sl!(ui, "length", 0.15, 3.0, g.body.nodes[i].length);
                    sl!(ui, "radius", 0.05, 1.2, g.body.nodes[i].radius);
                    sl!(ui, "taper", 0.05, 1.0, g.body.nodes[i].taper);
                    sl!(ui, "shade r", 0.0, 1.0, g.body.nodes[i].r);
                    sl!(ui, "shade g", 0.0, 1.0, g.body.nodes[i].g);
                    sl!(ui, "shade b", 0.0, 1.0, g.body.nodes[i].b);
                }
                ui.separator();
                ui.label("edges (how parts attach)");
                for i in 0..g.body.edges.len() {
                    ui.separator();
                    ui.label(format!("edge {i}: node {} -> node {}", g.body.edges[i].from, g.body.edges[i].to));
                    sl!(ui, "along (up parent)", 0.0, 1.0, g.body.edges[i].along);
                    sl!(ui, "around (side)", -pi, pi, g.body.edges[i].around);
                    sl!(ui, "pitch (tilt)", -1.5, 1.5, g.body.edges[i].pitch);
                    sl!(ui, "roll", -pi, pi, g.body.edges[i].roll);
                    sl!(ui, "scale (per recursion)", 0.2, 1.0, g.body.edges[i].scale);
                    let mut refl = g.body.edges[i].reflect;
                    if ui.checkbox(&mut refl, "reflect (mirror to other side)").changed() {
                        g.body.edges[i].reflect = refl;
                    }
                    let mut rec = g.body.edges[i].recurse as i32;
                    if ui.add(egui::Slider::new(&mut rec, 0..=5).text("recurse (chain length)")).changed() {
                        g.body.edges[i].recurse = rec as u8;
                    }
                }
            });
        });
    });
}

// On a gene edit, rebuild the inspected creature IN PLACE (no invisible frame -> no flash/"reset"): swap the
// body Mesh3d (BodyMeshCache keys on body_hash, so body-graph edits build a fresh mesh; scalar edits reuse
// the cached one) and respawn the eye children from the current genome. Color/scale refresh comes free from
// viz::restyle_creatures (also Changed<Genome>). With<Mesh3d> filter skips the spawn frame (initial dress is
// add_creature_visuals' job -> no double eyes). Scoped to the inspected entity, so editing after release
// never touches the rest of the population.
fn rebuild_on_edit(
    mut commands: Commands,
    state: Option<Res<ViewerState>>,
    parts: Option<Res<CreatureParts>>,
    mut cache: ResMut<BodyMeshCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    changed: Query<(&Genome, &Children), (With<Creature>, With<Mesh3d>, Changed<Genome>)>,
    eyes: Query<(), With<EyeVis>>,
) {
    let (Some(state), Some(parts)) = (state, parts) else { return };
    let Ok((g, children)) = changed.get(state.creature) else { return };
    let mesh = cache.get_or_build(g, &mut meshes);
    commands.entity(state.creature).insert(Mesh3d(mesh)); // overwrite in place
    for c in children.iter() {
        if eyes.get(c).is_ok() {
            commands.entity(c).despawn();
        }
    }
    crate::viz::spawn_eyes(&mut commands, state.creature, g, &parts.eye, &mut materials);
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
    over_ui: Res<PointerOverUi>,
    target: Query<&GlobalTransform, With<Creature>>,
    mut tf: Query<&mut Transform, With<crate::camera::OrbitCam>>,
) {
    let Some(state) = state else { return };
    if state.released {
        return;
    }
    // skip camera input while the pointer is over the panel (set by viewer_panel) -> UI scroll/drag never moves
    // the world.
    if !over_ui.0 {
        if buttons.pressed(MouseButton::Right) {
            cam.yaw -= motion.delta.x * 0.006;
            cam.pitch = (cam.pitch + motion.delta.y * 0.006).clamp(-1.4, 1.4);
        }
        if scroll.delta.y != 0.0 {
            cam.dist = (cam.dist * (1.0 - scroll.delta.y * 0.12)).clamp(2.5, 60.0);
        }
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
