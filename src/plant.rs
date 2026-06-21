// Living food: plants are organisms with their own genome + continuous GA (see 13).
// They grow mass (autotroph), reproduce by dispersing mutated offspring, evolve, and die when eaten.
// Selection is implicit: a plant that reproduces before being eaten passes on its genes.
// Arms race: plant `defense` vs creature `bite` decides whether a contact actually consumes it.
use crate::genome::{NFOOD, NUTRIENTS};
use crate::rng::Rng;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const PLANT_CAP: usize = 1400; // carrying capacity (bounds population), scaled to the doubled arena
pub const PLANT_MIN: usize = 140; // reseed floor so the food web can't fully collapse
pub const P_REPRO: f32 = 0.015; // per-tick reproduction chance for a mature plant
const GROWTH_BASE: f32 = 1.2; // mass/sec at full growth allocation

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct PlantGenome {
    pub kind: u8,      // food/diet type (couples to creature expression, see 12)
    pub nutrient: f32, // 0..1 energy density delivered when eaten
    pub defense: f32,  // 0..1 resistance to being eaten (vs creature bite)
    pub quality: f32,  // 0..1 digestibility: scales energy the eater extracts AND seed-dispersal-on-eat (13)
    pub wet: f32,      // 0..1 preferred soil moisture; mismatch with local moisture stresses->kills (P3)
    #[serde(default)]
    pub height: f32,   // 0..1 plant height: short creatures can't reach tall plants (reach defense); costs growth
    #[serde(default = "half_light")]
    pub light_pref: f32, // 0=shade .. 1=full sun: growth peaks at preferred light (day/night + biome niche)
    #[serde(default)]
    pub regrow: f32,   // 0=consumed whole when eaten (carrot) .. 1=survives small bites + regrows (berry bush)
    #[serde(default)]
    pub branches: f32, // 0..1 (trees): branches let SHORT creatures reach a tall tree AND feed harmlessly
                       // (no mass damage); costs growth. Trade-off vs growing tall + bare.
    pub spread: f32,   // offspring dispersal distance
    pub maturity: f32, // mass needed before it can reproduce
    #[serde(default = "default_nutrients")]
    pub nutrients: [f32; NUTRIENTS], // genetic baseline production per nutrient (sparse: a plant is rich in a few).
                                     // Effective output at eat-time = this x soil fertility (richer ground -> more).
    #[serde(default)]
    pub toxicity: f32, // 0..1 genetic toxin load: deters eaters (energy hit + growth-load) but costs growth.
}

// serde default for nutrients on old saves: a flat mid spread (a generic plant feeds every nutrient a little).
fn default_nutrients() -> [f32; NUTRIENTS] {
    [0.3; NUTRIENTS]
}

// serde default for light_pref on old saves: mid-light
fn half_light() -> f32 {
    0.5
}

// Per-plant state: mass grows over life; eaten plants are despawned.
#[derive(Component)]
pub struct PlantState {
    pub mass: f32,
    pub age: u32,
}

impl PlantGenome {
    pub fn random(rng: &mut Rng, ntypes: u8) -> Self {
        // nutrient profile: sparse -> each plant species is rich in a few nutrients, scarce in others, so
        // no single food is nutritionally complete -> eaters must vary their diet (drives the food web).
        let mut nutrients = [0.0f32; NUTRIENTS];
        for n in nutrients.iter_mut() {
            *n = if rng.f32() < 0.35 { rng.range(0.4, 1.0) } else { rng.f32() * 0.2 };
        }
        PlantGenome {
            kind: ((rng.f32() * ntypes as f32) as u8).min(ntypes.saturating_sub(1)),
            nutrient: rng.f32(),
            defense: rng.f32() * 0.5,
            quality: rng.f32(),
            wet: rng.f32(),
            height: rng.f32() * 0.4,
            light_pref: rng.f32(),
            regrow: rng.f32(),
            branches: rng.f32() * 0.1, // ground plants barely branch; trees set this high (tree_genome)
            spread: rng.range(2.0, 8.0),
            maturity: rng.range(2.0, 6.0),
            nutrients,
            toxicity: rng.f32() * 0.3, // most plants mildly toxic; evolves up as a defense (costs growth)
        }
    }

