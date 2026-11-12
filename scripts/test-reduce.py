#!/usr/bin/env python3
"""Tests of scripts/rast.py, scripts/rcheck.py and scripts/reduce.py.

    python3 scripts/test-reduce.py

They need GNU R (Rscript; the oracle test: 4.5.0, else it is skipped) but no
robigo: a stand-in "robigo" runs GNU R and corrupts its output in one tier,
or crashes on one word, so each failure the reducer preserves is known.
"""
import atexit
import pathlib
import random
import shutil
import stat
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import fuzz_gen  # noqa: E402
import rast  # noqa: E402
import rcheck  # noqa: E402

RSCRIPT = str(rcheck.find_oracle())
TMP = pathlib.Path(tempfile.mkdtemp(prefix="robigo-test-reduce-"))
atexit.register(shutil.rmtree, TMP, ignore_errors=True)
NOISE = """a <- 1:10
b <- sum(a) # 55
print(b)
f <- function(x, y = 2) {
    z <- x * y
    w <- z - 1
    z
}
print(f(3))
"""


def fake(name, body):
    """A robigo stand-in: bash BODY with $f the script, $out GNU R's stdout."""
    p = TMP / name
    p.write_text(f"""#!/bin/bash
f="${{@: -1}}"
out=$("{RSCRIPT}" --vanilla "$f" 2>/tmp/null-{name}-$$); st=$?
err=$(cat /tmp/null-{name}-$$); rm -f /tmp/null-{name}-$$
{body}
printf '%s\\n' "$out"; [ -n "$err" ] && printf '%s\\n' "$err" >&2
exit $st
""")
    p.chmod(p.stat().st_mode | stat.S_IEXEC)
    return p


def reduce(args, expect_ok=True):
    r = subprocess.run([sys.executable, str(ROOT / "scripts/reduce.py"), *args, "--seconds", "300", "--jobs", "4"],
                       capture_output=True, text=True)
    if expect_ok:
        assert r.returncode == 0, r.stderr
    return r


def test_parse_seeds():
    p = rast.Parser(RSCRIPT)
    trees = []
    for f in sorted((ROOT / "tests/r").rglob("*.R")):
        t = p.parse(f.read_text(encoding="utf-8", errors="surrogateescape"), strict=True)
        if t:
            trees.append(t)
    assert len(trees) > 100, len(trees)
    pool = [x for t in trees for x in rast.fragments(t)]
    rnd = random.Random(7)
    n, parsed, round_trip = 300, 0, 0
    for _ in range(n):
        t = rnd.choice(trees)
        parsed += p.parse(rast.mutate(rnd, t, pool)) is not None
        d = p.parse(rast.insert_dead(rnd, t))
        round_trip += d is not None and rast.strip_dead(d) == t.src
    gen = sum(p.parse(fuzz_gen.program(rnd)) is not None for _ in range(100))
    p.close()
    print(f"  AST mutants parse {parsed}/{n}, EMI round trips {round_trip}/{n}, generated parse {gen}/100")
    assert parsed >= 0.95 * n and round_trip == n and gen == 100


def test_groups_and_hoists():
    p = rast.Parser(RSCRIPT)
    t = p.parse("g(1, b = 2, 3); h <- function(x, y) { x; y }\n")
    gs = rast.groups(t)
    texts = {g.render(set()) for g in gs} | {gs[0].render({1})}
    assert " h <- function(x, y) { x; y }\n" in texts, texts         # a statement and its `;`
    assert " \n" in texts, texts                                       # every top-level statement
    assert "g(); h <- function(x, y) { x; y }\n" in texts, texts      # call arguments
    assert "g(1, b = 2, 3); h <- function() { x; y }\n" in texts, texts  # formals
    assert "g(1, b = 2, 3); h <- function(x, y) {   }\n" in texts, texts  # a block's statements
    assert any(h.startswith("1;") for h in rast.hoists(t))
    assert all(p.parse(c) is not None for c in texts)
    p.close()


def test_signature_and_blame():
    exp = (b"[1] 1\n[1] 2\n", b"", 0)
    assert rcheck.signature(exp, (b"[1] 1\n[1] 3\n", b"", 0)) == ("out: '[1] 2' -> '[1] 3'",)
    assert rcheck.signature(exp, (b"[1] 1\n", b"", 0)) == ("out: '[1] 2' -> ''",)
    assert rcheck.signature(exp, (exp[0], b"Error\n", 1)) == ("err: '' -> 'Error'", "status: 0 -> 1")
    assert rcheck.signature(exp, (b"", b"E\n", 1), loose=True) == ("out", "err", "status")
    x, y = (b"a", b"", 0), (b"b", b"", 0)
    assert rcheck.blame({"jit": x, "vm": x, "interp": x}) is None
    assert rcheck.blame({"jit": y, "vm": x, "interp": x}) == ("jit", "interp")
    assert rcheck.blame({"jit": x, "vm": y, "interp": x}) == ("vm", "interp")
    assert rcheck.blame({"jit": x, "vm": x, "interp": y}) == ("interp", "vm")
    z = (b"c", b"", 0)
    assert rcheck.blame({"jit": x, "vm": y, "interp": z}) == ("all", "interp")


