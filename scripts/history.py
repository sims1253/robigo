#!/usr/bin/env python3
"""Performance and compatibility over time: the README's charts and badges.

    scripts/history.py add HISTORY.jsonl DIR --commit SHA [--run ID] [--repo OWNER/NAME]
    scripts/history.py sync HISTORY.jsonl [--repo OWNER/NAME] [--limit N]
    scripts/history.py render HISTORY.jsonl --out SITE [--repo OWNER/NAME]

The history is one JSON line per commit on main, kept on the orphan branch
`progress-data` (never on main, so the repository does not grow by an
image per merge). A line holds what CI measured for that commit:

- `perf`: instruction counts of every benchmark (scripts/perf.py) for
  robigo and GNU R, and whether robigo's output was wrong;
- `corpora`: passing and scored cases of every corpus (scripts/corpus.py),
  and the conformance scripts under tests/r (crates/robigo-cli/tests/conformance.rs);
- `gaps`: the failing case ids of every corpus with at most GAPS_MAX of them;
- `packages`: the most used CRAN packages' own test suites against GNU R's
  outcome of every test block (scripts/package-suite.py), measured nightly,
  so only some commits have it;
- `builtins`: GNU R's .Primitive/.Internal functions robigo implements.

`add` reads a directory of CI artifacts (perf.json, corpus result files,
package-suite results, the robigo binary) and records one commit, merging
with what the history already holds for it. `sync` does the same for every
main CI run whose artifacts still exist and that the history lacks: the
backfill, and the repair when a CI run did not record itself. `render`
writes the site that CI publishes on GitHub Pages: an SVG of speed and one
of compatibility (light and dark), shields.io endpoint badges, and a page
with every benchmark and corpus.
"""
import argparse
import html
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_REPO = "sims1253/robigo"
ARTIFACTS = ["perf-main", "robigo", "corpus-*", "smoke"]

# Display order and names of the corpora; others follow alphabetically.
CORPORA = {
    "fastr": "FastR test snippets",
    "gnur-tests": "GNU R's regression tests",
    "gnur-examples": "Help-page examples",
    "gnur-demos": "GNU R's demos",
    "r-intro": "An Introduction to R",
    "cran-purer": "Pure-R CRAN packages",
    "packages": "Top CRAN packages' tests",
    "capi": "C API",
    "conformance": "Conformance scripts",
}
# The open gaps: a commit records the failing case ids of every corpus with
# at most this many, so the page can list them (known failures that agents
# add as unmarked conformance scripts or corpus cases show up here).
GAPS_MAX = 500
# Failure causes of the package suite kept per commit, most blocks first.
PACKAGE_CAUSES = 30
BUILTINS_LABEL = ".Primitive/.Internal builtins"
# Benchmark groups by name prefix, in display order.
GROUPS = [
    ("rbench/awf/", "Are We Fast Yet"),
    ("rbench/shootout/", "Benchmarks Game"),
    ("rbench/simple/", "Micro-benchmarks"),
    ("rbench/real/", "Real programs"),
    ("robigo/", "robigo's suite"),
]


# ------------------------------------------------------------------ data

def load(path):
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def save(path, entries):
    entries = sorted({e["sha"]: e for e in entries}.values(), key=lambda e: (e["date"], e["sha"]))
    with open(path, "w") as f:
        for e in entries:
            f.write(json.dumps(e, sort_keys=True, separators=(",", ":")) + "\n")


def title_of(message):
    """A merge commit's PR title (GitHub puts it after a blank line), else the subject."""
    lines = message.strip().split("\n")
    m = re.match(r"Merge pull request (#\d+)", lines[0])
    if m:
        body = [x for x in lines[1:] if x.strip()]
        return f"{m.group(1)}: {body[0].strip()}" if body else m.group(1)
    return lines[0]


def commit_meta(sha, repo):
    """(date, title) of a commit: from the local clone, else GitHub's API."""
    p = subprocess.run(["git", "-C", ROOT, "show", "-s", "--format=%cI%n%B", sha], capture_output=True, text=True)
    if p.returncode == 0:
        date, _, msg = p.stdout.partition("\n")
        return date.strip(), title_of(msg)
    c = json.loads(gh("api", f"repos/{repo}/commits/{sha}"))
    return c["commit"]["committer"]["date"], title_of(c["commit"]["message"])


def gh(*args):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=True).stdout


def builtin_names():
    """names.c's table, counted per entry as scripts/builtin-coverage.py does."""
    with open(os.path.join(ROOT, "scripts", "names-4.5.0.tsv")) as f:
        return [line.split("\t")[0] for line in f if line.strip()]


def packages_from(docs, install):
    """The package suite's result files of one run, as one history value.

    Complete only if every shard reported. A run with a broken canary, or an
    incomplete one, is recorded but sets no score: a missing shard must not
    look like a smaller denominator.
    """
    n = docs[0]["shard"][1]
    complete = {d["shard"][0] for d in docs} == set(range(1, n + 1))
    valid = all(d["valid"] for d in docs)
    pkgs = {k: v for d in docs for k, v in d["pkgs"].items()}
    causes = {}
    for d in docs:
        for why, count, names in d["causes"]:
            c = causes.setdefault(why, [0, set()])
            c[0] += count
            c[1] |= set(names)
    out = {"snapshot": docs[0]["snapshot"], "lock": docs[0]["lock"], "selection": docs[0]["selection"],
           "valid": valid, "complete": complete,
           "canary": [sum(d["canary"][0] for d in docs), sum(d["canary"][1] for d in docs)], "pkgs": pkgs,
           "score": [sum(p["m"] for p in pkgs.values()), sum(p["n"] for p in pkgs.values())],
           "green": sum(1 for p in pkgs.values() if p["green"]),
           "regressions": sum(len(d["regressions"]) for d in docs),
           "causes": [[w, c[0], sorted(c[1])] for w, c in
                      sorted(causes.items(), key=lambda kv: (-kv[1][0], kv[0]))[:PACKAGE_CAUSES]]}
    for d in docs:
        if "loads" in d:
            out["loads"] = [len(d["loads"]), len(d["closure"])]
    if install:
        out["installs"] = [sum(1 for v in install["state"].values() if v == "ok"), len(install["state"])]
    return out


