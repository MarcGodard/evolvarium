// Central tuning knobs (god-tunable later, see 07). All sim balance constants live here so the
// trade-offs are discoverable in one place; sim.rs re-exports them (`pub use crate::config::*`).
// No logic here -- pure parameters.

// --- world + run length ---
pub const POP: usize = 90; // founder/generational population. Near the continuous carrying capacity (~50-70) so the warmup->continuous handoff is a gentle settle, not a 140->K crash that overshoots to extinction on some seeds.
pub const FOOD: usize = 480;
pub const WORLD_HALF: f32 = 80.0; // square arena [-H, H] in x,z (doubled playground)
pub const GEN_TICKS: u32 = 4800; // steps/generation (generational mode) + log interval = 2 full days (see DAY_TICKS): longer lives, creatures live through several day/night cycles so rest-timing can pay off
pub const MAX_GEN_HEADLESS: u32 = 40; // default headless run length in generations (override: --gens=N)

// Fixed sim step. Constant (not wall-clock) so headless can fast-forward and runs are
// deterministic. Render mode runs this at 60 Hz for real-time visuals; headless spins it flat-out.
pub const DT: f32 = 1.0 / 60.0;

pub const CREATURE_Y: f32 = 0.5;
pub const FOOD_Y: f32 = 0.4;

// --- day/night + light --- (day length lives in sphere::DAY_TICKS; day/night is positional on the planet)
pub const LIGHT_COST: f32 = 0.8; // energy/sec per unit mismatch between local light and a creature's light_pref
pub const TEMP_COST: f32 = 0.9; // energy/sec per unit mismatch between local temperature and temp_pref (thermal niche: poles harsh for warm-adapted, equator harsh for cold-adapted)

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
// Continuous breeding must require GENUINELY EARNED surplus, else the warmup->continuous handoff (all
// creatures at START_ENERGY 30) triggers a synchronized birth burst -> boom-bust extinction. Threshold
// ABOVE start energy means no one breeds at the handoff; each must forage up to it -> staggered births.
pub const REPRO_THRESHOLD: f32 = 30.0; // energy to be eligible (BELOW the ~33-37 foraging energy so the fed majority can breed -> R>1; density taper + competition cap the growth, not a high threshold)
pub const REPRO_COST: f32 = 16.0; // energy the parent spends per child (parent stays viable after: 30->14)
pub const BIRTH_ENERGY: f32 = 28.0; // offspring's starting energy: buffer so newborns establish before starving (raised: newborn die-off was the R<1 driver keeping pop at the floor)
pub const P_REPRO_CREATURE: f32 = 0.025; // per-tick reproduction chance while eligible (x density taper)
pub const REPRO_MIN_AGE: u32 = 180; // min ticks of life before breeding (newborns establish first; paces waves)
pub const CREATURE_CAP: usize = 130; // population ceiling (kept below grazing pressure that crashes plants)
pub const CREATURE_MIN: usize = 12; // reseed floor (safety net): below this, survivors' offspring are spawned so a continuous world can't fully go extinct (mirrors PLANT_MIN). Well below the ~60 equilibrium -> only fires in a crash.
pub const WARMUP_GENS: u32 = 12; // generational warm-up before continuous birth/death kicks in
pub const CONT_LOG_TICKS: u32 = 600; // continuous-mode stats log interval (fine enough to watch a crash unfold)

// --- three energy stores (metabolic currencies, see components::Energy) ---
// fast: tiny cap, burned first, LEAKS even at rest -> can't bank (volatile quick power, "fermented fruit").
// sugar: medium cap (the staple plants give). fat: big cap (easy store) but slow to mobilize + costs upkeep.
pub const FAST_CAP: f32 = 8.0;     // fast-store ceiling (small: can't hoard volatile energy)
pub const SUGAR_CAP: f32 = 28.0;   // sugar-store ceiling
pub const FAT_CAP: f32 = 30.0;     // base fat-store ceiling (x adiposity gene x size, see fat_cap())
pub const FAST_LEAK: f32 = 0.4;    // fast energy/sec lost passively even at rest (use-it-or-lose-it)
pub const FAT_POWER: f32 = 0.12;   // fraction of stored fat counted as instantly-available power (slow burn)
pub const STORE_LOSS: f32 = 0.5;   // sugar->fat conversion efficiency on overflow (storing is lossy)
pub const FAT_UPKEEP: f32 = 0.7;   // energy/sec extra basal at a full fat store x adiposity (carrying fat costs)
pub const MOVE_POWER_REF: f32 = 8.0; // power() at/above which full thrust is available; below -> sluggish
pub const ADIPOSITY_CAP: f32 = 1.2; // fat_cap multiplier span: cap = FAT_CAP*(0.4 + this*adiposity)*size

