// Conserved matter: the world's C/N/P budget. Organizing invariant of the whole sim is MATTER IS CLOSED,
// ENERGY FLOWS THROUGH, exactly as a real biosphere works. Every gram of carbon, nitrogen, and phosphorus
// sits in exactly one reservoir at all times (soil mineral, soil organic, atmosphere, living tissue, buried
// sediment, unweathered rock); growth/death/decay MOVE matter between reservoirs and never create it.
// Energy is deliberately NOT conserved: sunlight enters, infrared leaves.
//
// Consequence that drives balance: carrying capacity is no longer a constant. A world supports only the
// biomass its scarcest element allows, so pop equilibrium falls out of the N and P budget. Anything that
// "just adds fertility" is a conservation bug, not a tuning knob.
//
// UNITS: SI throughout. 1 world unit = 1 metre (PLANET_R 80 = an 80 m worldlet, ~8 hectares of surface),
// masses in kg, time in seconds. Stocks are f64 because the reservoirs span six orders of magnitude:
// atmospheric N2 is ~7600 kg N per m^2 of surface while plant-available soil N is ~0.01 kg/m^2, and in f32
// a withdrawal that small vanishes into rounding against a stock that large.
use serde::{Deserialize, Serialize};

/// C/N/P triple in kg. Used for BOTH stocks (what a reservoir holds) and compositions (mass fraction per
/// kg of biomass), hence the generic arithmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Elements {
    pub c: f64,
    pub n: f64,
    pub p: f64,
}

impl Elements {
    pub const ZERO: Elements = Elements { c: 0.0, n: 0.0, p: 0.0 };

    pub const fn new(c: f64, n: f64, p: f64) -> Self {
        Elements { c, n, p }
    }

    pub fn total(&self) -> f64 {
        self.c + self.n + self.p
    }

    /// Largest biomass (kg) buildable from `self` at composition `comp`: LIEBIG'S LAW OF THE MINIMUM. The
    /// scarcest element alone sets the ceiling, with no averaging or substitution between elements. This
    /// one function replaces the old `fert / FERT_CAP` growth multiplier.
    pub fn max_biomass(&self, comp: Elements) -> f64 {
        let lim = |avail: f64, need: f64| if need <= 0.0 { f64::INFINITY } else { (avail / need).max(0.0) };
        lim(self.c, comp.c).min(lim(self.n, comp.n)).min(lim(self.p, comp.p))
    }

    /// Which element is binding. Diagnostic for the HUD and the balance logs: answers "what is this world
    /// actually short of", which under conservation is THE question about carrying capacity.
    pub fn limiting(&self, comp: Elements) -> Limiter {
        let lim = |avail: f64, need: f64| if need <= 0.0 { f64::INFINITY } else { (avail / need).max(0.0) };
        let (c, n, p) = (lim(self.c, comp.c), lim(self.n, comp.n), lim(self.p, comp.p));
        if c <= n && c <= p {
            Limiter::Carbon
        } else if n <= p {
            Limiter::Nitrogen
        } else {
            Limiter::Phosphorus
        }
    }

    /// Clamp each element at 0, guarding float drift at a reservoir floor. Callers must still withdraw only
    /// what `max_biomass` permits, or matter goes missing and the ledger test catches it.
    pub fn max0(&self) -> Elements {
        Elements::new(self.c.max(0.0), self.n.max(0.0), self.p.max(0.0))
    }

