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
