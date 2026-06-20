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
- [x] **Rest vs move economy.** Make resting genuinely valuable: low/zero cost when nearly still,
      rising cost with speed (replace flat MOVE_COST*thrust with a steeper curve), and a small basal
      that rewards stillness. Aim: aimless circling becomes costly, purposeful move+rest emerges.
- [x] **Overeating penalty.** Cap energy; eating at/near max converts the excess into growth-load G
      (harm), so gorging shortens life. Pressure to eat the BEST food in moderation, not constantly.
- [x] **Remove dead creatures visually.** viz::hide_dead sets Visibility::Hidden when Alive(false),
      restores on rebirth at the gen boundary. Carrion (a Food entity) appears in the corpse's place.

## P2 — persistence + food GA
- [x] **Save / load survivors.** `--save=<path>` writes the fitness-ranked survivor genomes + current
      food web (plant genomes + mass) to JSON at headless run end; `--load=<path>` resumes from it
      (random spawn if missing/corrupt). Verified: resume opens at evolved fitness, not cold-start.
      Positions re-randomized (only genes persist). persist.rs + serde/serde_json.
- [x] **Per-food digestibility GA.** Plant `quality` gene (0..1) scales energy the eater extracts
      (factor 0.5..1.5, balance-neutral at 0.5). Trade-off (no free lunch): quality costs growth
      (-0.2 in growth_rate) AND, when eaten, the eater disperses a mutated offspring (endozoochory,
      chance = quality x SEED_VIA_GUT) -> tasty plants lose individuals but win dispersal. Result:
      quality evolves to an INTERIOR optimum (~0.3-0.5 across seeds), not pegged 0/1. Plant-side only;
      did NOT touch creature genome/learning. (0.2 growth-cost + 0.5 SEED_VIA_GUT are tunable.)

## P3 — environment (the "environment stuff") — bigger; needs the fields system (spec 06)
- [x] **Elevation + climbing/falling.** terrain.rs heightfield (sinusoidal hills, HEIGHT_MAX 6);
      creatures/plants ride the surface. Moving uphill burns CLIMB_COST*dh, downhill refunds
      DESCEND_REFUND*dh (< cost -> net dissipative, no free lunch). Render shows a heightmesh.
      Verified headless: viable across seeds; roam ratio ROSE ~0.2 -> ~0.45 (relief reduces circling).
      `elev` added to gen log. (Future: tie altitude to a benefit so high ground is worth the climb.)
- [x] **Rot chain.** Death drops carrion (Rot component) = fresh defense-free meat; over ROT_GONE ticks
      nutrition fades to 0 and toxin rises to TOXIN_MAX (eating rotten meat poisons you + adds
      growth-load), then it despawns. Dead PLANTS (killed by moisture, see below) likewise drop poison
      detritus (DETRITUS_NUTRIENT, reuses Rot). Both are Food entities sensed/eaten via the normal path
      (separate eat branch, excluded from plant cap/stats/dispersal). Emergent: a SCAVENGING niche
      (survive on carrion instead of the bite-vs-defense arms race).
- [x] **Environmental pressure on plants.** terrain::moisture(x,z,season): lowlands wet + spatial patches
      + a seasonal wet/dry drift across generations. Plants carry a `wet` preference gene; mismatch
      beyond MOISTURE_TOLERANCE can kill them (MOISTURE_KILL per-tick), and the corpse becomes poison
      detritus. Drives spatial niches ("food near water") + local adaptation via dispersal. Verified
      headless: populations stay viable (reseed floor holds); position-dependent early die-off observed.
      `wet` added to gen log. (TOLERANCE/KILL/SEASON_FREQ tunable.)

## M6 — reproduction modes + interactive viz
- [~] **Continuous reproduction** (`--continuous`, experimental; default stays generational). Warm-up
      generationally for WARMUP_GENS to evolve foragers, then switch to emergent birth/death: a well-fed
      creature (energy > REPRO_THRESHOLD) spends REPRO_COST to bud a mutated child; death despawns +
      drops carrion. Density-dependent (breed only when fed) + CREATURE_CAP bound it. STATUS: stable +
      sustained on some seeds (seed 1 base survives full run, cycling 46-130), collapses on others
      (seed 3) and in `--diet` (diet energy drains + sync warm-up cohort). NEEDS: robust cross-seed
      balance (softer warm-up transition, gentler diet economy) before it can be the default.
- [x] **Click-to-inspect + follow.** Left-click selects a creature/plant -> live stats panel + yellow
      ring; F toggles a follow-cam that tracks the selected entity. Camera look moved to RIGHT-click.

## NOT for the build loop (owned elsewhere)
- Plant defense cost / arms-race balance constants → the TUNING loop owns these.
- Genome/learning architecture changes → discuss with the human first.
