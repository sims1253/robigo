#!/usr/bin/env python3
"""The most used CRAN packages' own test suites on robigo, scored against GNU R.

    scripts/package-suite.py select --snapshot DATE  # regenerate selection.tsv
    scripts/package-suite.py lock                    # rv.lock and sources.tsv
    scripts/package-suite.py sync                    # pinned library and sources
    scripts/package-suite.py record [PKG...]         # GNU R, three times
    scripts/package-suite.py run [PKG...]            # robigo, compare, ratchet
    scripts/package-suite.py absorb RESULT...        # markers from a run's results (CI's)
    scripts/package-suite.py show PKG [--all]        # one package's differences
    scripts/package-suite.py install [PKG...]        # robigo CMD INSTALL from source
    scripts/package-suite.py smoke [CASE...]         # user scripts, transcript-checked
    scripts/package-suite.py smoke --record [CASE...]  # GNU R: smoke expected transcripts

docs/verification.md describes the metric and tests/packages/README.md the
files. In short: `tests/packages/selection.tsv` pins the packages, `rv.lock`
pins their whole dependency tree, and every test file runs under its real
framework in a process of its own, supervised from here, so a crash, a memory
blow-up or a hang costs one file. GNU R's outcome of each test block is
recorded in `expected/`; a block counts when robigo gives the same outcome.

Everything runs in the oracle's environment: the `robigo-oracle:4.5.0` image
locally, the debian:trixie job on CI ($ROBIGO_PKG_ORACLE set).
"""
import argparse
import concurrent.futures
import fnmatch
import gzip
import hashlib
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIR = os.path.join(ROOT, "tests", "packages")
SELECTION = os.path.join(DIR, "selection.tsv")
SOURCES = os.path.join(DIR, "sources.tsv")
OWN_SKIPS = os.path.join(DIR, "own-skips.tsv")
PROJECT = os.path.join(DIR, "rproject.toml")
LOCK = os.path.join(DIR, "rv.lock")
EXPECTED = os.path.join(DIR, "expected")
PASSING = os.path.join(DIR, "passing")
RUNNER = os.path.join(DIR, "runner.R")
SMOKE_DIR = os.path.join(DIR, "smoke")
SMOKE_CASES = os.path.join(SMOKE_DIR, "cases")
SMOKE_EXPECTED = os.path.join(SMOKE_DIR, "expected")
SMOKE_PASSING = os.path.join(SMOKE_DIR, "passing")
SMOKE_DEFAULT_PACKAGES = "datasets,utils,grDevices,graphics,stats,methods"
SMOKE_LIMIT = 120
CANARY_SRC = os.path.join(DIR, "canary")
CACHE = os.environ.get("ROBIGO_PKG_CACHE", os.path.expanduser("~/.cache/robigo-packages"))
IMAGE = os.environ.get("ROBIGO_ORACLE_IMAGE", "robigo-oracle:4.5.0")
ROBIGO = os.environ.get("ROBIGO", os.path.join(ROOT, ".robigo", "bin", "robigo"))
PPM = "https://packagemanager.posit.co/cran"
CRANLOGS = "https://cranlogs.r-pkg.org/top/last-month/100"
BASE_PACKAGES = set("base compiler datasets graphics grDevices grid methods parallel splines stats "
                    "stats4 tcltk tools utils".split())

# The thresholds of the selection. A package enters above ENTER and stays
# until it falls below LEAVE, so a package at the line does not flap between
# snapshots.
ENTER = {"downloads": 900_000, "used_by": 500}
LEAVE = {"downloads": 810_000, "used_by": 450}

# The environment of every worker, GNU R's and robigo's alike. NOT_CRAN=true
# runs the snapshot tests, the ones that compare output exactly, which a CRAN
# run skips: over a quarter of all blocks. The memory cap and time limits are
# part of the metric: changing them changes what gets scored.
ENV = {"LANGUAGE": "en", "LC_ALL": "C.UTF-8", "TZ": "UTC", "NOT_CRAN": "true", "CI": "true",
       "TESTTHAT_PARALLEL": "false", "TESTTHAT_CPUS": "1", "R_LIBS_USER": "", "R_LIBS_SITE": ""}
MEMORY_CAP = 3 << 30
MIN_FILE_LIMIT = 60
FILE_LIMIT_FACTOR = 20
RECORD_FILE_LIMIT = 900  # GNU R's, while there is no recorded time to scale
PACKAGE_BUDGET = int(os.environ.get("ROBIGO_PKG_BUDGET", 1800))
GNU_RUNS = 3
GREEN_FLOOR = 0.8

# Test servers a package's tests talk to, by the name in selection.tsv's
# `server` column: started on the loopback of the file's own network
# namespace before R, for GNU R and robigo alike, so the tests still never
# reach the internet. `hosts` resolve to 127.0.0.1 in the whole oracle (an
# /etc/hosts of the worker's own would need a mount, which the container's
# AppArmor profile refuses): `docker run --add-host` here, the container's
# options in .github/workflows/packages.yml. In a worker without the server
# nothing listens there, so those names still lead nowhere. `env` is what
# the tests read to find the server.
#
# httpbin is python3-httpbin, pinned in apt.txt. curl's helper reads
# HTTPBIN_TEST_SERVER, but it evaluates `Sys.getenv(, find_test_server())`,
# and Sys.getenv forces its default: the helper probes its fallback list
# whatever the variable says, and stops if none answers. The last of that
# list is http://httpbin.org, so the server is httpbin.org on port 80.
SERVERS = {
    "httpbin": {"port": 80, "cmd": ["python3", "-I", "-m", "httpbin.core", "--host", "127.0.0.1", "--port", "80"],
                "hosts": ["httpbin.org"], "env": {"HTTPBIN_TEST_SERVER": "http://httpbin.org"}},
}
SERVER_START_LIMIT = 60

# Outcomes of the canary's tests: (pass, fail, error, skip, warn).
CANARY = {
    "pass": (1, 0, 0, 0, 0),
    "fail": (0, 1, 0, 0, 0),
    "error": (0, 0, 1, 0, 0),
    "skip": (0, 0, 0, 1, 0),
    "warn": (1, 0, 0, 0, 1),
    "pass: an unexported function, through the cloned namespace": (1, 0, 0, 0, 0),
    "pass: the edition survives an earlier test": (1, 0, 0, 0, 0),
    "pass: snapshot match": (1, 0, 0, 0, 0),
    "fail: snapshot mismatch": (0, 1, 0, 0, 0),
}


def die(msg):
    sys.exit(f"package-suite: {msg}")


def sha(data, n=12):
    return hashlib.sha256(data.encode() if isinstance(data, str) else data).hexdigest()[:n]


def file_sha(path, n=64):
    with open(path, "rb") as f:
        return sha(f.read(), n)


def fetch(url, dest=None):
    """Bytes of `url`, or the path `dest`; CRAN mirrors drop connections now and then."""
    for attempt in range(1, 4):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "robigo-package-suite"})
            with urllib.request.urlopen(req, timeout=120) as r:
                data = r.read()
            break
        except OSError:
            if attempt == 3:
                raise
            time.sleep(5 * attempt)
    if dest is None:
        return data
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(dest + ".tmp", "wb") as f:
        f.write(data)
    os.replace(dest + ".tmp", dest)
    return dest


