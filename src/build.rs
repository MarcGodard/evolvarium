//! Construction: creatures that weave nests and heap earthworks. Niche construction, the evolutionary feedback
//! where organisms reshape the environment that then selects on them (beaver dams, termite mounds, nests).
//!
//! Two structures, both per field cell (grid::field), both PUBLIC GOODS: any creature in the cell benefits,
//! the builder pays. That makes free-riding vs kin benefit a live evolutionary question rather than a rule.
//!
//! - NEST (brain OUT_BUILD): litter woven from the cell's soil organic pool into `SoilCell.nest`. Pure matter
//!   transfer (chem::gather_nest), rots back to organic (chem::decay_nest). Resting creatures share it: shelter
//!   = nest mass vs the TOTAL body mass in the cell, so a herd crowding one small nest gets little from it.
//!   Land only (an ocean-floor nest would also dodge burial). Gives insulation (series with pelt) + cover.
//! - EARTHWORK (brain OUT_DIG): soil heaped into a berm/dam, 0..1 per cell (`Earthworks`). No element change
//!   (earth moved, not made). Holds ground water against evaporation, so plants beside it grow wetter. Erodes.
//!
//! - TOOL (brain OUT_CRAFT): a knapped stone carried by the maker (Brain.tool, 0..1 quality, per life).
//!   Adds TOOL_BITE to bite against plant defense (nut cracking: capuchins, sea otters) and speeds digging.
//!   Knapping needs stone underfoot (fastest on rocky ground). Costs: carry weight on every step, wear per
//!   use, slow loss, and the knapping labour.
//! - CULTURE: a juvenile whose own making outputs fire beside someone who was making last tick gets
//!   R_IMITATE (social facilitation). Reward-modulated Hebbian learning then reinforces the habit in-life,
//!   so building spreads by example faster than by genes alone.
//!
//! Costs (every trait has one): `builder` gene upkeep paid always; building burns BUILD_WORK_X_BASAL x basal
//! power while the intent is on; weaving needs the creature still and grounded (no travelling to food); litter
//! woven into a nest is litter not mineralized for plants.
use bevy::prelude::*;

/// Intent above this = building this tick (sigmoid outputs idle ~0.5 on a fresh net; 0.6 keeps a random net
/// from building by accident most of the time, so the habit has to be selected or learned).
pub const BUILD_GATE: f32 = 0.6;
/// Thrust below this counts as resting: weaving, digging and sheltering all need a creature that stays put.
pub const REST_THRUST: f32 = 0.25;
/// Litter woven per REAL day per kg^0.75 of builder at full skill + intent, kg. A ~1 kg weaver bird builds a
/// ~0.3 kg nest in ~3-5 days; 0.08 kg/day lands there. Kleiber-scaled like all working power.
pub const WEAVE_KG_PER_DAY: f64 = 0.08;
/// Earthwork level (0..1 of a full dam) heaped per real day per kg^0.75 at full skill, on a MEAN-area field
/// cell (~13 m^2). A beaver pair raises a working dam in weeks: a ~3 kg digger (M^0.75 2.3) fills one in ~2 weeks.
pub const DIG_LEVEL_PER_DAY: f64 = 0.03;
/// Building burns this multiple of basal power on top of living (heavy manual work runs ~2-4x basal).
pub const BUILD_WORK_X_BASAL: f32 = 2.5;
/// Per-tick energy upkeep of the builder machinery at builder = 1 (dexterous mouthparts/limbs + the
/// planning circuitry). Same order as other morphology upkeeps (CLIMB_FLAT_COST, PELT_UPKEEP).
pub const BUILDER_UPKEEP: f32 = 0.0015;
/// Nest mass needed per kg of occupant for full shelter. A nest is ~0.2-0.5x its occupant's mass in birds;
/// 0.4 here, so a small body is sheltered by a small nest and a big one needs a big build.
pub const NEST_KG_PER_BODY_KG: f64 = 0.4;
/// Insulation a full nest adds, in pelt units (0..1, in series with the coat: see thermo::conductance).
pub const NEST_INSULATION: f32 = 0.6;
/// Fraction of predation success removed by full nest cover (a burrowed/nested animal is hard to extract).
pub const NEST_COVER: f32 = 0.6;
/// Nest rot rate per real day: woven grass/twig nests last a season (~120 d).
pub const NEST_DECAY_PER_DAY: f64 = 1.0 / 120.0;
/// Earthwork erosion per real day at full rain; unattended dams breach within ~a year of rain.
pub const EARTH_ERODE_PER_DAY: f64 = 1.0 / 365.0;
/// Fraction of ground-water evaporation a full earthwork holds back (a dammed pond loses water only slowly).
pub const DAM_RETAIN: f32 = 0.6;
/// Ground water above this counts as wet ground (world mean sits ~0.02; rain-soaked cells run 0.1-0.8).
pub const WET_GROUND: f32 = 0.05;
/// Comfort reward per unit of thermoregulation power the nest saved, normalized by basal: the felt relief of
/// warmth is the in-life learning signal for SEEKING shelter (building itself is selected, not rewarded).
pub const R_COMFORT: f32 = 0.05;

