#!/usr/bin/env python3
"""Which of the base functions GNU R's own build reaches robigo-rt lacks (L4).

    scripts/rt-build-gaps.py --robigo-rt BIN [--reach DIR]

DIR (default .robigo/build-reach) holds scripts/rt-build-reach.sh's keys,
one file per build step. Each primitive and .Internal among them is called
in robigo-rt with no arguments; an "is not implemented yet" error marks it
missing (an arity or type error means it exists). A call that crashes
robigo-rt (a panic, not an R error) is reported as such and skipped. The
report lists each missing function with the steps that reach it; "static"
means only compiler's or tools' code names it, untraced.

Calls that would act rather than fail with no arguments are skipped (quit,
Sys.sleep, the control-flow specials).
"""

import argparse
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
STEPS = ["base", "compiler", "tools", "desc", "vdesc", "vns", "static"]
SKIP = {
    # control flow and the specials whose bare call would act, not error
    "quit", "q", "Sys.sleep", "readline", "browser", "invokeRestart", ".Internal", "return", "break",
    "next", "repeat", "while", "for", "if", "function", "{", "(", "<-", "<<-", "=", "on.exit",
    "UseMethod", "NextMethod", "standardGeneric", "Recall", "stop", "warning",
}
# (the probe is R code that robigo-rt runs: no readLines, no call(); the
# keys are written into it and the calls built with parse())
PROBE = r'''
for (k in keys) {
    kind <- sub(":.*", "", k); nm <- sub("^[^:]*:", "", k)
    if (!(kind %in% c("primitive", "internal")) || nm %in% skip) next
    e <- parse(text = if (kind == "internal") paste0(".Internal(`", nm, "`())") else paste0("`", nm, "`()"))[[1L]]
    cat("TRY", k, "\n")
    msg <- tryCatch({ eval(e, baseenv()); "" }, error = function(e) conditionMessage(e), warning = function(w) "")
    # (the stubs of names robigo-rt has no function for; another "not
    # implemented" error is a function that exists, called without arguments)
    if (grepl("^robigo: (\\.Internal\\(.*\\(\\)\\)|the primitive '.*') is not implemented yet$", msg))
        cat("MISSING", k, "\n")
}
cat("DONE\n")
'''


def rstr(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--robigo-rt", required=True)
    ap.add_argument("--reach", default=str(ROOT / ".robigo/build-reach"))
    a = ap.parse_args()
    reach = pathlib.Path(a.reach)
    by_key = {}
    for st in STEPS:
        f = reach / f"{st}.keys"
        if not f.exists():
            sys.exit(f"rt-build-gaps: no {f}; run scripts/rt-build-reach.sh first")
        for k in f.read_text().split():
            if re.match(r"^(primitive|internal):", k):
                by_key.setdefault(k, []).append(st)
    keys = sorted(by_key)
    # the tree robigo-rt finds as its own R_HOME (its build checkout's): it
    # ignores an inherited R_HOME with GNU R's warning
    made = subprocess.run([str(ROOT / "scripts/rt-home.sh")], capture_output=True, text=True, check=False)
    if made.returncode:
        sys.exit(f"rt-build-gaps: scripts/rt-home.sh failed (status {made.returncode}):\n{made.stderr}")
    env = {k: v for k, v in os.environ.items() if k != "R_HOME"}
    # (stage 0, booting from that source tree, as GNU R's build runs: the
    # default boot would need the database tree, which this does not make)
    env.update(R_DEFAULT_PACKAGES="NULL", LANGUAGE="en", LC_ALL="C.UTF-8", ROBIGO_RT_BOOT="stage0")
    skip, crashed, missing = set(SKIP), [], []
    while True:
        script = (f"keys <- c({', '.join(rstr(k) for k in keys)})\n"
                  f"skip <- c({', '.join(rstr(s) for s in sorted(skip))})\n" + PROBE)
        r = subprocess.run([a.robigo_rt, "--vanilla", "-e", script], env=env, capture_output=True, text=True, check=False)
        out = r.stdout.splitlines()
        missing = [l.split(" ", 1)[1].strip() for l in out if l.startswith("MISSING ")]
        if "DONE" in out:
            break
        tried = [l.split(" ", 1)[1].strip() for l in out if l.startswith("TRY ")]
        if not tried:
            sys.exit(f"rt-build-gaps: robigo-rt ran no probe:\n{r.stderr}")
        err = r.stderr.splitlines()
        at = next((i for i, l in enumerate(err) if "panicked at" in l), None)
        crashed.append((tried[-1], " ".join(err[at:at + 2]) if at is not None else (err or ["?"])[-1]))
        skip.add(tried[-1].split(":", 1)[1])
    print(f"robigo-rt lacks {len(missing)} of the {len(keys)} primitives and .Internals GNU R's build reaches:")
    for k in sorted(missing, key=lambda k: (-len(by_key[k]), k)):
        print(f"  {k:<36} {' '.join(by_key[k])}")
    for k, msg in crashed:
        print(f"  {k:<36} CRASHED (called with no arguments): {msg[:120]}")


if __name__ == "__main__":
    main()