# ---------------------------------------------------------------- the oracle

def in_oracle():
    if os.environ.get("ROBIGO_PKG_ORACLE"):
        return
    if not shutil.which("docker"):
        die("needs docker, or $ROBIGO_PKG_ORACLE set inside an equivalent environment")
    # the image is rebuilt when its definition changes (an apt.txt edit)
    want = sha(open(os.path.join(DIR, "Dockerfile")).read() + open(os.path.join(DIR, "apt.txt")).read())
    have = subprocess.run(["docker", "image", "inspect", "-f", '{{index .Config.Labels "robigo.oracle"}}', IMAGE],
                          capture_output=True, text=True).stdout.strip()
    if have != want:
        print(f"building {IMAGE}", flush=True)
        subprocess.run(["docker", "build", "-q", "--build-arg", f"ORACLE_ID={want}", "-t", IMAGE, DIR], check=True)
    os.makedirs(CACHE, exist_ok=True)
    rv = shutil.which("rv")
    cmd = ["docker", "run", "--rm", "--cap-add", "SYS_ADMIN", "--cap-add", "NET_ADMIN", "-e", "ROBIGO_PKG_ORACLE=1",
           *[f"--add-host={h}:127.0.0.1" for s in SERVERS.values() for h in s["hosts"]],
           "-e", f"ROBIGO_PKG_CACHE={CACHE}", "-e", f"ROBIGO_PKG_CHOWN={os.getuid()}:{os.getgid()}",
           "-e", f"ROBIGO={os.path.abspath(ROBIGO)}", "-v", f"{ROOT}:{ROOT}", "-v", f"{CACHE}:{CACHE}", "-w", ROOT]
    if rv:
        cmd += ["-v", f"{os.path.realpath(rv)}:/usr/local/bin/rv:ro"]
    # a robigo built in another checkout: its binary, its R_HOME beside it
    # (<exe>/../home) and its natives, at their own paths
    robigo_dir = os.path.dirname(os.path.dirname(os.path.abspath(ROBIGO)))
    extra = {robigo_dir}
    # its R_HOME's headers are a link into that checkout (scripts/robigo-home.sh)
    include = os.path.join(robigo_dir, "home", "include")
    if os.path.islink(include):
        extra.add(os.path.realpath(include))
    # where --out writes, so the result outlives the container
    for i, arg in enumerate(sys.argv):
        out = arg.split("=", 1)[1] if arg.startswith("--out=") else \
            sys.argv[i + 1] if arg == "--out" and i + 1 < len(sys.argv) else None
        if out:
            d = os.path.dirname(os.path.abspath(out))
            os.makedirs(d, exist_ok=True)
            extra.add(d)
    for k in ("ROBIGO_NATIVES", "ROBIGO_PKG_KEEP_LOGS"):
        if os.environ.get(k):
            os.makedirs(os.environ[k], exist_ok=True)
            extra.add(os.path.abspath(os.environ[k]))
    for d in sorted(extra):
        if os.path.isdir(d) and not (d + "/").startswith(ROOT + "/") and not (d + "/").startswith(CACHE + "/"):
            cmd += ["-v", f"{d}:{d}"]
    for k in ("ROBIGO_NATIVES", "ROBIGO_RDBFLAT", "ROBIGO_PKG_BUDGET", "ROBIGO_PKG_JOBS", "ROBIGO_PKG_KEEP_LOGS"):
        if k in os.environ:
            cmd += ["-e", f"{k}={os.environ[k]}"]
    os.execvp("docker", cmd + [IMAGE, "python3", os.path.abspath(__file__)] + sys.argv[1:])


def hand_back(*paths):
    """Files written in the image as root belong to whoever started it."""
    owner = os.environ.get("ROBIGO_PKG_CHOWN")
    if owner:
        subprocess.run(["chown", "-R", owner, *[p for p in paths if os.path.exists(p)]], check=False)


def locked():
    with open(LOCK, "rb") as f:
        return tomllib.load(f)


def lock_id():
    return file_sha(LOCK, 12)


def snapshot():
    with open(PROJECT, "rb") as f:
        url = tomllib.load(f)["project"]["repositories"][0]["url"]
    return url.rstrip("/").rsplit("/", 1)[1]


def library():
    return os.path.join(CACHE, f"project-{lock_id()}", "rv", "library", "4.5", "x86_64", "trixie")


def canary_library():
    return os.path.join(CACHE, f"canary-{sha(open(os.path.join(CANARY_SRC, 'tests/testthat/test-canary.R')).read())}")


# ----------------------------------------------------------------- selection

def selection():
    """{name: row of selection.tsv} in file order."""
    if not os.path.exists(SELECTION):
        return {}
    rows, cols = {}, None
    for line in open(SELECTION):
        if line.startswith("#"):
            continue
        fields = line.rstrip("\n").split("\t")
        if cols is None:
            cols = fields
        else:
            row = dict(zip(cols, fields))
            rows[row["package"]] = row
    return rows


def parse_packages(text):
    out = {}
    for block in text.split("\n\n"):
        f, key = {}, None
        for line in block.split("\n"):
            if line[:1] in (" ", "\t") and key:
                f[key] += " " + line.strip()
            elif ":" in line:
                key, v = line.split(":", 1)
                f[key] = v.strip()
        if "Package" in f:
            out[f["Package"]] = f
    return out


def depends(f, fields=("Depends", "Imports", "LinkingTo")):
    names = set()
    for field in fields:
        for item in f.get(field, "").split(","):
            m = re.match(r"\s*([A-Za-z][A-Za-z0-9.]*)", item)
            if m and m[1] != "R":
                names.add(m[1])
    return names


def cmd_select(a):
    """The packages over the thresholds on the snapshot's CRAN.

    "Used by" is the R Observatory's figure: distinct CRAN packages naming
    this one under Depends, Imports or LinkingTo. Downloads are cranlogs',
    the Posit mirror's count that the Observatory shows too.
    """
    index = gzip.decompress(fetch(f"{PPM}/{a.snapshot}/src/contrib/PACKAGES.gz")).decode("utf8", "replace")
    pk = parse_packages(index)
    used_by = {}
    for f in pk.values():
        for d in depends(f):
            used_by[d] = used_by.get(d, 0) + 1
    top = json.loads(fetch(CRANLOGS))
    downloads = {t["package"]: int(t["downloads"]) for t in top["downloads"]}
    if min(downloads.values()) >= LEAVE["downloads"]:
        die("the cranlogs list ends above the threshold: it may hide a package")
    old = selection()
    rows = []
    for name, dl in sorted(downloads.items(), key=lambda kv: -kv[1]):
        limit = LEAVE if name in old else ENTER
        if name in pk and dl > limit["downloads"] and used_by.get(name, 0) > limit["used_by"]:
            rows.append((name, pk[name]["Version"], dl, used_by[name]))
    window = f"{top['start'][:10]} to {top['end'][:10]}"
    with open(SELECTION, "w") as f:
        f.write(f"# CRAN packages with over {ENTER['downloads']:,} downloads in 30 days ({window}) and over "
                f"{ENTER['used_by']} packages using them,\n# at PPM snapshot {a.snapshot}. Written by "
                "`scripts/package-suite.py select`.\n")
        f.write("package\tversion\tdownloads\tused_by\tframework\tfilter\tenv\tserver\n")
        for name, version, dl, used in rows:
            prev = old.get(name, {})
            f.write(f"{name}\t{version}\t{dl}\t{used}\t{prev.get('framework', 'testthat')}\t"
                    f"{prev.get('filter', '')}\t{prev.get('env', '')}\t{prev.get('server', '')}\n")
    print(f"{len(rows)} packages, downloads {window}")


