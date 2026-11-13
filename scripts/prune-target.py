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
    # An empty unit counts from its own last change.
    return newest or path.lstat().st_mtime


def units(profile):
    """The units of one profile: lists of paths deleted together."""
    found = []
    legacy = {}  # hash -> paths
    build = profile / "build"
    for d in sorted(build.iterdir()) if build.is_dir() else []:
        m = LEGACY.match(d.name)
        if m:
            legacy.setdefault(m["hash"], []).append(d)
        elif d.is_dir() and not d.is_symlink():
            found += [[u] for u in sorted(d.iterdir()) if HASH.match(u.name)]
    for sub in (".fingerprint", "deps", "examples"):
        dirpath = profile / sub
        for p in sorted(dirpath.iterdir()) if dirpath.is_dir() else []:
            m = LEGACY.match(p.name)
            if m:
                legacy.setdefault(m["hash"], []).append(p)
    found += legacy.values()
    inc = profile / "incremental"
    found += [[d] for d in sorted(inc.iterdir())] if inc.is_dir() else []
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
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path, ignore_errors=True)
    else:
        try:
            path.unlink()
        except FileNotFoundError:
            pass


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
    meta = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=root, check=True, capture_output=True, text=True,
    )
    return Path(json.loads(meta.stdout)["target_directory"])


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("-n", dest="dry", action="store_true", help="list, delete nothing")
    ap.add_argument("--days", type=float, default=5, help="age limit (default 5)")
    ap.add_argument("target", nargs="?", type=Path, help="target directory (default: cargo's)")
    a = ap.parse_args()
    target = a.target or default_target()
    if not target.is_dir():
        sys.exit(f"no target directory: {target}")
    cut = time.time() - a.days * 86400
    count = freed = 0
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
                freed += size(paths)
                for p in paths:
                    delete(p)
                count += 1
            for d in (profile / "build").glob("*/"):
                if not LEGACY.match(d.name):
                    try:
                        d.rmdir()  # a package directory left empty
                    except OSError:
                        pass
    verb = "would delete" if a.dry else "deleted"
    print(f"{verb} {count} units, {human(freed)}, in {target} ({human(size([target]))} total)")


if __name__ == "__main__":
    main()
