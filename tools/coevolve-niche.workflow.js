export const meta = {
  name: 'coevolve-niche',
  description: 'Within-niche competition: 2-3 contrasting cohorts + grazers compete in one shared mini-world per biome, harvest winners into the library',
  whenToUse: 'After the isolated tuning run: see whether plants co-evolve differently when several strategies compete head-to-head in the same environment + face grazers.',
  phases: [
    { title: 'build', detail: 'compile the binary once' },
    { title: 'compete', detail: 'one agent per niche runs a multi-cohort + grazer world, harvests winners' },
    { title: 'synthesize', detail: 'merge each niche winner into the library as <niche>-coevo + smoke' },
  ],
}

const BIN = './target/debug/evolvarium'
const DIR = '/home/marc/Documents/Github/evolvarium'

// Each niche pits 2-3 contrasting strategies against each other in ONE shared world (same band + wetness),
// plus grazers (the bite-vs-defense arms race). The cohort population is capped cohort-scale (~2x target),
// so the strategies genuinely compete for limited space. archetypes = the contrasting starting strategies.
const NICHES = [
  { name: 'tropical-wet',     band: [0.0, 0.4], wetness: 0.8, archetypes: ['Fern', 'Wildflower', 'BerryBush'] },
  { name: 'temperate-meadow', band: [0.4, 0.8], wetness: 0.55, archetypes: ['Clover', 'BerryBush', 'Wildflower'] },
  { name: 'arid-desert',      band: [0.5, 0.9], wetness: 0.12, archetypes: ['Cactus', 'Tumbleweed', 'Thistle'] },
  { name: 'polar-alpine',     band: [1.05, 1.45], wetness: 0.4, archetypes: ['AlpineCushion', 'Moss'] },
  { name: 'highland-rocky',   band: [0.4, 0.9], wetness: 0.4, rocky: true, archetypes: ['Thistle', 'AlpineCushion'] },
  { name: 'shallow-sunlit',   band: [0.1, 0.6], wetness: 0.95, aquatic: true, archetypes: ['Waterlily', 'Eelgrass', 'AlgaeMat'] },
  { name: 'deep-kelp',        band: [0.2, 0.8], wetness: 0.98, aquatic: true, archetypes: ['Kelp', 'Eelgrass'] },
]

const TUNE_SCHEMA = {
  type: 'object',
  required: ['niche', 'result_path', 'health_score', 'survived', 'frictions'],
  properties: {
    niche: { type: 'string' },
    result_path: { type: 'string', description: 'path to the BEST result.json produced' },
    health_score: { type: 'number' },
    survived: { type: 'number' },
    winners: { type: 'string', description: 'which strategies/traits dominated the shared world + how it differs from a solo cohort' },
    frictions: { type: 'array', items: { type: 'string' } },
  },
}

phase('build')
await agent(`Run \`cargo build\` in ${DIR} and report only "ok" or the error tail.`, { label: 'build', phase: 'build' })

phase('compete')
const results = await pipeline(NICHES, (niche) =>
  agent(
    `You run a WITHIN-NICHE COMPETITION experiment with the evolvarium --scenario harness: several contrasting
plant strategies compete head-to-head in ONE shared mini-world for the same biome, plus grazers (the
bite-vs-defense arms race). Goal: get a VIABLE competing world (not total extinction, not one strategy
instantly monopolizing then crashing), let it run, and HARVEST the survivors. ONLY use the scenario JSON +
run the binary. Do NOT edit any source/config. Work in ${DIR}. Binary prebuilt at ${BIN}.

NICHE: ${JSON.stringify(niche)}

Author a scenario with ALL of the niche's archetypes as SEPARATE cohorts in the SAME world, plus grazers:
{ "seed": <K>, "ticks": 16000, "target_count": 30,
  "world": { "lat_band": ${JSON.stringify(niche.band)}, "wetness": ${niche.wetness},${niche.aquatic ? ' "aquatic": true,' : ''}${niche.rocky ? ' "rocky": true,' : ''} "grazers": 25 },
  "plant_cohort": [ ${niche.archetypes.map((a) => `{ "count": 10, "archetype": "${a}", "genome": {} }`).join(', ')} ] }

KEY FACTS:
- wetness IS the effective local moisture (tune plant \`wet\` near it; succulence buffers a DRY env).
- The cohort population is capped ~2x target (so the strategies compete for limited space).
- grazers apply predation: plants with higher defense/regrow/toxicity survive grazing better (arms race).
- Run each candidate at seeds 1,2,3. Read result fields: survived, peak_count, mean_mass, r, health_score,
  deaths_by_cause {moisture,temp,drown,desiccate,habitat,fire,eaten}, trait_drift {gene:[seeded,survivor]},
  best_genomes.

METHOD (iterate ~4-6 rounds, keep it tight):
- Run the multi-cohort + grazer world at 3 seeds. If EVERYTHING dies, the world is too harsh: fix the obvious
  killer from deaths_by_cause (tune the cohorts' \`wet\`/\`temp_pref\` toward the band, or lower grazers).
- If one strategy trivially wins with no contest, that's a finding (note it) — keep the world as-is to see the
  competitive outcome; do not hand-balance the cohorts to be equal.
- Watch trait_drift: which traits the SURVIVORS converged on under competition + grazing (e.g. defense/regrow
  rising = arms race). Compare to what a solo cohort would do (you know solo tuning favors growth, low defense).
- Aim for a world where survivors reach a stable mix or a clear winner with R>=1 and good mean_mass.

DELIVERABLE: leave your BEST result at /tmp/coevo-${niche.name}/best.json (re-run the best config to that exact
path at seed 1). Return JSON per schema: niche ("${niche.name}"), result_path, health_score, survived,
winners (which strategies/traits dominated + how it differs from solo tuning), frictions.`,
    { label: `compete:${niche.name}`, phase: 'compete', schema: TUNE_SCHEMA },
  ),
)

phase('synthesize')
const good = results.filter(Boolean).filter((r) => r.survived > 0 && r.result_path)
const frictions = results.filter(Boolean).flatMap((r) => (r.frictions || []).map((f) => `[${r.niche}-coevo] ${f}`))

const report = await agent(
  `Finalize the within-niche competition run. Work in ${DIR}.
1) MERGE each niche's harvested winners into the library under a SEPARATE "-coevo" niche (so they coexist with
   the isolated entries for comparison), ONE AT A TIME (the library file is shared, do not parallelize):
${good.map((r) => `     ${BIN} --merge=${r.result_path} --niche=${r.niche}-coevo --plant-lib=plant-library.json --lib-cap=8`).join('\n')}
   Report the entry count after each.
2) SMOKE the library-seeded planet still boots stable:
     ${BIN} --headless --gens=2 --plant-lib=plant-library.json --no-load
   Report plants + trees counts from the last gen log line + that it reached "headless run done".
3) Append these FRICTIONS to /home/marc/Documents/Github/clients/evolvarium/tuning-frictions.md (append-only):
${frictions.length ? frictions.map((f) => `     - ${f}`).join('\n') : '     (none reported)'}
Return a short plain-text report: library entry count, smoke result, and a one-line summary per niche of how
competition changed the survivors vs solo tuning (from each agent's "winners" note):
${good.map((r) => `     - ${r.niche}: ${r.winners || '(no note)'}`).join('\n')}`,
  { label: 'synthesize', phase: 'synthesize' },
)

return { niches: good.length, frictions, report }
