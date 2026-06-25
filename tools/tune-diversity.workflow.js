export const meta = {
  name: 'tune-diversity',
  description: 'Evolve 30+ hardy survivors for each niche x climate combo in competitive mini-worlds, merge into a diversity seed',
  whenToUse: 'Build a richly diverse, hardy creature seed: every niche (flying/land/water/highland) x climate (warm/temperate/cold) represented by 30+ survivors that hold their identity + develop brains under competition.',
  phases: [
    { title: 'build', detail: 'compile release once so tuners reuse it' },
    { title: 'tune', detail: 'one agent per niche x climate combo: evolve 30+ hardy survivors in a competitive mini-world' },
    { title: 'synthesize', detail: 'merge every combo winner into evolved-diversity.json + smoke' },
  ],
}

const BIN = './target/release/evolvarium'
const DIR = '/home/marc/Documents/Github/evolvarium'
const SEED_OUT = 'evolved-diversity.json'

// Niche x climate matrix. band = |latitude| radians (0 equator .. 1.57 pole). climate sets band+temp_pref;
// niche sets the locomotion identity + flags. food = climate-appropriate plant archetypes (the cohort's FOOD).
const CLIMATES = {
  warm: { band: [0.0, 0.4], temp_pref: 0.8, food: ['BerryBush', 'Wildflower', 'Clover'], wet: 0.6 },
  temperate: { band: [0.45, 0.85], temp_pref: 0.5, food: ['Clover', 'BerryBush', 'Fern'], wet: 0.55 },
  cold: { band: [0.95, 1.4], temp_pref: 0.2, food: ['AlpineCushion', 'Moss', 'Thistle'], wet: 0.45 },
}
const NICHES = {
  flying: { hint: { flight: 0.72, size: 0.16, eyes: 0.6 }, flags: {}, idkey: 'flight', idmin: 0.4 },
  land: { hint: { limbs: 0.55, size: 0.32, swim: 0.05, flight: 0.0 }, flags: {}, idkey: 'land', idmin: 0 },
  water: { hint: { swim: 0.85, size: 0.3 }, flags: { aquatic: true }, idkey: 'swim', idmin: 0.6, foodOverride: { warm: ['Waterlily', 'Eelgrass'], temperate: ['Eelgrass', 'Kelp'], cold: ['Kelp', 'Eelgrass'] }, wet: 0.95 },
  highland: { hint: { alpine: 0.7, size: 0.25 }, flags: { rocky: true }, idkey: 'alpine', idmin: 0.45, wet: 0.4 },
}

const COMBOS = []
for (const [nn, niche] of Object.entries(NICHES)) {
  for (const [cn, clim] of Object.entries(CLIMATES)) {
    const food = (niche.foodOverride && niche.foodOverride[cn]) || clim.food
    const wet = niche.wet ?? clim.wet
    const hint = { ...niche.hint, temp_pref: clim.temp_pref }
    if (cn === 'cold') hint.pelt = niche.flags.aquatic ? 0.2 : 0.65 // cold land/air = pelted; pelt drags swimmers
    COMBOS.push({ name: `${nn}-${cn}`, niche: nn, climate: cn, band: clim.band, wet, food, hint, flags: niche.flags, idkey: niche.idkey, idmin: niche.idmin })
  }
}

