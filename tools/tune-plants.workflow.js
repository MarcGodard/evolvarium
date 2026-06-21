export const meta = {
  name: 'tune-plants',
  description: 'Tune plant/tree cohorts per niche via the --scenario harness, merge winners into plant-library.json',
  whenToUse: 'Evolve plant/tree genetics for each environment niche and build the seed-bank library used to seed the planet.',
  phases: [
    { title: 'build', detail: 'compile the binary once so tuner agents reuse it' },
    { title: 'tune', detail: 'one agent per niche: author scenario, run seeds, adjust genes, repeat' },
    { title: 'synthesize', detail: 'merge each niche winner into the library, smoke the seeded planet' },
  ],
}

// Niches to tune. Each agent owns ONE row + its isolated mini-world(s). Bands are |latitude| in radians
// (0 = equator .. ~1.57 = pole). archetype = starting base; tree = seed as a Tree; second_band => MIXED.
// These are STARTING points; the agent adjusts genome overrides + (within reason) the band to find survival.
const NICHES = [
  // --- core land ---
  { name: 'tropical-wet',     archetype: 'Fern',          band: [0.0, 0.4], wetness: 0.8 },
  { name: 'temperate-meadow', archetype: 'BerryBush',     band: [0.4, 0.8], wetness: 0.6 },
  { name: 'arid-desert',      archetype: 'Cactus',        band: [0.5, 0.9], wetness: 0.12 },
  { name: 'polar-alpine',     archetype: 'AlpineCushion', band: [1.05, 1.45], wetness: 0.4 },
  { name: 'highland-rocky',   archetype: 'Thistle',       band: [0.4, 0.9], wetness: 0.4, rocky: true },
  // --- aquatic ---
  { name: 'shallow-sunlit',   archetype: 'Waterlily',     band: [0.1, 0.6], wetness: 0.95, aquatic: true },
  { name: 'deep-kelp',        archetype: 'Kelp',          band: [0.2, 0.8], wetness: 0.98, aquatic: true },
  // --- trees ---
  { name: 'fruit-tree-temperate', archetype: null, tree: true, band: [0.3, 0.7], wetness: 0.6 },
  { name: 'evergreen-cold',       archetype: null, tree: true, band: [0.8, 1.2], wetness: 0.5 },
  // --- mixed-environment pairs (generalists straddling two bands) ---
  { name: 'temperate-arid-edge', archetype: 'Wildflower', band: [0.45, 0.65], wetness: 0.45, second_band: [0.7, 0.95] },
  { name: 'coast-land-shallow',  archetype: 'Reed',       band: [0.2, 0.6], wetness: 0.85, second_band: [0.2, 0.6], aquatic_second: true },
]

const BIN = './target/debug/evolvarium'

// Structured return from each tuner agent.
const TUNE_SCHEMA = {
  type: 'object',
  required: ['niche', 'result_path', 'health_score', 'reached_target', 'survived', 'frictions'],
  properties: {
    niche: { type: 'string' },
    result_path: { type: 'string', description: 'path to the BEST result.json this agent produced (merged into the library)' },
    health_score: { type: 'number' },
    reached_target: { type: 'boolean' },
    survived: { type: 'number' },
    rounds: { type: 'number', description: 'how many scenario runs it took' },
    frictions: {
      type: 'array',
      description: 'balance frictions hit (gene pegging 0/1, impossible niche, free-lunch combo, extinction trigger)',
      items: { type: 'string' },
    },
  },
}

phase('build')
// build once up front so the parallel tuners reuse the compiled binary (no first-build race).
await agent(`Run \`cargo build\` in /home/marc/Documents/Github/evolvarium and report only "ok" or the error tail.`, {
  label: 'build', phase: 'build',
})

