"""One R script, run differentially: robigo against GNU R 4.5.0, robigo's
tiers against each other, or a program against itself with code that never
runs. The verdicts scripts/fuzz-eval.py buckets and scripts/reduce.py
preserves.

    v = Checker(robigo, oracle).oracle(path)    # or .tiers(path), .crash(path),
    v.kind, v.key                                #    .emi(path, variant_path)

`kind` is "ok", "diff", "crash", "timeout" or "unstable"; `key` says how a
"diff" or "crash" differs, independent of where in the script: each part
that differs (stdout, stderr, status) with its first differing line on each
side (`signature`), the tier blamed, or the crash kind. Two scripts with equal keys fail the same way,
which is what lets the reducer delete code around a failure.
"""
import collections
import os
import pathlib
import re
import resource
import signal
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import corpus  # noqa: E402
import fuzz  # noqa: E402

RTMP = re.compile(rb"/tmp/Rtmp[A-Za-z0-9]+")

ENV = {"PATH": "/usr/bin:/bin", "LANGUAGE": "en", "LC_ALL": "C.UTF-8",
       "R_DEFAULT_PACKAGES": "NULL", "HOME": "/tmp", "TZ": "UTC",
       # A local machine's R_LIBS_USER would load the host R's packages into
       # robigo (with "built under R version x.y.z" warnings the oracle never
       # prints): run base-only, like CI and scripts/regen-expected.sh.
       "R_LIBS": "", "R_LIBS_USER": "", "R_LIBS_SITE": "",
       "R_ENVIRON": "", "R_ENVIRON_USER": "", "R_PROFILE": "", "R_PROFILE_USER": ""}
TIMEOUT = 30
MEMORY_LIMIT = 16 << 30


