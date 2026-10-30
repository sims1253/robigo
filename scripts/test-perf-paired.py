#!/usr/bin/env python3
"""Check that paired confirmation preserves the gate's strict failure cases."""
import importlib.util
from pathlib import Path
import unittest

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

    def test_wrong_output_cannot_clear_gate(self):
        with self.assertRaisesRegex(RuntimeError, 'output differs'):
            self.run_pair(119, output='wrong')

    def test_failed_baseline_cannot_clear_gate(self):
        with self.assertRaisesRegex(RuntimeError, 'measurement failed'):
            self.run_pair(119, base_status=1)


if __name__ == '__main__':
    unittest.main()
