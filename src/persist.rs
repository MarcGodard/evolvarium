// Save/load survivor population so a good run stops + resumes (BACKLOG P2).
// Snapshot = fitness-ranked creature genomes + current plant genomes/mass + generation, as JSON.
// Positions NOT saved (re-randomized on load); only evolved genes that matter for resume.
// NOTE: genome/NN-architecture changes invalidate saved snapshots (Genome serde shape shifts).
use crate::genome::Genome;
use crate::plant::PlantGenome;
use serde::{Deserialize, Serialize};

// Plant heritable genes + current mass (grown food web reloads mid-life, not all seedlings).
#[derive(Serialize, Deserialize)]
pub struct SavedPlant {
    pub g: PlantGenome,
    pub mass: f32,
}

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub generation: u32,          // gen survivors saved at (informational)
    pub creatures: Vec<Genome>,   // fitness-ranked, best first (LEGACY: genomes only, positions/state dropped)
    pub plants: Vec<SavedPlant>,  // LEGACY: plant genome + mass, regenerated biome-matched on load
    // Full top-to-bottom world (positions + brains-via-genome + dynamic field grids + seed bank + clock).
    // Present in new saves -> load RESTORES the exact world (wear/soil/fire/groundwater + every creature/plant
    // where it stood). Absent in old seeds -> load falls back to the lossy legacy path (scatter + fresh fields).
    #[serde(default)]
    pub world: Option<WorldState>,
}

// Dynamic per-cell field grids (SOIL_RES^2 each). Empty vec = field not saved (load keeps its fresh default).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Grids {
    #[serde(default)]
    pub soil: Vec<f32>,
    #[serde(default)]
    pub groundwater: Vec<f32>,
    #[serde(default)]
    pub climate: Vec<f32>,
    #[serde(default)]
    pub fire: Vec<f32>,
    #[serde(default)]
    pub wear: Vec<f32>,
}

// One creature, full live state. pos = surface_pos(dir, CREATURE_Y + alt). Working brain re-derives from
// genome on load (Baldwin: genome carries the learnable priors; per-life learned net is not persisted).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedCreature {
    pub g: Genome,
    pub dir: [f32; 3],   // unit surface direction
    pub alt: f32,        // vertical offset (flier height / swimmer water-column rise)
    pub heading: f32,
    pub energy: [f32; 3], // fast, sugar, fat
    pub fitness: f32,
    pub reserves: Vec<f32>, // diet nutrient reserves (NUTRIENTS)
    pub diet_g: f32,
    pub age: u32,
    pub fatigue: f32,
    pub starve: u16,
    pub toxic_load: f32,
}

// One plant-class entity (living plant, tree, carrion, ferment, fallen fruit). Grass + seaweed carpets are
// NOT saved (regenerated on load) to keep files small; the wear grid that bared them IS saved.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedPlantEntity {
    pub g: PlantGenome,
    pub dir: [f32; 3],
    pub mass: f32,
    pub age: u32,
    #[serde(default)]
    pub tree: Option<bool>, // Some(edible) -> Tree marker
    #[serde(default)]
    pub rot_age: Option<u32>, // Some -> Rot clock (carrion / ferment / fruit)
    #[serde(default)]
    pub ferment_toxic: Option<f32>, // Some -> Ferment marker
    #[serde(default)]
    pub seed: Option<PlantGenome>, // Some -> Seed (fruit carrying parent genome)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedSeed {
    pub g: PlantGenome,
    pub dir: [f32; 3],
    pub ticks: u32, // ticks until germination
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct WorldState {
    pub tick: u32,
    pub generation: u32,
    pub weather_rain: f32,
    pub grids: Grids,
    #[serde(default)]
    pub seed_bank: Vec<SavedSeed>,
    pub creatures: Vec<SavedCreature>,
    pub plants: Vec<SavedPlantEntity>,
}

// Write any serializable as pretty JSON. Logs on failure, never panics mid-run. `what` = success-log subject.
fn write_pretty<T: Serialize>(path: &str, val: &T, what: &str) {
    match serde_json::to_string_pretty(val) {
        Ok(s) => match std::fs::write(path, s) {
            Ok(()) => bevy::log::info!("saved {} -> {}", what, path),
            Err(e) => bevy::log::error!("{} write failed ({}): {}", what, path, e),
        },
        Err(e) => bevy::log::error!("{} serialize failed: {}", what, e),
    }
}

// Write snapshot as pretty JSON. Logs on failure, never panics mid-run.
pub fn save_snapshot(path: &str, snap: &Snapshot) {
    write_pretty(path, snap, &format!("{} creatures + {} plants", snap.creatures.len(), snap.plants.len()));
}

