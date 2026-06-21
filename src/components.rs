// ECS components for the M1 foraging sim.
use bevy::prelude::*;

#[derive(Component)]
pub struct Creature;

#[derive(Component)]
pub struct Food;

// Tree marker: a long-lived plant. Two kinds: edible=true is a tall fruit tree only TALL creatures
// (sensors >= TREE_REACH) can reach + eat; edible=false is an evergreen that NOTHING can eat (pure
// structure / refuge). Moisture-immune; grows large; reproduces slowly into its own kind. (BACKLOG trees)
#[derive(Component, Clone, Copy)]
pub struct Tree {
    pub edible: bool,
}

// Grass marker: a lesser ground plant. Shares PlantGenome/PlantState/Food (so the eat path treats it
// like any plant) but lives on its own cap + lifecycle step (grass_step). Lesser = one nutrient, low
// energy, ~0 defense, ~0 height, high regrow (turf survives grazing). Ubiquitous on plant-capable soil.
#[derive(Component)]
pub struct Grass;

// Carrion marker + rot clock. A dead creature drops carrion: fresh = rich meat (no defense), but as
// `age` climbs its nutrition fades and toxin rises -> rotten meat poisons the eater (BACKLOG P3).
// Despawns once fully decomposed. Distinguishes carrion from living plants (which carry no Rot).
#[derive(Component)]
pub struct Rot {
    pub age: u32,
}

// Plant-matter food that FERMENTS instead of rotting into meat: fallen fruit + dead-plant detritus.
// Shares the Rot clock. Fresh -> sugar; mid-age fermentation window -> FAST energy (ethanol) + toxicity;
// over-rotted -> spoiled/gone. `toxic` scales the toxicity (fruit low, detritus high). Its presence is
// how the eat dispatch tells plant matter (-> fast/sugar) from animal carrion (-> fat).
#[derive(Component)]
pub struct Ferment {
    pub toxic: f32,
}

// Three energy currencies (metabolic stores), each a distinct burn/storage trade-off (no free lunch):
//   fast  = fermented-fruit / ethanol: burned FIRST + leaks even at rest, tiny cap -> can't bank, quick power.
//   sugar = staple (plants deliver ONLY sugar): medium burn, medium cap.
//   fat   = the bank: burned LAST + mobilizes SLOW (low power output), big cap (easy to store) but adds upkeep.
// Burn order fast->sugar->fat. Death when total() <= 0. Refilled by eating (routed by food source).
use crate::config::{FAT_POWER, STORE_LOSS};
#[derive(Component)]
pub struct Energy {
    pub fast: f32,
    pub sugar: f32,
    pub fat: f32,
}

// Starting split of a total energy budget across the three stores (sums to 1.0). Lean-ish: most as
// sugar (staple), a small quick-burn buffer, a modest fat reserve.
const SPLIT_FAST: f32 = 0.13;
const SPLIT_SUGAR: f32 = 0.47;
const SPLIT_FAT: f32 = 0.40;

impl Energy {
    pub fn total(&self) -> f32 {
        self.fast + self.sugar + self.fat
    }

    // Build stores from a single total budget (spawn/birth/generation-reset use this).
    pub fn from_total(t: f32) -> Self {
        Energy { fast: t * SPLIT_FAST, sugar: t * SPLIT_SUGAR, fat: t * SPLIT_FAT }
    }

    // Drain `cost` across stores fast->sugar->fat. Returns leftover shortfall (cost not covered -> starving).
    pub fn burn(&mut self, cost: f32) -> f32 {
        let mut c = cost.max(0.0);
        let f = self.fast.min(c);
        self.fast -= f;
        c -= f;
        let s = self.sugar.min(c);
        self.sugar -= s;
        c -= s;
        let t = self.fat.min(c);
        self.fat -= t;
        c -= t;
        c
    }

    // Instantly-available power: fat mobilizes SLOWLY (FAT_POWER<1) so a fat-only creature can't sprint.
    // Used to cap thrust -> "slow burning" teeth.
    pub fn power(&self) -> f32 {
        self.fast + self.sugar + FAT_POWER * self.fat
    }

    // Add to fast, capped. Returns wasted excess (no room).
    pub fn add_fast(&mut self, amt: f32, cap: f32) -> f32 {
        let room = (cap - self.fast).max(0.0);
        let take = amt.min(room);
        self.fast += take;
        amt - take
    }

