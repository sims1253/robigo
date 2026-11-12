#!/usr/bin/env python3
"""A pull request's size label and test delta (.github/workflows/pr-delta.yml).

    scripts/pr-delta.py BASE HEAD                 # print the label and the comment
    scripts/pr-delta.py BASE HEAD --apply NUMBER  # set them on the pull request (gh)

The size counts changed lines outside tests (`tests/**`, `crates/*/tests/**`,
`scripts/test*.py`), whitespace ignored; a pull request of tests alone is
sized by its test lines. The comment lists what the tests gained or lost:
conformance scripts, ratchet markers per suite (a removed marker is a
ratchet going back, which AGENTS.md forbids), Rust #[test]s and Python
test methods. Only git objects are read: nothing from HEAD is run.
"""
import json
import os
import re
import subprocess
import sys
from collections import Counter

SIZES = [(10, "size:XS"), (30, "size:S"), (100, "size:M"), (500, "size:L"), (1000, "size:XL")]
LARGEST = "size:XXL"
LABELS = {
    "size:XS": ("0e8a16", "0-9 changed lines outside tests"),
    "size:S": ("5ebd3e", "10-29 changed lines outside tests"),
    "size:M": ("fbca04", "30-99 changed lines outside tests"),
    "size:L": ("fe7d37", "100-499 changed lines outside tests"),
    "size:XL": ("d93f0b", "500-999 changed lines outside tests"),
    "size:XXL": ("b60205", "1000+ changed lines outside tests"),
}
MARKER = "<!-- pr-delta -->"
TEST_RE = re.compile(r"^(tests/|crates/[^/]+/tests/|scripts/test[-_][^/]*\.py$)")
NOT_COUNTED = {"Cargo.lock"}


def git(*args):
    # Diffs carry test data in other encodings (latin1 recorded output);
    # only line counts matter, so undecodable bytes become U+FFFD.
    return subprocess.run(["git", "-c", "safe.directory=*", "-c", "core.quotepath=off", *args], capture_output=True, text=True,
                          errors="replace", check=True).stdout


def size_label(lines):
    for limit, label in SIZES:
        if lines < limit:
            return label
    return LARGEST


def suite_of(path):
    """The ratchet a `.pass` marker belongs to, or None."""
    if not path.endswith(".pass"):
        return None
    m = re.match(r"tests/corpora/([^/]+)/passing/", path)
    if m:
        return m.group(1)
    m = re.match(r"tests/parse/passing/([^/]+)/", path)
    if m:
        return "parse " + m.group(1)
    if path.startswith("tests/r/passing/"):
        return "conformance"
    if path.startswith("tests/packages/passing/"):
        return "packages"
    return None


def measure(numstat, name_status, patch):
    """The delta from `git diff --numstat`, `--name-status -M` and `-U0` output."""
    code = tests = 0
    for line in numstat.splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        added, deleted, path = parts[0], parts[1], parts[-1]
        n = (int(added) if added.isdigit() else 0) + (int(deleted) if deleted.isdigit() else 0)
        if path in NOT_COUNTED:
            continue
        if TEST_RE.match(path):
            tests += n
        else:
            code += n
    markers, scripts = Counter(), Counter()
    for line in name_status.splitlines():
        parts = line.split("\t")
        status, paths = parts[0][:1], parts[1:]
        if status == "R" or status == "C":
            continue  # a moved file is neither gained nor lost
        if status not in "AD" or not paths:
            continue
        path, sign = paths[0], (1 if status == "A" else -1)
        suite = suite_of(path)
        if suite:
            markers[(suite, status)] += 1
        elif path.startswith("tests/r/") and path.endswith(".R"):
            scripts[status] += 1
    rust, python = Counter(), Counter()
    current = old = ""
    for line in patch.splitlines():
        if line.startswith("--- "):
            old = line[6:] if line.startswith("--- a/") else ""
            continue
        if line.startswith("+++ "):
            # a deleted file's lines belong to its old path
            current = line[6:] if line.startswith("+++ b/") else old
            continue
        if not line[:1] in "+-":
            continue
        sign, body = line[0], line[1:].strip()
        if current.endswith(".rs") and re.match(r"#\[(tokio::)?test\b", body):
            rust[sign] += 1
        elif TEST_RE.match(current) and current.endswith(".py") and re.match(r"def test_", body):
            python[sign] += 1
    effective = code if code else tests
    return {
        "code_lines": code, "test_lines": tests, "label": size_label(effective),
        "scripts": {"added": scripts["A"], "removed": scripts["D"]},
        "markers": {s: {"added": markers[(s, "A")], "removed": markers[(s, "D")]}
                    for s in sorted({s for s, _ in markers})},
        "rust_tests": {"added": rust["+"], "removed": rust["-"]},
        "python_tests": {"added": python["+"], "removed": python["-"]},
    }


