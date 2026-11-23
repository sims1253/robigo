"""base's database tree for a robigo-rt binary, for the tools that run one
outside rt-ci.py's plans (scripts/corpus.py --runtime rt, scripts/wall13.py).

A robigo-rt that names no boot boots rdb once it is the default, from the
checkout's .robigo/rt-home-db, which scripts/rt-home-db.sh writes with one
binary (its stamp ends in that binary's sha256). `ensure(binary)` makes the
tree when it is missing or another binary's, and does nothing when it is
this one's (a fresh stamp and its translation cache): the two scripts'
own freshness rules decide, and a fresh tree costs a stamp read.
"""

import hashlib
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def stamp_binary(root=ROOT):
    """The binary's hash the checkout's tree was written by (the stamp's
    last word), or None without a tree."""
    try:
        with open(os.path.join(root, ".robigo", "rt-home-db", ".rt-home-db")) as f:
            words = f.read().split()
    except OSError:
        return None
    return words[-1] if words else None


def binary_hash(binary):
    """rt-home-db.sh's hash of a binary: sha256, its first 16 digits."""
    h = hashlib.sha256()
    with open(binary, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def ensure(binary, root=ROOT):
    """Make the checkout's R_HOME tree and base's database tree for
    `binary` unless they are already its (exits on a failure)."""
    cache = os.path.join(root, ".robigo", "rt-home-db-cache")
    fresh = stamp_binary(root) == binary_hash(binary) and all(
        os.path.getsize(os.path.join(cache, f)) > 0 if os.path.exists(os.path.join(cache, f)) else False
        for f in ("base.rtc", "compiler.rtc"))
    if fresh:
        return
    for cmd in (["scripts/rt-home.sh"], ["scripts/rt-home-db.sh", "--robigo-rt", os.path.abspath(binary)]):
        p = subprocess.run(cmd, cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if p.returncode:
            sys.exit(f"{' '.join(cmd)} failed: base's database tree for {binary} could not be made\n"
                     + p.stderr[-2000:])
