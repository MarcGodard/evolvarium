// Living food: plants = organisms w/ own genome + continuous GA (spec 13). Autotroph: grow mass, reproduce
// via mutated offspring, evolve, die when eaten. Selection implicit: reproduce-before-eaten passes genes.
// Arms race: plant `defense` vs creature `bite` decides whether contact consumes it.
use crate::genome::{NFOOD, NUTRIENTS};
use crate::rng::Rng;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const PLANT_CAP: usize = 4000; // carrying cap: dense whole-planet food so scattered creatures find food
                                   // where they land. Bounded for perf: plant_step (+ mating search) scales w/ this/tick.
pub const WHOLE_PLANET_SEED_MULT: usize = 20; // loaded/diverse world seeds 20x founding FOOD count -> establishes
                                              // dense everywhere (many land in survivable biomes)
pub const PLANT_MIN: usize = 140; // reseed floor: food web can't fully collapse
pub const P_REPRO: f32 = 0.015; // per-tick reproduction chance, mature plant
const GROWTH_BASE: f32 = 1.2; // mass/sec at full growth allocation

// Plant growth FORM (silhouette) -> render mesh; world reads as botanical mix not identical balls. Visual
// identity per lineage (stable: doesn't drift on mutate). Aquatic forms (LILYPAD/KELP) render at/under
// waterline. Meshes in viz::PlantForms.
pub mod form {
    pub const HERB: u8 = 0; // generic default: small leafy plant
    pub const SHRUB: u8 = 1; // rounded woody bush
    pub const GROUNDCOVER: u8 = 2; // low spreading mat (clover, creeping herbs)
    pub const FERN: u8 = 3; // arching fronds (shade, moist)
    pub const SUCCULENT: u8 = 4; // fat ribbed body (cactus/aloe, desert)
    pub const REED: u8 = 5; // tall thin stalks (wetland edge)
    pub const FLOWER_STALK: u8 = 6; // slim stem + bloom (wildflower)
    pub const ROSETTE: u8 = 7; // low radial leaves (agave, lettuce, thistle)
    pub const LILYPAD: u8 = 8; // flat floating disc, water surface
    pub const KELP: u8 = 9; // tall swaying fronds, deep + submerged
    pub const MUSHROOM: u8 = 10; // cap on stem (fungus, detritus)
    pub const MOSS: u8 = 11; // very low ground mat (deep shade)
    pub const COUNT: u8 = 12;
}

