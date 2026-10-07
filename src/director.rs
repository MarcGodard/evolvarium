//! Director: an automatic cinematographer for watching the planet. Picks what is interesting right now (an
//! eruption, a hunter on the prowl, a builder at work, a flier aloft, a wildfire, a crowd), frames it with a
//! slow circling shot and a caption, holds, then cuts to the next thing; slow orbital drifts in between.
//!
//! I toggles it. It also takes over by itself after IDLE_TAKEOVER_S with no input, and any input hands the
//! camera straight back. Runs in PostUpdate after the normal camera systems and simply overwrites the camera
//! transform, so the manual camera code needs no knowledge of it. Render-only: reads the sim, never writes it.
use bevy::prelude::*;

use crate::camera::{CameraMode, WalkCam};

const IDLE_TAKEOVER_S: f32 = 90.0;
const CAPTION_FADE_S: f32 = 1.2;

pub struct DirectorPlugin;

impl Plugin for DirectorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Director>()
            .add_systems(Startup, spawn_caption)
            .add_systems(Update, (director_input, pick_shot).chain())
            .add_systems(PostUpdate, (drive_camera, update_caption).chain().before(bevy::transform::TransformSystems::Propagate));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Overview,
    Eruption,
    Hunter,
    Builder,
    Flier,
    Fire,
    Crowd,
}

impl Kind {
    fn caption(self) -> &'static str {
        match self {
            Kind::Overview => "The planet",
            Kind::Eruption => "A volcano erupts",
            Kind::Hunter => "A hunter on the prowl",
            Kind::Builder => "A builder at work",
            Kind::Flier => "Aloft",
            Kind::Fire => "Wildfire",
            Kind::Crowd => "Where the herds gather",
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    Entity(Entity),
    Point(Vec3),
    Planet,
}

#[derive(Clone, Copy)]
struct Shot {
    kind: Kind,
    target: Target,
    dur: f32,
    age: f32,
    dist: f32,   // camera distance from target (world units)
    height: f32, // camera height above the target
    spin: f32,   // rad/s around the target's local up
    phase: f32,  // starting angle
}

#[derive(Resource)]
pub struct Director {
    pub on: bool,
    idle: f32,
    shot: Option<Shot>,
    last_kind: Option<Kind>,
    seen_eruptions: Option<u32>,
    prev_mode: Option<CameraMode>, // mode the viewer was in when the director took over (restored on release)
    shot_count: u32,
    caption: String,
    caption_age: f32,
    // smoothed camera state so moving targets do not jitter the frame
    cam_pos: Option<Vec3>,
    cam_look: Vec3,
}

impl Default for Director {
    fn default() -> Self {
        Director {
            on: false,
            idle: 0.0,
            shot: None,
            last_kind: None,
            seen_eruptions: None,
            prev_mode: None,
            shot_count: 0,
            caption: String::new(),
            caption_age: 0.0,
            cam_pos: None,
            cam_look: Vec3::ZERO,
        }
    }
}

impl Director {
    pub fn engage(&mut self) {
        self.on = true;
        self.shot = None;
        self.cam_pos = None;
    }
}

// Hand the camera back in the mode the viewer left it in. A walk return lands at the director's last surface
// point, level, at normal eye height (the director moved WalkCam.dir and left pitch/eye height stale).
fn release(d: &mut Director, mode: &mut CameraMode, walk: &mut Query<&mut WalkCam>) {
    d.on = false;
    d.shot = None;
    let back = d.prev_mode.take().unwrap_or(*mode);
    if back == CameraMode::Walk {
        if let Ok(mut w) = walk.single_mut() {
            if let Some(p) = d.cam_pos {
                w.dir = p.normalize_or(w.dir);
            }
            w.pitch = 0.0;
            w.eye_alt = crate::camera::WALK_EYE;
        }
    }
    *mode = back;
}

fn director_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    time: Res<Time<Real>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    capture: Option<Res<crate::capture::CaptureCfg>>,
    mut mode: ResMut<CameraMode>,
    mut walk: Query<&mut WalkCam>,
    mut d: ResMut<Director>,
) {
    if keys.just_pressed(KeyCode::KeyI) {
        if d.on {
            release(&mut d, &mut mode, &mut walk);
            info!("director: off (I)");
        } else {
            d.engage();
            d.prev_mode = Some(*mode);
            info!("director: on (I) -- any key or click hands the camera back");
        }
        d.idle = 0.0;
        return;
    }
    let scrolled = wheel.read().count() > 0;
    let touched = scrolled || keys.get_just_pressed().next().is_some() || buttons.get_just_pressed().next().is_some();
    if touched {
        d.idle = 0.0;
        if d.on {
            release(&mut d, &mut mode, &mut walk);
            info!("director: handing the camera back");
        }
        return;
    }
    // real seconds, not sim time: the takeover delay and shot pacing must not stretch with the sim speed dial
    d.idle += time.delta_secs();
    // a capture never gets taken over unless it asked (--cap-director): its own camera owns the frame
    if !d.on && d.idle > IDLE_TAKEOVER_S && capture.is_none() {
        d.prev_mode = Some(*mode);
        d.engage();
        info!("director: taking over after {IDLE_TAKEOVER_S:.0}s idle (I or any input to stop)");
    }
}