const SCHEMA = {
  type: 'object',
  required: ['combo', 'result_path', 'survived', 'creature_survival', 'identity_held', 'frictions'],
  properties: {
    combo: { type: 'string' },
    result_path: { type: 'string', description: 'path to the BEST result.json (harvested into the seed)' },
    survived: { type: 'number', description: 'survivors at end (aim >= 30)' },
    creature_survival: { type: 'number', description: 'survived/started (>=1 = sustaining/growing)' },
    mean_master: { type: 'number', description: 'mean digestion expression of survivors (diet fit to food)' },
    identity_held: { type: 'boolean', description: 'survivors STILL match the niche+climate (e.g. swimmers kept swim>=0.6, cold kept temp_pref<=0.35)' },
    mean_sensors: { type: 'number', description: 'mean sensor count of survivor brains (from best_creatures[].sensors.length)' },
    mean_hidden: { type: 'number', description: 'mean hidden-layer size of survivor brains (from best_creatures[].net.ih.length)' },
    brain_note: { type: 'string', description: 'brain development observed: did sensors/hidden grow vs seed, did behavior adapt to predation/competition' },
    rounds: { type: 'number' },
    frictions: { type: 'array', items: { type: 'string' } },
  },
}

phase('build')
await agent(`Run \`cargo build --release\` in ${DIR} and report only "ok" or the error tail.`, { label: 'build', phase: 'build' })

phase('tune')
const results = await pipeline(COMBOS, (c) =>
  agent(
    `You evolve a HARDY, SELF-SUSTAINING cohort of CREATURES for ONE niche x climate combo in a COMPETITIVE
mini-world, using the evolvarium --scenario harness. Goal: >= 30 survivors that HOLD the combo identity and
show BRAIN DEVELOPMENT under competition. ONLY use the scenario JSON + run the binary. Do NOT edit any
source/config. Work in ${DIR}. Binary prebuilt at ${BIN}.

COMBO: ${JSON.stringify(c)}
  niche=${c.niche} climate=${c.climate}. Identity gene to HOLD: ${c.idkey} (survivors must keep it${c.idmin ? ` >= ${c.idmin}` : ''});
  climate temp_pref target ~${c.hint.temp_pref} (cold survivors keep temp_pref low, warm keep it high).

THE HARNESS:
- Write scenario JSON, run: ${BIN} --scenario=<scn.json> --out=<result.json> --seed=<K>
- Run each candidate at seeds 1,2,3 and average (one seed is noisy). Keep it tight: ~4-6 rounds total.
- Scenario template (COMPETITIVE: finite food + grazers competing for it + a predator sub-cohort; tight cap):
  {
    "seed": <K>, "ticks": 7000, "target_count": 30,
    "world": { "lat_band": ${JSON.stringify(c.band)}, "wetness": ${c.wet},${c.flags.aquatic ? ' "aquatic": true,' : ''}${c.flags.rocky ? ' "rocky": true,' : ''} "grazers": 22 },
    "plant_cohort": [ ${c.food.map((f) => `{ "count": 26, "archetype": "${f}", "genome": {} }`).join(', ')} ],
    "creature_cohort": [
      { "count": 30, "reflex": "approach-food", "genome": ${JSON.stringify(c.hint)} },
      { "count": 7, "reflex": "approach-food", "genome": { "carnivory": 0.85, "size": ${(c.hint.size || 0.3) + 0.25}, "bite": 0.7, "temp_pref": ${c.hint.temp_pref}${c.flags.aquatic ? ', "swim": 0.85' : ''}${c.flags.rocky ? ', "alpine": 0.6' : ''}${c.niche === 'flying' ? ', "flight": 0.6' : ''} } }
    ]
  }
  The 2nd cohort = PREDATORS (carnivory + bigger) pressuring the prey; grazers eat the plants (food
  competition). Make the prey hardy ENOUGH to reach 30+ survivors DESPITE this -> that is the "hardy in a
  competitive landscape" goal. The creature genome object is FREE-FORM (any gene): size, metab, longevity,
  parental, adiposity, bite, height, light_pref, temp_pref, swim, alpine, social, rigidity, detox, carnivory,
  pelt, armor, venom, limbs, climb, eyes, head, flight, hearing, hear_freq, uptake (10 nums), sensors
  (array of {angle,range}).

TUNING LEVERS (no free lunch):
- temp_pref MUST match the climate band (cold band -> low temp_pref + pelt; warm -> high). Mismatch burns energy.
- uptake (10 nums 0..1) MUST match the food's nutrients or mean_master stays low -> starvation with food present.
- ${c.idkey} is the niche identity: keep it strong (${c.niche === 'water' ? 'swim>=0.6 to live in the sea' : c.niche === 'flying' ? 'flight>=0.4 to fly' : c.niche === 'highland' ? 'alpine>=0.45 for rock' : 'low swim/flight = a true land walker'}).
- sensors: range costs energy/unit; 2-3 sensors range ~18-30. social: herd safety vs lone drain.
- Against PREDATORS: prey survive via vigilance (social), evasion (climb/flight/swim escape), or just out-breeding
  the losses. Lifetime learning + the recurrent memory + evolvable sensors/hidden-layer mean BRAINS DEVELOP
  during the run -> report it.

BRAIN DEVELOPMENT (the user explicitly wants to SEE this): from your BEST result.json read best_creatures
(top survivor genomes). For each, sensors.length = sensor count, net.ih.length = hidden-layer neuron count.
Report mean_sensors + mean_hidden across survivors, and whether they GREW vs the seeded base (~2-3 sensors,
~3-6 hidden) -> evidence the brains complexified under competition.

OBJECTIVE: survived >= 30 (or as close as the niche allows) with creature_survival >= ~1, high mean_master,
and survivors that STILL match the combo identity (identity_held=true). Read result fields: creature_started,
creature_survived, creature_survival, creature_mean_age, creature_mean_master, creature_trait_drift
{gene:[seed,survivor]}, best_creatures.

DELIVERABLE: leave your single BEST result.json at /tmp/diversity-${c.name}/best.json (re-run the best config
to that exact path at seed 1). Return JSON per schema: combo ("${c.name}"), result_path, survived,
creature_survival, mean_master, identity_held, mean_sensors, mean_hidden, brain_note, rounds, frictions.`,
    { label: `tune:${c.name}`, phase: 'tune', schema: SCHEMA },
  ),
)

