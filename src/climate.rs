//! Planet climate coupled to the conserved carbon budget: the greenhouse, ice-albedo and volcanic forcings
//! that let LIFE change the planet's temperature and the planet push back.
//!
//! Energy flows through (forcing in W/m^2, Planck response out); matter stays closed (eruptions only MOVE
//! carbon buried -> air and phosphorus rock -> soil). Result is one global mean anomaly (K), written to
//! `sphere::set_temp_anomaly` once per tick so every temperature reader (plants, creatures, decomposition Q10,
//! ice line) feels it.
//!
//! Feedback loops that emerge, none scripted:
//! - soil-carbon: warm -> faster decomposition (Q10) -> more CO2 -> warmer (positive)
//! - CO2 fertilization: more CO2 -> more NPP -> carbon drawn into plants and soil (negative)
//! - ice-albedo: warm -> ice line retreats -> darker surface absorbs more sun (positive)
//! - Planck: warmer planet radiates more (negative, the anchor)
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// CO2 radiative forcing coefficient, W/m^2 per e-fold (Myhre 1998: dF = 5.35 ln(C/C0)). 3.7 W/m^2 per doubling.
pub const CO2_FORCING: f64 = 5.35;
/// Climate response WITHOUT ice-albedo, K per W/m^2. Earth ECS ~3 K / 3.7 W/m^2 = 0.81 total; ice-albedo
/// contributes ~0.3 W/m^2/K of that, so the remaining feedback is 3.7/3 + 0.3 = 1.53 -> 0.65. Ice-albedo is
/// computed explicitly from THIS planet's ice cover instead, so its share comes from geography, not Earth.
pub const LAMBDA_NO_ICE: f64 = 0.65;
/// Global mean insolation absorbed-or-reflected budget, W/m^2 (Earth S0/4 = 340).
pub const INSOLATION: f64 = 340.0;
/// Albedo contrast ice (0.6) vs unfrozen ground/sea mix (~0.25).
pub const ICE_ALBEDO_CONTRAST: f64 = 0.35;
/// Ocean mixed layer heat capacity per m^2 of OCEAN, J/m^2/K (70 m x 1025 kg/m^3 x 3990 J/kg/K). Weighted by
/// the planet's ocean fraction: land stores ~nothing on these timescales.
pub const MIXED_LAYER_HEAT: f64 = 70.0 * 1025.0 * 3990.0;
/// CO2 fertilization of NPP: NPP x (1 + beta ln(C/C0)). beta ~0.6 is the mid FACE-experiment estimate. Clamped
/// so a CO2 crash cannot drive NPP negative.
pub const CO2_FERT_BETA: f64 = 0.6;
/// Volcanic aerosol e-folding time, real days (~1 yr: Pinatubo cooling decayed over ~1-2 yr).
pub const AEROSOL_EFOLD_DAYS: f64 = 365.0;
/// Rate of VEI>=4 eruptions planet-wide, per real day (~1 per 1.5 yr on Earth). One whole-planet cadence,
/// not per-area: this worldlet's 8 ha would otherwise never erupt.
pub const ERUPT_PER_DAY: f64 = 1.0 / 547.0;
/// Each VEI step is ~10x rarer (Newhall & Self). P(VEI = 4 + k) = (1 - q) q^k.
pub const VEI_STEP_RARITY: f64 = 0.1;
/// Peak stratospheric aerosol forcing of a VEI 6 (Pinatubo ~ -3 W/m^2); x3 per VEI step.
pub const AEROSOL_VEI6_WM2: f64 = -3.0;
/// Ash phosphorus deposited per m^2 at a VEI 4 vent, kg P (1 cm tephra ~10 kg/m^2 at ~0.02% labile P). x3
/// per VEI step. Volcanic soils are famously fertile; this is why.
pub const ASH_P_VEI4_PER_M2: f64 = 0.002;
/// Fraction of DEEP SEDIMENT carbon a VEI 4 arc eruption returns to the air; x3 per step. Arc volcanoes
/// outgas subducted sediment carbon, so the source is `Biosphere.buried`, never invented.
pub const OUTGAS_VEI4_FRAC: f64 = 2.0e-4;

