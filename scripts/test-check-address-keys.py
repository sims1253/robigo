#!/usr/bin/env python3
"""Positive and negative controls for scripts/check-address-keys.py:
a check that cannot fail looks exactly like a clean tree."""

import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("check", os.path.join(HERE, "check-address-keys.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

CASES = [
    # (description, source, expected hit count)
    ("raw Sexp key in HashMap", "let m: HashMap<Sexp, i32> = HashMap::new();", 1),
    ("raw Sexp key in HashSet", "let s: HashSet<Sexp> = HashSet::new();", 1),
    ("raw Sexp tuple key", "let m: BTreeMap<(Sexp, u8), i32> = BTreeMap::new();", 1),
    ("raw Sexp key in PosMap", "struct S { p: PosMap<Sexp, u16> }", 1),
    ("raw Sexp key justified", "// address-key: stop-the-world walk\nlet s: HashSet<Sexp> = HashSet::new();", 0),
    ("raw Sexp VALUE is allowed", "struct S { assign_fns: HashMap<SymId, Sexp, BuildFx> }", 0),
    ("Ref key is allowed", "let mut m: HashMap<Ref<'_>, i32> = HashMap::new();", 0),
    ("HashSet::from([e.addr()])", "fn f(e: Sexp) { let mut seen = HashSet::from([e.addr()]); }", 1),
    (
        "HashSet::from justified",
        "fn f(e: Sexp) {\n    // address-key: non-moving walk\n    let mut seen = HashSet::from([e.addr()]);\n}",
        0,
    ),
    (
        "inferred HashSet inserted with .addr()",
        "fn f(c: Sexp) {\n    let mut seen = HashSet::new();\n    seen.insert(c.addr());\n}",
        1,
    ),
    (
        "inferred HashMap inserted with as_ptr() as usize",
        "fn f(x: Sexp) {\n    let mut m = HashMap::new();\n    m.insert(x.as_ptr() as usize, 1);\n}",
        1,
    ),
    (
        "local address alias inserted into map",
        "fn f(x: Sexp) {\n    let mut m: HashMap<usize, i32> = HashMap::new();\n    let k = x.addr();\n    m.insert(k, 1);\n}",
        1,
    ),
    (
        "struct field usize map populated with .addr()",
        "struct W { refs: HashMap<usize, i32> }\nimpl W { fn add(&mut self, x: Sexp) { self.refs.insert(x.addr(), 1); } }",
        1,
    ),
    (
        "struct field justified",
        "struct W {\n    // address-key: cleared on restart\n    refs: HashMap<usize, i32>,\n}\nimpl W { fn add(&mut self, x: Sexp) { self.refs.insert(x.addr(), 1); } }",
        0,
    ),
    (
        "fn param usize map queried with .addr()",
        "fn f(s: Sexp, reps: &mut HashMap<usize, i32>) { reps.get_mut(&s.addr()); }",
        1,
    ),
    (
        "fn param justified",
        "fn f(\n    s: Sexp,\n    // address-key: rebuilt on restart\n    reps: &mut HashMap<usize, i32>,\n) { reps.get_mut(&s.addr()); }",
        0,
    ),
    (
        "non-address usize map (bytecode PCs) is allowed",
        "fn f(pc: usize) {\n    let mut landing: HashMap<usize, Vec<u8>> = HashMap::new();\n    landing.insert(pc, Vec::new());\n}",
        0,
    ),
    (
        "empty justification rejected",
        "fn f(e: Sexp) {\n    // address-key:\n    let mut seen = HashSet::from([e.addr()]);\n}",
        1,
    ),
    (
        "commented-out HashMap<Sexp, i32> ignored",
        "// let m: HashMap<Sexp, i32> = HashMap::new();\nfn f() {}",
        0,
    ),
    (
        "string literal HashMap<Sexp, i32> ignored",
        'const S: &str = "HashMap<Sexp, i32>";',
        0,
    ),
    (
        "collect into HashSet<usize> from .addr()",
        "fn f(xs: &[Sexp]) {\n    let s: HashSet<usize> = xs.iter().map(|x| x.addr()).collect();\n}",
        1,
    ),
    (
        "PosMap<usize, u16> with as_ptr() as usize",
        "struct C { pool: PosMap<usize, u16> }\nimpl C { fn k(&mut self, x: Sexp) { self.pool.get(&(x.as_ptr() as usize)); } }",
        1,
    ),
]


def main():
    bad = 0
    for desc, src, want in CASES:
        got = list(check.hits("t.rs", src))
        if len(got) != want:
            print(f"FAIL ({desc}): got {len(got)} hits {got}, want {want}")
            bad += 1
    print(f"check-address-keys: {len(CASES) - bad}/{len(CASES)} cases")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
