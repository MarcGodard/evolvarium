// ECS components for the M1 foraging sim.
use bevy::prelude::*;

#[derive(Component)]
pub struct Creature;

#[derive(Component)]
pub struct Food;

// Tree marker: a long-lived plant. Two kinds: edible=true is a tall fruit tree only TALL creatures
// (sensors >= TREE_REACH) can reach + eat; edible=false is an evergreen that NOTHING can eat (pure
// structure / refuge). Moisture-immune; grows large; reproduces slowly into its own kind. (BACKLOG trees)
#[derive(Component, Clone, Copy)]
pub struct Tree {
    pub edible: bool,
}

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
// fatigue = exertion debt 0..1: rises with movement, sheds while resting. High fatigue costs stress
// energy + saps movement output (effort still burns) -> resting is the only recovery -> day/night rest.
use crate::genome::NFOOD;
#[derive(Component)]
pub struct DietState {
    pub expr: [f32; NFOOD],
    pub g: f32,
    pub age: u32,
    pub fatigue: f32,
}
