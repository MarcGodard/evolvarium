// Central tuning knobs (god-tunable later, see 07). All sim balance constants here so trade-offs
// discoverable in one place. sim.rs re-exports via `pub use crate::config::*`. Pure params, no logic.

// --- world + run length ---
pub const POP: usize = 90; // founder/generational pop. Near continuous carrying capacity (~50-70) so warmup->continuous handoff is gentle settle, not 140->K crash that overshoots to extinction on some seeds.
pub const FOOD: usize = 480;
pub const WORLD_HALF: f32 = 80.0; // square arena [-H, H] in x,z
pub const GEN_TICKS: u32 = 4800; // steps/gen (generational mode) + log interval = 2 full days (see DAY_TICKS). Longer lives -> creatures span several day/night cycles so rest-timing pays off.
pub const MAX_GEN_HEADLESS: u32 = 40; // default headless run length in gens (override: --gens=N)

// Fixed sim step. Constant (not wall-clock) so headless fast-forwards + runs deterministic. Render
// mode runs this at 60 Hz real-time; headless spins flat-out.
pub const DT: f32 = 1.0 / 60.0;

pub const CREATURE_Y: f32 = 0.5;
pub const FOOD_Y: f32 = 0.4;

// --- day/night + light --- (day length in sphere::DAY_TICKS; day/night positional on planet)
pub const LIGHT_COST: f32 = 0.8; // energy/sec per unit mismatch: local light vs light_pref
pub const TEMP_COST: f32 = 0.9; // energy/sec per unit mismatch: local temp vs temp_pref. Thermal niche: poles harsh for warm-adapted, equator harsh for cold-adapted.

// --- fatigue/stress (rest instinct) ---
// Exertion accrues fatigue, rest sheds it. Trade-off teeth: fatigue burns stress energy AND saps move
// output while effort still costs full MOVE_COST -> grinding while exhausted is pure loss, so resting
// (low thrust) pays. Fed into NN -> brains evolve to rest in unfavorable-light hours (diurnal vs
// nocturnal niches emerge). See daylight() + light_pref.
pub const FATIGUE_GAIN: f32 = 0.5;  // fatigue/sec at full thrust (exertion debt)
pub const FATIGUE_REST: f32 = 0.35; // fatigue/sec shed at zero thrust (recovery)
pub const STRESS_COST: f32 = 1.6;   // energy/sec at full fatigue (chronic-exertion drain)
pub const FATIGUE_DRAG: f32 = 0.6;  // fraction of thrust output lost at full fatigue (tired = sluggish)

// --- continuous reproduction (default-off, --continuous) ---
// Continuous breeding must require GENUINELY EARNED surplus, else warmup->continuous handoff (all at
// START_ENERGY 30) triggers synchronized birth burst -> boom-bust extinction. Threshold ABOVE start
// energy means no one breeds at handoff; each must forage up to it -> staggered births.
pub const REPRO_THRESHOLD: f32 = 30.0; // eligibility energy. BELOW ~33-37 foraging energy so fed majority breeds -> R>1. Density taper + competition cap growth, not a high threshold.
pub const REPRO_COST: f32 = 16.0; // energy parent spends per child (parent stays viable: 30->14)
pub const BIRTH_ENERGY: f32 = 28.0; // offspring start energy: buffer so newborns establish before starving (raised: newborn die-off was R<1 driver pinning pop at floor)
pub const P_REPRO_CREATURE: f32 = 0.025; // per-tick repro chance while eligible (x density taper)
pub const REPRO_MIN_AGE: u32 = 180; // min ticks of life before breeding (newborns establish first; paces waves)
pub const CREATURE_CAP: usize = 1100; // global pop ceiling. Scaled ~8.5x for the ~1000-creature world (live_step parallel). Food web supports it: probe held plants ~5800 at pop 1600, no crash. Per-niche caps bind first.
// Per-niche carrying capacity (repro tapers on the breeder's OWN niche fill, not global pop) -> each habitat
// fills independently so no single niche soaks the shared cap (was winner-take-all: one niche -> ~83% planet,
// which one is seed-stochastic). Order = Niche::idx [aquatic,aerial,highland,cold,warm,land]. Aquatic biggest
// (ocean ~half planet) but ~30% not 83%. Scaled ~7x from [45,15,16,18,22,36] for the ~1000-pop world; ratios
// preserved so habitat balance holds. Sum (~1064) ~= CREATURE_CAP so global cap + niche caps both bind near 1000.
pub const NICHE_CAP: [usize; 6] = [315, 105, 112, 126, 154, 252];
pub const WARMUP_GENS: u32 = 12; // generational warm-up before continuous birth/death kicks in
pub const CONT_LOG_TICKS: u32 = 600; // continuous-mode stats log interval (fine enough to watch a crash unfold)

// Per-niche rescue (niche.rs): floors + hall-of-fame banks keep each habitat alive (aquatic/aerial/highland/
// cold/warm/land). Replaces the global CREATURE_MIN floor for continuous mode. DAY_TICKS=2400, GEN_TICKS=4800.
pub const NICHE_FLOOR: usize = 40;         // per-niche min pop; below -> rescue from bank. Scaled ~7x with NICHE_CAP for the ~1000-pop world (emergency backstop; never triggers at healthy density).
pub const NICHE_BANK_CAP: usize = 8;       // hall-of-fame size per niche (best genomes kept for revival)
pub const NICHE_BANK_TICKS: u32 = 1200;    // rebuild banks every half-day from fittest living per niche
pub const NICHE_RESCUE_COOLDOWN: u32 = 600; // min ticks between rescues of the SAME niche (ease in, not burst)
pub const NICHE_RESCUE_BATCH: u32 = 4;     // max seeded per rescue event per niche
pub const NICHE_SUSTAIN_WINDOW: u32 = 4800; // no rescue for this long (2 days) -> ecosystem self-sustaining

// --- three energy stores (metabolic currencies, see components::Energy) ---
// fast: tiny cap, burned first, LEAKS even at rest -> can't bank (volatile quick power, "fermented fruit").
// sugar: medium cap (staple plants give). fat: big cap (easy store) but slow to mobilize + costs upkeep.
pub const FAST_CAP: f32 = 8.0;     // fast-store ceiling (small: can't hoard volatile energy)
pub const SUGAR_CAP: f32 = 28.0;
pub const FAT_CAP: f32 = 30.0;     // base fat-store ceiling (x adiposity gene x size, see fat_cap())
pub const FAST_LEAK: f32 = 0.4;    // fast energy/sec lost passively even at rest (use-it-or-lose-it)
pub const FAT_POWER: f32 = 0.12;   // fraction of stored fat as instantly-available power (slow burn)
pub const STORE_LOSS: f32 = 0.5;   // sugar->fat conversion efficiency on overflow (storing is lossy)
pub const FAT_UPKEEP: f32 = 0.7;   // energy/sec extra basal at full fat store x adiposity (carrying fat costs)
pub const MOVE_POWER_REF: f32 = 8.0; // power() at/above which full thrust available; below -> sluggish
pub const ADIPOSITY_CAP: f32 = 1.2; // fat_cap mult span: cap = FAT_CAP*(0.4 + this*adiposity)*size

