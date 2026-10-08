//! Render performance probe: logs real frame time (mean / p95 / worst) every LOG_EVERY_S with the counts that
//! drive draw cost (entities with meshes, distinct materials in use). Always on in the windowed app; costs one
//! Vec push per frame. Read it from `--capture` logs to compare graphics changes with numbers, not eyeballs.
use bevy::prelude::*;

const LOG_EVERY_S: f32 = 10.0;

pub struct PerfHudPlugin;

impl Plugin for PerfHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, frame_probe);
    }
}

#[derive(Default)]
struct Probe {
    frames: Vec<f32>, // ms
    since: f32,
    skip: u32, // first frames include shader compiles + asset uploads: not representative
}

fn frame_probe(
    time: Res<Time<Real>>,
    mut p: Local<Probe>,
    meshes: Query<(), With<Mesh3d>>,
    mats: Res<Assets<StandardMaterial>>,
    kinds: Query<(Has<crate::components::Grass>, Has<crate::components::Seaweed>, Has<crate::components::Food>, Has<crate::components::Creature>)>,
) {
    if p.skip < 30 {
        p.skip += 1;
        return;
    }
    let dt = time.delta_secs();
    p.frames.push(dt * 1000.0);
    p.since += dt;
    if p.since < LOG_EVERY_S {
        return;
    }
    let mut v = std::mem::take(&mut p.frames);
    p.since = 0.0;
    v.sort_by(f32::total_cmp);
    let n = v.len().max(1);
    let mean = v.iter().sum::<f32>() / n as f32;
    let p95 = v[((n as f32 * 0.95) as usize).min(n - 1)];
    let worst = *v.last().unwrap_or(&0.0);
    let mut k = [0usize; 4]; // grass, seaweed, other food/plants, creatures (roots only; their parts are extra meshes)
    for (g, w, f, c) in &kinds {
        let i = if g { 0 } else if w { 1 } else if f { 2 } else if c { 3 } else { continue };
        k[i] += 1;
    }
    info!(
        "perf: frame mean {mean:.2} ms ({:.0} fps) p95 {p95:.2} worst {worst:.2} | mesh entities {} | materials {} | grass {} seaweed {} plants {} creatures {}",
        1000.0 / mean.max(1e-3),
        meshes.iter().count(),
        mats.len(),
        k[0],
        k[1],
        k[2],
        k[3]
    );
}
