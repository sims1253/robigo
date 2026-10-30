#!/usr/bin/env python3
"""Check that paired confirmation preserves the gate's strict failure cases."""
import importlib.util
from pathlib import Path
import unittest
import json
import os
import tempfile
import io
from contextlib import redirect_stdout
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('paired', Path(__file__).with_name('perf-paired.py'))
paired = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paired)


class PairedGate(unittest.TestCase):
    def setUp(self):
        # Synthetic regression verdicts must not resemble CI measurements.
        self.output = io.StringIO()
        self.capture = redirect_stdout(self.output)
        self.capture.__enter__()
        self.addCleanup(self.capture.__exit__, None, None, None)

    def verdict(self, base, new):
        with tempfile.TemporaryDirectory() as tmp:
            b, n = Path(tmp) / 'base.json', Path(tmp) / 'new.json'
            b.write_text(json.dumps(base))
            n.write_text(json.dumps(new))
            return paired.perf.cmd_compare(SimpleNamespace(base=b, new=n, threshold=0.5))
    def run_pair(self, candidate, output='correct', base_status=0):
        base = {'results': {'case': {'robigo': {'instr': 100, 'status': 0}}}}
        new = {'results': {'case': {'robigo': {'instr': 130, 'status': 0}}}}

        def measure(program, side, repeat):
            return {'instr': {'base': 120, 'candidate': candidate, 'gnur': 200}[side],
                    'out': output if side == 'candidate' else 'correct',
                    'status': base_status if side == 'base' else 0}

        b, n = paired.confirm(base, new, {'case': 'case'}, measure, 0.5)
        self.assertEqual(base['results']['case']['robigo']['instr'], 100)
        self.assertEqual(new['results']['case']['robigo']['instr'], 130)
        return b['results']['case']['robigo']['instr'], n['results']['case']['robigo']['instr']

    def test_same_environment_clears_historical_difference(self):
        self.assertEqual(self.run_pair(119), (120, 119))

    def test_real_slowdown_remains_above_threshold(self):
        baseline, candidate = self.run_pair(130)
        self.assertGreater(100 * (candidate / baseline - 1), 0.5)

    def test_stored_cache_win_cannot_hide_slowdown(self):
        # Candidate's cached count looks faster than the historical base.
        # Cache-OFF measurements must still expose the actual slowdown.
        base = {'results': {'case': {'robigo': {'instr': 100, 'status': 0}}}}
        new = {'results': {'case': {'robigo': {'instr': 90, 'status': 0}}}}
        def measure(program, side, repeat):
            return {'instr': {'base': 120, 'candidate': 130, 'gnur': 200}[side],
                    'out': 'correct', 'status': 0}
        b, n = paired.confirm(base, new, {'case': 'case'}, measure, 0.5)
        self.assertGreater(n['results']['case']['robigo']['instr'],
                           b['results']['case']['robigo']['instr'] * 1.005)

    def oracle_failure(self, count=120, output='correct'):
        doc = {'results': {'case': {'robigo': {'instr': 100, 'status': 0}}}}
        def measure(program, side, repeat):
            if side == 'gnur':
                return {'instr': 200, 'status': 1, 'out': ''}
            return {'instr': count if side == 'candidate' else 120, 'status': 0,
                    'out': output if side == 'candidate' else 'correct'}
        return paired.confirm(doc, doc, {'case': 'case'}, measure, 0.5)

    def test_failed_oracle_warns_and_clean_robigo_passes(self):
        b, n = self.oracle_failure()
        self.assertEqual(self.verdict(b, n), 0)
        self.assertIn('WARNING case: GNU R measurement failed', self.output.getvalue())
        paired.perf.report(n, False)
        self.assertIn('ratio unavailable', self.output.getvalue())
        self.assertIn('case', n['results'])

    def test_failed_oracle_cannot_hide_slowdown(self):
        b, n = self.oracle_failure(count=130)
        self.assertEqual(self.verdict(b, n), 1)

    def test_failed_oracle_cannot_hide_changed_output(self):
        with self.assertRaisesRegex(RuntimeError, 'output differs from base'):
            self.oracle_failure(output='wrong')

    def test_benchmark_present_on_only_one_side(self):
        base = {'results': {'removed': {'robigo': {'instr': 100, 'status': 0}}}}
        new = {'results': {'added': {'robigo': {'instr': 120, 'status': 0}}}}
        def measure(*args):
            self.fail('unmatched benchmarks must not be paired')
        b, n = paired.confirm(base, new, {}, measure, 0.5)
        self.assertEqual(self.verdict(b, n), 0)
        self.assertIn('REMOVED removed', self.output.getvalue())
        self.assertIn('ADDED added', self.output.getvalue())
        n['results']['added']['robigo']['status'] = 1
        self.assertEqual(self.verdict(b, n), 1)

    def test_existing_failure_reported_but_new_failure_fails(self):
        failed = {'results': {'case': {'robigo': {'instr': 100, 'status': 1}}}}
        b, n = paired.confirm(failed, failed, {}, lambda *args: self.fail('known failure'), 0.5)
        self.assertEqual(self.verdict(b, n), 0)
        self.assertIn('UNSUPPORTED case', self.output.getvalue())
        b['results']['case']['robigo']['status'] = 0
        self.assertEqual(self.verdict(b, n), 1)

    def test_initial_run_failed_oracle_does_not_mark_wrong(self):
        with tempfile.TemporaryDirectory() as tmp:
            result_path = Path(tmp) / 'perf.json'
            args = SimpleNamespace(suite=None, filter=None, robigo='binary', runtime='both',
                                   jobs=1, metric='instr', runs=1, timeout=1,
                                   quiet=True, out=str(result_path), merge=False)
            def measure(program, runtime, *args):
                return {'instr': 100, 'out': 'answer' if runtime == 'robigo' else '',
                        'status': 0 if runtime == 'robigo' else 1}
            with patch.object(paired.perf, 'programs', return_value=[('case', '', '', '')]), \
                 patch.object(paired.perf, 'robigo_path', return_value='binary'), \
                 patch.object(paired.perf, 'run_one', side_effect=measure), \
                 patch('sys.stderr', io.StringIO()):
                paired.perf.cmd_run(args)
            row = json.loads(result_path.read_text())['results']['case']
            self.assertFalse(row.get('wrong'))
            self.assertIn('ratio unavailable', row['oracle_warning'])

    def test_failed_candidate_measurement_fails(self):
        doc = {'results': {'case': {'robigo': {'instr': 100, 'status': 0}}}}
        def measure(program, side, repeat):
            return {'instr': 100, 'out': 'correct', 'status': 1 if side == 'candidate' else 0}
        with self.assertRaisesRegex(RuntimeError, 'candidate measurement failed'):
            paired.confirm(doc, doc, {'case': 'case'}, measure, 0.5)

    def test_cache_feature_cannot_credit_candidate(self):
        self.cache_feature(oracle_status=0)

    def test_diagnostic_oracle_failure_does_not_abort_comparison(self):
        self.cache_feature(oracle_status=1)

    def cache_feature(self, oracle_status):
        # Simulate an old binary that ignores the cache, and a candidate
        # whose cached run is faster despite a 10% cache-OFF regression.
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ):
            root = Path(tmp)
            artifact = root / 'artifact'
            artifact.mkdir()
            binary = artifact / 'robigo'
            binary.write_text('fake binary')
            (root / 'rdbflat').mkdir()  # Would be discovered without isolation.
            os.environ['ROBIGO_RDBFLAT'] = str(root / 'rdbflat')
            doc = {'results': {'suite/nbody_2': {'robigo': {'instr': 100}}}}
            for side in ['base', 'new']:
                (root / side).write_text(json.dumps(doc))
            calls = []
            def measure(program, runtime, executable, *args, **kwargs):
                cached = 'ROBIGO_RDBFLAT' in os.environ
                calls.append((runtime, cached))
                if runtime == 'gnur':
                    count = 200
                else:
                    path = Path(executable)
                    self.assertFalse((path.parent.parent / 'rdbflat').exists())
                    self.assertEqual(path.read_text(), 'fake binary')
                    count = 100 if 'base' in path.parts else (80 if cached else 110)
                return {'instr': count, 'out': 'correct', 'status': oracle_status if runtime == 'gnur' else 0}
            argv = ['paired', str(root / 'base'), str(root / 'new'),
                    '--base-robigo', str(binary), '--robigo', str(binary),
                    '--profiles', str(root / 'profiles')]
            with patch('sys.argv', argv), patch.object(paired.perf, 'programs',
                    return_value=[('suite/nbody_2', '', '', '')]), patch.object(
                    paired.perf, 'run_one', side_effect=measure):
                self.assertNotEqual(paired.main(), 0)
            result = json.loads((root / 'new').read_text())
            self.assertEqual(result['results']['suite/nbody_2']['robigo']['instr'], 110)
            self.assertEqual(sum(cached for _, cached in calls), 2)

    def test_wrong_output_cannot_clear_gate(self):
        with self.assertRaisesRegex(RuntimeError, 'output differs'):
            self.run_pair(119, output='wrong')

    def test_failed_baseline_cannot_clear_gate(self):
        with self.assertRaisesRegex(RuntimeError, 'measurement failed'):
            self.run_pair(119, base_status=1)


if __name__ == '__main__':
    unittest.main()