// --- creature metabolism + movement ---
pub const START_ENERGY: f32 = 30.0;
pub const BASAL_COST: f32 = 0.5; // energy/sec just to live. Low so fed creature can coast/rest + competent forager is net-positive -> continuous persistence. Bad foragers still starve = selection.
pub const STARVE_FLOOR: f32 = 1.0; // energy below = STARVING (~3% of START_ENERGY). Pinned below for STARVE_TICKS straight = death (kills grass-trickle zombies clinging at ~0 energy).
pub const STARVE_TICKS: u16 = 240; // consecutive starving ticks before death (~4s at 60/s): grace for forager bridging meals, lethal for one that never recovers.
pub const MOVE_COST: f32 = 6.0; // move cost scales thrust^2 (sprinting dear, gentle motion cheap)
pub const MOVE_SPEED: f32 = 9.0; // units/sec at full thrust
pub const TURN_SPEED: f32 = 3.0; // rad/sec at full turn
// Elevation (P3): climbing burns energy per unit height gained; descending refunds less than climb cost
// so round trip is net loss (dissipative, no free lunch -> high ground "expensive").
pub const CLIMB_COST: f32 = 1.2;
pub const DESCEND_REFUND: f32 = 0.4;
pub const ROCK_MOVE_COST: f32 = 9.0; // extra energy/sec over rocky highland (hard to cross)
// Alpine gene (mountain niche, mirror of swim): high alpine relieves most rock-crossing cost (climber)
// but heavy mountain build wastes energy on flat ground. Mountains ~5% of world, so in lowland majority
// alpine is pure cost -> stays low on average, rises only in highland dwellers. Real niche axis,
// self-limiting -> won't peg like armor did.
pub const ALPINE_RELIEF: f32 = 0.85;    // fraction of rock-crossing cost removed at full alpine
pub const ALPINE_FLAT_COST: f32 = 1.7;  // energy/sec penalty at full alpine on flat (non-rocky) ground. Raised 1.2->1.7 (balance harness): soft penalty let highland creatures forage cheaply across ~95% flat planet -> dominated + starved other niches. Confines heavy mountain build to the rocks.
pub const SENSE_COST: f32 = 0.012; // energy/sec per unit total sensor range (long-range vision not free)
pub const BRAIN_COST: f32 = 0.05; // energy/sec per hidden neuron. Bigger brain costs upkeep, so NN grows only when extra compute pays off -> brain size evolves to interior optimum.
pub const EAT_RADIUS: f32 = 1.1;
pub const OVEREAT_G: f32 = 0.2; // growth-load gained per unit energy eaten while already full
pub const HEIGHT_COST: f32 = 0.7; // energy/sec upkeep per unit height (tall reaches trees but costs more)
// Body size (mass): bigger creature stores more energy + hits harder in combat, but costs more to run +
// maintain. Small = nimble + cheap; large = tank. Physical axis visualizer shows as scale.
pub const SIZE_ENERGY: f32 = 1.0;  // fat-store ceiling scales: fat_cap *= (1 + this*size)
pub const SIZE_COMBAT: f32 = 0.5;  // added to bite as effective combat power in predation (mass wins fights)
pub const SIZE_BASAL: f32 = 2.8;   // energy/sec extra basal upkeep at full size (big bodies cost to maintain)
pub const SIZE_MOVE: f32 = 1.2;    // move cost mult scales (1 + this*size) (more mass to push)
// Swim (aquatic): in water/wet lowland a swimmer moves faster + cheaper (exploits river + productive
// moist shoreline -> "fish" niche); on dry high ground its fins are liability (move penalty).
pub const SWIM_WET_LEVEL: f32 = 4.0;   // terrain height below = wet/aquatic (near WATER_LEVEL 2.8)
pub const SWIM_SPEED: f32 = 0.8;       // speed bonus fraction at full swim in water (fast fish)
pub const SWIM_LAND_COST: f32 = 5.0;   // energy/sec penalty at full swim on fully-dry land (clumsy on land)
// Water pressure: non-swimmer (low swim) in OPEN water struggles + tires, scaling with DEPTH below
// surface (0 shallow coast .. 1 abyss). Mirror of SWIM_LAND_COST: swimmers exempt -> deep ocean hostile
// to land creatures, shallow coastal water stays crossable.
pub const WATER_PRESSURE_COST: f32 = 6.0; // energy/sec penalty at full depth for full non-swimmer in water
// Drowning (hard kill): NON-aquatic creature (swim below this) in genuinely deep OPEN ocean drowns
// instantly. Gradual WATER_PRESSURE_COST still bites in shallow/coastal water (crossable), but open sea
// lethal to land animals -> only real swimmers (swim >= SWIM_DROWN_MIN) live there. DROWN_DEPTH keeps
// kill to true open water (deep submersion) so wading shoreline edge is safe.
pub const SWIM_DROWN_MIN: f32 = 0.5;   // min swim gene to survive open water; below = drown
pub const DROWN_DEPTH: f32 = 0.25;     // min submersion (0 coast .. 1 abyss) before drown kill applies

// Flight (aerial): VERTICAL mirror of swim. `flight` gene + per-creature altitude (surface-offset units
// above CREATURE_Y). Brain out[6] = climb intent. Airborne = fast + skips ground collision/drowning, but
// holding altitude burns energy and big wings are clumsy on the ground (mirror SWIM_LAND_COST). Conservative
// costs in this visuals-first pass: flight must NOT become a free escape valve (balance-phase follow-up).
pub const FLIGHT_KNEE: f32 = 0.35;      // min flight gene to leave the ground (below = grounded walker)
pub const MAX_FLIGHT_ALT: f32 = 6.0;    // altitude ceiling (surface-offset units above CREATURE_Y)
pub const FLIGHT_SPEED: f32 = 1.2;      // top-speed bonus fraction at full flight when fully airborne (fast flier)
pub const FLIGHT_CLIMB_RATE: f32 = 4.0; // altitude units/sec gained/lost at full climb/descend intent
pub const FLIGHT_BUOYANCY: f32 = 1.2;   // passive relax/sec toward cruise altitude (neutral buoyancy: fliers
                                        // hover aloft, fish hover mid-water, even with a neutral brain). Brain
                                        // out[6] climbs/descends around it; landing to eat = sustained descend.