def entry_from(d, sha, date, title, run=None):
    """One history line from a directory of CI artifacts (any layout)."""
    e = {"sha": sha, "date": date, "title": title}
    if run:
        e["run"] = int(run)
    corpora, shards, failing = {}, {}, {}
    package_docs, own_package_docs, package_install = [], [], None
    for top, _, files in os.walk(d):
        for fn in files:
            path = os.path.join(top, fn)
            if fn == "perf.json":
                with open(path) as f:
                    doc = json.load(f)
                perf = {}
                for name, r in doc["results"].items():
                    x = {rt: r[rt]["instr"] for rt in ("robigo", "gnur") if "instr" in r.get(rt, {})}
                    if "error" in r.get("robigo", {}):
                        x["error"] = True
                    if r.get("wrong"):
                        x["wrong"] = True
                    perf[name] = x
                e["perf"] = perf
            elif fn.endswith(".json"):
                with open(path) as f:
                    try:
                        doc = json.load(f)
                        s = doc.get("summary")
                    except (ValueError, AttributeError):
                        continue
                if doc.get("kind") == "packages":
                    package_docs.append(doc)
                    continue
                if doc.get("kind") == "own-packages":
                    own_package_docs.append(doc)
                    continue
                if doc.get("kind") == "package-install":
                    package_install = doc
                    continue
                if doc.get("kind") == "smoke":
                    e["smoke"] = {"pass": doc["pass"], "scored": doc["scored"],
                                  "cases": {k: v["state"] for k, v in doc["cases"].items()},
                                  "pending": doc.get("pending", [])}
                    continue
                # `corpus.py run --tier` results check the same cases again.
                if not isinstance(s, dict) or "corpus" not in s or s.get("tier"):
                    continue
                c = corpora.setdefault(s["corpus"], [0, 0])
                c[0] += s["pass"]
                c[1] += s["cases"]
                results = doc.get("results")
                if isinstance(results, list):
                    failing.setdefault(s["corpus"], []).extend(
                        r["id"] for r in results if isinstance(r, dict) and "id" in r and not r.get("pass"))
                m = re.search(r"\.shard(\d+)of(\d+)\.json$", fn)
                i, n = (int(m.group(1)), int(m.group(2))) if m else (1, 1)
                shards.setdefault(s["corpus"], [n, set()])[1].add(i)
            elif fn == "robigo" and "builtins" not in e:
                os.chmod(path, 0o755)
                p = subprocess.run([path, "--list-builtins"], capture_output=True, text=True)
                if p.returncode == 0:
                    have = {line.split("\t")[0] for line in p.stdout.splitlines() if line}
                    names = builtin_names()
                    e["builtins"] = [sum(1 for x in names if x in have), len(names)]
    # A corpus counts only if every shard reported.
    done = {k: v for k, v in corpora.items() if len(shards[k][1]) == shards[k][0]}
    if done:
        e["corpora"] = done
        # Only when the result files list every case (older ones may not).
        gaps = {k: sorted(failing.get(k, [])) for k, (p, n) in done.items()
                if len(failing.get(k, [])) == n - p and 0 < n - p <= GAPS_MAX}
        if gaps:
            e["gaps"] = gaps
    if package_docs:
        pk = e["packages"] = packages_from(package_docs, package_install)
        if pk["complete"] and pk["valid"] and pk["score"][1]:
            e.setdefault("corpora", {})["packages"] = pk["score"]
    if own_package_docs:
        pk = e["own_packages"] = packages_from(own_package_docs, None)
        pk["excluded"] = {}
        for d in own_package_docs:
            pk["excluded"].update(d.get("excluded", {}))
    return e


def merge_entry(old, new):
    """What CI measured of one commit at different times: the nightly package
    suite and the commit's own run arrive in either order."""
    out = {**old, **new}
    if "corpora" in old and "corpora" in new:
        out["corpora"] = {**old["corpora"], **new["corpora"]}
    # a broken or partial nightly does not replace a good one of the commit
    if "packages" in old and "packages" in new and old["packages"]["valid"] and old["packages"]["complete"] \
            and not (new["packages"]["valid"] and new["packages"]["complete"]):
        out["packages"] = old["packages"]
    if "own_packages" in old and "own_packages" in new and old["own_packages"].get("valid") and old["own_packages"].get("complete") \
            and not (new["own_packages"].get("valid") and new["own_packages"].get("complete")):
        out["own_packages"] = old["own_packages"]
    return out


def cmd_add(a):
    entries = load(a.history)
    date, title = commit_meta(a.commit, a.repo)
    e = entry_from(a.dir, a.commit, date, title, a.run)
    if not ({"perf", "corpora", "builtins", "packages", "own_packages", "smoke"} & e.keys()):
        sys.exit(f"nothing to record in {a.dir}")
    old = next((x for x in entries if x["sha"] == a.commit), None)
    if old:
        e = merge_entry(old, e)
    save(a.history, [x for x in entries if x["sha"] != a.commit] + [e])
    print(f"recorded {a.commit[:12]}: " + ", ".join(k for k in ("perf", "corpora", "builtins", "packages", "own_packages", "smoke") if k in e))


