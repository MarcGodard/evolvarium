// ECS components for the M1 foraging sim.
use bevy::prelude::*;

#[derive(Component)]
pub struct Creature;

#[derive(Component)]
pub struct Food;

// Food type 0 or 1. In --poison mode one type is nutritious, the other toxic, and which is
// which flips each generation (unknowable at birth -> learners must taste + adapt). See 05.
#[derive(Component)]
pub struct FoodKind(pub u8);

// Carrion marker + rot clock. A dead creature drops carrion: fresh = rich meat (no defense), but as
// `age` climbs its nutrition fades and toxin rises -> rotten meat poisons the eater (BACKLOG P3).
// Despawns once fully decomposed. Distinguishes carrion from living plants (which carry no Rot).
#[derive(Component)]
pub struct Rot {
    pub age: u32,
}

// Current energy store. Burns each tick; refilled by eating; death at <=0.
#[derive(Component)]
pub struct Energy(pub f32);

// Food eaten this generation = selection fitness.
#[derive(Component)]
pub struct Fitness(pub f32);

// Facing angle (radians) in the x-z plane. NN turn output rotates it.
#[derive(Component)]
pub struct Heading(pub f32);

// Starved creatures stop acting but keep their fitness until generation end.
#[derive(Component)]
pub struct Alive(pub bool);

// Per-life working brain: weights start as a copy of the genome's w0, then learn during life
// (reward-gated Hebbian). Discarded at death; NOT inherited. The GA selects genomes that
// LEARN well, not the learned weights themselves (Baldwin effect, see 04).
use crate::genome::Net;
#[derive(Component)]
pub struct Brain {
    pub net: Net, // working copy of genome weights, tuned by lifetime learning
    pub prev_dist: f32, // distance to nearest food last tick, for approach-reward shaping
}

// Per-life locomotion diagnostic: where the creature was born + total path walked. Lets us measure
// roaming vs circling: net displacement / path ratio ~1 = straight rover, ~0 = spinning in place.
#[derive(Component)]
pub struct Locomotion {
    pub start: Vec3,
    pub path: f32,
}

// Per-life epigenetic diet state (see 12). expr[t] = current expression of digestion genes for
// food type t = efficiency on it. Eating a type ramps its expr, unused types decay (use-it-or-lose-it).
// g = accumulated growth-signaling load (chronic mismatch -> disease). age in ticks (aging hazard).
use crate::genome::NFOOD;
#[derive(Component)]
pub struct DietState {
    pub expr: [f32; NFOOD],
    pub g: f32,
    pub age: u32,
}