// --- creature metabolism + movement ---
pub const START_ENERGY: f32 = 30.0;
pub const BASAL_COST: f32 = 0.5; // energy/sec just to live (low so a fed creature can coast/rest and a competent forager is net-positive -> continuous persistence; bad foragers still starve = selection)
pub const STARVE_FLOOR: f32 = 1.0; // energy below which a creature is STARVING (~3% of START_ENERGY). Pinned below this for STARVE_TICKS in a row = death (kills grass-trickle zombies clinging at ~0 energy).
pub const STARVE_TICKS: u16 = 240; // consecutive starving ticks before death (~4s at 60/s): grace for a forager bridging between meals, lethal for a creature that never recovers.
pub const MOVE_COST: f32 = 6.0; // movement cost scales with thrust^2 (sprinting dear, gentle motion cheap)
pub const MOVE_SPEED: f32 = 9.0; // units/sec at full thrust
pub const TURN_SPEED: f32 = 3.0; // rad/sec at full turn
// Elevation (P3): climbing burns energy per unit height gained; descending refunds less than the
// climb cost so a round trip is a net loss (dissipative, no free lunch -> high ground is "expensive").
pub const CLIMB_COST: f32 = 1.2;
pub const DESCEND_REFUND: f32 = 0.4;
pub const ROCK_MOVE_COST: f32 = 9.0; // extra energy/sec moving over rocky highland (hard to cross)
// Alpine gene (mountain niche, mirror of swim): high alpine relieves most rock-crossing cost (climber) but
// a heavy mountain build wastes energy on flat ground. Mountains are only ~5% of the world, so in the
// lowland majority alpine is pure cost -> it stays low on average + only rises in highland dwellers (a real
// niche axis, self-limiting -> won't peg like armor did).
pub const ALPINE_RELIEF: f32 = 0.85;    // fraction of rock-crossing cost removed at full alpine
pub const ALPINE_FLAT_COST: f32 = 1.2;  // energy/sec penalty at full alpine on flat (non-rocky) ground
pub const SENSE_COST: f32 = 0.012; // energy/sec per unit of total sensor range (long-range vision isn't free)
pub const BRAIN_COST: f32 = 0.05; // energy/sec per hidden neuron: a bigger brain (more nodes) costs upkeep, so the NN grows only when the extra compute pays off -> brain size evolves to an interior optimum
pub const EAT_RADIUS: f32 = 1.1;
pub const OVEREAT_G: f32 = 0.2; // growth-load gained per unit of energy eaten while already full
pub const HEIGHT_COST: f32 = 0.7; // energy/sec upkeep per unit height (tall reaches trees but costs more)
// Body size (mass): a bigger creature stores more energy + hits harder in combat, but costs more to run
// and to maintain. Small = nimble + cheap; large = a tank. A physical axis the visualizer shows as scale.
pub const SIZE_ENERGY: f32 = 1.0;  // fat-store ceiling scales: fat_cap *= (1 + this*size) (bigger body banks more)
pub const SIZE_COMBAT: f32 = 0.5;  // added to bite as effective combat power in predation (mass wins fights)
pub const SIZE_BASAL: f32 = 1.6;   // energy/sec extra basal upkeep at full size (big bodies cost to maintain)
pub const SIZE_MOVE: f32 = 1.2;    // movement cost multiplier scales by (1 + this*size) (more mass to push)
// Swim (aquatic adaptation): in water / wet lowland a swimmer moves faster + cheaper (exploits the river +
// productive moist shoreline -> a "fish" niche); on dry high ground its fins are a liability (move penalty).
pub const SWIM_WET_LEVEL: f32 = 4.0;   // terrain height below which it counts as wet/aquatic (near WATER_LEVEL 2.8)
pub const SWIM_SPEED: f32 = 0.8;       // speed bonus fraction at full swim in water (fast fish)
pub const SWIM_LAND_COST: f32 = 5.0;   // energy/sec penalty at full swim on fully-dry land (clumsy on land)
// Water pressure: a non-swimmer (low swim gene) caught in OPEN water struggles + tires, scaling with DEPTH
// below the surface (0 at the shallow coast .. 1 at the abyss). Mirror of SWIM_LAND_COST: swimmers are exempt
// (built for it), so deep ocean is hostile to land creatures while shallow coastal water stays crossable.
pub const WATER_PRESSURE_COST: f32 = 6.0; // energy/sec penalty at full depth for a full non-swimmer in water
// Drowning (hard kill): a NON-aquatic creature (swim below this) caught in genuinely deep OPEN ocean drowns
// instantly. The gradual WATER_PRESSURE_COST still bites in shallow/coastal water (crossable), but the open
// sea is lethal to land animals -> only real swimmers (swim >= SWIM_DROWN_MIN) live there. DROWN_DEPTH keeps
// the kill to true open water (deep submersion), so wading the shoreline edge is safe.
pub const SWIM_DROWN_MIN: f32 = 0.5;   // min swim gene to survive open water; below it = drown
pub const DROWN_DEPTH: f32 = 0.25;     // min submersion (0 coast .. 1 abyss) before the drown kill applies

