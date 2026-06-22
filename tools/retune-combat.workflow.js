export const meta = {
  name: 'retune-combat',
  description: 'Retune the 10 creature niches UNDER predator pressure so fighting + active defense emerge (6-output brains), then rebuild the showcase seed from champions',
  whenToUse: 'After adding the attack/defend/eat/sprint brain outputs: evolve creatures that fight + defend (not just flee) by tuning each niche against a seeded predator cohort, then harvest a combat-capable population seed.',
  phases: [
    { title: 'build', detail: 'compile release binary once' },
    { title: 'tune', detail: 'one agent per niche: prey cohort + predator cohort, tune prey to survive + breed + USE combat' },
    { title: 'synthesize', detail: 'merge champions into a fresh evolved-continuous.json, smoke it' },
  ],
}

const REPO = '/home/marc/Documents/Github/evolvarium'
const BIN = './target/release/evolvarium'
const SEED_OUT = 'evolved-continuous.json'

// Same 10 creature niches as the tournament, each now facing a PREDATOR cohort so the new attack/defend
// outputs have real pressure to act on. food = plant_cohort archetypes ("TREE" => a {count:4,tree:true} spec).
const NICHES = [
  { name: 'warm-generalist',  band: [0.1, 0.45], wetness: 0.6,  food: ['BerryBush', 'Clover'],     hint: { temp_pref: 0.7, size: 0.35 } },
  { name: 'tropical-forager', band: [0.0, 0.35], wetness: 0.85, food: ['Fern', 'BerryBush'],       hint: { temp_pref: 0.78, size: 0.32 } },
  { name: 'savanna-grazer',   band: [0.1, 0.4],  wetness: 0.45, food: ['Clover', 'Wildflower'],    hint: { temp_pref: 0.7, size: 0.6, metab: 0.3 } },
  { name: 'temperate-omni',   band: [0.4, 0.75], wetness: 0.6,  food: ['BerryBush', 'Clover'],     hint: { temp_pref: 0.5, size: 0.4 } },
  { name: 'cold-pelted',      band: [0.85, 1.25], wetness: 0.45, food: ['Moss', 'AlpineCushion'],  hint: { temp_pref: 0.25, pelt: 0.7, size: 0.45 } },
  { name: 'polar-survivor',   band: [1.0, 1.4],  wetness: 0.4,  food: ['AlpineCushion', 'Moss'],   hint: { temp_pref: 0.15, pelt: 0.85, adiposity: 0.6 } },
  { name: 'arid-desert',      band: [0.5, 0.9],  wetness: 0.15, food: ['Cactus', 'Thistle'],       hint: { temp_pref: 0.8, adiposity: 0.7, size: 0.35 } },
  { name: 'highland-climber', band: [0.4, 0.9],  wetness: 0.4,  rocky: true, food: ['Thistle', 'Clover'], hint: { alpine: 0.7, size: 0.3 } },
  { name: 'tree-climber',     band: [0.3, 0.7],  wetness: 0.6,  food: ['Clover', 'TREE', 'TREE'],  hint: { climb: 0.75, size: 0.2, height: 0.2 } },
  { name: 'aquatic-forager',  band: [0.2, 0.6],  wetness: 0.95, aquatic: true, food: ['Waterlily', 'Kelp'], hint: { swim: 0.85, temp_pref: 0.6 } },
]

const TUNE_SCHEMA = {
  type: 'object',
  required: ['niche', 'result_path', 'creature_survival', 'survived', 'combat_emerged', 'frictions'],
  properties: {
    niche: { type: 'string' },
    result_path: { type: 'string', description: 'path to the BEST result.json (harvested into the seed)' },
    creature_survival: { type: 'number', description: 'final survivors / started (>=1 = lineage bred + sustained UNDER predation)' },
    survived: { type: 'number' },
    combat_emerged: { type: 'boolean', description: 'did fighting/defense emerge? (trait_drift pulls bite/armor/carnivory UP, survival held vs predators without pure-flee genes)' },
    combat_evidence: { type: 'string', description: 'one line: which combat genes drifted and how (e.g. "bite 0.3->0.6, armor 0.2->0.5; prey stood + fought")' },
    rounds: { type: 'number' },
    frictions: { type: 'array', items: { type: 'string' } },
  },
}

phase('build')
await agent(`Run \`cargo build --release\` in ${REPO}. Report only "ok" or the error tail.`, { label: 'build', phase: 'build' })