fn hash01(a: u32, b: u32) -> f32 {
    let h = a.wrapping_mul(2654435761) ^ b.wrapping_mul(40503).rotate_left(11);
    (h >> 8) as f32 / (1u32 << 24) as f32
}

#[allow(clippy::type_complexity)]
fn pick_shot(
    time: Res<Time<Real>>,
    mut d: ResMut<Director>,
    clim: Option<Res<crate::climate::PlanetClimate>>,
    fire: Option<Res<crate::sim::Fire>>,
    creatures: Query<(Entity, &Transform, &crate::components::Brain, &crate::components::Locomotion, &crate::components::Alive), With<crate::components::Creature>>,
) {
    if !d.on {
        return;
    }
    let dt = time.delta_secs();
    // an eruption pre-empts whatever is playing
    let erupts = clim.as_ref().map_or(0, |c| c.eruptions);
    let new_eruption = matches!(d.seen_eruptions, Some(prev) if erupts > prev);
    d.seen_eruptions = Some(erupts);
    if new_eruption {
        if let Some(e) = clim.as_ref().and_then(|c| c.last_eruption) {
            let n = d.shot_count;
            d.shot = Some(Shot { kind: Kind::Eruption, target: Target::Point(e.vent), dur: 28.0, age: 0.0, dist: 26.0, height: 12.0, spin: 0.05, phase: hash01(n, 1) * 6.28 });
            start_shot(&mut d, Kind::Eruption);
            return;
        }
    }
    // end the current shot on time, or when its subject has died
    if let Some(mut s) = d.shot {
        s.age += dt;
        let subject_gone = matches!(s.target, Target::Entity(e) if creatures.get(e).map_or(true, |(_, _, _, _, a)| !a.0));
        if s.age < s.dur && !subject_gone {
            d.shot = Some(s);
            return;
        }
        d.shot = None;
    }
    // score candidates; never repeat the last kind back to back; an overview every few shots for breathing room
    let n = d.shot_count;
    let mut best: Option<(f32, Shot)> = None;
    let consider = |score: f32, shot: Shot, best: &mut Option<(f32, Shot)>| {
        let jitter = 0.6 + 0.8 * hash01(n, shot.kind as u32 + 7);
        let s = score * jitter;
        if best.is_none_or(|(b, _)| s > b) {
            *best = Some((s, shot));
        }
    };
    let last = d.last_kind;
    let close = |kind: Kind, target: Target, dist: f32, height: f32| Shot {
        kind,
        target,
        dur: 14.0 + 6.0 * hash01(n, 3),
        age: 0.0,
        dist,
        height,
        spin: if hash01(n, 4) > 0.5 { 0.12 } else { -0.12 },
        phase: hash01(n, 5) * std::f32::consts::TAU,
    };
    let mut hunters = Vec::new();
    let mut builders = Vec::new();
    let mut fliers = Vec::new();
    let mut all = Vec::new();
    for (e, tf, b, l, a) in &creatures {
        if !a.0 {
            continue;
        }
        all.push((e, tf.translation));
        if b.attack > crate::config::ATTACK_INTENT_THRESH {
            hunters.push(e);
        }
        if b.effort > 0.0 {
            builders.push(e);
        }
        if l.alt > 2.5 {
            fliers.push(e);
        }
    }
    let pick = |v: &Vec<Entity>, salt: u32| -> Option<Entity> {
        (!v.is_empty()).then(|| v[((hash01(n, salt) * v.len() as f32) as usize).min(v.len() - 1)])
    };
    if last != Some(Kind::Hunter) {
        if let Some(e) = pick(&hunters, 11) {
            consider(3.0, close(Kind::Hunter, Target::Entity(e), 6.5, 2.2), &mut best);
        }
    }
    if last != Some(Kind::Builder) {
        if let Some(e) = pick(&builders, 12) {
            consider(2.6, close(Kind::Builder, Target::Entity(e), 5.0, 2.0), &mut best);
        }
    }
    if last != Some(Kind::Flier) {
        if let Some(e) = pick(&fliers, 13) {
            consider(1.8, close(Kind::Flier, Target::Entity(e), 9.0, 3.0), &mut best);
        }
    }
    if last != Some(Kind::Fire) {
        if let Some(f) = fire.as_ref() {
            if let Some((c, _)) = f.cell.iter().enumerate().filter(|(_, v)| **v > 0.4).max_by(|a, b| a.1.total_cmp(b.1)) {
                let p = crate::grid::field().center(c);
                consider(2.2, close(Kind::Fire, Target::Point(p), 14.0, 6.0), &mut best);
            }
        }
    }
    if last != Some(Kind::Crowd) && !all.is_empty() {
        // densest spot: the creature with the most neighbours within 6 units (sampled, not O(n^2) on all)
        let sample = all.len().min(48);
        let mut top: Option<(usize, Entity)> = None;
        for k in 0..sample {
            let (e, p) = all[((hash01(n ^ k as u32, 17) * all.len() as f32) as usize).min(all.len() - 1)];
            let c = all.iter().filter(|(_, q)| q.distance_squared(p) < 36.0).count();
            if top.is_none_or(|(tc, _)| c > tc) {
                top = Some((c, e));
            }
        }
        if let Some((c, e)) = top {
            if c >= 4 {
                consider(1.4, close(Kind::Crowd, Target::Entity(e), 11.0, 4.5), &mut best);
            }
        }
    }
    if last != Some(Kind::Overview) {
        let breathe = if n % 4 == 3 { 4.0 } else { 0.9 };
        consider(breathe, Shot { kind: Kind::Overview, target: Target::Planet, dur: 18.0, age: 0.0, dist: 230.0, height: 0.0, spin: 0.06, phase: hash01(n, 6) * 6.28 }, &mut best);
    }
    if let Some((_, shot)) = best {
        d.shot = Some(shot);
        start_shot(&mut d, shot.kind);
    }
}

