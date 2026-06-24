// Locomotion tuning harness (gym arm; complements tune-creatures, which tunes ECOLOGICAL survival via
// --scenario). One agent per BODY-PLAN niche evolves locomoting creatures in the avian physics gym
// (--gym-evolve), each exploring a distinct seed band so the cohorts are morphologically diverse. The
// synthesize stage folds every niche cohort into a single creature seed (--merge-snap) and smoke-tests the
// seeded planet. Output = a creatures-only seed of gym-vetted, physically-capable bodies, which then feeds a
// planet co-evolution (warm-up evolves brains + ecology on top of the capable bodies).
export const meta = {
  name: 'tune-locomotion',
  description: 'Evolve locomoting creatures per body-plan niche in the physics gym, merge winners into a creature seed',
  whenToUse: 'Build a seed of physically-capable, morphologically-diverse creature bodies before a planet co-evolution.',
  phases: [
    { title: 'build', detail: 'compile the binary once (release) so gym agents reuse it' },
    { title: 'tune', detail: 'one agent per body-plan niche: run --gym-evolve over a seed band, keep the best cohort' },
    { title: 'synthesize', detail: 'merge each niche cohort into the creature seed, smoke the seeded planet' },
  ],
}

const BIN = './target/release/evolvarium'
const SEED_OUT = 'creature-seed.json' // growing creatures-only seed the synthesize stage builds

// Body-plan niches = distinct seed bands (the gym arena is flat land, so diversity comes from founder body
// graphs). Each agent sweeps its band for the best mover cohort. Names are descriptive, not enforced shapes.
const NICHES = [
  { name: 'sprinters-a', seeds: [1, 2, 3, 4] },
  { name: 'sprinters-b', seeds: [11, 12, 13, 14] },
  { name: 'many-legged', seeds: [21, 22, 23, 24] },
  { name: 'compact', seeds: [31, 32, 33, 34] },
  { name: 'long-bodied', seeds: [41, 42, 43, 44] },
  { name: 'mixed', seeds: [51, 52, 53, 54] },
]

const TUNE_SCHEMA = {
  type: 'object',
  required: ['niche', 'cohort_path', 'best_fitness', 'best_seed'],
  properties: {
    niche: { type: 'string' },
    cohort_path: { type: 'string', description: 'path to the BEST cohort snapshot this agent saved (merged into the seed)' },
    best_fitness: { type: 'number', description: 'best gym fitness (horizontal COM travel, minus airborne penalty)' },
    best_seed: { type: 'number' },
    rounds: { type: 'number', description: 'how many --gym-evolve runs it took' },
    notes: { type: 'string', description: 'frictions: exploders, no-movers, fitness plateau, suspected gym bug' },
  },
}

phase('build')
await agent(`Compile the sim in release so the gym tuners reuse it: run \`cargo build --release\`. Report only success/failure.`, { label: 'build', phase: 'build' })

phase('tune')
const results = await pipeline(NICHES, (niche) =>
  agent(
    `You tune ONE creature body-plan niche ("${niche.name}") using ONLY the gym CLI + the compiled binary. Goal: find the seed in your band that evolves the best LOCOMOTING cohort (creatures that travel farthest under their own actuated joints), and leave that cohort saved.

GYM CLI (deterministic, headless, exits on its own):
  ${BIN} --gym --gym-evolve --gym-pop=24 --gym-gens=20 --gym-steps=600 --gym-seed=<S> --save=/tmp/gym-${niche.name}-<S>.json
Prints per-gen \`gym-evolve gen N: best=<fit> mean=<fit>\`, then \`saved top K movers -> <path>\`. Fitness = horizontal COM travel minus an airborne penalty; higher = better mover. Non-finite/exploded bodies score about -1000 and are culled.

DO:
1. Run the GA for each seed in your band: ${JSON.stringify(niche.seeds)}.
2. Compare the final-gen \`best\` fitness across seeds. Pick the seed with the highest best fitness.
3. (Optional) For the winning seed, try one longer run (--gym-gens=30) if best was still climbing at gen 20.
4. Return the cohort path of your best run, its best_fitness + best_seed, and any frictions.

Keep total runs modest (one per seed + maybe one longer). Each run is ~15-30s.`,
    { label: `tune:${niche.name}`, phase: 'tune', schema: TUNE_SCHEMA },
  ),
)

phase('synthesize')
const good = results.filter(Boolean).filter((r) => r.cohort_path && Number.isFinite(r.best_fitness))
const lines = good.map((r) => `  ${BIN} --merge-snap=${r.cohort_path} --snap=${SEED_OUT} --cap=96`).join('\n')
const report = await agent(
  `Build the creature seed from the tuned cohorts, then smoke-test it.

1. Run these merges SEQUENTIALLY (shared seed file, no parallel writes), in order:
${lines}
   Each prints \`merge-snap: +K from <cohort> -> ${SEED_OUT} now has <N> creatures\`.
2. Smoke the seeded planet (must boot + keep a population): \`${BIN} --load=${SEED_OUT} --headless --gens=2\`. Confirm it prints \`continuous headless done\` (or generational gens) with pop > 0 and does NOT crash.
3. Report: final creature count in ${SEED_OUT}, per-niche best_fitness, and whether the smoke passed. Note any niche that produced only exploders/non-movers (best_fitness near 0 or negative) as a friction.`,
  { label: 'synthesize', phase: 'synthesize' },
)

return { niches: good.map((r) => ({ niche: r.niche, best_fitness: r.best_fitness, best_seed: r.best_seed })), seed: SEED_OUT, report }