phase('tune')
const results = await pipeline(NICHES, (niche) =>
  agent(
    `You tune a cohort of CREATURES to survive + breed UNDER PREDATOR PRESSURE, so that FIGHTING and ACTIVE
DEFENSE emerge (not just running/hiding). Use ONLY the scenario JSON interface + run the binary. Do NOT edit
Rust/source/config. Work in ${REPO}.

NICHE: ${JSON.stringify(niche)}

WHAT'S NEW (6-output brains): the creature brain now has outputs [thrust, turn, ATTACK, DEFEND, EAT, SPRINT].
- ATTACK (out[2]): the creature CHOOSES to hunt; attacking costs energy land-or-miss; a kill = big fat windfall.
- DEFEND (out[3]): bracing raises effective defense vs an attacker but immobilizes (can't forage while braced).
- EAT (out[4]): ingestion is now a choice (founders default ~0.5, gate 0.3, so they still eat).
- SPRINT (out[5]): burst speed to CHASE prey or FLEE, paid in energy + fatigue.
These are learned in-life (reward-modulated) AND selected across generations. Passive escape (climb/herd) was
SOFTENED so active defense + fighting can compete.

THE HARNESS:
- Write a scenario JSON, then run: ${BIN} --scenario=<scn.json> --out=<result.json> --seed=<K>. Use 3 seeds (1,2,3), average.
- Scenario fields: seed, ticks (use 4000), target_count (use 12),
    world:{ lat_band:[lo,hi], wetness:0..1, aquatic:bool, rocky:bool },
    plant_cohort:[ { count, archetype:"<Name>"|null, tree:bool, genome:{...} } ]   // the FOOD (abundant: 60+ ground plants)
    creature_cohort:[ <PREY cohort>, <PREDATOR cohort> ]   // creature_cohort is a LIST -> seed BOTH
  Seed FOOD from: ${JSON.stringify(niche.food)} ("TREE" => { count:4, tree:true }).
  PREY cohort (the one you tune): { count:12, reflex:"approach-food", genome:{ ...start from hint ${JSON.stringify(niche.hint)} } }.
  PREDATOR cohort (FIXED pressure, seed it the same every run): { count:3, reflex:"approach-food",
    genome:{ carnivory:0.9, bite:0.85, size:0.7, detox:0.6, temp_pref:<niche temp>, ${niche.aquatic ? 'swim:0.85, ' : ''}${niche.rocky ? 'alpine:0.6, ' : ''}metab:0.45 } }.
  Creature genome is FREE-FORM: size, metab, bite, armor, venom, carnivory, climb, pelt, adiposity, temp_pref,
  swim, alpine, social, limbs, eyes, head, uptake(10), sensors([{angle,range}]), detox, longevity, parental.

- Result creature fields: creature_started, creature_survived, creature_survival (final/started; >=1 = bred +
  sustained), creature_mean_age, creature_mean_energy, creature_mean_master, creature_trait_drift{gene:[seed,survivor]},
  best_creatures. Watch trait_drift on bite/armor/carnivory/venom/size to see if COMBAT is part of the solution.

OBJECTIVE (two goals, both matter):
  1) creature_survival >= 1 for the PREY cohort (it breeds + sustains DESPITE the predators).
  2) FIGHTING/DEFENSE emerges: the survivors don't win purely by fleeing. Look for trait_drift pulling bite,
     armor, carnivory, or size UP, and survival holding even though escape (climb/herd) was softened. A prey
     lineage that fights back (raises bite/armor) or counter-hunts (carnivory up) is the win. Hold niche identity
     (aquatic stays swim-high, cold stays pelted, climber keeps climb).

METHOD (~4-6 rounds): start from the hint + predator cohort, run 3 seeds. If the prey is wiped, give it the means
to survive: raise bite/armor (fight + tank), or size (combat power), or keep some climb/herd but lean into
defense. Read trait_drift each round. Keep the config with the best mean creature_survival WHERE combat genes are
non-trivial (not a pure-flee build). A gene pegging 0/1 across runs is a FRICTION.

DELIVERABLE: leave your single BEST result.json at /tmp/retune-combat-${niche.name}/best.json (re-run the best
config to that exact path). Return JSON per schema: niche, result_path, creature_survival, survived,
combat_emerged (true if bite/armor/carnivory drifted up + survival held without pure-flee), combat_evidence
(one line), rounds, frictions.`,
    { label: `tune:${niche.name}`, phase: 'tune', schema: TUNE_SCHEMA },
  ),
)

phase('synthesize')
const good = results.filter(Boolean).filter((r) => r.survived > 0 && r.result_path)
const fought = good.filter((r) => r.combat_emerged).map((r) => r.niche)
const frictions = results.filter(Boolean).flatMap((r) => (r.frictions || []).map((f) => `[${r.niche}] ${f}`))

const report = await agent(
  `You finalize the combat retune: harvest each niche's survivors into a FRESH population seed. Work in ${REPO}.

1) START FRESH: \`rm -f ${SEED_OUT}\` so the seed is built only from this run's combat-capable champions.
2) MERGE each niche's best result, ONE AT A TIME (sequential, shared file):
     ${BIN} --merge-creatures=<result_path> --snap=${SEED_OUT} --cap=120
   Entries:
${good.map((r) => `     - niche=${r.niche} result_path=${r.result_path} (survival ${r.creature_survival?.toFixed?.(2)}, combat ${r.combat_emerged ? 'YES' : 'no'})`).join('\n')}
3) SMOKE the seeded world (must boot + stay populated, NOT collapse or explode):
     ${BIN} --load=${SEED_OUT} --headless --gens=3
   Confirm it reaches "headless run done"/"continuous headless done" with a non-trivial, stable pop in the last log.
4) Append FRICTIONS to /home/marc/Documents/Github/clients/evolvarium/tuning-frictions.md (append-only, "## F<n>" style):
${frictions.length ? frictions.map((f) => `     - ${f}`).join('\n') : '     (none reported)'}

Return a short plain-text report: seed creature count, smoke result (final pop), how many niches showed combat
emergence (${fought.length}/${good.length}: ${fought.join(', ') || 'none'}), and frictions logged.`,
  { label: 'synthesize', phase: 'synthesize' },
)

return {
  niches_tuned: good.length,
  sustaining: good.filter((r) => r.creature_survival >= 1).map((r) => r.niche),
  combat_emerged: fought,
  frictions,
  report,
}