    // Add to fat, capped. Returns wasted excess (no room).
    pub fn add_fat(&mut self, amt: f32, cap: f32) -> f32 {
        let room = (cap - self.fat).max(0.0);
        let take = amt.min(room);
        self.fat += take;
        amt - take
    }

    // Add to sugar; overflow above sugar_cap converts to fat at STORE_LOSS (storing is lossy, no free lunch).
    // Returns wasted excess in intake-equivalent units (-> growth-load via OVEREAT_G).
    pub fn add_sugar(&mut self, amt: f32, sugar_cap: f32, fat_cap: f32) -> f32 {
        let room = (sugar_cap - self.sugar).max(0.0);
        let take = amt.min(room);
        self.sugar += take;
        let over = amt - take;
        if over > 0.0 {
            let wasted_fat = self.add_fat(over * STORE_LOSS, fat_cap);
            wasted_fat / STORE_LOSS
        } else {
            0.0
        }
    }
}

// Food eaten this generation = selection fitness.
#[derive(Component)]
pub struct Fitness(pub f32);

// Facing angle (radians) in the x-z plane. NN turn output rotates it.
#[derive(Component)]
pub struct Heading(pub f32);

// Starved creatures stop acting but keep their fitness until generation end.
#[derive(Component)]
pub struct Alive(pub bool);

// Per-life working brain: weights start as a copy of the genome's w0, then learn during life
// (reward-gated Hebbian). Discarded at death; NOT inherited. The GA selects genomes that
// LEARN well, not the learned weights themselves (Baldwin effect, see 04).
use crate::genome::Net;
#[derive(Component)]
pub struct Brain {
    pub net: Net, // working copy of genome weights, tuned by lifetime learning
    pub prev_dist: f32, // distance to nearest food last tick, for approach-reward shaping
}

// Per-life locomotion diagnostic: where the creature was born + total path walked. Lets us measure
// roaming vs circling: net displacement / path ratio ~1 = straight rover, ~0 = spinning in place.
#[derive(Component)]
pub struct Locomotion {
    pub start: Vec3,
    pub path: f32,
}

// Per-life diet state. reserves[i] = current pool of nutrient i (topped by eating x uptake, depleted each
// tick by use). The master expression gene (genome::master_expression) reads reserves vs the uptake genes'
// demand -> gates how much energy a creature extracts from food. Low reserves on a demanded nutrient =
// deficiency -> growth-load (soft-gradient disease). g = accumulated growth-signaling load (mismatch +
// deficiency + toxin -> disease). age in ticks (aging hazard). fatigue = exertion debt 0..1 (rest instinct).
use crate::genome::NUTRIENTS;
#[derive(Component)]
pub struct DietState {
    pub reserves: [f32; NUTRIENTS],
    pub g: f32,
    pub age: u32,
    pub fatigue: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::STORE_LOSS;

    #[test]
    fn burn_drains_fast_then_sugar_then_fat() {
        let mut e = Energy { fast: 2.0, sugar: 3.0, fat: 5.0 };
        // burn 4 -> empties fast(2), takes 2 from sugar
        let short = e.burn(4.0);
        assert_eq!(short, 0.0);
        assert!((e.fast - 0.0).abs() < 1e-6);
        assert!((e.sugar - 1.0).abs() < 1e-6);
        assert!((e.fat - 5.0).abs() < 1e-6);
        // burn 10 -> drains the rest (1 sugar + 5 fat = 6), shortfall 4
        let short2 = e.burn(10.0);
        assert!((short2 - 4.0).abs() < 1e-6);
        assert_eq!(e.total(), 0.0);
    }

    #[test]
    fn sugar_overflow_converts_to_fat_at_loss() {
        let mut e = Energy { fast: 0.0, sugar: 0.0, fat: 0.0 };
        // sugar_cap 10, fat_cap 100: add 14 -> 10 sugar, 4 overflow -> 4*STORE_LOSS to fat, no waste
        let wasted = e.add_sugar(14.0, 10.0, 100.0);
        assert!((e.sugar - 10.0).abs() < 1e-6);
        assert!((e.fat - 4.0 * STORE_LOSS).abs() < 1e-6);
        assert!(wasted < 1e-6);
    }

    #[test]
    fn full_stores_waste_excess() {
        let mut e = Energy { fast: 0.0, sugar: 10.0, fat: 2.0 };
        // sugar full (cap 10), fat cap 2 (full) -> all 6 wasted back as intake-equivalent
        let wasted = e.add_sugar(6.0, 10.0, 2.0);
        assert!((wasted - 6.0).abs() < 1e-6);
    }
}
