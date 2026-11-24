# Draft for scripts/test-rt-ci.py (runner PR 3, after #1028): every robigo-rt
# that runs under the default (rdb) boot has base's database tree first.
# TREE_HELPER and TREE_STEP are L's names from #1028 (to fill in).
import os, re, sys
from pathlib import Path

ROOT = Path(sys.argv[1])
# the helper's two ways to a Command pointed at the binary's own test tree
# (crates/robigo-rtapi/tests/common/db_tree.rs, #1028): a bare
# ensure_db_tree() then Command::new makes the tree but does not point the
# process at it, so it is no way in
TREE_HELPER = re.compile(r"\brt_command\(|\bcommand_for\(")
# test binaries that spawn robigo-rt without the helper, each with its reason
# (an explicit tree-less boot: every spawn sets ROBIGO_RT_BOOT=m1 or stage0)
TREE_TEST_CALLERS = {}
# the helper itself: the one place that hands the binary to Command::new
HELPER_FILE = "crates/robigo-rtapi/tests/common/db_tree.rs"

SPAWN = re.compile(r"CARGO_BIN_EXE_robigo-rt")


def spawns_around_the_helper(text):
    """The places a test file hands a robigo-rt binary to Command::new
    itself: the binary's path, or a variable bound to it, as the command."""
    bound = set(re.findall(r"let\s+(?:mut\s+)?(\w+)\s*(?::[^=]+)?=\s*[^;]*CARGO_BIN_EXE_robigo-rt", text))
    out = re.findall(r"Command::new\([^)]*CARGO_BIN_EXE_robigo-rt[^)]*\)", text)
    for v in bound:
        out += re.findall(r"Command::new\(\s*&?" + re.escape(v) + r"\b[^)]*\)", text)
    return out


def names_a_tree_less_boot(text):
    """Whether every robigo-rt spawn of a test file names a boot that needs
    no database tree (ROBIGO_RT_BOOT = m1 or stage0): as many such settings
    as spawns, and none of another boot."""
    spawns = len(SPAWN.findall(text))
    boots = re.findall(r"\"ROBIGO_RT_BOOT\",\s*\"(\w*)\"", text)
    return spawns > 0 and len(boots) >= spawns and all(b in ("m1", "stage0") for b in boots)


def test_binaries_without_a_tree():
    """(a) crates/*/tests files that spawn robigo-rt and neither call the
    helper nor are listed."""
    out = []
    for p in sorted(ROOT.glob("crates/*/tests/**/*.rs")):
        rel = str(p.relative_to(ROOT))
        text = p.read_text(errors="replace")
        if not SPAWN.search(text) or rel in TREE_TEST_CALLERS or rel == HELPER_FILE:
            continue
        around = spawns_around_the_helper(text)
        if names_a_tree_less_boot(text):
            continue
        if not TREE_HELPER.search(text) or around:
            out.append(f"{rel}" + (f" (spawns around the helper: {around[0][:60]})" if around else ""))
    return out


def jobs(yml):
    """{job: [(step name or '', run text)]} of a workflow, by indentation
    (jobs at two spaces, steps' `- name:`/`run:` under `steps:`)."""
    out, job, step = {}, None, None
    lines = yml.splitlines()
    i = 0
    in_jobs = False
    while i < len(lines):
        l = lines[i]
        if l.startswith("jobs:"):
            in_jobs = True
        elif in_jobs and re.match(r"^  [A-Za-z0-9_-]+:\s*$", l):
            job = l.strip()[:-1]
            out[job] = []
        elif job and re.match(r"^      - ", l):
            out[job].append(["", ""])
            m = re.match(r"^      - name: (.*)$", l)
            if m:
                out[job][-1][0] = m.group(1).strip()
        elif job and out[job] and re.match(r"^        name: (.*)$", l):
            out[job][-1][0] = re.match(r"^        name: (.*)$", l).group(1).strip()
        elif job and out[job] and re.match(r"^      (- )?\s*run: ", l) or (job and out[job] and re.match(r"^        run: ", l)):
            m = re.match(r"^\s*(?:- )?run: (.*)$", l)
            body = m.group(1)
            if body.strip() in ("|", ">"):
                j = i + 1
                block = []
                while j < len(lines) and (lines[j].startswith("          ") or not lines[j].strip()):
                    block.append(lines[j]); j += 1
                body = "\n".join(block)
            out[job][-1][1] += body + "\n"
        i += 1
    return out


# a run line that executes a robigo-rt binary itself (a path ending in
# robigo-rt as the command), not one that passes it to a script
DIRECT = re.compile(r"(?:^|[\s;&|(])(?:\"?\$?[\w./{}$-]*/)?robigo-rt(?:-main)?\"?\s+(?!--robigo-rt)[-\w]", re.M)
TREE_SOURCE = re.compile(r"rt-home-db\.sh|rt-test-tree\.sh|tar -xf [^\n]*rt-trees|--trees\b|--pack-trees\b")
TREE_LESS = re.compile(r"ROBIGO_RT_BOOT=(m1|stage0)\b")


def rt_ci_writes_tree(run):
    """Whether an rt-ci.py command's plan writes base's database tree."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("rt_ci", ROOT / "scripts" / "rt-ci.py")
    rt = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(rt)
    m = re.search(r"--shard (\S+)", run)
    plan = rt.shard_steps(["robigo-rt", "robigo-rtapi"], m.group(1)) if m else rt.steps(["robigo-rt", "robigo-rtapi"])
    return any(c[:1] == ["scripts/rt-home-db.sh"] for _t, _e, c in plan)


def direct_runs_without_a_tree():
    """(b) ci.yml jobs with a step that runs robigo-rt itself (not through
    cargo test, which (a) covers, or rt-ci.py, whose plans #1005's walk
    covers) with no tree source earlier in the job and no tree-less boot."""
    out = []
    for name, steps in jobs((ROOT / ".github/workflows/ci.yml").read_text()).items():
        seen_tree = False
        for title, run in steps:
            if TREE_SOURCE.search(run):
                seen_tree = True
            if "scripts/rt-ci.py" in run:
                seen_tree = seen_tree or rt_ci_writes_tree(run)
                continue
            for line in run.splitlines():
                if DIRECT.search(line) and "--robigo-rt" not in line.split("robigo-rt")[0][-12:] \
                        and not TREE_LESS.search(line) and not seen_tree:
                    out.append(f"{name}: {line.strip()[:80]}")
    return out


print("(a)", test_binaries_without_a_tree())
print("(b)", direct_runs_without_a_tree())
