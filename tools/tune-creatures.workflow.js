export const meta = {
  name: 'tune-creatures',
  description: 'Tune creature cohorts per niche via the --scenario harness, harvest survivors into evolved-continuous.json',
  whenToUse: 'Evolve creature genetics + brains for each environment niche and build a fresh population seed (the showcase seed loaded by `cargo run`).',
  phases: [
    { title: 'build', detail: 'compile the binary once so tuner agents reuse it' },
    { title: 'tune', detail: 'one agent per niche: author scenario, run seeds, adjust genes/reflex, repeat' },
    { title: 'synthesize', detail: 'merge each niche winner into the population snapshot, smoke the seeded world' },
  ],
}

// Creature niches. Each agent owns ONE row + its isolated mini-world(s). Bands are |latitude| in radians
// (0 = equator .. ~1.57 = pole). `food` lists the plant_cohort archetypes to seed as the cohort's FOOD; a
// creature cohort is co-located with its food in a compact patch (so foraging works). These are STARTING
// points; the agent adjusts the creature genome overrides + reflex + (within reason) the band/food to find a
// self-sustaining lineage.
const NICHES = [
  { name: 'warm-generalist',  band: [0.1, 0.45], wetness: 0.6,  food: ['BerryBush', 'Clover'],  hint: { temp_pref: 0.7, size: 0.3 } },
  { name: 'cold-pelted',      band: [0.9, 1.3],  wetness: 0.45, food: ['AlpineCushion', 'Moss'], hint: { temp_pref: 0.25, pelt: 0.7, size: 0.4 } },
  { name: 'aquatic-forager',  band: [0.2, 0.6],  wetness: 0.95, aquatic: true, food: ['Waterlily', 'Eelgrass'], hint: { swim: 0.85, temp_pref: 0.6 } },
  { name: 'highland-climber', band: [0.4, 0.9],  wetness: 0.4,  rocky: true,  food: ['Thistle', 'Clover'],   hint: { alpine: 0.7, size: 0.25 } },
  { name: 'tree-climber',     band: [0.3, 0.7],  wetness: 0.6,  food: ['Clover', 'TREE', 'TREE'], hint: { climb: 0.7, size: 0.18, height: 0.2 } },
  { name: 'arid-desert',      band: [0.5, 0.9],  wetness: 0.15, food: ['Cactus', 'Thistle'],   hint: { temp_pref: 0.8, adiposity: 0.7, size: 0.3 } },
  // aerial: the bird niche (flight gene). Flies over a temperate band, lands to feed on ground plants.
  { name: 'aerial-forager',   band: [0.1, 0.55], wetness: 0.5,  food: ['BerryBush', 'Wildflower'], hint: { flight: 0.7, size: 0.15, temp_pref: 0.6, eyes: 0.6 } },
]

const BIN = './target/release/evolvarium' // release: scenario sims run ~3x faster over many tuning rounds
const SEED_OUT = 'evolved-continuous.json'

const TUNE_SCHEMA = {
  type: 'object',
  required: ['niche', 'result_path', 'creature_survival', 'survived', 'frictions'],
  properties: {
    niche: { type: 'string' },
    result_path: { type: 'string', description: 'path to the BEST result.json this agent produced (harvested into the seed)' },
    creature_survival: { type: 'number', description: 'final survivors / started (>=1 = self-sustaining lineage)' },
    survived: { type: 'number' },
    mean_master: { type: 'number', description: 'mean digestion expression of survivors (diet fit to the food)' },
    rounds: { type: 'number' },
    frictions: {
      type: 'array',
      description: 'balance frictions hit (gene pegging 0/1, niche impossible to sustain, free-lunch combo, instant die-off)',
      items: { type: 'string' },
    },
  },
}

phase('build')
await agent(`Run \`cargo build --release\` in /home/marc/Documents/Github/evolvarium and report only "ok" or the error tail.`, {
  label: 'build', phase: 'build',
})