    pub fn is_finite(&self) -> bool {
        self.c.is_finite() && self.n.is_finite() && self.p.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limiter {
    Carbon,
    Nitrogen,
    Phosphorus,
}

impl Limiter {
    pub fn label(&self) -> &'static str {
        match self {
            Limiter::Carbon => "C",
            Limiter::Nitrogen => "N",
            Limiter::Phosphorus => "P",
        }
    }
}

impl std::ops::Add for Elements {
    type Output = Elements;
    fn add(self, o: Elements) -> Elements {
        Elements::new(self.c + o.c, self.n + o.n, self.p + o.p)
    }
}

impl std::ops::Sub for Elements {
    type Output = Elements;
    fn sub(self, o: Elements) -> Elements {
        Elements::new(self.c - o.c, self.n - o.n, self.p - o.p)
    }
}

impl std::ops::Mul<f64> for Elements {
    type Output = Elements;
    fn mul(self, k: f64) -> Elements {
        Elements::new(self.c * k, self.n * k, self.p * k)
    }
}

impl std::ops::AddAssign for Elements {
    fn add_assign(&mut self, o: Elements) {
        *self = *self + o;
    }
}

impl std::ops::SubAssign for Elements {
    fn sub_assign(&mut self, o: Elements) {
        *self = *self - o;
    }
}

// --- tissue composition (mass fraction per kg dry biomass) ---
// Real dry-matter chemistry. The PLANT/ANIMAL asymmetry is load-bearing, not flavor: plant tissue is mostly
// structural carbon (cellulose, lignin) and nitrogen-poor, while animal tissue is protein and so ~7x richer
// in N, ~10x in P. That gap is why a herbivore must process large plant volume to net enough nitrogen, and
// why an obligate carnivore takes in N far past what it can use. Rabbit starvation stops being a written
// rule and becomes stoichiometry.
pub const PLANT_COMP: Elements = Elements::new(0.45, 0.015, 0.0015);
pub const ANIMAL_COMP: Elements = Elements::new(0.45, 0.100, 0.0150);

// --- Earth-derived starting stocks, per m^2 of planet surface ---
// Sourced from standard terrestrial values so the world starts with a defensible budget rather than a
// tuned one. At PLANET_R = 80 m the surface is 4*pi*R^2 ~= 80,425 m^2.
/// Atmospheric carbon as CO2. Earth: ~830 Gt C over 5.1e14 m^2.
pub const AIR_C_PER_M2: f64 = 1.6;
/// Atmospheric nitrogen as N2. Earth: 78% of a 10,330 kg/m^2 air column. Enormous but INERT: this is
/// precisely why nitrogen fixation matters, and why an N-limited world can sit under an ocean of nitrogen
/// and still starve for it.
pub const AIR_N_PER_M2: f64 = 7600.0;
/// Soil organic carbon, temperate profile 0-30 cm.
pub const SOIL_ORG_C_PER_M2: f64 = 8.0;
// Soil organic C:N and C:P, DERIVED from litter composition rather than taken from a humus table. Soil
// organic matter here is litter-derived, so it inherits the litter's ratios, and that is what makes every
// element balance at steady state at once: first-order decay of a pool at litter stoichiometry releases each
// element at exactly the rate litterfall supplies it.
// Getting this wrong is subtle and was a real bug: humus tables give C:N ~12 while plant litter is ~30, so a
// pool held at 12 and decayed proportionally released 2.5x more nitrogen than litterfall could replace. Soil
// N then climbed to 7-18x real levels, nothing was ever nutrient-limited, and the plant COUNT stayed pinned
// to its cap instead of to the budget.
// (Real soils DO narrow toward C:N 12, because microbes retain N while respiring C. Modelling that
// immobilization explicitly is the refinement; it needs decomposition to hold N back until microbial demand
// is met, rather than releasing it in proportion.)
pub const SOIL_CN_RATIO: f64 = PLANT_COMP.c / PLANT_COMP.n;
pub const SOIL_CP_RATIO: f64 = PLANT_COMP.c / PLANT_COMP.p;
/// Plant-AVAILABLE mineral pools. Small by design: most soil N and P is locked in organic matter and must
/// be mineralized by decomposers before a plant can touch it.
pub const SOIL_MIN_N_PER_M2: f64 = 0.010;
pub const SOIL_MIN_P_PER_M2: f64 = 0.005;
/// Unweathered crustal phosphorus, the slow geological source. Phosphorus has NO gaseous phase on a real
/// planet, so unlike C and N it cannot be resupplied from the air at all: rock weathering is the only tap,
/// and burial to deep sediment is a real sink. That asymmetry is why P is the classic long-run limiter.
pub const ROCK_P_PER_M2: f64 = 2.0;

// --- biological tempo: the one place the time-lapse lives ---
// The sim runs TWO clocks and they genuinely disagree, so the disagreement is confined here rather than
// smeared across every rate constant.
//   Astronomical clock: DAY_TICKS(2400) * DT(1/60) = 40 s of step time per calendar day, vs 86400 real.
//   Biological clock: AGE_SCALE makes a creature live ~2400 ticks = 1 calendar day, vs ~1.5 years real.
// Feeding literal Earth rates into that would grow nothing inside a lifetime and starve the food web. The
// compression is FORCED, not sloppy: a realistic lifespan is ~1.2M ticks, about 69 min of wall clock per
// creature, far too slow to watch evolution happen.
//
// Resolution: make real what can be real (stoichiometry, conservation, limiting laws, standing STOCKS) and
// express every biotic RATE as a real per-day figure scaled by this single factor. Because all biotic rates
// carry the SAME factor, their ratios to each other stay physically real, and the ratios are what generate
// the emergent behavior: peat vs tropical turnover is the litterfall-to-decomposition ratio, not either
// rate alone. Change this to retime the whole biosphere coherently; never retune one rate against another.
//
// At 2000: one tick advances biology ~0.83 real days, so a 2400-tick life spans ~5.5 real years (a plausible
// small-mammal lifespan) and a cell's standing vegetation regrows in ~400 ticks (~330 real days, about the
// real annual turnover of grassland). Both land in real ranges, which is the check that this factor is sane.
pub const BIO_ACCEL: f64 = 2000.0;

/// Real biological days advanced by one sim tick. All biotic rates are quoted per real day and multiplied
/// by this, so a rate constant can be checked directly against a literature value.
pub fn bio_days_per_tick() -> f64 {
    BIO_ACCEL / crate::sphere::DAY_TICKS as f64
}

// --- decomposition: bacteria + fungi as a rate law, not entities ---
// Decomposition IS microbial activity, so it is modelled rather than assumed, but as a first-order pool
// transfer with environmental response (the standard soil-carbon approach, cf. RothC/CENTURY) instead of a
// separate living microbial stock. Living microbial biomass is part of the organic pool it feeds on, so
// this conserves trivially: organic -> (C to air, N and P to mineral).
//
// Emergent consequence, real and well known: where cold or dry, turnover runs slower than litter falls and
// organic matter accumulates (peat, tundra carbon lock-up); where warm and wet, turnover is near-instant so
// soil reads nutrient-POOR despite lush growth, because the budget is all held in living biomass. That is
// tropical soil, and it falls out of two rate terms.
/// Base first-order decay of soil organic matter, per REAL DAY at reference temperature and optimal
/// moisture. NOT free to pick: with NPP and the soil organic stock both taken from real Earth values, steady
/// state (mineralization = litterfall) pins this rate at
///   k = NPP * PLANT_COMP.c / SOIL_ORG_C_PER_M2 = 0.002 * 0.45 / 8 = 1.125e-4 /day
/// which is the stabilized-humus figure real soils show, and a ~8900-day bulk turnover. An earlier 0.002/day
/// was a fast-litter number and was inconsistent with the two stocks either side of it: it mineralized the
/// whole 8 kg/m2 profile within a few generations while litterfall could not keep up, draining organic carbon
/// to ~0 and piling mineral N to 67x its real level. `steady_state_is_self_consistent` locks the three
/// constants together so changing one without the others fails the build.
pub const DECOMP_K_PER_DAY: f64 = 1.125e-4;
/// Q10 = 2.0: respiration doubles per 10 K, the standard soil value.
pub const DECOMP_Q10: f64 = 2.0;
/// Reference temperature for Q10, in the sim's 0..1 temperature field (see `sphere::base_temperature`).
/// Kept in field units until Phase 3 gives temperature real Kelvin.
pub const DECOMP_T_REF: f64 = 0.6;
/// Field-temperature units per 10 K, so Q10 can act on the 0..1 field before the Kelvin conversion lands.
pub const DECOMP_T_PER_10K: f64 = 0.25;

/// Temperature multiplier on decomposition (Q10 law).
pub fn decomp_temp_factor(temp01: f64) -> f64 {
    DECOMP_Q10.powf((temp01 - DECOMP_T_REF) / DECOMP_T_PER_10K)
}

/// Moisture multiplier on decomposition. Rises from bone-dry (microbes inactive) to a broad optimum, then
/// falls back as saturation excludes oxygen and turns the cell anaerobic. The wet-end falloff is what lets
/// waterlogged ground bank organic matter as peat instead of respiring it away.
pub fn decomp_moisture_factor(moist01: f64) -> f64 {
    let m = moist01.clamp(0.0, 1.0);
    // peak at m = 0.6, zero at both ends; normalized so the peak is 1.0
    let f = 4.0 * m * (1.0 - m);
    (f * 1.04).clamp(0.0, 1.0)
}

/// Fraction of an organic pool mineralized over `days` real days, given local temperature and moisture.
/// Bounded below 1 so a cell can never mineralize more than it holds.
pub fn decomp_fraction(temp01: f64, moist01: f64, days: f64) -> f64 {
    (DECOMP_K_PER_DAY * decomp_temp_factor(temp01) * decomp_moisture_factor(moist01) * days).clamp(0.0, 1.0)
}

// --- other biotic and geological rates, all quoted per REAL DAY so they are checkable ---
/// Net primary production ceiling per m^2 per real day. Real temperate grassland is ~0.7 kg dry/m^2/yr.
/// This is a CEILING on demand, not a guarantee: Liebig still decides what actually gets built.
pub const NPP_PER_M2_DAY: f64 = 0.002;
/// Biological N fixation by a full legume, kg N per m^2 per real day. Real rhizobial systems run
/// ~200-300 kg N/ha/yr, which is ~6-8e-5 kg/m^2/day.
pub const NFIX_PER_M2_DAY: f64 = 8.0e-5;
/// Phosphorus released by rock weathering, kg P per m^2 per real day. Genuinely tiny: real continental
/// weathering is on the order of 1e-7, which is exactly why P accumulates slowly and limits over long runs.
pub const WEATHER_P_PER_M2_DAY: f64 = 2.0e-7;
/// Fraction of a cell's organic pool buried to deep sediment per real day. Only meaningful underwater.
pub const BURY_FRAC_PER_DAY: f64 = 1.0e-6;
/// Fraction of buried sediment returned by uplift and volcanism per real day. Slower than burial, so
/// sediment is a real net sink on ecological timescales while still not a permanent trap.
pub const UPLIFT_FRAC_PER_DAY: f64 = 2.0e-7;
/// Share of a burned plant's nitrogen that volatilizes to the atmosphere rather than staying as ash. Real
/// wildfire loses most site N this way, so fire is a nitrogen EXPORT: burned ground gains P and loses N.
pub const COMBUST_N_VOLATILE: f64 = 0.8;
/// Nominal ground a single plant draws from, m^2. Sets the per-plant NPP ceiling so growth speed stays
/// physically bounded instead of being limited only by the cell budget. 10 m^2 matches the world's observed
/// flora density (~4000 plants over ~40,000 m^2 of land), and the resulting ceiling of ~0.017 kg/tick lands
/// within a percent of the growth rate the sim already used, which is the check that the two agree.
pub const PLANT_FOOTPRINT_M2: f64 = 10.0;

/// Ground a tree's canopy and root plate draw from, m^2. Larger than a herb's, which is most of why a tree
/// out-grows the plants under it.
pub const TREE_FOOTPRINT_M2: f64 = 60.0;

/// Per-tick ceiling on one plant's mass gain, kg, from real net primary production. Liebig then decides how
/// much of this the local element budget can actually fund.
pub fn npp_ceiling_per_tick() -> f64 {
    NPP_PER_M2_DAY * PLANT_FOOTPRINT_M2 * bio_days_per_tick()
}

/// Density of animal tissue, kg/m^3. Real flesh is close to water.
pub const BODY_DENSITY: f64 = 1000.0;

/// Convert a body's geometric volume (`Morphometrics.mass` = sum of part volumes, m^3 under the 1 unit =
/// 1 metre scale) into kilograms. Creature bodies are not yet element-backed stocks; this is what lets the
/// ledger MEASURE the fauna it does not yet govern.
pub fn creature_mass_kg(morph_volume: f32) -> f64 {
    morph_volume.max(0.0) as f64 * BODY_DENSITY
}

/// Plausible band for animal standing stock as a fraction of plant standing stock. Each trophic step loses
/// roughly 90% of its energy, so a real ecosystem carries far less animal than plant: ~1-10%. A world above
/// this is asking its plants to support more animal flesh than they physically can, whatever the energy
/// bookkeeping says, and no amount of tuning fixes it because it is a thermodynamic constraint.
pub const TROPHIC_RATIO_MAX: f64 = 0.10;

/// Animal-to-plant standing-stock ratio. Diagnostic, not a limiter: the sim currently runs far above the
/// plausible band (measured ~10-16x, i.e. animals outweighing plants) and correcting that is a live balance
/// question about planet size versus population, not something to silently clamp.
pub fn trophic_ratio(flora_kg: f64, fauna_kg: f64) -> f64 {
    if flora_kg <= 0.0 {
        return f64::INFINITY;
    }
    fauna_kg / flora_kg
}

/// Total mass one soil cell's plant community can add per tick, kg. This is the AREA constraint: sunlight
/// falling on a patch is finite regardless of how many stems are planted in it, so the whole community
/// shares this budget. The per-plant ceilings above are the separate physiological limit on a single
/// individual's own canopy; both apply.
pub fn cell_npp_per_tick() -> f64 {
    NPP_PER_M2_DAY * cell_area() * bio_days_per_tick()
}

/// As `npp_ceiling_per_tick` for a tree's larger footprint.
pub fn npp_ceiling_tree_per_tick() -> f64 {
    NPP_PER_M2_DAY * TREE_FOOTPRINT_M2 * bio_days_per_tick()
}

// --- the world's reservoirs ---

/// One soil grid cell. The MINERAL/ORGANIC split is the whole point: death deposits into organic, which
/// nothing can eat, and only microbial mineralization makes it plant-available. Without that split,
/// "fertility" is instantly recycled and a corpse feeds the next plant on the same tick, which is what the
/// old scalar model did.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SoilCell {
    pub mineral: Elements,
    pub organic: Elements,
}

