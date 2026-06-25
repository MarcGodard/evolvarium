// ECS components, M1 foraging sim.
use bevy::prelude::*;

#[derive(Component)]
pub struct Creature;

#[derive(Component)]
pub struct Food;

// Tree marker, long-lived plant. edible=true: tall fruit tree, only TALL creatures (sensors >= TREE_REACH)
// reach + eat. edible=false: evergreen, NOTHING eats (pure structure/refuge). Moisture-immune, grows large,
// reproduces slow into own kind. See BACKLOG trees.
#[derive(Component, Clone, Copy)]
pub struct Tree {
    pub edible: bool,
}

// Grass marker, lesser ground plant. Shares PlantGenome/PlantState/Food (eat path treats like any plant)
// but own cap + lifecycle (grass_step). Lesser = one nutrient, low energy, ~0 defense, ~0 height, high
// regrow (turf survives grazing). Ubiquitous on plant-capable soil.
#[derive(Component)]
pub struct Grass;

// Seaweed/kelp marker, OCEAN analog of grass. Same PlantGenome/PlantState, own cap + lifecycle
// (seaweed_step) + own visuals (add_seaweed_visuals: tall swaying fronds). Submerged (anchored to seafloor,
// above abyssal floor), feeds swimmers via position-based ocean graze. Reliable balance-isolated sea food
// carpet, never goes through fragile plant_step lifecycle.
#[derive(Component)]
pub struct Seaweed;

// Carrion marker + rot clock. Dead creature drops carrion: fresh = rich meat (no defense). As `age` climbs,
// nutrition fades + toxin rises -> rotten meat poisons eater (BACKLOG P3). Despawns once fully decomposed.
// Marks carrion vs living plants (no Rot).
#[derive(Component)]
pub struct Rot {
    pub age: u32,
}

// Plant-matter food that FERMENTS instead of rotting into meat: fallen fruit + dead-plant detritus.
// Shares Rot clock. Fresh -> sugar; mid-age fermentation window -> FAST energy (ethanol) + toxicity;
// over-rotted -> spoiled/gone. `toxic` scales toxicity (fruit low, detritus high). Presence tells eat
// dispatch: plant matter (-> fast/sugar) vs animal carrion (-> fat).
#[derive(Component)]
pub struct Ferment {
    pub toxic: f32,
}

// Viable seed a fallen fruit carries: PARENT'S FULL genome. Fruit's own Food genome is height/defense-zeroed
// (renders flat on ground, anyone eats), so real genes ride here. Creature eats RIPE fruit (past RIPEN_FRAC)
// -> this genome planted nearby (animal dispersal). Eaten unripe -> seed not viable, no Seed to plant (no
// reproduction).
#[derive(Component)]
pub struct Seed(pub crate::plant::PlantGenome);

// Three energy stores, each a burn/storage trade-off (no free lunch):
//   fast  = fermented-fruit/ethanol: burned FIRST + leaks at rest, tiny cap -> can't bank, quick power.
//   sugar = staple (plants deliver ONLY sugar): medium burn, medium cap.
//   fat   = the bank: burned LAST + mobilizes SLOW (low power output), big cap, adds upkeep.
// Burn order fast->sugar->fat. Death when total() <= 0. Refilled by eating (routed by food source).
use crate::config::{FAT_POWER, STORE_LOSS};
#[derive(Component)]
pub struct Energy {
    pub fast: f32,
    pub sugar: f32,
    pub fat: f32,
}

// Start split of total energy across stores (sums to 1.0). Lean: most sugar (staple), small fast buffer,
// modest fat reserve.
const SPLIT_FAST: f32 = 0.13;
const SPLIT_SUGAR: f32 = 0.47;
const SPLIT_FAT: f32 = 0.40;

impl Energy {
    pub fn total(&self) -> f32 {
        self.fast + self.sugar + self.fat
    }

    // Build stores from total budget (spawn/birth/gen-reset).
    pub fn from_total(t: f32) -> Self {
        Energy { fast: t * SPLIT_FAST, sugar: t * SPLIT_SUGAR, fat: t * SPLIT_FAT }
    }

    // Drain `cost` fast->sugar->fat. Returns shortfall (uncovered -> starving).
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

    // Instant power: fat mobilizes SLOW (FAT_POWER<1), fat-only creature can't sprint. Caps thrust.
    pub fn power(&self) -> f32 {
        self.fast + self.sugar + FAT_POWER * self.fat
    }

