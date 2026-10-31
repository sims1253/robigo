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
import re
import shutil
import hashlib
import tempfile
from pathlib import Path
from types import SimpleNamespace

import perf


def confirm(base, new, programs, measure, threshold, jobs=1):
    baseline = copy.deepcopy(base)
    candidate = copy.deepcopy(new)
    for name in sorted(base['results'].keys() - new['results'].keys()):
        print(f"REMOVED {name}: absent from candidate; no paired comparison", flush=True)

    def is_flagged(name, row):
        old = base['results'].get(name, {})
        if not old:
            return False
        b, n = old.get('robigo', {}), row.get('robigo', {})
        base_failed = old.get('wrong') or 'error' in b or b.get('status', 0) != 0
        if base_failed:
            return False
        if row.get('wrong') or 'error' in n or n.get('status', 0) != 0:
            return True
        if 'instr' not in b or 'instr' not in n or b['instr'] <= 0:
            return True
        delta = 100.0 * (n['instr'] - b['instr']) / b['instr']
        return delta > threshold

    flagged = [(name, row) for name, row in new['results'].items() if is_flagged(name, row)]
    unflagged_count = len(new['results']) - len(flagged)
    if unflagged_count > 0:
        print(f"{unflagged_count} benchmark(s) unflagged; keeping direct comparison", flush=True)
    if flagged:
        print(f"{len(flagged)} benchmark(s) flagged for paired confirmation: {[f[0] for f in flagged]}", flush=True)

    def compare_case(item):
        name, row = item
        old = base['results'].get(name, {})
        b, n = old.get('robigo', {}), row.get('robigo', {})
        program = programs.get(name, (name, '', '', ''))
        driver = program[2] if isinstance(program, (list, tuple)) and len(program) > 2 else (program if isinstance(program, str) else '')
        packages = program[3] if isinstance(program, (list, tuple)) and len(program) > 3 else ''
        parameter = re.search(r'^\.n <- (.+)$', driver, re.MULTILINE)
        digest = hashlib.sha256(driver.encode()).hexdigest()
        print(f"paired scenario {name}: parameter={parameter[1] if parameter else 'in driver'}; "
              f"repeats=2; packages={packages}; driver_sha256={digest}; "
              "same driver on base/candidate; cache OFF", flush=True)
        gnur_record = old.get('gnur', {})
        if gnur_record and 'out' in gnur_record and gnur_record.get('status', 0) == 0 and 'error' not in gnur_record:
            oracle = gnur_record
            oracle_ok = True
        else:
            oracle = measure(program, 'gnur', 0)
            oracle_ok = oracle_succeeded(name, oracle)
        if not oracle_ok:
            candidate['results'][name]['gnur'] = {k: v for k, v in oracle.items() if k != 'out'}
            candidate['results'][name]['oracle_warning'] = 'GNU R measurement failed; ratio unavailable'
        samples = {side: [measure(program, side, repeat) for repeat in (1, 2)]
                   for side in ('base', 'candidate')}
        for side, runs in samples.items():
            for result in runs:
                if 'instr' not in result or 'out' not in result or 'error' in result or result.get('status') != 0:
                    raise RuntimeError(f'{name}: {side} measurement failed: {result}')
                reference = oracle if oracle_ok else samples['base'][0]
                if (result.get('out'), result['status']) != (reference.get('out'), reference.get('status')):
                    source = 'GNU R' if oracle_ok else 'base'
                    raise RuntimeError(f'{name}: {side} output differs from {source}')
        counts = {side: [r['instr'] for r in runs] for side, runs in samples.items()}
        baseline['results'][name]['robigo']['instr'] = min(counts['base'])
        candidate['results'][name]['robigo']['instr'] = min(counts['candidate'])
        candidate['results'][name].pop('wrong', None)
        candidate['results'][name]['robigo']['status'] = 0
        candidate['results'][name]['robigo'].pop('error', None)
        stored_b = f"{b['instr']:,}" if 'instr' in b else "None"
        stored_n = f"{n['instr']:,}" if 'instr' in n else "None"
        print(f"paired cache OFF {name}: stored {stored_b} -> {stored_n}; "
              f"base {counts['base']}; candidate {counts['candidate']}", flush=True)

    with ThreadPoolExecutor(max_workers=jobs) as workers:
        list(workers.map(compare_case, flagged))
    return baseline, candidate