impl SoilCell {
    pub fn total(&self) -> Elements {
        self.mineral + self.organic
    }
}

/// Every non-living reservoir on the planet. Living tissue is held by the ECS (plant mass, creature mass,
/// carrion), so the full ledger is `Biosphere::total() + living biomass`, which is what `world_matter`
/// checks. Nothing here is a tuning knob: every field is a physical stock.
#[derive(bevy::prelude::Resource, Clone, Debug, Serialize, Deserialize)]
pub struct Biosphere {
    pub soil: Vec<SoilCell>,
    /// Well-mixed atmosphere. `p` is permanently zero: phosphorus has no gaseous phase.
    pub air: Elements,
    /// Deep sediment. Real sink; returns only via the slow uplift path.
    pub buried: Elements,
    /// Unweathered crust. Only tap for phosphorus, since P cannot come from the air.
    pub rock: Elements,
    /// Non-living total at world creation, kept so `drift_ppm` can report leakage as a live number rather
    /// than only in tests. Once living biomass is element-backed, pass it to `drift_ppm` and this becomes
    /// the single honest answer to "is the world still conserving".
    pub initial_total: Elements,
}

/// Surface area of one soil cell, m^2. The grid tiles the sphere, so this is total area over cell count.
pub fn cell_area() -> f64 {
    let r = crate::sphere::PLANET_R as f64;
    let n = (crate::config::SOIL_RES * crate::config::SOIL_RES) as f64;
    4.0 * std::f64::consts::PI * r * r / n
}