// --- eating / arms race / predation (see 13, M5) ---
pub const BITE_K: f32 = 8.0; // eat/combat decisiveness = sigmoid(BITE_K*(bite - defense))
pub const BITE_COST: f32 = 0.7; // energy/sec maintenance cost of bite strength (linear)
pub const EAT_GAIN: f32 = 19.0; // energy per (mass * nutrient) consumed. Raised so a TYPICAL (not just elite) forager is comfortably net-positive -> more creatures persist + breed, so the equilibrium is set by food competition (plants drawn below cap) instead of mass starvation pinning pop at the reseed floor.
pub const MEAT_BONUS: f32 = 1.6; // meat (carrion) is richer + longer-lasting than plant food
pub const ATTACK_RADIUS: f32 = 1.6; // must be adjacent to attack
pub const PREDATION_GAIN: f32 = 16.0; // energy a predator gains from a kill (moderate: a big windfall fuels boom-bust)
// (the old PREDATION_HUNGER gate was removed in M6: attack is now NN-gated by ATTACK_INTENT_THRESH, not hunger)
// Predation needs a real combat EDGE, not just proximity. Success = sigmoid(BITE_K*(adv) - BIAS) where
// adv = attacker combat - prey combat. With equal combat (a uniform population) success ~= sigmoid(-BIAS)
// = low, so creatures can't cannibalize their own kind into a boom-bust crash; a genuine carnivore (high
// bite + size vs smaller prey) still wins. Turns predation from population-wide churn into a real niche.
pub const PREDATION_BIAS: f32 = 2.2; // combat-edge required: equal-combat success = sigmoid(-2.2) ~= 0.10
// --- combat outputs (M6): NN-driven attack + active defense + sprint (brain out[2]/out[3]/out[5]) ---
// Attack is the brain's choice now (no hunger gate). Committing costs energy land-or-miss, and a whiff earns a
// learned penalty -> pointless aggression is selected against (this + the cost replace the old well-fed skip).
// Defense (out[3]): bracing raises effective defense but immobilizes. Sprint (out[5]): burst chase/flee speed,
// paid in fuel + fatigue. All single dials for tuning fight-vs-flight balance.
pub const ATTACK_INTENT_THRESH: f32 = 0.5; // out[2] above this = the creature is hunting this tick
pub const ATTACK_COST: f32 = 1.2; // energy/sec at full attack intent, paid whether the attack lands or misses
pub const BRACE_DEF: f32 = 2.0; // prey effective-defense bonus at full brace (compare ARMOR_DEF 2.5)
pub const BRACE_DRAG: f32 = 0.7; // fraction of move speed lost at full brace (the immobilize cost)
pub const SPRINT_BOOST: f32 = 0.6; // max burst-speed multiplier added at full sprint
pub const SPRINT_COST: f32 = 1.5; // extra energy/sec at full sprint
pub const SPRINT_FATIGUE: f32 = 0.8; // extra fatigue/sec weight at full sprint
pub const R_KILL: f32 = 1.5; // learn reward for a successful kill (reinforce the attack output)
pub const R_DEFEND: f32 = 0.8; // learn reward for surviving an attack while bracing (reinforce the defend output)
pub const R_WASTE: f32 = -0.3; // learn penalty for a committed attack that landed nothing
pub const EAT_GATE: f32 = 0.3; // out[4] threshold to ingest; BELOW the fresh-net 0.5 baseline so founders feed before learning
// Kin-based social need (herd instinct). Being near genetically-SIMILAR creatures (kin) satisfies a
// social creature + protects it from predators (herd vigilance); ISOLATION drains energy (loneliness).
// Trade-off: social creatures must stay with their kind (constrains roaming) but gain safety; asocial
// loners roam free but a hunting loner finds no herd safety + (if social) starves of loneliness.
pub const SOCIAL_RADIUS: f32 = 13.0; // distance within which kin count as company (wide: easy to satisfy)
pub const SOCIAL_SIM: f32 = 0.7; // max signature distance to count as KIN (smaller = stricter species)
pub const SOCIAL_TARGET: f32 = 2.0; // just 2 kin nearby satisfies (avoids an Allee death-spiral at low density)
pub const SOCIAL_COST: f32 = 0.6; // energy/sec loneliness drain at full social gene + full isolation (MILD: a flavor pressure + herd benefit, NOT a population killer -- a strong drain spirals a spread-out population to extinction)
pub const SOCIAL_SAFETY: f32 = 0.5; // max predation-success reduction for prey surrounded by kin (herd safety); softened from 0.7 so active defense (brace) competes with passive herd-hiding
// Body collision (M4): creatures are solid + don't pass through each other. Two bodies whose collision
// radii overlap get a soft tangential SHOVE apart (no hard stacking) + pay a jostle energy cost scaled by
// penetration depth. SOCIAL creatures are crowd-tolerant (herd animals pack tight) so the cost is x(1-social):
// a high-social herder huddles ~free, a low-social loner gets drained when bumped -> personal-space pressure
// that complements the kin-herd benefit (herders cluster, loners spread).
pub const COLLIDE_R: f32 = 0.9; // collision radius multiplier on the visual body half-width (just inside the silhouette)
pub const COLLIDE_COST: f32 = 0.5; // energy/sec jostle drain per unit overlap at full isolation (MILD, like loneliness)
pub const SEPARATION_STRENGTH: f32 = 0.5; // fraction of penetration shoved out per tick (soft, settles over a few ticks)
pub const SEED_VIA_GUT: f32 = 0.5; // max chance (x quality) an eaten plant disperses an offspring (13)
pub const PLANT_START_MASS: f32 = 0.6;
pub const PLANT_MIN_MASS: f32 = 0.15; // below this a grazed plant is fully consumed (carrot eaten whole)

// --- grass: render-only whole-planet ground cover (NOT in the food scan -> cheap at high counts). Edible
// only as a thin POSITION-based fallback (live_step): a hungry creature on grass-bearing soil nibbles. ---
pub const GRASS_CAP: usize = 8000; // target tuft count for whole-planet cover (render only; size set at attach)
pub const GRASS_START_MASS: f32 = 0.4;
pub const GRASS_HAB_MIN: f32 = 0.25; // min plant_habitability to seed/keep grass = "soil capable of plants"
pub const GRASS_GRAZE: f32 = 1.0; // energy/sec a HUNGRY creature nibbles from grassy soil (x habitability); thin fallback (balance TBD)

// --- rocky land: scattered boulders (render-only dressing). Spawned once on rocky highland so rocky terrain
// reads as a field of stone with grass growing between the rocks. Static -> no per-frame cost. ---
pub const ROCK_SCATTER: usize = 1200; // boulders scattered across rocky ground worldwide (denser the rockier)
pub const ROCK_GRASS_FRAC: f32 = 0.15; // chance a rocky-ground sample still seeds a (thin) grass tuft -> sparse grass between the rocks, not bare stone
pub const ROCK_GRASS_HAB: f32 = 0.28;  // survival-habitability floor for grass on rocky ground (keeps the sparse rocky tufts alive instead of churning)