def cmd_lock(a):
    """rv.lock for the selection and its Suggests, and the sources' checksums."""
    sel = selection()
    url = f"{PPM}/__linux__/trixie/{a.snapshot}"
    deps = ",\n".join(f'  {{ name = "{p}", install_suggestions = true }}' for p in sel)
    with open(PROJECT, "w") as f:
        f.write('# Written by `scripts/package-suite.py lock`: the selection and its Suggests,\n'
                '# so that tests run instead of skipping for lack of a package.\n'
                f'[project]\nname = "robigo-package-suite"\nr_version = "4.5"\n'
                f'repositories = [{{ alias = "ppm", url = "{url}" }}]\ndependencies = [\n{deps},\n]\n')
    work = tempfile.mkdtemp(prefix="package-suite-lock")
    shutil.copy(PROJECT, work)
    if os.path.exists(LOCK):
        os.remove(LOCK)
    subprocess.run(["rv", "plan"], cwd=work, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["rv", "sync"], cwd=work, check=True, stdout=subprocess.DEVNULL,
                   env=dict(os.environ, RV_CACHE_DIR=os.path.join(CACHE, "rv")))
    shutil.copy(os.path.join(work, "rv.lock"), LOCK)
    versions = {p["name"]: p["version"] for p in locked()["packages"]}
    with open(SOURCES, "w") as f:
        f.write("# Source tarballs of the selection at the snapshot: rv's binaries carry no\n"
                "# tests, and rv.lock no checksums. Written by `scripts/package-suite.py lock`.\n")
        for name in sel:
            tar = source_tarball(name, versions[name])
            f.write(f"{name}\t{versions[name]}\t{file_sha(tar)}\n")
    hand_back(CACHE, PROJECT, LOCK, SOURCES)
    print(f"{len(versions)} packages locked at snapshot {a.snapshot}")


def source_tarball(name, version):
    dest = os.path.join(CACHE, "src", f"{name}_{version}.tar.gz")
    if not os.path.exists(dest):
        fetch(f"{PPM}/{snapshot()}/src/contrib/{name}_{version}.tar.gz", dest)
    return dest


def sources():
    out = {}
    for line in open(SOURCES):
        if not line.startswith("#"):
            name, version, digest = line.split()
            out[name] = (version, digest)
    return out


def source_dir(name):
    version, digest = sources()[name]
    dest = os.path.join(CACHE, "tree", f"{name}_{version}")
    if not os.path.isdir(dest):
        tar = source_tarball(name, version)
        if file_sha(tar) != digest:
            die(f"{tar} does not match sources.tsv")
        tmp = tempfile.mkdtemp(dir=CACHE)
        with tarfile.open(tar) as t:
            t.extractall(tmp, filter="data")
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        os.replace(os.path.join(tmp, name), dest)
        shutil.rmtree(tmp)
    return dest


def cmd_sync(_):
    sync()
    hand_back(CACHE)


def sync():
    """The pinned library, the canary, and every selected package's sources."""
    project = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(library())))))
    os.makedirs(project, exist_ok=True)
    shutil.copy(PROJECT, project)
    shutil.copy(LOCK, project)
    subprocess.run(["rv", "sync", "--locked"], cwd=project, check=True, stdout=subprocess.DEVNULL,
                   env=dict(os.environ, RV_CACHE_DIR=os.path.join(CACHE, "rv")))
    lib = canary_library()
    if not os.path.isdir(os.path.join(lib, "pkgsuitecanary")):
        os.makedirs(lib, exist_ok=True)
        subprocess.run(["R", "CMD", "INSTALL", f"--library={lib}", CANARY_SRC], check=True,
                       capture_output=True, env=dict(os.environ, R_LIBS=library()))
    for name in selection():
        source_dir(name)


# ------------------------------------------------------------------- workers

# Tests run without a network, with a working loopback: the results must not
# depend on what the internet does that day. argv: the test server to start
# first as JSON (`server_of`, or null), then the worker's command. The server
# stays in the worker's process group, so the supervisor counts its memory
# and kills it with the file.
OFFLINE = """
import fcntl, json, os, socket, struct, subprocess, sys, time
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
try:
    fcntl.ioctl(s, 0x8914, struct.pack("16sh", b"lo", 0x1 | 0x8 | 0x40))  # SIOCSIFFLAGS: up, loopback, running
except PermissionError:
    # outside a namespace of its own (the unit tests): fine if lo is up
    if not struct.unpack("16sh", fcntl.ioctl(s, 0x8913, struct.pack("16sh", b"lo", 0)))[1] & 0x1:
        raise
server = json.loads(sys.argv[1])
if server:
    with open(server["log"], "wb") as out:
        p = subprocess.Popen(server["cmd"], stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT)
    deadline = time.time() + server["limit"]
    while True:
        try:
            socket.create_connection(("127.0.0.1", server["port"]), timeout=1).close()
            break
        except OSError:
            if p.poll() is not None or time.time() > deadline:
                p.kill()
                tail = open(server["log"], errors="replace").read()[-300:]
                sys.exit(f"package-suite: the test server {server['cmd'][0]} did not start on port "
                         f"{server['port']}: {tail}")
            time.sleep(0.05)
os.execvp(sys.argv[2], sys.argv[2:])
"""


def engine_cmd(engine):
    r = "R" if engine == "gnur" else ROBIGO
    return [r, "--vanilla", "--quiet", "--no-echo", "-f", RUNNER]


def offline(cmd, server=None):
    """`cmd` in a network namespace of its own, after `server` is listening on
    its loopback."""
    return ["unshare", "-n"] + in_namespace(cmd, server)


def in_namespace(cmd, server=None):
    return ["python3", "-I", "-c", OFFLINE, json.dumps(server)] + cmd


def server_of(row, log):
    """The test server of a selection row: (argument of `offline`, its environment)."""
    name = row.get("server", "")
    if not name:
        return None, {}
    if name not in SERVERS:
        die(f"unknown test server {name!r} in selection.tsv")
    spec = SERVERS[name]
    return {"port": spec["port"], "cmd": spec["cmd"], "log": log, "limit": SERVER_START_LIMIT}, spec["env"]