impl Biosphere {
    pub fn new() -> Self {
        let n = crate::config::SOIL_RES * crate::config::SOIL_RES;
        let a = cell_area();
        let org_c = SOIL_ORG_C_PER_M2 * a;
        let cell = SoilCell {
            mineral: Elements::new(0.0, SOIL_MIN_N_PER_M2 * a, SOIL_MIN_P_PER_M2 * a),
            organic: Elements::new(org_c, org_c / SOIL_CN_RATIO, org_c / SOIL_CP_RATIO),
        };
        let total_area = a * n as f64;
        let mut b = Biosphere {
            soil: vec![cell; n],
            air: Elements::new(AIR_C_PER_M2 * total_area, AIR_N_PER_M2 * total_area, 0.0),
            buried: Elements::ZERO,
            rock: Elements::new(0.0, 0.0, ROCK_P_PER_M2 * total_area),
            initial_total: Elements::ZERO,
        };
        b.initial_total = b.total();
        b
    }

    /// Leakage since creation, parts per million per element, given the `living` biomass currently held
    /// outside these reservoirs. Must stay at 0; anything else means a path is creating or destroying matter.
    pub fn drift_ppm(&self, living: Elements) -> Elements {
        let now = self.total() + living;
        let base = self.initial_total;
        let rel = |a: f64, b: f64| if b.abs() > 0.0 { (a - b) / b * 1.0e6 } else { 0.0 };
        Elements::new(rel(now.c, base.c), rel(now.n, base.n), rel(now.p, base.p))
    }

