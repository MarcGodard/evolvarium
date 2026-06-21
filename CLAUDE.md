# CLAUDE.md — Evolvarium

Guidance for Claude Code working in this repo. This file is project-specific and takes precedence over the
parent workspace `CLAUDE.md` for anything that conflicts.

## What this is

A 3D artificial-life sim on a small **planet** (Rust + Bevy 0.18). Tiny per-creature neural-net brains
(genome = weights + sensors + traits) forage, eat, fight, breed, and learn during their lives; a genetic
algorithm + lifetime learning evolve them against a living, co-evolving food web on a sphere with day/night,
clouds, rain, wildfire, oceans, mountains, and cold-pole/warm-equator climate.

## Current phase: VISUALS FIRST, balance later

We are in a **visuals-first** phase: polish how the world looks and feels (clouds, weather, lighting,
terrain, creatures, effects) before tuning the simulation. Until this note is removed:

- **Do NOT gate visual work on population balance.** Skip the headless balance / carrying-capacity sanity
  runs for render-only changes. Build + a `--gens=1` smoke (so it compiles + boots) is enough.
- Population balancing and creature/genome improvements come in a **later phase**. Don't divert into balance
  tuning mid-visual-task unless explicitly asked.
- Still keep the tree green (`cargo build` + `cargo test`), still write honest commits.
- This overrides the "verify population stays stable before committing" guidance below for as long as the
  visuals phase is active.

## Commit & push policy (this project)

**Standing permission: commit and push whenever you judge it's a good time** — you do not need to ask first.
This overrides the parent workspace's per-commit-approval rule for this repo. Use judgment:

- Commit at coherent stopping points (a feature/fix done + verified), not mid-broken-state.
- Before committing, the tree must be green: `cargo build` clean, `cargo test` passing, and a headless smoke
  run (`cargo run -- --headless --gens=1`) OK.
- Keep verification runs SHORT. The sim got heavier (grass, climate, bathymetry) and the worktree is often
  shared, so several `evolvarium` processes contend for cores -> long runs crawl. Default to `--gens=1` for a
  smoke and `--gens=3` to `--gens=5` for a quick balance sanity check; only reach for `--gens=15+` when a
  change is genuinely balance-critical and a short run can't show the trend. Headless logs are block-buffered
  to a pipe (flush at exit), so prefer a short run that finishes over tailing a long one.
- Write honest commit messages (end body with the standard Co-Authored-By trailer).
- Push to `origin main`; also mirror to the backup branch when convenient: `git push origin main:build`.
- Note: this worktree may be shared with another agent at times — if files outside your change are
  mid-refactor and don't build/test, hold the commit until the tree is green again.

## Run / inspect

```bash
cargo run                                   # planet visualizer (auto-loads evolved-continuous.json)
cargo run -- --headless --gens=N            # no window, fast-forward, per-gen stats, exits
cargo run -- --headless --gens=N --save=run.json   # evolve then save best-healthy snapshot
cargo run -- --load=run.json                # resume a saved population
```

### GPU capture tool (`--capture`) — primary way to verify rendering offline

Renders the REAL Bevy scene (true directional light + shadows + ambient) from a chosen vantage, writes a
PNG, exits. Needs a GPU + display. Read the PNG back to inspect. Flags:

```bash
cargo run -- --capture=PREFIX               # walk view at the homeland (morning sun)
  --cap-when=morning|noon|dusk|night        # sun phase
  --cap-off=N                               # raw sun-tick offset (overrides --cap-when), dial sun angle
  --cap-pitch=F                             # camera pitch (negative = look down)
  --cap-yaw=F                               # walk heading
  --cap-orbit                               # capture from orbit (space) instead of walk (surface)
  --cap-dist=F                              # orbit distance from planet center (test zoom; 95..420)
  --cap-lat=DEG                             # top-down orbit view at latitude DEG (+90 = north pole, -90 = south); implies orbit, pair w/ --cap-dist
  --cap-water                               # stand submerged in deep ocean (verify swim + blue tint)
```

`--shots[=PREFIX]` is a separate CPU ray-traced snapshot (no GPU) for offline planet views.

**NEVER auto-start the long-running visualizer** (`cargo run` with a window) to "watch" it — use `--capture`
or `--headless` so it exits on its own.

## Module map (`src/`)

- `main.rs` — app wiring, scene setup (globe, ocean shell, sun light + cascade, moon, sun disc, stars), CLI.
- `sphere.rs` — the spherical world: terrain/ocean/temperature/moisture noise fields, sun/moon, clouds,
  cloud-driven rain. Pure functions shared by sim + render + snapshot.
- `sim.rs` — the simulation: weather, fire, life/predation/plant/rot steps, generation step.
- `camera.rs` — orbit + walk cameras; per-mode shadow config (cascade, filtering, planet caster), swim.
- `viz.rs` — render-only visuals: creature/plant/tree meshes, clouds, rain streaks, day/night lighting,
  underwater tint, HUD, legend, god-controls.
- `capture.rs` — the `--capture` GPU screenshot tool.
- `genome.rs` / `components.rs` / `plant.rs` — genome, ECS components, plant model.
- `terrain.rs` — globe render mesh. `snapshot.rs` — CPU `--shots` renderer. `config.rs` — balance constants.

## Design docs & specs (outside the repo)

The full design specs + roadmap live in **`~/Documents/Github/clients/evolvarium/`** (a plain docs folder,
NOT a git repo — edit the files directly; nothing to commit there). Read the relevant doc before nontrivial
design work:

- `00-concept.md` … `13-living-food-and-distribution.md` — numbered design specs (concept, architecture,
  genome encoding, brain/NN, metabolism + nutrients, environment fields, god controls, roadmap, open
  questions, environment trade-offs, crate stack, diet/growth/disease, living food).
- `14-tuning-harness.md` — **complete build blueprint for the agent tuning harness** (a `--scenario` JSON
  runner + a Workflow agent fan-out that tunes cohorts to survival). Planned, NOT built; intended to be
  built by a separate agent. Full schemas, CLI contract, reflex presets, and the list of private symbols
  to expose are in there.
- `tuning-frictions.md` — running log of balance frictions for the harness to dial in (F1 = nutrient
  master-expression gradient too soft; F2 = bite pegs ~1.0). The harness's first job is F1.
- `PITCH.md`, `SESSION-STATUS.md` — friend-facing pitch + a resume/handoff note.

`BACKLOG.md` (in this repo) is the source of truth for what's done vs open; the spec folder is the "why/how".

## Conventions

- Comments follow the existing verbose-prose style in this repo (match surrounding code).
- No em/en dashes in generated text; use commas/periods/colons or "and".
- Balance-affecting sim changes (rain, mortality, reproduction) are sensitive: verify headless population
  stays stable (~70-90 carrying capacity) before committing. Genome/NN-architecture changes invalidate saved
  seeds — gate or regenerate them.
- `BACKLOG.md` tracks roadmap + done items; update it when landing notable work.
```
