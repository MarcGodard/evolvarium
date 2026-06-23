//! Per-niche population floors + hall-of-fame banks + rescue seeding (the "keep every habitat alive" loop).
//!
//! Continuous co-evolution lets specialist niches (aquatic/aerial/highland/cold) quietly go extinct: the old
//! GLOBAL reseed floor refilled from whatever survivor dominated (land), so a collapsed niche never came back.
//! Here each niche has its OWN floor + a bank of its best-ever genomes. A niche below floor revives from ITS
//! bank (evolved competence preserved) -> mutate -> spawn in matching habitat. Banks persist -> even total
//! extinction recovers. Forced-random fallback only if a niche never banked anything.
//!
//! Self-sustain: track ticks since ANY niche last needed rescue. No rescue for NICHE_SUSTAIN_WINDOW -> ecosystem
//! holds itself up -> (with --until-sustain) save best snapshot + exit. Still rescuing at run end = that niche's
//! balance is broken (the signal for the auto-balance loop).

use crate::components::{Creature, DietState, Fitness};
use crate::config::*;
use crate::genome::Genome;
use crate::rng::Rng;
use bevy::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Niche {
    Aquatic,
    Aerial,
    Highland,
    Cold,
    Warm,
    Land,
}
pub const NICHE_COUNT: usize = 6;
pub const NICHE_NAMES: [&str; NICHE_COUNT] = ["aquatic", "aerial", "highland", "cold", "warm", "land"];

impl Niche {
    pub fn idx(self) -> usize {
        match self {
            Niche::Aquatic => 0,
            Niche::Aerial => 1,
            Niche::Highland => 2,
            Niche::Cold => 3,
            Niche::Warm => 4,
            Niche::Land => 5,
        }
    }
    fn from_idx(i: usize) -> Niche {
        [Niche::Aquatic, Niche::Aerial, Niche::Highland, Niche::Cold, Niche::Warm, Niche::Land][i]
    }
}

// Classify a genome into ONE niche for floor-counting. Order = specificity: hard specialist habitats
// (aquatic/aerial/highland) win over the latitudinal cold/warm split; generalists fall to Land. Mirrors the
// gene logic in sim::loaded_creature_pos so a rescued genome's niche matches where it gets placed.
pub fn niche_of(g: &Genome) -> Niche {
    if g.swim > 0.6 {
        Niche::Aquatic
    } else if g.flight > FLIGHT_KNEE {
        Niche::Aerial
    } else if g.alpine > 0.5 {
        Niche::Highland
    } else if g.temp_pref < 0.33 {
        Niche::Cold
    } else if g.temp_pref > 0.67 {
        Niche::Warm
    } else {
        Niche::Land
    }
}

// Force a random genome onto a niche (fallback when that niche's bank is empty: never thrived, no evolved
// stock to draw from). Brain stays random -> low competence, likely needs repeated rescue = balance signal.
fn force_niche(g: &mut Genome, n: Niche) {
    match n {
        Niche::Aquatic => {
            g.swim = 0.8;
            g.flight = 0.0;
        }
        Niche::Aerial => {
            g.flight = 0.7;
            g.swim = 0.0;
        }
        Niche::Highland => g.alpine = 0.7,
        Niche::Cold => g.temp_pref = 0.15,
        Niche::Warm => g.temp_pref = 0.85,
        Niche::Land => {}
    }
}

// Per-niche best-genome banks (hall of fame). Rebuilt from the fittest living members of each niche; an empty
// niche keeps its last-known-good bank so a crash is recoverable.
#[derive(Resource, Default)]
pub struct NicheBanks {
    pub banks: [Vec<Genome>; NICHE_COUNT],
}

// Rescue bookkeeping + self-sustain clock.
#[derive(Resource, Default)]
pub struct NicheTracker {
    pub started: bool,           // armed on first continuous tick (window counts from continuous start)
    pub last_rescue_tick: u32,   // ANY-niche last rescue; self-sustain = now - this >= NICHE_SUSTAIN_WINDOW
    pub per_last_rescue: [u32; NICHE_COUNT], // per-niche cooldown clock
    pub total_rescues: [u32; NICHE_COUNT],   // lifetime rescue count per niche (balance report)
    pub counts: [u32; NICHE_COUNT],          // last live count per niche
    pub next_bank_tick: u32,                 // next tick to rebuild banks
}