phase('tune')
const results = await pipeline(NICHES, (niche) =>
  agent(
    `You tune a cohort of plants/trees toward SURVIVAL + GROWTH in one environment niche, using the evolvarium
--scenario tuning harness. You may ONLY use the scenario JSON interface + run the binary. Do NOT edit any
Rust/source/config files. Work in /home/marc/Documents/Github/evolvarium.

NICHE: ${JSON.stringify(niche)}

THE HARNESS:
- Write a scenario JSON, then run: ${BIN} --scenario=<scn.json> --out=<result.json> --seed=<K>
- Run each candidate at 3 seeds (e.g. 1, 2, 3) and average — one seed is noisy.
- Scenario input fields:
    seed (int), ticks (int; use 12000), target_count (int; use 30),
    world: { lat_band:[lo,hi] (|lat| radians), wetness:0..1, aquatic:bool, rocky:bool, fire:0..1,
             grazers:int, second_band:[lo,hi]|null },
    plant_cohort: [ { count:int (start 10), archetype:"<Name>"|null, tree:bool,
                      genome:{ <any PlantGenome gene>: <value>, ... } } ]
  The genome object is FREE-FORM: override ANY plant gene by name (wet, temp_pref, succulence, light_pref,
  defense, regrow, fruiting, height, submerged, nitrogen_fix, seed_weight, windborne, clonal, maturity,
  spread, nutrient, quality, toxicity, ... and any gene added later). Unknown keys warn + are ignored.
- Result JSON fields you read: started, survived, peak_count, reached_target, mean_mass, max_mass,
  births, deaths, r (births/deaths; aim >= 1), mean_growth_rate, deaths_by_cause {moisture,temp,drown,
  desiccate,habitat,fire,eaten}, trait_drift {gene:[seeded_mean,survivor_mean]}, health_score, best_genomes.

OBJECTIVE (maximize health_score, which rewards survival * growth-toward-30 * R>=1):
  1) survival_rate high, 2) peak_count reaches target_count (30), 3) R >= 1 (self-sustaining),
  4) hold the niche identity (a tree stays a tree, an aquatic stays aquatic).

METHOD (iterate ~4-6 rounds):
  - Start from the niche's archetype + band. Run 3 seeds. Read deaths_by_cause to see WHY it dies:
      moisture -> tune \`wet\` toward the band's effective moisture (or raise \`succulence\` for drought);
      temp     -> tune \`temp_pref\` toward the band's temperature;
      drown/desiccate -> fix \`wet\`/\`submerged\` vs aquatic-vs-land; habitat -> wrong band, shift lat_band;
      eaten    -> raise \`defense\`/\`regrow\`/\`toxicity\` (only if grazers>0).
  - Also watch trait_drift: a gene whose survivor_mean pulls hard away from seeded_mean wants tuning toward
    the survivor value. A gene pegging to 0 or 1 across runs is a FRICTION — note it.
  - Re-run, keep the best (highest mean health_score across seeds). Stop when reached_target + R>=1, or after
    ~6 rounds of no improvement.
  - You may seed from the existing plant-library.json (if it has entries for this niche) to continue evolving
    prior winners: paste a best_genome's fields into the genome override.

DELIVERABLE: leave your single BEST result.json at /tmp/tune-${niche.name}/best.json (re-run the best config
to that exact path). Return JSON per the schema: niche, result_path (that best.json path), health_score,
reached_target, survived, rounds, and frictions (anything that smells like a balance bug — gene pegging,
impossible niche, free-lunch combo, extinction trigger).`,
    { label: `tune:${niche.name}`, phase: 'tune', schema: TUNE_SCHEMA },
  ),
)

phase('synthesize')
const good = results.filter(Boolean).filter((r) => r.survived > 0 && r.result_path)
const frictions = results.filter(Boolean).flatMap((r) => (r.frictions || []).map((f) => `[${r.niche}] ${f}`))

// One synthesize agent runs the merges SEQUENTIALLY (the library file is shared; merge load+save must not
// race), then smokes the seeded planet and appends frictions. Returns a short report.
const report = await agent(
  `You finalize the plant tuning run. Work in /home/marc/Documents/Github/evolvarium.

1) MERGE each niche's best result into the seed-bank library, ONE AT A TIME (sequential — the library file is
   shared, do not parallelize). For each entry below run:
     ${BIN} --merge=<result_path> --niche=<niche> --plant-lib=plant-library.json --lib-cap=8
   Entries:
${good.map((r) => `     - niche=${r.niche} result_path=${r.result_path} (health ${r.health_score?.toFixed?.(2)})`).join('\n')}

2) SMOKE the library-seeded planet (must boot + stay stable):
     ${BIN} --headless --gens=3 --plant-lib=plant-library.json --no-load
   Confirm it runs to "headless run done" with a non-trivial plant + tree count in the last gen log.

3) Append any FRICTIONS below to /home/marc/Documents/Github/clients/evolvarium/tuning-frictions.md (append-only,
   each as a new "## F<n>" style entry with the niche + symptom). Frictions:
${frictions.length ? frictions.map((f) => `     - ${f}`).join('\n') : '     (none reported)'}

Return a short plain-text report: how many library entries now exist, the smoke result, and frictions logged.`,
  { label: 'synthesize', phase: 'synthesize' },
)

return {
  niches_tuned: good.length,
  reached_target: good.filter((r) => r.reached_target).map((r) => r.niche),
  frictions,
  report,
}