// --- trees: long-lived, near-uneatable plants ---
pub const N_TREES: usize = 240; // initial trees (whole-planet seeding, scattered worldwide)
pub const TREE_CAP: usize = 480; // max trees (whole-planet forests; ambient reproduction fills toward this)
pub const TREE_MATURITY: f32 = 14.0; // trees grow large before reproducing
pub const P_TREE_REPRO: f32 = 0.004; // slow reproduction (long-lived, sparse)
pub const TREE_DENSITY_R: f32 = 18.0; // trees self-limit clustering within this radius
pub const TREE_MAX_LOCAL: usize = 4; // max trees within TREE_DENSITY_R before a tree stops seeding nearby
pub const TREE_BITE_MASS: f32 = 2.5; // mass a creature strips per feeding (tree survives + regrows)
pub const TREE_MIN_MASS: f32 = 1.0; // below this a fruit tree is over-eaten and dies
pub const TREE_GROWTH_SCALE: f32 = 0.4; // trees grow slowly: scales their growth rate down (long-lived, gradual)
// Tree SOIL response: a tree grows faster + to a BIGGER final size on good ground -- nutritious (fertile)
// AND a moisture SWEET SPOT (wet enough but not waterlogged). Survival stays moisture-immune; this only
// shapes growth speed + max size, so good soil makes visibly larger trees.
pub const TREE_WET_OPT: f32 = 0.5;     // ideal effective moisture for a tree (wet enough, not too wet)
pub const TREE_WET_TOL: f32 = 0.5;     // moisture distance from the optimum at which the growth bonus -> floor
pub const TREE_WET_FLOOR: f32 = 0.35;  // min growth multiplier off the sweet spot (a dry/swampy tree still grows, slowly)
pub const TREE_SOIL_SIZE: f32 = 1.2;   // max final-size bonus on ideal soil (up to (1+this)x maturity = 2.2x bigger)
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

// --- plant dispersal genetics (how seed_weight / windborne / clonal / fruiting shape where offspring land) ---
// Effective seed dispersal = genome `spread` x wind stretch x seed-weight drag. Light + windborne flies far
// (dandelion); heavy drops near the parent (acorn). Animal-carried (endozoochory) seeds go farther still,
// but toxic fruit is eaten less -> toxic plants stay clustered. Clonal is a separate short-range runner path.
pub const WIND_RANGE: f32 = 1.6;            // full windborne stretches dispersal by +160% (2.6x reach)
pub const SEED_DRAG: f32 = 0.6;             // full seed_weight shortens dispersal to 40% (heavy seed drops near parent)
pub const P_PLANT_EAT_DISPERSE: f32 = 0.06; // per-grazed-tick chance a fruiting plant's seed is animal-carried far
pub const PLANT_EAT_SPREAD_MULT: f32 = 2.5; // animal-carried plant seeds travel this much farther than wind-fall
pub const P_CLONAL: f32 = 0.012;            // per-tick clonal ramet chance scale (x clonal gene), for a mature plant
pub const CLONAL_RADIUS: f32 = 2.5;         // a runner / ramet sprouts this far from the parent (dense local patch)
// cling (epizoochory): a passing animal snags a burr + carries it far, even from an inedible plant. Abstracts
// animal traffic as a flat per-tick chance (no proximity scan, matching the cheap-ecology-gene convention).
pub const P_CLING: f32 = 0.05;              // per-tick chance scale a mature plant's seed hitches a ride (x cling)
pub const CLING_SPREAD_MULT: f32 = 3.0;     // a fur-borne seed travels this much farther than wind-fall
// hydrochory: a seed from a plant AT/NEAR water floats + rides far. near_water tapers from 1 at/below sea
// level to 0 a short band above it, so only coastal + aquatic plants get the long water dispersal.
pub const HYDRO_RANGE: f32 = 4.0;           // full hydrochory at the waterline stretches dispersal by +400%
pub const HYDRO_COAST_BAND: f32 = 0.06;     // elevation01 band above sea level still counted as near-water
// dormancy (seed bank): a fraction of seeds wait DORMANT in the soil, then germinate later -> a patch wiped
// by fire/drought/grazing re-greens from its buried bank. The bank persists independent of surface plants.
pub const DORMANCY_FRAC: f32 = 0.7;         // at dormancy=1, this fraction of seeds go to the bank instead of sprouting
pub const SEED_BANK_CAP: usize = 6000;      // max buried seeds tracked (drop new ones when full)
pub const DORMANT_TICKS_MIN: u32 = 200;     // shortest a seed waits buried before it germinates
pub const DORMANT_TICKS_MAX: u32 = 1600;    // longest a seed waits buried (staggers recruitment over time)
// plant + tree mating (--mating mode, shared with creatures): a seeding plant crosses with the nearest
// genetically-similar plant within range (assortative -> reproductive isolation -> emergent plant species);
// no compatible neighbor -> falls back to selfing (a clone), so sparse patches still set seed.
pub const PLANT_MATE_RADIUS: f32 = 9.0;     // pollen/pollinator reach: a mate must be within this distance
pub const PLANT_SPECIES_SIM: f32 = 0.18;    // max genetic distance to be the same species (cross-compatible)

