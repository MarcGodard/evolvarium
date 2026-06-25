// Deterministic Latin binomial for a creature, from its genome. Used by the on-screen name labels (viz N
// toggle) + the click inspector so the player can identify + find creature kinds. Genus = niche x diet (the
// genus alone says flier/swimmer/etc.); species epithet = a meaningful trait + a hash filler for variety.
// Pure: same genome -> same name (a lineage keeps its name); similar lineages share, distinct ones differ.
use crate::genome::Genome;

// Genus by [niche][diet]; niche 0=flying 1=aquatic 2=montane 3=land, diet 0=herbivore 1=omnivore 2=carnivore.
const GENUS: [[&str; 3]; 4] = [
    ["Volans", "Volucris", "Aquilo"],    // flying
    ["Natans", "Nereus", "Squalus"],     // aquatic
    ["Montanus", "Rupes", "Apex"],       // montane
    ["Pascuus", "Ambulator", "Venator"], // land
];

// Hash-picked filler epithets when no single trait dominates (keeps names varied + pronounceable).
const FILLER: [&str; 8] =
    ["viridis", "gracilis", "rapidus", "communis", "silvaticus", "pallidus", "robustus", "vagans"];

pub fn latin_name(g: &Genome) -> String {
    // niche priority mirrors the inspector + niche_of: aquatic, then flying, then montane, then land.
    let niche = if g.swim > 0.6 {
        1
    } else if g.flight >= crate::config::FLIGHT_KNEE {
        0
    } else if g.alpine > 0.5 {
        2
    } else {
        3
    };
    let diet = if g.carnivory > 0.6 {
        2
    } else if g.carnivory < 0.3 {
        0
    } else {
        1
    };
    let genus = GENUS[niche][diet];

    // species epithet: first dominant trait wins (meaningful), else a stable hash filler.
    let epithet = if g.armor > 0.55 {
        "loricatus" // armored
    } else if g.size > 0.62 {
        "magnus" // large
    } else if g.size < 0.28 {
        "minor" // small
    } else if g.temp_pref < 0.33 {
        "borealis" // cold-adapted
    } else if g.temp_pref > 0.67 {
        "australis" // warm-adapted
    } else if g.pelt > 0.5 {
        "villosus" // furred
    } else {
        // FNV-1a over stable genome floats -> a deterministic filler so siblings match, strangers differ
        let mut h: u32 = 2166136261;
        for v in [g.size, g.temp_pref, g.bite, g.metab, g.carnivory, g.swim, g.skin_hue] {
            h ^= (v * 9973.0) as i32 as u32;
            h = h.wrapping_mul(16777619);
        }
        FILLER[(h as usize) % FILLER.len()]
    };
    format!("{genus} {epithet}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn deterministic_and_niche_sensitive() {
        let mut rng = Rng::seed(1);
        let g = Genome::random(&mut rng);
        assert_eq!(latin_name(&g), latin_name(&g), "same genome -> same name");

        // a flier and a swimmer get different genera (the genus reveals the niche)
        let mut flier = Genome::random(&mut rng);
        flier.flight = 0.8;
        flier.swim = 0.0;
        let mut swimmer = Genome::random(&mut rng);
        swimmer.swim = 0.9;
        let (fg, sg) = (latin_name(&flier), latin_name(&swimmer));
        assert!(fg.starts_with("Volans") || fg.starts_with("Volucris") || fg.starts_with("Aquilo"), "flier genus, got {fg}");
        assert!(sg.starts_with("Natans") || sg.starts_with("Nereus") || sg.starts_with("Squalus"), "swimmer genus, got {sg}");
    }

    #[test]
    fn epithet_encodes_size() {
        let mut rng = Rng::seed(2);
        let mut big = Genome::random(&mut rng);
        big.size = 0.9;
        big.armor = 0.0;
        assert!(latin_name(&big).ends_with("magnus"), "large -> magnus, got {}", latin_name(&big));
    }
}
