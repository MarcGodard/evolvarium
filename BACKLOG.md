# Build-loop backlog

The build loop works ONLY in this worktree (`evolvarium-build`, branch `build`) and commits here.
The tuning loop works ONLY in the main tree (`evolvarium`, uncommitted) and owns BALANCE CONSTANTS
(plant defense cost, BITE_K, BITE_COST, EAT_GAIN, caps). To avoid overlap, the build loop must NOT
edit those tuning constants — it adds FEATURES. One item per fire: implement, keep compile-green,
verify headless, commit on `build` with a clear message, then tick the item here.

## P1 — behavioral / energy economy (fixes the "creatures just circle" problem)
- [x] **Movement-range diagnostic.** Track per-creature net displacement / area covered over life;
      log avg in generation_step. So we can SEE whether creatures roam vs circle. (No fitness change
      yet — just observability first.)
- [ ] **Rest vs move economy.** Make resting genuinely valuable: low/zero cost when nearly still,
      rising cost with speed (replace flat MOVE_COST*thrust with a steeper curve), and a small basal
      that rewards stillness. Aim: aimless circling becomes costly, purposeful move+rest emerges.
- [ ] **Overeating penalty.** Cap energy; eating at/near max converts the excess into growth-load G
      (harm), so gorging shortens life. Pressure to eat the BEST food in moderation, not constantly.
- [ ] **Remove dead creatures visually.** On death, despawn (or hide) the creature mesh in render.

## P2 — persistence + food GA
- [ ] **Save / load survivors.** `--save <path>` writes the surviving population's genomes (+plants?)
      to disk (RON/JSON via serde); `--load <path>` resumes from it. So a good run can be stopped and
      continued without starting from scratch.
- [ ] **Per-food digestibility GA.** Make some foods intrinsically more GA-beneficial: e.g. a plant
      `quality` gene that boosts a creature's digestion efficiency, and/or a heritable creature
      per-food digestion gene beyond the lifetime `expr`. Clarify vs existing diet model first.

## P3 — environment (the "environment stuff") — bigger; needs the fields system (spec 06)
- [ ] **Elevation + climbing/falling.** Add terrain height; moving uphill costs more energy, downhill
      less (gradient energy loss). Gives a real 3D range of motion.
- [ ] **Rot chain.** Dead creatures become carrion (edible), then rot to POISON over time; dead plants
      → poison too. ("Make things rot.") Feeds the nutrient cycle (conservation, spec 05).
- [ ] **Environmental pressure on plants.** Rain/moisture field: too much or too little kills plants
      (and they rot to poison). Needs weather/field machinery (spec 06).

## NOT for the build loop (owned elsewhere)
- Plant defense cost / arms-race balance constants → the TUNING loop owns these.
- Genome/learning architecture changes → discuss with the human first.