pub const FLIGHT_CRUISE: f32 = 0.15;    // resting altitude as fraction of medium ceiling (where buoyancy settles). Low so fliers FORAGE near ground (within EAT_RADIUS 1.1 of ground food) + pop up to travel/escape. At 0.55 a flier hovered ~2.3 alt >> eat range -> untrained birds starved before learning to dive (bootstrap death).
pub const FLIGHT_ALT_COST: f32 = 0.8;   // energy/sec BASE to hold full altitude (small-bird flapping upkeep). Lowered 1.5->0.8: birds must be SMALL + CHEAP aloft (wing-loading via FLIGHT_SIZE_LIFT adds the body-mass cost on top). Cheap base lets a small flier self-sustain on the wing.
// Wing loading: big body costs MUCH more to hold aloft (mass to lift). Adds to FLIGHT_ALT_COST per unit
// size, so flight selects SMALL birds (a heavy flier can't pay its altitude). Small (size 0.15) flier ~free;
// big (size 0.6) flier punished -> aerial niche evolves toward tiny light bodies.
pub const FLIGHT_SIZE_LIFT: f32 = 3.0;
// Glide ("slide"): fast FORWARD flight discounts altitude cost (airspeed gives lift -> stop flapping, soar).
// Scales with flight gene x forward effort: a fast strong flier cruising barely pays to stay up; a hovering
// flier pays full. Max discount = this * flight * move_thrust (capped so cost stays positive).
pub const GLIDE_RELIEF: f32 = 0.7;
pub const FLIGHT_GROUND_COST: f32 = 2.0;// energy/sec penalty for full flight gene while grounded (clumsy wings). Lowered 3.0->2.0 so landing to feed on ground food isn't brutal (birds still feed grounded until aerial food exists).
// Wings + fins are mutually antagonistic body plans: paying for BOTH (high flight AND high swim) drains hard, so
// evolution forces a creature to specialize -> a flier has low swim (drowns in deep water, can't reside there), a
// swimmer has low flight (< FLIGHT_KNEE -> grounded, can't reside in air). Closes the amphibious loophole that
// let one creature live in every medium. energy/sec at full flight x full swim; pure specialists pay 0.
pub const FLIGHT_SWIM_CONFLICT: f32 = 3.0;
pub const GROUND_EPS: f32 = 0.4;        // altitude below this = "on the ground" (can drown). Eating/predation/
                                        // collision need no vertical gate: all key off 3D translation, so an
                                        // airborne flier is auto > EAT_RADIUS/ATTACK_RADIUS from ground stuff.

// --- eating / arms race / predation (see 13, M5) ---
pub const BITE_K: f32 = 8.0; // eat/combat decisiveness = sigmoid(BITE_K*(bite - defense))
pub const BITE_COST: f32 = 0.7; // energy/sec maintenance cost of bite strength (linear)
pub const EAT_GAIN: f32 = 19.0; // energy per (mass * nutrient). Raised so TYPICAL (not just elite) forager is comfortably net-positive -> more creatures persist + breed, so equilibrium set by food competition (plants drawn below cap) not mass starvation pinning pop at reseed floor.
pub const MEAT_BONUS: f32 = 1.6; // meat (carrion) richer + longer-lasting than plant food
pub const ATTACK_RADIUS: f32 = 1.6; // must be adjacent to attack
pub const PREDATION_GAIN: f32 = 16.0; // energy predator gains from kill (moderate: big windfall fuels boom-bust)
// (old PREDATION_HUNGER gate removed in M6: attack now NN-gated by ATTACK_INTENT_THRESH, not hunger)
// Predation needs real combat EDGE, not just proximity. Success = sigmoid(BITE_K*(adv) - BIAS) where adv
// = attacker combat - prey combat. Equal combat (uniform pop) -> success ~= sigmoid(-BIAS) = low, so
// creatures can't cannibalize their own kind into boom-bust crash; genuine carnivore (high bite + size
// vs smaller prey) still wins. Turns predation from pop-wide churn into a real niche.
pub const PREDATION_BIAS: f32 = 2.2; // combat-edge required: equal-combat success = sigmoid(-2.2) ~= 0.10
// --- combat outputs (M6): NN-driven attack + active defense + sprint (brain out[2]/out[3]/out[5]) ---
// Attack is brain's choice now (no hunger gate). Committing costs energy land-or-miss, whiff earns
// learned penalty -> pointless aggression selected against (this + cost replace old well-fed skip).
// Defense (out[3]): bracing raises effective defense but immobilizes. Sprint (out[5]): burst chase/flee
// speed, paid in fuel + fatigue. All single dials for tuning fight-vs-flight balance.
pub const ATTACK_INTENT_THRESH: f32 = 0.5; // out[2] above this = creature hunting this tick
pub const ATTACK_COST: f32 = 1.2; // energy/sec at full attack intent, paid whether attack lands or misses
pub const BRACE_DEF: f32 = 2.0; // prey effective-defense bonus at full brace (compare ARMOR_DEF 2.5)
pub const BRACE_DRAG: f32 = 0.7; // fraction of move speed lost at full brace (immobilize cost)
pub const SPRINT_BOOST: f32 = 0.6; // max burst-speed mult added at full sprint
pub const SPRINT_COST: f32 = 1.5; // extra energy/sec at full sprint
pub const SPRINT_FATIGUE: f32 = 0.8; // extra fatigue/sec weight at full sprint
pub const R_KILL: f32 = 1.5; // learn reward for successful kill (reinforce attack output)
pub const R_DEFEND: f32 = 0.8; // learn reward for surviving attack while bracing (reinforce defend output)
pub const R_WASTE: f32 = -0.3; // learn penalty for committed attack that landed nothing
pub const EAT_GATE: f32 = 0.3; // out[4] threshold to ingest. BELOW fresh-net 0.5 baseline so founders feed before learning.
// Kin-based social need (herd instinct). Near genetically-SIMILAR creatures (kin) satisfies social
// creature + protects from predators (herd vigilance); ISOLATION drains energy (loneliness). Trade-off:
// social creatures must stay with kind (constrains roaming) but gain safety; asocial loners roam free but
// hunting loner finds no herd safety + (if social) starves of loneliness.
pub const SOCIAL_RADIUS: f32 = 13.0; // distance kin count as company (wide: easy to satisfy)
pub const SOCIAL_SIM: f32 = 0.7; // max signature distance to count as KIN (smaller = stricter species)
pub const SOCIAL_TARGET: f32 = 2.0; // just 2 kin nearby satisfies (avoids Allee death-spiral at low density)
pub const SOCIAL_COST: f32 = 0.6; // energy/sec loneliness drain at full social gene + full isolation. MILD: flavor pressure + herd benefit, NOT pop killer (strong drain spirals spread-out pop to extinction).
pub const SOCIAL_SAFETY: f32 = 0.5; // max predation-success reduction for prey surrounded by kin (herd safety). Softened from 0.7 so active defense (brace) competes with passive herd-hiding.
// Body collision (M4): creatures solid, don't pass through each other. Two bodies whose collision radii
// overlap get soft tangential SHOVE apart (no hard stacking) + pay jostle energy cost scaled by
// penetration depth. SOCIAL creatures crowd-tolerant (herd animals pack tight) so cost is x(1-social):
// high-social herder huddles ~free, low-social loner drained when bumped -> personal-space pressure
// complements kin-herd benefit (herders cluster, loners spread).
pub const COLLIDE_R: f32 = 0.9; // collision radius mult on visual body half-width (just inside silhouette)
pub const COLLIDE_COST: f32 = 0.5; // energy/sec jostle drain per unit overlap at full isolation (MILD, like loneliness)
pub const SEPARATION_STRENGTH: f32 = 0.5; // fraction of penetration shoved out per tick (soft, settles over a few ticks)
pub const SEED_VIA_GUT: f32 = 0.5; // max chance (x quality) eaten plant disperses an offspring (13)
pub const PLANT_START_MASS: f32 = 0.6;
pub const PLANT_MIN_MASS: f32 = 0.15; // below this a grazed plant fully consumed (carrot eaten whole)