// Founding species presets: each = recognizable real-world plant (sensible gene combo), then evolves.
// sim::plant_for_site picks one per biome (temp, moisture, water depth, rockiness).
#[derive(Clone, Copy)]
pub enum Archetype {
    Clover,       // groundcover legume: low, edible, nitrogen-fixing, fast
    Wildflower,   // flowering herb: sweet, blooms, mid
    BerryBush,    // shrub: fruiting, sweet, thorny, regrows after bite
    Fern,         // shade frond: low light, moist, no flower
    Cactus,       // desert succulent: drought-buffered, spiny, slow, warm
    Reed,         // wetland edge: tall, wet-loving, emergent
    Thistle,      // weed: bitter/toxic, thorny rosette, hardy + allelopathic
    Nightshade,   // toxic berries: poisonous fruit (sweet-looking, bitter)
    Moss,         // deep-shade mat: tiny, moist, cool
    AlpineCushion,// cold-niche cushion: tiny dense, cold-tolerant
    Tumbleweed,   // arid weed: dry, hardy, fire-adapted, allelopathic
    Waterlily,    // surface aquatic: full sun, floating pad + bloom
    Eelgrass,     // shallow submerged seagrass: mid light
    Kelp,         // deep submerged: NEEDS LESS SUN (shade), tall fronds
    AlgaeMat,     // surface algae film: full sun, fast, low
}

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct PlantGenome {
    pub kind: u8,      // food/diet type. Couples to creature expression (spec 12).
    pub nutrient: f32, // 0..1 energy density delivered when eaten
    pub defense: f32,  // 0..1 resistance to being eaten (vs creature bite)
    pub quality: f32,  // 0..1 digestibility: scales energy eater extracts AND seed-dispersal-on-eat (spec 13)
    pub wet: f32,      // 0..1 preferred soil moisture. Mismatch w/ local moisture stresses -> kills (P3).
    #[serde(default)]
    pub height: f32,   // 0..1 height: short creatures can't reach tall plants (reach defense). Costs growth.
    #[serde(default = "half_light")]
    pub light_pref: f32, // 0=shade .. 1=full sun. Growth peaks at preferred light (day/night + biome niche).
    #[serde(default)]
    pub regrow: f32,   // 0=consumed whole (carrot) .. 1=survives small bites + regrows (berry bush)
    #[serde(default)]
    pub branches: f32, // 0..1 (trees): let SHORT creatures reach tall tree AND feed harmlessly (no mass
                       // damage). Costs growth. Trade-off vs growing tall + bare.
    pub spread: f32,   // offspring dispersal distance
    pub maturity: f32, // mass needed before reproduce
    #[serde(default = "default_nutrients")]
    pub nutrients: [f32; NUTRIENTS], // genetic baseline production per nutrient (sparse: rich in a few).
                                     // Effective output at eat-time = this x soil fertility (richer ground -> more).
    #[serde(default)]
    pub toxicity: f32, // 0..1 toxin load of PLANT BODY: deters grazers (energy hit + growth-load). Costs growth.
    #[serde(default)]
    pub fruit_toxicity: f32, // 0..1 toxin load of UNRIPE fruit (separate from body toxicity). High = protects
                             // developing seed (eaten-early fruit poisons + yields little). RIPENING detoxifies
                             // -> ripe fruit sweet + disperses seed. Costs a little growth.

    // --- climate + survival genes (affect sim AND visuals) ---
    #[serde(default = "half_light")]
    pub temp_pref: f32, // 0 cold/alpine .. 1 warm/tropical: climate niche (mirror of creature temp_pref). Off-niche = slow growth + soft death.
    #[serde(default)]
    pub succulence: f32, // 0..1 water storage: buffers DROUGHT (survives dry/desert), costs growth (cactus/aloe).
    #[serde(default)]
    pub submerged: f32, // 0 surface/emergent .. 1 deep submerged: render depth + minor self-shade (lily -> kelp).
    #[serde(default)]
    pub fruiting: f32, // 0..1 non-tree fruit-bearing: drops fallen fruit like tree (berry bush), costs growth.

    // --- ecology genes (cheap, no neighbor scans; affect sim AND visuals) ---
    #[serde(default)]
    pub nitrogen_fix: f32, // 0..1 legume: enriches local soil fertility each tick (clover/beans), costs growth.
    #[serde(default)]
    pub fire_seed: f32, // 0..1 serotiny: on burn-up releases seeds (post-fire recruitment), costs a little growth.
    #[serde(default)]
    pub climb: f32, // 0..1 vine: climbs to light (light-factor bonus) w/o paying height; weak structure, costs growth.
    #[serde(default)]
    pub allelopathy: f32, // 0..1 chemical warfare: litter (detritus) extra toxic (suppresses competitors), costs growth.

    // --- dispersal / reproduction genes (how offspring spread; affect sim, cheap no neighbor scans) ---
    #[serde(default)]
    pub seed_weight: f32, // 0 light/dust .. 1 heavy provisioned. Heavy disperses SHORT (drops near parent, oak
                          // acorn) but seedling establishes BIGGER + hardier; light flies far (windborne) but
                          // starts tiny. Provisioning costs growth.
    #[serde(default)]
    pub windborne: f32,   // 0..1 wind dispersal (pappus/wings/dust): extends seed range, strongest on light
                          // seed (dandelion, maple samara, tumbleweed). Costs growth.
    #[serde(default)]
    pub clonal: f32,      // 0..1 vegetative spread (rhizome/runner/sucker): near-identical ramets beside itself
                          // -> dense local patch WITHOUT seeding (strawberry, aspen). No long dispersal, no gene
                          // shuffle (true clone). Costs growth.
    #[serde(default)]
    pub cling: f32,       // 0..1 epizoochory (burrs/hooks/sticky seed): passing animal snags seed + carries FAR
                          // even though plant never eaten -> defended/toxic/inedible plants still get animal
                          // dispersal (burdock, cleavers). Costs growth.
    #[serde(default)]
    pub dormancy: f32,    // 0..1 seed bank: fraction of seeds wait DORMANT in soil + germinate later, not now.
                          // Outlasts fire/drought/grazing that clears surface -> wiped patch re-greens from
                          // buried bank (boom after disturbance). Costs growth.
    #[serde(default)]
    pub hydrochory: f32,  // 0..1 water dispersal (buoyant/corky seed): seed from plant AT/NEAR water floats +
                          // rides far (coconut, mangrove propagule). Only helps where water to float on; inland
                          // does nothing. Costs growth.

    // --- visual-only genes (cosmetic; zero sim cost) ---
    #[serde(default = "default_form")]
    pub form: u8, // PlantForm silhouette -> render mesh (see plant::form). Visual identity; stable per lineage.
    #[serde(default)]
    pub flower: f32, // 0..1 bloom presence/size (render: colored bloom child)
    #[serde(default = "half_light")]
    pub flower_hue: f32, // 0..1 petal hue (x360). Spans full color wheel.
    #[serde(default = "half_light")]
    pub flower_sat: f32, // 0..1 petal saturation -> mapped to BRIGHT range (vivid..pure), never dull/grey.
    #[serde(default = "half_light")]
    pub flower_light: f32, // 0..1 petal lightness -> mapped to BRIGHT range (rich..pale), never dark.
    #[serde(default = "half_light")]
    pub leaf_hue: f32, // 0..1 foliage hue jitter (variety on top of family hue)
    #[serde(default = "half_light")]
    pub bushiness: f32, // 0..1 clump fullness (render: girth/fullness)
    #[serde(default)]
    pub droop: f32, // 0..1 upright .. weeping arch (render)
}