/// One eruption, for logs and render (plume / glow at `vent`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Eruption {
    pub tick: u32,
    pub vent: Vec3,
    pub vei: u32,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlanetClimate {
    /// Atmospheric C at the end of spin-up, kg: the CO2 reference C0.
    pub air_c0: f64,
    /// Area-weighted ice fraction at the end of spin-up, the albedo reference.
    pub ice0: f64,
    /// Ticks of spin-up left. The initial soil stocks are far from this world's own carbon equilibrium
    /// (organic C drains ~8 -> ~1 kg/m^2 over the first ~10 gens, ~x3 CO2), and the 0..1 temperature field
    /// describes the SETTLED planet. So C0 and ice0 track the world until spin-up ends, then lock: the
    /// anomaly measures what life does to an equilibrated planet, not the cold-start transient (standard
    /// control-run practice). Aerosols act throughout.
    #[serde(default)]
    pub spinup_left: u32,
    #[serde(default)]
    pub started: bool,
    /// Current global mean temperature anomaly, K.
    pub anomaly_k: f64,
    /// Current volcanic aerosol forcing, W/m^2 (<= 0), decays.
    pub aerosol_wm2: f64,
    pub ice_frac: f64,
    pub ocean_frac: f64,
    pub last_eruption: Option<Eruption>,
    pub eruptions: u32,
}

impl PlanetClimate {
    pub fn co2_ratio(&self, air_c: f64) -> f64 {
        if self.air_c0 > 0.0 { (air_c / self.air_c0).max(1e-6) } else { 1.0 }
    }
    pub fn forcing_co2(&self, air_c: f64) -> f64 {
        CO2_FORCING * self.co2_ratio(air_c).ln()
    }
    pub fn forcing_ice(&self) -> f64 {
        -INSOLATION * ICE_ALBEDO_CONTRAST * (self.ice_frac - self.ice0)
    }
    pub fn report(&self, air_c: f64) -> String {
        format!(
            " | CLIM{} co2 x{:.3} dT {:+.2}K ice {:.1}% aer {:.2} erupt {}",
            if self.spinup_left > 0 { "(spinup)" } else { "" },
            self.co2_ratio(air_c),
            self.anomaly_k,
            self.ice_frac * 100.0,
            self.aerosol_wm2,
            self.eruptions
        )
    }
    /// NPP multiplier from CO2 fertilization.
    pub fn co2_fertilization(&self, air_c: f64) -> f64 {
        (1.0 + CO2_FERT_BETA * self.co2_ratio(air_c).ln()).clamp(0.2, 3.0)
    }
    /// Advance the energy balance by `secs` real seconds: C dT/dt = F - T/lambda.
    pub fn step_energy(&mut self, air_c: f64, secs: f64) {
        let f = self.forcing_co2(air_c) + self.forcing_ice() + self.aerosol_wm2;
        let heat = (MIXED_LAYER_HEAT * self.ocean_frac).max(MIXED_LAYER_HEAT * 0.05);
        let tau = LAMBDA_NO_ICE * heat; // seconds
        let eq = LAMBDA_NO_ICE * f;
        // exact exponential relaxation: stable for any step, unlike explicit Euler at large secs/tau
        self.anomaly_k = eq + (self.anomaly_k - eq) * (-secs / tau).exp();
        self.aerosol_wm2 *= (-(secs / 86400.0) / AEROSOL_EFOLD_DAYS).exp();
    }
}

/// Global anomaly in the sim's 0..1 temperature field units.
pub fn anomaly_field(k: f64) -> f32 {
    (k / (crate::thermo::FIELD_MAX_K - crate::thermo::FIELD_MIN_K)) as f32
}

/// Area-weighted fraction of the field grid that is frozen (below FREEZE_TEMP at the current anomaly) and
/// the ocean fraction. Reads `base_temperature`, which already includes the live anomaly.
fn surface_fractions() -> (f64, f64) {
    let g = crate::grid::field();
    let (mut ice, mut ocean, mut total) = (0.0, 0.0, 0.0);
    for c in 0..g.len() {
        let d = g.center(c);
        let a = g.area_m2(c);
        total += a;
        if crate::sphere::base_temperature(d) < crate::config::FREEZE_TEMP {
            ice += a;
        }
        if crate::sphere::is_ocean(d) {
            ocean += a;
        }
    }
    (ice / total, ocean / total)
}

/// God control / capture hook: Some(vei) erupts a vent next climate tick (viz V key, `--cap-erupt`).
#[derive(Resource, Default)]
pub struct EruptRequest(pub Option<u32>);

/// Spin-up length: the generational warm-up span, by which soil carbon has settled to within a few %/gen.
pub const SPINUP_TICKS: u32 = crate::config::WARMUP_GENS * crate::config::GEN_TICKS;

/// Ice cover is a slow field: resample every this many ticks (6144 cells is cheap, but not every tick).
const ICE_RESAMPLE_TICKS: u32 = 60;

