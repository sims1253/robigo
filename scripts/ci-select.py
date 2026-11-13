#!/usr/bin/env python3
"""Choose which CI jobs a pull request runs, from its changed paths.

    scripts/ci-select.py [--event pull_request|push|...] [--base REF] [--head REF]
    scripts/ci-select.py --paths-from FILE|-          # dry run: paths, one per line
    scripts/ci-select.py --changed PATH [PATH ...]    # dry run
    scripts/ci-select.py ... --github-output          # also write $GITHUB_OUTPUT

The mapping is data: scripts/ci-selection.toml (rules from path globs to a
mode). The result is `full` (run every job the event would normally run),
`light` (skip build-dependent heavy jobs: build, test, counters, corpora, fuzz, cran,
smoke, perf, and the package regression gate) or `rt` (`light` plus the `rt`
job: only G3's runtime crates changed, which the binary does not link). lint,
miri and bytecode-fidelity always run. The pass/fail rules of every gate are
unchanged.

Fail-safe: the run is `full` unless this is a pull request AND the changed
paths could be listed AND every changed path matches a rule. An unmatched
path, an unreadable diff, a malformed mapping or any other event (main,
workflow_dispatch) runs everything. Never silently skip a needed check.

Prints a markdown summary (what runs and why) on stdout. `--github-output`
sets `mode`, `heavy` (true unless `light`) and `runtime` (true when the
package regression gate should run) and `rt` (true unless `light`: the `rt`
job runs) for later steps.
"""
import argparse
import os
import re
import subprocess
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIG = os.path.join(ROOT, "scripts", "ci-selection.toml")


def glob_re(pattern):
    """`*` within a segment, `**` across segments (`**/` may be empty), `?` one char."""
    out, i = [], 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif pattern.startswith("**", i):
            out.append(".*")
            i += 2
        elif pattern[i] == "*":
            out.append("[^/]*")
            i += 1
        elif pattern[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("".join(out) + r"\Z")


# From least to most work: a diff gets the most demanding mode any of its paths asks for.
MODES = ("light", "rt", "full")


def load(path=CONFIG):
    with open(path, "rb") as f:
        conf = tomllib.load(f)
    rules = []
    for r in conf["rules"]:
        mode = r["mode"]
        if mode not in MODES:
            raise ValueError(f"rule {r['name']!r}: mode must be one of {', '.join(MODES)}")
        runtime = bool(r.get("runtime", False))
        pats = [glob_re(p) for p in r["paths"] + r.get("planned", [])]
        rules.append((r["name"], pats, mode, runtime))
    if not rules:
        raise ValueError("no rules")
    return rules


def decide(paths, rules):
    """(mode, runtime, [(path, rule name or None)]).

    `runtime` is true when any matched path can change the binary's behaviour
    (crates, rlib, Cargo, toolchain). On a full result caused by an unmatched
    path or an empty diff, `runtime` is true (fail safe).
    """
    rows, mode, runtime, unmatched = [], "light", False, False
    for p in paths:
        for name, pats, rmode, rt in rules:
            if any(g.match(p) for g in pats):
                break
        else:
            rows.append((p, None))
            unmatched = True
            continue
        rows.append((p, name))
        mode = max(mode, rmode, key=MODES.index)
        runtime = runtime or rt
    if not paths or unmatched:
        return "full", True, rows
    return mode, runtime, rows



def changed_paths(base, head):
    """Files the pull request changes: the diff of the test merge commit
    (head) against its first parent, main's commit (base). Needs both trees,
    not history (checkout with fetch-depth: 2)."""
    p = subprocess.run(["git", "-c", "safe.directory=*", "diff", "--name-only", "-z", "--no-renames", base, head],
                       cwd=ROOT, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "git diff failed")
    return [x for x in p.stdout.split("\0") if x]


def summary(mode, why, rows, gate):
    """The step summary. `gate`: the package regression gate runs (ci.yml runs
    it only on a pull request whose selection says runtime)."""
    heavy = "build, test, counters, corpora, fuzz, cran, smoke, perf"
    always = "lint, miri and bytecode-fidelity"
    pkg = "the package regression gate"
    if mode == "full":
        run = f"{heavy}, {pkg + ', ' if gate else ''}rt, {always}"
        skip = "nothing" if gate else pkg
    elif mode == "rt":
        run, skip = f"rt, {always}", f"{heavy} and {pkg}"
    else:
        run, skip = always, f"{heavy}, rt and {pkg}"
    lines = [f"### CI jobs: {mode}", "", why, "", f"- **run:** {run}", f"- **skip:** {skip}", ""]
    if rows:
        lines += ["| changed path | rule |", "|---|---|"]
        shown = rows if len(rows) <= 60 else rows[:60]
        lines += [f"| `{p}` | {r or '**no rule: full CI**'} |" for p, r in shown]
        if len(rows) > len(shown):
            lines.append(f"| … {len(rows) - len(shown)} more | |")
        lines.append("")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "pull_request"),
                    help="only pull_request narrows (default: $GITHUB_EVENT_NAME, else pull_request)")
    ap.add_argument("--base", default="HEAD^1", help="main's commit: the merge commit's first parent")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--paths-from", help="read changed paths from FILE (- for stdin) instead of git")
    ap.add_argument("--changed", nargs="+", help="changed paths, for a dry run")
    ap.add_argument("--config", default=CONFIG)
    ap.add_argument("--github-output", action="store_true")
    a = ap.parse_args()

    mode, runtime, rows, why = "full", True, [], ""
    try:
        rules = load(a.config)
        if a.changed is not None:
            paths = a.changed
        elif a.paths_from:
            f = sys.stdin if a.paths_from == "-" else open(a.paths_from)
            paths = [x.strip() for x in f.read().splitlines() if x.strip()]
        elif a.event != "pull_request":
            paths = None
        else:
            paths = changed_paths(a.base, a.head)
        if paths is None:
            why = f"Event `{a.event}` always runs every job (main records the full history)."
        else:
            mode, runtime, rows = decide(paths, rules)
            why = {
                "full": "No narrowing applies: the diff is empty, a path can change the binary or a gate, "
                        "or a path no rule covers.",
                "light": "Every changed path is a file no CI job reads (docs, repo metadata or hooks); "
                         "build-dependent heavy jobs are skipped.",
                "rt": "Every changed path is in G3's runtime crates or tests (or a file no CI job reads), "
                      "which the robigo binary does not link: the `rt` job runs and the heavy jobs are skipped.",
            }[mode]
    except Exception as e:  # fail safe: run everything
        mode, runtime, rows = "full", True, []
        why = f"Selection failed ({e}); running every job."
        print(f"ci-select: {e}", file=sys.stderr)

    print(summary(mode, why, rows, gate=runtime and mode == "full" and a.event == "pull_request"))
    if a.github_output and os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as f:
            heavy = "true" if mode == "full" else "false"
            rt = "false" if mode == "light" else "true"
            f.write(f"mode={mode}\nheavy={heavy}\nruntime={'true' if runtime and heavy == 'true' else 'false'}\n"
                    f"rt={rt}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