// --- grass: render-only whole-planet ground cover (NOT in food scan -> cheap at high counts). Edible
// only as thin POSITION-based fallback (live_step): hungry creature on grass-bearing soil nibbles. ---
pub const GRASS_CAP: usize = 8000; // target tuft count for whole-planet cover (render only; size set at attach)
pub const GRASS_START_MASS: f32 = 0.4;
pub const GRASS_HAB_MIN: f32 = 0.25; // min plant_habitability to seed/keep grass = "soil capable of plants"
pub const GRASS_GRAZE: f32 = 12.0; // energy/sec HUNGRY grazer nibbles from grassy soil (x habitability x herbivory). Tuned grazer lives off grass alone.

// --- seaweed/kelp: OCEAN analog of grass. Own cap + lifecycle (seaweed_step); blankets submerged band
// (above abyssal floor) -> reliable food carpet for swimmers (position-based ocean graze). ---
pub const SEAWEED_CAP: usize = 3500; // target frond count across submerged band (ocean ~half the planet)
pub const SEAWEED_START_MASS: f32 = 0.5;
pub const SEAWEED_GRAZE: f32 = 26.0; // energy/sec HUNGRY SUBMERGED grazer nibbles from kelp band (x depth x herbivory). Raised 20->26 (balance harness): aquatic grazers were net-marginal -> couldn't sustain. Richer kelp feeding lets fish breed past floor.
// Grazing FORAGE nutrient: grass + seaweed each carry ONE fixed nutrient axis, so grazer can TUNE gut
// (uptake at that index) to live off carpet alone: specialty, not free lunch. Graze refills that reserve
// x creature's uptake-match, so mismatched gut starves of deficiency even while energy is fine.
pub const GRASS_FORAGE_IDX: usize = 0;   // grass nutrient axis (land grazers tune uptake[0])
pub const SEAWEED_FORAGE_IDX: usize = 1; // kelp nutrient axis (sea grazers tune uptake[1])
pub const GRAZE_NUTRIENT: f32 = 0.30;    // reserve refill/sec per unit uptake while grazing (> NUTRIENT_USE so tuned gut stays fed)
pub const GRAZE_FULL: f32 = 38.0;        // grazing tops energy to here (ABOVE REPRO_THRESHOLD 30) so tuned grazer builds breeding surplus, not just clings at threshold

// --- rocky land: scattered boulders (render-only dressing). Spawned once on rocky highland so rocky
// terrain reads as field of stone with grass between rocks. Static -> no per-frame cost. ---
pub const ROCK_SCATTER: usize = 1200; // boulders scattered across rocky ground worldwide (denser the rockier)
pub const ROCK_GRASS_FRAC: f32 = 0.15; // chance rocky-ground sample still seeds thin grass tuft -> sparse grass between rocks, not bare stone
pub const ROCK_GRASS_HAB: f32 = 0.28;  // survival-habitability floor for grass on rocky ground (keeps sparse rocky tufts alive, not churning)

// --- trees: long-lived, near-uneatable plants ---
pub const N_TREES: usize = 240; // initial trees (whole-planet seeding, scattered worldwide)
pub const TREE_CAP: usize = 480; // max trees (whole-planet forests; ambient repro fills toward this)
pub const TREE_MATURITY: f32 = 14.0; // trees grow large before reproducing
pub const P_TREE_REPRO: f32 = 0.004; // slow reproduction (long-lived, sparse)
pub const TREE_DENSITY_R: f32 = 18.0; // trees self-limit clustering within this radius
pub const TREE_MAX_LOCAL: usize = 4; // max trees within TREE_DENSITY_R before tree stops seeding nearby
pub const TREE_BITE_MASS: f32 = 2.5; // mass creature strips per feeding (tree survives + regrows)
pub const TREE_MIN_MASS: f32 = 1.0; // below this a fruit tree over-eaten and dies
pub const TREE_GROWTH_SCALE: f32 = 0.4; // trees grow slowly: scales growth rate down (long-lived, gradual)
// Tree SOIL response: tree grows faster + to BIGGER final size on good ground: nutritious (fertile) AND
// moisture SWEET SPOT (wet enough but not waterlogged). Survival stays moisture-immune; this only shapes
// growth speed + max size, so good soil makes visibly larger trees.
pub const TREE_WET_OPT: f32 = 0.5;     // ideal effective moisture for tree (wet enough, not too wet)
pub const TREE_WET_TOL: f32 = 0.5;     // moisture distance from optimum at which growth bonus -> floor
pub const TREE_WET_FLOOR: f32 = 0.35;  // min growth mult off sweet spot (dry/swampy tree still grows, slowly)
pub const TREE_SOIL_SIZE: f32 = 1.2;   // max final-size bonus on ideal soil (up to (1+this)x maturity = 2.2x bigger)
// Fruit-tree life-history (evolvable height + trade-offs that bound it). Tree height is a gene drifting
// over gens within [TREE_HEIGHT_MIN, 1.0] (wide range, never taller than today's max). Three pressures
// shape it: (1) reach: creature reaches fruit tree only if its height + TREE_REACH_MARGIN >= tree height,
// so taller trees feed fewer creatures; (2) dispersal-on-eat: being eaten spreads seeds FARTHER
// (animal-carried), so reachable tree out-reproduces unreachable one; (3) mass-nutrition cost: bulkier
// tree dilutes fruit (less nutrient/bite). Net: too tall trades away dispersal -> height settles at
// interior optimum.
pub const TREE_HEIGHT_MIN: f32 = 0.4; // shortest a freshly-seeded tree starts (then evolves freely 0..1)
pub const TREE_REACH_MARGIN: f32 = 0.4; // creature reaches tree if creature.height + this (+branches) >= tree.height
pub const BRANCH_REACH: f32 = 0.7; // full branches extend tree reachable-from height down by this much
pub const TREE_MASS_NUTRI: f32 = 0.5; // at full maturity tree fruit is (1-this)x as nutritious (mass dilutes nutrition)
pub const P_TREE_EAT_DISPERSE: f32 = 0.03; // per-grazed-tick chance eaten fruit tree disperses seed (animal-carried)
pub const TREE_EAT_SPREAD_MULT: f32 = 2.5; // animal-carried seeds travel this much farther than wind-fall