def check_hosts(names):
    """The test servers' names must lead to the loopback, or their packages
    would score whatever the probes of their helpers find."""
    for server in sorted({row.get("server", "") for row in names.values()} - {""}):
        if server not in SERVERS:
            die(f"unknown test server {server!r} in selection.tsv")
        for host in SERVERS[server]["hosts"]:
            try:
                addr = socket.gethostbyname(host)
            except OSError:
                addr = None
            if addr != "127.0.0.1":
                die(f"{host} resolves to {addr}, not 127.0.0.1: the oracle needs `--add-host {host}:127.0.0.1` "
                    f"(the {server} test server)")


def group_rss(pgid):
    total = 0
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/stat") as f:
                fields = f.read().rsplit(")", 1)[1].split()
        except OSError:
            continue
        if int(fields[2]) == pgid:
            total += int(fields[21]) * 4096
    return total


def supervise(cmd, env, cwd, log, limit, errlog=None):
    """Run `cmd` as a process group; returns (status, returncode, seconds).
    With `errlog`, stderr goes there instead of into `log` (the smoke
    tier compares the two streams separately)."""
    start = time.time()
    out = open(log, "wb")
    err = open(errlog, "wb") if errlog else subprocess.STDOUT
    p = subprocess.Popen(cmd, env=env, cwd=cwd, stdout=out, stderr=err, start_new_session=True)
    out.close()
    if errlog:
        err.close()
    status = None
    while p.poll() is None:
        time.sleep(0.25)
        if time.time() - start > limit:
            status = "timeout"
        elif group_rss(p.pid) > MEMORY_CAP:
            status = "oom"
        if status:
            break
    if status:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    rc = p.wait()
    try:
        os.killpg(p.pid, signal.SIGKILL)  # whatever the file left behind
    except ProcessLookupError:
        pass
    return status, rc, time.time() - start


def parse_events(path):
    """Blocks of a worker's event stream, and how the stream ended."""
    # a stack: describe() and it() nest, and a parent's results after a
    # child are the parent's
    blocks, seen, stack, ended, abort = [], {}, [], None, None

    def block(test):
        n = seen[test] = seen.get(test, 0) + 1
        b = {"test": test, "n": n, "pass": 0, "fail": 0, "error": 0, "skip": 0, "warn": 0,
             "reasons": [], "msg": None, "open": True}
        blocks.append(b)
        return b

    if os.path.exists(path):
        for line in open(path, encoding="utf8", errors="replace"):
            try:
                ev = json.loads(line)
            except ValueError:
                continue  # the line being written when the process died
            e = ev["e"]
            if e == "start":
                stack.append(block(ev["test"]))
            elif e == "end":
                if stack:
                    stack.pop()["open"] = False
            elif e == "result":
                cur = stack[-1] if stack else None
                b = cur or block("(outside any test)")
                k = {"success": "pass", "failure": "fail", "error": "error", "skip": "skip",
                     "warning": "warn"}[ev["kind"]]
                b[k] += 1
                if k == "skip":
                    b["reasons"].append(normalize(ev.get("msg", "")))
                elif k in ("fail", "error") and b["msg"] is None:
                    b["msg"] = ev.get("msg", "")
                if cur is None:
                    b["open"] = False
            elif e == "abort":
                abort = ev.get("msg", "")
            elif e == "done":
                ended = True
    return blocks, bool(ended), abort


def normalize(msg):
    """A message without what differs from run to run."""
    m = re.sub(r"0x[0-9a-f]+", "0x", msg or "")
    m = re.sub(r"/[\w./+-]*/(Rtmp\w+|tmp\w*|file\w+)\S*", "<tmp>", m)
    return re.sub(r"\s+", " ", m).strip()[:300]


def run_file(engine, pkg, row, path, lib, limit):
    """One test file in one fresh process; (file status, seconds, blocks, abort message)."""
    with tempfile.TemporaryDirectory(prefix="pkgsuite-") as tmp:
        tests = os.path.join(tmp, "tests")
        shutil.copytree(os.path.dirname(path), tests, symlinks=True)
        home = os.path.join(tmp, "home")
        os.mkdir(home)
        events = os.path.join(tmp, "events.jsonl")
        env = dict(os.environ, **ENV, HOME=home, TMPDIR=tmp, R_LIBS=lib, PKG_SUITE_PKG=pkg,
                   PKG_SUITE_FILE=os.path.join(tests, os.path.basename(path)), PKG_SUITE_OUT=events,
                   PKG_SUITE_FRAMEWORK=row["framework"])
        env.pop("R_HOME", None)
        server, server_env = server_of(row, os.path.join(tmp, "server.log"))
        env.update(server_env)
        for kv in filter(None, row.get("env", "").split(",")):
            k, v = kv.split("=", 1)
            env[k] = v
        status, rc, secs = supervise(offline(engine_cmd(engine), server), env, tests, os.path.join(tmp, "log"), limit)
        keep = os.environ.get("ROBIGO_PKG_KEEP_LOGS")
        if keep:
            # the worker's own output (compiler errors, crashes) for a look later
            os.makedirs(keep, exist_ok=True)
            name = f"{engine}-{pkg}-{file_key(path)}"
            shutil.copy(os.path.join(tmp, "log"), os.path.join(keep, f"{name}.log"))
            if server:
                shutil.copy(os.path.join(tmp, "server.log"), os.path.join(keep, f"{name}.server.log"))
        blocks, done, abort = parse_events(events)
        if status is None:
            if rc == 0 and done:
                status = "done"
            elif abort is not None:
                status = "setup" if not blocks else "abort"
            elif rc < 0 or rc > 128:
                status = "crash"
            else:
                status = "abort"
                with open(os.path.join(tmp, "log"), errors="replace") as f:
                    abort = f.read()[-400:]
        return status, secs, blocks, abort


def test_files(pkg, row):
    tree = source_dir(pkg)
    d = os.path.join(tree, "tests", "testthat") if row["framework"] == "testthat" else \
        os.path.join(tree, "inst", "tinytest")
    pattern = re.compile(r"^test.*\.[rR]$")
    names = sorted(f for f in os.listdir(d) if pattern.match(f))
    if row.get("filter"):
        keep = re.compile(row["filter"])
        names = [f for f in names if keep.search(re.sub(r"^test-?|\.[rR]$", "", f))]
    return [os.path.join(d, f) for f in names]


def file_key(path):
    return re.sub(r"\.[rR]$", "", os.path.basename(path))


def block_id(file, b):
    return f"{file}/{sha(b['test'] + chr(0) + str(b['n']))}"


def outcome(b):
    return [b["pass"], b["fail"], b["error"], b["skip"], b["warn"], sorted(b["reasons"])]


def jobs():
    """Workers at once: one per core, or $ROBIGO_PKG_JOBS. Each may use up to
    MEMORY_CAP, so a machine with less than that per core wants fewer."""
    return max(1, int(os.environ.get("ROBIGO_PKG_JOBS") or os.cpu_count() or 1))