// Load snapshot. Returns None (+logs) on missing/corrupt file -> caller falls back to random spawn.
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
// Reusable bank of tuned plant/tree genomes, one entry per saved winner. Tuning harness writes it; normal
// `cargo run` seeds planet from it (biome-matched). GENE-AGNOSTIC: each entry stores whole PlantGenome,
// `#[serde(default)]`s any newly-added gene -> library written before a gene existed still loads after it
// lands (missing gene fills default, then re-tuning evolves it). Accumulates across runs (merge_in), so
// repeated harness runs grow + improve bank without clobbering prior work.

fn lib_version() -> u32 {
    1
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LibEntry {
    pub niche: String, // tuned niche that produced this (tropical-wet, deep-kelp, ...). Informational.
    #[serde(default)]
    pub tree: bool, // seed as Tree (tall/slow) vs ground plant
    #[serde(default)]
    pub score: f32, // health_score harness gave it (higher = healthier). Drives per-niche keep-best.
    pub genome: PlantGenome,
    // genes ABSENT from this entry's stored JSON (library written before gene existed). Computed at load,
    // never serialized. Seeding RANDOMIZES these per-plant (vs flat serde default) so planet gets variety in
    // newly-added genes while keeping tuned ones. See load_plant_library + pick_for_site.
    #[serde(skip, default)]
    pub missing: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct PlantLibrary {
    #[serde(default = "lib_version")]
    pub version: u32,
    pub entries: Vec<LibEntry>,
}

pub fn save_plant_library(path: &str, lib: &PlantLibrary) {
    write_pretty(path, lib, &format!("{} library entries", lib.entries.len()));
}

// Load plant library. Returns None (+logs) on missing/corrupt OR empty file -> caller falls back to
// archetype seeding so no biome goes bare. Computes each entry's `missing` genes (current struct fields
// absent from stored JSON = genes added since library written) so seeding randomizes them instead of flat
// serde default -> planet varies in newly-added genes.
pub fn load_plant_library(path: &str) -> Option<PlantLibrary> {
    let s = std::fs::read_to_string(path).ok()?;
    parse_plant_library(&s)
}

// Parse + compute per-entry `missing` genes. Split out so tests exercise it without a temp file.
fn parse_plant_library(s: &str) -> Option<PlantLibrary> {
    let mut lib: PlantLibrary = match serde_json::from_str(s) {
        Ok(l) => l,
        Err(e) => {
            bevy::log::error!("library parse failed: {}", e);
            return None;
        }
    };
    if lib.entries.is_empty() {
        return None; // empty library -> fall back to archetypes
    }
    // diff each stored genome's RAW keys against current struct's full field set -> absent = new genes.
    use std::collections::HashSet;
    let raw_entries = serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| v.get("entries").and_then(|e| e.as_array()).cloned());
    for (i, entry) in lib.entries.iter_mut().enumerate() {
        let current: HashSet<String> = serde_json::to_value(&entry.genome)
            .ok()
            .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
            .unwrap_or_default();
        let present: HashSet<String> = raw_entries
            .as_ref()
            .and_then(|a| a.get(i))
            .and_then(|e| e.get("genome"))
            .and_then(|g| g.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        entry.missing = current.difference(&present).cloned().collect();
    }
    Some(lib)
}

// Materialize a library genome for seeding: clone, then RANDOMIZE any `missing` (newly-added) gene from a
// fresh random genome so planet gets variety in new genes while keeping tuned ones. Gene-agnostic (works at
// serde-Value level), so a gene added later needs no change here.
fn materialize(g: &PlantGenome, missing: &[String], rng: &mut crate::rng::Rng) -> PlantGenome {
    if missing.is_empty() {
        return g.clone();
    }
    let (mut v, rand) = match (serde_json::to_value(g), serde_json::to_value(PlantGenome::random(rng, crate::genome::NFOOD as u8))) {
        (Ok(v), Ok(r)) => (v, r),
        _ => return g.clone(),
    };
    if let (serde_json::Value::Object(map), serde_json::Value::Object(rmap)) = (&mut v, &rand) {
        for k in missing {
            if let Some(rv) = rmap.get(k) {
                map.insert(k.clone(), rv.clone());
            }
        }
    }
    serde_json::from_value(v).unwrap_or_else(|_| g.clone())
}

impl PlantLibrary {
    // Score how well a stored genome fits a surface site (temperature + moisture niche + water gating).
    // Higher = better fit. Returns None when entry categorically wrong for site (land plant in deep water,
    // aquatic on dry land, tree in ocean) so it's never drawn there.
    fn site_fit(g: &PlantGenome, d: bevy::math::Vec3, is_tree: bool) -> Option<f32> {
        let ocean = crate::sphere::is_ocean(d);
        if is_tree && ocean {
            return None; // trees land-only
        }
        let aquatic = g.wet > 0.7; // wet > 0.7 = aquatic plant
        // land plant can't live submerged; aquatic plant can't carpet dry land
        if ocean && !aquatic {
            return None;
        }
        if !ocean && aquatic {
            return None;
        }
        let t = crate::sphere::base_temperature(d);
        let m = crate::sphere::moisture(d);
        // closeness on temperature niche + moisture niche (1 = perfect fit, 0 = opposite)
        let temp_fit = 1.0 - (t - g.temp_pref).abs();
        let moist_fit = 1.0 - (m - g.wet).abs();
        Some(temp_fit + moist_fit)
    }

    // Pick biome-matched genome for a site. Scores every entry of requested kind (plant vs tree), keeps top
    // few fits, samples one at random so same biome gets varied draws (not monoculture). Returns a clone
    // caller can mutate + spawn. None -> no decent match (caller falls back to archetypes).
    pub fn pick_for_site(&self, rng: &mut crate::rng::Rng, d: bevy::math::Vec3, want_tree: bool) -> Option<PlantGenome> {
        let mut scored: Vec<(f32, &LibEntry)> = self
            .entries
            .iter()
            .filter(|e| e.tree == want_tree)
            .filter_map(|e| Self::site_fit(&e.genome, d, want_tree).map(|f| (f, e)))
            .collect();
        if scored.is_empty() {
            return None;
        }
        // best fits first; sample among top handful so a biome draws varied mix, not one winner
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let top = scored.len().min(4);
        let i = (rng.f32() * top as f32) as usize % top;
        let e = scored[i].1;
        // randomize any genes added since this entry written (variety), keep tuned genes
        Some(materialize(&e.genome, &e.missing, rng))
    }

    // Merge winners into library: append, then keep only best `per_niche_cap` entries per niche (by score),
    // dropping near-duplicate genomes. Accumulates across harness runs without clobbering.
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
                    continue; // near-duplicate of already-kept (higher-scoring) genome
                }
                kept.push(e);
            }
            out.extend(kept);
        }
        self.entries = out;
    }
}