/// Tool quality gained per REAL day of full-effort knapping at full skill on pure rock (a usable flake in a
/// day or two of work, a fine one in a week).
pub const CRAFT_Q_PER_DAY: f64 = 0.3;
/// Bite added by a perfect tool, in bite units (plant defense spans 0..1).
pub const TOOL_BITE: f32 = 0.5;
/// Dig speed-up from a perfect tool (a digging stick/stone roughly doubles earth moved).
pub const TOOL_DIG: f32 = 1.0;
/// Locomotion cost added per unit tool quality (a good stone ~5% of a small body's mass, carried everywhere).
pub const TOOL_CARRY: f32 = 0.06;
/// Quality lost per use (each cracked plant, each dig tick).
pub const TOOL_WEAR_PER_USE: f32 = 0.01;
/// Fraction lost per real day regardless (dropped, chipped, mislaid).
pub const TOOL_LOSS_PER_DAY: f64 = 1.0 / 150.0;
/// Imitation reward for making beside a demonstrator, per unit of demonstrator effort.
pub const R_IMITATE: f32 = 0.3;
/// Ticks of life during which a creature learns by imitation (~a third of a typical ~2400-tick life).
pub const LEARN_AGE: u32 = 800;

// `--no-build` A/B arm: a world without making (no weaving/digging/knapping AND no builder upkeep), for
// multi-seed comparisons of what construction does to the planet. Process-global like the temp anomaly:
// read from parallel closures, set once at startup.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Per-tick builder upkeep (0 in the --no-build arm).
pub fn upkeep(builder: f32) -> f32 {
    if enabled() { BUILDER_UPKEEP * builder } else { 0.0 }
}

#[derive(Resource, Clone)]
pub struct Earthworks {
    pub level: Vec<f32>,
}

impl Default for Earthworks {
    fn default() -> Self {
        Earthworks { level: vec![0.0; crate::grid::field().len()] }
    }
}

impl Earthworks {
    pub fn add(&mut self, cell: usize, amt: f32) {
        self.level[cell] = (self.level[cell] + amt).min(1.0);
    }
    pub fn erode(&mut self, cell: usize, frac: f32) {
        self.level[cell] *= 1.0 - frac.clamp(0.0, 1.0);
    }
    pub fn built_cells(&self, min: f32) -> usize {
        self.level.iter().filter(|&&l| l > min).count()
    }
}

/// 0..1 shelter a nest of `nest_kg` gives when `occupant_kg` of bodies (all creatures in the cell, at least
/// the asker) share it.
pub fn shelter01(nest_kg: f64, occupant_kg: f64) -> f32 {
    if occupant_kg <= 0.0 {
        return 0.0;
    }
    (nest_kg / (NEST_KG_PER_BODY_KG * occupant_kg)).clamp(0.0, 1.0) as f32
}

/// Effective weave/dig/craft effort 0..1 this tick: intent past the gate, only while resting on the ground with
/// some skill. (Skill scales the OUTPUT of the work in weave_kg/dig_level/craft_gain, not the effort spent.)
pub fn intents(builder: f32, build_out: f32, dig_out: f32, craft_out: f32, resting: bool) -> (f32, f32, f32) {
    if !resting || builder <= 0.0 || !enabled() {
        return (0.0, 0.0, 0.0);
    }
    let on = |o: f32| if o > BUILD_GATE { (o - BUILD_GATE) / (1.0 - BUILD_GATE) } else { 0.0 };
    (on(build_out), on(dig_out), on(craft_out))
}

/// Tool quality knapped this tick at `rockiness` 0..1 underfoot (cobbles everywhere on land, best on rock).
pub fn craft_gain(builder: f32, effort: f32, rockiness: f32) -> f32 {
    (CRAFT_Q_PER_DAY * (builder * effort * (0.15 + 0.85 * rockiness)) as f64 * crate::chem::bio_days_per_tick()) as f32
}

