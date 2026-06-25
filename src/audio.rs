// Procedural world audio (render-only). NO sound assets in repo -> synthesize PCM in-memory -> WAV bytes ->
// AudioSource (rodio decodes). Three layers:
//   - ambient beds (wind/rain/surf/fire): looping noise, per-frame volume from camera-local sim state.
//   - creature calls: NN-driven (Brain.voice) spatial one-shot tones, pitch = emitter size (big=low, small=high).
//   - thunder: one-shot crack on a lightning strike near the camera.
// Only added in the render branch (main.rs) -> never runs headless. Listener = the camera (SpatialListener).
use crate::components::Creature;
use crate::genome::Genome;
use crate::sim::{Fire, GenState};
use bevy::audio::{AudioPlayer, AudioSink, AudioSource, PlaybackSettings, SpatialListener, Volume};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

const SR: u32 = 22050; // sample rate (low = small buffers; ambient/calls don't need hi-fi)
const BED_SECS: f32 = 3.0;
const CALL_PITCHES: usize = 6; // discrete pre-baked call-tone pitch bins
const CALL_RADIUS: f32 = 70.0; // camera earshot for creature calls (world units)
const CALL_COOLDOWN: u32 = 26; // ticks between a creature's audible chirps (~0.4s) -> no per-frame retrigger spam
const CALL_MAX_PER_FRAME: usize = 4; // bound new call sounds/frame (2000 creatures -> no cacophony)
const THUNDER_RADIUS: f32 = 600.0; // camera distance within which a strike is audible

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CallLog>()
            .add_systems(Startup, setup_audio)
            .add_systems(Update, (attach_listener, ambient_mix, creature_calls, thunder_audio));
    }
}

// --- synthesis ---------------------------------------------------------------

// Tiny LCG for synth noise (NOT sim rng; render-only, determinism irrelevant). Returns -1..1.
struct Lcg(u32);
impl Lcg {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        ((self.0 >> 8) as f32 / 16_777_215.0) * 2.0 - 1.0
    }
}

// 16-bit mono PCM -> WAV byte buffer (rodio decodes this).
fn wav_bytes(samples: &[f32]) -> Arc<[u8]> {
    let data_len = samples.len() * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits/sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for &s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b.into()
}

// Crossfade the tail back onto the head so a noise loop has no seam click. Drops `fade` samples.
fn loopify(mut s: Vec<f32>, fade: usize) -> Vec<f32> {
    let n = s.len();
    if n <= fade * 2 {
        return s;
    }
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        s[i] = s[i] * t + s[n - fade + i] * (1.0 - t);
    }
    s.truncate(n - fade);
    s
}

// One-pole low-pass smoothed noise bed. `cut` 0..1 = brightness (low = dark rumble, high = hiss). `crackle` adds
// random pops (fire). Seeded distinct per bed so they don't phase-align.
fn noise_bed(seed: u32, cut: f32, crackle: f32) -> Arc<[u8]> {
    let n = (SR as f32 * BED_SECS) as usize;
    let mut rng = Lcg(seed);
    let mut lp = 0.0f32;
    let a = cut.clamp(0.02, 1.0);
    let mut s = Vec::with_capacity(n);
    for _ in 0..n {
        let white = rng.f();
        lp += a * (white - lp);
        let mut v = lp;
        if crackle > 0.0 && rng.f() > 1.0 - crackle * 0.02 {
            v += rng.f() * 0.8; // spark pop
        }
        s.push(v);
    }
    // normalize toward ~0.6 peak
    let peak = s.iter().fold(1e-4f32, |m, v| m.max(v.abs()));
    let g = 0.6 / peak;
    for v in &mut s {
        *v *= g;
    }
    wav_bytes(&loopify(s, SR as usize / 12))
}