def cmd_sync(a):
    entries = load(a.history)
    # a commit the nightly package suite recorded still lacks its own run
    have = {e["sha"] for e in entries if {"perf", "builtins"} & e.keys()}
    runs = json.loads(gh("api", f"repos/{a.repo}/actions/workflows/ci.yml/runs?branch=main&status=completed"
                                f"&per_page={min(a.limit, 100)}", "--jq", ".workflow_runs"))
    added = 0
    for r in runs[:a.limit]:
        if r["head_branch"] != "main" or r["event"] not in ("push", "workflow_dispatch") or r["head_sha"] in have:
            continue
        with tempfile.TemporaryDirectory(prefix="robigo-history-") as tmp:
            pats = [x for p in ARTIFACTS for x in ("-p", p)]
            p = subprocess.run(["gh", "run", "download", str(r["id"]), "-R", a.repo, "-D", tmp, *pats],
                               capture_output=True, text=True)
            if p.returncode != 0:
                print(f"{r['head_sha'][:12]} (run {r['id']}): no artifacts left", file=sys.stderr)
                continue
            date, title = commit_meta(r["head_sha"], a.repo)
            e = entry_from(tmp, r["head_sha"], date, title, r["id"])
        if not ({"perf", "corpora", "builtins"} & e.keys()):
            continue
        old = next((x for x in entries if x["sha"] == e["sha"]), None)
        if old:
            e = merge_entry(old, e)
            entries.remove(old)
        entries.append(e)
        have.add(e["sha"])
        added += 1
        print(f"recorded {e['sha'][:12]} (run {r['id']}): " +
              ", ".join(k for k in ("perf", "corpora", "builtins") if k in e))
    save(a.history, entries)
    print(f"{added} commit(s) added, {len(have)} in the history")


# --------------------------------------------------------------- metrics

def geomean(xs):
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else None


def speedups(perf, prefix=""):
    """GNU R instructions / robigo instructions of every correct program."""
    return {k: v["gnur"] / v["robigo"] for k, v in perf.items()
            if k.startswith(prefix) and v.get("robigo") and v.get("gnur") and not v.get("wrong")
            and not v.get("error")}


def series(entries):
    """Per-metric time series: name -> [(index, value, count text)]."""
    out = {"speed": []}
    for i, e in enumerate(entries):
        if "perf" in e:
            s = speedups(e["perf"])
            if s:
                out["speed"].append((i, geomean(list(s.values())), f"{len(s)} of {len(e['perf'])} programs"))
            for prefix, label in GROUPS:
                s = speedups(e["perf"], prefix)
                if s:
                    n = sum(1 for k in e["perf"] if k.startswith(prefix))
                    out.setdefault("group:" + label, []).append((i, geomean(list(s.values())), f"{len(s)} of {n}"))
        for k, (p, n) in e.get("corpora", {}).items():
            if n:
                what = f"{p:,} of {n:,} tests, snapshot {e['packages']['snapshot']}" if k == "packages" \
                    else f"{p:,} of {n:,} cases"
                out.setdefault("corpus:" + k, []).append((i, p / n, what))
        if "builtins" in e:
            p, n = e["builtins"]
            out.setdefault("builtins", []).append((i, p / n, f"{p} of {n}"))
    return out


def corpus_order(names):
    known = [k for k in CORPORA if k in names]
    return known + sorted(set(names) - set(known))


# ------------------------------------------------------------------- SVG
# Colors: the dataviz reference palette (light and dark steps of one blue,
# recessive ink and hairlines). One series per plot, so no legends.

THEMES = {
    "light": {"surface": "#fcfcfb", "border": "#e1e0d9", "ink": "#0b0b0b", "ink2": "#52514e", "muted": "#898781",
              "grid": "#e1e0d9", "axis": "#c3c2b7", "line": "#2a78d6", "wash": "#2a78d6"},
    "dark": {"surface": "#1a1a19", "border": "#2c2c2a", "ink": "#f0efec", "ink2": "#c3c2b7", "muted": "#898781",
             "grid": "#2c2c2a", "axis": "#383835", "line": "#3987e5", "wash": "#3987e5"},
}
FONT = "system-ui, -apple-system, 'Segoe UI', Helvetica, Arial, sans-serif"
E = html.escape


def fmt_x(v):
    return f"{v:.2f}×" if v < 10 else f"{v:.1f}×"


def fmt_pct(v):
    return f"{100 * v:.1f}%"


class LogScale:
    def __init__(self, values):
        lo, hi = min(values + [1.0]), max(values + [1.0])
        self.lo, self.hi = lo / 1.15, hi * 1.15
        cands = []
        for step in ([1], [1, 2, 5], [1, 1.5, 2, 3, 5, 7]):
            cands = [m * 10 ** d for d in range(-3, 4) for m in step if self.lo <= m * 10 ** d <= self.hi]
            if len(cands) >= 3:
                break
        while len(cands) > 6:
            cands = cands[::2] if 1.0 not in cands[1::2] else cands[1::2]
        self.ticks = cands

    def frac(self, v):
        return (math.log(v) - math.log(self.lo)) / (math.log(self.hi) - math.log(self.lo))


class PctScale:
    ticks = [0, 0.5, 1]

    def frac(self, v):
        return v


def tick_label(scale, v):
    if isinstance(scale, PctScale):
        return f"{int(100 * v)}%"
    return (f"{v:g}" if v >= 1 else f"{v:g}") + "×"


