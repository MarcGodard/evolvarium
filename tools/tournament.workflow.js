export const meta = {
  name: 'tournament',
  description: 'Competitive 30-agent harness: 10 creature + 20 plant cohorts tune for reproduction + survival across every environment, then a cross-environment gauntlet ranks generalists, winners merge into the seed bank',
  whenToUse: 'Big competitive tuning + friction-finding run. Each agent shepherds one cohort toward a self-sustaining (breeding) lineage in its niche; a gauntlet then finds which champions survive everywhere; winners seed the planet + population.',
  phases: [
    { title: 'build', detail: 'compile release binary once so all agents reuse it (fast sim)' },
    { title: 'compete', detail: '10 creature + 20 plant agents tune cohorts in parallel; reproduction is the qualifying gate' },
    { title: 'gauntlet', detail: 'each qualifying champion runs UNCHANGED across a panel of foreign environments; robustness ranks generalists' },
    { title: 'synthesize', detail: 'merge plant winners into plant-library.json, creature winners into a fresh tournament seed, smoke + gate the showcase seed, log frictions, report the leaderboard' },
  ],
}

const REPO = '/home/marc/Documents/Github/evolvarium'
const BIN = './target/release/evolvarium'

// =============================================================================
// NICHES: cover EVERY environment. Bands are |latitude| in radians (0 = equator
// .. ~1.57 = pole). Each agent owns ONE row + its isolated mini-world(s). These
// are STARTING points; agents adjust genome overrides + (creatures) reflex/brain
// + (within reason) band/food to find a BREEDING, self-sustaining cohort.
// =============================================================================

// 20 plant/tree niches. archetype = starting base; tree = seed as a Tree;
// second_band => MIXED generalist straddling two bands; grazers/fire = pressure.
const PLANT_NICHES = [
  // core land by climate x moisture
  { name: 'tropical-wet',          archetype: 'Fern',          band: [0.0, 0.35], wetness: 0.85 },
  { name: 'tropical-rainforest',   archetype: 'BerryBush',     band: [0.05, 0.4], wetness: 0.9, hint: { light_pref: 0.3 } },
  { name: 'equatorial-savanna',    archetype: 'Wildflower',    band: [0.1, 0.4],  wetness: 0.4 },
  { name: 'temperate-meadow',      archetype: 'BerryBush',     band: [0.4, 0.75], wetness: 0.6 },
  { name: 'temperate-understory',  archetype: 'Fern',          band: [0.45, 0.8], wetness: 0.7, hint: { light_pref: 0.3 } },
  { name: 'boreal-cold',           archetype: 'Moss',          band: [0.8, 1.1],  wetness: 0.5 },
  { name: 'polar-alpine',          archetype: 'AlpineCushion', band: [1.05, 1.42], wetness: 0.4 },
  { name: 'tundra-frostedge',      archetype: 'AlpineCushion', band: [0.95, 1.25], wetness: 0.45 },
  // arid
  { name: 'hot-desert',            archetype: 'Cactus',        band: [0.45, 0.85], wetness: 0.1 },
  { name: 'cold-desert',           archetype: 'Thistle',       band: [0.9, 1.2],  wetness: 0.12 },
  // rocky highland
  { name: 'highland-rocky',        archetype: 'Thistle',       band: [0.4, 0.9],  wetness: 0.4, rocky: true },
  { name: 'alpine-rock-cold',      archetype: 'AlpineCushion', band: [0.9, 1.3],  wetness: 0.35, rocky: true },
  // disturbance pressure
  { name: 'fire-prone-scrub',      archetype: 'Wildflower',    band: [0.3, 0.7],  wetness: 0.35, fire: 0.3 },
  { name: 'grazed-grassland',      archetype: 'Clover',        band: [0.3, 0.7],  wetness: 0.55, grazers: 6 },
  // aquatic
  { name: 'shallow-sunlit',        archetype: 'Waterlily',     band: [0.1, 0.6],  wetness: 0.95, aquatic: true },
  { name: 'deep-kelp',             archetype: 'Kelp',          band: [0.2, 0.8],  wetness: 0.98, aquatic: true },
  { name: 'cold-water',            archetype: 'Kelp',          band: [0.8, 1.2],  wetness: 0.97, aquatic: true },
  // trees
  { name: 'fruit-tree-temperate',  archetype: null, tree: true, band: [0.3, 0.7], wetness: 0.6 },
  { name: 'evergreen-cold',        archetype: null, tree: true, band: [0.8, 1.2], wetness: 0.5 },
  { name: 'tropical-canopy-tree',  archetype: null, tree: true, band: [0.0, 0.4], wetness: 0.85 },
].map((n) => ({ ...n, kind: 'plant' }))