// --- fruit + fermentation (Phase B): the forageable source of FAST energy ---
// Fruit trees drop fruit; fallen fruit + dead-plant detritus ferment over their Rot clock. Eating in
// the fermentation window (FERMENT_START..FERMENT_END as a fraction of ROT_GONE) yields FAST energy
// (ethanol). Before it: fresh -> sugar. After it: spoiled (toxic, near-zero yield), then rot_step gone.
pub const P_FRUIT_DROP: f32 = 0.012;        // per-tick chance a mature fruit tree drops a fruit
pub const FALLEN_FRUIT_MASS: f32 = 0.8;     // mass of a dropped fruit (a bite-sized ground food)
pub const FERMENT_START: f32 = 0.25;        // rot fraction where fermentation begins (before: fresh sugar)
pub const FERMENT_END: f32 = 0.70;          // rot fraction where it spoils (after: toxic, near-zero yield)
pub const FRUIT_FAST_GAIN: f32 = 22.0;      // fast energy per (mass*nutrient) from fermented fruit
pub const DETRITUS_FAST_GAIN: f32 = 5.0;    // fast energy from fermented detritus (<< fruit: poor, scrappy)
pub const FERMENT_TOX_DETRITUS: f32 = 0.85; // toxicity scale of fermented detritus (high: rotten sludge)
// Fruit ripeness (seed development). A fallen fruit's Rot clock now also models RIPENING: before RIPEN_FRAC
// the fruit is UNRIPE -- the seed is not viable yet (eating it disperses NOTHING, the seed is destroyed) and
// the flesh is bitter + carries the plant's fruit_toxicity (protects the developing seed). Past RIPEN_FRAC the
// fruit is ripe: sweet, detoxified, and eating it DISPERSES the seed (animal-carried). Tension: too-palatable
// unripe fruit gets eaten early (lost reproduction); too-toxic ripe fruit poisons dispersers -> interior optimum.
pub const RIPEN_FRAC: f32 = 0.12;       // rot fraction at which the seed becomes viable + the fruit sweetens
pub const UNRIPE_YIELD: f32 = 0.25;     // sugar an UNRIPE fruit yields vs ripe (low: not worth eating early)

// --- rot chain (P3): dead creature -> carrion -> poison -> gone ---
pub const CARRION_KIND: u8 = 0; // meat = food type 0 (couples to diet expr only via sensing, not digestion)
pub const CARRION_MASS: f32 = 3.0; // a meaty chunk: worth scavenging while fresh
pub const CARRION_NUTRIENT: f32 = 0.9; // fresh meat is energy-dense
pub const ROT_GONE: u32 = 900; // ticks from death to full decomposition (~15s sim); viz reads it for color
pub const TOXIN_MAX: f32 = 9.0; // toxin magnitude from eating fully-rotten carrion (now feeds toxic_load, see TOX_LOAD_*)

// --- moisture pressure + rain cycle (P3) ---
pub const DETRITUS_NUTRIENT: f32 = 0.3; // dead vegetation: poor food fresh, rots to poison
pub const MOISTURE_TOLERANCE: f32 = 0.3; // mismatch under this is harmless
pub const MOISTURE_KILL: f32 = 0.012; // per-tick death scale for mismatch beyond tolerance
pub const HABITAT_KILL: f32 = 0.03; // per-tick death scale in poor sites (deep water / arid desert)
// Drowning: a plant underwater needs the aquatic `wet` gene to survive. Mortality scales with submersion
// (how far below sea level) x (1 - wet): land flora (low wet) drowns, aquatic flora (high wet) thrives ->
// the wet gene splits land vs aquatic plants. Trees can't be aquatic at all (DROWN_TREE).
pub const DROWN_KILL: f32 = 0.06;   // per-tick death scale for a non-aquatic plant fully submerged
pub const DROWN_TREE: f32 = 0.5;    // per-tick death for a tree in water (trees are land-only, no kelp forests)
pub const SEASON_FREQ: f32 = 0.4; // seasonal wet/dry oscillation speed (radians per generation)
// --- new plant genes (real-life flora: aquatic depth, drought, climate niche, ecology) ---
pub const WATER_LIGHT_ATTEN: f32 = 0.85; // sunlight lost over a full water column: deep sea is dim, so only
                                         // shade plants (low light_pref, e.g. kelp) thrive deep; sun-lovers
                                         // (lily) need the shallows -> the deep-vs-shallow sun gradient.
pub const CLIMB_LIGHT: f32 = 0.4;        // vine light bonus: a climber reaches better-lit canopy (lf += climb*this)
pub const SUCC_BUFFER: f32 = 0.9;        // succulence drought buffer: water storage offsets DRY-side moisture stress
pub const TEMP_FLOOR: f32 = 0.45;        // floor on a plant's thermal growth factor (off-niche grows slow, not zero)
pub const TEMP_KILL: f32 = 0.01;         // per-tick death scale for climate-niche mismatch beyond tolerance
pub const TEMP_TOL: f32 = 0.3;           // temp_pref mismatch under this is harmless
// Hard freeze (instant kill): below this absolute temperature the GROUND is frozen solid -> no plant or tree
// survives (the polar ice core is barren). Set just inside the solid-ice core (below the temp<0.25 fuel cutoff)
// so the tundra/frost-edge band (0.25..0.34) keeps its cold-adapted flora (alpine cushion, moss); only the
// genuinely frozen cap kills outright. Independent of temp_pref -- even a cold-lover freezes on the ice.
pub const FREEZE_TEMP: f32 = 0.22;       // base_temperature below this = frozen -> plants/trees die instantly
// Trees get a climate niche too, but SOFTER than ground plants (long-lived, deep roots): wider harmless
// band + a gentler per-tick kill, so a tree dies back only well off its thermal niche (frozen pole / desert
// heat), not at the first mismatch. Growth still tapers off-niche via the shared TEMP_FLOOR factor.
pub const TREE_TEMP_TOL: f32 = 0.4;      // tree temp_pref mismatch under this is harmless (wider than plants)
pub const TREE_TEMP_KILL: f32 = 0.004;   // per-tick death scale for a tree beyond its (wide) thermal tolerance
pub const NFIX_RATE: f32 = 0.6;          // soil fertility/sec a full nitrogen-fixer (legume) adds at its spot
pub const DESICCATE_KILL: f32 = 0.1;     // per-tick death for an aquatic plant (high wet) stranded on dry land
                                         // (mirror of drown): aquatic flora needs water, so it can't carpet land
