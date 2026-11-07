#!/usr/bin/env python3
"""robigo against every other R: GNU R, Ř (rir) and FastR, on wall time.

    scripts/rivals.py setup                       # pull/build the rivals' images
    scripts/rivals.py run [--filter RE] [--impl robigo,gnur,rir,fastr]
                          [--budget SECS] [--jobs N] [--out FILE]
    scripts/rivals.py report [FILE] [--markdown]

"The fastest R" needs a number. `scripts/perf.py` compares instruction counts
with GNU R, which is the right regression gate and the wrong finish line:
instructions are not time, and the fastest R is not GNU R. This script runs
the programs of benchmarks/benchmarks.toml on all four and reports two wall
times for each:

- shot: one fresh process, startup included. What `Rscript file.R` costs.
- peak: the fastest iteration of the program repeated in one process for
  --budget seconds, the number a JIT with warmup is judged by.

Rivals run in containers (FastR and Ř do not install on a current
distribution, and the host's GNU R is not 4.5.0), with the clock read inside
the container so that nothing is charged for starting one. A verdict for an
implementation is only recorded if its answer is GNU R's. Rivals implement
older versions of R, so "the answer" is the program's stdout, compared
exactly: a rival that prints something else is reported as `wrong` and not
counted, not silently ranked.
"""
import argparse
import concurrent.futures
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perf import BASE_ENV, CACHE, ROOT, STATE, conf, geomean, suite_src  # noqa: E402

RIVALS = os.path.expanduser("~/.cache/robigo-rivals")
GRAAL = "/opt/graalvm-ce-java17-22.3.1"
IMAGES = {
    "gnur": ("robigo-oracle:4.5.0", ["Rscript"]),
    "rir": ("registry.gitlab.com/rirvm/rir_mirror:master", ["/opt/rir/build/release/bin/Rscript"]),
    "fastr": ("robigo-rival-fastr:22.3.1", [f"{GRAAL}/bin/Rscript"]),
}
IMPLS = ["robigo", "gnur", "rir", "fastr"]

# Wraps one program: `.body()` runs it once. The clock lives inside R, and the
# repetitions grow until a batch is long enough for proc.time()'s millisecond
# resolution to mean something: only batches of 50 ms or more count, so a
# program that takes microseconds is still timed to about 2%.
PEAK = """.budget <- {budget}
.k <- 1
.t0 <- proc.time()[[3]]
repeat {{
  .s <- proc.time()[[3]]
  for (.j in seq_len(.k)) .body()
  .d <- proc.time()[[3]] - .s
  if (.d >= 0.05) cat("PEAK", .d / .k, "\\n", file = {report!r}, append = TRUE)
  if (proc.time()[[3]] - .t0 > .budget) break
  if (.d < 0.1) .k <- .k * 2
}}
"""

RBENCH_BODY = """setwd({dir!r})
source({file!r})
.n <- {param}
.body <- if (length(formals(execute)) > 0) function() execute(.n) else function() {{ for (.i in seq_len(.n)) .v <- execute(); .v }}
"""

# Parsed once: re-evaluating the same expressions keeps each implementation's
# per-function-body caches, which re-parsing would throw away.
SUITE_BODY = """.exprs <- parse({file!r})
.null <- file("/dev/null", "w")
.body <- function() {{
  sink(.null)
  on.exit(sink())
  e <- new.env(parent = globalenv())
  for (x in .exprs) eval(x, e)
}}
"""


def docker(*args, **kw):
    return subprocess.run(["docker", *args], capture_output=True, text=True, **kw)


def have_image(img):
    return docker("image", "inspect", img).returncode == 0