phase('synthesize')
const good = results.filter(Boolean).filter((r) => r.survived > 0 && r.result_path)
const frictions = results.filter(Boolean).flatMap((r) => (r.frictions || []).map((f) => `[${r.combo}] ${f}`))

const report = await agent(
  `You finalize the diversity run: harvest every combo's survivors into ONE fresh, diverse seed. Work in ${DIR}.
1) START FRESH: rm -f ${SEED_OUT}
2) MERGE each combo's best result into the seed, ONE AT A TIME (shared file, do NOT parallelize). cap high so
   all combos fit (12 combos x up to ~40 survivors):
${good.map((r) => `     ${BIN} --merge-creatures=${r.result_path} --snap=${SEED_OUT} --cap=520   # ${r.combo} (survived ${r.survived})`).join('\n')}
3) SMOKE: ${BIN} --load=${SEED_OUT} --headless --gens=2  -> must reach "continuous headless done" with a
   healthy pop in the last log line. Report the final pop + creature count in ${SEED_OUT}.
4) Append FRICTIONS to /home/marc/Documents/Github/clients/evolvarium/tuning-frictions.md (append-only):
${frictions.length ? frictions.map((f) => `     - ${f}`).join('\n') : '     (none)'}

Return a short plain-text report: total creatures in the seed, the smoke pop, a per-combo line (survivors +
identity_held + brain note), and which combos fell short of 30.`,
  { label: 'synthesize', phase: 'synthesize' },
)

return {
  combos_tuned: good.length,
  hit_30: good.filter((r) => r.survived >= 30).map((r) => r.combo),
  identity_held: good.filter((r) => r.identity_held).map((r) => r.combo),
  brain_dev: good.map((r) => `${r.combo}: sensors~${r.mean_sensors?.toFixed?.(1)} hidden~${r.mean_hidden?.toFixed?.(1)} ${r.brain_note || ''}`),
  frictions,
  report,
}
