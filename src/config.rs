// Central tuning knobs (god-tunable later, see 07). All sim balance constants live here so the
// trade-offs are discoverable in one place; sim.rs re-exports them (`pub use crate::config::*`).
// No logic here -- pure parameters.

// --- world + run length ---
pub const POP: usize = 140;
pub const FOOD: usize = 480;
pub const WORLD_HALF: f32 = 80.0; // square arena [-H, H] in x,z (doubled playground)
pub const GEN_TICKS: u32 = 4800; // steps/generation (generational mode) + log interval = 2 full days (see DAY_TICKS): longer lives, creatures live through several day/night cycles so rest-timing can pay off
pub const MAX_GEN_HEADLESS: u32 = 40; // default headless run length in generations (override: --gens=N)

// Fixed sim step. Constant (not wall-clock) so headless can fast-forward and runs are
// deterministic. Render mode runs this at 60 Hz for real-time visuals; headless spins it flat-out.
pub const DT: f32 = 1.0 / 60.0;

pub const CREATURE_Y: f32 = 0.5;
pub const FOOD_Y: f32 = 0.4;

// --- day/night + light ---
pub const DAY_TICKS: u32 = 2400; // ticks per full day-night cycle (40s at 60Hz render: slow, watchable sunrise->sunset)
pub const LIGHT_COST: f32 = 2.2; // energy/sec per unit mismatch between local light and a creature's light_pref

// --- fatigue/stress (rest instinct) ---
// Exertion accrues fatigue, rest sheds it. Trade-off teeth: fatigue burns stress energy AND saps
// movement output while effort still costs full MOVE_COST -> grinding while exhausted is pure loss, so
// resting (low thrust) pays. Fed into the NN -> brains evolve to rest during their unfavorable-light
// hours (diurnal vs nocturnal niches emerge), see daylight() + light_pref.
pub const FATIGUE_GAIN: f32 = 0.5;  // fatigue/sec at full thrust (exertion debt)
pub const FATIGUE_REST: f32 = 0.35; // fatigue/sec shed at zero thrust (recovery)
pub const STRESS_COST: f32 = 1.6;   // energy/sec at full fatigue (chronic-exertion drain)
pub const FATIGUE_DRAG: f32 = 0.6;  // fraction of thrust output lost at full fatigue (tired = sluggish)

// --- continuous reproduction (default-off, --continuous) ---
pub const REPRO_THRESHOLD: f32 = 22.0; // energy to be eligible (near the sustained-forager equilibrium ~18)
pub const REPRO_COST: f32 = 10.0; // energy the parent spends per child (>BIRTH_ENERGY: birth dissipates some)
pub const BIRTH_ENERGY: f32 = 8.0; // offspring's starting energy
pub const P_REPRO_CREATURE: f32 = 0.02; // per-tick reproduction chance while eligible (gentle: damps overshoot)
pub const CREATURE_CAP: usize = 130; // population ceiling (kept below grazing pressure that crashes plants)
pub const WARMUP_GENS: u32 = 12; // generational warm-up before continuous birth/death kicks in

// --- creature metabolism + movement ---
pub const START_ENERGY: f32 = 30.0;
pub const BASAL_COST: f32 = 1.4; // energy/sec just to live (low -> resting is genuinely valuable)
pub const MOVE_COST: f32 = 6.0; // movement cost scales with thrust^2 (sprinting dear, gentle motion cheap)
pub const MOVE_SPEED: f32 = 9.0; // units/sec at full thrust
pub const TURN_SPEED: f32 = 3.0; // rad/sec at full turn
// Elevation (P3): climbing burns energy per unit height gained; descending refunds less than the
// climb cost so a round trip is a net loss (dissipative, no free lunch -> high ground is "expensive").
pub const CLIMB_COST: f32 = 1.2;
pub const DESCEND_REFUND: f32 = 0.4;
pub const ROCK_MOVE_COST: f32 = 9.0; // extra energy/sec moving over rocky highland (hard to cross)
pub const SENSE_COST: f32 = 0.012; // energy/sec per unit of total sensor range (long-range vision isn't free)
pub const EAT_RADIUS: f32 = 1.1;
pub const ENERGY_MAX: f32 = 60.0; // energy ceiling; eating past it harms (overeating trade-off, see 12)
pub const OVEREAT_G: f32 = 0.2; // growth-load gained per unit of energy eaten while already full
pub const HEIGHT_COST: f32 = 1.4; // energy/sec upkeep per unit height (tall reaches trees but costs more)