def cmd_setup(_):
    """FastR 22.3.1 is the last release; its components only install one by one."""
    for impl, (img, _cmd) in IMAGES.items():
        if have_image(img):
            print(f"{impl}: have {img}")
            continue
        if impl == "gnur":
            sys.exit("build robigo-oracle:4.5.0 from .github/actions/setup/action.yml's Dockerfile")
        if impl == "rir":
            subprocess.run(["docker", "pull", img], check=True)
            continue
        d = os.path.join(RIVALS, "fastr")
        os.makedirs(d, exist_ok=True)
        ce = "https://github.com/graalvm/graalvm-ce-builds/releases/download/vm-22.3.1"
        for name, url in (
            ("graalvm.tgz", f"{ce}/graalvm-ce-java17-linux-amd64-22.3.1.tar.gz"),
            ("llvm-toolchain.jar", f"{ce}/llvm-toolchain-installable-java17-linux-amd64-22.3.1.jar"),
            ("llvm.jar", f"{ce}/llvm-installable-svm-java17-linux-amd64-22.3.1.jar"),
            ("r.jar", "https://github.com/oracle/fastr/releases/download/vm-22.3.1/"
                      "r-installable-java17-linux-amd64-22.3.1.jar"),
        ):
            p = os.path.join(d, name)
            if not os.path.exists(p):
                subprocess.run(["curl", "-sSfL", "-o", p + ".part", url], check=True)
                os.replace(p + ".part", p)
        with tempfile.TemporaryDirectory() as ctx:
            shutil.copy(os.path.join(d, "graalvm.tgz"), ctx)
            with open(os.path.join(ctx, "Dockerfile"), "w") as f:
                f.write("FROM ubuntu:22.04\n"
                        "RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends gcc g++ gfortran "
                        "make libcurl4 libbz2-1.0 liblzma5 libpcre2-8-0 libpcre3 libgomp1 libgfortran5 zlib1g-dev "
                        "libbz2-dev liblzma-dev libpcre2-dev libcurl4-openssl-dev libreadline8 ca-certificates\n"
                        "ADD graalvm.tgz /opt/\n")
            subprocess.run(["docker", "build", "-t", "robigo-rival-fastr:base", ctx], check=True)
        name = "robigo-fastr-setup"
        docker("rm", "-f", name)
        script = (f"cd /j; G={GRAAL}/bin; $G/gu install -L llvm-toolchain.jar; $G/gu install -L llvm.jar; "
                  f"$G/gu install -L r.jar; {GRAAL}/languages/R/bin/configure_fastr")
        subprocess.run(["docker", "run", "--name", name, "-v", f"{d}:/j", "robigo-rival-fastr:base", "bash", "-c",
                        script], check=True)
        subprocess.run(["docker", "commit", name, img], check=True)
        docker("rm", name)
        print(f"fastr: built {img}")


class Runner:
    """One long-lived container per rival, so that `docker run` is never timed."""

    def __init__(self, impls, shared):
        self.names = {}
        self.shared = shared
        for impl in impls:
            if impl == "robigo":
                continue
            img = IMAGES[impl][0]
            name = f"robigo-rivals-{impl}-{os.getpid()}"
            mounts = []
            for d in (ROOT, CACHE, shared):
                mounts += ["-v", f"{d}:{d}"]
            r = docker("run", "-d", "--rm", "--name", name, *mounts, "--entrypoint", "sleep", img, "infinity")
            if r.returncode:
                sys.exit(f"{impl}: {r.stderr}")
            self.names[impl] = name

    def close(self):
        for n in self.names.values():
            docker("rm", "-f", n)

    def run(self, impl, file, pk, timeout, robigo):
        """(seconds, stdout, status) of one process; the clock runs where the program does."""
        env = dict(BASE_ENV, R_DEFAULT_PACKAGES=pk)
        if impl == "robigo":
            cmd = [robigo, "--vanilla", file]
        else:
            cmd = ["docker", "exec"]
            for k, v in env.items():
                cmd += ["-e", f"{k}={v}"]
            cmd += [self.names[impl], "bash", "-c"]
        clock = 'S=$(date +%s.%N); "$@"; C=$?; E=$(date +%s.%N); echo "$S $E $C" >&3'
        fd, tfile = tempfile.mkstemp(dir=self.shared, suffix=".time")
        os.close(fd)
        os.chmod(tfile, 0o666)  # (containers may run as another user)
        inner = [*IMAGES[impl][1], "--vanilla", file] if impl != "robigo" else cmd
        # The limit runs where the program does: killing a `docker exec` client
        # would leave the benchmark running beside the next measurement.
        inner = ["timeout", "-s", "KILL", str(int(timeout)), *inner]
        wrapped = f"exec 3>{shlex.quote(tfile)}; {clock}"
        full = ["bash", "-c", wrapped, "bash", *inner]
        if impl == "robigo":
            run_env = dict(os.environ, **env, R_HOME=os.environ.get("R_HOME_ORACLE") or robigo_r_home())
        else:
            full = [*cmd, wrapped, "bash", *inner]
            run_env = None
        try:
            p = subprocess.run(full, cwd=self.shared, env=run_env, capture_output=True, timeout=timeout + 30)
        except subprocess.TimeoutExpired:
            return None, "timeout", -1
        try:
            with open(tfile) as f:
                s, e, status = f.read().split()
            os.unlink(tfile)
        except (OSError, ValueError):
            return None, p.stderr.decode("utf-8", "replace")[-300:], -1
        return float(e) - float(s), p.stdout.decode("utf-8", "replace"), int(status)