// --- plant dispersal genetics (how seed_weight / windborne / clonal / fruiting shape where offspring land) ---
// Effective seed dispersal = genome `spread` x wind stretch x seed-weight drag. Light + windborne flies
// far (dandelion); heavy drops near parent (acorn). Animal-carried (endozoochory) goes farther still, but
// toxic fruit eaten less -> toxic plants stay clustered. Clonal is separate short-range runner path.
pub const WIND_RANGE: f32 = 1.6;            // full windborne stretches dispersal +160% (2.6x reach)
pub const SEED_DRAG: f32 = 0.6;             // full seed_weight shortens dispersal to 40% (heavy seed drops near parent)
pub const P_PLANT_EAT_DISPERSE: f32 = 0.06; // per-grazed-tick chance fruiting plant seed animal-carried far
pub const PLANT_EAT_SPREAD_MULT: f32 = 2.5; // animal-carried plant seeds travel this much farther than wind-fall
pub const P_CLONAL: f32 = 0.012;            // per-tick clonal ramet chance scale (x clonal gene), mature plant
pub const CLONAL_RADIUS: f32 = 2.5;         // runner/ramet sprouts this far from parent (dense local patch)
// cling (epizoochory): passing animal snags burr + carries far, even from inedible plant. Abstracts animal
// traffic as flat per-tick chance (no proximity scan, matches cheap-ecology-gene convention).
pub const P_CLING: f32 = 0.05;              // per-tick chance scale mature plant seed hitches a ride (x cling)
pub const CLING_SPREAD_MULT: f32 = 3.0;     // fur-borne seed travels this much farther than wind-fall
// hydrochory: seed from plant AT/NEAR water floats + rides far. near_water tapers 1 at/below sea level to
// 0 a short band above, so only coastal + aquatic plants get long water dispersal.
pub const HYDRO_RANGE: f32 = 4.0;           // full hydrochory at waterline stretches dispersal +400%
pub const HYDRO_COAST_BAND: f32 = 0.06;     // elevation01 band above sea level still counted as near-water
// dormancy (seed bank): fraction of seeds wait DORMANT in soil, germinate later -> patch wiped by
// fire/drought/grazing re-greens from buried bank. Bank persists independent of surface plants.
pub const DORMANCY_FRAC: f32 = 0.7;         // at dormancy=1, this fraction of seeds go to bank instead of sprouting
pub const SEED_BANK_CAP: usize = 6000;      // max buried seeds tracked (drop new ones when full)
pub const DORMANT_TICKS_MIN: u32 = 200;     // shortest a seed waits buried before germinating
pub const DORMANT_TICKS_MAX: u32 = 1600;    // longest a seed waits buried (staggers recruitment over time)
// plant + tree mating (--mating mode, shared with creatures): seeding plant crosses with nearest
// genetically-similar plant in range (assortative -> reproductive isolation -> emergent plant species);
// no compatible neighbor -> falls back to selfing (clone), so sparse patches still set seed.
pub const PLANT_MATE_RADIUS: f32 = 9.0;     // pollen/pollinator reach: mate must be within this distance
pub const PLANT_SPECIES_SIM: f32 = 0.18;    // max genetic distance to be same species (cross-compatible)

// --- fruit + fermentation (Phase B): forageable source of FAST energy ---
// Fruit trees drop fruit; fallen fruit + dead-plant detritus ferment over Rot clock. Eating in
// fermentation window (FERMENT_START..FERMENT_END as fraction of ROT_GONE) yields FAST energy (ethanol).
// Before: fresh -> sugar. After: spoiled (toxic, near-zero yield), then rot_step gone.
pub const P_FRUIT_DROP: f32 = 0.012;        // per-tick chance mature fruit tree drops a fruit
pub const FALLEN_FRUIT_MASS: f32 = 0.8;     // mass of dropped fruit (bite-sized ground food)
pub const FERMENT_START: f32 = 0.25;        // rot fraction where fermentation begins (before: fresh sugar)
pub const FERMENT_END: f32 = 0.70;          // rot fraction where it spoils (after: toxic, near-zero yield)
pub const FRUIT_FAST_GAIN: f32 = 22.0;      // fast energy per (mass*nutrient) from fermented fruit
pub const DETRITUS_FAST_GAIN: f32 = 5.0;    // fast energy from fermented detritus (<< fruit: poor, scrappy)
pub const FERMENT_TOX_DETRITUS: f32 = 0.85; // toxicity scale of fermented detritus (high: rotten sludge)
// Fruit ripeness (seed development). Fallen fruit Rot clock also models RIPENING: before RIPEN_FRAC fruit
// is UNRIPE: seed not viable (eating disperses NOTHING, seed destroyed), flesh bitter + carries plant
// fruit_toxicity (protects developing seed). Past RIPEN_FRAC fruit ripe: sweet, detoxified, eating
// DISPERSES seed (animal-carried). Tension: too-palatable unripe fruit eaten early (lost repro);
// too-toxic ripe fruit poisons dispersers -> interior optimum.
pub const RIPEN_FRAC: f32 = 0.12;       // rot fraction at which seed becomes viable + fruit sweetens
pub const UNRIPE_YIELD: f32 = 0.25;     // sugar UNRIPE fruit yields vs ripe (low: not worth eating early)