def plot(t, x, y, w, h, n, pts, scale, fmt, hover, ref=None, ytick_labels=True):
    """One plot area at (x, y) of w×h: hairline grid at the scale's ticks, an
    optional reference line, the series as a 2px line with an end dot and an
    end label. pts are (index of n, value, detail)."""
    out = []
    px = (lambda i: x + w * i / (n - 1)) if n > 1 else (lambda i: x + w / 2)
    py = lambda v: y + h * (1 - scale.frac(v))
    for v in scale.ticks:
        if v == ref:
            continue
        out.append(f'<line x1="{x}" x2="{x + w}" y1="{py(v):.1f}" y2="{py(v):.1f}" stroke="{t["grid"]}" '
                   f'stroke-width="1"/>')
        if ytick_labels:
            out.append(f'<text x="{x - 6}" y="{py(v) + 4:.1f}" text-anchor="end" font-size="11" '
                       f'fill="{t["muted"]}">{tick_label(scale, v)}</text>')
    if ref is not None:
        out.append(f'<line x1="{x}" x2="{x + w}" y1="{py(ref):.1f}" y2="{py(ref):.1f}" stroke="{t["axis"]}" '
                   f'stroke-width="1"/>')
        if ytick_labels:
            out.append(f'<text x="{x - 6}" y="{py(ref) + 4:.1f}" text-anchor="end" font-size="11" '
                       f'fill="{t["muted"]}">{tick_label(scale, ref)}</text>')
            out.append(f'<text x="{x + 4}" y="{py(ref) - 5:.1f}" font-size="11" fill="{t["muted"]}">GNU R</text>')
    if not pts:
        out.append(f'<text x="{x + w / 2}" y="{y + h / 2}" text-anchor="middle" font-size="11" '
                   f'fill="{t["muted"]}">no data yet</text>')
        return "\n".join(out)
    coords = [(px(i), py(v)) for i, v, _ in pts]
    if len(coords) > 1:
        d = " ".join(f"{'M' if k == 0 else 'L'}{cx:.1f},{cy:.1f}" for k, (cx, cy) in enumerate(coords))
        area = d + f" L{coords[-1][0]:.1f},{y + h} L{coords[0][0]:.1f},{y + h} Z"
        out.append(f'<path d="{area}" fill="{t["wash"]}" fill-opacity="0.10" stroke="none"/>')
        out.append(f'<path d="{d}" fill="none" stroke="{t["line"]}" stroke-width="2" stroke-linejoin="round" '
                   f'stroke-linecap="round"/>')
    ex, ey = coords[-1]
    out.append(f'<circle cx="{ex:.1f}" cy="{ey:.1f}" r="4" fill="{t["line"]}" stroke="{t["surface"]}" '
               f'stroke-width="2"/>')
    out.append(f'<text x="{ex + 8:.1f}" y="{ey + 4:.1f}" font-size="12" font-weight="600" '
               f'fill="{t["ink"]}">{fmt(pts[-1][1])}</text>')
    if hover is not None:
        data = [[round(cx, 1), round(cy, 1), hover(i, v, detail)] for (cx, cy), (i, v, detail) in zip(coords, pts)]
        out.append(f'<g class="hover" data-y0="{y}" data-y1="{y + h}" data-pts="{E(json.dumps(data))}"></g>')
    return "\n".join(out)


def x_ticks(t, entries, x, w, y, max_ticks=6):
    """Dates under the plot: the first commit of each day, thinned."""
    n = len(entries)
    days, seen = [], set()
    for i, e in enumerate(entries):
        d = e["date"][:10]
        if d not in seen:
            seen.add(d)
            days.append((i, d))
    step = max(1, math.ceil(len(days) / max_ticks))
    out = []
    for i, d in days[::step]:
        cx = x + w * i / (n - 1) if n > 1 else x + w / 2
        label = datetime.strptime(d, "%Y-%m-%d").strftime("%b %-d")
        out.append(f'<text x="{cx:.1f}" y="{y}" text-anchor="middle" font-size="11" fill="{t["muted"]}">'
                   f'{label}</text>')
    return "\n".join(out)


def svg_doc(t, w, h, body, standalone):
    bg = (f'<rect x="0.5" y="0.5" width="{w - 1}" height="{h - 1}" rx="8" fill="{t["surface"]}" '
          f'stroke="{t["border"]}"/>')
    ns = ' xmlns="http://www.w3.org/2000/svg"' if standalone else ""
    return (f'<svg{ns} class="chart" viewBox="0 0 {w} {h}" width="{w}" height="{h}" font-family="{E(FONT)}" '
            f'style="font-variant-numeric: tabular-nums" role="img">\n{bg}\n{body}\n</svg>\n')


def hover_text(entries):
    def f(fmt):
        return lambda i, v, detail: (f"{entries[i]['sha'][:7]} · {entries[i]['date'][:10]} · {fmt(v)} "
                                     f"({detail}) · {entries[i]['title']}")
    return f


def speed_svg(entries, ser, t, standalone=True):
    W, H = 720, 296
    x, y, w, h = 56, 92, W - 56 - 64, H - 92 - 40
    pts = ser["speed"]
    last = pts[-1] if pts else None
    head = (f'<text x="24" y="32" font-size="16" font-weight="600" fill="{t["ink"]}">'
            f'Speed: robigo vs GNU R 4.5.0</text>'
            f'<text x="24" y="54" font-size="12" fill="{t["ink2"]}">GNU R instructions ÷ robigo instructions '
            f'(callgrind), geometric mean. Higher is faster.</text>'
            f'<text x="24" y="70" font-size="12" fill="{t["ink2"]}">Only programs whose output matches GNU R\'s '
            f'count{f": {E(last[2])}" if last else ""}.</text>')
    scale = LogScale([v for _, v, _ in pts])
    body = [head, plot(t, x, y, w, h, len(entries), pts, scale, fmt_x,
                       None if standalone else hover_text(entries)(fmt_x), ref=1.0),
            x_ticks(t, entries, x, w, y + h + 20)]
    return svg_doc(t, W, H, "\n".join(body), standalone)


