#!/usr/bin/env python3
"""Positive and negative controls for scripts/check-sexp-safety.py (#736):
a check that cannot fail looks exactly like a clean tree."""

import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("check", os.path.join(HERE, "check-sexp-safety.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

CASES = [
    # (source, the names the check reports)
    ("pub fn f(x: Sexp) {}", ["f"]),
    ("pub fn g<'a, T: Into<u8>>(cx: &Cx, v: &[Sexp]) -> u8 { 0 }", ["g"]),
    ("pub fn h(cx: &Cx,\n    x: Sexp,\n) {}", ["h"]),
    ('pub extern "C" fn R_f(x: Sexp) -> c_int { 0 }', ["R_f"]),
    ("        pub extern \"C\" fn $name(class: Sexp, fun: *const c_void) {}", ["$name"]),
    ("pub const fn k(x: Sexp) -> Sexp { x }", ["k"]),
    # not reported
    ("pub unsafe fn f(x: Sexp) {}", []),
    ('pub unsafe extern "C" fn R_f(x: Sexp) {}', []),
    ("pub(crate) fn f(x: Sexp) {}", []),
    ("fn f(x: Sexp) {}", []),
    ("pub fn f(x: Ref<'_>) -> Sexp { x.sexp() }", []),
    ("pub fn each(&self, f: impl FnMut(Sexp) -> bool) {}", []),
    ("pub fn roots(&self, f: &mut dyn FnMut(&mut Sexp)) {}", []),
    ("pub fn f(&self, // a Sexp in a comment\n    n: usize) {}", []),
    ("pub fn f(&self, a: impl Fn(u8) -> Sexp, b: Sexp) {}", ["f"]),
    ("impl Cx { pub fn f(x: Sexp) {} }", ["f"]),
    ("pub\nfn f(x: Sexp) {}", ["f"]),
    ("pub fn f<T: Into<Sexp>>(x: T) {}", ["f"]),
    ("pub fn f<T>(x: T)\nwhere\n    T: Into<Sexp>,\n{}", ["f"]),
    ("pub fn f(x: u8) -> Sexp\nwhere\n    u8: Copy,\n{ todo!() }", []),
    ("/// a safe `pub fn f(x: Sexp)` is refused\npub fn g() {}", []),
    ("// pub fn f(x: Sexp) {}\npub fn g() {}", []),
    ("pub(super) fn f(x: Sexp) {}", []),
    ("pub fn each<F: FnMut(Sexp)>(f: F) {}", []),
    ("pub fn each<F>(f: F)\nwhere\n    F: Fn(&mut Sexp) -> bool,\n{}", []),
    ("pub fn each<F: FnMut(Sexp)>(f: F, x: Sexp) {}", ["each"]),
]


def main():
    bad = 0
    for src, want in CASES:
        got = [h.split("safe pub fn ")[1].split(" ")[0] for h in check.hits("t.rs", src)]
        if got != want:
            print(f"FAIL: {src!r}: reported {got}, want {want}")
            bad += 1
    print(f"check-sexp-safety: {len(CASES) - bad}/{len(CASES)} cases")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