phase('tune')
const results = await pipeline(NICHES, (niche) =>
  agent(
    `You tune a cohort of CREATURES toward a SELF-SUSTAINING lineage in one environment niche, using the
evolvarium --scenario tuning harness. You may ONLY use the scenario JSON interface + run the binary. Do NOT
edit any Rust/source/config files. Work in /home/marc/Documents/Github/evolvarium.

NICHE: ${JSON.stringify(niche)}

THE HARNESS:
- Write a scenario JSON, then run: ${BIN} --scenario=<scn.json> --out=<result.json> --seed=<K>
- Run each candidate at 3 seeds (e.g. 1, 2, 3) and average creature_survival — one seed is noisy.
- Scenario input fields:
    seed (int), ticks (int; use 4000), target_count (int; use 12),
    world: { lat_band:[lo,hi] (|lat| radians), wetness:0..1, aquatic:bool, rocky:bool },
    plant_cohort: [ { count:int, archetype:"<Name>"|null, tree:bool, genome:{...} } ]  // the FOOD
    creature_cohort: [ { count:int (use 12), reflex:"approach-food"|"flee-predator"|"rest-at-night"|"wander"|null,
                         genome:{ <any Genome gene>: <value>, ... } } ]
  Seed FOOD with plant_cohort: ${JSON.stringify(niche.food)} (use archetype names; "TREE" entries => a
  { count:4, tree:true } spec for fruit + shade). Make food abundant (e.g. 60+ ground plants) so starvation
  is a TUNING result, not just scarcity.
  The creature genome object is FREE-FORM: override ANY gene by name: size, metab, longevity, parental,
  adiposity, bite, height, light_pref, temp_pref, swim, alpine, social, rigidity, detox, carnivory, pelt,
  armor, venom, limbs, climb, eyes, head, flight, uptake (array of 10), sensors (array of {angle,range}). Unknown
  keys warn + are ignored. Start from the niche hint: ${JSON.stringify(niche.hint)}.
- Result JSON creature fields you read: creature_started, creature_survived, creature_survival
  (final/started; >=1 means the lineage sustained/grew), creature_mean_age, creature_mean_energy,
  creature_mean_master (digestion fit to the food, aim high ~1.0), creature_trait_drift {gene:[seed,survivor]},
  best_creatures (top survivors, harvested into the seed).

OBJECTIVE: get creature_survival >= 1 (the cohort breeds + sustains, not dwindles) with high mean_master and
a healthy mean_age. The creatures breed during the run (continuous), so a well-adapted cohort GROWS (survival
> 1). Hold the niche identity (aquatic stays swim-high, cold stays pelted/cold-pref, climber keeps climb).

KEY TUNING LEVERS (no free lunch — every gene has a cost):
  - temp_pref: match the band temperature (cold bands need LOW temp_pref; equatorial HIGH). Mismatch burns energy.
  - uptake (10 nums 0..1): must MATCH the food's nutrients or digestion (mean_master) is low -> starvation
    even with food present. A broad uptake digests anything but costs gut overhead; start ~[0.6]*10 then narrow.
  - sensors: range costs energy per unit (long eyes are expensive). 2-3 sensors, range ~18-30 is usually enough;
    if they starve mid-patch, they may not SEE food (raise range) or sensing costs too much (lower it).
  - size: bigger = hungrier (basal scales super-linearly) but more combat/storage; small = cheap + nimble.
  - pelt: insulates cold (cut cold-band temp cost) but overheats + drags in water. swim: fast/cheap in water,
    costly on land. alpine: cheap on rock, costly on flat. climb: reaches fruit trees + evades, costly on flat.
  - flight: above FLIGHT_KNEE the creature flies (fast aloft, skips ground collision + drowning) but holding
    altitude burns energy and big wings are clumsy grounded. The aerial niche needs flight high enough to fly
    yet cheap enough upkeep to forage (it lands to feed on ground plants); pair with good eyes to spot food.
  - heat/shade: hot sunny bands burn energy in the open; seed a couple of trees (shade) or pick a cooler band.
  - reflex "approach-food" is the bread-and-butter forager prior; lifetime learning refines it.

METHOD (iterate ~4-6 rounds): start from the hint, run 3 seeds, read why they fail (low mean_master ->
fix uptake/temp_pref; survival 0 with food present -> sensing/energy balance; check trait_drift for a gene
the survivors pull toward). Re-run, keep the best mean creature_survival. A gene pegging to 0/1 across runs
is a FRICTION — note it.

DELIVERABLE: leave your single BEST result.json at /tmp/tune-cre-${niche.name}/best.json (re-run the best
config to that exact path). Return JSON per the schema: niche, result_path (that best.json path),
creature_survival, survived, mean_master, rounds, frictions.`,
    { label: `tune:${niche.name}`, phase: 'tune', schema: TUNE_SCHEMA },
  ),
)

phase('synthesize')
const good = results.filter(Boolean).filter((r) => r.survived > 0 && r.result_path)
const frictions = results.filter(Boolean).flatMap((r) => (r.frictions || []).map((f) => `[${r.niche}] ${f}`))

const report = await agent(
  `You finalize the creature tuning run: harvest each niche's survivors into a FRESH population seed. Work in
/home/marc/Documents/Github/evolvarium.

1) START FRESH: remove the old seed so the new one is built only from this run's winners:
     rm -f ${SEED_OUT}
2) MERGE each niche's best result into the seed snapshot, ONE AT A TIME (sequential — the snapshot file is
   shared, do not parallelize). For each entry below run:
     ${BIN} --merge-creatures=<result_path> --snap=${SEED_OUT} --cap=90
   Entries:
${good.map((r) => `     - niche=${r.niche} result_path=${r.result_path} (survival ${r.creature_survival?.toFixed?.(2)})`).join('\n')}
3) SMOKE the seeded world (must boot + stay populated):
     ${BIN} --load=${SEED_OUT} --headless --gens=2
   Confirm it runs to "headless run done"/"continuous headless done" with a non-trivial pop in the last log.
4) Append any FRICTIONS below to /home/marc/Documents/Github/clients/evolvarium/tuning-frictions.md
   (append-only, each as a new "## F<n>"-style entry with the niche + symptom). Frictions:
${frictions.length ? frictions.map((f) => `     - ${f}`).join('\n') : '     (none reported)'}

Return a short plain-text report: how many creatures the seed now has, the smoke result (final pop), and
frictions logged.`,
  { label: 'synthesize', phase: 'synthesize' },
)

return {
  niches_tuned: good.length,
  sustaining: good.filter((r) => r.creature_survival >= 1).map((r) => r.niche),
  frictions,
  report,
}
