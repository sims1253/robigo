#!/usr/bin/env python3
"""What `scripts/base-subset.sh --check` says when a committed file is not
what the tree generates: which names it gains and loses, which inputs reach
each new one, and what to run.

    scripts/base-subset-stale.py COMMITTED GENERATED [REACHED_BY]

Exits 0 when the two files have the same lines, 1 (with the report on
stderr) when they do not. Comment lines (`#`, the header) are
compared too, but reported only when nothing else differs.
REACHED_BY is base-subset.R's `reached-by.txt` (KIND NAME [FILE] INPUTS).
"""
import sys

MAX = 40   # names listed per direction; the rest are counted


def lines(path):
    with open(path, encoding="utf-8") as f:
        return f.read().splitlines()


def reached(path):
    """`KIND NAME` -> the ids of the inputs that reach it."""
    out = {}
    if not path:
        return out
    try:
        for line in lines(path):
            if line.startswith("#") or not line.strip():
                continue
            f = line.split()
            out[" ".join(f[:2])] = f[-1]
    except OSError:
        pass
    return out


def report(committed, generated, by_path=None):
    """The report (a string), or None when the files agree."""
    old, new = lines(committed), lines(generated)
    if old == new:
        return None
    body_old = [x for x in old if not x.startswith("#")]
    body_new = [x for x in new if not x.startswith("#")]
    so, sn = set(body_old), set(body_new)
    added = [x for x in body_new if x not in so]
    removed = [x for x in body_old if x not in sn]
    by = reached(by_path)
    out = [f"error: {committed} is not what scripts/base-subset.sh generates."]
    if added or removed:
        out.append(f"The inputs now reach {len(added)} name(s) it does not list, and no longer reach "
                   f"{len(removed)} it lists:")
        for x in added[:MAX]:
            who = by.get(" ".join(x.split()[:2]))
            out.append(f"  + {x}" + (f"    (reached by {who})" if who else ""))
        if len(added) > MAX:
            out.append(f"  + ... {len(added) - MAX} more")
        for x in removed[:MAX]:
            out.append(f"  - {x}")
        if len(removed) > MAX:
            out.append(f"  - ... {len(removed) - MAX} more")
    elif body_old != body_new:
        out.append("The same names, in another order.")
    else:
        out.append("Only its header differs (the generator's comments).")
    out += [
        "The generator ran; its output differs from the committed file. Usually that is a case added",
        "or changed among its inputs (the M1 scripts, the d2s/e7 rows, tests/rt/capi's cases, the",
        "compiler's cmp.R and the tests/rt areas base-subset.R names; `Rscript scripts/base-subset.R",
        "--inputs` lists them), which now reaches other base names. Check that the names above are",
        "ones your change should reach (a change you did not expect can be a generator bug), then",
        "regenerate and commit the three files:",
        "  scripts/base-subset.sh      # GNU R 4.6.2: the oracle image via docker, about 7 minutes",
        "  git add crates/robigo-rt/proto/base-subset.txt crates/robigo-rt/proto/base-bound.txt \\",
        "          crates/robigo-rt/proto/base-wide.txt",
    ]
    return "\n".join(out)


def main(argv):
    if len(argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    r = report(*argv)
    if r is None:
        return 0
    print(r, file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
