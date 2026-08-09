// Thermodynamics: real heat balance for a body, in SI. Companion to chem.rs, which owns matter. The split
// is the organizing invariant: MATTER IS CLOSED (chem), ENERGY FLOWS THROUGH (here). Sunlight arrives,
// metabolism and sunlight warm a body, and infrared leaves. Nothing here conserves.
//
// Replaces three constants that each faked part of one physical process:
//   SIZE_BASAL_EXP = 1.5   invented allometry, where real biology is Kleiber's 0.75
//   TEMP_COST * mismatch   a penalty for being off a preferred temperature, with no body heat in it
//   PELT_COLD_RELIEF etc.  insulation as a discount on that penalty rather than as thermal resistance
// Under one heat balance those stop being three knobs pointing the same way and become one trade-off, which
// is what makes ENDOTHERMY EVOLVABLE: a warm body pays continuously to hold its temperature and buys
// cold-climate activity, a cold one is cheap and weather-bound. Neither is coded as better.
//
// UNITS: kg, metres, seconds, watts, KELVIN. The sim's 0..1 temperature field is converted at the boundary
// by `field_to_kelvin`; nothing inside this module sees field units.

/// Stefan-Boltzmann constant, W/m^2/K^4.
pub const STEFAN_BOLTZMANN: f64 = 5.670374e-8;
/// Longwave emissivity of skin, fur, and feathers. Real biological surfaces are near-perfect emitters in the
/// infrared regardless of visible colour, which is why a white animal still radiates like a black one.
pub const EMISSIVITY: f64 = 0.95;
/// Specific heat of animal tissue, J/(kg*K). Mostly water, so close to water's 4184.
#[allow(dead_code)]
pub const TISSUE_SPECIFIC_HEAT: f64 = 3470.0;
/// Convective heat transfer coefficient in still air, W/(m^2*K).
pub const H_AIR: f64 = 10.0;
/// Same in water. ~25x air: water strips heat far faster, which is why small aquatic endotherms need either
/// blubber or a large body, and why a swimmer's thermal problem is nothing like a walker's.
pub const H_WATER: f64 = 250.0;
/// Meeh coefficient for surface area A = MEEH_K * M^(2/3), m^2 per kg^(2/3). ~0.1 for mammals.
pub const MEEH_K: f64 = 0.1;

/// Kleiber coefficient for a resting ENDOTHERM, W per kg^0.75. The classic 70 kcal/day * M^0.75.
pub const KLEIBER_ENDO_W: f64 = 3.4;
/// Same for an ECTOTHERM at moderate temperature. Real reptile resting metabolism runs roughly 10-25x below
/// a mammal of equal mass; this is the whole energetic case for being cold-blooded.
pub const KLEIBER_ECTO_W: f64 = 0.17;
/// Kleiber exponent. NOT a free parameter: 3/4-power scaling holds across ~20 orders of magnitude of body
/// mass and is the single most robust quantitative law in physiology.
pub const KLEIBER_EXP: f64 = 0.75;

/// Thermal resistance added by a full pelt, m^2*K/W. Real winter fur runs ~0.15-0.4; the upper end is arctic
/// specialists. Acts as a resistance IN SERIES with the surface film, which is why insulation has sharply
/// diminishing returns once it already dominates that film.
pub const PELT_RESISTANCE: f64 = 0.30;

/// Body temperature an endotherm defends, K (~37 C).
pub const ENDO_SETPOINT_K: f64 = 310.0;

// --- field <-> SI boundary ---
/// Coldest surface temperature the 0..1 field represents, K (~-33 C, polar ice).
pub const FIELD_MIN_K: f64 = 240.0;
/// Warmest, K (~42 C, equatorial noon).
pub const FIELD_MAX_K: f64 = 315.0;

/// Convert the sim's 0..1 temperature field to Kelvin. The one place field units cross into physics.
pub fn field_to_kelvin(t01: f32) -> f64 {
    FIELD_MIN_K + (FIELD_MAX_K - FIELD_MIN_K) * (t01 as f64).clamp(0.0, 1.0)
}

