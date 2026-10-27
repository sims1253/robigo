#!/usr/bin/env bash
# Run serial kernel measurements. Build first, under the shared build lock:
# flock ~/.cache/cargo-target/.robigo-build.lock \
#   cargo build --release -p robigo-numerics --examples
# Usage: scripts/ldouble-callgrind.sh [output.json]
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 - "$@" <<'PY'
import json, os, re, statistics, subprocess, sys, tempfile
from pathlib import Path

target = Path(json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps']))['target_directory'])
exe = target / 'release/examples/policy_bench'
valgrind = os.environ.get('VALGRIND', str(Path.home() / '.cache/rr-tools/bin/valgrind'))
state = Path('.robigo/numerics')
state.mkdir(parents=True, exist_ok=True)
iters = int(os.environ.get('LD_ITERS', 300))
wall_iters = int(os.environ.get('LD_WALL_ITERS', 10000))
runs = int(os.environ.get('LD_RUNS', 5))
result = {'n': 8192, 'iterations': iters, 'wall_iterations': wall_iters, 'wall_runs': runs, 'kernels': {}}
# Subtract the matched process/setup/dispatch baseline, not an unrelated loop.
def count(kernel, mode, path):
    subprocess.run([valgrind, '-q', '--tool=callgrind', f'--callgrind-out-file={path}', str(exe), kernel, mode, str(iters)], stdout=subprocess.DEVNULL, check=True)
    return int(re.search(r'^summary: (\d+)', path.read_text(), re.M)[1])
with tempfile.TemporaryDirectory(prefix='callgrind-', dir=state) as tmp:
    path = Path(tmp) / 'out'
    base = {p: count('none', p, path) for p in ('exact', 'fast')}
    print('kernel,exact_instr/element,fast_instr/element,exact_ns/element,fast_ns/element', flush=True)
    for k in ('sum', 'prod', 'mean', 'cumsum', 'cumprod', 'dev', 'sqdev', 'cross', 'add_each'):
        row = {}
        for p in ('exact', 'fast'):
            ins = (count(k, p, path) - base[p]) / (8192 * iters)
            times = [float(subprocess.check_output([str(exe), k, p, str(wall_iters)], text=True).split()[1]) for _ in range(runs)]
            row[p] = {'instructions_per_element': ins, 'ns_per_element': min(times), 'median_ns_per_element': statistics.median(times)}
        result['kernels'][k] = row
        print(f"{k},{row['exact']['instructions_per_element']:.5f},{row['fast']['instructions_per_element']:.5f},{row['exact']['ns_per_element']:.5f},{row['fast']['ns_per_element']:.5f}", flush=True)
out = Path(sys.argv[1] if len(sys.argv) > 1 else state / 'kernels.json')
out.write_text(json.dumps(result, indent=2) + '\n')
PY