// 10 creature niches. food = plant_cohort archetypes seeded as the cohort's FOOD
// (co-located patch). "TREE" entries => a { count:4, tree:true } food spec.
const CREATURE_NICHES = [
  { name: 'warm-generalist',  band: [0.1, 0.45], wetness: 0.6,  food: ['BerryBush', 'Clover'],     hint: { temp_pref: 0.7, size: 0.3 } },
  { name: 'tropical-forager', band: [0.0, 0.35], wetness: 0.85, food: ['Fern', 'BerryBush'],       hint: { temp_pref: 0.78, size: 0.3 } },
  { name: 'savanna-grazer',   band: [0.1, 0.4],  wetness: 0.45, food: ['Clover', 'Wildflower'],    hint: { temp_pref: 0.7, size: 0.6, metab: 0.3 } }, // slow energy-efficient cow
  { name: 'temperate-omni',   band: [0.4, 0.75], wetness: 0.6,  food: ['BerryBush', 'Clover'],     hint: { temp_pref: 0.5, size: 0.4 } },
  { name: 'cold-pelted',      band: [0.85, 1.25], wetness: 0.45, food: ['Moss', 'AlpineCushion'],  hint: { temp_pref: 0.25, pelt: 0.7, size: 0.45 } },
  { name: 'polar-survivor',   band: [1.0, 1.4],  wetness: 0.4,  food: ['AlpineCushion', 'Moss'],   hint: { temp_pref: 0.15, pelt: 0.85, adiposity: 0.6 } },
  { name: 'arid-desert',      band: [0.5, 0.9],  wetness: 0.15, food: ['Cactus', 'Thistle'],       hint: { temp_pref: 0.8, adiposity: 0.7, size: 0.3 } },
  { name: 'highland-climber', band: [0.4, 0.9],  wetness: 0.4,  rocky: true, food: ['Thistle', 'Clover'], hint: { alpine: 0.7, size: 0.25 } },
  { name: 'tree-climber',     band: [0.3, 0.7],  wetness: 0.6,  food: ['Clover', 'TREE', 'TREE'],  hint: { climb: 0.75, size: 0.16, height: 0.2, metab: 0.6 } }, // small fast climber
  { name: 'aquatic-forager',  band: [0.2, 0.6],  wetness: 0.95, aquatic: true, food: ['Waterlily', 'Kelp'], hint: { swim: 0.85, temp_pref: 0.6 } },
].map((n) => ({ ...n, kind: 'creature' }))

const ALL = [...CREATURE_NICHES, ...PLANT_NICHES]

// Cross-environment PANEL for the gauntlet: a champion genome runs UNCHANGED in
// each. Survival in many = a generalist that "survives in every environment".
const PANEL = [
  { env: 'equator-wet',   band: [0.05, 0.35], wetness: 0.85, aquatic: false },
  { env: 'temperate',     band: [0.45, 0.75], wetness: 0.55, aquatic: false },
  { env: 'hot-desert',    band: [0.5, 0.85],  wetness: 0.12, aquatic: false },
  { env: 'polar-cold',    band: [1.05, 1.4],  wetness: 0.4,  aquatic: false },
  { env: 'shallow-water', band: [0.15, 0.55], wetness: 0.95, aquatic: true },
]