/// Body surface area from mass (Meeh's law). Area scales as M^(2/3) while heat production scales as M^0.75,
/// and that mismatch IS the surface-area-to-volume problem: a small body has proportionally far more skin to
/// lose heat through, so tiny endotherms live on the edge of their heat budget.
pub fn surface_area(mass_kg: f64) -> f64 {
    MEEH_K * mass_kg.max(1e-6).powf(2.0 / 3.0)
}

/// How much food a body takes per feeding event, relative to a reference-sized animal.
///
/// SAME exponent as Kleiber on purpose. An animal must ingest what it burns, so if metabolism goes as
/// M^0.75 then so must intake, and the ratio between them is then size-INDEPENDENT. That is the point: it
/// makes body size neutral in the energy budget, leaving thermal inertia, predation, and famine buffering to
/// decide it. Before this, cost scaled with mass while a bite paid the same at any size, so small was
/// strictly cheaper with nothing pushing back, and the population evolved to ~1/87 of founder mass.
///
/// Reference is MORPH_MASS_REF, the body mass the rest of the balance is already written against.
pub fn intake_scale(mass_kg: f64) -> f64 {
    let reference = crate::chem::creature_mass_kg(crate::config::MORPH_MASS_REF);
    (mass_kg.max(1e-6) / reference).powf(KLEIBER_EXP)
}

/// Resting metabolic heat production, watts. `endothermy` 0 = fully ectothermic, 1 = fully endothermic.
pub fn basal_watts(mass_kg: f64, endothermy: f32) -> f64 {
    let e = (endothermy as f64).clamp(0.0, 1.0);
    let coeff = KLEIBER_ECTO_W + (KLEIBER_ENDO_W - KLEIBER_ECTO_W) * e;
    coeff * mass_kg.max(1e-6).powf(KLEIBER_EXP)
}

/// Total conductance from core to environment, W/K: surface film and insulation in series.
pub fn conductance(mass_kg: f64, pelt: f32, in_water: bool) -> f64 {
    let a = surface_area(mass_kg);
    let h = if in_water { H_WATER } else { H_AIR };
    let r_film = 1.0 / h;
    let r_insul = PELT_RESISTANCE * (pelt as f32).clamp(0.0, 1.0) as f64 * if in_water { 0.35 } else { 1.0 };
    // wet fur mats and loses most of its trapped air, so insulation is heavily discounted in water
    a / (r_film + r_insul)
}

/// Net heat flow into a body, watts. Positive warms it.
/// `metabolic_w` includes basal plus whatever activity is producing; `solar_w` is absorbed shortwave.
// BODY TEMPERATURE AS STATE: written, unit-tested, NOT yet wired into sim.rs. The live path uses the
// steady-state pair (thermoregulation_watts + activity_scale), which assumes a creature is always AT its
// equilibrium temperature. These three carry the transient instead: dT/dt = net_watts / (mass * specific
// heat). Wiring them buys thermal inertia, which scales with MASS and is the size advantage the energy
// budget currently lacks (a big body coasts through a cold night, a small one tracks the air). Kept rather
// than deleted because deleting tested physics to silence a warning trades a warning for a rewrite.
#[allow(dead_code)]
pub fn net_heat_watts(
    body_k: f64,
    env_k: f64,
    metabolic_w: f64,
    solar_w: f64,
    mass_kg: f64,
    pelt: f32,
    in_water: bool,
) -> f64 {
    let a = surface_area(mass_kg);
    // radiative exchange with the environment, linearised nowhere: the T^4 law is what makes a cold night
    // punishing out of proportion to its air temperature
    let radiated = EMISSIVITY * STEFAN_BOLTZMANN * a * (body_k.powi(4) - env_k.powi(4));
    let convected = conductance(mass_kg, pelt, in_water) * (body_k - env_k);
    metabolic_w + solar_w - radiated - convected
}

/// Advance body temperature by `dt` seconds under a net heat flow. Thermal inertia scales with MASS, so a
/// large body tracks the environment slowly (thermal stability for free, the real advantage of being big)
/// while a small one equilibrates almost immediately.
#[allow(dead_code)]
pub fn step_body_temp(body_k: f64, net_w: f64, mass_kg: f64, dt_s: f64) -> f64 {
    let heat_capacity = mass_kg.max(1e-6) * TISSUE_SPECIFIC_HEAT;
    body_k + net_w * dt_s / heat_capacity
}

