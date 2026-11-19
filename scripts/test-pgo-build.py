#!/usr/bin/env python3
"""Tests of scripts/pgo-build.sh's robigo-rt training that need no build: the
check that a conformance run ran cases, and that robigo-rt's R_HOME tree is
made before the training runs that boot from it."""
import os
import re
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "pgo-build.sh")


def function(name):
    """The shell function `name` as the script defines it."""
    text = open(SCRIPT).read()
    m = re.search(rf"^{name}\(\) {{\n.*?^}}\n", text, re.M | re.S)
    assert m, f"no {name}() in pgo-build.sh"
    return m.group(0)


class Training(unittest.TestCase):
    def ran(self, log):
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as f:
            f.write(log)
        try:
            return subprocess.run(["bash", "-c", function("ran_cases") + 'ran_cases "$1"', "bash", f.name]).returncode
        finally:
            os.unlink(f.name)

    def test_a_run_with_cases_counts(self):
        log = ("ok   tests/rt/a.R\nFAIL tests/rt/b.R\n\n"
               "robigo-rt conformance against GNU R 4.6.2: 1/2 pass; 2 stdout, 1 stderr, 2 exit status match\n")
        self.assertEqual(self.ran(log), 0)
        self.assertEqual(self.ran(log.replace("ok   tests/rt/a.R\nFAIL", "none")), 0)

    def test_a_summary_of_nothing_does_not(self):
        # the runner prints its summary for 0/0 too
        self.assertNotEqual(self.ran("\nrobigo-rt conformance against GNU R 4.6.2: 0/0 pass; 0 stdout, 0 stderr, "
                                     "0 exit status match\n(no robigo-rt case selected)\n"), 0)
        self.assertNotEqual(self.ran("error: could not compile `robigo-rtapi`\n"), 0)

    def test_the_tree_is_made_before_the_first_boot(self):
        body = function("train_rt").splitlines()
        home = next(i for i, l in enumerate(body) if "scripts/rt-home.sh" in l)
        boot = next(i for i, l in enumerate(body) if '"$bin"' in l)
        self.assertLess(home, boot)


if __name__ == "__main__":
    unittest.main()