// --- rot chain (P3): dead creature -> carrion -> poison -> gone ---
pub const CARRION_KIND: u8 = 0; // meat = food type 0 (couples to diet expr only via sensing, not digestion)
pub const CARRION_MASS: f32 = 3.0; // meaty chunk: worth scavenging while fresh
pub const CARRION_NUTRIENT: f32 = 0.9; // fresh meat energy-dense
pub const ROT_GONE: u32 = 900; // ticks from death to full decomposition (~15s sim); viz reads it for color
pub const TOXIN_MAX: f32 = 9.0; // toxin magnitude from eating fully-rotten carrion (feeds toxic_load, see TOX_LOAD_*)

// --- moisture pressure + rain cycle (P3) ---
pub const DETRITUS_NUTRIENT: f32 = 0.3; // dead vegetation: poor food fresh, rots to poison
pub const MOISTURE_TOLERANCE: f32 = 0.3; // mismatch under this harmless
pub const MOISTURE_KILL: f32 = 0.012; // per-tick death scale for mismatch beyond tolerance
pub const HABITAT_KILL: f32 = 0.03; // per-tick death scale in poor sites (deep water / arid desert)
// Drowning: plant underwater needs aquatic `wet` gene to survive. Mortality scales submersion (how far
// below sea level) x (1 - wet): land flora (low wet) drowns, aquatic flora (high wet) thrives -> wet gene
// splits land vs aquatic plants. Trees can't be aquatic at all (DROWN_TREE).
pub const DROWN_KILL: f32 = 0.06;   // per-tick death scale for non-aquatic plant fully submerged
pub const DROWN_TREE: f32 = 0.5;    // per-tick death for tree in water (trees land-only, no kelp forests)
pub const SEASON_FREQ: f32 = 0.4; // seasonal wet/dry oscillation speed (radians per generation)
// --- new plant genes (real-life flora: aquatic depth, drought, climate niche, ecology) ---
pub const WATER_LIGHT_ATTEN: f32 = 0.85; // sunlight lost over full water column: deep sea dim, so only
                                         // shade plants (low light_pref, e.g. kelp) thrive deep; sun-lovers
                                         // (lily) need shallows -> deep-vs-shallow sun gradient.
pub const CLIMB_LIGHT: f32 = 0.4;        // vine light bonus: climber reaches better-lit canopy (lf += climb*this)
pub const SUCC_BUFFER: f32 = 0.9;        // succulence drought buffer: water storage offsets DRY-side moisture stress
pub const TEMP_FLOOR: f32 = 0.45;        // floor on plant thermal growth factor (off-niche grows slow, not zero)
pub const TEMP_KILL: f32 = 0.01;         // per-tick death scale for climate-niche mismatch beyond tolerance
pub const TEMP_TOL: f32 = 0.3;           // temp_pref mismatch under this harmless
// Hard freeze (instant kill): below this absolute temp GROUND frozen solid -> no plant or tree survives
// (polar ice core barren). Set just inside solid-ice core (below temp<0.25 fuel cutoff) so tundra/frost-
// edge band (0.25..0.34) keeps cold-adapted flora (alpine cushion, moss); only genuinely frozen cap kills
// outright. Independent of temp_pref: even cold-lover freezes on ice.
pub const FREEZE_TEMP: f32 = 0.22;       // base_temperature below this = frozen -> plants/trees die instantly
// Cold gates for ground-cover (grass) + ocean carpet (seaweed), which lack the plant_step freeze kill. Match
// the biome_color render bands so green stops where white starts. Grass fades across ICE_CAP_TEMP..FREEZE_TEMP
// (tundra fringe) then hard-culls; seaweed culled below SEA_ICE_TEMP so no kelp under rendered pack ice.
pub const ICE_CAP_TEMP: f32 = 0.34;      // land ice-cap render onset: grass fully faded by here
pub const SEA_ICE_TEMP: f32 = 0.30;      // ocean pack-ice render onset: seaweed gone below this (cold kelp OK above)
// Trees get climate niche too, but SOFTER than ground plants (long-lived, deep roots): wider harmless band
// + gentler per-tick kill, so tree dies back only well off thermal niche (frozen pole / desert heat), not
// at first mismatch. Growth still tapers off-niche via shared TEMP_FLOOR factor.
pub const TREE_TEMP_TOL: f32 = 0.4;      // tree temp_pref mismatch under this harmless (wider than plants)
pub const TREE_TEMP_KILL: f32 = 0.004;   // per-tick death scale for tree beyond its (wide) thermal tolerance
pub const NFIX_RATE: f32 = 0.6;          // soil fertility/sec full nitrogen-fixer (legume) adds at its spot
pub const DESICCATE_KILL: f32 = 0.1;     // per-tick death for aquatic plant (high wet) stranded on dry land
                                         // (mirror of drown): aquatic flora needs water, can't carpet land
// Dynamic ground water (rain cycle): wetness layer on TOP of static terrain moisture. Sun evaporates
// (faster at noon), storms refill. Rocky cells shed runoff (no gain); grassy cells soak it up -> after
// heavy rain low-lying grassland turns wet + favors wet-liking plants, which then dry out + get stressed
// next drought. Drives temporal selection on plant `wet`.
// (Rain now LOCAL + cloud-driven: sphere::rain_at / weather_step. No global storm onset/decay.)
pub const RAIN_RATE: f32 = 0.8;  // ground-water added/sec at full rain on fully-absorbing (grassy) cell
pub const EVAP: f32 = 0.06;      // ground-water evaporated/sec at noon (scaled by sunlight, x current water)
pub const WET_GAIN: f32 = 0.45;  // how much saturated ground water adds to plant effective local moisture
pub const WET_GROWTH: f32 = 0.3; // growth-rate boost from watered ground (rain visibly greens land)
// --- dynamic climate (geological): THIRD moisture layer, slower than GroundWater. Per-cell long-term
// moisture relaxes toward drifting target (regional rain propensity minus aridity) over MONTHS of sim
// time, so persistently-dry regions turn desert + persistently-wet regions turn lush, wet belt slowly
// migrates -> deserts + rainforests form, persist, move. Shared sim+render (deterministic).
pub const CLIMATE_RATE: f32 = 0.0003; // climate relax rate /sec toward target. tau=1/rate ~3333 sim-sec ~83 days (one day=2400 ticks=40s); geological
pub const CLIMATE_VEG: f32 = 1.0; // how strongly climate moisture (vs static) drives plant growth/mortality (1=full)
// (spatial drift consts CLIMATE_DRIFT + CLIMATE_SPEED live in sphere.rs alongside other field consts)
// Lightning -> fire: during storm, lightning strikes ignite fire-grid cell. Fire spreads through DRY
// vegetation, burns plants/trees + hurts creatures caught in it, doused by rain/wet ground, leaves fertile
// ash. Ties weather to dramatic, visible ecological disturbance + renewal cycle.
pub const P_LIGHTNING: f32 = 0.02; // per-tick strike chance while storm active (several per storm; most fizzle on wet ground, dry-fuel ones catch)
pub const LIGHTNING_RAIN: f32 = 0.4; // rain intensity above which lightning can strike (storms only)
pub const FIRE_WET_MAX: f32 = 0.45; // fire only ignites/spreads into cells drier than this (wet ground won't burn)
pub const FIRE_DECAY: f32 = 0.18; // natural burnout per sec (raised: fires die out sooner, don't run away)
pub const FIRE_DOUSE: f32 = 2.0; // extra burnout per sec per unit local ground water (rain puts fire out)
pub const FIRE_SPREAD: f32 = 0.18; // base spread rate per sec to adjacent dry cells (lowered + now scaled by neighbor fuel density in fire_step, so sparse vegetation barely carries fire)
pub const FIRE_ASH: f32 = 2.5; // soil fertility deposited per sec by burning cell (ash enriches regrowth)
pub const FIRE_BURN_ASH: f32 = 1.2; // EXTRA soil fertility when plant/tree burns UP (biomass -> ash), x mass; trees deposit more (bigger biomass). Burned ground regrows richer.
pub const FIRE_KILL: f32 = 0.4; // fire intensity at which plant/tree in cell burns up
pub const FIRE_DAMAGE: f32 = 9.0; // energy/sec creature loses standing in fire
// Defense also taxes REPRODUCTION, not just growth: at carrying capacity growth cost toothless, so armored
// plants pegged defense ~free; penalizing repro lets cheaper plants win cap slots -> interior def.
pub const DEF_REPRO_COST: f32 = 0.7; // armored plant (def=1) reproduces at (1-0.7)=30% base rate