    /// One-line ledger for the balance logs: what the world holds, what binds it, and whether it leaks.
    /// `living` is biomass held outside the reservoirs (plants, creatures, carrion).
    pub fn report(&self, living: Elements, fauna_kg: f64) -> String {
        let a = cell_area();
        let n_cells = self.soil.len() as f64;
        let mineral: Elements = self.soil.iter().fold(Elements::ZERO, |t, c| t + c.mineral);
        let organic: Elements = self.soil.iter().fold(Elements::ZERO, |t, c| t + c.organic);
        // per-m^2 figures are the checkable ones: they compare directly against literature values
        let org_c_m2 = organic.c / (n_cells * a);
        let min_n_m2 = mineral.n / (n_cells * a);
        let avail = Elements::new(self.air.c, mineral.n / n_cells, mineral.p / n_cells);
        let drift = self.drift_ppm(living);
        // standing biomass per m^2 is the number that says whether the budget is actually binding: real
        // grassland carries 0.5-1 kg/m^2, and if the world sits far below that while mineral N accumulates,
        // something other than nutrients is limiting growth.
        let flora_kg_m2 = if PLANT_COMP.c > 0.0 { living.c / PLANT_COMP.c / (n_cells * a) } else { 0.0 };
        // fauna alongside flora is the comparison that matters: a real ecosystem's animal standing stock is
        // a small FRACTION of its plant standing stock (roughly 1-10%), because each trophic step loses ~90%.
        // Animals outweighing their food supply is thermodynamically impossible and means the world is
        // overstocked relative to the area it actually has.
        let fauna_kg_m2 = fauna_kg / (n_cells * a);
        format!(
            "flora {flora_kg_m2:.3} fauna {fauna_kg_m2:.3} orgC {org_c_m2:.2} minN {min_n_m2:.4} kg/m2 | lim {} | buried {:.1} | drift C{:+.3} N{:+.3} P{:+.3} ppm",
            avail.limiting(PLANT_COMP).label(),
            self.buried.total(),
            drift.c,
            drift.n,
            drift.p
        )
    }

    /// All non-living matter. Pair with living biomass for the full ledger.
    pub fn total(&self) -> Elements {
        let mut t = self.air + self.buried + self.rock;
        for c in &self.soil {
            t += c.total();
        }
        t
    }

    /// Grow `want` kg of tissue at composition `comp` in cell `idx`, drawing C from the air (photosynthesis)
    /// and N/P from soil mineral. Returns the mass actually built, which Liebig caps at whatever the
    /// scarcest element allows. The caller MUST add exactly this mass to the organism, or matter leaks.
    pub fn draw_for_growth(&mut self, idx: usize, comp: Elements, want: f64) -> f64 {
        if want <= 0.0 {
            return 0.0;
        }
        let cell = &mut self.soil[idx];
        // C comes from the atmosphere, N and P from this cell's mineral pool
        let avail = Elements::new(self.air.c, cell.mineral.n, cell.mineral.p);
        let built = want.min(avail.max_biomass(comp));
        if built <= 0.0 {
            return 0.0;
        }
        let take = comp * built;
        self.air.c -= take.c;
        cell.mineral.n -= take.n;
        cell.mineral.p -= take.p;
        cell.mineral = cell.mineral.max0();
        built
    }

    /// Return dead tissue to the soil as litter. Goes to ORGANIC, so it is not plant-available until
    /// decomposers mineralize it.
    pub fn deposit_litter(&mut self, idx: usize, mass: f64, comp: Elements) {
        if mass <= 0.0 {
            return;
        }
        self.soil[idx].organic += comp * mass;
    }

    /// Microbial turnover in one cell: respire organic C to the atmosphere and mineralize its N and P.
    /// This is the bacteria-and-fungi step; see `decomp_fraction` for the rate law.
    pub fn decompose(&mut self, idx: usize, temp01: f64, moist01: f64, dt: f64) {
        let f = decomp_fraction(temp01, moist01, dt);
        if f <= 0.0 {
            return;
        }
        let cell = &mut self.soil[idx];
        let moved = cell.organic * f;
        cell.organic -= moved;
        cell.organic = cell.organic.max0();
        self.air.c += moved.c; // respiration
        cell.mineral.n += moved.n;
        cell.mineral.p += moved.p;
    }

    /// How much plant growth this cell's mineral pool can still fund, normalized 0..1 against a full cell.
    /// Read-only, so the parallel decide can use it to gate things like seed set without touching the pool.
    pub fn cell_fertility01(&self, idx: usize) -> f32 {
        let a = cell_area();
        let full = Elements::new(f64::INFINITY, SOIL_MIN_N_PER_M2 * a, SOIL_MIN_P_PER_M2 * a);
        let cap = full.max_biomass(PLANT_COMP);
        if cap <= 0.0 {
            return 0.0;
        }
        let here = Elements::new(f64::INFINITY, self.soil[idx].mineral.n, self.soil[idx].mineral.p);
        (here.max_biomass(PLANT_COMP) / cap).clamp(0.0, 1.0) as f32
    }

