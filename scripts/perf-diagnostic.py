#!/usr/bin/env python3
"""Profile exact CI artifacts together without changing the performance gate.

Run in the oracle container: perf-diagnostic.py BASE_DIR CANDIDATE_DIR OUT_DIR.
Each artifact directory must contain robigo, natives/, and rdbflat/.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile

import perf

base, candidate, output = map(lambda p: Path(p).resolve(), sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
program = perf.programs(perf.conf(), rx=r'rbench/shootout/nbody_2$')[0]
script = output / 'bench.R'
script.write_text(program[2])
metadata = {'environment': {k: v for k, v in os.environ.items()
                            if k.startswith(('ROBIGO_', 'R_', 'MIMALLOC_', 'LD_'))}}
for name, command in [('cpu', ['lscpu']), ('libc', ['ldd', '--version']),
                      ('valgrind', ['valgrind', '--version']), ('R', ['R', '--version'])]:
    try:
        metadata[name] = subprocess.run(command, capture_output=True, text=True).stdout
    except FileNotFoundError:
        metadata[name] = 'unavailable'
for label, artifact in [('base', base), ('candidate', candidate)]:
    metadata[label + '_files'] = {
        str(p.relative_to(artifact)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in [artifact / 'robigo', *sorted((artifact / 'rdbflat').glob('*.flat')),
                  *sorted((artifact / 'natives').glob('*/*.so'))]
    }
    guards = []
    for flat in sorted((artifact / 'rdbflat').glob('*.flat')):
        data = flat.read_bytes()
        size, mtime, plen = struct.unpack_from('<QQQ', data, 8)
        path = data[32:32 + plen].decode()
        stat = os.stat(path)
        guards.append({'source': path, 'expected': [size, mtime],
                       'actual': [stat.st_size, stat.st_mtime_ns],
                       'valid': (size, mtime) == (stat.st_size, stat.st_mtime_ns)})
    metadata[label + '_cache'] = guards
results = {}
for mode in ('own', 'shared'):
    for label, artifact in [('base', base), ('candidate', candidate)]:
        resources = artifact if mode == 'own' else candidate
        env = dict(os.environ, **perf.BASE_ENV, R_HOME=perf.r_home(),
                   R_DEFAULT_PACKAGES=program[3],
                   ROBIGO_RDBFLAT=str(resources / 'rdbflat'),
                   ROBIGO_NATIVES=str(resources / 'natives'))
        for repeat in (1, 2):
            stem = f'{mode}-{label}-{repeat}'
            cg = output / (stem + '.cg')
            with tempfile.TemporaryDirectory(prefix='robigo-bench-') as tmp:
                benchmark = Path(tmp) / 'bench.R'
                benchmark.write_text(program[2])
                result = subprocess.run(['valgrind', '--tool=callgrind',
                                         f'--callgrind-out-file={cg}', str(artifact / 'robigo'),
                                         '--vanilla', str(benchmark)], cwd=tmp, env=env,
                                        capture_output=True)
            (output / (stem + '.stdout')).write_bytes(result.stdout)
            (output / (stem + '.stderr')).write_bytes(result.stderr)
            count = int(re.search(r'^summary: (\d+)', cg.read_text(), re.M)[1])
            results[stem] = {'instr': count, 'status': result.returncode,
                             'stdout': result.stdout.decode(errors='replace')}
            with (output / (stem + '.txt')).open('w') as annotated:
                subprocess.run(['callgrind_annotate', '--threshold=99', str(cg)],
                               stdout=annotated, check=True)
            print(stem, count, 'status', result.returncode, flush=True)
(output / 'results.json').write_text(json.dumps(results, indent=2))
(output / 'environment.json').write_text(json.dumps(metadata, indent=2))
assert all(r['status'] == 0 for r in results.values()), results
assert len({r['stdout'] for r in results.values()}) == 1, results
