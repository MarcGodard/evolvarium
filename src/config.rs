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

// --- eating / arms race / predation (see 13, M5) ---
pub const BITE_K: f32 = 8.0; // eat/combat decisiveness = sigmoid(BITE_K*(bite - defense))
pub const BITE_COST: f32 = 0.7; // energy/sec maintenance cost of bite strength (linear)
pub const EAT_GAIN: f32 = 19.0; // energy per (mass * nutrient) consumed. Raised so a TYPICAL (not just elite) forager is comfortably net-positive -> more creatures persist + breed, so the equilibrium is set by food competition (plants drawn below cap) instead of mass starvation pinning pop at the reseed floor.
pub const MEAT_BONUS: f32 = 1.6; // meat (carrion) is richer + longer-lasting than plant food
pub const ATTACK_RADIUS: f32 = 1.6; // must be adjacent to attack
pub const PREDATION_GAIN: f32 = 16.0; // energy a predator gains from a kill (moderate: a big windfall fuels boom-bust)
pub const PREDATION_HUNGER: f32 = 20.0; // only creatures below this energy hunt (fed crowds don't cannibalize)
// Predation needs a real combat EDGE, not just proximity. Success = sigmoid(BITE_K*(adv) - BIAS) where
// adv = attacker combat - prey combat. With equal combat (a uniform population) success ~= sigmoid(-BIAS)
// = low, so creatures can't cannibalize their own kind into a boom-bust crash; a genuine carnivore (high
// bite + size vs smaller prey) still wins. Turns predation from population-wide churn into a real niche.
pub const PREDATION_BIAS: f32 = 2.2; // combat-edge required: equal-combat success = sigmoid(-2.2) ~= 0.10
// Kin-based social need (herd instinct). Being near genetically-SIMILAR creatures (kin) satisfies a
// social creature + protects it from predators (herd vigilance); ISOLATION drains energy (loneliness).
// Trade-off: social creatures must stay with their kind (constrains roaming) but gain safety; asocial
// loners roam free but a hunting loner finds no herd safety + (if social) starves of loneliness.
pub const SOCIAL_RADIUS: f32 = 13.0; // distance within which kin count as company (wide: easy to satisfy)
pub const SOCIAL_SIM: f32 = 0.7; // max signature distance to count as KIN (smaller = stricter species)
pub const SOCIAL_TARGET: f32 = 2.0; // just 2 kin nearby satisfies (avoids an Allee death-spiral at low density)
pub const SOCIAL_COST: f32 = 0.6; // energy/sec loneliness drain at full social gene + full isolation (MILD: a flavor pressure + herd benefit, NOT a population killer -- a strong drain spirals a spread-out population to extinction)
pub const SOCIAL_SAFETY: f32 = 0.7; // max predation-success reduction for prey surrounded by kin (herd safety)
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
pub const FERMENT_TOX_FRUIT: f32 = 0.15;    // toxicity scale of fermented fruit (low: ripe ethanol)
pub const FERMENT_TOX_DETRITUS: f32 = 0.85; // toxicity scale of fermented detritus (high: rotten sludge)

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