def limit_resources():
    """The address-space cap and no core files, for this process and so
    every child: a failed allocation is then robigo's to report, the same
    in every tool that runs it."""
    resource.setrlimit(resource.RLIMIT_AS, (MEMORY_LIMIT, MEMORY_LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
TIERS = corpus.TIERS  # interp, vm, jit (the default)

Run = collections.namedtuple("Run", "out err status")
Verdict = collections.namedtuple("Verdict", "kind key detail")


def norm(b):
    """corpus.py's masking, plus the random R CMD SHLIB/dyn.load tmpdirs
    whose names differ on every run."""
    return corpus.normalize(RTMP.sub(b"/tmp/RtmpXXXXXX", b))


def find_oracle(user=None):
    """GNU R's Rscript: USER, $ROBIGO_ORACLE_R, the research build, the
    system's."""
    candidates = [pathlib.Path(p) for p in (user, os.environ.get("ROBIGO_ORACLE_R")) if p]
    candidates.append(pathlib.Path.home() / ".cache/robigo-research/R-4.5.0-build/bin/Rscript")
    candidates.append(pathlib.Path("/usr/bin/Rscript"))
    for c in candidates:
        if c.exists():
            return c
    sys.exit("no oracle found: pass --oracle, set ROBIGO_ORACLE_R, or run scripts/build-oracle.sh")


def oracle_info(rscript):
    """(version, R_HOME) of the oracle, asked of R itself; ("", "") when it
    cannot answer (hung, unexecutable, garbage)."""
    try:
        lines = subprocess.run([str(rscript), "--vanilla", "-e", 'cat(format(getRversion()), R.home(), sep="\\n")'],
                               capture_output=True, text=True, timeout=60).stdout.splitlines()
    except (OSError, subprocess.TimeoutExpired):
        return "", ""
    return (lines[-2], lines[-1]) if len(lines) >= 2 else ("", "")


def require_oracle(user=None):
    """(Rscript, R_HOME) of a GNU R 4.5.0, or exit: newer R disagrees with
    4.5.0 for reasons that are not robigo's bugs."""
    rscript = find_oracle(user)
    version, home = oracle_info(rscript)
    if version != "4.5.0":
        sys.exit(f"oracle {rscript} is R {version or '(unknown)'}, not 4.5.0; refusing (false diffs)")
    return rscript, home


def run(cmd, env, timeout=TIMEOUT, cwd=None):
    """(stdout, stderr, status) of CMD; status "timeout" when it ran out of
    time (its whole process group is killed)."""
    try:
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, cwd=cwd,
                             start_new_session=True)
    except OSError as e:
        # An environment failure, not a finding: stop every worker now
        # (sys.exit in a pool thread would wait out the whole sweep).
        print(f"cannot run {cmd[0]}: {e}", file=sys.stderr, flush=True)
        sys.stdout.flush()
        os._exit(2)
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(os.getpgid(p.pid), signal.SIGKILL)
        except OSError:
            p.kill()
        out, err = p.communicate()
        return Run(out, err, "timeout")
    return Run(out, err, p.returncode)


def outcome(r):
    """A run's comparable result: normalized stdout and stderr, and status."""
    return norm(r.out), norm(r.err), r.status


def text(r):
    return r.err.decode("utf-8", "replace")


def signature(exp, got, loose=False):
    """How outcome GOT differs from EXP: for each part that differs
    (stdout, stderr, status), its name and the first differing line on each
    side (LOOSE: the names only)."""
    parts = []
    for i, name in ((0, "out"), (1, "err")):
        if exp[i] != got[i]:
            if loose:
                parts.append(name)
                continue
            a, b = exp[i].split(b"\n"), got[i].split(b"\n")
            j = next((j for j, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            line = lambda ls: ls[j].decode("utf-8", "replace") if j < len(ls) else "<end>"
            parts.append(f"{name}: {line(a)!r} -> {line(b)!r}")
    if exp[2] != got[2]:
        parts.append("status" if loose else f"status: {exp[2]} -> {got[2]}")
    return tuple(parts)


def blame(o):
    """The tier that disagrees, given outcomes O by tier: (odd one, the
    reference it is compared with), or None when all three agree."""
    a, b, c = o["jit"], o["vm"], o["interp"]
    if a == b == c:
        return None
    if b == c:
        return "jit", "interp"
    if a == c:
        return "vm", "interp"
    if a == b:
        return "interp", "vm"   # or the VM and the JIT share a bug
    return "all", "interp"


class Checker:
    """Verdicts on scripts. ROBIGO and ORACLE (Rscript) are paths; ENV the
    environment of every run (R_HOME: the oracle's own home, for robigo's
    base library too); STABLE runs the reference twice and calls a script
    whose reference output changes "unstable"."""

    def __init__(self, robigo, oracle=None, env=None, stable=True, loose=False, timeout=TIMEOUT):
        self.robigo_bin, self.oracle_bin = str(robigo), oracle and str(oracle)
        self.env = dict(env or ENV)
        self.stable, self.loose, self.timeout = stable, loose, timeout

    # The runners: a script path in, a Run out. scripts/reduce.py replaces
    # them for corpus cases (corpus.run_case).
    def robigo(self, path, tier=None):
        env = dict(self.env, **TIERS[tier]) if tier else self.env
        return run([self.robigo_bin, "--vanilla", str(path)], env, self.timeout)

    def gnur(self, path):
        return run([self.oracle_bin, "--vanilla", str(path)], self.env, self.timeout)

    def _crashed(self, runs):
        for r in runs:
            if r.status != "timeout" and (kind := fuzz.classify(r.status, text(r))):
                return Verdict("crash", ("crash", kind), text(r))
        return None

    def crash(self, path, tier=None):
        """Does robigo crash (scripts/fuzz.py's classification)?"""
        r = self.robigo(path, tier)
        return self._crashed([r]) or Verdict("timeout" if r.status == "timeout" else "ok", None, "")

    def oracle(self, path):
        """robigo against GNU R."""
        exp = []
        for _ in range(2 if self.stable else 1):
            o = self.gnur(path)
            if o.status == "timeout":
                return Verdict("timeout", "oracle", "")
            exp.append(outcome(o))
        if len(set(exp)) > 1:
            return Verdict("unstable", "oracle", "")
        r = self.robigo(path)
        if v := self._crashed([r]):
            return v
        if r.status == "timeout":
            return Verdict("timeout", "robigo", "")
        got = outcome(r)
        if got == exp[0]:
            return Verdict("ok", None, "")
        return Verdict("diff", ("diff",) + signature(exp[0], got, self.loose), text(r))

    def tiers(self, path):
        """robigo's three tiers against each other (the default, twice when
        STABLE)."""
        runs = {"jit": self.robigo(path)}
        again = self.robigo(path) if self.stable else None
        runs["vm"] = self.robigo(path, "vm")
        runs["interp"] = self.robigo(path, "interp")
        # A crash is the finding, even when another run timed out.
        if v := self._crashed(list(runs.values()) + ([again] if again else [])):
            return v
        if any(r.status == "timeout" for r in runs.values()):
            return Verdict("timeout", "tiers", "")
        o = {t: outcome(r) for t, r in runs.items()}
        # Unseeded RNG, the clock or tmpdir names differ within one tier.
        if again is not None and outcome(again) != o["jit"]:
            return Verdict("unstable", "tiers", "")
        b = blame(o)
        if b is None:
            return Verdict("ok", None, "")
        odd, ref = b
        got = o["jit" if odd == "all" else odd]
        return Verdict("diff", ("tier", odd) + signature(o[ref], got, self.loose),
                       f"[{odd}]\n{text(runs['jit' if odd == 'all' else odd])}\n[{ref}]\n{text(runs[ref])}")

    def emi(self, path, variant):
        """Equivalence modulo inputs: VARIANT is the program at PATH plus
        code that never runs (rast.insert_dead), so it must print the same.
        The interpreter decides whether the insertion is neutral at all (a
        program that prints its own functions sees the new code): only then
        does a difference in the default tier count."""
        p, v = self.robigo(path), self.robigo(variant)
        pi, vi = self.robigo(path, "interp"), self.robigo(variant, "interp")
        if c := self._crashed([p, v, pi, vi]):
            return c
        if "timeout" in (p.status, v.status, pi.status, vi.status):
            return Verdict("timeout", "emi", "")
        if outcome(pi) != outcome(vi):
            return Verdict("unstable", "emi not neutral", "")
        if outcome(p) == outcome(v):
            return Verdict("ok", None, "")
        return Verdict("diff", ("emi",) + signature(outcome(p), outcome(v), self.loose),
                       f"[program]\n{text(p)}\n[with dead code]\n{text(v)}")
