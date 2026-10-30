#!/usr/bin/env python3
"""Check that paired confirmation preserves the gate's strict failure cases."""
import importlib.util
from pathlib import Path
import unittest
import json
import os
import tempfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('paired', Path(__file__).with_name('perf-paired.py'))
paired = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paired)


class PairedGate(unittest.TestCase):
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

    def test_failed_oracle_cannot_clear_gate(self):
        base = {'results': {'case': {'robigo': {'instr': 100}}}}
        with self.assertRaisesRegex(RuntimeError, 'GNU R measurement failed'):
            paired.confirm(base, base, {'case': 'case'},
                           lambda *args: {'error': 'failed'}, 0.5)

    def test_cache_feature_cannot_credit_candidate(self):
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
                return {'instr': count, 'out': 'correct', 'status': 0}
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