def test_tiers_all_differ_and_tier_crash():
    # every tier prints something else; a VM-only crash reduces in tiers mode
    robigo = fake("each", 'out="$out $ROBIGO_COMPILE$ROBIGO_JIT"')
    src = TMP / "t.R"
    src.write_text("print(1)\n")
    v = rcheck.Checker(robigo).tiers(src)
    assert v.kind == "diff" and v.key[:2] == ("tier", "all"), v
    robigo = fake("vmboom", 'if [ "$ROBIGO_JIT" = 0 ] && [ "$ROBIGO_COMPILE" = 1 ] && grep -q boom "$f"; then '
                  'echo "thread \'main\' panicked at src/vm.rs:1:1:" >&2; exit 101; fi')
    src = TMP / "tier-crash-0123456789.R"
    src.write_text(NOISE + "boom <- 1\n")
    small = reduce([str(src), "--robigo", str(robigo)]).stdout
    print(f"  vm-only crash: -> {small.strip()!r}")
    assert "boom" in small and len(small) < 20, small


def test_reduce_tiers():
    # the JIT tier alone prints 43 where R prints 42
    robigo = fake("jit42", 'if [ "$ROBIGO_COMPILE" != 0 ] && [ "$ROBIGO_JIT" != 0 ]; then out=${out//42/43}; fi')
    src = TMP / "tier-case.R"
    src.write_text(NOISE + "k <- function(v) v + 40\nprint(k(2))\n" + NOISE)
    r = reduce([str(src), "--robigo", str(robigo)])
    small = r.stdout
    print(f"  tiers: {len(src.read_text())} -> {len(small)} bytes: {small.strip()!r}")
    assert small == "k <- function(v) v + 40\nk(2)\n", small
    assert (TMP / "tier-case.min.R").read_text() == small


def test_reduce_crash():
    robigo = fake("boom", 'if grep -q boom "$f"; then echo "thread \'main\' panicked at src/fake.rs:1:1:" >&2; exit 101; fi')
    src = TMP / "crash-case.R"
    src.write_text(NOISE + "x <- c(boom = 1)\n" + NOISE)
    small = reduce([str(src), "--robigo", str(robigo)]).stdout
    print(f"  crash: -> {small.strip()!r}")
    assert "boom" in small and len(small) < 20, small


def test_reduce_emi():
    # with dead code, the default tier drops a line of output
    robigo = fake("emi", 'if [ "$ROBIGO_COMPILE" != 0 ] && grep -q "emi" "$f"; then out=${out/\\[1\\] 55/}; fi')
    p = rast.Parser(RSCRIPT)
    variant = rast.insert_dead(random.Random(3), p.parse(NOISE), 2)
    p.close()
    src = TMP / "emi-case.R"
    src.write_text(variant)
    small = reduce([str(src), "--robigo", str(robigo)]).stdout
    print(f"  emi: {len(variant)} -> {len(small)} bytes: {small.strip()!r}")
    assert "emi" in small and "sum(a)" in small and len(small) < 60, small


def test_reduce_oracle():
    if rcheck.oracle_info(RSCRIPT)[0] != "4.5.0":
        print("  oracle: skipped (no GNU R 4.5.0)")
        return
    robigo = fake("always42", "out=${out//55/56}")
    src = TMP / "case.R"
    src.write_text(NOISE)
    small = reduce([str(src), "--robigo", str(robigo), "--oracle", RSCRIPT]).stdout
    print(f"  oracle: -> {small.strip()!r}")
    assert small == "a <- 1:10\nsum(a)\n", small
    # a script that does not fail is refused
    ok = TMP / "ok.R"
    ok.write_text("print(1)\n")
    r = reduce([str(ok), "--robigo", str(robigo), "--oracle", RSCRIPT], expect_ok=False)
    assert r.returncode != 0 and "does not fail" in r.stderr, r.stderr


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        print(t.__name__)
        t()
    print(f"{len(tests)} tests passed")


if __name__ == "__main__":
    main()
