#!/usr/bin/env python3
"""Delete build artifacts that no build has used for N days from the shared
target directory (AGENTS.md, Machine).

    scripts/prune-target.py [-n] [--days N] [TARGET_DIR]

Cargo never deletes old artifacts. A unit (one crate, test, binary or build
script built with one set of features, flags and dependency versions) gets a
new hash whenever any of those change, and the worktrees sharing the
directory build many such variants, so the directory only grows.

A profile is a directory holding cargo's `.cargo-lock` (debug/, release/,
x86_64-unknown-linux-gnu/release/, miri/.../debug/, ...). In each profile
the script deletes every unit whose files no build has read or written for N
days (default 5):

    build/<pkg>/<hash>/                      one unit (cargo >= 1.100, nightly)
    .fingerprint/<pkg>-<hash>/, build/<pkg>-<hash>/, deps/<name>-<hash>*
                                             one unit (cargo < 1.100)
    incremental/<crate>-<id>/                one incremental cache

Every build reads the fingerprint files of every unit in its graph, even
when nothing recompiles, so a unit that some checkout still builds stays
fresh. Deleting a unit costs only a rebuild: cargo finds the fingerprint or
output missing and compiles it again. Each profile is pruned under an
exclusive lock on its `.cargo-lock`, which a running build holds, so a build
there finishes first. -n lists what would be deleted and takes no lock.

Freshness rests on reads updating access times, which Linux's default
`relatime` does at least once a day. On a filesystem mounted `noatime` every
unit would look stale, so the script refuses to run there.
"""

import argparse
import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HASH = re.compile(r"^[0-9a-f]{16}$")
LEGACY = re.compile(r"^(?P<stem>[^.]+)-(?P<hash>[0-9a-f]{16})(\..*)?$")


def listdir(d):
    """The entries of `d`, sorted; none if a concurrent build removed it."""
    try:
        return sorted(d.iterdir())
    except (FileNotFoundError, NotADirectoryError):
        return []


def last_use(path):
    """Newest access or modification of the regular files under `path`."""
    newest = 0.0
    try:
        st = path.lstat()
    except FileNotFoundError:
        return None
    if not path.is_dir() or path.is_symlink():
        return max(st.st_atime, st.st_mtime)
    for root, _, files in os.walk(path):
        for f in files:
            try:
                st = os.lstat(os.path.join(root, f))
            except FileNotFoundError:
                continue
            newest = max(newest, st.st_atime, st.st_mtime)
    if newest:
        return newest
    try:  # an empty unit counts from its own last change
        return path.lstat().st_mtime
    except FileNotFoundError:
        return None


def units(profile):
    """The units of one profile: lists of paths deleted together."""
    found = []
    legacy = {}  # hash -> paths
    build = profile / "build"
    for d in listdir(build):
        m = LEGACY.match(d.name)
        if m:
            legacy.setdefault(m["hash"], []).append(d)
        elif d.is_dir() and not d.is_symlink():
            found += [[u] for u in listdir(d) if HASH.match(u.name)]
    for sub in (".fingerprint", "deps", "examples"):
        for p in listdir(profile / sub):
            m = LEGACY.match(p.name)
            if m:
                legacy.setdefault(m["hash"], []).append(p)
    found += legacy.values()
    found += [[d] for d in listdir(profile / "incremental")]
    return found


def stale(profile, cut):
    out = []
    for paths in units(profile):
        seen = [t for t in map(last_use, paths) if t is not None]
        if seen and max(seen) < cut:
            out.append((max(seen), paths))
    return out


def size(paths):
    """Bytes on disk of the distinct files under `paths`."""
    inodes = {}
    for p in paths:
        walk = os.walk(p) if p.is_dir() and not p.is_symlink() else [(p.parent, [], [p.name])]
        for root, _, files in walk:
            for f in files:
                try:
                    st = os.lstat(os.path.join(root, f))
                except FileNotFoundError:
                    continue
                inodes[(st.st_dev, st.st_ino)] = st.st_blocks * 512
    return sum(inodes.values())


def delete(path):
    """Delete `path`; the error if that failed."""
    try:
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()
    except FileNotFoundError:
        pass
    except OSError as e:
        return e
    return None


def human(n):
    for unit in ("B", "K", "M", "G", "T"):
        if n < 1024 or unit == "T":
            return f"{n:.1f}{unit}" if unit != "B" else f"{n}B"
        n /= 1024


def profiles(target):
    """Directories holding `.cargo-lock`, at most three levels down."""
    out = []
    for root, dirs, files in os.walk(target):
        depth = Path(root).relative_to(target).parts
        if ".cargo-lock" in files:
            out.append(Path(root))
            dirs[:] = []  # a profile's own subdirectories are its units
        elif len(depth) >= 3:
            dirs[:] = []
    return sorted(out)


def default_target():
    root = Path(__file__).resolve().parent.parent
    try:
        meta = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=root, check=True, capture_output=True, text=True,
        )
    except (OSError, subprocess.CalledProcessError) as e:
        why = getattr(e, "stderr", None) or e
        sys.exit(f"cargo metadata failed ({str(why).strip()}); pass TARGET_DIR")
    return Path(json.loads(meta.stdout)["target_directory"])


def noatime(path, mountinfo="/proc/self/mountinfo"):
    """Whether the filesystem holding `path` is mounted `noatime`."""
    try:
        lines = Path(mountinfo).read_text().splitlines()
    except OSError:
        return False  # not Linux: cannot tell
    path = str(path.resolve())
    best, opts = "", ""
    for line in lines:
        fields = line.split()
        mnt = re.sub(r"\\([0-7]{3})", lambda m: chr(int(m[1], 8)), fields[4])
        inside = path == mnt or path.startswith(mnt.rstrip("/") + "/")
        if inside and len(mnt) >= len(best):
            best, opts = mnt, fields[5]
    return "noatime" in opts.split(",")


def days(text):
    n = float(text)
    if not n > 0:
        raise argparse.ArgumentTypeError(f"must be positive: {text}")
    return n


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("-n", dest="dry", action="store_true", help="list, delete nothing")
    ap.add_argument("--days", type=days, default=5, help="age limit (default 5)")
    ap.add_argument("target", nargs="?", type=Path, help="target directory (default: cargo's)")
    a = ap.parse_args()
    target = a.target or default_target()
    if not target.is_dir():
        sys.exit(f"no target directory: {target}")
    if noatime(target):
        sys.exit(f"{target} is on a noatime mount: units in use would look stale")
    cut = time.time() - a.days * 86400
    count = freed = failed = 0
    for profile in profiles(target):
        if a.dry:
            for seen, paths in stale(profile, cut):
                day = time.strftime("%F", time.localtime(seen))
                for p in paths:
                    print(f"{day} {p}")
                count += 1
                freed += size(paths)
            continue
        with open(profile / ".cargo-lock", "rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            for _, paths in stale(profile, cut):
                bytes_ = size(paths)
                errors = [e for e in map(delete, paths) if e]
                for e in errors:
                    print(f"cannot delete: {e}", file=sys.stderr)
                if errors:
                    failed += 1
                else:
                    count += 1
                    freed += bytes_
            for d in (profile / "build").glob("*/"):
                if not LEGACY.match(d.name):
                    try:
                        d.rmdir()  # a package directory left empty
                    except OSError:
                        pass
    verb = "would delete" if a.dry else "deleted"
    print(f"{verb} {count} units, {human(freed)}, in {target} ({human(size([target]))} total)")
    if failed:
        sys.exit(f"{failed} units could not be deleted")


if __name__ == "__main__":
    main()