// Dynamic ground water (rain cycle): a wetness layer on TOP of the static terrain moisture. The sun
// evaporates it (faster at noon), storms refill it. Rocky cells shed runoff (no gain); grassy cells
// soak it up -> after a heavy rain, low-lying grassland turns wet and favors wet-liking plants, which
// then dry out and get stressed during the next drought. Drives temporal selection on plant `wet`.
// (Rain is now LOCAL + cloud-driven: sphere::rain_at / weather_step. No global storm onset/decay.)
pub const RAIN_RATE: f32 = 0.8;  // ground-water added/sec at full rain on a fully-absorbing (grassy) cell
pub const EVAP: f32 = 0.06;      // ground-water evaporated/sec at noon (scaled by sunlight, x current water)
pub const WET_GAIN: f32 = 0.45;  // how much saturated ground water adds to a plant's effective local moisture
pub const WET_GROWTH: f32 = 0.3; // growth-rate boost from watered ground (rain visibly greens the land)
// --- dynamic climate (geological): a THIRD moisture layer, slower than GroundWater. Per-cell long-term
// moisture relaxes toward a drifting target (regional rain propensity minus aridity) over MONTHS of sim
// time, so persistently-dry regions turn to desert + persistently-wet regions turn lush, and the wet belt
// slowly migrates -> deserts + rainforests form, persist, and move. Shared sim+render (deterministic).
pub const CLIMATE_RATE: f32 = 0.0003; // climate relax rate /sec toward target. tau=1/rate ~3333 sim-sec ~83 days (one day=2400 ticks=40s); geological
pub const CLIMATE_VEG: f32 = 1.0; // how strongly climate moisture (vs static) drives plant growth/mortality (1=full)
// (spatial drift consts CLIMATE_DRIFT + CLIMATE_SPEED live in sphere.rs alongside the other field consts)
// Lightning -> fire: during a storm, lightning strikes ignite a fire-grid cell. Fire spreads through DRY
// vegetation, burns plants/trees + hurts creatures caught in it, is doused by rain/wet ground, and leaves
// fertile ash. Ties the weather system to a dramatic, visible ecological disturbance + renewal cycle.
pub const P_LIGHTNING: f32 = 0.02; // per-tick strike chance while a storm is active (several per storm; most fizzle on wet ground, the dry-fuel ones catch)
pub const LIGHTNING_RAIN: f32 = 0.4; // rain intensity above which lightning can strike (storms only)
pub const FIRE_WET_MAX: f32 = 0.45; // fire only ignites/spreads into cells drier than this (wet ground won't burn)
pub const FIRE_DECAY: f32 = 0.18; // natural burnout per sec (raised: fires die out sooner, don't run away)
pub const FIRE_DOUSE: f32 = 2.0; // extra burnout per sec per unit local ground water (rain puts fire out)
pub const FIRE_SPREAD: f32 = 0.18; // base spread rate per sec to adjacent dry cells (lowered + now scaled by the neighbor's fuel density in fire_step, so sparse vegetation barely carries fire)
pub const FIRE_ASH: f32 = 2.5; // soil fertility deposited per sec by a burning cell (ash enriches regrowth)
pub const FIRE_BURN_ASH: f32 = 1.2; // EXTRA soil fertility deposited when a plant/tree burns UP (its biomass -> ash), x mass; trees deposit more (bigger biomass). Burned ground regrows richer.
pub const FIRE_KILL: f32 = 0.4; // fire intensity at which a plant/tree in the cell burns up
pub const FIRE_DAMAGE: f32 = 9.0; // energy/sec a creature loses standing in fire
// Defense also taxes REPRODUCTION, not just growth: at carrying capacity growth cost is toothless, so
// armored plants pegged defense ~free; penalizing repro lets cheaper plants win cap slots -> interior def.
pub const DEF_REPRO_COST: f32 = 0.7; // armored plant (def=1) reproduces at (1-0.7)=30% the base rate

// --- nutrient closed loop (M5): death -> soil fertility -> richer food ---
pub const SOIL_RES: usize = 32; // fertility grid cells per axis
pub const SOIL_DECAY: f32 = 0.999; // fertility leaches/is taken up each tick
pub const DECOMP_FERT: f32 = 3.0; // fertility released on full decomposition (x corpse nutrient)
pub const DEATH_FERT: f32 = 0.5; // fertility deposited IMMEDIATELY where a creature/plant dies (body enriches the ground there); plants then grow better on death sites (death feeds the area), on top of the slower decomposition release. Kept gentle: a strong death->fertility->plant-boom->creature-boom->crash loop is a destabilizing positive feedback.
pub const FERT_GROWTH: f32 = 0.6; // max growth-rate bonus from saturated soil
pub const FERT_CAP: f32 = 1.5; // fertility level at which the growth bonus saturates
pub const PLANT_REPRO_FRAC: f32 = 0.5; // fraction of mass kept after budding off a child

