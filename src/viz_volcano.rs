//! Render-only: volcanic eruptions (climate::erupt) made visible. A glowing vent with its own orange light and
//! a billowing ash plume that rises, spreads and thins, sized by the eruption's VEI. Watches
//! PlanetClimate.eruptions for new events; V key asks climate_step for a VEI 6 (god control).
//! Real wall-clock time drives the animation so a plume billows smoothly at any sim speed.
use bevy::prelude::*;

pub struct VolcanoVizPlugin;

impl Plugin for VolcanoVizPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_volcano_viz).add_systems(Update, (key_erupt, spawn_eruptions, animate_plumes, animate_vents));
    }
}

#[derive(Resource)]
struct VolcanoAssets {
    puff: Handle<Mesh>,
    glow: Handle<Mesh>,
}

#[derive(Component)]
struct PlumePuff {
    age: f32,   // s; negative = not yet emitted (staggered column)
    life: f32,  // s
    up: Vec3,   // vent normal
    side: Vec3, // drift direction (prevailing wind bends the column)
    top: f32,   // world units the puff climbs over its life
    size: f32,  // final radius
    vent: Vec3, // vent surface point
    mat: Handle<StandardMaterial>,
}

#[derive(Component)]
struct VentGlow {
    age: f32,
    life: f32,
    mat: Handle<StandardMaterial>,
}

fn setup_volcano_viz(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(VolcanoAssets {
        puff: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
        glow: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
    });
}

fn key_erupt(keys: Res<ButtonInput<KeyCode>>, mut req: ResMut<crate::climate::EruptRequest>) {
    if keys.just_pressed(KeyCode::KeyV) {
        req.0 = Some(6);
        info!("god: volcano (VEI 6) on the next climate tick [V]");
    }
}

// deterministic per-puff jitter without touching the sim RNG
fn hash01(a: u32, b: u32) -> f32 {
    let h = a.wrapping_mul(2654435761) ^ b.wrapping_mul(40503).rotate_left(13);
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn spawn_eruptions(
    mut commands: Commands,
    clim: Res<crate::climate::PlanetClimate>,
    assets: Res<VolcanoAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut seen: Local<Option<u32>>,
) {
    let count = clim.eruptions;
    let last = seen.replace(count);
    // first frame (or a load): adopt the count; past eruptions are history, not events to draw
    let Some(prev) = last else { return };
    if count <= prev {
        return;
    }
    let Some(e) = clim.last_eruption else { return };
    let up = e.vent.normalize_or_zero();
    let vent = crate::sphere::surface_pos(up, 0.0);
    let k = e.vei.saturating_sub(4) as f32; // 0 = VEI 4 .. 4 = VEI 8
    let (east, north) = crate::sphere::tangent_frame(up);
    let wind = (east * (hash01(e.tick, 1) - 0.5) + north * (hash01(e.tick, 2) - 0.5)).normalize_or(east);
    // vent: emissive lava bulb + its own orange light (seen for kilometres at night)
    let glow_mat = mats.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.45, 0.1),
        emissive: LinearRgba::rgb(30.0, 9.0, 1.5),
        unlit: true,
        ..default()
    });
    commands
        .spawn((
            Mesh3d(assets.glow.clone()),
            MeshMaterial3d(glow_mat.clone()),
            Transform::from_translation(vent).with_scale(Vec3::splat(0.6 + 0.3 * k)),
            bevy::light::NotShadowCaster,
            VentGlow { age: 0.0, life: 40.0 + 15.0 * k, mat: glow_mat },
        ))
        .with_child((PointLight { color: Color::srgb(1.0, 0.5, 0.2), intensity: 4.0e6 * (1.0 + k), range: 40.0, ..default() }, Transform::from_translation(up * 1.5)));
    // plume: a staggered column of ash puffs; bigger eruptions climb higher, billow wider, last longer
    let n = 18 + 10 * k as usize;
    let top = 10.0 + 9.0 * k;
    for i in 0..n {
        let j = i as u32;
        let grey = 0.22 + 0.18 * hash01(e.tick ^ j, 3);
        let mat = mats.add(StandardMaterial {
            base_color: Color::srgba(grey, grey * 0.95, grey * 0.9, 0.0),
            emissive: LinearRgba::BLACK, // set per frame: lava underglow near the vent only (animate_plumes)
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        });
        commands.spawn((
            Mesh3d(assets.puff.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(vent).with_scale(Vec3::splat(0.01)),
            bevy::light::NotShadowCaster,
            PlumePuff {
                age: -(i as f32) * (0.35 + 0.2 * hash01(j, 4)),
                life: 22.0 + 10.0 * k + 6.0 * hash01(j, 5),
                up,
                side: wind,
                top: top * (0.7 + 0.5 * hash01(j, 6)),
                size: (2.0 + 1.2 * k) * (0.7 + 0.6 * hash01(j, 7)),
                vent,
                mat,
            },
        ));
    }
    info!("eruption VEI {} visuals: {} plume puffs", e.vei, n);
}

fn animate_plumes(
    mut commands: Commands,
    time: Res<Time>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut PlumePuff, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut p, mut tf) in &mut q {
        p.age += dt;
        if p.age < 0.0 {
            continue;
        }
        let t = (p.age / p.life).min(1.0);
        if t >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        // rise fast then stall at the plume top (buoyant column -> neutral-buoyancy umbrella), drifting downwind
        let rise = 1.0 - (1.0 - t).powi(3);
        let drift = t * t * p.top * 0.8;
        tf.translation = p.vent + p.up * (0.5 + rise * p.top) + p.side * drift;
        let r = p.size * (0.35 + 0.65 * t.sqrt()) * (1.0 + 0.6 * t); // billow + spread into the umbrella cloud
        tf.scale = Vec3::new(r * 1.2, r, r * 1.2);
        // fade in fast, linger, thin out
        let alpha = (t / 0.08).min(1.0) * (1.0 - t).powf(1.5) * 0.85;
        // underglow: the column's base is lit orange by the lava, the umbrella above is plain ash
        let glow = (1.0 - rise * 3.0).max(0.0).powi(2) * alpha;
        if let Some(mut m) = mats.get_mut(&p.mat) {
            m.base_color.set_alpha(alpha);
            m.emissive = LinearRgba::rgb(1.6 * glow, 0.45 * glow, 0.08 * glow);
        }
    }
}

fn animate_vents(
    mut commands: Commands,
    time: Res<Time>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut VentGlow, &mut Transform, &Children)>,
    mut lights: Query<&mut PointLight>,
) {
    let dt = time.delta_secs();
    for (e, mut g, mut tf, kids) in &mut q {
        g.age += dt;
        let t = g.age / g.life;
        if t >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        let fade = (1.0 - t).powi(2);
        let pulse = 0.8 + 0.2 * (g.age * 6.0).sin() * (g.age * 2.3).cos(); // lava sloshing
        let s = tf.scale.x.max(0.01);
        tf.scale = Vec3::splat(s); // keep size; brightness carries the pulse
        if let Some(mut m) = mats.get_mut(&g.mat) {
            m.emissive = LinearRgba::rgb(30.0 * fade * pulse, 9.0 * fade * pulse, 1.5 * fade);
        }
        for k in kids.iter() {
            if let Ok(mut l) = lights.get_mut(k) {
                l.intensity = 4.0e6 * fade * pulse;
            }
        }
    }
}