def oracle_succeeded(name, result):
    if 'error' in result or result.get('status') != 0 or 'out' not in result:
        print(f"WARNING {name}: GNU R measurement failed; ratio unavailable: {result}", flush=True)
        return False
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base', nargs='?', default=None, help='baseline perf.json')
    parser.add_argument('new', nargs='?', default=None, help='candidate perf.json')
    parser.add_argument('--base-robigo', default=None, help='baseline robigo executable')
    parser.add_argument('--robigo', required=True, help='candidate robigo executable')
    parser.add_argument('--rdbflat', default=None, help='path to rdbflat directory')
    parser.add_argument('--diagnostic-only', action='store_true',
                        help='run only the rdbflat diagnostic on nbody_2')
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--threshold', type=float, default=0.5)
    parser.add_argument('--profiles', default='.robigo/perf-paired')
    args = parser.parse_args()

    if not args.diagnostic_only:
        if not args.base or not args.new or not args.base_robigo:
            parser.error("base, new, and --base-robigo are required unless --diagnostic-only is set")

    profiles = Path(args.profiles).resolve()
    profiles.mkdir(parents=True, exist_ok=True)
    # Isolate executables from artifact caches: unset also searches beside
    # the executable. The isolated parent has no rdbflat/ directory.
    with tempfile.TemporaryDirectory(prefix='robigo-paired-') as isolated:
        binaries = {}
        items = [('candidate', args.robigo)]
        if args.base_robigo:
            items.insert(0, ('base', args.base_robigo))
        for side, source in items:
            target = Path(isolated) / side / 'bin' / 'robigo'
            target.parent.mkdir(parents=True)
            shutil.copy2(source, target)
            binaries[side] = str(target)
        (profiles / 'binaries.json').write_text(json.dumps({side: {
            'source': source, 'sha256': hashlib.sha256(Path(source).read_bytes()).hexdigest()}
            for side, source in items}, indent=1))
        return run(args, profiles, binaries)


def run(args, profiles, binaries):
    cache = args.rdbflat or os.environ.pop('ROBIGO_RDBFLAT', None)
    if 'ROBIGO_RDBFLAT' in os.environ:
        os.environ.pop('ROBIGO_RDBFLAT')
    if not cache and Path('dist/rdbflat').is_dir():
        cache = str(Path('dist/rdbflat').resolve())

    def measure(program, side, repeat):
        profile = profiles / f"{program[0].replace('/', '_')}-{side}-{repeat}.cg"
        # Retain the exact shared driver and package set, so parameter and
        # warmup differences can be checked without reconstructing a run.
        driver = program[2] if isinstance(program, (list, tuple)) and len(program) > 2 else (program if isinstance(program, str) else '')
        packages = program[3] if isinstance(program, (list, tuple)) and len(program) > 3 else ''
        scenario = {'name': program[0], 'default_packages': packages,
                    'driver': driver, 'cache': os.environ.get('ROBIGO_RDBFLAT'),
                    'driver_sha256': hashlib.sha256(driver.encode()).hexdigest()}
        profile.with_suffix('.scenario.json').write_text(json.dumps(scenario, indent=1))
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
            base_results = json.loads(Path(args.base).read_text()).get('results', {}) if args.base else {}
            gnur_record = base_results.get(name, {}).get('gnur')
            if gnur_record and 'out' in gnur_record and gnur_record.get('status', 0) == 0:
                oracle = gnur_record
                oracle_ok = True
            else:
                oracle = measure(program, 'gnur', 'diagnostic')
                oracle_ok = oracle_succeeded(name, oracle)
            for mode in ('unset', 'set'):
                if mode == 'set':
                    os.environ['ROBIGO_RDBFLAT'] = str(cache)
                else:
                    os.environ.pop('ROBIGO_RDBFLAT', None)
                for side in binaries:
                    result = measure(program, side, mode)
                    if 'error' in result or 'instr' not in result or result.get('status') != 0:
                        raise RuntimeError(f'{name}: {side}-{mode} measurement failed: {result}')
                    reference = oracle if oracle_ok else diagnostic.get(f'{side}-unset', result)
                    if result.get('out') != reference['out']:
                        raise RuntimeError(f'{name}: {side}-{mode} output differs from reference')
                    diagnostic[f'{side}-{mode}'] = result
            os.environ.pop('ROBIGO_RDBFLAT', None)
        (profiles / 'cache-diagnostic.json').write_text(json.dumps(diagnostic, indent=1))
        print('rdbflat diagnostic ONLY (excluded from gate): ' +
              json.dumps(diagnostic), flush=True)
        for side in binaries:
            u_key, s_key = f'{side}-unset', f'{side}-set'
            if u_key in diagnostic and s_key in diagnostic:
                u = diagnostic[u_key]['instr']
                s = diagnostic[s_key]['instr']
                win = u - s
                pct = 100.0 * win / u
                print(f"rdbflat diagnostic: {side} saves {win:,} instructions ({pct:.2f}%) on nbody_2 with cache enabled", flush=True)

    if args.diagnostic_only:
        return 0

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
