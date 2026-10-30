#!/usr/bin/env python3
"""Confirm flagged regressions with both exact binaries in the current environment.

Stored measurements select candidates; the paired measurements decide the gate.
Original measurements and callgrind profiles are retained for diagnosis.
"""
import argparse
import copy
import json
from pathlib import Path
from types import SimpleNamespace

import perf


def confirm(base, new, programs, measure, threshold):
    baseline = copy.deepcopy(base)
    candidate = copy.deepcopy(new)
    for name, row in new['results'].items():
        old = base['results'].get(name, {})
        b, n = old.get('robigo', {}), row.get('robigo', {})
        if old.get('wrong') or row.get('wrong') or 'error' in b or 'error' in n:
            continue
        if 'instr' not in b or 'instr' not in n or n['instr'] <= b['instr'] * (1 + threshold / 100):
            continue
        program = programs[name]
        oracle = measure(program, 'gnur', 0)
        samples = {side: [measure(program, side, repeat) for repeat in (1, 2)]
                   for side in ('base', 'candidate')}
        for side, runs in samples.items():
            for result in runs:
                if 'instr' not in result or 'error' in result or result.get('status') != 0:
                    raise RuntimeError(f'{name}: {side} measurement failed: {result}')
                if (result['out'], result['status']) != (oracle.get('out'), oracle.get('status')):
                    raise RuntimeError(f'{name}: {side} output differs from GNU R')
        counts = {side: [r['instr'] for r in runs] for side, runs in samples.items()}
        baseline['results'][name]['robigo']['instr'] = min(counts['base'])
        candidate['results'][name]['robigo']['instr'] = min(counts['candidate'])
        print(f"paired {name}: stored {b['instr']:,} -> {n['instr']:,}; "
              f"base {counts['base']}; candidate {counts['candidate']}", flush=True)
    return baseline, candidate


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base')
    parser.add_argument('new')
    parser.add_argument('--base-robigo', required=True)
    parser.add_argument('--robigo', required=True)
    parser.add_argument('--threshold', type=float, default=0.5)
    parser.add_argument('--profiles', default='.robigo/perf-paired')
    args = parser.parse_args()
    profiles = Path(args.profiles).resolve()
    profiles.mkdir(parents=True, exist_ok=True)
    binaries = {'base': str(Path(args.base_robigo).resolve()),
                'candidate': str(Path(args.robigo).resolve())}

    def measure(program, side, repeat):
        profile = profiles / f"{program[0].replace('/', '_')}-{side}-{repeat}.cg"
        return perf.run_one(program, 'gnur' if side == 'gnur' else 'robigo',
                            binaries.get(side, ''), 'instr', 1, 'valgrind', 600,
                            profile=str(profile))

    original = json.loads(Path(args.new).read_text())
    (profiles / 'unpaired.json').write_text(json.dumps(original, indent=1))
    base, new = confirm(json.loads(Path(args.base).read_text()), original,
                        {p[0]: p for p in perf.programs(perf.conf())}, measure, args.threshold)
    paired = profiles / 'base.json'
    paired.write_text(json.dumps(base, indent=1))
    Path(args.new).write_text(json.dumps(new, indent=1))
    return perf.cmd_compare(SimpleNamespace(base=str(paired), new=args.new, threshold=args.threshold))


if __name__ == '__main__':
    raise SystemExit(main())