    // Add to fast, capped. Returns wasted excess.
    pub fn add_fast(&mut self, amt: f32, cap: f32) -> f32 {
        let room = (cap - self.fast).max(0.0);
        let take = amt.min(room);
        self.fast += take;
        amt - take
    }

    // Add to fat, capped. Returns wasted excess.
    pub fn add_fat(&mut self, amt: f32, cap: f32) -> f32 {
        let room = (cap - self.fat).max(0.0);
        let take = amt.min(room);
        self.fat += take;
        amt - take
    }

    // Add to sugar; overflow above sugar_cap converts to fat at STORE_LOSS (storing lossy). Returns wasted
    // excess in intake-equivalent units (-> growth-load via OVEREAT_G).
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

// Food eaten this gen = selection fitness.
#[derive(Component)]
pub struct Fitness(pub f32);

// Facing angle (radians), x-z plane. NN turn output rotates it.
#[derive(Component)]
pub struct Heading(pub f32);

// Starved creatures stop acting, keep fitness until gen end.
#[derive(Component)]
pub struct Alive(pub bool);

// Per-life working brain: weights start as copy of genome w0, learn during life (reward-gated Hebbian).
// Discarded at death, NOT inherited. GA selects genomes that LEARN well, not learned weights (Baldwin
// effect, see 04).
use crate::genome::Net;
#[derive(Component)]
pub struct Brain {
    pub net: Net, // working copy of genome weights, tuned by lifetime learning
    pub prev_dist: f32, // dist to nearest food last tick, approach-reward shaping
    // combat intents stashed each tick in live_step (out[2]/out[3]), predation_step reads same-tick.
    pub attack: f32,  // NN attack intent 0..1: hunts above ATTACK_INTENT_THRESH
    pub defend: f32,  // NN brace intent 0..1: raises defense, costs mobility
    pub voice: f32,   // NN call intensity 0..1 (out[7]) stashed this tick -> read by next tick's hearing snapshot + audio
    // (1-tick delay, same pattern as prev_dist/fight_reward).
    // pending combat reward set by predation_step (kill/defend/whiff), consumed + cleared next live_step
    // learn() call. 1-tick delay, same pattern as prev_dist.
    pub fight_reward: f32,
}

// Per-life locomotion diagnostic: birth pos + total path walked. Measures roaming vs circling: net
// displacement/path ratio ~1 = straight rover, ~0 = spinning in place.
#[derive(Component)]
pub struct Locomotion {
    pub start: Vec3,
    pub path: f32,
    pub alt: f32, // vertical offset above terrain (surface-offset units, >=0). Fliers: height in sky.
                  // Swimmers: rise off seafloor through water column toward waterline. Unified axis (out[6]).
}

// Per-life diet state. reserves[i] = pool of nutrient i (topped by eating x uptake, depleted each tick).
// master expression gene (genome::master_expression) reads reserves vs uptake-gene demand -> gates energy
// extracted from food. Low reserves on demanded nutrient = deficiency -> growth-load (soft-gradient
// disease). g = accumulated growth-signaling load (mismatch + deficiency + toxin -> disease). age in ticks
// (aging hazard). fatigue = exertion debt 0..1 (rest instinct).
use crate::genome::NUTRIENTS;
#[derive(Component)]
pub struct DietState {
    pub reserves: [f32; NUTRIENTS],
    pub g: f32,
    pub age: u32,
    pub fatigue: f32,
    // consecutive ticks pinned below starvation floor. Creature that can't pay metabolism clamps stores to 0
    // (burn shortfall discarded); grass trickle lifts a hair above 0 each tick, dodging <=0 death check
    // forever ("grass zombie"). Counter kills chronically near-empty creature after grace period; forager
    // that briefly dips to 0 between meals recovers + resets.
    pub starve: u16,
    // accumulated TOXIC LOAD (M4): ingested toxins (toxic plants, rotten meat, fermented spoilage, venomous
    // prey) + metabolic ammonia from protein-without-carbs (rabbit starvation) build here, not instant energy
    // hit. Drains energy + drives disease + death hazard while high; cleared slow each tick (faster w/ detox
    // gene). Poisons accumulate + linger like real toxic load.
    pub toxic_load: f32,
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
        // burn 10 -> drains rest (1 sugar + 5 fat = 6), shortfall 4
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
