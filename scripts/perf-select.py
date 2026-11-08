#!/usr/bin/env python3
"""Choose the benchmarks a pull request's perf gate runs, from its changed paths.

    scripts/perf-select.py [--event pull_request|push|...] [--base REF] [--head REF]
    scripts/perf-select.py --paths-from FILE|-          # dry run: paths, one per line
    scripts/perf-select.py --changed PATH [PATH ...]    # dry run
    scripts/perf-select.py ... --github-output          # also write $GITHUB_OUTPUT

The mapping is data: benchmarks/selection.toml (groups of benchmarks, rules
from path globs to groups). The result is `full` (every benchmark), `subset`
(`perf.py run --filter FILTER`) or `none` (no changed path can alter an
instruction count). Only the set of benchmarks narrows; the gate and its
threshold are scripts/perf.py's and unchanged.

Fail-safe: the run is `full` unless this is a pull request AND the changed
paths could be listed AND every changed path matches a rule. An unmatched
path, an unreadable diff, a malformed mapping or any other event (main,
workflow_dispatch) selects everything. Never silently nothing.

Prints a markdown summary (what runs and why) on stdout. `--github-output`
sets `mode`, `run` (true unless `none`) and `filter` for later steps.
"""
import argparse
import os
import re
import subprocess
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIG = os.path.join(ROOT, "benchmarks", "selection.toml")


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


def load(path=CONFIG):
    with open(path, "rb") as f:
        conf = tomllib.load(f)
    groups = conf["groups"]
    rules = []
    for r in conf["rules"]:
        sel = r["select"]
        for g in sel:
            if g not in ("all", "none") and g not in groups:
                raise ValueError(f"rule {r['name']!r} selects unknown group {g!r}")
        if ("all" in sel or "none" in sel) and len(sel) != 1:
            raise ValueError(f"rule {r['name']!r}: 'all'/'none' cannot be combined with groups")
        rules.append((r["name"], [glob_re(p) for p in r["paths"]], sel))
    for name, pats in groups.items():
        for p in pats:
            re.compile(p)
    return groups, rules


def decide(paths, groups, rules):
    """(mode, filter or None, [(path, rule name or None)])."""
    rows, selected, full, unmatched = [], [], False, False
    for p in paths:
        for name, pats, sel in rules:
            if any(g.match(p) for g in pats):
                break
        else:
            rows.append((p, None))
            unmatched = True
            continue
        rows.append((p, name))
        if sel == ["all"]:
            full = True
        elif sel != ["none"]:
            for g in sel:
                if g not in selected:
                    selected.append(g)
    if not paths:
        # Nothing changed (or nothing listed): not evidence that nothing matters.
        return "full", None, rows
    if full or unmatched:
        return "full", None, rows
    if not selected:
        return "none", None, rows
    pats = []
    for g in selected:
        for p in groups[g]:
            if p not in pats:
                pats.append(p)
    return "subset", "|".join(f"(?:{p})" for p in pats), rows


def changed_paths(base, head):
    """Files the pull request changes: the diff of the test merge commit
    (head) against its first parent, main's commit (base). Needs both trees,
    not history (checkout with fetch-depth: 2)."""
    p = subprocess.run(["git", "-c", "safe.directory=*", "diff", "--name-only", "-z", "--no-renames", base, head],
                       cwd=ROOT, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "git diff failed")
    return [x for x in p.stdout.split("\0") if x]


def summary(mode, flt, why, rows, groups):
    lines = [f"### Perf gate benchmarks: {mode}", "", why, ""]
    if mode == "subset":
        lines += ["Filter (`perf.py run --filter`):", "", f"    {flt}", ""]
    if rows:
        lines += ["| changed path | rule |", "|---|---|"]
        shown = rows if len(rows) <= 60 else rows[:60]
        lines += [f"| `{p}` | {r or '**no rule: full suite**'} |" for p, r in shown]
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
    ap.add_argument("--list", action="store_true", help="also list the benchmarks the filter selects")
    a = ap.parse_args()

    mode, flt, rows, why = "full", None, [], ""
    try:
        groups, rules = load(a.config)
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
            why = f"Event `{a.event}` always runs the full suite (it records the next baseline)."
        else:
            mode, flt, rows = decide(paths, groups, rules)
            why = {
                "full": "No narrowing applies: the diff is empty, touches a path the mapping sends to the full "
                        "suite, or a path no rule covers.",
                "subset": "Every changed path is covered by a rule; only the benchmarks those rules name run. "
                          "The gate and its 0.5% threshold are unchanged.",
                "none": "Every changed path is one no instruction count can depend on (docs, tests, other "
                        "workflows); no benchmark runs.",
            }[mode]
    except Exception as e:  # fail safe: measure everything
        mode, flt, rows = "full", None, []
        why = f"Selection failed ({e}); running the full suite."
        print(f"perf-select: {e}", file=sys.stderr)

    print(summary(mode, flt, why, rows, None))
    if a.list:
        sys.path.insert(0, os.path.join(ROOT, "scripts"))
        import perf
        names = [p[0] for p in perf.programs(perf.conf(), None, flt)]
        print(f"{len(names)} benchmark(s): " + ", ".join(names))
    if a.github_output and os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as f:
            f.write(f"mode={mode}\nrun={'false' if mode == 'none' else 'true'}\nfilter={flt or ''}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