def run_package(engine, pkg, row, lib, limits=None, only=None):
    """Every file of a package: {file: (status, seconds, blocks, abort)}."""
    paths = test_files(pkg, row)
    if only is not None:
        paths = [p for p in paths if file_key(p) in only]
    limits = limits or {}
    deadline = time.time() + PACKAGE_BUDGET

    def one(path):
        key = file_key(path)
        if time.time() > deadline:
            return key, ("budget", 0.0, [], None)
        limit = max(MIN_FILE_LIMIT, FILE_LIMIT_FACTOR * limits[key]) if key in limits else RECORD_FILE_LIMIT
        return key, run_file(engine, pkg, row, path, lib, limit)

    with concurrent.futures.ThreadPoolExecutor(jobs()) as ex:
        return dict(ex.map(one, paths))


# --------------------------------------------------------------- GNU R's side

def expected_path(pkg):
    return os.path.join(EXPECTED, f"{pkg}.jsonl.gz")


def cmd_record(a):
    in_oracle()
    sync()
    sel = selection()
    check_hosts({p: sel[p] for p in a.packages or sel})
    for pkg in a.packages or sel:
        runs = []
        for i in range(a.runs):
            runs.append(run_package("gnur", pkg, sel[pkg], library()))
            blocks = sum(len(r[2]) for r in runs[-1].values())
            bad = {k: r[0] for k, r in runs[-1].items() if r[0] != "done"}
            print(f"{pkg}: GNU R run {i + 1}/{a.runs}: {blocks} blocks, "
                  f"{sum(r[1] for r in runs[-1].values()):.0f}s" + (f", not done: {bad}" if bad else ""), flush=True)
            if not blocks:
                die(f"GNU R ran no test of {pkg}: {bad}")
        write_expected(pkg, runs)
    hand_back(EXPECTED, CACHE)