// --- eating / arms race / predation (see 13, M5) ---
pub const BITE_K: f32 = 8.0; // eat success = sigmoid(BITE_K*(bite - defense))
pub const BITE_COST: f32 = 1.5; // energy/sec maintenance cost of bite strength
pub const EAT_GAIN: f32 = 7.0; // energy per (mass * nutrient) consumed
pub const MEAT_BONUS: f32 = 1.6; // meat (carrion) is richer + longer-lasting than plant food
pub const ATTACK_RADIUS: f32 = 1.6; // must be adjacent to attack
pub const PREDATION_GAIN: f32 = 22.0; // energy a predator gains from a kill
pub const SEED_VIA_GUT: f32 = 0.5; // max chance (x quality) an eaten plant disperses an offspring (13)
pub const PLANT_START_MASS: f32 = 0.6;
pub const PLANT_MIN_MASS: f32 = 0.15; // below this a grazed plant is fully consumed (carrot eaten whole)

// --- trees: long-lived, near-uneatable plants ---
pub const N_TREES: usize = 40; // initial trees
pub const TREE_CAP: usize = 70; // max trees
pub const TREE_MATURITY: f32 = 14.0; // trees grow large before reproducing
pub const P_TREE_REPRO: f32 = 0.004; // slow reproduction (long-lived, sparse)
pub const TREE_DENSITY_R: f32 = 18.0; // trees self-limit clustering within this radius
pub const TREE_MAX_LOCAL: usize = 4; // max trees within TREE_DENSITY_R before a tree stops seeding nearby
pub const TREE_BITE_MASS: f32 = 2.5; // mass a creature strips per feeding (tree survives + regrows)
pub const TREE_MIN_MASS: f32 = 1.0; // below this a fruit tree is over-eaten and dies
pub const TREE_GROWTH_SCALE: f32 = 0.4; // trees grow slowly: scales their growth rate down (long-lived, gradual)
// Fruit-tree life-history (evolvable height + the trade-offs that bound it). Tree height is a gene that
// drifts over generations within [TREE_HEIGHT_MIN, 1.0] (wide range, never taller than today's max).
// Three pressures shape it: (1) reach -- a creature reaches a fruit tree only if its height +
// TREE_REACH_MARGIN >= the tree's height, so taller trees feed fewer creatures; (2) dispersal-on-eat --
// being eaten spreads the tree's seeds FARTHER (animal-carried), so a reachable tree out-reproduces an
// unreachable one; (3) mass-nutrition cost -- a bulkier tree dilutes its fruit (less nutrient/bite). Net:
// growing too tall trades away the dispersal benefit, so height settles at an interior optimum.
pub const TREE_HEIGHT_MIN: f32 = 0.4; // shortest a freshly-seeded tree starts (then evolves freely 0..1)
pub const TREE_REACH_MARGIN: f32 = 0.4; // creature reaches tree if creature.height + this (+branches) >= tree.height
pub const BRANCH_REACH: f32 = 0.7; // full branches extend a tree's reachable-from height down by this much
pub const TREE_MASS_NUTRI: f32 = 0.5; // at full maturity a tree's fruit is (1-this)x as nutritious (mass dilutes nutrition)
pub const P_TREE_EAT_DISPERSE: f32 = 0.03; // per-grazed-tick chance an eaten fruit tree disperses a seed (animal-carried)
pub const TREE_EAT_SPREAD_MULT: f32 = 2.5; // animal-carried seeds travel this much farther than wind-fall

// --- rot chain (P3): dead creature -> carrion -> poison -> gone ---
pub const CARRION_KIND: u8 = 0; // meat = food type 0 (couples to diet expr only via sensing, not digestion)
pub const CARRION_MASS: f32 = 3.0; // a meaty chunk: worth scavenging while fresh
pub const CARRION_NUTRIENT: f32 = 0.9; // fresh meat is energy-dense
pub const ROT_GONE: u32 = 900; // ticks from death to full decomposition (~15s sim); viz reads it for color
pub const TOXIN_MAX: f32 = 9.0; // energy hit from eating fully-rotten carrion (poison)
pub const TOXIN_G: f32 = 0.15; // growth-load per unit toxin ingested