// --- nutrient closed loop (M5): death -> soil fertility -> richer food ---
pub const SOIL_RES: usize = 32; // fertility grid cells per axis
pub const SOIL_DECAY: f32 = 0.999; // fertility leaches/taken up each tick
pub const DECOMP_FERT: f32 = 3.0; // fertility released on full decomposition (x corpse nutrient)
pub const DEATH_FERT: f32 = 0.5; // fertility deposited IMMEDIATELY where creature/plant dies (body enriches ground there); plants grow better on death sites, on top of slower decomposition release. Kept gentle: strong death->fertility->plant-boom->creature-boom->crash loop is destabilizing positive feedback.
pub const FERT_GROWTH: f32 = 0.6; // max growth-rate bonus from saturated soil
pub const FERT_CAP: f32 = 1.5; // fertility level at which growth bonus saturates
pub const PLANT_REPRO_FRAC: f32 = 0.5; // fraction of mass kept after budding off a child

// --- nutrients + regulatory diet genome (Phase C, see 14/05) ---
// 10 nutrients. Plants produce sparse profile (x soil fertility); meat balanced. Creatures absorb per
// uptake genes into reserves, which deplete with use. Master expression gene (reserves vs uptake demand)
// gates energy extraction; unmet demanded nutrient = deficiency -> growth-load (soft).
pub const RESERVE_REQ: f32 = 0.6;       // reserve level at which nutrient demand fully satisfied
pub const RESERVE_CAP: f32 = 1.5;       // max stored per nutrient (bank a little, not unlimited)
pub const MASTER_FLOOR: f32 = 0.45;     // floor on master expression: digestion never below this (soft gradient, not lethal)
pub const NUTRIENT_USE: f32 = 0.02;     // per-sec reserve depletion per unit uptake (slow: reserves are buffer, not fuse)
pub const NUTRIENT_ABSORB: f32 = 1.0;   // fraction of delivered nutrient (x uptake) entering reserves on eat
pub const UPTAKE_OVERHEAD: f32 = 0.08;  // energy/sec upkeep per unit total uptake (gentle generalist tax; was lethal at 0.45)
pub const DEFICIT_G: f32 = 0.15;        // growth-load/sec per unit avg demanded-nutrient deficiency (soft)
pub const SOIL_NUTRI: f32 = 0.8;        // soil fertility boost to plant nutrient output: x(1 - this + this*fert/FERT_CAP)
pub const PLANT_TOX_HIT: f32 = 2.0;     // energy hit per unit plant toxicity eaten (x bite mass fraction)
pub const MEAT_RESERVE: f32 = 0.5;      // flat reserve top-up (all nutrients) from eating meat (balanced tissue)

// --- diet model (--diet): growth-load disease + aging (nutrient mechanics in Phase C block above) ---
pub const G_DECAY: f32 = 0.015; // growth-load shed per tick (recovery when well-nourished)
pub const DISEASE_K: f32 = 0.004; // per-tick disease mortality per unit growth-load
pub const AGE_HAZARD: f32 = 0.02; // late-life mortality ceiling (decelerates -> ~plateau)
pub const AGE_SCALE: f32 = 2400.0; // ticks; age at which aging hazard reaches half its ceiling (longer lifespans)
// Longevity gene: effective lifespan = AGE_SCALE * (0.4 + 1.2*longevity) (longevity 0.5 = baseline x1.0).
// Long-lived body pays extra basal upkeep per unit lifespan above baseline -> slow-life (durable but
// costly) vs fast-life (cheap, breed young, die young) trade-off. Default longevity 0.5 = no extra cost.
pub const LONGEVITY_COST: f32 = 1.1; // energy/sec extra basal at max lifespan multiplier

// --- generational GA ---
pub const ELITE_FRAC: f32 = 0.3;
pub const MUT_RATE: f32 = 0.12;
pub const MUT_STD: f32 = 0.3;

// --- lifetime learning (see 04 + 09) ---
pub const LEARN_RATE: f32 = 0.04;
pub const R_APPROACH: f32 = 0.6; // reward per unit closer to nearest food this tick
pub const R_EAT: f32 = 1.0; // bonus reward on tick food eaten