// ---- structured returns -----------------------------------------------------
const COMPETE_SCHEMA = {
  type: 'object',
  required: ['name', 'kind', 'result_path', 'qualified', 'bred', 'score', 'survived', 'frictions'],
  properties: {
    name: { type: 'string' },
    kind: { type: 'string', enum: ['plant', 'creature'] },
    result_path: { type: 'string', description: 'path to the BEST result.json (merged into seed bank)' },
    qualified: { type: 'boolean', description: 'cohort BRED and sustained (reproduction gate): plant R>=1 & births>0, or creature_survival>=1' },
    bred: { type: 'boolean', description: 'any offspring produced during the run' },
    score: { type: 'number', description: 'plant: health_score; creature: creature_survival' },
    survived: { type: 'number' },
    rounds: { type: 'number' },
    frictions: { type: 'array', items: { type: 'string' }, description: 'balance frictions (gene pegging 0/1, impossible niche, free-lunch combo, instant die-off)' },
  },
}

const GAUNTLET_SCHEMA = {
  type: 'object',
  required: ['name', 'kind', 'envs_survived', 'envs_total', 'robustness', 'notes'],
  properties: {
    name: { type: 'string' },
    kind: { type: 'string', enum: ['plant', 'creature'] },
    envs_survived: { type: 'number' },
    envs_total: { type: 'number' },
    robustness: { type: 'number', description: 'envs_survived / envs_total (1.0 = survives every environment)' },
    survived_envs: { type: 'array', items: { type: 'string' }, description: 'which PANEL env names it survived' },
    notes: { type: 'array', items: { type: 'string' } },
  },
}

// =============================================================================
phase('build')
await agent(
  `Run \`cargo build --release\` in ${REPO}. Report only "ok" or the error tail. (Release = fast sim; many agents will reuse this binary.)`,
  { label: 'build', phase: 'build' },
)

// =============================================================================
phase('compete')
const HARNESS_PLANT = `THE HARNESS (plants):
- Write a scenario JSON, then run: ${BIN} --scenario=<scn.json> --out=<result.json> --seed=<K>
- Run each candidate at 3 seeds (1,2,3) and average; one seed is noisy.
- Scenario fields: seed(int), ticks(int; use 10000), target_count(int; use 30),
    world:{ lat_band:[lo,hi], wetness:0..1, aquatic:bool, rocky:bool, fire:0..1, grazers:int, second_band:[lo,hi]|null },
    plant_cohort:[ { count:int(start 10), archetype:"<Name>"|null, tree:bool, genome:{ <any PlantGenome gene>:<value> } } ]
  Genome FREE-FORM: wet, temp_pref, succulence, light_pref, defense, regrow, fruiting, height, submerged,
  nitrogen_fix, seed_weight, windborne, clonal, maturity, spread, nutrient, quality, toxicity, ... and any gene
  added later. Unknown keys warn + ignored.
- Result fields: started, survived, peak_count, final_count, reached_target, mean_mass, max_mass, births, deaths,
  r (births/deaths; want >=1), mean_growth_rate, deaths_by_cause{moisture,temp,drown,desiccate,habitat,fire,eaten},
  trait_drift{gene:[seed,survivor]}, health_score, best_genomes.
- REPRODUCTION GATE (hard): a cohort only QUALIFIES if it BRED and sustained -> births>0 AND r>=1 AND final_count
  near/above target. A transient peak that then dwindles does NOT qualify. Read deaths_by_cause to see WHY it dies
  (moisture->tune wet/succulence; temp->tune temp_pref; drown/desiccate->wet vs submerged; habitat->shift band;
  eaten->raise defense/regrow/toxicity if grazers>0). Watch trait_drift for genes the survivors pull toward.`