// --- moisture pressure + rain cycle (P3) ---
pub const DETRITUS_NUTRIENT: f32 = 0.3; // dead vegetation: poor food fresh, rots to poison
pub const MOISTURE_TOLERANCE: f32 = 0.3; // mismatch under this is harmless
pub const MOISTURE_KILL: f32 = 0.012; // per-tick death scale for mismatch beyond tolerance
pub const HABITAT_KILL: f32 = 0.03; // per-tick death scale in poor sites (deep water / arid desert)
pub const SEASON_FREQ: f32 = 0.4; // seasonal wet/dry oscillation speed (radians per generation)
// Dynamic ground water (rain cycle): a wetness layer on TOP of the static terrain moisture. The sun
// evaporates it (faster at noon), storms refill it. Rocky cells shed runoff (no gain); grassy cells
// soak it up -> after a heavy rain, low-lying grassland turns wet and favors wet-liking plants, which
// then dry out and get stressed during the next drought. Drives temporal selection on plant `wet`.
pub const P_STORM: f32 = 0.0008; // per-tick chance a downpour begins (~1 storm / 1250 ticks)
pub const RAIN_RATE: f32 = 0.8;  // ground-water added/sec at full rain on a fully-absorbing (grassy) cell
pub const RAIN_DECAY: f32 = 0.2; // rain intensity shed/sec (a storm fades over ~5s)
pub const EVAP: f32 = 0.06;      // ground-water evaporated/sec at noon (scaled by sunlight, x current water)
pub const WET_GAIN: f32 = 0.45;  // how much saturated ground water adds to a plant's effective local moisture
pub const WET_GROWTH: f32 = 0.3; // growth-rate boost from watered ground (rain visibly greens the land)
// Defense also taxes REPRODUCTION, not just growth: at carrying capacity growth cost is toothless, so
// armored plants pegged defense ~free; penalizing repro lets cheaper plants win cap slots -> interior def.
pub const DEF_REPRO_COST: f32 = 0.7; // armored plant (def=1) reproduces at (1-0.7)=30% the base rate

// --- nutrient closed loop (M5): death -> soil fertility -> richer food ---
pub const SOIL_RES: usize = 32; // fertility grid cells per axis
pub const SOIL_DECAY: f32 = 0.999; // fertility leaches/is taken up each tick
pub const DECOMP_FERT: f32 = 3.0; // fertility released on full decomposition (x corpse nutrient)
pub const FERT_GROWTH: f32 = 0.6; // max growth-rate bonus from saturated soil
pub const FERT_CAP: f32 = 1.5; // fertility level at which the growth bonus saturates
pub const PLANT_REPRO_FRAC: f32 = 0.5; // fraction of mass kept after budding off a child

// --- diet/epigenetic model (--diet, see 12) ---
pub const EXPR_RAMP: f32 = 0.08; // how fast expression of the eaten type rises (x (1-rigidity))
pub const EXPR_DECAY: f32 = 0.04; // how fast unused types' expression falls (x (1-rigidity))
pub const EXPR_OVERHEAD: f32 = 1.2; // maintenance energy/sec per unit total expression (generalist cost)
pub const G_GAIN: f32 = 0.6; // growth-load gained per low-efficiency (mismatch) eat
pub const G_DECAY: f32 = 0.01; // growth-load shed per tick when on-diet
pub const DISEASE_K: f32 = 0.012; // per-tick disease mortality per unit growth-load
pub const MISMATCH_STRESS: f32 = 3.0; // energy hit for eating a poorly-expressed (wrong) food
pub const AGE_HAZARD: f32 = 0.02; // late-life mortality ceiling (decelerates -> ~plateau)
pub const AGE_SCALE: f32 = 2400.0; // ticks; age at which aging hazard reaches half its ceiling (longer lifespans)

// --- generational GA ---
pub const ELITE_FRAC: f32 = 0.3;
pub const MUT_RATE: f32 = 0.12;
pub const MUT_STD: f32 = 0.3;

// --- lifetime learning (see 04 + 09) ---
pub const LEARN_RATE: f32 = 0.04;
pub const R_APPROACH: f32 = 0.6; // reward per unit closer to nearest food this tick
pub const R_EAT: f32 = 1.0; // bonus reward on the tick food is eaten
