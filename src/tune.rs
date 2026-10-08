//! Runtime balance knobs: `--set=NAME=VALUE` (or `--set NAME=VALUE`) overrides a default without a rebuild, so an agent can A/B a
//! value with tools/ab.sh across seeds. Knobs live in config.rs as `static Knob` (read with `.get()`), so a
//! use site that forgets `.get()` fails to compile instead of silently ignoring an override.
//! Only knobs nothing else is DERIVED from at compile time belong here (EAT_GAIN feeds COVER_ENERGY_PER_KG, so
//! overriding it would desync the two).
use std::sync::atomic::{AtomicU32, Ordering};

pub struct Knob {
    pub name: &'static str,
    pub default: f32,
    bits: AtomicU32,
}

impl Knob {
    pub const fn new(name: &'static str, v: f32) -> Self {
        Knob { name, default: v, bits: AtomicU32::new(v.to_bits()) }
    }
    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Relaxed))
    }
}

use crate::config as c;
pub static ALL: &[&Knob] = &[
    &c::MUT_RATE, &c::MUT_STD, &c::REPRO_THRESHOLD, &c::REPRO_COST, &c::BIRTH_ENERGY, &c::BASAL_COST, &c::CARPET_GRAZE,
    &c::ATTACK_COST, &c::SIZE_COMBAT, &c::ARMOR_DEF, &c::BRACE_DEF, &c::DISEASE_K, &c::AGE_HAZARD, &c::THREAT_MARGIN,
];

/// Apply one `NAME=VALUE`. Err names every knob so a typo is caught, not ignored.
pub fn set(spec: &str) -> Result<(), String> {
    let (name, val) = spec.split_once('=').ok_or_else(|| format!("--set wants NAME=VALUE, got {spec}"))?;
    let v: f32 = val.trim().parse().ok().filter(|v: &f32| v.is_finite()).ok_or_else(|| format!("--set {name}: {val} is not a finite number"))?;
    let k = ALL.iter().find(|k| k.name == name.trim()).ok_or_else(|| {
        format!("--set: unknown knob {name}; knobs: {}", ALL.iter().map(|k| k.name).collect::<Vec<_>>().join(" "))
    })?;
    k.bits.store(v.to_bits(), Ordering::Relaxed);
    Ok(())
}

/// Documented relationships an override can quietly break (config.rs comments). Warnings, not errors: an
/// experiment may break one on purpose, but it should say so in its log.
pub fn invariant_warnings() -> Vec<String> {
    let mut w = Vec::new();
    if c::BRACE_DEF.get() >= c::ARMOR_DEF.get() {
        w.push(format!("BRACE_DEF {} >= ARMOR_DEF {}: config says brace must stay below armour", c::BRACE_DEF.get(), c::ARMOR_DEF.get()));
    }
    if c::REPRO_THRESHOLD.get() >= c::GRAZE_FULL {
        w.push(format!("REPRO_THRESHOLD {} >= GRAZE_FULL {}: grazers can no longer top up to a breeding surplus", c::REPRO_THRESHOLD.get(), c::GRAZE_FULL));
    }
    w
}

/// Knobs that differ from their default, for logs and --metrics (an A/B result must say what it ran).
pub fn overrides() -> Vec<(&'static str, f32)> {
    ALL.iter().filter(|k| k.get().to_bits() != k.default.to_bits()).map(|k| (k.name, k.get())).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn set_rejects_unknown_and_applies_known() {
        assert!(super::set("NOPE=1").is_err());
        assert!(super::set("DISEASE_K=x").is_err());
        assert!(super::set("DISEASE_K=nan").is_err());
        assert!(super::set("DISEASE_K=inf").is_err());
        // knobs are process-global and tests run in parallel: only ever re-set a knob to its own default here
        let k = &crate::config::DISEASE_K;
        super::set(&format!("DISEASE_K={}", k.default)).unwrap();
        assert_eq!(k.get(), k.default);
        assert!(super::overrides().iter().all(|(n, _)| *n != "DISEASE_K"));
    }
}