    /// Combustion. Real fire volatilizes carbon and most nitrogen straight to the air and leaves phosphorus
    /// behind in the ash, which is exactly why burned ground regrows richer in P but not in N: a fire is a
    /// nitrogen EXPORT event for the site. Replaces FIRE_ASH/FIRE_BURN_ASH inventing fertility.
    pub fn combust(&mut self, idx: usize, mass: f64, comp: Elements) {
        if mass <= 0.0 {
            return;
        }
        let burned = comp * mass;
        self.air.c += burned.c;
        self.air.n += burned.n * COMBUST_N_VOLATILE;
        let cell = &mut self.soil[idx];
        cell.mineral.n += burned.n * (1.0 - COMBUST_N_VOLATILE);
        cell.mineral.p += burned.p; // P has no gaseous phase, so it cannot burn off: it all stays as ash
    }

    /// Herbivory in Phase 1: eaten tissue is respired and excreted on the spot rather than banked in a body,
    /// because creature bodies are not element-backed until Phase 2. Conserves exactly; what it lacks is the
    /// standing stock a living animal represents, which is small next to plant biomass. Phase 2 replaces this
    /// with a real body pool and makes excretion a separate, costed step.
    pub fn consume_and_excrete(&mut self, idx: usize, mass: f64, comp: Elements) {
        if mass <= 0.0 {
            return;
        }
        let eaten = comp * mass;
        self.air.c += eaten.c; // respiration
        let cell = &mut self.soil[idx];
        cell.mineral.n += eaten.n; // urea and dung, already plant-available
        cell.mineral.p += eaten.p;
    }

    /// Biological nitrogen fixation: rhizobia pulling inert atmospheric N2 into plant-available soil N.
    /// The `PlantGenome.nitrogen_fix` gene drives this, so a legume genuinely enriches its patch instead of
    /// conjuring fertility from nothing.
    pub fn fix_nitrogen(&mut self, idx: usize, kg: f64) {
        let take = kg.min(self.air.n).max(0.0);
        self.air.n -= take;
        self.soil[idx].mineral.n += take;
    }

    /// Rock weathering: the ONLY source of new phosphorus, since P has no atmospheric phase.
    pub fn weather(&mut self, idx: usize, kg_p: f64) {
        let take = kg_p.min(self.rock.p).max(0.0);
        self.rock.p -= take;
        self.soil[idx].mineral.p += take;
    }

    /// Sedimentation: matter leaving the active system for deep burial.
    pub fn bury(&mut self, idx: usize, frac: f64) {
        let f = frac.clamp(0.0, 1.0);
        if f <= 0.0 {
            return;
        }
        let cell = &mut self.soil[idx];
        let moved = cell.organic * f;
        cell.organic -= moved;
        self.buried += moved;
    }

    /// Uplift and volcanism returning buried matter: C to the air, N to the air, P back to rock. Slow, and
    /// the reason a single element cannot permanently bottleneck the world into a hard lock.
    pub fn uplift(&mut self, frac: f64) {
        let f = frac.clamp(0.0, 1.0);
        if f <= 0.0 {
            return;
        }
        let moved = self.buried * f;
        self.buried -= moved;
        self.air.c += moved.c;
        self.air.n += moved.n;
        self.rock.p += moved.p;
    }
}