// Underwater ambience: deep muffled rumble + sparse rising bubble blips. Distinct from surf -> "everything
// sounds different down here". Loops.
fn sub_bed() -> Arc<[u8]> {
    let n = (SR as f32 * BED_SECS) as usize;
    let mut rng = Lcg(5);
    let mut lp = 0.0f32;
    let mut s = vec![0.0f32; n];
    for v in s.iter_mut() {
        lp += 0.02 * (rng.f() - lp); // very dark muffled rumble
        *v = lp * 2.0;
    }
    let mut i = 0usize; // sparse bubble blips: short rising sine pops
    loop {
        i += (SR as f32 * (0.25 + rng.f().abs() * 0.6)) as usize;
        if i >= n {
            break;
        }
        let f0 = 300.0 + rng.f().abs() * 500.0;
        let blen = ((SR as f32 * 0.08) as usize).min(n - i);
        for k in 0..blen {
            let t = k as f32 / SR as f32;
            let env = (-t * 30.0).exp();
            s[i + k] += (t * f0 * (1.0 + t * 6.0) * std::f32::consts::TAU).sin() * env * 0.25; // rising chirp
        }
    }
    let peak = s.iter().fold(1e-4f32, |m, v| m.max(v.abs()));
    let g = 0.55 / peak;
    for v in &mut s {
        *v *= g;
    }
    wav_bytes(&loopify(s, SR as usize / 12))
}

// Decaying call tone: sine at `freq` + a fifth, exponential decay + vibrato. ~0.3s one-shot.
fn call_tone(freq: f32) -> Arc<[u8]> {
    let n = (SR as f32 * 0.3) as usize;
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR as f32;
        let env = (-t * 9.0).exp(); // pluck decay
        let vib = 1.0 + 0.02 * (t * 38.0 * std::f32::consts::TAU).sin();
        let w = (t * freq * vib * std::f32::consts::TAU).sin() * 0.7 + (t * freq * 1.5 * std::f32::consts::TAU).sin() * 0.3;
        s.push(w * env * 0.7);
    }
    wav_bytes(&s)
}

// Thunder: low rumble (dark noise) with a sharp attack + long decay.
fn thunder() -> Arc<[u8]> {
    let n = (SR as f32 * 1.6) as usize;
    let mut rng = Lcg(0xBEEF);
    let mut lp = 0.0f32;
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR as f32;
        lp += 0.05 * (rng.f() - lp); // very dark
        let attack = (t * 60.0).min(1.0); // fast ramp in
        let decay = (-t * 2.2).exp();
        s.push(lp * 3.0 * attack * decay);
    }
    let peak = s.iter().fold(1e-4f32, |m, v| m.max(v.abs()));
    let g = 0.85 / peak;
    for v in &mut s {
        *v *= g;
    }
    wav_bytes(&s)
}

// --- resources + components --------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Bed {
    Wind,
    Rain,
    Surf,
    Fire,
    Sub, // underwater: muffled deep hum + bubbles (replaces ALL land beds when the listener is submerged)
}

#[derive(Component)]
struct AmbientBed(Bed);

#[derive(Resource)]
struct AudioAssets {
    calls: Vec<Handle<AudioSource>>, // by pitch bin
    thunder: Handle<AudioSource>,
}

// Per-creature last audible-call tick -> throttle so a creature holding voice>0 chirps, not screams every frame.
#[derive(Resource, Default)]
struct CallLog {
    last: HashMap<Entity, u32>,
    last_strike_tick: u32,
}

// --- systems -----------------------------------------------------------------

