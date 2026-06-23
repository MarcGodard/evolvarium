// Phase-0 profiler for the parallelization plan (PARALLELIZATION.md). Global, thread-safe, near-free when
// off. Each hot system opens a `scope("name")` at its top; guard records elapsed on drop. Works in headless,
// render, scenario chains alike (no per-system param threading). report() prints cumulative ranking.
//
// Why global static, not a Resource: Bevy may run chained systems on different worker threads, and a Resource
// param would have to be threaded into every step fn (+ exist in every app). A static Mutex sidesteps both.
// Cost when disabled: one Relaxed atomic load + early return (no alloc, no lock).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub static ENABLED: AtomicBool = AtomicBool::new(false);

// label -> (total_nanos, sample_count). BTreeMap::new() const since 1.66 so static init needs no lazy.
static DATA: Mutex<BTreeMap<&'static str, (u128, u64)>> = Mutex::new(BTreeMap::new());

pub struct Guard {
    label: &'static str,
    start: Instant,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let e = self.start.elapsed().as_nanos();
        let mut d = DATA.lock().unwrap();
        let ent = d.entry(self.label).or_insert((0, 0));
        ent.0 += e;
        ent.1 += 1;
    }
}

// Open timing scope. None when profiling off (cheap). Bind to `_g` so it lives to end of fn body.
#[inline]
pub fn scope(label: &'static str) -> Option<Guard> {
    if ENABLED.load(Ordering::Relaxed) {
        Some(Guard { label, start: Instant::now() })
    } else {
        None
    }
}

// Print cumulative per-system ranking: total ms, mean us/tick, % of measured tick. Sorted slowest first.
pub fn report(tick: u32) {
    let d = DATA.lock().unwrap();
    let total: u128 = d.values().map(|v| v.0).sum();
    if total == 0 {
        return;
    }
    let mut rows: Vec<(&'static str, u128, u64)> = d.iter().map(|(k, v)| (*k, v.0, v.1)).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    println!("--- profile @ tick {tick} (cumulative over measured ticks) ---");
    for (label, nanos, count) in &rows {
        let pct = *nanos as f64 / total as f64 * 100.0;
        let total_ms = *nanos as f64 / 1.0e6;
        let mean_us = if *count > 0 { *nanos as f64 / *count as f64 / 1.0e3 } else { 0.0 };
        println!("  {label:<12} {pct:5.1}%  {total_ms:9.1} ms total  {mean_us:8.1} us/tick  (n={count})");
    }
    // tick estimate = sum of per-system means (segments are serial in the chain).
    let tick_us: f64 = rows.iter().map(|r| r.1 as f64 / r.2.max(1) as f64 / 1e3).sum();
    println!("  measured tick mean: {tick_us:.1} us/tick  (~{:.0} ticks/s single-thread)", 1e6 / tick_us.max(1e-9));
}