const HARNESS_CRE = `THE HARNESS (creatures):
- Write a scenario JSON, then run: ${BIN} --scenario=<scn.json> --out=<result.json> --seed=<K>
- Run each candidate at 3 seeds (1,2,3) and average; one seed is noisy.
- Scenario fields: seed(int), ticks(int; use 4000), target_count(int; use 12),
    world:{ lat_band:[lo,hi], wetness:0..1, aquatic:bool, rocky:bool },
    plant_cohort:[ { count:int, archetype:"<Name>"|null, tree:bool, genome:{...} } ]   // the FOOD (make ABUNDANT, 60+ ground plants, so starvation is a TUNING result not scarcity)
    creature_cohort:[ { count:int(use 12), reflex:"approach-food"|"flee-predator"|"rest-at-night"|"wander"|null,
                        genome:{ <any Genome gene>:<value> } } ]
  Creature genome FREE-FORM: size, metab, longevity, parental, adiposity, bite, height, light_pref, temp_pref,
  swim, alpine, social, rigidity, detox, carnivory, pelt, armor, venom, limbs, climb, eyes, head, uptake(array 10),
  sensors(array of {angle,range}). reflex = a hand-wired brain prior; lifetime learning refines it in-run. Unknown
  keys warn + ignored.
- Result creature fields: creature_started, creature_survived, creature_survival (final/started; >=1 = lineage
  bred + sustained/grew), creature_mean_age, creature_mean_energy, creature_mean_master (digestion fit to the
  food; aim ~1.0), creature_trait_drift{gene:[seed,survivor]}, best_creatures.
- REPRODUCTION GATE (hard): creatures breed continuously; a cohort QUALIFIES only if creature_survival>=1 (started
  12 -> survived >=12 means net births covered deaths; >1 means the lineage GREW). survival 0 with food present =>
  they cannot SEE food (raise sensor range) or sensing/basal too costly (lower it) or mean_master low (fix uptake
  to match the food). NO FREE LUNCH: every gene has a cost (size->hungrier, pelt->overheats+drags in water,
  sensors->energy/unit, swim->costly on land). Hold niche identity (aquatic stays swim-high, cold stays pelted).`

const results = await pipeline(ALL, (n) => {
  const isCre = n.kind === 'creature'
  const tmp = `/tmp/tourney-${n.kind}-${n.name}`
  const prompt = isCre
    ? `You compete in a tournament: tune a CREATURE cohort toward a BREEDING, self-sustaining lineage in one
environment niche, using the evolvarium --scenario harness. ONLY use the scenario JSON interface + run the
binary. Do NOT edit any Rust/source/config files. Work in ${REPO}.

NICHE: ${JSON.stringify(n)}

${HARNESS_CRE}

Seed FOOD with plant_cohort from this niche's food list: ${JSON.stringify(n.food)} ("TREE" => { count:4, tree:true }).
Start the creature genome from the hint: ${JSON.stringify(n.hint || {})}. ${n.aquatic ? 'Aquatic niche: keep swim high.' : ''} ${n.rocky ? 'Rocky niche: alpine helps on rock.' : ''}

OBJECTIVE: maximize creature_survival (the lineage BREEDS + grows) with high mean_master and a healthy mean_age,
while HOLDING the niche identity. Iterate ~4-6 rounds: run 3 seeds, read why they fail, adjust genome + reflex,
re-run, keep the best mean. A gene pegging to 0/1 across runs is a FRICTION.

DELIVERABLE: leave your single BEST result.json at ${tmp}/best.json (re-run the best config to that exact path).
Return JSON per schema: name="${n.name}", kind="creature", result_path="${tmp}/best.json", qualified (true only if
mean creature_survival>=1), bred (any net offspring), score=mean creature_survival, survived, rounds, frictions.`
    : `You compete in a tournament: tune a PLANT/TREE cohort toward SURVIVAL + GROWTH + REPRODUCTION in one
environment niche, using the evolvarium --scenario harness. ONLY use the scenario JSON interface + run the binary.
Do NOT edit any Rust/source/config files. Work in ${REPO}.

NICHE: ${JSON.stringify(n)}

${HARNESS_PLANT}

Start from the niche archetype + band${n.tree ? ' (tree=true: keep it a TREE)' : ''}${n.hint ? `, hint ${JSON.stringify(n.hint)}` : ''}.
${n.grazers ? 'Grazers present: survivors need defense/regrow/toxicity.' : ''} ${n.fire ? 'Fire pressure: reward fire-survival genes.' : ''}
You may seed from plant-library.json if it has entries for this niche (paste a best_genome into the override) to
continue evolving prior winners.

OBJECTIVE: maximize health_score AND pass the reproduction gate (births>0, r>=1, final_count near/above target).
Iterate ~4-6 rounds. Hold niche identity (tree stays tree, aquatic stays aquatic). Note any gene pegging 0/1.

DELIVERABLE: leave your single BEST result.json at ${tmp}/best.json (re-run the best config to that exact path).
Return JSON per schema: name="${n.name}", kind="plant", result_path="${tmp}/best.json", qualified (births>0 AND r>=1
AND final_count>=~0.7*target), bred (births>0), score=mean health_score, survived, rounds, frictions.`
  return agent(prompt, { label: `${n.kind}:${n.name}`, phase: 'compete', schema: COMPETE_SCHEMA })
})

