#!/usr/bin/env python3
"""Check scripts/perf-gnur.py: the GNU R records perf's cache keeps."""
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("perf_gnur", HERE / "perf-gnur.py")
pg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pg)

OK = {"instr": 10, "out": "1\n", "status": 0}


class Gnur(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()

    def write(self, name, doc):
        p = os.path.join(self.d, name)
        with open(p, "w") as f:
            json.dump(doc, f)
        return p

    def test_extract_keeps_only_usable_gnur_records(self):
        run = self.write("perf.json", {"results": {
            "a/x": {"robigo": {"instr": 5}, "gnur": OK},
            "a/failed": {"gnur": {"instr": 1, "out": "", "status": 1}},
            "a/error": {"gnur": {"error": "timeout"}},
            "a/noout": {"gnur": {"instr": 3, "status": 0}},
            "a/robigo-only": {"robigo": {"instr": 5}}}})
        out = os.path.join(self.d, "gnur.json")
        self.assertEqual(pg.extract(run, out), 0)
        with open(out) as f:
            self.assertEqual(json.load(f), {"results": {"a/x": {"gnur": OK}}})

    def test_covers_only_when_every_selected_benchmark_has_a_record(self):
        g = self.write("gnur.json", {"results": {"a/x": {"gnur": OK}}})
        progs = [("a/x", "a", "", ""), ("a/y", "a", "", "")]

        def programs(c, suite=None, rx=None):
            import re
            return [p for p in progs if not rx or re.search(rx, p[0])]
        with mock.patch.object(pg.perf, "programs", programs), mock.patch.object(pg.perf, "conf", lambda: {}):
            self.assertEqual(pg.missing(g, "a/x"), [])
            self.assertEqual(pg.missing(g, None), ["a/y"])           # the full suite needs a/y too
            self.assertEqual(pg.missing(os.path.join(self.d, "absent"), "a/x"), ["a/x"])
            self.assertEqual(pg.main(["covers", g, "a/x"]), 0)
            self.assertEqual(pg.main(["covers", g]), 1)


if __name__ == "__main__":
    unittest.main()
