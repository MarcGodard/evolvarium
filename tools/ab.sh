#!/usr/bin/env bash
# Multi-seed A/B: run arm A and arm B on the same seeds, summarize --metrics side by side.
# Single runs are useless for balance (same seed spans pop 100..1900 across equivalent builds), so every
# claim needs several seeds. Arms differ by binary and/or extra flags.
#
#   tools/ab.sh [--gens=20] [--seeds="1 5 9"] [--jobs=3] [--out=DIR] \
#       [--bin-a=PATH] [--bin-b=PATH] [--a="flags"] [--b="flags"]
#
# Defaults: both arms use target/release/evolvarium; --jobs caps parallel sims (~0.5 GB each at 5k pop).
# Copy a binary before rebuilding if one arm is the old build: cp target/release/evolvarium /tmp/evo-a
# Paths (--bin-*, --out) are relative to the REPO root. Flag strings are word-split, never globbed.
# Exit status 1 if any run left no metrics.
set -euo pipefail
set -f
cd "$(dirname "$0")/.."

gens=20; seeds="1 5 9"; jobs=3; out=""
bin_a=target/release/evolvarium; bin_b=target/release/evolvarium; fa=""; fb=""
for arg in "$@"; do
  case "$arg" in
    --gens=*) gens="${arg#*=}" ;;
    --seeds=*) seeds="${arg#*=}" ;;
    --jobs=*) jobs="${arg#*=}" ;;
    --out=*) out="${arg#*=}" ;;
    --bin-a=*) bin_a="${arg#*=}" ;;
    --bin-b=*) bin_b="${arg#*=}" ;;
    --a=*) fa="${arg#*=}" ;;
    --b=*) fb="${arg#*=}" ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done
[ "$jobs" -ge 1 ] 2>/dev/null || jobs=1
seeds="$(printf '%s\n' $seeds | awk 'NF && !seen[$0]++' | tr '\n' ' ')" # a repeated seed would race on one file
out="${out:-$(mktemp -d "${TMPDIR:-/tmp}/evo-ab.XXXXXX")}"
mkdir -p "$out"
echo "A: $bin_a $fa"
echo "B: $bin_b $fb"
echo "seeds: $seeds  gens: $gens  out: $out"

run() { # arm bin flags seed
  rm -f "$out/$1-s$4.json" # a reused --out must never feed a previous run's numbers into this summary
  "$2" --headless --no-load --gens="$gens" --seed="$4" --metrics="$out/$1-s$4.json" $3 > "$out/$1-s$4.log" 2>&1 \
    || echo "arm $1 seed $4 exited $?" >&2
}
for s in $seeds; do
  for arm in a b; do
    while [ "$(jobs -rp | wc -l)" -ge "$jobs" ]; do wait -n; done
    if [ $arm = a ]; then run a "$bin_a" "$fa" "$s" & else run b "$bin_b" "$fb" "$s" & fi
  done
done
wait

python3 - "$out" $seeds <<'PY'
import json, os, statistics as st, sys
out, seeds = sys.argv[1], sys.argv[2:]
fields = [  # (label, path into the metrics json)
    ("pop", ("pop",)), ("carnivory", ("world", "mean", "carnivory")), ("builder", ("world", "mean", "builder")),
    ("size", ("world", "mean", "size")), ("flora kg/m2", ("world", "flora_kg_m2")), ("cover kg/m2", ("world", "cover_kg_m2")),
    ("drift C ppm", ("world", "drift_ppm", "c")), ("drift P ppm", ("world", "drift_ppm", "p")),
    ("dT K", ("world", "climate", "anomaly_k")), ("dams", ("world", "building", "dam_cells")),
]
def get(d, path):
    for k in path:
        d = d.get(k) if isinstance(d, dict) else None
    return d
def load(arm):
    rows = []
    for s in seeds:
        p = os.path.join(out, f"{arm}-s{s}.json")
        try:
            rows.append(json.load(open(p)))
        except (OSError, ValueError):
            rows.append(None)  # missing or truncated: the run crashed or was killed
    return rows
A, B = load("a"), load("b")
def fmt(v):
    return "-" if v is None else (f"{v:.0f}" if abs(v) >= 100 else f"{v:.3g}")
print(f"\n{'metric':14} {'A per seed':28} {'B per seed':28} {'A mean':>9} {'B mean':>9} {'B-A':>9}")
for label, path in fields:
    a = [get(r, path) if r else None for r in A]
    b = [get(r, path) if r else None for r in B]
    num = lambda x: isinstance(x, (int, float))
    pairs = [(x, y) for x, y in zip(a, b) if num(x) and num(y)]  # means over seeds BOTH arms finished
    av = [x for x, _ in pairs]
    bv = [y for _, y in pairs]
    am = st.mean(av) if av else None
    bm = st.mean(bv) if bv else None
    diff = (bm - am) if am is not None and bm is not None else None
    print(f"{label:14} {' '.join(fmt(x) for x in a):28} {' '.join(fmt(x) for x in b):28} {fmt(am):>9} {fmt(bm):>9} {fmt(diff):>9}")
missing = [f"{arm}-s{s}" for arm, rows in (("a", A), ("b", B)) for s, r in zip(seeds, rows) if r is None]
print("\nmeans pair seeds present in both arms. A difference smaller than the per-seed spread is not a result.")
if missing:
    print("missing metrics (crashed or killed):", " ".join(missing))
    sys.exit(1)
PY