fn start_shot(d: &mut Director, kind: Kind) {
    d.last_kind = Some(kind);
    d.shot_count = d.shot_count.wrapping_add(1);
    d.caption = kind.caption().to_string();
    d.caption_age = 0.0;
    d.cam_pos = None; // cut, then settle
}

fn drive_camera(
    time: Res<Time<Real>>,
    mut d: ResMut<Director>,
    mut mode: ResMut<CameraMode>,
    targets: Query<&Transform, (Without<Camera3d>, Without<WalkCam>)>,
    mut cam: Query<(&mut Transform, &mut WalkCam), With<Camera3d>>,
) {
    if !d.on {
        return;
    }
    let Some(shot) = d.shot else { return };
    let Ok((mut tf, mut walk)) = cam.single_mut() else { return };
    let angle = shot.phase + shot.spin * shot.age;
    let (pos, look) = match shot.target {
        Target::Planet => {
            if *mode != CameraMode::Orbit {
                *mode = CameraMode::Orbit;
            }
            // slow sweep along a tilted great circle so the whole globe passes under the camera
            let dir = Vec3::new(angle.cos() * 0.8, 0.45 + 0.2 * (angle * 0.5).sin(), angle.sin() * 0.8).normalize();
            (dir * shot.dist, Vec3::ZERO)
        }
        Target::Entity(_) | Target::Point(_) => {
            let p = match shot.target {
                Target::Entity(e) => match targets.get(e) {
                    Ok(t) => t.translation,
                    Err(_) => return,
                },
                Target::Point(p) => crate::sphere::surface_pos(p.normalize_or_zero(), 0.0),
                Target::Planet => unreachable!(),
            };
            if *mode != CameraMode::Walk {
                *mode = CameraMode::Walk;
            }
            let up = p.normalize_or_zero();
            let (east, north) = crate::sphere::tangent_frame(up);
            let around = east * angle.cos() + north * angle.sin();
            let mut eye = p + around * shot.dist + up * shot.height;
            // never inside a hill: keep the eye above terrain + a margin
            let ed = eye.normalize_or_zero();
            let floor = crate::sphere::surface_pos(ed, 1.2);
            if eye.length() < floor.length() {
                eye = floor;
            }
            // walk-mode lighting/sky/ambient key off WalkCam.dir: keep it under the eye
            walk.dir = ed;
            let look = p + up * (if shot.kind == Kind::Eruption { shot.height * 0.6 } else { 0.6 });
            (eye, look)
        }
    };
    // ease toward the framing (fast enough to keep a moving subject in frame, slow enough to feel filmed)
    let k = (time.delta_secs() * 2.5).min(1.0);
    let fresh = d.cam_pos.is_none(); // a cut: snap, then settle
    let cur = d.cam_pos.unwrap_or(pos);
    let next = cur + (pos - cur) * k;
    d.cam_pos = Some(next);
    d.cam_look = if fresh { look } else { d.cam_look + (look - d.cam_look) * k };
    // overview looks at the planet centre, so the radial "up" would lie along the view axis (undefined roll that
    // flips as the sweep crosses z = 0): use the spin axis there. The sweep keeps |y| < 0.7, never near a pole.
    let up = if matches!(shot.target, Target::Planet) { Vec3::Y } else { next.normalize_or_zero() };
    *tf = Transform::from_translation(next).looking_at(d.cam_look, up);
}

#[derive(Component)]
struct DirectorCaption;

fn spawn_caption(mut commands: Commands) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(54.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((
            Text::new(""),
            TextFont { font_size: FontSize::Px(26.0), ..default() },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.0)),
            DirectorCaption,
        ));
}

fn update_caption(time: Res<Time<Real>>, mut d: ResMut<Director>, mut q: Query<(&mut Text, &mut TextColor), With<DirectorCaption>>) {
    let Ok((mut text, mut color)) = q.single_mut() else { return };
    if !d.on {
        color.0.set_alpha(0.0);
        return;
    }
    d.caption_age += time.delta_secs();
    if text.0 != d.caption {
        text.0 = d.caption.clone();
    }
    // fade in, hold ~4 s, fade out: a title card, not a permanent label
    let t = d.caption_age;
    let a = (t / CAPTION_FADE_S).min(1.0) * (1.0 - ((t - 4.0) / CAPTION_FADE_S).clamp(0.0, 1.0));
    color.0.set_alpha(a * 0.92);
}