def render(d):
    """The comment body, or "" when no test changed."""
    rows = []

    def row(what, c):
        if c["added"] or c["removed"]:
            net = c["added"] - c["removed"]
            rows.append(f"| {what} | +{c['added']} | -{c['removed']} | {net:+d} |")

    row("conformance scripts (`tests/r`)", d["scripts"])
    for suite, c in d["markers"].items():
        row(f"ratchet markers: {suite}", c)
    row("Rust `#[test]`s", d["rust_tests"])
    row("Python test methods", d["python_tests"])
    if not rows:
        return ""
    out = [MARKER, "### Test delta", "", "| | added | removed | net |", "|---|---|---|---|", *rows]
    lost = sum(c["removed"] for c in d["markers"].values())
    if lost:
        out += ["", f"**{lost} ratchet marker(s) removed.** Markers never go back (AGENTS.md, Testing): "
                "a case that stopped passing is a regression to fix, not a marker to delete."]
    out += ["", f"<sub>{d['code_lines']} changed lines outside tests, {d['test_lines']} in tests: "
            f"`{d['label']}`.</sub>"]
    return "\n".join(out) + "\n"


def delta(base, head):
    rng = f"{base}...{head}"
    return measure(git("diff", "--numstat", "--no-renames", "--ignore-all-space", "--ignore-blank-lines", rng),
                   git("diff", "--name-status", "-M", rng),
                   git("diff", "-U0", "--no-color", rng))


def gh(*args, input=None):
    return subprocess.run(["gh", *args], input=input, capture_output=True, text=True, check=True).stdout


def apply(number, d, body):
    repo = os.environ.get("GITHUB_REPOSITORY") or gh("repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner").strip()
    for name, (color, desc) in LABELS.items():
        gh("label", "create", name, "--color", color, "--description", desc, "--force")
    have = json.loads(gh("api", f"repos/{repo}/issues/{number}/labels"))
    for lb in have:
        if lb["name"] in LABELS and lb["name"] != d["label"]:
            gh("api", "-X", "DELETE", f"repos/{repo}/issues/{number}/labels/{lb['name']}")
    if d["label"] not in {lb["name"] for lb in have}:
        gh("api", f"repos/{repo}/issues/{number}/labels", "-f", f"labels[]={d['label']}")
    comments = json.loads(gh("api", "--paginate", "--slurp", f"repos/{repo}/issues/{number}/comments"))
    mine = [c for page in comments for c in page
            if c.get("user", {}).get("type") == "Bot" and (c.get("body") or "").startswith(MARKER)]
    if body and mine:
        if mine[0]["body"] != body:
            gh("api", "-X", "PATCH", f"repos/{repo}/issues/comments/{mine[0]['id']}", "-F", "body=@-", input=body)
    elif body:
        gh("api", f"repos/{repo}/issues/{number}/comments", "-F", "body=@-", input=body)
    elif mine:
        gh("api", "-X", "DELETE", f"repos/{repo}/issues/comments/{mine[0]['id']}")


def main(argv):
    if len(argv) not in (2, 4) or (len(argv) == 4 and argv[2] != "--apply"):
        sys.exit(__doc__)
    d = delta(argv[0], argv[1])
    body = render(d)
    print(json.dumps(d, indent=1))
    print(body or "(no test delta)")
    if len(argv) == 4:
        apply(int(argv[3]), d, body)


if __name__ == "__main__":
    main(sys.argv[1:])