fn setup_audio(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    // looping ambient beds (start silent; ambient_mix raises volume by environment). Non-spatial = global bed.
    let beds = [
        (Bed::Wind, noise_bed(1, 0.06, 0.0)),
        (Bed::Rain, noise_bed(2, 0.5, 0.0)),
        (Bed::Surf, noise_bed(3, 0.03, 0.0)),
        (Bed::Fire, noise_bed(4, 0.25, 1.0)),
        (Bed::Sub, sub_bed()),
    ];
    for (kind, bytes) in beds {
        let h = sources.add(AudioSource { bytes });
        commands.spawn((
            AudioPlayer(h),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
            AmbientBed(kind),
        ));
    }
    // pre-bake call tones across the pitch range (big body = low, small = high).
    let calls = (0..CALL_PITCHES)
        .map(|i| {
            let p = i as f32 / (CALL_PITCHES - 1) as f32; // 0 low .. 1 high
            sources.add(AudioSource { bytes: call_tone(140.0 * (1.0 - p) + 1100.0 * p) })
        })
        .collect();
    commands.insert_resource(AudioAssets { calls, thunder: sources.add(AudioSource { bytes: thunder() }) });
}

// Make the camera the spatial-audio listener (once it exists).
fn attach_listener(mut commands: Commands, cam: Query<Entity, (With<Camera3d>, Without<SpatialListener>)>) {
    for e in &cam {
        commands.entity(e).insert(SpatialListener::new(4.0));
    }
}

// Camera sub-point direction in walk mode (where the listener "stands"); None in orbit/orrery -> ambient fades.
fn cam_ground(mode: &crate::camera::CameraMode, walkers: &Query<&crate::camera::WalkCam>) -> Option<Vec3> {
    if *mode != crate::camera::CameraMode::Walk {
        return None;
    }
    walkers.single().ok().map(|w| w.dir.normalize_or_zero())
}

// Per-frame ambient volumes from camera-local environment. Lerp toward target for smooth swells.
fn ambient_mix(
    mode: Res<crate::camera::CameraMode>,
    gen: Res<GenState>,
    fire: Res<Fire>,
    underwater: Res<crate::viz::Underwater>,
    walkers: Query<&crate::camera::WalkCam>,
    mut beds: Query<(&AmbientBed, &mut AudioSink)>,
) {
    let ground = cam_ground(&mode, &walkers);
    let (mut t_wind, mut t_rain, mut t_surf, mut t_fire, mut t_sub) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
    // Submerged: kill ALL land beds, run the muffled underwater bed instead -> sound is wholly different below.
    if let Some(d) = ground {
        if underwater.0 {
            t_sub = 0.55;
        } else {
            let rain = crate::sphere::rain_at(d, gen.tick);
            // surf: sample a few offsets -> fraction ocean nearby (hear the coast, not just standing in water).
            let mut ocean = 0.0;
            for k in 0..6 {
                let a = k as f32 * std::f32::consts::TAU / 6.0;
                let off = (Vec3::new(a.cos(), 0.0, a.sin()) * 0.05).normalize_or_zero();
                let probe = (d + off * 0.05).normalize_or_zero();
                if crate::sphere::is_ocean(probe) {
                    ocean += 1.0 / 6.0;
                }
            }
            let fire_here = fire.get(crate::sphere::surface_pos(d, 0.0));
            t_wind = 0.05 + 0.18 * rain; // breeze always; storm picks up wind
            t_rain = (rain * 1.1).min(1.0) * 0.5;
            t_surf = ocean * 0.45;
            t_fire = (fire_here * 1.5).min(1.0) * 0.5;
        }
    }
    for (bed, mut sink) in &mut beds {
        let target = match bed.0 {
            Bed::Wind => t_wind,
            Bed::Rain => t_rain,
            Bed::Surf => t_surf,
            Bed::Fire => t_fire,
            Bed::Sub => t_sub,
        };
        // lerp current -> target (~0.5s glide) so swells are smooth, not stepped.
        let cur = sink.volume().to_linear();
        let next = cur + (target - cur) * 0.04;
        sink.set_volume(Volume::Linear(next));
    }
}