def grid_svg(entries, panels, scale, fmt, title, subtitle, t, standalone=True, cols=4, ref=None):
    """Small multiples on one shared scale: panels are (label, pts)."""
    W = 720
    pw, ph, gap_x, top = (W - 48 - (cols - 1) * 24) / cols, 64, 24, 72
    rows = math.ceil(len(panels) / cols)
    row_h = ph + 80
    H = top + rows * row_h + 4
    body = [f'<text x="24" y="32" font-size="16" font-weight="600" fill="{t["ink"]}">{E(title)}</text>',
            f'<text x="24" y="54" font-size="12" fill="{t["ink2"]}">{E(subtitle)}</text>']
    hv = None if standalone else hover_text(entries)(fmt)
    for k, (label, pts) in enumerate(panels):
        r, c = divmod(k, cols)
        x = 24 + c * (pw + gap_x)
        y = top + r * row_h
        val = f"{fmt(pts[-1][1])}" if pts else "–"
        detail = pts[-1][2] if pts else ""
        body.append(f'<text x="{x:.1f}" y="{y + 14}" font-size="12" fill="{t["ink2"]}">{E(label)}</text>')
        body.append(f'<text x="{x:.1f}" y="{y + 35}" font-size="18" font-weight="600" fill="{t["ink"]}">{val}'
                    f'</text>')
        body.append(f'<text x="{x:.1f}" y="{y + 51}" font-size="11" fill="{t["muted"]}">{E(detail)}</text>')
        # no end label in a panel: the figure above says it
        body.append(plot(t, x, y + 62, pw - 8, ph, len(entries), pts, scale, lambda v: "", hv, ref=ref,
                         ytick_labels=False))
    return svg_doc(t, W, H, "\n".join(body), standalone)


def compat_panels(entries, ser):
    names = corpus_order({k.split(":", 1)[1] for k in ser if k.startswith("corpus:")})
    panels = [(CORPORA.get(k, k), ser[f"corpus:{k}"]) for k in names]
    panels.append((BUILTINS_LABEL, ser.get("builtins", [])))
    return panels


def compat_svg(entries, ser, t, standalone=True):
    return grid_svg(entries, compat_panels(entries, ser), PctScale(), fmt_pct,
                    "Compatibility with GNU R 4.5.0",
                    "Share of cases whose stdout, stderr and exit status match GNU R's (each panel 0-100%); "
                    "builtins implemented.", t, standalone)


def groups_svg(entries, ser, t, standalone=True):
    panels = [(label, ser.get("group:" + label, [])) for _, label in GROUPS]
    vals = [v for _, pts in panels for _, v, _ in pts]
    return grid_svg(entries, panels, LogScale(vals), fmt_x, "Speed by benchmark suite",
                    "GNU R instructions ÷ robigo instructions, geometric mean; one shared log scale, "
                    "the gray line is GNU R.", t, standalone, cols=5, ref=1.0)


# ------------------------------------------------------------------ site

def badge(label, message):
    return {"schemaVersion": 1, "label": label, "message": message, "color": "2a78d6", "cacheSeconds": 3600}