def write_expected(pkg, runs):
    lines = []
    for file in sorted(runs[0]):
        first = runs[0][file]
        secs = max(r[file][1] for r in runs)
        status = [r[file][0] for r in runs]
        lines.append({"file": file, "seconds": round(secs, 1), "status": status[0],
                      "stable": len(set(status)) == 1})
        others = [{(b["test"], b["n"]): b for b in r[file][2]} for r in runs[1:]]
        for b in first[2]:
            same = all((b["test"], b["n"]) in o and outcome(o[(b["test"], b["n"])]) == outcome(b) for o in others)
            lines.append({"id": block_id(file, b), "file": file, "test": b["test"], "n": b["n"],
                          "outcome": outcome(b), "unstable": not same or b["open"]})
    os.makedirs(EXPECTED, exist_ok=True)
    tmp = expected_path(pkg) + ".tmp"
    with open(tmp, "wb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as f:
        for line in lines:
            f.write((json.dumps(line, sort_keys=True, ensure_ascii=False) + "\n").encode())
    os.replace(tmp, expected_path(pkg))


def read_expected(pkg):
    """(blocks by id, {file: {seconds, status, stable}})."""
    if not os.path.exists(expected_path(pkg)):
        die(f"no recording for {pkg}: scripts/package-suite.py record {pkg}")
    blocks, files = {}, {}
    with gzip.open(expected_path(pkg), "rt", encoding="utf8") as f:
        for line in f:
            d = json.loads(line)
            if "id" in d:
                blocks[d["id"]] = d
            else:
                files[d["file"]] = d
    return blocks, files


# ------------------------------------------------------------------- scoring

def scored_block(e):
    """Is this GNU R block part of the score? It must run, and run clean."""
    p, f, err, s, _, _ = e["outcome"]
    return not e["unstable"] and f == 0 and err == 0 and not (p == 0 and s > 0)


def compare(exp, exp_files, got):
    """Compare a run with the recording; one dict per expected block."""
    rows = {}
    seen = {}
    for file, (status, _, blocks, abort) in got.items():
        for b in blocks:
            seen[block_id(file, b)] = b
    for bid, e in exp.items():
        file = e["file"]
        status, _, blocks, abort = got.get(file, ("notrun", 0, [], None))
        b = seen.get(bid)
        if b is None:
            state, why = "notrun", f"{status}: {normalize(abort)}" if abort else f"file: {status}"
        elif b["open"]:
            state, why = "interrupted", f"file: {status}"
        elif outcome(b) == e["outcome"]:
            state, why = "match", None
        else:
            state, why = "differ", normalize(b["msg"]) or describe(e["outcome"], outcome(b))
        rows[bid] = {"state": state, "why": why, "scored": scored_block(e), "expected": e}
    extra = [k for k in seen if k not in exp]
    return rows, extra


def describe(want, have):
    names = ("passed", "failed", "errored", "skipped", "warned")
    parts = [f"{n} {h} (GNU R {w})" for n, w, h in zip(names, want, have) if w != h]
    return ", ".join(parts) or f"skip reason: {have[5]} (GNU R {want[5]})"


def marker(pkg, bid):
    return os.path.join(PASSING, pkg, bid + ".pass")


def markers(pkg):
    root = os.path.join(PASSING, pkg)
    out = set()
    for d, _, files in os.walk(root):
        for f in files:
            if f.endswith(".pass"):
                out.add(os.path.relpath(os.path.join(d, f), root)[:-5].replace(os.sep, "/"))
    return out


def own_skips(pkg, version):
    """The skip reasons `pkg`'s own code always takes (own-skips.tsv)."""
    out = set()
    with open(OWN_SKIPS) as f:
        lines = [line.rstrip("\n").split("\t") for line in f if line.strip() and not line.startswith("#")]
    for p, v, reason in lines[1:]:
        if p == pkg and v == version:
            out.add(reason)
    return out


def own_skip(e, reasons):
    """Did GNU R skip this block, steadily, for a reason of the package's own?"""
    p, f, err, s, _, why = e["outcome"]
    return not e["unstable"] and p == f == err == 0 and s > 0 and bool(why) and set(why) <= reasons


def summarize(pkg, row, exp, exp_files, got):
    rows, extra = compare(exp, exp_files, got)
    # the floor guards against what this machine cannot run, not against
    # tests the package's authors switched off everywhere
    reasons = own_skips(pkg, row["version"])
    own = sum(1 for r in rows.values() if own_skip(r["expected"], reasons))
    scored = {k: r for k, r in rows.items() if r["scored"]}
    matching = {k for k, r in scored.items() if r["state"] == "match"}
    diverged = [k for k, r in rows.items() if not r["scored"] and not r["expected"]["unstable"]
                and r["state"] in ("differ", "interrupted") and r["expected"]["outcome"][0:3] == [0, 0, 0]]
    files = {}
    for status, *_ in got.values():
        files[status] = files.get(status, 0) + 1
    causes = {}
    for k, r in scored.items():
        if k not in matching:
            causes[r["why"]] = causes.get(r["why"], 0) + 1
    # a block GNU R does not have (an error outside any test, say) is a
    # difference too, though it has nothing to be scored against
    green = bool(scored) and len(matching) == len(scored) and not diverged and not extra and \
        set(files) <= {"done"} and len(scored) >= GREEN_FLOOR * (len(rows) - own)
    return {"v": row["version"], "m": len(matching), "n": len(scored), "blocks": len(rows), "own_skips": own,
            "unstable": sum(1 for r in rows.values() if r["expected"]["unstable"]),
            "skip_diverged": len(diverged), "extra": len(extra), "files": files,
            "s": round(sum(g[1] for g in got.values()), 1),
            "gnu_s": round(sum(f["seconds"] for f in exp_files.values()), 1), "green": green}, matching, causes, rows


# ------------------------------------------------------------- robigo's side

def check_canary():
    """[matching, total] of the harness's own known outcomes."""
    row = {"framework": "testthat", "filter": "", "env": ""}
    path = os.path.join(CANARY_SRC, "tests", "testthat", "test-canary.R")
    status, _, blocks, abort = run_file("robigo", "pkgsuitecanary", row, path,
                                        f"{canary_library()}:{library()}", 300)
    if abort:
        print(f"canary: {status}: {normalize(abort)}", flush=True)
    return canary_score(status, blocks)


def canary_score(status, blocks):
    """[matching, total]: a test counts when it ran to its end with the
    expected outcome, in a file that finished, with nothing else in it."""
    have = {b["test"]: tuple(outcome(b)[:5]) for b in blocks if not b["open"]}
    bad = {t: have.get(t) for t, want in CANARY.items() if have.get(t) != want}
    for t, h in bad.items():
        print(f"canary: {t!r}: expected {CANARY[t]}, got {h}", flush=True)
    extra = [b["test"] for b in blocks if b["test"] not in CANARY]
    if status != "done" or extra or len(blocks) != len(CANARY):
        print(f"canary: file {status}, unexpected blocks {extra}", flush=True)
        return [0, len(CANARY)]
    return [len(CANARY) - len(bad), len(CANARY)]


def closure():
    """The selection and everything it needs to install."""
    by_name = {p["name"]: p for p in locked()["packages"]}
    seen, stack = set(), list(selection())
    while stack:
        n = stack.pop()
        if n in seen or n not in by_name:
            continue
        seen.add(n)
        for d in by_name[n].get("dependencies", []):
            stack.append(d["name"] if isinstance(d, dict) else d)
    return sorted(seen)


def check_loads(lib):
    """Packages of `closure()` robigo loads, each in a fresh process."""
    def one(name):
        env = dict(os.environ, **ENV, R_LIBS=lib)
        env.pop("R_HOME", None)
        code = f'loadNamespace("{name}")'
        try:
            p = subprocess.run([ROBIGO, "--vanilla", "--quiet", "--no-echo", "-e", code], env=env,
                               capture_output=True, timeout=120)
            return name, p.returncode == 0
        except subprocess.TimeoutExpired:
            return name, False

    with concurrent.futures.ThreadPoolExecutor(jobs()) as ex:
        return dict(ex.map(one, closure()))


def shard_of(packages, shard):
    """Greedy balance on GNU R's seconds: robigo's cost follows the suite's size."""
    i, n = (int(x) for x in shard.split("/"))
    bins = [[0.0, []] for _ in range(n)]
    weight = {p: sum(f["seconds"] for f in read_expected(p)[1].values()) for p in packages}
    for p in sorted(packages, key=lambda p: -weight[p]):
        b = min(bins, key=lambda x: x[0])
        b[0] += weight[p]
        b[1].append(p)
    return [p for p in packages if p in bins[i - 1][1]]


def cmd_run(a):
    in_oracle()
    sync()
    sel = selection()
    names = a.packages or list(sel)
    if a.shard:
        names = shard_of(names, a.shard)
    check_hosts({p: sel[p] for p in names})
    i, n = (int(x) for x in a.shard.split("/")) if a.shard else (1, 1)
    canary = check_canary()
    valid = canary[0] == canary[1]
    print(f"canary {canary[0]}/{canary[1]}" + ("" if valid else ": this run cannot move the ratchet"), flush=True)
    result = {"kind": "packages", "snapshot": snapshot(), "lock": lock_id(), "canary": canary, "valid": valid,
              "shard": [i, n], "selection": len(sel), "pkgs": {}, "causes": {}, "regressions": []}
    for pkg in names:
        exp, exp_files = read_expected(pkg)
        limits = {f: d["seconds"] for f, d in exp_files.items()}
        got = run_package("robigo", pkg, sel[pkg], library(), limits)
        s, matching, causes, rows = summarize(pkg, sel[pkg], exp, exp_files, got)
        # a marker whose block is not in this recording is from another
        # snapshot: it cannot regress here (README: moving to a new snapshot)
        stale = sorted(b for b in markers(pkg) - matching if b not in exp)
        if stale:
            print(f"{pkg}: {len(stale)} markers have no recorded block (another snapshot?)", flush=True)
        lost = sorted(b for b in markers(pkg) - matching if b in exp)
        if lost:
            # Twice before it counts: a difference that does not repeat is
            # flakiness, itself worth a look, not a regression.
            again = run_package("robigo", pkg, sel[pkg], library(), limits, {exp[b]["file"] for b in lost})
            s2, match2, *_ = summarize(pkg, sel[pkg], exp, exp_files, dict(got, **again))
            fixed = [b for b in lost if b in match2]
            s["flaky"] = len(fixed)
            lost = [b for b in lost if b not in match2]
        result["pkgs"][pkg] = s
        result.setdefault("matching", {})[pkg] = sorted(matching)
        result["regressions"] += [f"{pkg}/{b}" for b in lost]
        for k, v in causes.items():
            c = result["causes"].setdefault(k, [0, []])
            c[0] += v
            c[1].append(pkg)
        print(f"{pkg}: {s['m']}/{s['n']} blocks match GNU R, files {s['files']}, {s['s']:.0f}s "
              f"(GNU R {s['gnu_s']:.0f}s)" + (" GREEN" if s["green"] else "") +
              (f", {len(lost)} REGRESSED" if lost else ""), flush=True)
        if a.update and valid:
            add_markers(pkg, matching)
    if i == 1 and not a.packages:
        loads = check_loads(library())
        result["loads"] = sorted(k for k, v in loads.items() if v)
        result["closure"] = sorted(loads)
        print(f"loads {len(result['loads'])}/{len(loads)}", flush=True)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    result["causes"] = sorted(([k, v[0], sorted(v[1])] for k, v in result["causes"].items()), key=lambda c: -c[1])
    with open(a.out, "w") as f:
        json.dump(result, f, indent=1, sort_keys=True)
    hand_back(a.out, PASSING, CACHE)
    total = [sum(p[k] for p in result["pkgs"].values()) for k in ("m", "n")]
    print(f"{total[0]:,} of {total[1]:,} blocks match GNU R; "
          f"{sum(p['green'] for p in result['pkgs'].values())} of {len(result['pkgs'])} packages green")
    for b in result["regressions"][:50]:
        print(f"REGRESSED {b}")
    sys.exit(1 if result["regressions"] or not valid else 0)


def add_markers(pkg, ids):
    exp, _ = read_expected(pkg)
    for b in sorted(ids):
        if b not in exp:
            die(f"{pkg}/{b} is not a recorded block (a result of another snapshot?)")
        os.makedirs(os.path.dirname(marker(pkg, b)), exist_ok=True)
        open(marker(pkg, b), "w").close()


def cmd_absorb(a):
    """Markers for what a run matched, from its result files (the nightly's
    `packages-*` artifacts): no need to run the suites again locally."""
    added = 0
    for path in a.results:
        with open(path) as f:
            r = json.load(f)
        if r.get("kind") != "packages":
            die(f"{path} is not a package-suite result")
        if not r["valid"]:
            die(f"{path}: the canary failed in that run ({r['canary'][0]}/{r['canary'][1]})")
        if r["lock"] != lock_id():
            die(f"{path} was measured against another rv.lock")
        for pkg, ids in r.get("matching", {}).items():
            new = set(ids) - markers(pkg)
            add_markers(pkg, new)
            added += len(new)
    print(f"{added} new markers")


def cmd_show(a):
    """One package's differences; `--file` narrows, `--all` lists matches too."""
    in_oracle()
    sync()
    sel = selection()
    check_hosts({a.package: sel[a.package]})
    exp, exp_files = read_expected(a.package)
    limits = {f: d["seconds"] for f, d in exp_files.items()}
    only = {f for f in exp_files if fnmatch.fnmatch(f, a.file)} if a.file else None
    got = run_package("robigo", a.package, sel[a.package], library(), limits, only)
    exp = {k: v for k, v in exp.items() if only is None or v["file"] in only}
    _, _, causes, rows = summarize(a.package, sel[a.package], exp, exp_files, got)
    for k, r in rows.items():
        if a.all or r["state"] != "match":
            print(f"{r['state']:<12}{'' if r['scored'] else '(unscored) '}{r['expected']['file']}: "
                  f"{r['expected']['test']}\n    {r['why'] or ''}")
    for cause, count in sorted(causes.items(), key=lambda c: -c[1])[:20]:
        print(f"{count:6} {cause}")
    hand_back(CACHE)


def cmd_install(a):
    """Tier two: the closure from source through `robigo CMD INSTALL`.

    What the test score does not cover: the binaries the suites run on are
    GNU R's builds. Installs in dependency order; a package whose dependency
    failed counts as failed without being tried.
    """
    in_oracle()
    sync()
    by_name = {p["name"]: p for p in locked()["packages"]}
    names = a.packages or closure()
    lib = os.path.join(CACHE, f"source-{lock_id()}")
    os.makedirs(lib, exist_ok=True)
    state, order = {}, []

    def visit(n):
        if n in order or n not in by_name:
            return
        for d in by_name[n].get("dependencies", []):
            visit(d["name"] if isinstance(d, dict) else d)
        order.append(n)

    for n in names:
        visit(n)
    for n in order:
        dep_names = [d["name"] if isinstance(d, dict) else d for d in by_name[n].get("dependencies", [])]
        bad = [d for d in dep_names if state.get(d) not in (None, "ok")]
        if bad:
            state[n] = f"dependency {bad[0]}"
        else:
            tar = source_tarball(n, by_name[n]["version"])
            env = dict(os.environ, **ENV, MAKEFLAGS=f"-j{jobs()}", R_LIBS=lib)
            env.pop("R_HOME", None)
            with tempfile.TemporaryDirectory(prefix="pkgsuite-") as tmp:
                log = os.path.join(tmp, "log")
                status, rc, secs = supervise([ROBIGO, "CMD", "INSTALL", f"--library={lib}", tar], env, tmp, log, 3600)
                if status is None and rc == 0:
                    state[n] = "ok"
                else:
                    state[n] = status or normalize(open(log, errors="replace").read()[-300:])
        print(f"{n}: {state[n]}", flush=True)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    with open(a.out, "w") as f:
        json.dump({"kind": "package-install", "lock": lock_id(), "state": state}, f, indent=1, sort_keys=True)
    hand_back(a.out, CACHE)
    print(f"{sum(v == 'ok' for v in state.values())} of {len(state)} install from source")


# ------------------------------------------------------------ smoke scripts

def smoke_cases():
    """{name: (path, needs, limit)} of tests/packages/smoke/cases/*.R, in
    name order. Leading comment lines carry the directives:
    `# needs: pkg ...` (the case is `absent`, unscored, where a package
    is not in the library) and `# timeout: SECONDS`."""
    cases = {}
    for fn in sorted(os.listdir(SMOKE_CASES)) if os.path.isdir(SMOKE_CASES) else []:
        if not fn.endswith(".R"):
            continue
        path = os.path.join(SMOKE_CASES, fn)
        needs, limit = [], SMOKE_LIMIT
        for line in open(path):
            m = re.match(r"# (needs|timeout): (.+)", line)
            if not line.startswith("#"):
                break
            if m and m.group(1) == "needs":
                needs = m.group(2).split()
            elif m:
                limit = int(m.group(2))
        cases[fn[:-2]] = (path, needs, limit)
    return cases


def smoke_expected(name):
    """(stdout, stderr, status) GNU R recorded for a smoke case, or None."""
    base = os.path.join(SMOKE_EXPECTED, name)
    if not all(os.path.exists(base + ext) for ext in (".stdout", ".stderr", ".status")):
        return None
    return (open(base + ".stdout", "rb").read(), open(base + ".stderr", "rb").read(),
            int(open(base + ".status").read().strip()))


def smoke_mask(data, tmp):
    """A transcript without what names the run: object addresses and the
    temporary directory. Everything else must match byte for byte: that
    equality is the smoke tier's whole claim."""
    s = data.decode("utf8", errors="replace")
    s = re.sub(r"0x[0-9a-fA-F]+", "0x", s)
    # Rtmp paths first: replacing the tmp root first would leave
    # "<tmp>/RtmpXXXX" behind, which the path pattern no longer matches
    s = re.sub(r"/[\w./+-]*/(Rtmp\w+|file[0-9a-f]+)\S*", "<tmp>", s)
    if tmp:
        s = s.replace(tmp, "<tmp>")
    return s


def smoke_state(expected, got, status, rc):
    """The outcome of one smoke case. `expected` is smoke_expected()'s
    triple, `got` the (stdout, stderr) robigo produced, already masked
    (expected is masked the same way by the caller), `status`/`rc` what
    supervise returned. A case passes exactly when stdout, stderr and
    exit status all equal GNU R's recorded ones: a script whose point
    is that it errors (rlang-abort) passes by erroring identically,
    and one that hangs where GNU R exits is a timeout, never a pass."""
    if status in ("timeout", "oom"):
        return status
    if rc < 0:
        return "crash"
    want_out, want_err, want_rc = expected
    if got == (want_out, want_err) and rc == want_rc:
        return "pass"
    return "diff"


def smoke_marker(name):
    return os.path.join(SMOKE_PASSING, f"{name}.pass")


def run_smoke_case(engine, name, path, lib, limit):
    """One smoke script in one fresh process, like `Rscript FILE`;
    (status, rc, seconds, stdout, stderr), the streams masked."""
    with tempfile.TemporaryDirectory(prefix="pkgsmoke-") as tmp:
        home = os.path.join(tmp, "home")
        os.mkdir(home)
        script = os.path.join(tmp, f"{name}.R")
        shutil.copy(path, script)
        out_log, err_log = os.path.join(tmp, "stdout"), os.path.join(tmp, "stderr")
        keep = os.environ.get("ROBIGO_PKG_KEEP_LOGS")
        if keep and engine != "gnur":
            os.makedirs(keep, exist_ok=True)
            out_log = os.path.join(keep, f"smoke-{name}.stdout")
            err_log = os.path.join(keep, f"smoke-{name}.stderr")
        env = dict(os.environ, **ENV, HOME=home, TMPDIR=tmp, R_LIBS=lib,
                   R_DEFAULT_PACKAGES=SMOKE_DEFAULT_PACKAGES)
        env.pop("R_HOME", None)
        cmd = ["Rscript", "--vanilla", script] if engine == "gnur" else [ROBIGO, script]
        status, rc, secs = supervise(offline(cmd), env, tmp, out_log, limit, errlog=err_log)
        out = open(out_log, "rb").read() if os.path.exists(out_log) else b""
        err = open(err_log, "rb").read() if os.path.exists(err_log) else b""
        return status, rc, secs, smoke_mask(out, tmp), smoke_mask(err, tmp)


def cmd_smoke(a):
    """User-level scripts against the pinned library, transcript-checked.

    What the block score cannot say: whether a script that attaches
    packages and does representative work produces GNU R's stdout,
    stderr and exit status, in seconds. --record runs GNU R only and
    is the only writer of expected/ (as `record` is for blocks). A
    case in passing/ that stops passing fails the run; one that was
    never marked is a known gap, listed as pending."""
    # --keep-logs before in_oracle(): the re-exec into docker mounts
    # and forwards ROBIGO_PKG_KEEP_LOGS only when the host has it set.
    if getattr(a, "keep_logs", None):
        os.environ["ROBIGO_PKG_KEEP_LOGS"] = a.keep_logs
    in_oracle()
    sync()
    lib = library()
    installed = set(os.listdir(lib)) if os.path.isdir(lib) else set()
    cases = smoke_cases()
    names = a.cases or list(cases)
    unknown = [n for n in names if n not in cases]
    if unknown:
        die(f"unknown smoke case(s): {', '.join(unknown)} (in {SMOKE_CASES})")
    result = {"kind": "smoke", "snapshot": snapshot(), "lock": lock_id(), "cases": {}}
    states = {}
    for name in names:
        path, needs, limit = cases[name]
        missing = [p for p in needs if p not in installed and p not in BASE_PACKAGES]
        if missing:
            states[name] = "absent"
            print(f"{name}: absent (needs {', '.join(missing)})", flush=True)
            continue
        if a.record:
            status, rc, secs, out, err = run_smoke_case("gnur", name, path, lib, limit)
            if status is not None or rc < 0:
                die(f"{name}: GNU R did not finish ({status or rc}); fix the case, then record")
            os.makedirs(SMOKE_EXPECTED, exist_ok=True)
            base = os.path.join(SMOKE_EXPECTED, name)
            # expected streams are stored masked, the form runs compare in
            open(base + ".stdout", "w", encoding="utf8").write(out)
            open(base + ".stderr", "w", encoding="utf8").write(err)
            open(base + ".status", "w", encoding="utf8").write(f"{rc}\n")
            states[name] = "recorded"
            print(f"{name}: recorded from GNU R ({secs:.0f}s, status {rc})", flush=True)
            continue
        exp = smoke_expected(name)
        if exp is None:
            states[name] = "norecord"
            print(f"{name}: no expected transcript; record it with `smoke --record {name}`", flush=True)
            continue
        exp = (exp[0].decode("utf8", errors="replace"), exp[1].decode("utf8", errors="replace"), exp[2])
        status, rc, secs, out, err = run_smoke_case("robigo", name, path, lib, limit)
        state = smoke_state(exp, (out, err), status, rc)
        states[name] = state
        if state == "pass" and a.update:
            os.makedirs(SMOKE_PASSING, exist_ok=True)
            open(smoke_marker(name), "w").close()
        print(f"{name}: {state} ({secs:.0f}s)", flush=True)
        result["cases"][name] = {"state": state, "seconds": round(secs, 2),
                                  "marked": os.path.exists(smoke_marker(name))}
    for name, state in states.items():
        result["cases"].setdefault(name, {"state": state, "marked": os.path.exists(smoke_marker(name))})
    scored = [n for n, s in states.items() if s not in ("absent", "norecord", "recorded")]
    result["pass"] = sum(1 for n in scored if states[n] == "pass")
    result["scored"] = len(scored)
    pending = sorted(n for n, s in states.items()
                       if s not in ("pass", "absent", "recorded") and not os.path.exists(smoke_marker(n)))
    regressed = sorted(n for n, s in states.items()
                        if s not in ("pass", "absent", "recorded") and os.path.exists(smoke_marker(n)))
    result["pending"] = pending
    result["regressions"] = regressed
    if not a.record:
        os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
        with open(a.out, "w") as f:
            json.dump(result, f, indent=1, sort_keys=True)
        hand_back(a.out, SMOKE_PASSING, CACHE)
        print(f"smoke: {result['pass']} of {result['scored']} cases match GNU R"
              + (f"; pending: {', '.join(pending)}" if pending else ""))
        for n in regressed:
            print(f"REGRESSED smoke/{n} ({states[n]})")
        sys.exit(1 if regressed else 0)
    hand_back(SMOKE_EXPECTED, CACHE)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("select")
    p.add_argument("--snapshot", required=True)
    p.set_defaults(fn=cmd_select)
    p = sub.add_parser("lock")
    p.add_argument("--snapshot", required=True)
    p.set_defaults(fn=lambda a: (in_oracle(), cmd_lock(a)))
    p = sub.add_parser("sync")
    p.set_defaults(fn=lambda a: (in_oracle(), cmd_sync(a)))
    p = sub.add_parser("record")
    p.add_argument("packages", nargs="*")
    p.add_argument("--runs", type=int, default=GNU_RUNS)
    p.set_defaults(fn=cmd_record)
    p = sub.add_parser("run")
    p.add_argument("packages", nargs="*")
    p.add_argument("--update", action="store_true")
    p.add_argument("--shard")
    p.add_argument("--out", default=os.path.join(CACHE, "results", "packages.json"))
    p.set_defaults(fn=cmd_run)
    p = sub.add_parser("absorb")
    p.add_argument("results", nargs="+")
    p.set_defaults(fn=cmd_absorb)
    p = sub.add_parser("show")
    p.add_argument("package")
    p.add_argument("--file")
    p.add_argument("--all", action="store_true")
    p.set_defaults(fn=cmd_show)
    p = sub.add_parser("install")
    p.add_argument("packages", nargs="*")
    p.add_argument("--out", default=os.path.join(CACHE, "results", "package-install.json"))
    p.set_defaults(fn=cmd_install)
    p = sub.add_parser("smoke")
    p.add_argument("cases", nargs="*")
    p.add_argument("--record", action="store_true")
    p.add_argument("--update", action="store_true")
    p.add_argument("--out", default=os.path.join(CACHE, "results", "smoke.json"))
    p.add_argument("--keep-logs")
    p.set_defaults(fn=cmd_smoke)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
