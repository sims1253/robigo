#!/usr/bin/env python3
"""Measure S4 instructions per iteration, subtracting N from 2N iterations.

Set R_HOME (and, for a relocated GNU R, LD_LIBRARY_PATH) to GNU R 4.5.0.
Example: scripts/s4-instructions.py --runtime robigo --output s4.json
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

CASES = {
    "generic": ('setGeneric("f", function(x) standardGeneric("f")); '
                'setMethod("f", "P", function(x) x@x)\n', "f(x)", 10000),
    "slot": ("", "x@x", 10000),
    "replace": ("", "x@x <- 1", 1000),
    "new": ("", 'new("P", x=1)', 500),
    "valid": ("", "validObject(x)", 1000),
}
SETUP = 'setClass("P", slots=c(x="numeric")); x <- new("P", x=1)\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", choices=("robigo", "gnur", "both"), default="both")
    parser.add_argument("--binary", default=os.environ.get("ROBIGO", ".robigo/bin/robigo"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--filter", default=".", help="regular expression matching workload names")
    args = parser.parse_args()
    env = dict(os.environ, R_DEFAULT_PACKAGES="methods", DEBUGINFOD_URLS="",
               LC_ALL="C.UTF-8", TZ="UTC")
    commands = {"robigo": [str(Path(args.binary).resolve()), "--vanilla"]}
    if args.runtime != "robigo":
        commands["gnur"] = [str(Path(env["R_HOME"]) / "bin/exec/R"),
                            "--vanilla", "--no-echo", "-f"]
    results = {}
    with tempfile.TemporaryDirectory(prefix="robigo-s4-") as directory:
        directory = Path(directory)
        for name, (extra, expression, n) in CASES.items():
            if not re.search(args.filter, name):
                continue
            results[name] = {}
            for runtime, command in commands.items():
                if args.runtime not in (runtime, "both"):
                    continue
                totals = []
                for count in (n, 2 * n):
                    script, profile = directory / "work.R", directory / "callgrind.out"
                    script.write_text(SETUP + extra +
                                      f"for (i in seq_len({count})) {{ {expression} }}\n")
                    run = subprocess.run(["valgrind", "--tool=callgrind",
                                          f"--callgrind-out-file={profile}",
                                          *command, str(script)], env=env, capture_output=True)
                    if run.returncode:
                        raise RuntimeError(run.stderr.decode(errors="replace"))
                    totals.append(int(re.search(r"^summary: (\d+)",
                                                profile.read_text(), re.M)[1]))
                results[name][runtime] = (totals[1] - totals[0]) / n
            print(name, results[name], flush=True)
    if args.output:
        args.output.write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