    // Lesser ground plant (grass). One nutrient, low energy density, defenseless, ~flat, full-sun, high
    // regrow (turf survives grazing), fast maturity. Cheap fallback food; blankets plant-capable soil.
    pub fn grass(rng: &mut Rng) -> Self {
        // exactly ONE nutrient axis populated -> "fewer nutrient types" than a normal plant (~3-4).
        let mut nutrients = [0.0f32; NUTRIENTS];
        nutrients[(rng.f32() * NUTRIENTS as f32) as usize % NUTRIENTS] = rng.range(0.4, 0.6);
        PlantGenome {
            kind: 0, // green family
            nutrient: rng.range(0.2, 0.3), // low energy density
            defense: 0.0,                  // defenseless: trivial to graze
            quality: rng.range(0.4, 0.6),
            wet: rng.range(0.3, 0.7), // tolerant of most non-extreme soil moisture
            height: 0.02,             // flat: any creature reaches it
            light_pref: 0.85,         // sun-loving turf
            regrow: 0.9,              // survives small bites + regrows (turf, not consumed whole)
            branches: 0.0,
            spread: 2.5,
            maturity: 1.0, // matures fast
            nutrients,
            toxicity: 0.0,
        }
    }

    // Tree-specific evolution: trees evolve like plants but with BIGGER ranges + tree-relevant genes.
    // kind + defense stay pinned (tree identity); height capped at 1.0 (never taller than today's max).
    pub fn mutate_tree(&mut self, rng: &mut Rng) {
        self.nutrient = (self.nutrient + rng.normal() * 0.12).clamp(0.0, 1.0);
        self.quality = (self.quality + rng.normal() * 0.12).clamp(0.0, 1.0);
        self.height = (self.height + rng.normal() * 0.12).clamp(0.0, 1.0); // wide drift, max 1.0
        self.light_pref = (self.light_pref + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.regrow = (self.regrow + rng.normal() * 0.12).clamp(0.0, 1.0);
        self.branches = (self.branches + rng.normal() * 0.12).clamp(0.0, 1.0);
        self.spread = (self.spread + rng.normal() * 1.5).clamp(3.0, 16.0); // bigger dispersal range
        self.maturity = (self.maturity + rng.normal() * 1.5).clamp(8.0, 26.0); // trees stay large
        for n in &mut self.nutrients {
            *n = (*n + rng.normal() * 0.1).clamp(0.0, 1.0);
        }
        self.toxicity = (self.toxicity + rng.normal() * 0.08).clamp(0.0, 1.0);
    }

    pub fn mutate(&mut self, rng: &mut Rng) {
        // kind rarely flips; trait values drift
        if rng.f32() < 0.03 {
            self.kind ^= if rng.f32() < 0.5 { 0 } else { 1 };
        }
        self.nutrient = (self.nutrient + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.defense = (self.defense + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.quality = (self.quality + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.wet = (self.wet + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.height = (self.height + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.light_pref = (self.light_pref + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.regrow = (self.regrow + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.branches = (self.branches + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.spread = (self.spread + rng.normal() * 1.0).clamp(1.0, 12.0);
        self.maturity = (self.maturity + rng.normal() * 0.8).clamp(1.5, 10.0);
        for n in &mut self.nutrients {
            *n = (*n + rng.normal() * 0.1).clamp(0.0, 1.0);
        }
        self.toxicity = (self.toxicity + rng.normal() * 0.08).clamp(0.0, 1.0);
    }

    // Investing in nutrient richness, defense, and digestible quality slows growth (no free lunch, 10).
    // Defense penalty is QUADRATIC: cheap when light, crippling when maxed -> bounds the arms race so
    // plants can't armor up to ~1.0 for free (balance lever, iter 1). Quality (palatable soft tissue)
    // costs growth too; its payoff is dispersal-on-eat (13), so quality reaches an interior optimum.
    pub fn growth_rate(&self) -> f32 {
        // producing nutrients + toxins costs growth (no free lunch). Mean nutrient richness taxes growth;
        // toxicity is QUADRATIC (cheap when light, dear when maxed) so chemical defense can't peg for free.
        let mean_nutri = self.nutrients.iter().sum::<f32>() / NUTRIENTS as f32;
        GROWTH_BASE
            * (1.0_f32
                - 0.3 * self.nutrient
                - 0.85 * self.defense * self.defense
                - 0.2 * self.quality
                - 0.25 * self.height
                - 0.15 * self.regrow
                - 0.2 * self.branches
                - 0.18 * mean_nutri
                - 0.3 * self.toxicity * self.toxicity)
                .clamp(0.12, 1.0)
    }
}

// Plant render color encodes its genome: hue by kind, but tinted by nutrient (brighter = richer)
// and defense (toward red = tougher). Makes food variability visible at a glance.
pub fn plant_color(g: &PlantGenome) -> Color {
    let base_hue = match g.kind as usize % NFOOD {
        0 => 130.0,
        1 => 285.0,
        2 => 45.0,
        _ => 190.0,
    };
    let hue = base_hue - 40.0 * g.defense + 60.0 * g.toxicity; // tough -> warmer/red; toxic -> shifts purple (warning)
    let light = 0.35 + 0.35 * g.nutrient; // richer plants brighter
    let sat = 0.35 + 0.55 * g.quality; // tastier (digestible) plants more vivid; tough/fibrous = washed out
    Color::hsl(hue, sat, light)
}