impl NicheTracker {
    // Self-sustaining = continuous run going, armed, and no rescue fired for a full window.
    pub fn self_sustaining(&self, tick: u32) -> bool {
        self.started && tick.saturating_sub(self.last_rescue_tick) >= NICHE_SUSTAIN_WINDOW
    }
}

// Per-tick: count niches, periodically refresh banks, rescue any niche below floor (cooldown-gated), track the
// self-sustain clock. Runs after live/predation/rot so births+deaths have settled; before generation_step
// (which reads the tracker for the --until-sustain stop). Scenario mode disabled (isolated cohort).
pub fn niche_step(
    gen: Res<crate::sim::GenState>,
    mut rng: ResMut<Rng>,
    mut commands: Commands,
    mut banks: ResMut<NicheBanks>,
    mut tr: ResMut<NicheTracker>,
    q: Query<(&Genome, &Fitness, &DietState), With<Creature>>,
    scen: Option<Res<crate::scenario::ScenarioStats>>,
) {
    let live_continuous = gen.continuous && gen.generation >= WARMUP_GENS;
    if scen.is_some() || !live_continuous {
        return;
    }
    let tick = gen.tick;
    if !tr.started {
        tr.started = true;
        tr.last_rescue_tick = tick; // window counts from continuous start, not tick 0
    }

    // count live per niche
    let mut counts = [0u32; NICHE_COUNT];
    for (g, _, _) in &q {
        counts[niche_of(g).idx()] += 1;
    }
    tr.counts = counts;

    // refresh banks: rebuild each niche from its fittest living members (tiebreak older = proven). Empty niche
    // keeps its existing bank (last-known-good -> revivable).
    if tick >= tr.next_bank_tick {
        tr.next_bank_tick = tick + NICHE_BANK_TICKS;
        let mut pools: [Vec<(f32, f32, Genome)>; NICHE_COUNT] = Default::default();
        for (g, fit, diet) in &q {
            pools[niche_of(g).idx()].push((fit.0, diet.age as f32, g.clone()));
        }
        for (i, pool) in pools.iter_mut().enumerate() {
            if pool.is_empty() {
                continue; // keep last-known-good bank
            }
            pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(b.1.total_cmp(&a.1)));
            banks.banks[i] = pool.iter().take(NICHE_BANK_CAP).map(|(_, _, g)| g.clone()).collect();
        }
    }

    // rescue: any niche below floor + cooldown elapsed -> seed a small batch from its bank (or forced-random).
    let mut rescued_any = false;
    for i in 0..NICHE_COUNT {
        if counts[i] >= NICHE_FLOOR as u32 {
            continue;
        }
        if tick.saturating_sub(tr.per_last_rescue[i]) < NICHE_RESCUE_COOLDOWN && tr.per_last_rescue[i] != 0 {
            continue; // eased in: cooldown spaces rescues ("every once in a while")
        }
        let need = (NICHE_FLOOR as u32 - counts[i]).min(NICHE_RESCUE_BATCH) as usize;
        let niche = Niche::from_idx(i);
        for _ in 0..need {
            let mut child = if banks.banks[i].is_empty() {
                let mut g = Genome::random(&mut rng);
                force_niche(&mut g, niche);
                g
            } else {
                let k = (rng.f32() * banks.banks[i].len() as f32) as usize % banks.banks[i].len();
                banks.banks[i][k].clone()
            };
            child.mutate(&mut rng, MUT_RATE, MUT_STD);
            let pos = crate::sim::loaded_creature_pos(&child, &mut rng); // habitat-matched placement
            crate::sim::spawn_creature(&mut commands, child, pos, &mut rng, BIRTH_ENERGY);
        }
        tr.per_last_rescue[i] = tick;
        tr.total_rescues[i] += need as u32;
        rescued_any = true;
        info!("niche rescue: {} +{} (pop {} < floor {})", NICHE_NAMES[i], need, counts[i], NICHE_FLOOR);
    }
    if rescued_any {
        tr.last_rescue_tick = tick; // resets the self-sustain clock
    }
}