// NN-driven creature calls: nearby calling creatures (Brain.voice>0) emit spatial one-shot tones, pitch by size.
// Throttled per-creature + capped per-frame so 2000 creatures don't cacophony.
fn creature_calls(
    mut commands: Commands,
    gen: Res<GenState>,
    assets: Res<AudioAssets>,
    mut log: ResMut<CallLog>,
    underwater: Res<crate::viz::Underwater>,
    cam: Query<&GlobalTransform, With<Camera3d>>,
    callers: Query<(Entity, &GlobalTransform, &Genome, &crate::components::Brain), With<Creature>>,
) {
    let Ok(cam_t) = cam.single() else { return };
    let cam_pos = cam_t.translation();
    let r2 = CALL_RADIUS * CALL_RADIUS;
    // collect audible callers off cooldown, loudest first.
    let mut cand: Vec<(Entity, Vec3, f32, f32)> = callers
        .iter()
        .filter(|(_, _, _, b)| b.voice > 0.0)
        .filter_map(|(e, t, g, b)| {
            let p = t.translation();
            if p.distance_squared(cam_pos) > r2 {
                return None;
            }
            if gen.tick.saturating_sub(*log.last.get(&e).unwrap_or(&0)) < CALL_COOLDOWN {
                return None;
            }
            Some((e, p, 1.0 - g.size, b.voice)) // pitch = 1-size (anatomical), loud = voice
        })
        .collect();
    cand.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
    // underwater listener: calls muffled -> slower+lower (speed<1) + quieter. "Sound is different down here."
    let (speed, vmul) = if underwater.0 { (0.55, 0.55) } else { (1.0, 1.0) };
    for (e, pos, pitch, loud) in cand.into_iter().take(CALL_MAX_PER_FRAME) {
        let bin = ((pitch * (CALL_PITCHES - 1) as f32).round() as usize).min(CALL_PITCHES - 1);
        commands.spawn((
            AudioPlayer(assets.calls[bin].clone()),
            PlaybackSettings::DESPAWN
                .with_spatial(true)
                .with_volume(Volume::Linear((loud.clamp(0.2, 1.0)) * vmul))
                .with_speed(speed),
            Transform::from_translation(pos),
        ));
        log.last.insert(e, gen.tick);
    }
    // prune stale cooldown entries so the map can't grow unbounded across long runs.
    if log.last.len() > 4096 {
        let tick = gen.tick;
        log.last.retain(|_, t| tick.saturating_sub(*t) < CALL_COOLDOWN * 4);
    }
}

// Thunder one-shot when lightning strikes near the camera. Reads strike events from the viz lightning system.
// Thunder one-shot when lightning strikes near the camera. Strikes is an ACCUMULATOR (filled by viz
// lightning_visuals at strike onset + the L god-key); this DRAINS it so writers never need a per-frame clear
// (avoids a clear-vs-write race) and so both natural + god-triggered strikes fire.
fn thunder_audio(
    mut commands: Commands,
    gen: Res<GenState>,
    assets: Res<AudioAssets>,
    mut log: ResMut<CallLog>,
    mut strikes: ResMut<crate::viz::Strikes>,
    cam: Query<&GlobalTransform, With<Camera3d>>,
) {
    if strikes.0.is_empty() {
        return;
    }
    if let Ok(cam_t) = cam.single() {
        if gen.tick.saturating_sub(log.last_strike_tick) >= 8 {
            // rate-limit: at most one thunder per ~8 ticks
            let cam_pos = cam_t.translation();
            let nearest = strikes
                .0
                .iter()
                .copied()
                .min_by(|a, b| a.distance_squared(cam_pos).partial_cmp(&b.distance_squared(cam_pos)).unwrap_or(std::cmp::Ordering::Equal));
            if let Some(n) = nearest {
                let d = n.distance(cam_pos);
                if d <= THUNDER_RADIUS {
                    let vol = (1.0 - d / THUNDER_RADIUS).clamp(0.0, 1.0).powi(2) * 0.85 + 0.12; // floor -> distant storms still rumble
                    commands.spawn((AudioPlayer(assets.thunder.clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(vol))));
                    log.last_strike_tick = gen.tick;
                }
            }
        }
    }
    strikes.0.clear(); // drain regardless (rate-limit drops extras)
}