def robigo_r_home():
    p = os.path.expanduser("~/.cache/robigo-oracle/fs/usr/lib/R")
    return p if os.path.isdir(p) else subprocess.run(["R", "RHOME"], capture_output=True, text=True).stdout.strip()


def programs(c, rx):
    """[(name, shot script, body script, default packages)]"""
    out = []
    for sname, s in c["suites"].items():
        src = suite_src(sname, s)
        pk = s.get("default_packages", "utils,stats")
        if "dir" in s:
            for fn in sorted(os.listdir(src)):
                if fn.endswith(".R"):
                    file = os.path.join(src, fn)
                    with open(file) as f:
                        out.append((f"{sname}/{fn[:-2]}", f.read(), SUITE_BODY.format(file=file), pk))
        else:
            for b in s["benchmarks"]:
                d = os.path.join(src, b["dir"])
                body = RBENCH_BODY.format(dir=os.path.join(src, b.get("workdir", b["dir"])),
                                         file=os.path.join(d, b["file"]), param=b["param"])
                shot = body + (".r <- .body()\nif (exists(\"verifyResult\", mode = \"function\")) "
                               f"cat(\"verified:\", isTRUE(verifyResult(.r, {b['param']})), \"\\n\")\n")
                out.append((f"{sname}/{b['name']}", shot, body, pk))
    return [p for p in out if not rx or re.search(rx, p[0])]


def measure(runner, impl, prog, a, robigo):
    _, shot, body, pk = prog
    res = {}
    with tempfile.TemporaryDirectory(dir=runner.shared) as tmp:
        file = os.path.join(tmp, "shot.R")
        with open(file, "w") as f:
            f.write(shot)
        secs, out, status = runner.run(impl, file, pk, a.timeout, robigo)
        if secs is None or status != 0:
            return {"error": out[-300:] if out else f"status {status}"}
        res["out"], res["shot"] = out, secs
        for _ in range(a.runs - 1):
            s2, _out, st = runner.run(impl, file, pk, a.timeout, robigo)
            if s2 is not None and st == 0:
                res["shot"] = min(res["shot"], s2)
        if a.budget > 0:
            report = os.path.join(tmp, "peak.txt")
            file = os.path.join(tmp, "peak.R")
            with open(file, "w") as f:
                f.write(body + PEAK.format(budget=a.budget, report=report))
            _, _, status = runner.run(impl, file, pk, a.timeout + a.budget * 3, robigo)
            try:
                with open(report) as f:
                    xs = [float(ln.split()[1]) for ln in f if ln.startswith("PEAK")]
            except OSError:
                xs = []
            # (A run that failed or was cut off proves nothing about its peak.)
            if xs and status == 0:
                res["peak"] = min(x for x in xs)
                res["iters"] = len(xs)
    return res