/// Extra metabolic watts an endotherm must burn to hold its setpoint against the environment. Zero when the
/// environment is warm enough that basal heat already covers the loss, which is why a warm-blooded animal is
/// cheap in the tropics and ruinous in the cold.
pub fn thermoregulation_watts(
    env_k: f64,
    mass_kg: f64,
    pelt: f32,
    endothermy: f32,
    in_water: bool,
) -> f64 {
    let e = (endothermy as f64).clamp(0.0, 1.0);
    if e <= 0.0 {
        return 0.0; // an ectotherm does not defend a setpoint; it takes the environment's temperature
    }
    let target = FIELD_MIN_K + (ENDO_SETPOINT_K - FIELD_MIN_K) * e;
    let a = surface_area(mass_kg);
    let radiated = EMISSIVITY * STEFAN_BOLTZMANN * a * (target.powi(4) - env_k.powi(4));
    let convected = conductance(mass_kg, pelt, in_water) * (target - env_k);
    let loss = radiated + convected;
    (loss - basal_watts(mass_kg, endothermy)).max(0.0)
}

#[cfg(test)]
mod tests {
    // THE invariant behind intake_scale: if intake and basal cost share Kleiber's exponent, their ratio does
    // not depend on body size, so the energy budget stops voting for small. Guards against someone "fixing"
    // intake with a different exponent, which would quietly restore the free lunch that shrank the
    // population to ~1/87 of founder mass.
    #[test]
    fn intake_and_upkeep_scale_together_so_size_is_neutral() {
        let mouse = super::intake_scale(0.03) / super::basal_watts(0.03, 0.0);
        let mid = super::intake_scale(3.75) / super::basal_watts(3.75, 0.0);
        let big = super::intake_scale(60.0) / super::basal_watts(60.0, 0.0);
        assert!((mouse / mid - 1.0).abs() < 1e-6, "mouse {mouse} vs mid {mid}");
        assert!((big / mid - 1.0).abs() < 1e-6, "big {big} vs mid {mid}");
    }

    // A reference-mass body takes a full-size bite, so existing balance constants keep their meaning.
    #[test]
    fn reference_mass_bites_at_unity() {
        let reference = crate::chem::creature_mass_kg(crate::config::MORPH_MASS_REF);
        assert!((super::intake_scale(reference) - 1.0).abs() < 1e-9);
    }

    use super::*;

    #[test]
    fn kleiber_matches_real_metabolic_rates() {
        // the check that this is physiology and not a curve fit: a 0.5 kg endotherm should sit near 2 W
        let w = basal_watts(0.5, 1.0);
        assert!((w - 2.02).abs() < 0.05, "0.5 kg endotherm burns {w:.2} W, expected ~2.0");
        // and a 1 kg animal is exactly the coefficient
        assert!((basal_watts(1.0, 1.0) - KLEIBER_ENDO_W).abs() < 1e-9);
    }

    #[test]
    fn mass_specific_rate_falls_with_size() {
        // the actual content of the 3/4 exponent: gram for gram, a mouse burns far hotter than an elephant.
        // A linear or 1.5-power law gets this backwards, which is what SIZE_BASAL_EXP = 1.5 was doing.
        let small = basal_watts(0.02, 1.0) / 0.02;
        let large = basal_watts(2000.0, 1.0) / 2000.0;
        assert!(small > large * 10.0, "per-kg: small {small:.2} vs large {large:.4} W/kg");
    }

    #[test]
    fn endothermy_costs_an_order_of_magnitude() {
        let endo = basal_watts(1.0, 1.0);
        let ecto = basal_watts(1.0, 0.0);
        let ratio = endo / ecto;
        assert!((10.0..=30.0).contains(&ratio), "endo/ecto = {ratio:.1}, real is ~10-25x");
    }

    #[test]
    fn insulation_and_water_move_conductance_the_right_way() {
        let bare = conductance(1.0, 0.0, false);
        let furred = conductance(1.0, 1.0, false);
        let wet = conductance(1.0, 0.0, true);
        assert!(furred < bare * 0.5, "a full pelt must more than halve heat loss");
        assert!(wet > bare * 5.0, "water must strip heat far faster than air");
        // A first version of this test asserted the pelt loses RELATIVE value in water. That is wrong, and
        // the model was right: in water the surface film resistance collapses (1/250 vs 1/10), so insulation
        // is nearly the only resistance left and matters relatively MORE. That is why wet fur still beats
        // bare skin and why marine endotherms invest so heavily in blubber. Matting cuts the ABSOLUTE
        // resistance, not its relative share.
        // What is true and worth pinning: fur in water is far worse than fur in air.
        let furred_wet = conductance(1.0, 1.0, true);
        assert!(furred_wet > furred * 3.0, "a furred body must still bleed heat in water: {furred_wet:.2} vs {furred:.2}");
    }