/// Order: after biogeochem_step (air C for this tick is final). biogeochem_step itself reads LAST tick's
/// anomaly via base_temperature: a deterministic one-tick lag, negligible against the ~3-yr ocean response.
pub fn climate_step(
    mut clim: ResMut<PlanetClimate>,
    mut bio: ResMut<crate::chem::Biosphere>,
    mut fire: ResMut<crate::sim::Fire>,
    mut rng: ResMut<crate::rng::Rng>,
    gen: Res<crate::sim::GenState>,
    mut request: Option<ResMut<EruptRequest>>,
) {
    let _g = crate::profile::scope("climate");
    if !clim.started {
        crate::sphere::set_temp_anomaly(0.0);
        let (ice, ocean) = surface_fractions();
        clim.ice_frac = ice;
        clim.ocean_frac = ocean;
        clim.spinup_left = SPINUP_TICKS;
        clim.started = true;
    }
    let days = crate::chem::bio_days_per_tick();
    if gen.tick % ICE_RESAMPLE_TICKS == 0 {
        clim.ice_frac = surface_fractions().0;
    }
    if clim.spinup_left > 0 {
        clim.spinup_left -= 1;
        clim.air_c0 = bio.air.c;
        clim.ice0 = clim.ice_frac;
    }
    // a request stays pending until a vent is found (pick_vent can miss all its draws on a given tick)
    if let Some(r) = request.as_mut() {
        if let Some(vei) = r.0 {
            if let Some(vent) = pick_vent(&mut rng) {
                erupt(&mut clim, &mut bio, &mut fire, vent, vei.clamp(4, 8), gen.tick);
                r.0 = None;
            }
        }
    }
    if (rng.f32() as f64) < ERUPT_PER_DAY * days {
        let mut vei = 4u32;
        while vei < 8 && (rng.f32() as f64) < VEI_STEP_RARITY {
            vei += 1;
        }
        if let Some(vent) = pick_vent(&mut rng) {
            erupt(&mut clim, &mut bio, &mut fire, vent, vei, gen.tick);
        }
    }
    let air_c = bio.air.c;
    clim.step_energy(air_c, days * 86400.0);
    crate::sphere::set_temp_anomaly(anomaly_field(clim.anomaly_k));
}

/// Vents sit on high rocky ground (arcs and ranges). A few random draws, keep the rockiest; None if the
/// draws all miss (an eruption that finds no vent simply does not happen this tick).
fn pick_vent(rng: &mut crate::rng::Rng) -> Option<usize> {
    let g = crate::grid::field();
    let mut best: Option<(usize, f32)> = None;
    for _ in 0..16 {
        let c = ((rng.f32() * g.len() as f32) as usize).min(g.len() - 1);
        let r = crate::sphere::rockiness(g.center(c));
        if r > 0.3 && best.is_none_or(|(_, br)| r > br) {
            best = Some((c, r));
        }
    }
    best.map(|(c, _)| c)
}