const finished = results.filter(Boolean)
const qualified = finished.filter((r) => r.qualified && r.result_path)
log(`compete done: ${finished.length}/${ALL.length} returned, ${qualified.length} bred + qualified`)

// =============================================================================
phase('gauntlet')
// Each qualifying champion runs UNCHANGED across the PANEL to find generalists.
const gauntlet = await pipeline(qualified, (champ) => {
  const isCre = champ.kind === 'creature'
  return agent(
    `You run the CROSS-ENVIRONMENT GAUNTLET for one tournament champion. Goal: find out in how many DIFFERENT
environments this genome survives UNCHANGED ("survives in every environment"). ONLY use the scenario interface +
run the binary. Do NOT edit source. Work in ${REPO}.

CHAMPION: name="${champ.name}" kind="${champ.kind}". Its best genomes are in: ${champ.result_path}
1) Read ${champ.result_path}; take the TOP genome (${isCre ? 'best_creatures[0]' : 'best_genomes[0].genome'}).
2) For EACH panel environment below, write a scenario that places ${isCre ? 'a creature_cohort of count 12 with that EXACT genome (reflex:"approach-food"), plus ABUNDANT generic food (plant_cohort: 60 BerryBush + 40 Clover, or Waterlily+Kelp if aquatic)' : 'a plant_cohort of count 12 with that EXACT genome (tree flag as in best_genomes[0].tree)'} into the env band, then run ${BIN} --scenario=<scn> --out=<out> --seed=1 (${isCre ? 'ticks 4000, target 12' : 'ticks 10000, target 12'}).
   PANEL: ${JSON.stringify(PANEL)}
3) An env counts as SURVIVED if ${isCre ? 'creature_survived > 0 (the lineage persisted)' : 'survived > 0'} at the end.
   Do NOT re-tune the genome; this measures the champion AS-IS. A specialist surviving only its home climate is
   expected; a generalist surviving most/all panel envs is the prize.

Return JSON per schema: name="${champ.name}", kind="${champ.kind}", envs_survived, envs_total=${PANEL.length},
robustness=envs_survived/${PANEL.length}, survived_envs (env names), notes (anything notable, e.g. died only in desert).`,
    { label: `gauntlet:${champ.name}`, phase: 'gauntlet', schema: GAUNTLET_SCHEMA },
  )
})

const ranked = gauntlet.filter(Boolean).sort((a, b) => b.robustness - a.robustness)
log(`gauntlet done: ${ranked.length} champions ranked; top robustness ${ranked[0]?.robustness ?? 0}`)