    #[test]
    fn a_body_equilibrates_and_then_holds() {
        // no metabolism, no sun: the body must converge to ambient and stay, not oscillate or diverge
        let env = field_to_kelvin(0.6);
        let mut t = env + 20.0;
        for _ in 0..200_000 {
            let net = net_heat_watts(t, env, 0.0, 0.0, 1.0, 0.0, false);
            t = step_body_temp(t, net, 1.0, 0.1);
        }
        assert!((t - env).abs() < 0.5, "body settled at {t:.2} K, ambient {env:.2}");
    }

    #[test]
    fn small_bodies_equilibrate_faster_than_large_ones() {
        // thermal inertia scales with mass: being big IS thermal stability, without any gene for it
        let env = field_to_kelvin(0.2);
        let drift = |m: f64| {
            let mut t = ENDO_SETPOINT_K;
            for _ in 0..600 {
                let net = net_heat_watts(t, env, 0.0, 0.0, m, 0.0, false);
                t = step_body_temp(t, net, m, 1.0);
            }
            ENDO_SETPOINT_K - t
        };
        assert!(drift(0.05) > drift(500.0) * 5.0, "small body must cool far faster");
    }

    #[test]
    fn thermoregulation_is_cheap_when_warm_and_ruinous_when_cold() {
        // the trade-off that makes endothermy an evolvable strategy rather than a free upgrade
        let warm = thermoregulation_watts(field_to_kelvin(0.95), 1.0, 0.0, 1.0, false);
        let cold = thermoregulation_watts(field_to_kelvin(0.05), 1.0, 0.0, 1.0, false);
        assert!(cold > warm, "cold must cost more: {cold:.2} vs {warm:.2} W");
        assert!(cold > basal_watts(1.0, 1.0), "deep cold should exceed basal itself");
        // and an ectotherm pays nothing to hold a setpoint it does not defend
        assert_eq!(thermoregulation_watts(field_to_kelvin(0.05), 1.0, 0.0, 0.0, false), 0.0);
    }

    #[test]
    fn a_pelt_pays_for_itself_in_the_cold() {
        let cold = field_to_kelvin(0.1);
        let bare = thermoregulation_watts(cold, 1.0, 0.0, 1.0, false);
        let furred = thermoregulation_watts(cold, 1.0, 1.0, 1.0, false);
        assert!(furred < bare, "fur must cut the heating bill: {furred:.2} vs {bare:.2} W");
    }

    #[test]
    fn field_conversion_spans_a_real_planetary_range() {
        assert!((field_to_kelvin(0.0) - FIELD_MIN_K).abs() < 1e-9);
        assert!((field_to_kelvin(1.0) - FIELD_MAX_K).abs() < 1e-9);
        // freezing must land inside the field, or ice and liquid water cannot coexist on this planet
        let freeze01 = (273.15 - FIELD_MIN_K) / (FIELD_MAX_K - FIELD_MIN_K);
        assert!((0.05..=0.95).contains(&freeze01), "0 C sits at field {freeze01:.2}");
    }
}

// --- the benefit side of endothermy ---
// Charging for warm blood without paying it back is what inverted this world's thermal niches: mean
// endothermy came out HIGHEST in the warm band (0.39) and LOWEST in the cold (0.19), the exact reverse of
// Earth, where endotherms dominate cold climates. With only a cost term, the cold is simply where warm blood
// is least affordable, so selection strips it there.
// What was missing is why endothermy is worth anything: a body's biochemistry runs at its own temperature.
// An ectotherm in the cold goes SLUGGISH and cannot forage, escape, or compete; an endotherm carries its
// climate with it and works at full rate in the dark and the cold. That is the payoff, and without it the
// trade-off is one-sided.

