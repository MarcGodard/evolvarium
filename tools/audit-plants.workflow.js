export const meta = {
  name: 'audit-plants',
  description: 'Fan out agents to verify plant/tree behavioral RULES hold via controlled --scenario experiments',
  whenToUse: 'QA the flora: confirm each design rule (climate niches, drown/desiccate, succulence, grazing arms race, growth trade-offs, dispersal, tree size/reach/sterility, no-zombies) actually holds in the sim.',
  phases: [
    { title: 'build', detail: 'compile the binary once' },
    { title: 'audit', detail: 'one agent per rule runs a controlled A/B experiment and judges PASS/FAIL' },
    { title: 'report', detail: 'aggregate verdicts + flag rule violations' },
  ],
}

const BIN = './target/debug/evolvarium'
const DIR = '/home/marc/Documents/Github/evolvarium'

// Each RULE is a property the flora MUST exhibit. `test` is a concrete recipe the agent runs (usually an A/B:
// same band, vary ONE gene, or same plant in two bands) + the expected signal in the result JSON. Agents
// confirm or refute it from real runs (3 seeds), not from reading code.
const RULES = [
  { id: 'R1-aquatic-desiccate', test:
    `Aquatic plant must DIE on dry land (desiccation) but live in water. A: archetype "Eelgrass" genome {wet:0.95,submerged:0.6} in world {lat_band:[0.4,0.7], wetness:0.2, aquatic:false} (dry land). B: same plant in world {lat_band:[0.2,0.5], wetness:0.95, aquatic:true} (water). PASS if A has low survival with 'desiccate' a top death cause AND B survives well.` },
  { id: 'R2-land-drown', test:
    `Land plant must DROWN when submerged. A: archetype "Clover" genome {wet:0.4} in world {lat_band:[0.2,0.5], wetness:0.9, aquatic:true} (submerged). B: same on land {wetness:0.45, aquatic:false}. PASS if A dies mostly by 'drown' and B survives.` },
  { id: 'R3-succulence-drought', test:
    `Succulence must buffer DROUGHT. Dry band world {lat_band:[0.5,0.8], wetness:0.12}, archetype "Cactus", both cohorts genome wet:0.6. A: succulence:0.0. B: succulence:0.9. PASS if B survival >> A (B far fewer moisture deaths).` },
  { id: 'R4-temp-niche-cold', test:
    `Cold-adapted plant lives cold + dies hot; warm-adapted the reverse. Cold band {lat_band:[1.05,1.45], wetness:0.4}: A temp_pref:0.2, B temp_pref:0.85. PASS if A survives and B dies (temp deaths). (Optionally confirm reverse in a hot band [0.0,0.3].)` },
  { id: 'R5-wet-niche', test:
    `A plant matched to the band's moisture lives; mismatched is moisture-stressed. Band {lat_band:[0.4,0.7], wetness:0.6}: A wet:0.6 (matched), B wet:0.1 (too dry for itself) and C wet:0.95 (wants it wetter). PASS if A survives best and B/C show more 'moisture' deaths. (wetness IS effective moisture.)` },
  { id: 'R6-light-niche', test:
    `Shade plants must out-grow sun plants in dim light and vice versa. Deep aquatic dim light: world {lat_band:[0.2,0.6], wetness:0.98, aquatic:true}, archetype "Kelp". A: light_pref:0.2 (shade). B: light_pref:0.9 (sun). PASS if A has higher mean_mass/survival than B in the dim deep.` },
  { id: 'R7-defense-vs-grazing', test:
    `Defense must reduce being eaten under grazers. Band {lat_band:[0.4,0.7], wetness:0.6, grazers:30}, archetype "BerryBush": A defense:0.05, B defense:0.8. PASS if B has fewer 'eaten' deaths / higher survival than A. (Run grazers; without grazers defense should NOT help.)` },
  { id: 'R8-regrow-survives-bites', test:
    `High regrow survives grazing (regrows after a bite); low regrow is consumed whole. Band {wetness:0.6, grazers:30}: A regrow:0.0, B regrow:0.9. PASS if B survives grazing better than A.` },
  { id: 'R9-toxicity-deters', test:
    `Toxicity must deter eaters (toxic plants eaten less). Band {wetness:0.6, grazers:30}: A toxicity:0.0, B toxicity:0.8. PASS if B has fewer 'eaten' deaths than A. NOTE if toxicity barely changes eaten counts (a known soft-selection friction).` },
  { id: 'R10-growth-tradeoff', test:
    `No free lunch: investing in defense/nutrient/succulence/toxicity must SLOW growth. Good band {lat_band:[0.4,0.7], wetness:0.6, grazers:0}: A a cheap plant {defense:0.0,toxicity:0.0,succulence:0.0,nutrient:0.2}. B an expensive plant {defense:0.9,toxicity:0.9,succulence:0.9,nutrient:0.9}. PASS if A mean_mass / mean_growth_rate clearly > B.` },
  { id: 'R11-dispersal-spreads', test:
    `Reproduction + dispersal must grow a cohort in a good band. Band {lat_band:[0.4,0.7], wetness:0.6}, start count 8, target 30, ticks 16000. archetype "Wildflower". PASS if peak_count rises well above the start (reaches target) with R>=1. Compare windborne:0.9 vs windborne:0.0 — windborne should not REDUCE establishment.` },
  { id: 'R12-fire-serotiny', test:
    `Serotiny: a fire_seed plant should recruit after fire (release seed on burn). Band {lat_band:[0.5,0.8], wetness:0.3, fire:0.6}, archetype "Tumbleweed": A fire_seed:0.0, B fire_seed:0.8. Expect fire deaths in both; PASS if B sustains/recovers its cohort better than A (more births / higher final_count) despite the burns. Mark UNCLEAR if the signal is too noisy.` },
  { id: 'R13-tree-bigger-good-soil', test:
    `NEW RULE: trees grow BIGGER on good soil (moisture sweet spot + fertile). Tree cohort (plant_cohort tree:true, no archetype), ticks 16000, target 12. A good soil {lat_band:[0.4,0.7], wetness:0.5}. B too-wet {wetness:0.95}. C too-dry {wetness:0.1}. PASS if A mean_mass / max_mass is clearly larger than B and C (A should reach mass well above TREE_MATURITY=14, up to ~30+).` },
  { id: 'R14-tree-land-only', test:
    `Trees are land-only: a tree in an aquatic band drowns. A: tree cohort in world {lat_band:[0.2,0.5], wetness:0.9, aquatic:true}. B: same on land {wetness:0.5, aquatic:false}. PASS if A dies (drown) and B survives.` },
  { id: 'R15-tree-not-sterile', test:
    `Trees must NOT be sterile zombies: a tree cohort in a good band must MATURE + reproduce, not sit static. Tree cohort, world {lat_band:[0.3,0.7], wetness:0.5}, ticks 20000, target 12. PASS if births > 0 (R>0) and mean_mass >= TREE_MATURITY (14) i.e. trees actually grow up + seed. FAIL if births=0 / R=0 (sterile).` },
  { id: 'R16-no-zombie-plants', test:
    `No zombie plants: in a clearly LETHAL band a cohort must actually DIE off, not persist. Band {lat_band:[1.2,1.5], wetness:0.05} (frozen + bone dry), archetype "Wildflower" genome {temp_pref:0.85, wet:0.7} (badly mismatched). PASS if survival_rate is low (cohort collapses) with deaths attributed (temp/habitat/moisture). FAIL if a chunk of the cohort persists indefinitely.` },
]