def cmd_run(a):
    impls = a.impl.split(",")
    from corpus import robigo_path

    robigo = robigo_path(None)
    progs = programs(conf(), a.filter)
    os.makedirs(RIVALS, exist_ok=True)
    out = a.out or os.path.join(STATE, "perf", "rivals.json")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    doc = {"results": {}}
    if a.merge and os.path.exists(out):
        with open(out) as f:
            doc = json.load(f)
    lock = threading.Lock()
    with tempfile.TemporaryDirectory(dir=RIVALS) as shared:
        os.chmod(shared, 0o777)
        runner = Runner(impls, shared)
        try:
            def go(t):
                prog, impl = t
                r = measure(runner, impl, prog, a, robigo)
                with lock:
                    doc["results"].setdefault(prog[0], {})[impl] = r
                    with open(out, "w") as f:
                        json.dump(doc, f, indent=1, sort_keys=True)
                show = r.get("error", "")[:60] or f"shot={r.get('shot'):.3f} peak={r.get('peak')}"
                print(f"{prog[0]:42} {impl:7} {show}", file=sys.stderr, flush=True)

            # GNU R's answer first would need ordering; `report` compares afterwards.
            with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as ex:
                list(ex.map(go, [(p, i) for p in progs for i in impls]))
        finally:
            runner.close()
    report(doc, False)


def report(doc, markdown):
    res = doc["results"]
    impls = [i for i in IMPLS if any(i in r for r in res.values())]

    def value(r, impl, key):
        x = r.get(impl, {})
        gn = r.get("gnur", {})
        if key not in x or x.get("out") != gn.get("out"):
            return None
        return x[key]

    for key, title in (("shot", "one process, startup included"), ("peak", "fastest iteration, warmed up")):
        rows = []
        for name in sorted(res):
            vals = {i: value(res[name], i, key) for i in impls}
            have = {i: v for i, v in vals.items() if v}
            if "robigo" not in have and "gnur" not in have:
                continue
            rows.append((name, vals, min(have, key=have.get)))
        print(f"\n## {key}: {title} (seconds; blank: failed or a different answer)\n")
        if markdown:
            print("| benchmark | " + " | ".join(impls) + " | fastest |")
            print("|---|" + "---|" * (len(impls) + 1))
        for name, vals, best in rows:
            cells = ["" if vals[i] is None else f"{vals[i]:.3g}" for i in impls]
            print(("| " + " | ".join([name, *cells, best]) + " |") if markdown else
                  f"{name:42} " + " ".join(f"{c:>8}" for c in cells) + f"  {best}")
        wins = {i: sum(1 for r in rows if r[2] == i) for i in impls}
        print(f"\nfastest on: {wins}")
        for i in impls:
            if i == "robigo":
                continue
            pairs = [r[1]["robigo"] / r[1][i] for r in rows if r[1].get("robigo") and r[1].get(i)]
            if pairs:
                print(f"robigo/{i}: geometric mean {geomean(pairs):.3f} over {len(pairs)}, "
                      f"robigo faster on {sum(1 for x in pairs if x < 1)}")


def cmd_report(a):
    with open(a.file or os.path.join(STATE, "perf", "rivals.json")) as f:
        report(json.load(f), a.markdown)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("setup").set_defaults(fn=cmd_setup)
    r = sub.add_parser("run")
    r.add_argument("--filter")
    r.add_argument("--impl", default=",".join(IMPLS))
    r.add_argument("--budget", type=float, default=8.0, help="seconds of repeated runs for the peak number")
    r.add_argument("--runs", type=int, default=2, help="fresh processes; the fastest counts")
    r.add_argument("--jobs", type=int, default=1, help="more than 1 only for a quick look: runs disturb each other")
    r.add_argument("--timeout", type=float, default=120.0)
    r.add_argument("--out")
    r.add_argument("--merge", action="store_true")
    r.set_defaults(fn=cmd_run)
    p = sub.add_parser("report")
    p.add_argument("file", nargs="?")
    p.add_argument("--markdown", action="store_true")
    p.set_defaults(fn=cmd_report)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