/// Q10 for whole-animal performance. Real ectotherm locomotion and digestion run Q10 ~2-3: every 10 K drop
/// Temperature at which an ectotherm performs at full rate, K (~30 C, a basking reptile's preferred body
/// temperature). Above this, performance is capped rather than continuing to climb.

/// Operating body temperature: an endotherm defends its setpoint, an ectotherm takes the environment's.
/// Mixed values interpolate, which is real: many animals are partial or seasonal endotherms.
#[allow(dead_code)]
pub fn body_temp_k(env_k: f64, endothermy: f32) -> f64 {
    let e = (endothermy as f64).clamp(0.0, 1.0);
    env_k + (ENDO_SETPOINT_K - env_k) * e
}

/// Half-width of an ectotherm's thermal performance curve, K. Real ectotherm performance curves are narrow:
/// a lizard near its preferred temperature is quick, and 15 K off it is torpid.
pub const ECTO_TOLERANCE_K: f64 = 14.0;

/// Performance multiplier 0..1. The real trade-off is not "warm is fast" but CURVE SHAPE: an ectotherm has a
/// narrow curve peaked at the temperature it is ADAPTED to, and an endotherm has a flat one because it
/// carries its own climate. So endothermy does not buy raw speed, it buys BREADTH: freedom to range across
/// climates, forage through the cold, and ignore where it is, while an ectotherm is locked to its band.
///
/// An earlier version pinned every ectotherm's optimum at a fixed 30 C. On a world averaging ~7 C that made
/// essentially the whole population permanently torpid (~9% speed) and halved the population, which is a
/// modelling error rather than a result: real ectotherms adapt their optimum to their local climate, and
/// `temp_pref` is exactly that adaptation.
pub fn activity_scale(env_k: f64, endothermy: f32, temp_pref: f32) -> f64 {
    let e = (endothermy as f64).clamp(0.0, 1.0);
    let optimum = field_to_kelvin(temp_pref);
    let miss = (env_k - optimum).abs();
    // gaussian-ish falloff off the optimum, flattened toward 1.0 as endothermy rises
    let ecto = (-(miss / ECTO_TOLERANCE_K).powi(2)).exp();
    (ecto + (1.0 - ecto) * e).clamp(0.05, 1.0)
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn an_ectotherm_is_fast_at_home_and_torpid_away_from_it() {
        // narrow curve: full rate at its adapted temperature, useless well off it
        let home = activity_scale(field_to_kelvin(0.3), 0.0, 0.3);
        let away = activity_scale(field_to_kelvin(0.9), 0.0, 0.3);
        assert!(home > 0.95, "an ectotherm at its own optimum is quick, got {home:.2}");
        assert!(away < 0.3, "far off its optimum it should be torpid, got {away:.2}");
    }

    #[test]
    fn endothermy_buys_breadth_not_speed() {
        // the actual payoff: an endotherm gives up nothing anywhere, so it can range across climates while an
        // ectotherm is pinned to its band. It gains NO advantage at the ectotherm's own optimum.
        let pref = 0.3;
        let at_home = (activity_scale(field_to_kelvin(0.3), 1.0, pref), activity_scale(field_to_kelvin(0.3), 0.0, pref));
        let away = (activity_scale(field_to_kelvin(0.9), 1.0, pref), activity_scale(field_to_kelvin(0.9), 0.0, pref));
        assert!((at_home.0 - at_home.1).abs() < 0.06, "no edge on the ectotherm's home ground");
        assert!(away.0 > away.1 * 2.5, "big edge away from it: {:.2} vs {:.2}", away.0, away.1);
    }

    #[test]
    fn a_cold_adapted_ectotherm_is_not_punished_for_the_cold() {
        // guards the bug this replaced: a fixed 30 C optimum made every creature on a ~7 C world torpid
        let cold = field_to_kelvin(0.15);
        assert!(activity_scale(cold, 0.0, 0.15) > 0.95, "cold-adapted life must work in the cold");
    }

    #[test]
    fn body_temp_tracks_strategy() {
        let cold = field_to_kelvin(0.1);
        assert!((body_temp_k(cold, 0.0) - cold).abs() < 1e-9, "an ectotherm takes ambient");
        assert!((body_temp_k(cold, 1.0) - ENDO_SETPOINT_K).abs() < 1e-9, "an endotherm holds setpoint");
    }
}