// =============================================================================
phase('synthesize')
// REPLACE the library: every plant niche's best goes in (not just breeders), built fresh from THIS run so the
// new genes are tuned (old entries predate them). Prefer qualified, but include all so no niche goes bare.
const plantWins = finished.filter((r) => r.kind === 'plant' && r.result_path)
const creWins = qualified.filter((r) => r.kind === 'creature')
const plantNonBreed = plantWins.filter((r) => !r.qualified).map((r) => r.name)
const frictions = finished.flatMap((r) => (r.frictions || []).map((f) => `[${r.kind}:${r.name}] ${f}`))
const board = ranked.map((r) => `${r.name} (${r.kind}): ${r.envs_survived}/${r.envs_total} envs${r.survived_envs ? ' [' + r.survived_envs.join(',') + ']' : ''}`).join('\n')

const report = await agent(
  `You finalize the tournament. Work in ${REPO}. Run merges SEQUENTIALLY (shared files must not race).

A) PLANTS -> REPLACE the seed-bank library with THIS run's bests (fresh, newest tuned genes). Steps:
   1) The pristine old library is already backed up at plant-library.json.bak (do NOT overwrite that backup).
   2) START FRESH so old pre-this-run entries are dropped: write a new plant-library.json containing exactly
      {"version":1,"entries":[]} (overwrite the file).
   3) Then for EACH entry below, ONE AT A TIME, merge into the now-empty library:
        ${BIN} --merge=<result_path> --niche=<name> --plant-lib=plant-library.json --lib-cap=8
   Entries (EVERY plant niche; non-breeders flagged but still included so no biome is bare):
${plantWins.map((r) => `     - niche=${r.name} result_path=${r.result_path} (health ${r.score?.toFixed?.(2)}${r.qualified ? '' : ', NON-BREEDER'})`).join('\n') || '     (none)'}
   After merging, report the final library entry count and confirm all ${plantWins.length} niches are present.

B) CREATURES -> a FRESH tournament seed (do NOT touch evolved-continuous.json yet):
     rm -f evolved-tournament.json
   then for each entry, ONE AT A TIME:
     ${BIN} --merge-creatures=<result_path> --snap=evolved-tournament.json --cap=120
   Entries:
${creWins.map((r) => `     - niche=${r.name} result_path=${r.result_path} (survival ${r.score?.toFixed?.(2)})`).join('\n') || '     (none)'}

C) SMOKE both seeded worlds (must boot + stay populated):
     ${BIN} --headless --gens=3 --plant-lib=plant-library.json --no-load   # library-seeded planet
     ${BIN} --load=evolved-tournament.json --headless --gens=2             # tournament population
   Confirm each reaches "headless run done"/"continuous headless done" with a non-trivial pop in the last gen log.

D) PROMOTE the tournament seed ONLY IF its smoke (step C, the --load run) stayed populated (final pop is
   non-trivial, not collapsing toward 0). If it passed: cp evolved-tournament.json evolved-continuous.json
   If it did NOT pass: leave evolved-continuous.json untouched and say so clearly.

E) Append FRICTIONS to /home/marc/Documents/Github/clients/evolvarium/tuning-frictions.md (append-only, each a new
   "## F<n>"-style entry with niche + symptom). Frictions:
${frictions.length ? frictions.map((f) => `     - ${f}`).join('\n') : '     (none reported)'}

Return a short plain-text report: library entry count, tournament-seed creature count, both smoke results (final
pops), whether the showcase seed was PROMOTED or held, and frictions logged.`,
  { label: 'synthesize', phase: 'synthesize' },
)

return {
  competed: ALL.length,
  returned: finished.length,
  qualified_bred: qualified.length,
  plant_winners: plantWins.map((r) => r.name),
  plant_non_breeders: plantNonBreed,
  creature_winners: creWins.map((r) => r.name),
  most_robust: ranked.slice(0, 5).map((r) => ({ name: r.name, kind: r.kind, robustness: r.robustness })),
  leaderboard: board,
  frictions,
  report,
}
