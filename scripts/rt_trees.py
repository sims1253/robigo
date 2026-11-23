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


def binary_hash(binary):
    """rt-home-db.sh's hash of a binary: sha256, its first 16 digits."""
    h = hashlib.sha256()
    with open(binary, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def script_hash(root, name):
    """The hash rt-home.sh / rt-home-db.sh put in a stamp for themselves."""
    return binary_hash(os.path.join(root, "scripts", name))


def fresh(binary, root=ROOT):
    """Whether the checkout's trees are `binary`'s and whole, as far as can
    be told without a build: the database stamp's script hashes and binary
    hash match (`<src> <rt-home.sh> <natives> <rt-home-db.sh> <binary>`), the
    R_HOME tree's stamp is the one it was made from, and the files a start
    needs exist (base's loader, the two databases and their indexes, the two translation caches). (The
    natives' hash needs robigo-heap's build: rt-home.sh checks that one.)"""
    db = os.path.join(root, ".robigo", "rt-home-db")
    try:
        with open(os.path.join(db, ".rt-home-db")) as f:
            words = f.read().split()
        with open(os.path.join(root, ".robigo", "rt-home", ".rt-home")) as f:
            home = f.read().split()
    except OSError:
        return False
    if len(words) != 5 or words[:3] != home:
        return False
    if (words[1], words[3], words[4]) != (script_hash(root, "rt-home.sh"), script_hash(root, "rt-home-db.sh"),
                                          binary_hash(binary)):
        return False
    need = [os.path.join(db, "library", "base", "R", f) for f in ("base", "base.rdb", "base.rdx")] + \
        [os.path.join(db, "library", "compiler", "R", f) for f in ("compiler.rdb", "compiler.rdx")] + [
            # (rt-home-db.sh's own freshness check requires the same files,
            # under its lock: a miss here only sends the scripts to decide)
            os.path.join(root, ".robigo", "rt-home-db-cache", "base.rtc"),
            os.path.join(root, ".robigo", "rt-home-db-cache", "compiler.rtc")]
    return all(os.path.isfile(p) and os.path.getsize(p) > 0 for p in need)


def ensure(binary, root=ROOT):
    """Make the checkout's R_HOME tree and base's database tree for
    `binary` unless they are already its and whole (exits on a failure),
    and point this process's robigo-rt children at that checkout
    (ROBIGO_RT_CHECKOUT): a binary built in another worktree, or an inherited
    ROBIGO_RT_CHECKOUT, would boot from another tree than the one made here."""
    root = os.path.abspath(root)
    os.environ["ROBIGO_RT_CHECKOUT"] = root
    if fresh(binary, root):
        return
    for cmd in (["scripts/rt-home.sh"], ["scripts/rt-home-db.sh", "--robigo-rt", os.path.abspath(binary)]):
        p = subprocess.run(cmd, cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if p.returncode:
            sys.exit(f"{' '.join(cmd)} failed: base's database tree for {binary} could not be made\n"
                     + p.stderr[-2000:])
