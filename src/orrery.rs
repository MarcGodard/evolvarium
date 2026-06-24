//! Tychos solar-system model (geocentric). Drives the sky: sun + moon + wandering planets.
//! Replicates TYCHOSIUM (pholmq/TSN, GPL-2.0) nested deferent/epicycle scene graph: every body = chain of
//! rotated pivots off a parent. World transform: world(node) = world(parent) * rotateY(start+speed*T) *
//! translateX(orbit); child frames inherit the accumulated rotation so epicycles ride deferents. Geocentric
//! sky dir of a body = normalize(world(body) - world(Earth)). Speeds are radians per YEAR (Sun = TAU = 1
//! rev/yr). Pure math in YEAR units; sphere.rs bridges ticks<->years + applies axial tilt + daily spin.
//!
//! Earth-Sirius binary precession (user spec, extends literal TSN): TSN's Earth row is a circular ~25344-yr
//! revolution about SystemCenter (the PVP/precession orbit). We make it ELLIPTICAL: period 24000 yr,
//! apsides aimed at a fixed Sirius direction (periapsis = closest = "faces Sirius"), eccentricity tuned so
//! the slow apoapsis phase reads the observed ~26000-yr apparent precession. Kepler-2 (omega ~ 1/r^2) gives
//! a global rate multiplier k(t) = omega/n that scales the WHOLE system "by the same ratio" (warped time
//! T_eff for all fast bodies), so the year length breathes with Sirius distance: mean 360 d, longer when far
//! (slow), shorter when near (fast). Imperceptible within a session (24000-yr cycle); correctness + readout.
#![allow(dead_code)] // full TSN table + helpers; wired into sphere/viz incrementally
use bevy::prelude::*;
use std::f32::consts::{PI, TAU};

// --- Sirius precession ellipse (the math: one ellipse reconciles all three user numbers) ---
pub const MEAN_YEAR_DAYS: f32 = 360.0; // time-mean year (12 mo x 30 d). Actual year = MEAN/k breathes ~332..390
pub const P_PRECESS_YEARS: f32 = 24000.0; // true precession cycle (Earth-Sirius binary orbital period)
// e fixed by apparent-precession constraint: P*(1+e)^2/sqrt(1-e^2)=26000 -> (1+e)^2/sqrt(1-e^2)=1.08333.
// Same e gives year swing: k_apo=0.923 -> 390 d (far), k_peri=1.085 -> 332 d (near), time-mean=360 d.
pub const PRECESS_ECC: f32 = 0.0404;
pub const EARTH_ORBIT: f32 = 37.8453; // TSN Earth deferent radius = precession-orbit semi-major axis a
const M0: f32 = PI; // epoch phase: tick 0 near APOAPSIS -> today reads the slow ~26000 apparent precession + ~390-d year
// Fixed Sirius ecliptic direction (apsidal line; periapsis points here). Arbitrary-but-fixed unit-ish vec.
const SIRIUS_DIR: Vec3 = Vec3::new(0.86, 0.0, 0.51);

// --- TSN body table (verbatim from design doc 15; parents reference earlier indices) ---
struct Body {
    name: &'static str,
    parent: i32,   // -1 = SystemCenter root
    speed: f32,    // rad / year
    orbit: f32,    // deferent/epicycle radius
    start_deg: f32,
    tilt_deg: f32, // applied to this node's CHILD frame
}
const fn b(name: &'static str, parent: i32, speed: f32, orbit: f32, start_deg: f32, tilt_deg: f32) -> Body {
    Body { name, parent, speed, orbit, start_deg, tilt_deg }
}

// Indices of bodies we consume downstream.
pub const SYSTEM_CENTER: usize = 0;
pub const EARTH: usize = 1;
pub const MOON: usize = 4;
pub const SUN: usize = 6;
pub const MERCURY: usize = 9;
pub const VENUS: usize = 12;
pub const MARS: usize = 15;
pub const JUPITER: usize = 19;
pub const SATURN: usize = 21;

