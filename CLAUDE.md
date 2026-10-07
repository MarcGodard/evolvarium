# Evolvarium

3D artificial-life sim on a small planet (Rust + Bevy 0.19). Per-creature NN brains evolve by GA + lifetime learning against a co-evolving food web with climate, weather, fire and oceans.

## Current phase: FUNDAMENTALS RETROFIT (chemistry + physics)

Replacing the phenomenological model with conserved chemistry + real physical law. Roadmap: fundamentals, then living ocean, then parasites.

**Organizing principle: matter is closed, energy flows through.** A real biosphere conserves its C/N/P and does NOT conserve energy: it intercepts sunlight and radiates infrared. Encode that asymmetry; everything else follows.

Until this note is removed:

- **Balance runs are back ON.** A change that moves the equilibrium needs a headless run showing it REACHES one and holds. It need not match old numbers, which this phase deliberately invalidates.
- **Conservation is the phase gate.** Any change touching matter carries a unit test asserting world element totals hold across ticks. A path that creates or destroys matter is a bug, not a tuning knob.
- **Prefer deleting a constant over adding one.** Replace invented constants with law (Kleiber, Liebig, Archimedes, Stefan-Boltzmann). A new magic number needs a reason it is not derivable.
- Baseline for comparison: `--gens=5` on seeds 1/5/9, captured before the retrofit began.
- Render-only work no longer skips balance verification.

## Commits

**Standing permission: commit and push when you judge it a good time.** Overrides the workspace per-commit rule for this repo.

- Only at coherent stopping points, tree green: `cargo build` clean, `cargo test` passing, `cargo run -- --headless --gens=1` OK.
- The worktree is often shared: if files outside your change are mid-refactor and do not build, hold the commit.
- Push `origin main`; mirror to backup when convenient: `git push origin main:build`.
- Keep verification runs SHORT (parallel `evolvarium` processes contend for cores): `--gens=1` smoke, `--gens=3..5` balance sanity, `--gens=15+` only when a short run cannot show the trend. Headless logs are block-buffered to a pipe, so prefer a short run that finishes over tailing a long one.

## Running

- **NEVER auto-start the windowed visualizer** (`cargo run` with a window). Use `--headless` or `--capture`, which exit on their own. Flags are in `main.rs`.
- **Redirect `--capture` output to a file, never pipe it.** The PNG is written a few frames after the "capture: writing" log line; piping into `head`/`tail`/`grep` SIGPIPEs the process first, and it exits 2 with no file, which looks like a crash. Use `--capture=X > x.log 2>&1`.

## Plant/tree tuning harness

Design, CLI contract and schemas: `~/Documents/Github/keepfiles/clients/evolvarium/14-tuning-harness.md`. Code: `src/scenario.rs`; workflows in `tools/*.workflow.js` (opt-in, Workflow tool).

- **Gene-agnostic:** overrides, drift and dedup go through serde, so a new `PlantGenome` gene needs zero harness edits (just `#[serde(default)]` + a `mutate()` drift line).
- `plant-library.json` is the tuned seed bank; normal runs seed every biome from it (`--no-plant-lib` disables). Genes added after the library was written are randomized per plant on seed, so do not rebuild the library just for a new gene.
- Balance frictions go to `keepfiles/clients/evolvarium/tuning-frictions.md`.

## Design docs

Specs live in `~/Documents/Github/keepfiles/clients/evolvarium/` (numbered `00-`..`15-`). Read the relevant one before nontrivial design work. `BACKLOG.md` in this repo is the source of truth for done vs open; update it when landing notable work.

## Conventions

- **Every trait has a cost.** Each gene, ability or behavior needs an explicit cost on another axis, or it maxes out and collapses variety (plant defense pegged ~0.9 when its cost was too weak). Name the cost side before shipping a mechanic. Same principle as the one-knob-opposite-signs rule in spec 10.
- Code is AI-built only, so comments target an agent: units/ranges (0..1, radians, ticks), why a constant has its value, cross-file coupling.
- Genome/NN-architecture changes invalidate saved seeds: gate or regenerate them.