// --- nutrients + regulatory diet genome (Phase C, see 14/05) ---
// 10 nutrients. Plants produce a sparse profile (x soil fertility); meat is balanced. Creatures absorb
// per their uptake genes into reserves, which deplete with use. The master expression gene (reserves vs
// uptake demand) gates energy extraction; an unmet demanded nutrient = deficiency -> growth-load (soft).
pub const RESERVE_REQ: f32 = 0.6;       // reserve level at which a nutrient's demand is fully satisfied
pub const RESERVE_CAP: f32 = 1.5;       // max stored per nutrient (bank a little, not unlimited)
pub const MASTER_FLOOR: f32 = 0.45;     // floor on master expression: digestion never below this (soft gradient, not lethal)
pub const NUTRIENT_USE: f32 = 0.02;     // per-sec reserve depletion per unit uptake (slow: reserves are a buffer, not a fuse)
pub const NUTRIENT_ABSORB: f32 = 1.0;   // fraction of delivered nutrient (x uptake) that enters reserves on eat
pub const UPTAKE_OVERHEAD: f32 = 0.08;  // energy/sec upkeep per unit total uptake (gentle generalist tax; was lethal at 0.45)
pub const DEFICIT_G: f32 = 0.15;        // growth-load/sec per unit of average demanded-nutrient deficiency (soft)
pub const SOIL_NUTRI: f32 = 0.8;        // soil fertility boost to plant nutrient output: x(1 - this + this*fert/FERT_CAP)
pub const PLANT_TOX_HIT: f32 = 2.0;     // energy hit per unit plant toxicity eaten (x bite mass fraction)
pub const MEAT_RESERVE: f32 = 0.5;      // flat reserve top-up (all nutrients) from eating meat (balanced tissue)

// --- diet model (--diet): growth-load disease + aging (nutrient mechanics live in the Phase C block above) ---
pub const G_DECAY: f32 = 0.015; // growth-load shed per tick (recovery when well-nourished)
pub const DISEASE_K: f32 = 0.004; // per-tick disease mortality per unit growth-load
pub const AGE_HAZARD: f32 = 0.02; // late-life mortality ceiling (decelerates -> ~plateau)
pub const AGE_SCALE: f32 = 2400.0; // ticks; age at which aging hazard reaches half its ceiling (longer lifespans)
// Longevity gene: effective lifespan = AGE_SCALE * (0.4 + 1.2*longevity) (so longevity 0.5 = baseline x1.0).
// A long-lived body pays extra basal upkeep per unit lifespan above baseline -> slow-life (durable but
// costly) vs fast-life (cheap, breed young, die young) trade-off. Default longevity 0.5 = no extra cost.
pub const LONGEVITY_COST: f32 = 1.1; // energy/sec extra basal at the maximum lifespan multiplier

// --- generational GA ---
pub const ELITE_FRAC: f32 = 0.3;
pub const MUT_RATE: f32 = 0.12;
pub const MUT_STD: f32 = 0.3;

// --- lifetime learning (see 04 + 09) ---
pub const LEARN_RATE: f32 = 0.04;
pub const R_APPROACH: f32 = 0.6; // reward per unit closer to nearest food this tick
pub const R_EAT: f32 = 1.0; // bonus reward on the tick food is eaten

