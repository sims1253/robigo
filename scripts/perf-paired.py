#!/usr/bin/env python3
"""Compare both exact binaries in the current environment without cache credit.

Every successful benchmark is compared with the cache disabled on both sides.
Cache-enabled measurements are diagnostic only and cannot clear the gate.
Original measurements and callgrind profiles are retained for diagnosis.
"""
import argparse
import copy
from concurrent.futures import ThreadPoolExecutor
import json
import os
import shutil
import tempfile
from pathlib import Path
from types import SimpleNamespace

import perf


def confirm(base, new, programs, measure, threshold, jobs=1):
    baseline = copy.deepcopy(base)
    candidate = copy.deepcopy(new)
    def compare_case(item):
        name, row = item
        old = base['results'].get(name, {})
        b, n = old.get('robigo', {}), row.get('robigo', {})
        if old.get('wrong') or row.get('wrong') or 'error' in b or 'error' in n:
            return
        if 'instr' not in b or 'instr' not in n:
            return
        program = programs[name]
        oracle = measure(program, 'gnur', 0)
        if 'error' in oracle or oracle.get('status') != 0 or 'out' not in oracle:
            raise RuntimeError(f'{name}: GNU R measurement failed: {oracle}')
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
        print(f"paired cache OFF {name}: stored {b['instr']:,} -> {n['instr']:,}; "
              f"base {counts['base']}; candidate {counts['candidate']}", flush=True)
    with ThreadPoolExecutor(max_workers=jobs) as workers:
        list(workers.map(compare_case, new['results'].items()))
    return baseline, candidate


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base')
    parser.add_argument('new')
    parser.add_argument('--base-robigo', required=True)
    parser.add_argument('--robigo', required=True)
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--threshold', type=float, default=0.5)
    parser.add_argument('--profiles', default='.robigo/perf-paired')
    args = parser.parse_args()
    profiles = Path(args.profiles).resolve()
    profiles.mkdir(parents=True, exist_ok=True)
    # Isolate executables from artifact caches: unset also searches beside
    # the executable. The isolated parent has no rdbflat/ directory.
    with tempfile.TemporaryDirectory(prefix='robigo-paired-') as isolated:
        binaries = {}
        for side, source in [('base', args.base_robigo), ('candidate', args.robigo)]:
            target = Path(isolated) / side / 'bin' / 'robigo'
            target.parent.mkdir(parents=True)
            shutil.copy2(source, target)
            binaries[side] = str(target)
        return run(args, profiles, binaries)


def run(args, profiles, binaries):
    cache = os.environ.pop('ROBIGO_RDBFLAT', None)

    def measure(program, side, repeat):
        profile = profiles / f"{program[0].replace('/', '_')}-{side}-{repeat}.cg"
        result = perf.run_one(program, 'gnur' if side == 'gnur' else 'robigo',
                              binaries.get(side, ''), 'instr', 1, 'valgrind', 600,
                              profile=str(profile))
        (profile.with_suffix('.json')).write_text(json.dumps(result, indent=1))
        return result

    # Four combinations in one job on the same program. These measurements
    # explain cache savings; only cache-OFF samples below decide pass/fail.
    programs = {p[0]: p for p in perf.programs(perf.conf())}
    diagnostic = {}
    if cache:
        for name, program in programs.items():
            if not name.endswith('/nbody_2'):
                continue
            oracle = measure(program, 'gnur', 'diagnostic')
            if 'error' in oracle or oracle.get('status') != 0 or 'out' not in oracle:
                raise RuntimeError(f'{name}: GNU R measurement failed: {oracle}')
            for mode in ('unset', 'set'):
                if mode == 'set':
                    os.environ['ROBIGO_RDBFLAT'] = cache
                else:
                    os.environ.pop('ROBIGO_RDBFLAT', None)
                for side in binaries:
                    result = measure(program, side, mode)
                    if 'error' in result or 'instr' not in result or result.get('status') != 0:
                        raise RuntimeError(f'{name}: {side}-{mode} measurement failed: {result}')
                    if result.get('out') != oracle['out']:
                        raise RuntimeError(f'{name}: {side}-{mode} output differs from GNU R')
                    diagnostic[f'{side}-{mode}'] = result
            os.environ.pop('ROBIGO_RDBFLAT', None)
        (profiles / 'cache-diagnostic.json').write_text(json.dumps(diagnostic, indent=1))
        print('rdbflat diagnostic ONLY (excluded from gate): ' +
              json.dumps(diagnostic), flush=True)

    original = json.loads(Path(args.new).read_text())
    (profiles / 'unpaired.json').write_text(json.dumps(original, indent=1))
    base, new = confirm(json.loads(Path(args.base).read_text()), original,
                        programs, measure, args.threshold, args.jobs)
    paired = profiles / 'base.json'
    paired.write_text(json.dumps(base, indent=1))
    Path(args.new).write_text(json.dumps(new, indent=1))
    return perf.cmd_compare(SimpleNamespace(base=str(paired), new=args.new, threshold=args.threshold))


if __name__ == '__main__':
    raise SystemExit(main())
