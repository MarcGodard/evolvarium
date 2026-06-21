//! Headless CPU snapshot renderer. Ray-traces the planet (no GPU) + overlays entity dots, writes PNGs so
//! the world can be inspected offline (e.g. by an agent reading the images). Perspective camera; the sphere
//! is shaded by terrain biome + sun lambert; clouds whiten; creatures/plants/trees draw as colored dots.
use crate::components::{Creature, Food, Rot, Tree};
use crate::genome::Genome;
use crate::sim::GenState;
use crate::sphere::{self, ELEV_MAX, PLANET_R};
use bevy::prelude::*;

// --shots config: capture a set of planet views to PNG at `at_tick`, then exit.
#[derive(Resource)]
pub struct ShotCfg {
    pub enabled: bool,
    pub at_tick: u32,
    pub prefix: String,
}
impl Default for ShotCfg {
    fn default() -> Self {
        ShotCfg { enabled: false, at_tick: 3000, prefix: "shot".into() }
    }
}

const SHOT_W: u32 = 900;
const SHOT_H: u32 = 600;

// Headless capture: once gen.tick reaches at_tick, render several views of the planet, save PNGs, exit.
pub fn snapshot_capture(
    gen: Res<GenState>,
    cfg: Res<ShotCfg>,
    mut done: Local<bool>,
    creatures: Query<(&Transform, &Genome), With<Creature>>,
    foods: Query<(&Transform, Option<&Tree>, Option<&Rot>), With<Food>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !cfg.enabled || *done || gen.tick < cfg.at_tick {
        return;
    }
    *done = true;
    let tick = gen.tick;
    // build the marker list. Creatures are colored by their thermal gene (cold=blue .. warm=red) so the
    // latitudinal niche is visible at a glance; trees dark green, carrion grey, plants green.
    let mut dots: Vec<Dot> = Vec::new();
    // niche census: count creatures by their evolved niche so biodiversity is quantified, not just drawn.
    let (mut cold, mut warm, mut aquatic, mut land, mut frugal, mut fast, mut spec, mut hidden) =
        (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    let (mut a_temp, mut a_lng, mut a_met, mut a_par, mut a_swim) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (t, g) in &creatures {
        let w = g.temp_pref.clamp(0.0, 1.0);
        let color = [
            (70.0 + (240.0 - 70.0) * w) as u8,  // cold blue -> warm red (R)
            (120.0 + (90.0 - 120.0) * w) as u8, // (G)
            (240.0 + (40.0 - 240.0) * w) as u8, // (B)
        ];
        dots.push(Dot { pos: t.translation, color, r: 2 });
        if g.temp_pref < 0.4 { cold += 1; } else if g.temp_pref > 0.6 { warm += 1; }
        if g.swim > 0.6 { aquatic += 1; } else if g.swim < 0.3 { land += 1; }
        if g.metab > 0.6 { frugal += 1; } else if g.metab < 0.4 { fast += 1; }
        if g.rigidity > 0.6 { spec += 1; } // diet specialist
        hidden += g.net.ih.len() as u32; // brain hidden neurons
        a_temp += g.temp_pref; a_lng += g.longevity; a_met += g.metab; a_par += g.parental; a_swim += g.swim;
    }
    let nc = creatures.iter().count().max(1) as f32;
    for (t, tree, rot) in &foods {
        let (color, r) = if tree.is_some() {
            ([25, 95, 35], 2)
        } else if rot.is_some() {
            ([160, 160, 160], 1)
        } else {
            ([70, 190, 80], 1)
        };
        dots.push(Dot { pos: t.translation, color, r });
    }
    let home = crate::sim::homeland_center();
    let pole = Vec3::new(0.15, 1.0, 0.0).normalize();
    let views: [(&str, Cam); 4] = [
        ("globe", Cam { eye: home * (PLANET_R * 3.0), target: Vec3::ZERO, fov_deg: 36.0 }),
        ("homeland", Cam { eye: home * (PLANET_R + 55.0), target: home * PLANET_R, fov_deg: 48.0 }),
        ("farside", Cam { eye: -home * (PLANET_R * 3.0), target: Vec3::ZERO, fov_deg: 36.0 }),
        ("pole", Cam { eye: pole * (PLANET_R * 3.0), target: Vec3::ZERO, fov_deg: 36.0 }),
    ];
    info!("snapshot at tick {tick}: {} creatures, {} foods", creatures.iter().count(), foods.iter().count());
    info!(
        "niche census: clime[cold {cold} / warm {warm}] habitat[aquatic {aquatic} / land {land}] metab[frugal {frugal} / fast {fast}] | diet-specialists {spec} | avg brain {:.1} neurons",
        hidden as f32 / nc
    );
    info!(
        "gene averages: temp {:.2} swim {:.2} longevity {:.2} metab {:.2} r/K {:.2}",
        a_temp / nc, a_swim / nc, a_lng / nc, a_met / nc, a_par / nc
    );
    for (name, cam) in &views {
        let buf = render(SHOT_W, SHOT_H, cam, tick, &dots);
        save_png(&format!("{}-{}.png", cfg.prefix, name), SHOT_W, SHOT_H, &buf);
    }
    exit.write(AppExit::Success);
}

pub struct Cam {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_deg: f32,
}

// A projected world marker: position + RGB color + pixel radius.
pub struct Dot {
    pub pos: Vec3,
    pub color: [u8; 3],
    pub r: i32,
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

// Nearest positive ray-sphere (centered at origin, radius R) hit distance.
fn ray_sphere(o: Vec3, d: Vec3, r: f32) -> Option<f32> {
    let b = o.dot(d);
    let c = o.length_squared() - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if t > 0.0 {
        Some(t)
    } else {
        None
    }
}

/// Ray-trace one frame to an RGB8 buffer (row-major, width*height*3).
pub fn render(width: u32, height: u32, cam: &Cam, tick: u32, dots: &[Dot]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let fwd = (cam.target - cam.eye).normalize();
    let right = fwd.cross(Vec3::Y).normalize();
    let up = right.cross(fwd);
    let aspect = w as f32 / h as f32;
    let tan = (cam.fov_deg.to_radians() * 0.5).tan();
    let sun = sphere::sun_dir(tick);
    let space = [4u8, 6, 12]; // near-black sky
    let mut buf = vec![0u8; w * h * 3];
    for c in buf.chunks_exact_mut(3) {
        c.copy_from_slice(&space);
    }
    // planet pass: enlarge the silhouette slightly so mountains don't clip the mean sphere
    let r_hit = PLANET_R + 0.5 * ELEV_MAX;
    for py in 0..h {
        for px in 0..w {
            let ndc_x = (2.0 * (px as f32 + 0.5) / w as f32 - 1.0) * aspect * tan;
            let ndc_y = (1.0 - 2.0 * (py as f32 + 0.5) / h as f32) * tan;
            let dir = (fwd + right * ndc_x + up * ndc_y).normalize();
            if let Some(t) = ray_sphere(cam.eye, dir, r_hit) {
                let n = (cam.eye + dir * t).normalize();
                let mut col = sphere::biome_color(n);
                let lam = n.dot(sun).max(0.0);
                let shade = 0.30 + 0.70 * lam; // ambient floor so the night side still reads, day side bright
                col = [col[0] * shade, col[1] * shade, col[2] * shade];
                let cloud = sphere::cloud_cover(n, tick);
                col = lerp3(col, [shade, shade, shade], cloud * 0.75); // clouds whiten, still sun-lit
                let i = (py * w + px) * 3;
                buf[i] = (col[0] * 255.0) as u8;
                buf[i + 1] = (col[1] * 255.0) as u8;
                buf[i + 2] = (col[2] * 255.0) as u8;
            }
        }
    }
    // entity overlay: project each dot; skip if behind the camera or on the hidden far hemisphere
    for d in dots {
        let rel = d.pos - cam.eye;
        let zc = rel.dot(fwd);
        if zc <= 0.1 {
            continue;
        }
        let n = d.pos.normalize_or_zero();
        if n.dot((cam.eye - d.pos).normalize_or_zero()) <= 0.02 {
            continue; // facing away (occluded by the globe)
        }
        let u = ((rel.dot(right) / zc) / (aspect * tan) * 0.5 + 0.5) * w as f32;
        let v = (0.5 - (rel.dot(up) / zc) / tan * 0.5) * h as f32;
        let (cxp, cyp) = (u as i32, v as i32);
        for dy in -d.r..=d.r {
            for dx in -d.r..=d.r {
                if dx * dx + dy * dy > d.r * d.r {
                    continue;
                }
                let (x, y) = (cxp + dx, cyp + dy);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let i = (y as usize * w + x as usize) * 3;
                buf[i..i + 3].copy_from_slice(&d.color);
            }
        }
    }
    buf
}

/// Save an RGB8 buffer as a PNG.
pub fn save_png(path: &str, width: u32, height: u32, rgb: &[u8]) {
    match image::save_buffer(path, rgb, width, height, image::ExtendedColorType::Rgb8) {
        Ok(()) => info!("wrote snapshot {path}"),
        Err(e) => warn!("snapshot {path} failed: {e}"),
    }
}
