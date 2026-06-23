# Parallelization Plan: full multi-core sim tick

Execution plan for parallelizing the WHOLE simulation tick across cores. Written to be executed by an AI
agent (Claude Code) phase by phase. Scope is "full parallelization, whatever the work" per owner request.

## Goal

Make one sim tick use all available cores so a SINGLE headless run and FAST-FORWARD visualization (virtual
clock cranked past ~1x) scale with core count. Correctness + run-to-run determinism preserved.

Not the goal (already handled, do not redo):
- Campaign throughput: already saturates cores by running MANY independent headless sims (island/replicate
  model via the workflows). Intra-tick threading does not help that.
- Normal-speed visualizer (<=1x): render/GPU-bound, not sim-bound. Parallelizing the sim changes nothing
  visible there. Smoothness/more-creatures at normal speed is a RENDER job (instancing/LOD/culling), out of
  scope here.

Where this DOES help: headless single runs, and the visualizer when sped up to watch evolution fast (the one
viz case where sim-CPU is the ceiling). Gains are Amdahl-bounded by whatever stays serial.

## Current state (why it is serial today)

- Headless spins flat-out (`ScheduleRunnerPlugin::run_loop(Duration::ZERO)`); render steps `FixedUpdate` fed by
  a `Time<Virtual>` whose `relative_speed` the user controls (default 0.35x). Both run the SAME system chain.
- Systems are `.chain()`ed -> forced serial, because they conflict on shared data.
- Single-threaded. No `par_iter` / rayon anywhere. `Cargo.toml` already has a release profile (opt3+thinLTO);
  always build/run `--release` for perf work.
- The hot systems interleave, inside one loop, three things that fight parallelism:
  1. per-entity reads of shared snapshots (food grid `fgrid`, creature snapshot `cre_snap`, `mate_pool`),
  2. per-entity writes to the entity's OWN components (safe to parallelize),
  3. SIDE EFFECTS: `ResMut<Rng>` draws, `Commands` spawn/despawn, shared `ResMut` (Soil, TreeBites, SeedBank),
     and shared dedup state (`eaten: HashSet`).

The side effects (3) are what block a naive `par_iter_mut`. The plan is to separate them out everywhere.

## Core pattern: snapshot -> parallel decide -> serial apply

Apply this same three-phase shape to every hot system:

1. SNAPSHOT (serial, cheap): build read-only views. live_step already does this (foods, fgrid, cre_snap,
   mate_pool). Reuse / extend.