// --- M4 creature expansion: physiology + morphology gene costs/effects (see plan + 02/03) ---
// size = energy use: basal upkeep scales SUPER-LINEARLY with body size (allometry), so a big body is
// markedly hungrier per its mass. size still buys storage/combat/reach (above) -> big = powerful but dear.
pub const SIZE_BASAL_EXP: f32 = 1.5; // allometric exponent: size basal term = SIZE_BASAL * size^this
// detox (liver): clears toxic_load; costs basal upkeep.
pub const DETOX_COST: f32 = 0.5; // energy/sec basal at full detox (running a big liver costs)
// toxic load: ingested toxins (plant tox, rotten meat, fermented spoilage, venomous prey, protein excess)
// accumulate as a LOAD that drains energy + drives disease + a death hazard, cleared slowly (faster with
// detox). Replaces the old instant-only toxin energy hit -> poisons build + linger like real toxic load.
pub const TOX_LOAD_GAIN: f32 = 1.0;      // load gained per unit ingested toxin
pub const TOX_LOAD_CAP: f32 = 12.0;      // max accumulated toxic load (a hard ceiling)
pub const TOX_CLEAR_BASE: f32 = 0.15;    // load cleared/sec at zero detox (baseline metabolism)
pub const TOX_CLEAR_DETOX: f32 = 0.6;    // extra load cleared/sec at full detox
pub const TOX_LOAD_DRAIN: f32 = 0.25;    // energy/sec drained per unit toxic load (feeling sick)
pub const TOX_LOAD_G: f32 = 0.02;        // growth-load (disease) accrued/sec per unit toxic load
pub const TOX_LOAD_HAZARD: f32 = 0.0008; // death/sec per unit toxic load (acute poisoning)
// rabbit starvation: a carcass's usable ENERGY is its FAT (carried per-carcass = prey fatness at death).
// LEAN meat is mostly PROTEIN, and converting protein to usable energy/fat needs CARBS (the eater's sugar).
// So eating lean prey with no carbs yields little energy AND dumps unconvertible protein as toxic load
// (ammonia) -> an obligate carnivore on lean kills starves + poisons itself; fatty prey or plant carbs avoid
// it. `gut` = carnivory scales overall meat extraction (a herbivore gut handles meat poorly).
pub const PROTEIN_FLOOR: f32 = 0.35; // meat-extraction fraction a pure herbivore (carnivory 0) still manages
pub const PROTEIN_CARN: f32 = 0.65;  // extra meat-extraction fraction at full carnivory (-> full at 1.0)
pub const PROTEIN_TOX: f32 = 0.06;   // toxic load per unit of UNCONVERTIBLE protein (lean meat eaten without carbs)
// pelt (hair/fur): insulation. Cuts the COLD side of thermal mismatch; adds a HEAT-side cost in hot places,
// drags in water, and costs a little basal upkeep.
pub const PELT_COLD_RELIEF: f32 = 0.7; // fraction of cold-side temp cost removed at full pelt
pub const PELT_HEAT_COST: f32 = 0.6;   // energy/sec extra at full pelt in the hottest places (overheating)
pub const PELT_WATER_DRAG: f32 = 1.2;  // energy/sec extra at full pelt fully in water (waterlogged coat)
pub const PELT_UPKEEP: f32 = 0.2;      // energy/sec basal at full pelt (growing + carrying a coat)
// armor: lowers predation success against it (defense-only); costs move + basal upkeep.
pub const ARMOR_DEF: f32 = 2.5;   // added to prey combat as DEFENSE-only at full armor (hard to kill)
pub const ARMOR_MOVE: f32 = 0.8;  // move-cost multiplier add at full armor (heavy plates to push)
pub const ARMOR_BASAL: f32 = 0.5; // energy/sec basal at full armor
// venom: toxic flesh deters predators -> a predator eating venomous prey gains far less (a sickening kill).
pub const VENOM_DETER: f32 = 0.9;   // fraction of predation gain removed at full prey venom
pub const VENOM_UPKEEP: f32 = 0.3;  // energy/sec basal at full venom (making toxins costs)
// limbs: more legs = land traction (a small ground-speed bonus) but more move cost per limb. The gene maps
// to a rendered leg COUNT of LIMB_MIN..LIMB_MIN+LIMB_SPAN (Phase 4 visuals).
pub const LIMB_MIN: f32 = 2.0;       // legs at limbs gene 0
pub const LIMB_SPAN: f32 = 6.0;      // legs added across the gene range (-> 8 legs at gene 1)
pub const LIMB_TRACTION: f32 = 0.25; // max land-speed bonus fraction at full limbs
pub const LIMB_MOVE_COST: f32 = 0.5; // move-cost multiplier add at full limbs (more legs to drive)
// climb (arboreal): nimble -> evades predators; reaches fruit trees w/o a tall body; an arboreal build
// wastes energy on open flat ground.
pub const CLIMB_EVADE: f32 = 0.35;    // max predation-success reduction at full climb (agile escape); softened from 0.5 so fleeing/hiding doesn't dominate active fighting + defense
pub const CLIMB_REACH: f32 = 0.6;     // effective tree-reach height added at full climb (climbs to fruit)
pub const CLIMB_FLAT_COST: f32 = 0.6; // energy/sec at full climb on flat non-rocky ground (arboreal misfit)
// eyes: a small detection bonus (effective sensor range) for per-eye upkeep. The gene maps to a rendered
// eye COUNT of EYE_MIN..EYE_MIN+EYE_SPAN (Phase 4 visuals).
pub const EYE_MIN: f32 = 1.0;         // eyes at gene 0
pub const EYE_SPAN: f32 = 5.0;        // eyes added across the gene range (-> 6 eyes at gene 1)
pub const EYE_SENSE_BONUS: f32 = 0.3; // effective sensor-range multiplier add at full eyes (+30%)
pub const EYE_COST: f32 = 0.3;        // energy/sec basal at full eyes (eyes are metabolically pricey)
// head: a bigger head houses the brain more efficiently (cuts per-neuron BRAIN_COST) but adds basal mass.
pub const HEAD_BRAIN_RELIEF: f32 = 0.5; // fraction of BRAIN_COST removed at full head (roomy braincase)
pub const HEAD_BASAL: f32 = 0.4;        // energy/sec basal at full head (carrying a big head)
// magnetoreception: a magnetite organ + neural processing for the magnetic-latitude + compass brain inputs.
// Scaled by mag_expression (the `magneto` gene switch), so the sense is only worth keeping where it pays off.
pub const MAG_COST: f32 = 0.25;         // energy/sec basal at full magneto expression (no free lunch)

// --- M4 brain inputs + need-for-shade (Phase 3) ---
// threat sense: a nearby creature with enough COMBAT edge over you registers as a predator -> the brain can
// learn to flee (new threat_dist/threat_bearing inputs).
pub const THREAT_RADIUS: f32 = 22.0; // a bigger-combat creature within this is sensed as a threat
pub const THREAT_MARGIN: f32 = 0.4;  // combat edge a neighbor needs over you to register as a threat
// need for shade: in hot SUN (daylight x local temperature) an exposed creature overheats + burns energy;
// standing within canopy shade (near a tree) or being a heat-tolerant build relieves it. A new `shade`
// brain input lets brains learn to seek trees at midday.
pub const SHADE_RADIUS: f32 = 6.0;   // within this of a tree counts as canopy shade
pub const HEAT_SUN_COST: f32 = 1.4;  // energy/sec scale of open-sun heat stress in hot places
pub const HEAT_COMFORT: f32 = 0.45;  // sun x temp below this is comfortable (no heat cost)
pub const SHADE_RELIEF: f32 = 0.85;  // fraction of heat stress removed standing in full shade