pub fn erupt(
    clim: &mut PlanetClimate,
    bio: &mut crate::chem::Biosphere,
    fire: &mut crate::sim::Fire,
    vent: usize,
    vei: u32,
    tick: u32,
) {
    let g = crate::grid::field();
    let scale = 3f64.powi(vei as i32 - 4);
    clim.aerosol_wm2 += AEROSOL_VEI6_WM2 / 9.0 * scale;
    // outgassing: deep sediment carbon back to the air (arc volcanism recycles what subduction buried)
    let c = (bio.buried.c * OUTGAS_VEI4_FRAC * scale).min(bio.buried.c).max(0.0);
    bio.buried.c -= c;
    bio.air.c += c;
    // ash blanket: rock P onto the vent cell and rings around it, thinning outward; vent ring set alight
    let rings = (vei - 3) as usize;
    let mut frontier = vec![vent];
    let mut seen = vec![vent];
    for ring in 0..=rings {
        let thin = 1.0 / (1 + ring) as f64;
        for &cell in &frontier {
            let p = ASH_P_VEI4_PER_M2 * scale * thin * g.area_m2(cell);
            bio.weather(cell, p);
            if ring <= 1 {
                fire.cell[cell] = 1.0;
            }
        }
        let mut next = Vec::new();
        for &cell in &frontier {
            for nb in g.neighbors(cell) {
                if !seen.contains(&nb) {
                    seen.push(nb);
                    next.push(nb);
                }
            }
        }
        frontier = next;
    }
    clim.eruptions += 1;
    clim.last_eruption = Some(Eruption { tick, vent: g.center(vent), vei });
    info!("ERUPTION VEI {vei} at cell {vent} | aerosol {:.2} W/m2 | outgassed {c:.1} kg C", clim.aerosol_wm2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clim_at(ice: f64) -> PlanetClimate {
        PlanetClimate { air_c0: 1000.0, ice0: ice, ice_frac: ice, ocean_frac: 0.5, ..Default::default() }
    }

    #[test]
    fn doubling_co2_without_ice_change_warms_by_planck_response() {
        let mut c = clim_at(0.1);
        for _ in 0..10_000 {
            c.step_energy(2000.0, 86400.0 * 10.0);
        }
        let want = LAMBDA_NO_ICE * CO2_FORCING * 2f64.ln(); // ~2.4 K
        assert!((c.anomaly_k - want).abs() < 1e-3, "anomaly {} want {want}", c.anomaly_k);
    }

    #[test]
    fn ocean_heat_lags_the_response_by_years() {
        let mut c = clim_at(0.1);
        c.step_energy(2000.0, 86400.0 * 365.0); // one real year
        let eq = LAMBDA_NO_ICE * CO2_FORCING * 2f64.ln();
        assert!(c.anomaly_k > 0.05 * eq && c.anomaly_k < 0.5 * eq, "1 yr reached {} of {eq}", c.anomaly_k);
    }

    #[test]
    fn melting_ice_adds_warming() {
        let mut c = clim_at(0.10);
        c.ice_frac = 0.08; // 2% of the planet thawed
        assert!(c.forcing_ice() > 0.0);
        let mut d = clim_at(0.10);
        for _ in 0..10_000 {
            c.step_energy(1000.0, 864000.0);
            d.step_energy(1000.0, 864000.0);
        }
        assert!(c.anomaly_k > d.anomaly_k + 1.0, "{} vs {}", c.anomaly_k, d.anomaly_k);
    }

    #[test]
    fn aerosol_cools_then_decays() {
        let mut c = clim_at(0.1);
        c.aerosol_wm2 = AEROSOL_VEI6_WM2;
        c.step_energy(1000.0, 86400.0 * 180.0);
        assert!(c.anomaly_k < -0.05);
        for _ in 0..40 {
            c.step_energy(1000.0, 86400.0 * 180.0);
        }
        assert!(c.aerosol_wm2.abs() < 1e-3 && c.anomaly_k.abs() < 0.01, "{} {}", c.aerosol_wm2, c.anomaly_k);
    }

    #[test]
    fn eruption_moves_matter_without_creating_it() {
        let mut bio = crate::chem::Biosphere::new();
        bio.buried.c = 5000.0;
        let before = bio.total();
        let mut clim = clim_at(0.1);
        let mut fire = crate::sim::Fire::new();
        let air0 = bio.air.c;
        erupt(&mut clim, &mut bio, &mut fire, 100, 6, 0);
        let after = bio.total();
        for (a, b) in [(before.c, after.c), (before.n, after.n), (before.p, after.p)] {
            assert!((a - b).abs() <= 1e-9 * a.abs().max(1.0), "matter changed {a} -> {b}");
        }
        assert!(bio.air.c > air0, "no outgassing");
        assert!(fire.cell[100] > 0.0, "vent not burning");
        assert!(clim.aerosol_wm2 < 0.0);
    }

    // Loop gain of the ice-albedo feedback on THIS planet's geography: K of extra warming per K, via ice cover.
    // >= 1 means runaway (snowball / hothouse). Mutates the global anomaly, so ignored by default:
    // `cargo test --release ice_albedo_gain -- --ignored --nocapture --test-threads=1`.
    #[test]
    #[ignore]
    fn ice_albedo_gain_probe() {
        for dk in [-4.0, -2.0, -1.0, 0.0, 1.0, 2.0, 4.0] {
            crate::sphere::set_temp_anomaly(anomaly_field(dk));
            let lo = surface_fractions().0;
            crate::sphere::set_temp_anomaly(anomaly_field(dk + 0.5));
            let hi = surface_fractions().0;
            let dice_dk = (hi - lo) / 0.5;
            let gain = -LAMBDA_NO_ICE * INSOLATION * ICE_ALBEDO_CONTRAST * dice_dk;
            println!("anomaly {dk:+.1} K: ice {lo:.4}, d(ice)/dK {dice_dk:.5}, loop gain {gain:.3}");
        }
        crate::sphere::set_temp_anomaly(0.0);
    }

    #[test]
    fn co2_fertilization_is_monotone_and_bounded() {
        let c = clim_at(0.1);
        assert!(c.co2_fertilization(2000.0) > c.co2_fertilization(1000.0));
        assert!((c.co2_fertilization(1000.0) - 1.0).abs() < 1e-12);
        assert!(c.co2_fertilization(1e-9) >= 0.2);
    }
}