// Two plant genomes near-identical when all numeric genes within epsilon 0.08. GENE-AGNOSTIC duplicate test
// over serialized numeric fields, folds in genes added later.
fn genome_close(a: &PlantGenome, b: &PlantGenome) -> bool {
    let (va, vb) = (serde_json::to_value(a), serde_json::to_value(b));
    if let (Ok(serde_json::Value::Object(ma)), Ok(serde_json::Value::Object(mb))) = (va, vb) {
        for (k, av) in &ma {
            if let (Some(x), Some(y)) = (av.as_f64(), mb.get(k).and_then(|v| v.as_f64())) {
                if (x - y).abs() > 0.08 {
                    return false; // meaningfully different gene -> not a duplicate
                }
            }
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    // Gene added AFTER library written must be RANDOMIZED per-plant on seed (variety), while genes library
    // DID store stay put. Simulate new gene by dropping one from stored JSON.
    #[test]
    fn new_genes_randomized_keep_tuned() {
        let mut rng = Rng::seed(7);
        let g = PlantGenome::random(&mut rng, crate::genome::NFOOD as u8);
        let lib = PlantLibrary { version: 1, entries: vec![LibEntry { niche: "t".into(), tree: false, score: 1.0, genome: g.clone(), missing: vec![] }] };
        // serialize, then DROP `succulence` from stored genome to mimic it being added later.
        let mut v = serde_json::to_value(&lib).unwrap();
        v["entries"][0]["genome"].as_object_mut().unwrap().remove("succulence");
        let loaded = parse_plant_library(&serde_json::to_string(&v).unwrap()).unwrap();
        let e = &loaded.entries[0];
        assert!(e.missing.contains(&"succulence".to_string()), "dropped gene detected as missing");

        // materialize many times: succulence must VARY (randomized), wet (stored gene) must stay put.
        let vals: Vec<f32> = (0..24).map(|_| materialize(&e.genome, &e.missing, &mut rng).succulence).collect();
        assert!(vals.iter().any(|&x| (x - vals[0]).abs() > 1e-6), "new gene randomized (not a constant default)");
        assert!((materialize(&e.genome, &e.missing, &mut rng).wet - g.wet).abs() < 1e-6, "stored gene kept");
    }

    // Library with all current genes present has no `missing`, so materialize = pure clone.
    #[test]
    fn full_library_unchanged() {
        let mut rng = Rng::seed(1);
        let g = PlantGenome::random(&mut rng, crate::genome::NFOOD as u8);
        let lib = PlantLibrary { version: 1, entries: vec![LibEntry { niche: "t".into(), tree: false, score: 1.0, genome: g.clone(), missing: vec![] }] };
        let loaded = parse_plant_library(&serde_json::to_string(&lib).unwrap()).unwrap();
        assert!(loaded.entries[0].missing.is_empty(), "no missing genes when all present");
    }
}
