// Save/load survivor population so a good run can be stopped + resumed (BACKLOG P2).
// Snapshot = fitness-ranked creature genomes + current plant genomes/mass + generation, as JSON.
// Positions are NOT saved (re-randomized on load); only the evolved genes that matter for resume.
use crate::genome::Genome;
use crate::plant::PlantGenome;
use serde::{Deserialize, Serialize};

// A plant's heritable genes + current mass (so a grown food web reloads mid-life, not all seedlings).
#[derive(Serialize, Deserialize)]
pub struct SavedPlant {
    pub g: PlantGenome,
    pub mass: f32,
}

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub generation: u32,          // gen the survivors were saved at (informational)
    pub creatures: Vec<Genome>,   // fitness-ranked, best first
    pub plants: Vec<SavedPlant>,
}

// Write snapshot as pretty JSON. Logs on failure; never panics mid-run.
pub fn save_snapshot(path: &str, snap: &Snapshot) {
    match serde_json::to_string_pretty(snap) {
        Ok(s) => match std::fs::write(path, s) {
            Ok(()) => bevy::log::info!(
                "saved {} creatures + {} plants -> {}",
                snap.creatures.len(),
                snap.plants.len(),
                path
            ),
            Err(e) => bevy::log::error!("save write failed ({}): {}", path, e),
        },
        Err(e) => bevy::log::error!("save serialize failed: {}", e),
    }
}

// Load snapshot. Returns None (+logs) on missing/corrupt file so caller falls back to random spawn.
pub fn load_snapshot(path: &str) -> Option<Snapshot> {
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(snap) => Some(snap),
            Err(e) => {
                bevy::log::error!("load parse failed ({}): {}", path, e);
                None
            }
        },
        Err(e) => {
            bevy::log::error!("load read failed ({}): {}", path, e);
            None
        }
    }
}