const TABLE: &[Body] = &[
    b("SystemCenter", -1, 0.0, 0.0, 0.0, 0.0),
    b("Earth", 0, -0.0002479160869310127, 37.8453, 0.0, -23.439062), // speed unused: replaced by elliptical precession
    b("Moon deferent A", 1, 0.71015440177343, 0.000712491685519208, 226.4, 0.0),
    b("Moon deferent B", 1, 0.0, 0.0, -1.8, 0.0), // TSN parent "Earth offset" (tiny) -> Earth
    b("Moon", 3, 83.28521, 0.25505129081458283, 261.2, 0.0),
    b("Sun deferent", 0, 0.0, 0.0, 0.0, 0.0),
    b("Sun", 5, 6.283185307179586, 100.0, 0.0, 0.0),
    b("Mercury deferent A", 0, 6.283185307179586, 100.0, 0.0, 0.0),
    b("Mercury deferent B", 7, -6.283185307179586, 0.6, 33.0, 0.0),
    b("Mercury", 8, 26.08763045, 38.710225, -180.8, 0.0),
    b("Venus deferent A", 0, 6.283185307179586, 100.0, 0.0, 0.0),
    b("Venus deferent B", 10, -6.283185307179586, 0.6, 16.6, 0.0),
    b("Venus", 11, 10.21331385, 72.327789, -23.6, 0.0),
    b("Mars deferent E", 0, 6.283185307179586, 100.0, 0.0, 0.0),
    b("Mars deferent S", 13, 0.3974599, 7.44385, -115.0, 0.0),
    b("Mars", 14, -3.33985, 152.677, 119.3, -23.439062),
    b("Phobos", 15, 6986.5, 5.0, 122.0, 0.0),
    b("Deimos", 15, 1802.0, 10.0, 0.0, 0.0),
    b("Jupiter deferent", 0, -6.283185307179586, 0.0, 75.4, 0.0),
    b("Jupiter", 18, 0.52994136, 520.4, -34.0, 0.0),
    b("Saturn deferent", 0, -6.283185307179586, 89.0, 518.0, 0.0),
    b("Saturn", 20, 0.21351984, 958.2, -123.8, 0.0),
    b("Uranus deferent", 0, -6.283185307179586, 170.0, 123.0, 0.0),
    b("Uranus", 22, 0.07500314, 1920.13568, 371.8, 0.0),
    b("Neptune deferent", 0, -6.283185307179586, 20.0, 175.2, 0.0),
    b("Neptune", 24, 0.03837314, 3004.72, 329.3, 0.0),
    b("Pluto deferent", 0, -6.283185307179586, 0.0, 8.0, 0.0),
    b("Pluto", 26, 0.0253303, 3948.2, 200.0, 122.5),
    b("Halleys deferent", 0, -6.283185307179586, 20.0, 179.0, 0.0),
    b("Halleys", 28, -0.0830100973, 1674.5, 76.33, 0.0),
    b("Eros deferent A", 0, 6.283185307179586, 100.0, 0.0, 0.0),
    b("Eros deferent B", 30, -7.291563307179587, 5.2, 0.0, 0.0),
    b("Eros", 31, 4.57668492, 145.79, 171.8, 0.0),
];

// --- precession (slow binary orbit on REAL time) ---
fn solve_ecc_anomaly(m: f32, e: f32) -> f32 {
    let m = m.rem_euclid(TAU);
    let mut ea = m; // good seed for small e
    for _ in 0..8 {
        ea -= (ea - e * ea.sin() - m) / (1.0 - e * ea.cos()); // Newton on Kepler eq M = E - e sin E
    }
    ea
}

/// Precession state at real time `tau` (mean-years elapsed). Returns (sirius_dist, true_anomaly, rate_k).
/// rate_k = omega/n (Kepler-2): >1 near periapsis (fast), <1 near apoapsis (slow); time-mean = 1.
fn precession(tau: f32) -> (f32, f32, f32) {
    let e = PRECESS_ECC;
    let m = M0 + TAU * tau / P_PRECESS_YEARS;
    let ea = solve_ecc_anomaly(m, e);
    let nu = 2.0 * (((1.0 + e).sqrt() * (ea * 0.5).sin()).atan2((1.0 - e).sqrt() * (ea * 0.5).cos()));
    let r = EARTH_ORBIT * (1.0 - e * ea.cos()); // distance Earth<->SystemCenter (focus)
    let k = (1.0 + e * nu.cos()).powi(2) / (1.0 - e * e).powf(1.5);
    (r, nu, k)
}

/// Global rate multiplier at `tau`: whole system speeds/slows by this ratio (Sirius binary drives it).
pub fn rate_k(tau: f32) -> f32 {
    precession(tau).2
}

/// Current year length (days). Mean 360; longer when Sirius far (slow), shorter when near (fast).
pub fn year_len_days(tau: f32) -> f32 {
    MEAN_YEAR_DAYS / rate_k(tau)
}