const SCHEMA = {
  type: 'object',
  required: ['rule_id', 'verdict', 'evidence'],
  properties: {
    rule_id: { type: 'string' },
    verdict: { type: 'string', enum: ['PASS', 'FAIL', 'UNCLEAR'] },
    evidence: { type: 'string', description: 'the concrete numbers from the runs that justify the verdict (survival, deaths_by_cause, mean_mass, R, across seeds)' },
    note: { type: 'string', description: 'any friction / surprise / suspected bug worth a human look' },
  },
}

phase('build')
await agent(`Run \`cargo build\` in ${DIR}; report only "ok" or the error tail.`, { label: 'build', phase: 'build' })

phase('audit')
const results = await pipeline(RULES, (rule) =>
  agent(
    `You are a QA auditor verifying ONE behavioral rule of the evolvarium flora using the --scenario harness.
Run REAL controlled experiments (do not judge from reading source). ONLY use the scenario JSON + the binary;
do NOT edit any source/config. Work in ${DIR}. Binary: ${BIN}.

RULE ${rule.id}:
${rule.test}

HARNESS:
- Write scenario JSON(s) under /tmp/audit-${rule.id}/ and run: ${BIN} --scenario=<s.json> --out=<r.json> --seed=<K>
- Run each cohort/condition at seeds 1,2,3 and AVERAGE (one seed is noisy). For an A/B, run BOTH at the same seeds.
- Scenario input: { seed, ticks (default 12000), target_count (default 30),
    world:{ lat_band:[lo,hi] (|lat| radians, 0=equator..1.57=pole), wetness (0..1 = effective local moisture),
            aquatic:bool, rocky:bool, fire:0..1, grazers:int, second_band:[lo,hi]|null },
    plant_cohort:[ { count:10, archetype:"<Name>"|null, tree:bool, genome:{ <any gene>:<value> } } ] }
  For an A/B, prefer SEPARATE runs (one cohort each) so survival/deaths are cleanly attributable to that cohort.
- Result fields: started, survived, peak_count, final_count, reached_target, mean_mass, max_mass, mean_age,
  births, deaths, r, mean_growth_rate, deaths_by_cause {moisture,temp,drown,desiccate,habitat,fire,eaten},
  trait_drift {gene:[seeded_mean,survivor_mean]}, health_score, best_genomes.

JUDGE: run the experiment, read the numbers, and decide:
- PASS = the rule clearly holds (state the gap: e.g. "B survived 28/30 vs A 3/30, A deaths 'desiccate'=21").
- FAIL = the rule is violated (the expected effect is absent or reversed) -> a real bug.
- UNCLEAR = signal too noisy / not separable across seeds (say what you'd need).
Be skeptical and quantitative. Return JSON per the schema with the rule_id, verdict, evidence (the numbers),
and a note for any friction/surprise.`,
    { label: rule.id, phase: 'audit', schema: SCHEMA },
  ),
)

phase('report')
const v = results.filter(Boolean)
const fails = v.filter((r) => r.verdict === 'FAIL')
const unclear = v.filter((r) => r.verdict === 'UNCLEAR')
const report = await agent(
  `Write a concise plant/tree RULE-AUDIT report from these verdicts (one line per rule: id, verdict, the key
evidence numbers). Lead with a summary: X PASS / Y FAIL / Z UNCLEAR. Then list the FAILs first (these are
real bugs -- spell out the violation), then UNCLEARs (what's needed), then PASSes (one line each). Append any
notable frictions. Verdicts:
${v.map((r) => `- ${r.rule_id}: ${r.verdict} | ${r.evidence}${r.note ? ' | note: ' + r.note : ''}`).join('\n')}`,
  { label: 'report', phase: 'report' },
)

return { total: v.length, pass: v.length - fails.length - unclear.length, fail: fails.map((r) => r.rule_id), unclear: unclear.map((r) => r.rule_id), report }