2. DECIDE (parallel, `par_iter_mut`): each entity reads snapshots + writes ONLY its own components (movement,
   altitude, brain forward + learn, basal metabolism). It produces an INTENT describing any side effect it
   wants (eat food#i, reproduce with partner#j, deposit soil at p, bite tree#e, die). Intents collected via
   Bevy `Parallel<Vec<Intent>>` (per-thread buffers, zero contention) or a per-entity buffer component.
3. APPLY (serial): drain intents in a DETERMINISTIC order, resolve conflicts (eaten dedup, mate pairing),
   perform all RNG-dependent outcomes + `Commands` spawn/despawn + shared-resource writes (Soil, TreeBites,
   SeedBank). This phase is small relative to decide, so Amdahl stays favorable.

## Determinism strategy (run-to-run reproducible, NOT bit-identical to old serial)

Parallel float-reduction + iteration order differ, so output will not byte-match the old serial code (the
code already notes grid iteration is not bit-identical). We hold a WEAKER but sufficient invariant: same seed
-> same result across runs of the NEW code, and statistically-equivalent dynamics vs the old code.

- Per-entity deterministic RNG: in the snapshot phase, give each entity a small RNG seeded from
  `hash(global_seed, stable_entity_index, tick)`. Decide-phase randomness draws from THAT, never the shared
  `ResMut<Rng>`. Result independent of thread scheduling. The shared Rng is used only in the serial apply
  phase (births etc.), in deterministic intent order.
- Deterministic apply order: sort intents by stable entity index before applying so conflict resolution
  (eaten, mate choice, density caps) is scheduling-independent.
- Reproducibility test (Phase gate): run twice same seed -> identical per-gen stats.

## Infrastructure to build first

- Deterministic per-entity RNG (splitmix64/pcg) seeded by (global_seed, entity_index, tick). New small module.
- Intent types per system + a `Parallel<Vec<_>>` collector pattern + serial drain helper.
- Stable entity index: a per-entity component assigned at spawn (monotonic counter) so RNG seeding + apply
  ordering are stable across ticks and independent of ECS internal ordering.
- Optional shared spatial grid resource (creature grid, like the existing food `fgrid`) to cut predation +
  social from O(N^2) to O(N*k). Reused read-only across decide phases.

## Per-system plan (RETARGETED from Phase 0 profile, 2026-06-23)

Profile overturned the original guess: the flora steps dominate (grass+plant+seaweed = 94.3% of tick);
`live_step` is only 2.1% and predation is noise. Drives the order below. Grass blades (`GRASS_CAP=8000`) are
the largest entity set on the planet, hence grass_step is #1.

A. `grass_step` (62.8% of tick, ~10.3 ms; iterates `GRASS_CAP`=8000 turf tufts). Blades are independent.
   DECIDE: per-tuft grow/stress/cull fate (parallel, own components). APPLY: spawn new tufts (RNG +
   SeedBank) + despawn dead + soil deposits (serial, deterministic). HIGHEST win by far.

B. `plant_step` (24.4%, ~4.0 ms; ~4300 plants + 270 trees). DECIDE: growth, stress, mortality, fruit/seed
   readiness per plant (parallel). APPLY: despawn dead, spawn seedlings/fruit-drops (RNG + SeedBank),
   tree_bites + soil deposits (serial). Second-biggest win.

C. `seaweed_step` (7.1%, ~1.2 ms; `SEAWEED_CAP`=3500 fronds). Same shape as grass. DECIDE per-frond grow/
   cull (parallel). APPLY spawn/despawn (serial). Co-implement with grass (near-identical pattern).

D. `weather_step` (3.3%, ~0.5 ms) + `fire_step` (0.2%) + climate grids. Grid-cell loops; parallelize cell
   chunks only if A-C don't already hit the Amdahl ceiling (low priority).

E. `live_step` (creatures, only 2.1%, ~0.34 ms). DECIDE: sensing via fgrid, brain forward + learn, movement,
   altitude, basal burn, eat-target selection, repro-eligibility (parallel, own components). APPLY: eat
   resolution with `eaten` dedup + soil/tree_bites, death rolls + carrion + soil, reproduction crossover/
   mutate/spawn (serial, deterministic). Most intricate, but LOW payoff at current pop: only do if it stays
   cheap and the equivalence gate is easy, OR if creature pop grows much larger later.

F. `predation_step` (0.0%, ~2 us) + `rot_step` (~7 us). Negligible. Skip parallelization (the O(N^2) scan
   is cheap at current pop). Revisit only if creature pop scales up an order of magnitude.

G. System-level parallelism: after A-C, unchain genuinely independent systems so Bevy's multithreaded
   executor runs them concurrently where data deps allow (limited; most conflict on creature/plant/food).

## Phases (each independently shippable + has a GREEN GATE; commit at each gate)

- Phase 0 - PROFILE. DONE 2026-06-23. `--profile` headless flag (src/profile.rs: global thread-safe scope
  guard at top of each hot system, report system prints cumulative ranking every 600 ticks). Result below.
  Retargeted per-system order (grass > plant > seaweed). No sim behavior change.
- Phase 1 - INFRA. Deterministic per-entity RNG module + stable entity-index component + intent/`Parallel`
  scaffolding + the serial-drain helper. Wire but do not parallelize yet (behavior byte-identical). Gate:
  cargo test, --gens smoke unchanged.
- Phase 2 - plant_step (or Phase-0 top system) parallelized. Gate: equivalence + determinism + perf (below).
- Phase 3 - live_step decide/apply split + parallel decide. Gate: full equivalence suite (most sensitive).
- Phase 4 - predation_step + creature spatial grid. Gate.
- Phase 5 - grass/seaweed/rot. Gate.
- Phase 6 - weather/fire/climate grids (only if Phase 0 flags them). Gate.
- Phase 7 - system-level unchaining + final report. Gate + update BACKLOG.

Each phase: implement -> run the verification gate -> if green, commit + push (`origin main`, mirror `main:build`).
If a gate fails, fix or revert that phase; never leave the tree red.

## Verification gate (run every phase, scripted)

1. Correctness: `cargo test` (currently 18) + `cargo run -- --headless --gens=1` smoke.
2. Determinism: NEW build, same `--seed`, run twice -> identical per-gen stats. Must match exactly.
3. Equivalence vs pre-phase baseline: run the SAME seed before/after the phase for `--gens=15`; compare
   per-gen aggregates (pop, mean energy, niche counts via `--metrics`, trait means, plants). Must stay within
   a tolerance band (define: niche counts +-15%, means +-10% over the run; no new extinctions). Use the
   subagent harness to fan this out across several seeds and judge PASS/FAIL (this is where "use the harness"
   pays off: parallel equivalence judges per seed + an adversarial reviewer).
4. Performance: `--profile` tick-time + gens/sec at fixed pop, before vs after. Record speedup. Expect
   near-linear on the parallelized phase, overall capped by remaining serial work (Amdahl).
5. No balance regression: niche `--metrics` rescues_total not materially worse for the same seed.

## Risks + mitigations

- Determinism drift -> per-entity seeded RNG + deterministic apply order (above); Phase gate #2 catches it.
- Behavior regression on SELECTION (subtle ordering effects change who eats/breeds) -> equivalence gate #3
  across multiple seeds with tolerance bands + adversarial review.
- Borrow-checker fights (par_iter_mut + Commands/ResMut) -> the intent/`Parallel` pattern sidesteps it; no
  shared mutable state touched inside decide.
- eaten/mate contention correctness -> resolved only in serial apply, deterministic order.
- Memory: intent buffers per tick -> reuse buffers across ticks (clear, not realloc).
- Amdahl ceiling: if Phase 0 shows side-effect/apply or an unparallelized system dominates, gains cap out.
  Phase 0 prevents wasted effort; re-evaluate scope after it.
- Shared worktree: another agent or a running campaign editing src/ will collide. Require an UNCONTENDED tree
  before executing (no concurrent agent, no running tune/balance workflow). Build artifacts churn, so this
  cannot run alongside a campaign that builds the binary.

## Expected payoff (set expectations)

- Headless single run + fast-forward viz: real but Amdahl-bounded. If decide+plant together are ~70% of the
  tick and parallelize ~Ncore, overall is roughly 1/(0.3 + 0.7/Ncore) -> e.g. ~2.5x at 8 cores, not 8x.
  Phase 0 will sharpen this estimate.
- Normal-speed viz + campaign throughput: no change (by design; see Goal non-goals).

## How to ask me to execute

Preconditions: tree must be uncontended (no other agent editing src/, no tune/balance workflow running, since
those build the binary). Confirm the current campaign has finished first.

Trigger phrases:
- "Execute the parallelization plan" -> I run Phase 0 first (profile), report the hot-spot ranking, then
  proceed phase by phase, committing at each green gate, pausing to show you the gate results between phases.
- "Execute parallelization phase N" -> just that phase (Phase 0 must have run first; Phase 1 infra before
  2+).
- "Execute the parallelization plan, do not pause" -> run straight through all phases, only stopping on a
  failed gate. (Uses the subagent harness for the equivalence fan-out.)
- "Re-profile" -> re-run Phase 0 only (e.g., after pop/balance changes) and update the ranking here.

I will: build/run `--release`, gate every phase, commit+push at each green gate, and keep this file's Status
section updated as the source of truth.

## Status

- [x] Phase 0 - profile + retarget (2026-06-23)
- [x] Phase 1 - infra: Rng::for_entity (per-entity deterministic stream) + GenState.seed. entity.index() is
      the stable id (deterministic spawn order), no extra component needed. (2026-06-23)
- [x] Phase 2 - grass_step parallel (snapshot->par decide->serial apply). 10333->1972 us/tick = 5.2x.
      Overall tick 16.5->8.2 ms. Deterministic; equivalent (plants/trees <1.5%, traits within seed noise). (2026-06-23)
- [ ] Phase 3 - plant_step (now 48% of tick, top hotspot)
- [ ] Phase 4 - seaweed_step (7.1%)
- [ ] Phase 5 - weather/climate grids (3.3%, if not already Amdahl-capped)
- [ ] (deferred) live_step / predation - only 2.1% / ~0%; do only if creature pop scales up
- [ ] Phase 6 - system unchaining + final report

Profile data (Phase 0, 2026-06-23): pop with 4349 plants + 270 trees + 8000 grass + 3500 seaweed, single
thread, `--release`, `--headless --gens=2 --profile` @ tick 7200 cumulative:

| system | % tick | mean us/tick |
|--------|-------:|-------------:|
| grass      | 62.8 | 10333 |
| plant      | 24.4 |  4018 |
| seaweed    |  7.1 |  1170 |
| weather    |  3.3 |   536 |
| live       |  2.1 |   344 |
| fire       |  0.2 |    32 |
| rot        |  0.0 |     7 |
| predation  |  0.0 |     2 |
| generation |  0.0 |   0.3 |
| niche      |  0.0 |   0.0 |

Measured tick mean ~16.5 ms = ~61 ticks/s single-thread. Flora (grass+plant+seaweed) = 94.3%. Amdahl
ceiling if those three parallelize ~Ncore at 16 cores: ~1/(0.057 + 0.943/16) ≈ 8.5x (minus serial apply).