/// Warped system time for fast bodies: dT_eff/dtau = k. k ~const within a session (precession is 24000-yr),
/// so T_eff = k*tau is exact to ~1e-7 per session and faithful in the long-run mean (mean k = 1).
fn t_eff(tau: f32) -> f32 {
    rate_k(tau) * tau
}

/// Fixed Sirius direction (ecliptic frame, unit). Apsidal line of the precession orbit points here.
pub fn sirius_dir() -> Vec3 {
    SIRIUS_DIR.normalize()
}

/// Sirius proximity 0..1: 0 = closest (periapsis, "facing Sirius"), 1 = farthest (apoapsis).
pub fn sirius_dist01(tau: f32) -> f32 {
    let r = precession(tau).0;
    let lo = EARTH_ORBIT * (1.0 - PRECESS_ECC);
    let hi = EARTH_ORBIT * (1.0 + PRECESS_ECC);
    ((r - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Apparent (instantaneous) precession period at `tau` in years: P / k. ~26000 now (apoapsis), 24000 true.
pub fn apparent_precession_years(tau: f32) -> f32 {
    P_PRECESS_YEARS / rate_k(tau)
}

// --- body world positions (ecliptic frame, SystemCenter at origin) ---
fn sirius_angle() -> f32 {
    let s = sirius_dir();
    s.z.atan2(s.x)
}

/// World positions of every TSN body at real time `tau`. Earth rides the elliptical precession orbit; all
/// other bodies ride warped time T_eff so the whole system breathes with the Sirius-distance rate k.
fn world_positions(tau: f32) -> Vec<Vec3> {
    let te = t_eff(tau);
    let n = TABLE.len();
    let mut rot = vec![Quat::IDENTITY; n];
    let mut pos = vec![Vec3::ZERO; n];
    for i in 0..n {
        let bd = &TABLE[i];
        if i == EARTH {
            // elliptical precession: periapsis aimed at Sirius (nu=0 -> closest, toward Sirius).
            let (r, nu, _) = precession(tau);
            let ang = sirius_angle() + nu;
            pos[i] = Vec3::new(r * ang.cos(), 0.0, r * ang.sin());
            // child frame (moon) follows Earth's orbital longitude only. Axial tilt handled in sphere::
            // ecliptic_to_sky, NOT here, so the moon stays near the ecliptic (no 23-deg over-incline).
            rot[i] = Quat::from_rotation_y(ang);
            continue;
        }
        let (pr, pp) = if bd.parent < 0 {
            (Quat::IDENTITY, Vec3::ZERO)
        } else {
            (rot[bd.parent as usize], pos[bd.parent as usize])
        };
        let spin = Quat::from_rotation_y(bd.start_deg.to_radians() + bd.speed * te);
        pos[i] = pp + pr * (spin * Vec3::new(bd.orbit, 0.0, 0.0));
        rot[i] = pr * spin * Quat::from_rotation_x(bd.tilt_deg.to_radians());
    }
    pos
}

/// Geocentric ecliptic direction (unit) of body `idx` at real time `tau` = normalize(world - Earth).
pub fn geocentric_dir(idx: usize, tau: f32) -> Vec3 {
    let p = world_positions(tau);
    (p[idx] - p[EARTH]).normalize_or_zero()
}

/// Earth world pos (focus = SystemCenter) on the elliptical precession orbit at `tau`.
fn earth_pos(tau: f32) -> Vec3 {
    let (r, nu, _) = precession(tau);
    let ang = sirius_angle() + nu;
    Vec3::new(r * ang.cos(), 0.0, r * ang.sin())
}

/// Sun geocentric ecliptic dir (unit). HOT PATH (per-creature daylight): closed form, no table walk.
/// Sun deferent is identity, Sun rides a circular radius-100 orbit at TAU/yr -> pos = rotateY(TAU*T_eff)*x100.
/// Equivalence to the general table walk asserted in tests.
pub fn sun_ecliptic_dir(tau: f32) -> Vec3 {
    let th = TAU * t_eff(tau);
    // glam from_rotation_y(th)*X = (cos, 0, -sin); match it so the closed form == table walk.
    let sun = Vec3::new(100.0 * th.cos(), 0.0, -100.0 * th.sin());
    (sun - earth_pos(tau)).normalize_or_zero()
}

/// Moon geocentric ecliptic dir (unit). Not a hot path (rendered ~once/frame, not per-creature), so the
/// general table walk is fine and avoids hand-deriving the deferent chain.
pub fn moon_ecliptic_dir(tau: f32) -> Vec3 {
    geocentric_dir(MOON, tau)
}
pub fn planet_ecliptic_dir(idx: usize, tau: f32) -> Vec3 {
    geocentric_dir(idx, tau)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        assert_eq!(world_positions(1.234), world_positions(1.234));
        assert_eq!(rate_k(3.3), rate_k(3.3));
    }

    #[test]
    fn bodies_at_startpos_at_t0() {
        // tau=0 -> t_eff=0, so each non-Earth fast body sits at its start angle. Sun deferent is identity,
        // Sun orbit 100 at start 0 deg -> Sun on +x at radius 100.
        let p = world_positions(0.0);
        assert!((p[SUN].x - 100.0).abs() < 1e-2, "sun x {}", p[SUN].x);
        assert!(p[SUN].z.abs() < 1e-2, "sun z {}", p[SUN].z);
    }

    #[test]
    fn geocentric_dirs_unit() {
        for &idx in &[SUN, MOON, MERCURY, VENUS, MARS, JUPITER, SATURN] {
            for &tau in &[0.0f32, 0.3, 1.7, 50.0] {
                let d = geocentric_dir(idx, tau);
                assert!((d.length() - 1.0).abs() < 1e-3, "body {idx} tau {tau} len {}", d.length());
            }
        }
    }

    #[test]
    fn sun_sweeps_full_circle_over_year() {
        // Sun geocentric dir at half a year apart should be roughly opposite (sun crossed the sky).
        let a = sun_ecliptic_dir(0.0);
        let half = year_len_days(0.0) / MEAN_YEAR_DAYS * 0.5; // half a (current) year in mean-year units
        let b = sun_ecliptic_dir(half);
        assert!(a.dot(b) < -0.5, "half-year sun dot {} (expect ~opposite)", a.dot(b));
    }

    #[test]
    fn sun_dist_from_center_constant() {
        // Sun orbits SystemCenter on a circle radius 100 -> |Sun - SystemCenter| ~ 100 for all tau.
        for &tau in &[0.0f32, 0.25, 0.9, 7.3] {
            let p = world_positions(tau);
            let d = (p[SUN] - p[SYSTEM_CENTER]).length();
            assert!((d - 100.0).abs() < 1e-1, "sun-center dist {d} at tau {tau}");
        }
    }

    #[test]
    fn ecc_reconciles_precession_and_year() {
        // tick 0 = apoapsis: slow phase, apparent precession ~26000, year ~390 d.
        let k_apo = rate_k(0.0);
        assert!((k_apo - 0.923).abs() < 0.01, "k_apo {k_apo}");
        assert!((apparent_precession_years(0.0) - 26000.0).abs() < 400.0, "apparent {}", apparent_precession_years(0.0));
        assert!((year_len_days(0.0) - 390.0).abs() < 4.0, "year_apo {}", year_len_days(0.0));
        assert!(sirius_dist01(0.0) > 0.98, "apoapsis should be farthest, got {}", sirius_dist01(0.0));

        // half a precession period later = periapsis: fast phase, year ~332 d, closest to Sirius.
        let peri = P_PRECESS_YEARS * 0.5;
        let k_peri = rate_k(peri);
        assert!((k_peri - 1.085).abs() < 0.01, "k_peri {k_peri}");
        assert!((year_len_days(peri) - 332.0).abs() < 4.0, "year_peri {}", year_len_days(peri));
        assert!(sirius_dist01(peri) < 0.02, "periapsis should be closest, got {}", sirius_dist01(peri));
    }

    #[test]
    fn hot_path_matches_table_walk() {
        // cheap closed-form sun/moon dirs must equal the general geocentric table walk (no silent drift).
        for &tau in &[0.0f32, 0.2, 1.3, 11.0, 600.0] {
            let s_fast = sun_ecliptic_dir(tau);
            let s_tab = geocentric_dir(SUN, tau);
            assert!(s_fast.dot(s_tab) > 0.9999, "sun fast vs table tau {tau}: dot {}", s_fast.dot(s_tab));
        }
    }

    #[test]
    fn year_time_mean_is_360() {
        // average year length over a full precession cycle ~ 360 d (mean k = 1).
        let n = 240;
        let mut sum = 0.0;
        for i in 0..n {
            let tau = P_PRECESS_YEARS * (i as f32 / n as f32);
            sum += year_len_days(tau);
        }
        let mean = sum / n as f32;
        assert!((mean - 360.0).abs() < 1.5, "mean year {mean}");
    }
}