impl Default for Biosphere {
    fn default() -> Self {
        Biosphere::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every reservoir op must be matter-neutral. Helper compares a before/after ledger where `living` is
    // whatever biomass the caller is holding outside the Biosphere.
    fn assert_conserved(before: Elements, after: Elements, what: &str) {
        for (a, b, el) in [
            (before.c, after.c, "C"),
            (before.n, after.n, "N"),
            (before.p, after.p, "P"),
        ] {
            let tol = 1e-9 * a.abs().max(1.0);
            assert!((a - b).abs() < tol, "{what} leaked {el}: {a} -> {b} (delta {})", b - a);
        }
    }

    #[test]
    fn arithmetic_round_trips() {
        let a = Elements::new(1.0, 2.0, 3.0);
        let b = Elements::new(0.5, 0.25, 0.125);
        assert_eq!(a + b - b, a);
        assert_eq!((a * 2.0).total(), a.total() * 2.0);
    }

    #[test]
    fn liebig_picks_the_scarcest_element() {
        // plenty of C and N, almost no P -> P alone sets the ceiling
        let pool = Elements::new(1000.0, 1000.0, 0.15);
        let m = pool.max_biomass(PLANT_COMP);
        assert!((m - 100.0).abs() < 1e-6, "P-limited biomass {m}, expected 100");
        assert_eq!(pool.limiting(PLANT_COMP), Limiter::Phosphorus);
        // relieving the limiter raises the ceiling; relieving a non-limiter does nothing at all
        assert!(Elements::new(1000.0, 1000.0, 1.5).max_biomass(PLANT_COMP) > m * 9.0);
        let more_c = Elements::new(9000.0, 1000.0, 0.15);
        assert!((more_c.max_biomass(PLANT_COMP) - m).abs() < 1e-6, "C is not limiting, ceiling must not move");
    }

    #[test]
    fn empty_pool_builds_nothing() {
        assert_eq!(Elements::ZERO.max_biomass(PLANT_COMP), 0.0);
    }

    #[test]
    fn animal_tissue_is_nitrogen_rich_vs_plant() {
        // the stoichiometric gap that makes herbivory hard and pure carnivory wasteful
        assert!(ANIMAL_COMP.n > PLANT_COMP.n * 5.0);
        assert!(ANIMAL_COMP.p > PLANT_COMP.p * 5.0);
    }

    #[test]
    fn building_then_returning_biomass_conserves() {
        // grow to the Liebig ceiling, then die back: the pool must land exactly where it started
        let pool = Elements::new(100.0, 10.0, 1.0);
        let m = pool.max_biomass(PLANT_COMP);
        let tissue = PLANT_COMP * m;
        let back = (pool - tissue) + tissue;
        for (a, b) in [(back.c, pool.c), (back.n, pool.n), (back.p, pool.p)] {
            assert!((a - b).abs() < 1e-12, "leaked matter: {a} vs {b}");
        }
    }

    #[test]
    fn f64_survives_the_reservoir_spread() {
        // the reason stocks are f64: withdrawing plant-scale N from the atmospheric N2 pool must register.
        // In f32 this subtraction is a no-op, which would silently mint nitrogen on every fixation tick.
        let air_n = AIR_N_PER_M2;
        let fixed = SOIL_MIN_N_PER_M2 * 0.01; // a plausible per-tick fixation bite
        assert!(fixed > 0.0);
        assert!(air_n - fixed < air_n, "f64 stock must register a small withdrawal");
        let as_f32 = air_n as f32 - fixed as f32;
        assert_eq!(as_f32, air_n as f32, "documents the f32 failure this type avoids");
    }

    #[test]
    fn q10_doubles_per_10k() {
        let cold = decomp_temp_factor(DECOMP_T_REF);
        let warm = decomp_temp_factor(DECOMP_T_REF + DECOMP_T_PER_10K);
        assert!((warm / cold - 2.0).abs() < 1e-9, "Q10 must double: {cold} -> {warm}");
    }

    #[test]
    fn decomposition_stalls_when_frozen_or_bone_dry() {
        let days = 1.0;
        let warm_moist = decomp_fraction(0.8, 0.6, days);
        let cold_moist = decomp_fraction(0.05, 0.6, days);
        let warm_dry = decomp_fraction(0.8, 0.0, days);
        let warm_drowned = decomp_fraction(0.8, 1.0, days);
        assert!(cold_moist < warm_moist * 0.2, "cold soil must bank litter (tundra), got {cold_moist} vs {warm_moist}");
        assert_eq!(warm_dry, 0.0, "bone-dry soil has no microbial activity");
        assert_eq!(warm_drowned, 0.0, "waterlogged soil goes anaerobic and banks peat");
    }

    #[test]
    fn decomp_fraction_never_exceeds_the_pool() {
        // a huge dt must not mineralize more than exists, or the ledger goes negative
        assert!(decomp_fraction(1.0, 0.6, 1.0e9) <= 1.0);
    }

    #[test]
    fn phosphorus_has_no_atmospheric_phase() {
        // real constraint worth locking down: C and N can be resupplied from air, P cannot. Any future
        // edit that gives air a P stock breaks the reason P is the long-run limiter.
        let air = Elements::new(AIR_C_PER_M2, AIR_N_PER_M2, 0.0);
        assert_eq!(air.p, 0.0);
        assert_eq!(Biosphere::new().air.p, 0.0);
    }

    // --- conservation: the phase gate ---

    #[test]
    fn growth_then_death_then_decay_conserves() {
        // one full trip round the loop: air+soil -> tissue -> litter -> mineralized back. Nothing may be
        // created or destroyed at any leg, and `living` must be carried explicitly or the leak shows up.
        let mut b = Biosphere::new();
        let start = b.total();
        let mut living = 0.0f64;

        for _ in 0..200 {
            living += b.draw_for_growth(7, PLANT_COMP, 0.05);
        }
        assert!(living > 0.0, "test is vacuous if nothing grew");
        assert_conserved(start, b.total() + PLANT_COMP * living, "growth");

        b.deposit_litter(7, living, PLANT_COMP);
        living = 0.0;
        assert_conserved(start, b.total(), "death");

        for _ in 0..500 {
            b.decompose(7, 0.7, 0.6, 20.0);
        }
        assert_conserved(start, b.total(), "decay");
    }

    #[test]
    fn geological_paths_conserve() {
        let mut b = Biosphere::new();
        let start = b.total();
        b.fix_nitrogen(3, 0.5);
        assert_conserved(start, b.total(), "fixation");
        b.weather(3, 0.25);
        assert_conserved(start, b.total(), "weathering");
        // litter must be GROWN out of the reservoirs, never conjured: depositing tissue that was never
        // drawn is itself a conservation bug, and an earlier version of this test had exactly that bug.
        let grown = b.draw_for_growth(3, PLANT_COMP, 10.0);
        assert!(grown > 0.0);
        b.deposit_litter(3, grown, PLANT_COMP);
        b.bury(3, 0.3);
        assert_conserved(start, b.total(), "burial");
        b.uplift(0.5);
        assert_conserved(start, b.total(), "uplift");
    }

    #[test]
    fn reservoirs_cannot_go_negative_under_overdraw() {
        // hammer a single cell far past its mineral stock: Liebig must throttle growth rather than let the
        // soil go negative, which would mint matter out of a rounding error.
        let mut b = Biosphere::new();
        let start = b.total();
        let mut living = 0.0f64;
        for _ in 0..10_000 {
            living += b.draw_for_growth(0, PLANT_COMP, 10.0);
        }
        assert!(b.soil[0].mineral.n >= 0.0, "soil N went negative");
        assert!(b.soil[0].mineral.p >= 0.0, "soil P went negative");
        assert_conserved(start, b.total() + PLANT_COMP * living, "overdraw");
    }

    #[test]
    fn fixation_cannot_outrun_the_atmosphere() {
        let mut b = Biosphere::new();
        let start = b.total();
        b.fix_nitrogen(0, f64::MAX / 4.0); // absurd demand
        assert!(b.air.n >= 0.0, "atmosphere N went negative");
        assert_conserved(start, b.total(), "unbounded fixation");
    }

    #[test]
    fn a_cell_becomes_nutrient_limited_not_infinite() {
        // the headline behavioral claim: growth in one patch STOPS at a budget rather than running forever.
        // This is what replaces PLANT_CAP as the thing bounding flora.
        let mut b = Biosphere::new();
        let mut living = 0.0f64;
        for _ in 0..100_000 {
            let got = b.draw_for_growth(0, PLANT_COMP, 1.0);
            if got <= 0.0 {
                break;
            }
            living += got;
        }
        assert!(living > 0.0);
        assert!(b.draw_for_growth(0, PLANT_COMP, 1.0) <= 1e-12, "a patch must exhaust, not grow forever");
        // and the binding element is one of N or P, never carbon: the air holds far more C than a cell's
        // mineral N and P can match at plant stoichiometry
        let avail = Elements::new(b.air.c, b.soil[0].mineral.n, b.soil[0].mineral.p);
        assert_ne!(avail.limiting(PLANT_COMP), Limiter::Carbon, "carbon must not be the limiter on land");
    }

    #[test]
    fn standing_biomass_lands_in_the_real_grassland_range() {
        // The reason for sourcing stocks from real per-m^2 Earth values instead of tuning them: the answer
        // they produce is checkable. A cell's mineral N supports ~0.67 kg/m^2 of plant tissue, and real
        // temperate grassland carries ~0.5-1 kg/m^2 standing dry biomass. If an edit to the stocks or to
        // PLANT_COMP pushes this out of range, the world has stopped being Earth-like and this fails.
        let b = Biosphere::new();
        let avail = Elements::new(b.air.c, b.soil[0].mineral.n, b.soil[0].mineral.p);
        let per_m2 = avail.max_biomass(PLANT_COMP) / cell_area();
        assert!(
            (0.3..=1.5).contains(&per_m2),
            "standing biomass {per_m2:.2} kg/m^2 is outside the real grassland range 0.3..1.5"
        );
        // temperate systems are characteristically NITROGEN-limited, and these stocks reproduce that
        assert_eq!(avail.limiting(PLANT_COMP), Limiter::Nitrogen);
    }

    #[test]
    fn steady_state_is_self_consistent() {
        // NPP, the soil organic stock, and the decomposition rate are three real Earth numbers that CANNOT be
        // chosen independently: at steady state mineralization must equal litterfall, so any two pin the
        // third. This test is why the world holds its soil instead of draining it.
        // must hold for EVERY element, not just carbon. Checking only C is what let soil N run away: the pool
        // balanced on carbon while releasing nitrogen 2.5x faster than litterfall replaced it.
        let org = Elements::new(
            SOIL_ORG_C_PER_M2,
            SOIL_ORG_C_PER_M2 / SOIL_CN_RATIO,
            SOIL_ORG_C_PER_M2 / SOIL_CP_RATIO,
        );
        for (el, litter, stock) in [
            ("C", NPP_PER_M2_DAY * PLANT_COMP.c, org.c),
            ("N", NPP_PER_M2_DAY * PLANT_COMP.n, org.n),
            ("P", NPP_PER_M2_DAY * PLANT_COMP.p, org.p),
        ] {
            let ratio = (DECOMP_K_PER_DAY * stock) / litter;
            assert!(
                (0.8..=1.25).contains(&ratio),
                "{el} is not at steady state: mineralization/litterfall = {ratio:.2}. Soil organic ratios \
                 must be derived from PLANT_COMP, and DECOMP_K_PER_DAY from \
                 NPP_PER_M2_DAY * PLANT_COMP.c / SOIL_ORG_C_PER_M2"
            );
        }
    }

    #[test]
    fn trophic_ratio_flags_an_inverted_pyramid() {
        // guards the CHECK, not the world: the sim currently measures ~10-16x (animals outweighing plants),
        // which is the inverted pyramid this is meant to catch once fauna becomes element-backed.
        assert!(trophic_ratio(1.0, 0.05) < TROPHIC_RATIO_MAX, "5% animal is plausible");
        assert!(trophic_ratio(0.2, 2.1) > TROPHIC_RATIO_MAX, "animals outweighing plants must fail");
        assert_eq!(trophic_ratio(0.0, 1.0), f64::INFINITY, "no plants at all cannot support animals");
    }

    #[test]
    fn cold_soil_banks_organic_matter_that_warm_soil_burns_off() {
        // tundra vs tropics, emergent from the Q10 rate law alone
        let run = |temp: f64| {
            let mut b = Biosphere::new();
            let start_org = b.soil[0].organic.c;
            for _ in 0..2000 {
                b.deposit_litter(0, 0.01, PLANT_COMP);
                b.decompose(0, temp, 0.6, 5.0);
            }
            b.soil[0].organic.c - start_org
        };
        let tundra = run(0.15);
        let tropics = run(0.95);
        assert!(tundra > tropics, "cold soil must accumulate litter faster than warm: {tundra} vs {tropics}");
    }
}