def sparkline(pts, n, log=True):
    """`log` for speedups; shares (which can be 0) go on a linear 0..1 scale."""
    if not pts:
        return ""
    w, h = 96, 20
    f = math.log if log else (lambda v: v)
    if log:
        lo, hi = min(v for _, v, _ in pts), max(v for _, v, _ in pts)
        # Each line on its own log range, but at least 1.25×, so noise stays flat.
        lo, hi = math.log(lo), math.log(hi)
        pad = max(0.0, math.log(1.25) - (hi - lo)) / 2
        lo, hi = lo - pad, hi + pad
    else:
        lo, hi = 0.0, 1.0
    xy = [(w * i / max(n - 1, 1), h - 2 - (h - 4) * (f(v) - lo) / (hi - lo)) for i, v, _ in pts]
    d = " ".join(f"{'M' if k == 0 else 'L'}{x:.1f},{y:.1f}" for k, (x, y) in enumerate(xy))
    return (f'<svg width="{w}" height="{h}" viewBox="0 0 {w} {h}"><path d="{d}" fill="none" '
            f'stroke="var(--line)" stroke-width="1.5" stroke-linejoin="round"/></svg>')


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>robigo: progress</title>
<style>
:root {{ color-scheme: light; --surface: #fcfcfb; --plane: #f9f9f7; --ink: #0b0b0b; --ink2: #52514e;
  --muted: #898781; --grid: #e1e0d9; --line: #2a78d6; }}
@media (prefers-color-scheme: dark) {{
  :root {{ color-scheme: dark; --surface: #1a1a19; --plane: #0d0d0d; --ink: #f0efec; --ink2: #c3c2b7;
    --muted: #898781; --grid: #2c2c2a; --line: #3987e5; }}
  .light {{ display: none; }}
}}
@media not (prefers-color-scheme: dark) {{ .dark {{ display: none; }} }}
body {{ margin: 0; background: var(--plane); color: var(--ink); font: 14px/1.5 {font}; }}
main {{ max-width: 760px; margin: 0 auto; padding: 32px 20px 64px; }}
h1 {{ font-size: 22px; margin: 0 0 4px; }}
h2 {{ font-size: 17px; margin: 40px 0 12px; }}
p, .meta {{ color: var(--ink2); }}
a {{ color: inherit; }}
figure {{ margin: 16px 0; position: relative; }}
svg.chart {{ max-width: 100%; height: auto; display: block; }}
table {{ border-collapse: collapse; width: 100%; font-variant-numeric: tabular-nums; }}
.scroll {{ overflow-x: auto; }}
th, td {{ text-align: left; padding: 4px 6px; border-bottom: 1px solid var(--grid); white-space: nowrap; }}
th {{ color: var(--ink2); font-weight: 600; }}
td.n, th.n {{ text-align: right; }}
td.note {{ color: var(--muted); }}
.tip {{ position: absolute; pointer-events: none; background: var(--surface); color: var(--ink);
  border: 1px solid var(--grid); border-radius: 6px; padding: 6px 8px; font-size: 12px; max-width: 360px;
  box-shadow: 0 2px 8px rgba(0,0,0,.12); display: none; white-space: normal; }}
details summary {{ cursor: pointer; color: var(--ink2); }}
ul.gaps {{ columns: 2 280px; margin: 8px 0 16px; padding-left: 20px; font-size: 13px; }}
</style>
</head>
<body>
<main>
<h1>robigo: progress</h1>
<div class="meta">GNU R 4.5.0 runtime in Rust · last commit <a href="{repo_url}/commit/{sha}"><code>{sha7}</code></a>
({date}) · <a href="{repo_url}">repository</a> · <a href="history.jsonl">raw data</a></div>
<p>Every merge to <code>main</code> is measured by CI (see <a href="{repo_url}/blob/main/docs/verification.md">
docs/verification.md</a>). Speed counts instructions with callgrind, which is exact and repeatable on a shared runner;
a program whose output differs from GNU R's does not count.</p>
<h2>Speed</h2>
{speed}
{groups}
<h2>Compatibility</h2>
{compat}
{packages}
<h2>Benchmarks at <code>{sha7}</code></h2>
<div class="scroll"><table>
<tr><th>benchmark</th><th class="n">vs GNU R</th><th>trend</th><th class="n">robigo instr</th>
<th class="n">GNU R instr</th><th></th></tr>
{bench_rows}
</table></div>
<h2>Corpora at <code>{sha7}</code></h2>
<table>
<tr><th>corpus</th><th class="n">passing</th><th class="n">cases</th><th class="n">share</th></tr>
{corpus_rows}
</table>
<h2>Open gaps at <code>{gap_sha7}</code></h2>
<p>Cases robigo still fails, per corpus with at most a few hundred. A known gap is a conformance script or corpus
case with GNU R's recorded output and no ratchet marker yet: CI tracks it without failing on it.</p>
{gaps}
<h2>Every commit</h2>
<details><summary>{n_commits} commits</summary>
<div class="scroll"><table>
<tr><th>commit</th><th>date</th><th class="n">speed</th>{corpus_heads}<th class="n">builtins</th></tr>
{history_rows}
</table></div>
</details>
</main>
<script>
// Crosshair and tooltip: the nearest commit to the pointer in each plot.
for (const fig of document.querySelectorAll("figure")) {{
  const tip = fig.querySelector(".tip");
  for (const svg of fig.querySelectorAll("svg.chart")) {{
    const groups = [...svg.querySelectorAll("g.hover")].map(g => ({{
      g, y0: +g.dataset.y0, y1: +g.dataset.y1, pts: JSON.parse(g.dataset.pts) }}));
    const ns = "http://www.w3.org/2000/svg";
    const mk = (tag, attrs) => {{ const e = document.createElementNS(ns, tag);
      for (const k in attrs) e.setAttribute(k, attrs[k]); return e; }};
    const line = mk("line", {{ stroke: "currentColor", "stroke-opacity": "0.35", "stroke-width": 1 }});
    const dot = mk("circle", {{ r: 4, fill: "var(--line)", stroke: "var(--surface)", "stroke-width": 2 }});
    line.style.display = dot.style.display = "none";
    svg.append(line, dot);
    svg.addEventListener("mousemove", ev => {{
      const p = new DOMPoint(ev.clientX, ev.clientY).matrixTransform(svg.getScreenCTM().inverse());
      let best = null;
      for (const gr of groups) {{
        if (p.y < gr.y0 - 40 || p.y > gr.y1 + 24) continue;
        for (const q of gr.pts) {{
          const d = Math.abs(q[0] - p.x);
          if (d < 40 && (!best || d < best.d)) best = {{ d, q, gr }};
        }}
      }}
      if (!best) {{ line.style.display = dot.style.display = tip.style.display = "none"; return; }}
      const [x, y, text] = best.q;
      Object.entries({{ x1: x, x2: x, y1: best.gr.y0, y2: best.gr.y1 }}).forEach(([k, v]) => line.setAttribute(k, v));
      dot.setAttribute("cx", x); dot.setAttribute("cy", y);
      line.style.display = dot.style.display = "";
      tip.textContent = text; tip.style.display = "block";
      const r = svg.getBoundingClientRect(), f = fig.getBoundingClientRect(), s = r.width / svg.viewBox.baseVal.width;
      const left = r.left - f.left + x * s + 12, top = r.top - f.top + y * s + 12;
      tip.style.left = Math.min(left, f.width - tip.offsetWidth) + "px"; tip.style.top = top + "px";
    }});
    svg.addEventListener("mouseleave", () => {{ line.style.display = dot.style.display = tip.style.display = "none"; }});
  }}
}}
</script>
</body>
</html>
"""


def gap_path(corpus, cid):
    """The repository file of a failing case, or None (cases.jsonl corpora)."""
    for p in (cid, f"tests/corpora/{corpus}/cases/{cid}.R"):
        if p.endswith(".R") and os.path.isfile(os.path.join(ROOT, p)):
            return p
    return None


def trusted(pk):
    """A run whose numbers count: the canary held and every shard reported."""
    return bool(pk.get("valid") and pk.get("complete"))


def suite_panel(entries, key, heading, about, notes=(), extra=""):
    """One package group's panel (`key`: "packages" or "own_packages").

    The headline score and the trends come from trusted runs only; the
    latest run's table is shown either way, under a note when it is not
    trusted. Green counts are out of the declared selection, so a package
    that did not report is not a smaller denominator."""
    measured = [(i, e) for i, e in enumerate(entries) if key in e]
    if not measured:
        return ""
    e, pk = measured[-1][1], measured[-1][1][key]
    good = [(i, x[key]) for i, x in measured if trusted(x[key])]
    rows = []
    for name, p in sorted(pk["pkgs"].items(), key=lambda kv: (-(kv[1]["m"] / (kv[1]["n"] or 1)), kv[0])):
        hist = [(i, x["pkgs"][name]["m"] / x["pkgs"][name]["n"], "") for i, x in good
                if x["pkgs"].get(name, {}).get("n")]
        unfinished = sum(v for k, v in p["files"].items() if k != "done")
        state = "green" if p["green"] else "no tests scored" if not p["n"] else \
            f"{unfinished} of {sum(p['files'].values())} files unfinished" if unfinished else ""
        ratio = f'{p["s"] / p["gnu_s"]:.1f}×' if p["gnu_s"] else ""
        rows.append(f'<tr><td>{E(name)} <span class="meta">{E(p["v"])}</span></td><td class="n">{p["m"]:,}</td>'
                    f'<td class="n">{p["n"]:,}</td><td class="n">{fmt_pct(p["m"] / p["n"]) if p["n"] else ""}</td>'
                    f'<td>{sparkline(hist, len(entries), log=False)}</td>'
                    f'<td class="n">{ratio}</td><td class="note">{E(state)}</td></tr>')
    causes = "".join(f'<tr><td class="n">{c:,}</td><td>{E(", ".join(names))}</td>'
                     f'<td style="white-space:normal">{E(why[:200])}</td></tr>' for why, c, names in pk["causes"])
    m, n = pk["score"]
    warn = []
    if not pk["valid"]:
        warn.append(f"The harness's canary failed ({pk['canary'][0]} of {pk['canary'][1]}): "
                    "these numbers are not trustworthy and set no score.")
    if not pk["complete"]:
        warn.append("Some shards did not report: this run is incomplete and sets no score.")
    if pk.get("regressions"):
        warn.append(f"{pk['regressions']} tests regressed against the ratchet.")
    if trusted(pk):
        score = f"<strong>{fmt_pct(m / n) if n else 'no score'}</strong> of {n:,} test blocks"
    else:
        score = "<strong>no score</strong> (see below) for the test blocks"
    head = f"{score} {about}, and <strong>{pk['green']} of {pk['selection']}</strong> packages are fully green"
    return f"""<h2>{heading} at <code>{e["sha"][:7]}</code></h2>
<p>{head}{"".join(notes)}</p>
{"".join(f"<p><strong>{E(x)}</strong></p>" for x in warn)}
{extra}
<div class="scroll"><table>
<tr><th>package</th><th class="n">matching</th><th class="n">scored</th><th class="n">share</th><th>trend</th>
<th class="n">time vs GNU R</th><th></th></tr>
{"".join(rows)}
</table></div>
{('<details><summary>What fails most</summary><div class="scroll"><table><tr><th class="n">tests</th><th>packages</th>'
  '<th>first failure</th></tr>' + causes + '</table></div></details>') if causes else ''}"""


def own_packages_section(entries, repo_url):
    """The user's own packages, scored as a group of their own."""
    last = next((e["own_packages"] for e in reversed(entries) if "own_packages" in e), None)
    if last is None:
        return ""
    excluded = last.get("excluded", {})
    extra = ""
    if excluded:
        items = "".join(f"<li><code>{E(k)}</code>: {E(v)}</li>" for k, v in sorted(excluded.items()))
        extra = f'<details><summary>{len(excluded)} own package(s) not run in CI</summary><ul class="gaps">{items}</ul></details>'
    notes = [f""". Pinned by git commit, scored and ratcheted as a group separate from the CRAN selection (see
<a href="{repo_url}/blob/main/tests/packages/README.md">tests/packages/README.md</a>)."""]
    return suite_panel(entries, "own_packages", "Own R packages",
                       "of the user's own R packages (dogfooding) behave as under GNU R", notes, extra)


def packages_section(entries, repo_url):
    """The package suite: what is green, what is close, and what blocks the rest."""
    last = next((e["packages"] for e in reversed(entries) if "packages" in e), None)
    cran = ""
    if last is not None:
        notes = []
        if "loads" in last:
            notes.append(f"; {last['loads'][0]} of {last['loads'][1]} packages of the dependency tree load")
        if "installs" in last:
            notes.append(f"; {last['installs'][0]} of {last['installs'][1]} install from source")
        notes.append(f""".
Packages with over 900,000 downloads a month and over 500 packages using them, at PPM snapshot {E(last["snapshot"])}.
Their own test suites run nightly under real testthat and tinytest. A block counts when its outcome matches GNU R 4.5.0's
recording, and a package is green when every block does (see
<a href="{repo_url}/blob/main/docs/verification.md#top-cran-packages">docs/verification.md</a>).""")
        previous = next((e["packages"]["snapshot"] for e in reversed(entries)
                         if "packages" in e and e["packages"]["snapshot"] != last["snapshot"]), None)
        if previous:
            notes.append(f" The packages moved from snapshot {E(previous)} to {E(last['snapshot'])}: "
                         "new versions bring new tests, so the score is not comparable across that step.")
        cran = suite_panel(entries, "packages", "Top CRAN packages", "behave as under GNU R", notes)
    return cran + own_packages_section(entries, repo_url) + smoke_section(entries, repo_url)


def smoke_section(entries, repo_url):
    """The smoke tier: user scripts whose whole transcript must match
    GNU R's, run on every pull request (tests/packages/smoke)."""
    measured = [x for x in entries if "smoke" in x]
    if not measured:
        return ""
    sm = measured[-1]["smoke"]
    rows = "".join(f'<tr><td>{E(name)}</td><td class="note">{E(state)}</td></tr>'
                   for name, state in sorted(sm["cases"].items()))
    return f"""
<h3>Package scripts as users run them</h3>
<p><strong>{sm["pass"]} of {sm["scored"]}</strong> smoke scripts produce GNU R 4.5.0's exact stdout, stderr
and exit status on the same pinned library. They run on every pull request in minutes, where the suites
above are nightly; a script counts only when the whole transcript matches, so this is the
"can I just use it" number. Cases that have never passed are pending gaps, not regressions
(see <a href="{repo_url}/blob/main/docs/verification.md#top-cran-packages">docs/verification.md</a>).</p>
<div class="scroll"><table>
<tr><th>script</th><th>state</th></tr>
{rows}
</table></div>"""


def figure(make, *args):
    return ('<figure><div class="light">' + make(*args, THEMES["light"], False) + '</div><div class="dark">' +
            make(*args, THEMES["dark"], False) + '</div><div class="tip"></div></figure>')


def cmd_render(a):
    entries = load(a.history)
    if not entries:
        sys.exit(f"{a.history} is empty")
    os.makedirs(os.path.join(a.out, "badges"), exist_ok=True)
    ser = series(entries)
    for mode, t in THEMES.items():
        suffix = "" if mode == "light" else "-dark"
        for name, make in (("speed", speed_svg), ("compat", compat_svg)):
            with open(os.path.join(a.out, f"{name}{suffix}.svg"), "w") as f:
                f.write(make(entries, ser, t))
    shutil.copy(a.history, os.path.join(a.out, "history.jsonl"))

    badges = {}
    if ser["speed"]:
        badges["speed"] = badge("speed vs GNU R", fmt_x(ser["speed"][-1][1]))
    if ser.get("builtins"):
        p, n = entries[ser["builtins"][-1][0]]["builtins"]
        badges["builtins"] = badge("builtins", f"{p}/{n}")
    for k in [k for k in ser if k.startswith("corpus:")]:
        name = k.split(":", 1)[1]
        p, n = entries[ser[k][-1][0]]["corpora"][name]
        badges["corpus-" + name] = badge(CORPORA.get(name, name), f"{fmt_pct(p / n)} ({p:,}/{n:,})")
    last = entries[-1]
    for k, b in badges.items():
        with open(os.path.join(a.out, "badges", f"{k}.json"), "w") as f:
            json.dump(b, f)

    # The tables: the newest commit that measured each kind of thing.
    perf_e = next((e for e in reversed(entries) if "perf" in e), None)
    rows = []
    if perf_e:
        hist = {}
        for i, e in enumerate(entries):
            for k, v in speedups(e.get("perf", {})).items():
                hist.setdefault(k, []).append((i, v, ""))
        for k, v in sorted(perf_e["perf"].items()):
            s = v["gnur"] / v["robigo"] if v.get("gnur") and v.get("robigo") else None
            note = "error" if v.get("error") else "wrong" if v.get("wrong") else ""
            rows.append(f'<tr><td>{E(k)}</td><td class="n">{fmt_x(s) if s and not note else ""}</td>'
                        f'<td>{sparkline(hist.get(k, []), len(entries))}</td>'
                        f'<td class="n">{v.get("robigo", 0):,}</td><td class="n">{v.get("gnur", 0):,}</td>'
                        f'<td class="note">{note}</td></tr>')
    corp_e = next((e for e in reversed(entries) if "corpora" in e), None)
    crows = []
    if corp_e:
        for k in corpus_order(corp_e["corpora"]):
            p, n = corp_e["corpora"][k]
            crows.append(f'<tr><td>{E(CORPORA.get(k, k))} <span class="meta">({E(k)})</span></td>'
                         f'<td class="n">{p:,}</td><td class="n">{n:,}</td><td class="n">{fmt_pct(p / n)}</td></tr>')
    gap_blocks = []
    for k in corpus_order((corp_e or {}).get("gaps", {})):
        ids = corp_e["gaps"][k]
        items = []
        for cid in ids:
            path = gap_path(k, cid)
            label = f"<code>{E(cid)}</code>"
            items.append(f'<li><a href="{a.repo_url}/blob/{corp_e["sha"]}/{E(path)}">{label}</a></li>' if path
                         else f"<li>{label}</li>")
        gap_blocks.append(f'<details><summary>{E(CORPORA.get(k, k))} <span class="meta">({E(k)})</span>: '
                          f'{len(ids):,} failing</summary><ul class="gaps">{"".join(items)}</ul></details>')
    gaps = ("\n".join(gap_blocks) if gap_blocks else "<p>No open gaps recorded.</p>")
    cnames = corpus_order({k for e in entries for k in e.get("corpora", {})})
    hrows = []
    for e in reversed(entries):
        s = speedups(e.get("perf", {}))
        cells = [f'<td class="n">{fmt_pct(e["corpora"][k][0] / e["corpora"][k][1])}</td>'
                 if k in e.get("corpora", {}) and e["corpora"][k][1] else '<td></td>' for k in cnames]
        b = e.get("builtins")
        hrows.append(f'<tr><td><a href="{a.repo_url}/commit/{e["sha"]}"><code>{e["sha"][:7]}</code></a> '
                     f'<span class="meta">{E(e["title"][:60])}</span></td><td>{e["date"][:10]}</td>'
                     f'<td class="n">{fmt_x(geomean(list(s.values()))) if s else ""}</td>{"".join(cells)}'
                     f'<td class="n">{f"{b[0]}/{b[1]}" if b else ""}</td></tr>')
    page = PAGE.format(
        font=FONT, repo_url=a.repo_url, sha=last["sha"], sha7=last["sha"][:7], date=last["date"][:10],
        speed=figure(speed_svg, entries, ser), groups=figure(groups_svg, entries, ser),
        compat=figure(compat_svg, entries, ser), bench_rows="\n".join(rows), corpus_rows="\n".join(crows),
        gaps=gaps, gap_sha7=(corp_e or last)["sha"][:7], packages=packages_section(entries, a.repo_url),
        n_commits=len(entries), corpus_heads="".join(f'<th class="n">{E(k)}</th>' for k in cnames),
        history_rows="\n".join(hrows))
    with open(os.path.join(a.out, "index.html"), "w") as f:
        f.write(page)
    print(f"rendered {len(entries)} commits into {a.out}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("add")
    p.add_argument("history")
    p.add_argument("dir")
    p.add_argument("--commit", required=True)
    p.add_argument("--run")
    p.add_argument("--repo", default=DEFAULT_REPO)
    p = sub.add_parser("sync")
    p.add_argument("history")
    p.add_argument("--repo", default=DEFAULT_REPO)
    p.add_argument("--limit", type=int, default=30, help="look at the newest N main runs")
    p = sub.add_parser("render")
    p.add_argument("history")
    p.add_argument("--out", required=True)
    p.add_argument("--repo", default=DEFAULT_REPO)
    a = ap.parse_args()
    if a.cmd == "render":
        a.repo_url = f"https://github.com/{a.repo}"
    sys.exit({"add": cmd_add, "sync": cmd_sync, "render": cmd_render}[a.cmd](a) or 0)


if __name__ == "__main__":
    main()
