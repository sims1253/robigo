#!/usr/bin/env python3
"""Check that a hung package times out, with its whole process tree killed."""
import contextlib
import importlib.util
import io
import os
import shutil
import stat
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('gate', Path(__file__).with_name('cran-gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

# Like R CMD INSTALL running a compiler: the process that hangs is a child,
# and the leader ignores the polite signal.
HANG = """#!/bin/sh
trap '' TERM
sh -c 'trap "" TERM; echo $$ > "$0"; while :; do sleep 1; done' "{pidfile}" &
echo partial output
wait
"""


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


class Timeouts(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir)
        self.pidfile = os.path.join(self.dir, 'child.pid')
        self.fake = os.path.join(self.dir, 'robigo')
        Path(self.fake).write_text(HANG.format(pidfile=self.pidfile))
        os.chmod(self.fake, os.stat(self.fake).st_mode | stat.S_IEXEC)

    def child(self):
        for _ in range(100):
            if os.path.exists(self.pidfile) and Path(self.pidfile).read_text().strip():
                return int(Path(self.pidfile).read_text())
            time.sleep(0.05)
        self.fail('the fake robigo never started its child')

    def assert_killed(self, pid):
        for _ in range(100):
            if not alive(pid):
                return
            time.sleep(0.05)
        os.kill(pid, 9)
        self.fail('the child outlived the timeout')

    def check_group_killed(self):
        with patch.object(gate, 'KILL_GRACE', 0.2):
            with self.assertRaises(gate.StepTimeout) as e:
                gate.run_step('test', [self.fake], 1)
        self.assert_killed(self.child())
        self.assertEqual((e.exception.step, e.exception.limit), ('test', 1))
        self.assertIn('partial output', e.exception.output)

    def test_kills_the_process_group(self):
        with patch.dict(os.environ, ROBIGO_NO_MEMORY_CAP='1'):
            self.check_group_killed()

    @unittest.skipUnless(shutil.which('systemd-run'), 'no systemd-run')
    def test_kills_the_process_group_under_the_memory_cap(self):
        self.check_group_killed()

    def test_finishing_steps_are_not_affected(self):
        with patch.dict(os.environ, ROBIGO_NO_MEMORY_CAP='1'):
            p = gate.run_step('test', ['sh', '-c', 'echo out; echo err >&2; exit 3'], 5)
        self.assertEqual((p.returncode, p.stdout, p.stderr), (3, 'out\n', 'err\n'))

    def test_run_reports_timeout_and_goes_on(self):
        """A hung package is its own outcome, and the next one still runs."""
        a = SimpleNamespace(pkgs=['hung@1.0', 'next@1.0'], tests='tests', diff=False)
        calls = []

        def test(name, version, tests):
            calls.append(name)
            if name == 'hung':
                raise gate.StepTimeout('test', 600, '')
            return version, 2, 0, [], None, None, None

        out = io.StringIO()
        with patch.object(gate, 'robigo_test', test), patch.object(gate, 'entry', lambda _: None), \
                patch('json.load', lambda _: {'next': {'version': '1.0', 'passed': 2, 'failed': 0}}), \
                contextlib.redirect_stdout(out), self.assertRaises(SystemExit) as e:
            gate.cmd_run(a)
        self.assertEqual(calls, ['hung', 'next'])
        self.assertEqual(e.exception.code, 1)
        self.assertIn('hung 1.0: TIMEOUT (test after 600s)', out.getvalue())
        self.assertIn(' - hung: timeout (test after 600s)', out.getvalue())


if __name__ == '__main__':
    unittest.main()
