export const meta = {
  name: 'balance-niches',
  description: 'Auto-tune src/config.rs constants until creature niches self-sustain (fewer rescues), scored by --metrics',
  whenToUse: 'When --until-sustain keeps rescuing a niche: search config constants for a tweak that lets the weak niches (aerial/aquatic/cold/warm) live without propping highland too hard.',
  phases: [
    { title: 'Baseline', detail: 'run current config -> baseline rescues_total' },
    { title: 'Hypothesize', detail: 'panel proposes bounded config-const tweaks' },
    { title: 'Test', detail: 'serial: apply -> build --release -> run -> score -> revert' },
  ],
}

// args: { candidates?, capGens?, seed? }. Serial testing on the MAIN repo (no worktrees): shared config.rs +
// target -> parallel builds would collide, and serial incremental builds are cheaper than N full worktree builds.
const REPO = '/home/marc/Documents/Github/evolvarium'
const BIN = `${REPO}/target/release/evolvarium`
const SEED = (args && args.seed) || 'evolved-continuous.json'
const CAP_GENS = (args && args.capGens) || 2 // fixed-length run so rescues_total is comparable across candidates
const N = (args && args.candidates) || 4
const RUN = `cd ${REPO} && ${BIN} --headless --gens=${CAP_GENS} --load=${SEED}`

const METRIC_SCHEMA = {
  type: 'object',
  required: ['ok', 'rescues_total', 'pop', 'niches_extinct'],
  properties: {
    ok: { type: 'boolean', description: 'true if it built + ran + wrote metrics' },
    rescues_total: { type: 'number', description: 'sum of per-niche rescues (LOWER = more balanced); -1 if failed' },
    pop: { type: 'number' },
    niches_extinct: { type: 'number' },
    sustained: { type: 'boolean' },
    per_niche: { type: 'string', description: 'compact "name:count/rescues" list' },
    error: { type: 'string', description: 'build/run error tail if ok=false' },
  },
}

const HYP_SCHEMA = {
  type: 'object',
  required: ['const_name', 'old_value', 'new_value', 'rationale', 'target'],
  properties: {
    const_name: { type: 'string', description: 'exact pub const identifier in src/config.rs' },
    old_value: { type: 'string', description: 'current literal (number) of that const' },
    new_value: { type: 'string', description: 'proposed literal (number), within ~50% of old' },
    rationale: { type: 'string' },
    target: { type: 'string', description: 'which weak niche(s) this helps + predicted effect on rescues_total' },
  },
}

phase('Baseline')
const baseline = await agent(
  `In ${REPO} the release binary is already built from the committed config. Run a clean baseline:\n` +
    `\`${RUN} --metrics=/tmp/bal-baseline.json\` then read /tmp/bal-baseline.json and report its fields.\n` +
    `Do NOT edit any file. Return the metrics.`,
  { label: 'baseline', phase: 'Baseline', schema: METRIC_SCHEMA },
)
const baseRescues = baseline && baseline.ok ? baseline.rescues_total : 9999
log(`baseline rescues_total = ${baseRescues} (pop ${baseline && baseline.pop})`)

phase('Hypothesize')
// Cheap parallel panel: each proposes ONE distinct config-const tweak. No builds here.
const lenses = [
  'reduce highland DOMINANCE (it has 79 pop / 0 rescues and starves the others): make the alpine/highland advantage smaller or its flat-terrain penalty real',
  'help the AERIAL niche (most rescues): lower flight upkeep / altitude-hold cost so birds survive',
  'help COLD + WARM latitudinal niches: soften the temp-preference mismatch energy cost so poles/equator are livable',
  'help the AQUATIC niche: lower swim land-cost or raise in-water food/speed so fish self-sustain',
]
const hyps = (
  await parallel(
    Array.from({ length: N }, (_, i) => () =>
      agent(
        `You tune an artificial-life sim. Goal: lower rescues_total (currently ${baseRescues}) by making weak creature niches self-sustain WITHOUT crashing total pop or making highland extinct.\n` +
          `Read ${REPO}/src/config.rs (all the pub const balance knobs), ${REPO}/src/niche.rs (niche classification + floors), and grep ${REPO}/src/sim.rs for where your target constant is USED so your tweak has the intended effect.\n` +
          `Baseline metrics: ${JSON.stringify(baseline)}.\n` +
          `Your angle: ${lenses[i % lenses.length]}.\n` +
          `Propose exactly ONE numeric pub const in src/config.rs to change, within ~50% of its current value. Do NOT edit files. Return the proposal.`,
        { label: `hyp:${i}`, phase: 'Hypothesize', schema: HYP_SCHEMA },
      ),
    ),
  )
).filter(Boolean)
log(`${hyps.length} hypotheses: ${hyps.map((h) => `${h.const_name} ${h.old_value}->${h.new_value}`).join(' | ')}`)

phase('Test')
// SERIAL on main repo: apply one tweak, build, run, score, REVERT. Serial avoids config.rs/target collisions.
const results = []
for (let i = 0; i < hyps.length; i++) {
  const h = hyps[i]
  const r = await agent(
    `In ${REPO}, test ONE config tweak, then REVERT it. Steps, in order:\n` +
      `1. Edit src/config.rs: set \`pub const ${h.const_name}\` to ${h.new_value} (change ONLY that const's value, keep type + comment).\n` +
      `2. \`cd ${REPO} && cargo build --release 2>&1 | tail -3\`. If it fails to compile: \`git checkout src/config.rs\` and return ok=false with the error.\n` +
      `3. \`${RUN} --metrics=/tmp/bal-cand-${i}.json\` then read /tmp/bal-cand-${i}.json.\n` +
      `4. ALWAYS \`cd ${REPO} && git checkout src/config.rs\` to revert, and confirm \`git diff --quiet src/config.rs\` (clean).\n` +
      `Return the metrics from step 3 (rescues_total, pop, niches_extinct, sustained, and a compact per_niche "name:count/rescues" list).`,
    { label: `test:${h.const_name}`, phase: 'Test', schema: METRIC_SCHEMA },
  )
  results.push({ hyp: h, metric: r })
}

// rank: valid (ok, pop healthy, highland not extinct via niches_extinct guard) by lowest rescues_total
const ranked = results
  .filter((x) => x.metric && x.metric.ok && x.metric.pop >= 40 && x.metric.niches_extinct === 0)
  .sort((a, b) => a.metric.rescues_total - b.metric.rescues_total)
const winner = ranked[0]
const improved = winner && winner.metric.rescues_total < baseRescues

return {
  baseline: { rescues_total: baseRescues, pop: baseline && baseline.pop },
  capGens: CAP_GENS,
  candidates: results.map((x) => ({
    const: x.hyp.const_name,
    change: `${x.hyp.old_value}->${x.hyp.new_value}`,
    rescues_total: x.metric ? x.metric.rescues_total : null,
    pop: x.metric ? x.metric.pop : null,
    extinct: x.metric ? x.metric.niches_extinct : null,
    target: x.hyp.target,
  })),
  winner: improved
    ? { const: winner.hyp.const_name, change: `${winner.hyp.old_value}->${winner.hyp.new_value}`, rescues_total: winner.metric.rescues_total, rationale: winner.hyp.rationale }
    : null,
  note: improved ? 'apply winner to src/config.rs and commit' : 'no candidate beat baseline; widen search or try multi-const changes',
}