/// Imitation reward: strongest demonstrator in view (last-tick effort) x my own making effort, juveniles only.
pub fn imitation_reward(age: u32, my_effort: f32, demo_effort: f32) -> f32 {
    if age >= LEARN_AGE || my_effort <= 0.0 {
        return 0.0;
    }
    R_IMITATE * demo_effort.min(1.0) * my_effort.min(1.0)
}

/// Kg of litter woven this tick.
pub fn weave_kg(body_kg: f64, builder: f32, effort: f32) -> f64 {
    WEAVE_KG_PER_DAY * body_kg.max(0.0).powf(0.75) * builder as f64 * effort as f64 * crate::chem::bio_days_per_tick()
}

/// Earthwork level heaped this tick, spread over the cell's area (a bigger cell takes longer to dam).
pub fn dig_level(body_kg: f64, builder: f32, effort: f32, cell: usize) -> f32 {
    let g = crate::grid::field();
    let area_scale = g.mean_area_m2() / g.area_m2(cell); // DIG_LEVEL_PER_DAY is per MEAN-area cell
    (DIG_LEVEL_PER_DAY * body_kg.max(0.0).powf(0.75) * builder as f64 * effort as f64 * area_scale * crate::chem::bio_days_per_tick()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelter_needs_a_nest_big_enough_for_the_body() {
        assert_eq!(shelter01(0.0, 2.0), 0.0);
        assert!(shelter01(0.4, 1.0) > 0.99, "full fit for a small body");
        assert!(shelter01(0.4, 10.0) < 0.15, "the same nest barely shelters a big body");
    }

    #[test]
    fn building_needs_skill_rest_and_intent() {
        assert_eq!(intents(0.0, 1.0, 1.0, 1.0, true), (0.0, 0.0, 0.0));
        assert_eq!(intents(1.0, 1.0, 1.0, 1.0, false), (0.0, 0.0, 0.0));
        assert_eq!(intents(1.0, 0.5, 0.5, 0.5, true), (0.0, 0.0, 0.0));
        let (w, d, c) = intents(1.0, 1.0, 0.8, 0.0, true);
        assert!((w - 1.0).abs() < 1e-6 && d > 0.0 && d < 1.0 && c == 0.0);
    }

    #[test]
    fn a_tool_takes_days_to_knap_and_rock_helps() {
        let ticks_rock = 0.5 / craft_gain(1.0, 1.0, 1.0);
        let ticks_soil = 0.5 / craft_gain(1.0, 1.0, 0.0);
        let days = ticks_rock as f64 * crate::chem::bio_days_per_tick();
        assert!((0.5..5.0).contains(&days), "half-quality tool in {days:.1} days on rock");
        assert!(ticks_soil > 5.0 * ticks_rock, "rock must matter");
    }

    #[test]
    fn imitation_rewards_only_young_makers_beside_a_demonstrator() {
        assert!(imitation_reward(10, 1.0, 1.0) > 0.0);
        assert_eq!(imitation_reward(LEARN_AGE, 1.0, 1.0), 0.0, "adults no longer learn by example");
        assert_eq!(imitation_reward(10, 0.0, 1.0), 0.0, "watching without doing earns nothing");
        assert_eq!(imitation_reward(10, 1.0, 0.0), 0.0, "no demonstrator, no reward");
    }

    #[test]
    fn a_digger_dams_a_cell_in_weeks() {
        let g = crate::grid::field();
        let c = (0..g.len()).min_by(|&a, &b| (g.area_m2(a) - g.mean_area_m2()).abs().total_cmp(&(g.area_m2(b) - g.mean_area_m2()).abs())).unwrap();
        let days = crate::chem::bio_days_per_tick() / dig_level(3.0, 1.0, 1.0, c) as f64;
        assert!((7.0..30.0).contains(&days), "{days:.1} real days to dam a mean cell");
    }

    #[test]
    fn a_weaver_builds_a_nest_within_days_not_lifetimes() {
        // ~1 kg builder at full skill: ticks to weave a nest that fully shelters itself
        let need = NEST_KG_PER_BODY_KG * 1.0;
        let per_tick = weave_kg(1.0, 1.0, 1.0);
        let ticks = need / per_tick;
        let days = ticks * crate::chem::bio_days_per_tick();
        assert!((2.0..15.0).contains(&days), "{days:.1} real days ({ticks:.0} ticks)");
        assert!(ticks < 2400.0, "must finish well inside a ~2400-tick life");
    }
}
