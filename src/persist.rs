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

// --- plant seed-bank library (tuning harness, see 14) ---
// A reusable bank of tuned plant/tree genomes, one entry per saved winner. The tuning harness writes it;
// a normal `cargo run` seeds the planet from it (biome-matched). GENE-AGNOSTIC: each entry stores a whole
// PlantGenome, which `#[serde(default)]`s any newly-added gene -> a library written before a gene existed
// still loads after it lands (missing gene fills its default, then re-tuning evolves it). Accumulates
// across runs (merge_in), so repeated harness runs grow + improve the bank without clobbering prior work.

fn lib_version() -> u32 {
    1
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LibEntry {
    pub niche: String, // which tuned niche produced this (tropical-wet, deep-kelp, ...). Informational.
    #[serde(default)]
    pub tree: bool, // seed it as a Tree (tall/slow) vs a ground plant
    #[serde(default)]
    pub score: f32, // health_score the harness gave it (higher = healthier). Drives per-niche keep-best.
    pub genome: PlantGenome,
}

#[derive(Serialize, Deserialize, Default)]
pub struct PlantLibrary {
    #[serde(default = "lib_version")]
    pub version: u32,
    pub entries: Vec<LibEntry>,
}

pub fn save_plant_library(path: &str, lib: &PlantLibrary) {
    match serde_json::to_string_pretty(lib) {
        Ok(s) => match std::fs::write(path, s) {
            Ok(()) => bevy::log::info!("saved {} library entries -> {}", lib.entries.len(), path),
            Err(e) => bevy::log::error!("library write failed ({}): {}", path, e),
        },
        Err(e) => bevy::log::error!("library serialize failed: {}", e),
    }
}

// Load a plant library. Returns None (+logs) on missing/corrupt OR empty file -> caller falls back to
// archetype seeding so no biome ever goes bare.
pub fn load_plant_library(path: &str) -> Option<PlantLibrary> {
    let s = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<PlantLibrary>(&s) {
        Ok(lib) if !lib.entries.is_empty() => Some(lib),
        Ok(_) => None, // empty library -> fall back to archetypes
        Err(e) => {
            bevy::log::error!("library parse failed ({}): {}", path, e);
            None
        }
    }
}

impl PlantLibrary {
    // Score how well a stored genome fits a surface site (temperature + moisture niche + water gating).
    // Higher = better fit. Returns None when the entry is categorically wrong for the site (land plant in
    // deep water, aquatic on dry land, tree in ocean) so it is never drawn there.
    fn site_fit(g: &PlantGenome, d: bevy::math::Vec3, is_tree: bool) -> Option<f32> {
        let ocean = crate::sphere::is_ocean(d);
        if is_tree && ocean {
            return None; // trees are land-only
        }
        let aquatic = g.wet > 0.7;
        // land plant can't live submerged; aquatic plant can't carpet dry land
        if ocean && !aquatic {
            return None;
        }
        if !ocean && aquatic {
            return None;
        }
        let t = crate::sphere::base_temperature(d);
        let m = crate::sphere::moisture(d);
        // closeness on temperature niche + moisture niche (1 = perfect, 0 = opposite)
        let temp_fit = 1.0 - (t - g.temp_pref).abs();
        let moist_fit = 1.0 - (m - g.wet).abs();
        Some(temp_fit + moist_fit)
    }

    // Pick a biome-matched genome for a site. Scores every entry of the requested kind (plant vs tree),
    // keeps the top few fits, and samples one at random so the same biome gets varied draws (not a monoculture).
    // Returns a clone the caller can mutate + spawn. None -> no decent match (caller falls back to archetypes).
    pub fn pick_for_site(&self, rng: &mut crate::rng::Rng, d: bevy::math::Vec3, want_tree: bool) -> Option<PlantGenome> {
        let mut scored: Vec<(f32, &PlantGenome)> = self
            .entries
            .iter()
            .filter(|e| e.tree == want_tree)
            .filter_map(|e| Self::site_fit(&e.genome, d, want_tree).map(|f| (f, &e.genome)))
            .collect();
        if scored.is_empty() {
            return None;
        }
        // best fits first; sample among the top handful so a biome draws a varied mix, not one winner
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let top = scored.len().min(4);
        let i = (rng.f32() * top as f32) as usize % top;
        Some(scored[i].1.clone())
    }

    // Merge winners into the library: append, then keep only the best `per_niche_cap` entries per niche
    // (by score), dropping near-duplicate genomes. Accumulates across harness runs without clobbering.
    pub fn merge_in(&mut self, new_entries: Vec<LibEntry>, per_niche_cap: usize) {
        self.entries.extend(new_entries);
        // group by niche, keep top-N by score, drop near-identical genomes within a niche
        use std::collections::HashMap;
        let mut by_niche: HashMap<String, Vec<LibEntry>> = HashMap::new();
        for e in self.entries.drain(..) {
            by_niche.entry(e.niche.clone()).or_default().push(e);
        }
        let mut out: Vec<LibEntry> = Vec::new();
        for (_niche, mut group) in by_niche {
            group.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            let mut kept: Vec<LibEntry> = Vec::new();
            for e in group {
                if kept.len() >= per_niche_cap {
                    break;
                }
                if kept.iter().any(|k| genome_close(&k.genome, &e.genome)) {
                    continue; // near-duplicate of an already-kept (higher-scoring) genome
                }
                kept.push(e);
            }
            out.extend(kept);
        }
        self.entries = out;
    }
}

// Two plant genomes count as near-identical when their defining niche genes are all within a small epsilon.
// GENE-AGNOSTIC duplicate test over the serialized numeric fields, so it folds in genes added later.
fn genome_close(a: &PlantGenome, b: &PlantGenome) -> bool {
    let (va, vb) = (serde_json::to_value(a), serde_json::to_value(b));
    if let (Ok(serde_json::Value::Object(ma)), Ok(serde_json::Value::Object(mb))) = (va, vb) {
        for (k, av) in &ma {
            if let (Some(x), Some(y)) = (av.as_f64(), mb.get(k).and_then(|v| v.as_f64())) {
                if (x - y).abs() > 0.08 {
                    return false; // a meaningfully different gene -> not a duplicate
                }
            }
        }
        return true;
    }
    false
}