// --- M4 creature expansion: physiology + morphology gene costs/effects (see plan + 02/03) ---
// size = energy use: basal upkeep scales SUPER-LINEARLY with body size (allometry), so big body markedly
// hungrier per mass. size still buys storage/combat/reach (above) -> big = powerful but dear.
pub const SIZE_BASAL_EXP: f32 = 2.0; // allometric exponent: size basal term = SIZE_BASAL * size^this
// detox (liver): clears toxic_load; costs basal upkeep.
pub const DETOX_COST: f32 = 0.5; // energy/sec basal at full detox (running a big liver costs)
// toxic load: ingested toxins (plant tox, rotten meat, fermented spoilage, venomous prey, protein excess)
// accumulate as LOAD draining energy + driving disease + death hazard, cleared slowly (faster with detox).
// Replaces old instant-only toxin energy hit -> poisons build + linger like real toxic load.
pub const TOX_LOAD_GAIN: f32 = 1.0;      // load gained per unit ingested toxin
pub const TOX_LOAD_CAP: f32 = 12.0;      // max accumulated toxic load (hard ceiling)
pub const TOX_CLEAR_BASE: f32 = 0.15;    // load cleared/sec at zero detox (baseline metabolism)
pub const TOX_CLEAR_DETOX: f32 = 0.6;    // extra load cleared/sec at full detox
pub const TOX_LOAD_DRAIN: f32 = 0.25;    // energy/sec drained per unit toxic load (feeling sick)
pub const TOX_LOAD_G: f32 = 0.02;        // growth-load (disease) accrued/sec per unit toxic load
pub const TOX_LOAD_HAZARD: f32 = 0.0008; // death/sec per unit toxic load (acute poisoning)
// rabbit starvation: carcass usable ENERGY is its FAT (carried per-carcass = prey fatness at death). LEAN
// meat mostly PROTEIN, converting protein to usable energy/fat needs CARBS (eater's sugar). So eating lean
// prey with no carbs yields little energy AND dumps unconvertible protein as toxic load (ammonia) ->
// obligate carnivore on lean kills starves + poisons itself; fatty prey or plant carbs avoid it. `gut` =
// carnivory scales overall meat extraction (herbivore gut handles meat poorly).
pub const PROTEIN_FLOOR: f32 = 0.35; // meat-extraction fraction pure herbivore (carnivory 0) still manages
pub const PROTEIN_CARN: f32 = 0.65;  // extra meat-extraction fraction at full carnivory (-> full at 1.0)
pub const PROTEIN_TOX: f32 = 0.06;   // toxic load per unit UNCONVERTIBLE protein (lean meat eaten without carbs)
// pelt (hair/fur): insulation. Cuts COLD side of thermal mismatch; adds HEAT-side cost in hot places,
// drags in water, costs little basal upkeep.
pub const PELT_COLD_RELIEF: f32 = 0.7; // fraction of cold-side temp cost removed at full pelt
pub const PELT_HEAT_COST: f32 = 0.6;   // energy/sec extra at full pelt in hottest places (overheating)
pub const PELT_WATER_DRAG: f32 = 1.2;  // energy/sec extra at full pelt fully in water (waterlogged coat)
pub const PELT_UPKEEP: f32 = 0.2;      // energy/sec basal at full pelt (growing + carrying a coat)
// armor: lowers predation success against it (defense-only); costs move + basal upkeep.
pub const ARMOR_DEF: f32 = 2.5;   // added to prey combat as DEFENSE-only at full armor (hard to kill)
pub const ARMOR_MOVE: f32 = 0.8;  // move-cost mult add at full armor (heavy plates to push)
pub const ARMOR_BASAL: f32 = 0.5; // energy/sec basal at full armor
// venom: toxic flesh deters predators -> predator eating venomous prey gains far less (sickening kill).
pub const VENOM_DETER: f32 = 0.9;   // fraction of predation gain removed at full prey venom
pub const VENOM_UPKEEP: f32 = 0.3;  // energy/sec basal at full venom (making toxins costs)
// limbs: more legs = land traction (small ground-speed bonus) but more move cost per limb. Gene maps to
// rendered leg COUNT of LIMB_MIN..LIMB_MIN+LIMB_SPAN (Phase 4 visuals).
pub const LIMB_MIN: f32 = 2.0;       // legs at limbs gene 0
pub const LIMB_SPAN: f32 = 6.0;      // legs added across gene range (-> 8 legs at gene 1)
pub const LIMB_TRACTION: f32 = 0.25; // max land-speed bonus fraction at full limbs
pub const LIMB_MOVE_COST: f32 = 0.5; // move-cost mult add at full limbs (more legs to drive)
// climb (arboreal): nimble -> evades predators; reaches fruit trees w/o tall body; arboreal build wastes
// energy on open flat ground.
pub const CLIMB_EVADE: f32 = 0.35;    // max predation-success reduction at full climb (agile escape). Softened from 0.5 so fleeing/hiding doesn't dominate active fighting + defense.
pub const CLIMB_REACH: f32 = 0.6;     // effective tree-reach height added at full climb (climbs to fruit)
pub const CLIMB_FLAT_COST: f32 = 0.6; // energy/sec at full climb on flat non-rocky ground (arboreal misfit)
// eyes: small detection bonus (effective sensor range) for per-eye upkeep. Gene maps to rendered eye COUNT
// of EYE_MIN..EYE_MIN+EYE_SPAN (Phase 4 visuals).
pub const EYE_MIN: f32 = 1.0;         // eyes at gene 0
pub const EYE_SPAN: f32 = 5.0;        // eyes added across gene range (-> 6 eyes at gene 1)
pub const EYE_SENSE_BONUS: f32 = 0.3; // effective sensor-range mult add at full eyes (+30%)
pub const EYE_COST: f32 = 0.3;        // energy/sec basal at full eyes (eyes metabolically pricey)
// head: bigger head houses brain more efficiently (cuts per-neuron BRAIN_COST) but adds basal mass.
pub const HEAD_BRAIN_RELIEF: f32 = 0.5; // fraction of BRAIN_COST removed at full head (roomy braincase)
pub const HEAD_BASAL: f32 = 0.4;        // energy/sec basal at full head (carrying a big head)
// magnetoreception: magnetite organ + neural processing for magnetic-latitude + compass brain inputs.
// Scaled by mag_expression (`magneto` gene switch), so sense only worth keeping where it pays off.
pub const MAG_COST: f32 = 0.25;         // energy/sec basal at full magneto expression (no free lunch)

// --- M4 brain inputs + need-for-shade (Phase 3) ---
// threat sense: nearby creature with enough COMBAT edge over you registers as predator -> brain can learn
// to flee (threat_dist/threat_bearing inputs).
pub const THREAT_RADIUS: f32 = 22.0; // bigger-combat creature within this sensed as threat
pub const THREAT_MARGIN: f32 = 0.4;  // combat edge neighbor needs over you to register as threat
// need for shade: in hot SUN (daylight x local temp) exposed creature overheats + burns energy; standing
// in canopy shade (near tree) or being heat-tolerant build relieves it. `shade` brain input lets brains
// learn to seek trees at midday.
pub const SHADE_RADIUS: f32 = 6.0;   // within this of a tree counts as canopy shade
pub const HEAT_SUN_COST: f32 = 1.4;  // energy/sec scale of open-sun heat stress in hot places
pub const HEAT_COMFORT: f32 = 0.45;  // sun x temp below this comfortable (no heat cost)
pub const SHADE_RELIEF: f32 = 0.85;  // fraction of heat stress removed standing in full shade