// serde default, old saves: generic small herb.
fn default_form() -> u8 {
    form::HERB
}

// serde default, old saves: flat mid spread (generic plant feeds every nutrient a little).
fn default_nutrients() -> [f32; NUTRIENTS] {
    [0.3; NUTRIENTS]
}

// serde default, old saves: mid-light. Shared by several genes.
fn half_light() -> f32 {
    0.5
}

// Per-plant state: mass grows over life; eaten plants despawned.
#[derive(Component)]
pub struct PlantState {
    pub mass: f32,
    pub age: u32,
}

impl PlantGenome {
    pub fn random(rng: &mut Rng, ntypes: u8) -> Self {
        // nutrient profile sparse: rich in a few, scarce in rest -> no single food complete -> eaters must
        // vary diet (drives food web).
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
            branches: rng.f32() * 0.1, // ground plants barely branch; trees set high (tree_genome)
            spread: rng.range(2.0, 8.0),
            maturity: rng.range(2.0, 6.0),
            nutrients,
            toxicity: rng.f32() * 0.3, // most plants mildly toxic; evolves up as defense (costs growth)
            fruit_toxicity: rng.f32() * 0.4, // unripe-fruit protection; evolves to interior optimum
            temp_pref: rng.f32(),
            succulence: rng.f32() * 0.3,
            submerged: 0.0,
            fruiting: rng.f32() * 0.3,
            nitrogen_fix: rng.f32() * 0.2,
            fire_seed: rng.f32() * 0.2,
            climb: rng.f32() * 0.2,
            allelopathy: rng.f32() * 0.2,
            seed_weight: rng.f32() * 0.5,
            windborne: rng.f32() * 0.4,
            clonal: rng.f32() * 0.3,
            cling: rng.f32() * 0.3,
            dormancy: rng.f32() * 0.4,
            hydrochory: rng.f32() * 0.2,
            // random fallback: land form (HERB..ROSETTE), never aquatic/special
            form: (rng.f32() * 8.0) as u8 % 8,
            flower: rng.f32() * 0.5,
            flower_hue: rng.f32(),
            flower_sat: rng.f32(),
            flower_light: rng.f32(),
            leaf_hue: rng.f32(),
            bushiness: rng.f32(),
            droop: rng.f32() * 0.3,
        }
    }

    // Sparse nutrient profile: n_rich axes get high value, rest stay 0 -> no food complete (drives diet
    // variety). Helper for species archetypes.
    fn sparse_nutrients(rng: &mut Rng, n_rich: usize) -> [f32; NUTRIENTS] {
        let mut a = [0.0f32; NUTRIENTS];
        for _ in 0..n_rich {
            let i = (rng.f32() * NUTRIENTS as f32) as usize % NUTRIENTS;
            a[i] = rng.range(0.5, 1.0);
        }
        a
    }

    // Build founding plant from species preset, then evolve. random() = neutral defaults, then override
    // genes that DEFINE species (real-life grounded combos).
    pub fn archetype(rng: &mut Rng, a: Archetype) -> Self {
        let mut g = Self::random(rng, NFOOD as u8);
        // visual defaults; each arm tweaks
        g.submerged = 0.0;
        g.flower = 0.0;
        g.fruiting = 0.0;
        match a {
            Archetype::Clover => {
                g.kind = 0; // green family
                g.nutrient = rng.range(0.35, 0.55);
                g.defense = 0.05;
                g.quality = rng.range(0.5, 0.8);
                g.wet = rng.range(0.4, 0.7);
                g.height = 0.05;
                g.light_pref = rng.range(0.6, 0.85);
                g.regrow = 0.7; // grazed lawn legume regrows
                g.maturity = rng.range(1.5, 3.0);
                g.toxicity = 0.0;
                g.nitrogen_fix = rng.range(0.6, 0.95); // legume enriches soil
                g.clonal = rng.range(0.5, 0.8); // creeping runners: clover spreads as clonal mat
                g.nutrients = Self::sparse_nutrients(rng, 3);
                g.form = form::GROUNDCOVER;
                g.leaf_hue = rng.range(0.30, 0.40); // green
                g.flower = rng.range(0.1, 0.3); // small white-pink clover heads
                g.flower_hue = rng.range(0.85, 0.98);
            }
            Archetype::Wildflower => {
                g.kind = 2; // warm/yellow family
                g.nutrient = rng.range(0.4, 0.7);
                g.defense = 0.05;
                g.quality = rng.range(0.6, 0.9); // sweet nectar, palatable
                g.wet = rng.range(0.4, 0.7);
                g.height = rng.range(0.2, 0.4);
                g.light_pref = rng.range(0.6, 0.9);
                g.maturity = rng.range(2.0, 4.0);
                g.toxicity = rng.f32() * 0.1;
                g.seed_weight = rng.range(0.0, 0.2); // light seed
                g.windborne = rng.range(0.4, 0.7);   // wind-blown
                g.nutrients = Self::sparse_nutrients(rng, 3);
                g.form = form::FLOWER_STALK;
                g.leaf_hue = rng.range(0.28, 0.38);
                g.flower = rng.range(0.6, 1.0); // big showy bloom
                g.flower_hue = rng.f32(); // any petal color
            }
            Archetype::BerryBush => {
                g.kind = 1; // purple/berry family
                g.nutrient = rng.range(0.6, 0.9);
                g.defense = rng.range(0.3, 0.6); // thorns
                g.quality = rng.range(0.6, 0.9);
                g.wet = rng.range(0.4, 0.7);
                g.height = rng.range(0.3, 0.6);
                g.light_pref = rng.range(0.5, 0.8);
                g.regrow = rng.range(0.7, 0.95); // survives bite, regrows
                g.maturity = rng.range(3.0, 6.0);
                g.toxicity = rng.f32() * 0.1;
                g.fruiting = rng.range(0.6, 1.0); // bears berries (fallen fruit -> fast energy)
                g.nutrients = Self::sparse_nutrients(rng, 4);
                g.form = form::SHRUB;
                g.leaf_hue = rng.range(0.30, 0.42);
                g.flower = rng.range(0.2, 0.4);
                g.flower_hue = rng.range(0.85, 0.98);
            }
            Archetype::Fern => {
                g.kind = 0;
                g.nutrient = rng.range(0.3, 0.5);
                g.defense = rng.range(0.1, 0.3);
                g.quality = rng.range(0.3, 0.6);
                g.wet = rng.range(0.6, 0.9); // moist understory
                g.height = rng.range(0.2, 0.45);
                g.light_pref = rng.range(0.1, 0.35); // SHADE: thrives in low light
                g.maturity = rng.range(2.5, 5.0);
                g.toxicity = rng.f32() * 0.15;
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::FERN;
                g.leaf_hue = rng.range(0.33, 0.45); // deep green
                g.flower = 0.0; // ferns don't flower
            }
            Archetype::Cactus => {
                g.kind = 3;
                g.nutrient = rng.range(0.4, 0.7);
                g.defense = rng.range(0.5, 0.8); // spines
                g.quality = rng.range(0.2, 0.4);
                g.wet = rng.range(0.05, 0.25); // desert: DRY
                g.height = rng.range(0.2, 0.5);
                g.light_pref = rng.range(0.8, 1.0); // full blazing sun
                g.temp_pref = rng.range(0.6, 0.9); // hot
                g.maturity = rng.range(4.0, 8.0);
                g.toxicity = rng.f32() * 0.2;
                g.succulence = rng.range(0.7, 1.0); // water storage: survives drought
                g.seed_weight = rng.range(0.4, 0.7); // heavy seed: drops near parent (no wind in open desert)
                g.dormancy = rng.range(0.6, 0.9); // desert seed bank: waits dormant for rare rain
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::SUCCULENT;
                g.leaf_hue = rng.range(0.40, 0.52); // blue-green
                g.flower = rng.range(0.0, 0.5); // occasional cactus bloom
                g.flower_hue = rng.range(0.0, 0.15); // red/yellow
            }
            Archetype::Reed => {
                g.kind = 0;
                g.nutrient = rng.range(0.3, 0.5);
                g.defense = rng.range(0.1, 0.3);
                g.quality = rng.range(0.3, 0.5);
                g.wet = rng.range(0.75, 1.0); // wetland edge
                g.height = rng.range(0.6, 0.95); // tall stalks
                g.light_pref = rng.range(0.6, 0.9);
                g.maturity = rng.range(3.0, 6.0);
                g.hydrochory = rng.range(0.4, 0.7); // wetland edge: seeds float off on water
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::REED;
                g.leaf_hue = rng.range(0.25, 0.35);
            }
            Archetype::Thistle => {
                g.kind = 1;
                g.nutrient = rng.range(0.3, 0.5);
                g.defense = rng.range(0.5, 0.85); // very thorny
                g.quality = rng.range(0.1, 0.3);
                g.wet = rng.range(0.3, 0.6);
                g.height = rng.range(0.2, 0.45);
                g.light_pref = rng.range(0.6, 0.9);
                g.maturity = rng.range(2.0, 4.0);
                g.toxicity = rng.range(0.4, 0.7); // bitter
                g.allelopathy = rng.range(0.4, 0.7);
                g.windborne = rng.range(0.5, 0.8); // thistledown: parachute seeds fly far
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::ROSETTE;
                g.leaf_hue = rng.range(0.30, 0.42);
                g.flower = rng.range(0.4, 0.7);
                g.flower_hue = rng.range(0.78, 0.9); // purple thistle head
            }
            Archetype::Nightshade => {
                g.kind = 1;
                g.nutrient = rng.range(0.5, 0.8); // looks rich
                g.defense = rng.range(0.1, 0.3);
                g.quality = rng.range(0.5, 0.8); // tempting
                g.wet = rng.range(0.4, 0.7);
                g.height = rng.range(0.3, 0.6);
                g.light_pref = rng.range(0.4, 0.7);
                g.maturity = rng.range(3.0, 5.0);
                g.toxicity = rng.range(0.6, 0.95); // poisonous berries
                g.fruiting = rng.range(0.5, 0.9);
                g.nutrients = Self::sparse_nutrients(rng, 3);
                g.form = form::SHRUB;
                g.leaf_hue = rng.range(0.33, 0.45);
                g.flower = rng.range(0.3, 0.6);
                g.flower_hue = rng.range(0.7, 0.85); // deadly-nightshade purple
            }
            Archetype::Moss => {
                g.kind = 0;
                g.nutrient = rng.range(0.2, 0.35);
                g.defense = 0.0;
                g.quality = rng.range(0.3, 0.5);
                g.wet = rng.range(0.7, 1.0);
                g.height = 0.02;
                g.light_pref = rng.range(0.05, 0.3); // deep shade
                g.temp_pref = rng.range(0.3, 0.6);
                g.maturity = rng.range(1.5, 3.0);
                g.regrow = 0.85;
                g.clonal = rng.range(0.6, 0.9); // moss spreads as creeping clonal mat
                g.windborne = rng.range(0.4, 0.7); // tiny spores also drift
                g.nutrients = Self::sparse_nutrients(rng, 1);
                g.form = form::MOSS;
                g.leaf_hue = rng.range(0.33, 0.45);
            }
            Archetype::AlpineCushion => {
                g.kind = 0;
                g.nutrient = rng.range(0.3, 0.5);
                g.defense = rng.range(0.1, 0.3);
                g.quality = rng.range(0.3, 0.5);
                g.wet = rng.range(0.3, 0.6);
                g.height = 0.05; // hugs ground out of wind
                g.light_pref = rng.range(0.6, 0.9);
                g.temp_pref = rng.range(0.05, 0.25); // COLD niche
                g.maturity = rng.range(2.0, 4.0);
                g.dormancy = rng.range(0.5, 0.8); // alpine seed bank: seeds overwinter dormant under snow
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::GROUNDCOVER;
                g.bushiness = rng.range(0.7, 1.0); // dense cushion
                g.leaf_hue = rng.range(0.30, 0.45);
                g.flower = rng.range(0.2, 0.5);
                g.flower_hue = rng.f32();
            }
            Archetype::Tumbleweed => {
                g.kind = 2;
                g.nutrient = rng.range(0.25, 0.45);
                g.defense = rng.range(0.3, 0.6);
                g.quality = rng.range(0.15, 0.35);
                g.wet = rng.range(0.1, 0.3); // arid
                g.height = rng.range(0.3, 0.6);
                g.light_pref = rng.range(0.7, 1.0);
                g.temp_pref = rng.range(0.5, 0.85);
                g.maturity = rng.range(2.0, 4.0);
                g.succulence = rng.range(0.3, 0.6);
                g.fire_seed = rng.range(0.5, 0.9); // fire-adapted recruiter
                g.allelopathy = rng.range(0.3, 0.6);
                g.windborne = rng.range(0.6, 0.9); // whole dead plant tumbles + scatters seed downwind
                g.seed_weight = rng.range(0.0, 0.2);
                g.dormancy = rng.range(0.5, 0.8); // arid seed bank: waits out dry, booms after rain/fire
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::SHRUB;
                g.leaf_hue = rng.range(0.12, 0.22); // dry straw color
            }
            Archetype::Waterlily => {
                g.kind = 3;
                g.nutrient = rng.range(0.4, 0.7);
                g.defense = rng.range(0.05, 0.2);
                g.quality = rng.range(0.5, 0.8);
                g.wet = rng.range(0.9, 1.0); // aquatic
                g.height = 0.05;
                g.light_pref = rng.range(0.8, 1.0); // floats up to FULL sun
                g.submerged = 0.0; // sits ON surface
                g.maturity = rng.range(2.5, 5.0);
                g.hydrochory = rng.range(0.6, 0.9); // floating seeds drift across water
                g.nutrients = Self::sparse_nutrients(rng, 3);
                g.form = form::LILYPAD;
                g.leaf_hue = rng.range(0.30, 0.42);
                g.flower = rng.range(0.5, 0.9); // the lily bloom
                g.flower_hue = rng.range(0.85, 1.0);
            }
            Archetype::Eelgrass => {
                g.kind = 3;
                g.nutrient = rng.range(0.3, 0.5);
                g.defense = rng.range(0.05, 0.2);
                g.quality = rng.range(0.4, 0.6);
                g.wet = rng.range(0.9, 1.0);
                g.height = rng.range(0.4, 0.7);
                g.light_pref = rng.range(0.4, 0.6); // shallow submerged, mid light
                g.submerged = rng.range(0.4, 0.7);
                g.maturity = rng.range(2.5, 5.0);
                g.clonal = rng.range(0.5, 0.8); // seagrass spreads by rhizome into meadows
                g.hydrochory = rng.range(0.5, 0.8); // seeds drift on current
                g.nutrients = Self::sparse_nutrients(rng, 2);
                g.form = form::KELP; // ribbon fronds (shorter via height)
                g.leaf_hue = rng.range(0.28, 0.38);
            }
            Archetype::Kelp => {
                g.kind = 3;
                g.nutrient = rng.range(0.5, 0.8);
                g.defense = rng.range(0.1, 0.3);
                g.quality = rng.range(0.4, 0.7);
                g.wet = rng.range(0.95, 1.0);
                g.height = rng.range(0.7, 1.0); // tall fronds reach up column
                g.light_pref = rng.range(0.05, 0.3); // NEEDS LESS SUN: thrives in dim deep
                g.submerged = rng.range(0.7, 1.0); // deep
                g.maturity = rng.range(4.0, 7.0);
                g.hydrochory = rng.range(0.6, 0.9); // buoyant spores/fragments drift far on current
                g.nutrients = Self::sparse_nutrients(rng, 3);
                g.form = form::KELP;
                g.leaf_hue = rng.range(0.20, 0.32); // brown-green kelp
            }
            Archetype::AlgaeMat => {
                g.kind = 0;
                g.nutrient = rng.range(0.3, 0.5);
                g.defense = 0.0;
                g.quality = rng.range(0.4, 0.6);
                g.wet = rng.range(0.9, 1.0);
                g.height = 0.02;
                g.light_pref = rng.range(0.7, 1.0); // surface film, full sun
                g.submerged = rng.range(0.0, 0.2);
                g.maturity = rng.range(1.5, 3.0);
                g.regrow = 0.85;
                g.clonal = rng.range(0.6, 0.9); // algal film spreads clonally across surface
                g.hydrochory = rng.range(0.6, 0.9); // surface film rafts across water
                g.nutrients = Self::sparse_nutrients(rng, 1);
                g.form = form::MOSS; // flat mat
                g.leaf_hue = rng.range(0.33, 0.5);
            }
        }
        g
    }

    // Grass: lesser ground plant. One nutrient, low energy density, defenseless, ~flat, full-sun, high
    // regrow (turf survives grazing), fast maturity. Cheap fallback food; blankets plant-capable soil.
    pub fn grass(rng: &mut Rng) -> Self {
        // exactly ONE nutrient axis populated -> fewer nutrient types than normal plant (~3-4).
        let mut nutrients = [0.0f32; NUTRIENTS];
        nutrients[crate::config::GRASS_FORAGE_IDX] = rng.range(0.4, 0.6); // fixed forage axis: grazers tune uptake to it
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
            fruit_toxicity: 0.0, // grass doesn't fruit
            temp_pref: 0.5, // climate-tolerant turf
            succulence: 0.0,
            submerged: 0.0,
            fruiting: 0.0,
            nitrogen_fix: 0.0,
            fire_seed: 0.0,
            climb: 0.0,
            allelopathy: 0.0,
            seed_weight: 0.1,  // tiny light grass seed
            windborne: 0.3,    // blows a little
            clonal: 0.6,       // turf creeps + fills by runners (main way grass spreads)
            cling: 0.1,        // grass seed occasionally hitches ride
            dormancy: 0.5,     // grass seed bank: turf re-greens fast after fire/drought
            hydrochory: 0.0,   // land turf: no water dispersal
            form: form::GROUNDCOVER,
            flower: 0.0,
            flower_hue: 0.3,
            flower_sat: 0.7,
            flower_light: 0.6,
            leaf_hue: 0.33, // green
            bushiness: 0.5,
            droop: 0.0,
        }
    }

    // Seaweed/kelp: OCEAN analog of grass(). Aquatic (high wet + submerged -> underwater), dim-light tolerant
    // (deep), brown-green fronds. Own cap + step (seaweed_step), so most fields just = hardy submerged
    // groundcover; feeds swimmers via position-based ocean graze.
    pub fn seaweed(rng: &mut Rng) -> Self {
        let mut nutrients = [0.0f32; NUTRIENTS];
        nutrients[crate::config::SEAWEED_FORAGE_IDX] = rng.range(0.4, 0.6); // fixed kelp forage axis
        PlantGenome {
            kind: 0,
            nutrient: rng.range(0.25, 0.4), // touch richer than grass (kelp hearty)
            defense: 0.0,
            quality: rng.range(0.4, 0.6),
            wet: 1.0,        // fully aquatic
            height: 0.04,
            light_pref: 0.3, // dim-tolerant (below surface)
            regrow: 0.9,     // regrows after grazing (kelp forest, not consumed whole)
            branches: 0.0,
            spread: 2.5,
            maturity: 1.2,
            nutrients,
            toxicity: 0.0,
            fruit_toxicity: 0.0,
            temp_pref: 0.5,
            succulence: 0.0,
            submerged: 0.85, // deeper water, not just surface
            fruiting: 0.0,
            nitrogen_fix: 0.0,
            fire_seed: 0.0,
            climb: 0.0,
            allelopathy: 0.0,
            seed_weight: 0.1,
            windborne: 0.0,
            clonal: 0.7,     // kelp spreads by holdfast runners
            cling: 0.0,
            dormancy: 0.4,
            hydrochory: 0.8, // water-dispersed
            form: form::GROUNDCOVER,
            flower: 0.0,
            flower_hue: 0.3,
            flower_sat: 0.5,
            flower_light: 0.5,
            leaf_hue: 0.28, // brown-green kelp
            bushiness: 0.6,
            droop: 0.4,     // fronds sway/droop
        }
    }

    // Tree evolution: like plants but BIGGER ranges + tree-relevant genes. kind + defense stay pinned (tree
    // identity); height capped 1.0 (never taller than today's max).
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
        self.fruit_toxicity = (self.fruit_toxicity + rng.normal() * 0.08).clamp(0.0, 1.0);
        // blossom variety drifts on trees (flowering/fruit); form fixed (trees use Tree marker)
        self.temp_pref = (self.temp_pref + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.fruiting = (self.fruiting + rng.normal() * 0.1).clamp(0.0, 1.0);
        // trees disperse by seed: acorn (heavy, short) vs samara (light, windborne) drifts; burr trees cling
        // to fur, riverside/coastal trees float seeds (mangrove), oaks bank acorns (dormancy).
        self.seed_weight = (self.seed_weight + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.windborne = (self.windborne + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.cling = (self.cling + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.dormancy = (self.dormancy + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.hydrochory = (self.hydrochory + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.flower = (self.flower + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.flower_hue = (self.flower_hue + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.flower_sat = (self.flower_sat + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.flower_light = (self.flower_light + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.leaf_hue = (self.leaf_hue + rng.normal() * 0.05).clamp(0.0, 1.0);
    }

    pub fn mutate(&mut self, rng: &mut Rng) {
        // kind rarely flips; trait values drift. Each gene needs a drift line here (harness reads drift generically).
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
        self.fruit_toxicity = (self.fruit_toxicity + rng.normal() * 0.08).clamp(0.0, 1.0);
        // form stays fixed (visual species identity); submerged drifts only a little
        self.temp_pref = (self.temp_pref + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.succulence = (self.succulence + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.submerged = (self.submerged + rng.normal() * 0.06).clamp(0.0, 1.0);
        self.fruiting = (self.fruiting + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.nitrogen_fix = (self.nitrogen_fix + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.fire_seed = (self.fire_seed + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.climb = (self.climb + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.allelopathy = (self.allelopathy + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.seed_weight = (self.seed_weight + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.windborne = (self.windborne + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.clonal = (self.clonal + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.cling = (self.cling + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.dormancy = (self.dormancy + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.hydrochory = (self.hydrochory + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.flower = (self.flower + rng.normal() * 0.1).clamp(0.0, 1.0);
        self.flower_hue = (self.flower_hue + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.flower_sat = (self.flower_sat + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.flower_light = (self.flower_light + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.leaf_hue = (self.leaf_hue + rng.normal() * 0.05).clamp(0.0, 1.0);
        self.bushiness = (self.bushiness + rng.normal() * 0.08).clamp(0.0, 1.0);
        self.droop = (self.droop + rng.normal() * 0.06).clamp(0.0, 1.0);
    }

    // Two-parent recombination (--mating mode): uniform crossover. Each gene from one parent or other at
    // 50/50 (discrete kind/form + nutrient axes too). Caller mutates result. Paired w/ assortative mate
    // choice (only genetically-close plants cross, see plant_gene_dist) -> reproductive isolation -> emergent
    // species, like creature path.
    pub fn crossover(a: &Self, b: &Self, rng: &mut Rng) -> Self {
        let mut c = a.clone();
        macro_rules! pick {
            ($f:ident) => { c.$f = if rng.f32() < 0.5 { a.$f } else { b.$f }; };
        }
        pick!(kind); pick!(form);
        pick!(nutrient); pick!(defense); pick!(quality); pick!(wet); pick!(height);
        pick!(light_pref); pick!(regrow); pick!(branches); pick!(spread); pick!(maturity);
        pick!(toxicity); pick!(fruit_toxicity); pick!(temp_pref); pick!(succulence); pick!(submerged); pick!(fruiting);
        pick!(nitrogen_fix); pick!(fire_seed); pick!(climb); pick!(allelopathy);
        pick!(seed_weight); pick!(windborne); pick!(clonal); pick!(cling); pick!(dormancy); pick!(hydrochory);
        pick!(flower); pick!(flower_hue); pick!(flower_sat); pick!(flower_light); pick!(leaf_hue); pick!(bushiness); pick!(droop);
        for i in 0..NUTRIENTS {
            c.nutrients[i] = if rng.f32() < 0.5 { a.nutrients[i] } else { b.nutrients[i] };
        }
        c
    }

    // No free lunch (spec 10): nutrient richness, defense, digestible quality all slow growth. Defense
    // penalty QUADRATIC: cheap when light, crippling when maxed -> bounds arms race, can't armor to ~1.0 free
    // (balance lever, iter 1). Quality (palatable soft tissue) costs growth too; payoff is dispersal-on-eat
    // (spec 13) -> reaches interior optimum.
    pub fn growth_rate(&self) -> f32 {
        // mean nutrient richness taxes growth; toxicity QUADRATIC (cheap light, dear maxed) -> chemical
        // defense can't peg for free.
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
                - 0.3 * self.toxicity * self.toxicity
                // each gene taxes growth (no free lunch): water storage, fruit, root nodules, resin, vine
                - 0.25 * self.succulence
                - 0.2 * self.fruiting
                - 0.2 * self.nitrogen_fix
                - 0.08 * self.fire_seed
                - 0.1 * self.climb
                - 0.12 * self.allelopathy
                // dispersal genes tax growth: heavy seed provisioning, pappus/wings, runners
                - 0.12 * self.seed_weight
                - 0.06 * self.windborne
                - 0.14 * self.clonal
                // burrs/hooks, dormant seed coat, buoyant seed each cost a little growth
                - 0.06 * self.cling
                - 0.08 * self.dormancy
                - 0.05 * self.hydrochory
                - 0.04 * self.fruit_toxicity)
                .clamp(0.12, 1.0)
    }
}

// Foliage color APOSEMATIC: hue encodes TOXICITY -> danger reads at a glance (warning palette, like
// nightshade + poison frogs). Safe (low toxicity) = leafy GREEN; rising toxicity sweeps green -> yellow ->
// orange -> red -> violet (deadliest). leaf_hue jitters within band for per-plant variety; nutrient
// brightens; toxic = extra-saturated (bold warning). Bright, never drab/muddy.
pub fn plant_color(g: &PlantGenome) -> Color {
    let tox = g.toxicity.clamp(0.0, 1.0);
    // green(130) at safe .. red(0) at tox 0.7, then wrap 360 -> 285 violet for most toxic 0.7..1.0
    let warn = if tox <= 0.7 {
        130.0 * (1.0 - tox / 0.7)
    } else {
        360.0 - 75.0 * ((tox - 0.7) / 0.3)
    };
    let hue = (warn + (g.leaf_hue - 0.5) * 36.0).rem_euclid(360.0); // +/-18 deg per-plant variety
    let sat = (0.6 + 0.4 * tox).min(1.0); // vivid; toxic = boldest warning
    let light = 0.45 + 0.22 * g.nutrient; // bright (richer = brighter), never near-black/muddy
    Color::hsl(hue, sat, light)
}

// Petal color, flowering plant's bloom child. Fully genetic HSL: flower_hue spans whole wheel; flower_sat +
// flower_light each map to BRIGHT sub-range -> any bright color (vivid pure, soft pastel, rich jewel) but
// never dull/grey/dark muddy. Old saves default genes to 0.5 -> sat 0.78, light 0.65 (previous fixed look).
pub fn flower_color(g: &PlantGenome) -> Color {
    let hue = (g.flower_hue * 360.0).rem_euclid(360.0);
    let sat = 0.55 + 0.45 * g.flower_sat; // 0.55 .. 1.0: always colorful
    let light = 0.50 + 0.30 * g.flower_light; // 0.50 .. 0.80: always bright, never near-black/near-white
    Color::hsl(hue, sat, light)
}

