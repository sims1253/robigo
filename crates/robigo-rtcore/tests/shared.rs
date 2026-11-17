//! H5: R_NilValue, NA_STRING, R_BlankString, R_UnboundValue and
//! R_MissingArg are the process's, one of each for every isolate
//! (robigo-heap's `shared.rs`). Run under Miri as well: the isolates are
//! made at once on two threads, and none collects.

use robigo_rtcore::{Encoding, OwnedIsolate};
use std::sync::{Arc, Barrier};

/// The constants' addresses as an isolate sees them, and its `mkChar("")`.
fn constants(iso: &mut OwnedIsolate) -> [usize; 6] {
    let cx = iso.cx();
    let blank = cx.mk_char(b"", Encoding::Native).unwrap();
    [
        cx.nil().sexp().addr(),
        robigo_rtcore::Ref::from(cx.na_string()).sexp().addr(),
        robigo_rtcore::Ref::from(cx.blank_string()).sexp().addr(),
        cx.unbound().sexp().addr(),
        cx.missing_arg().sexp().addr(),
        robigo_rtcore::Ref::from(blank).sexp().addr(),
    ]
}

/// Two isolates made at the same time on two threads share the constants,
/// and each one's cache gives the shared "".
#[test]
fn isolates_on_two_threads_share_the_constants() {
    let start = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                let mut iso = OwnedIsolate::new();
                let seen = constants(&mut iso);
                // and a second look after this isolate has allocated, set
                // NULL's bits to what they are, and installed "" (none of
                // which writes a constant: Miri sees any race between the
                // two threads)
                let cx = iso.cx();
                for i in 0..64 {
                    let _ = cx.mk_char(format!("s{i}").as_bytes(), Encoding::Native).unwrap();
                }
                cx.scope(|cx, s| {
                    let nil = s.root(cx, cx.nil());
                    cx.set_object_bit(nil, false);
                    cx.set_s4_bit(nil, false);
                    cx.mark_assignment_call(nil);
                });
                let _ = cx.install(b"").unwrap();
                assert_eq!(constants(&mut iso), seen);
                seen
            })
        })
        .collect();
    let seen: Vec<[usize; 6]> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(seen[0], seen[1], "the same constants in both isolates");
    assert_eq!(seen[0][2], seen[0][5], "mkChar(\"\") is R_BlankString");
    let mut here = OwnedIsolate::new();
    assert_eq!(constants(&mut here), seen[0], "and in an isolate made after them");
}

/// Changing a bit of a shared constant is refused: it would change in every
/// isolate at once.
#[test]
#[should_panic(expected = "a write into a shared immutable object")]
fn setting_nulls_object_bit_is_refused() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        cx.set_object_bit(nil, true);
    });
}

/// Two isolates on two threads intern the same names at once (H5, the
/// foreman's §4 conditions): each name is one symbol with one id in both,
/// bound to a different value in each. In each, the symbol's PRINTNAME is
/// the CHARSXP its own `mkChar` gives for the name, whether `mkChar` came
/// first or the symbol did, so `match(x, as.character(quote(foo)))` is TRUE
/// in both. Under Miri as well.
#[test]
fn two_isolates_intern_at_once_and_keep_their_own_printnames() {
    const NAMES: usize = if cfg!(miri) { 8 } else { 200 };
    let start = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|t| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                let mut iso = OwnedIsolate::new();
                let cx = iso.cx();
                start.wait();
                let mut seen = Vec::new();
                for i in 0..NAMES {
                    let name = format!("contended_{i}");
                    // even names: this isolate makes the string first in one
                    // isolate and the symbol first in the other
                    let string_first = (i + t) % 2 == 0;
                    let (c, s) = if string_first {
                        let c = robigo_rtcore::Ref::from(cx.mk_char(name.as_bytes(), Encoding::Native).unwrap());
                        (c, cx.install(name.as_bytes()).unwrap())
                    } else {
                        let s = cx.install(name.as_bytes()).unwrap();
                        (robigo_rtcore::Ref::from(cx.mk_char(name.as_bytes(), Encoding::Native).unwrap()), s)
                    };
                    let pn = robigo_rtcore::Ref::from(s.name(cx));
                    assert_eq!(pn.sexp(), c.sexp(), "{name}: the PRINTNAME is this isolate's CHARSXP");
                    assert!(unsafe { s.as_ref().sexp().heap_id() }.is_shared(), "{name}: the process's symbol");
                    seen.push((s.as_ref().sexp().addr(), unsafe { s.as_ref().sexp().sym_id() }, pn.sexp().addr()));
                }
                // the same symbol, a value per isolate
                cx.scope(|cx, sc| {
                    let sym = sc.root(cx, cx.install(b"contended_0").unwrap().as_ref());
                    let v = sc.root(cx, cx.alloc_with(1, |_| t as i32).unwrap().finish());
                    let g = sc.root(cx, cx.global_env());
                    cx.define_var(sym, v, g).unwrap();
                    start.wait();
                    let out = sc.out(cx);
                    let got = out.root();
                    cx.find_var(sym, g, out).unwrap();
                    assert_eq!(got.bind(cx).as_vector::<i32>().unwrap().get(0), t as i32, "this isolate's value");
                });
                seen
            })
        })
        .collect();
    let seen: Vec<Vec<(usize, u32, usize)>> = threads.into_iter().map(|h| h.join().unwrap()).collect();
    for (i, (a, b)) in seen[0].iter().zip(&seen[1]).enumerate() {
        assert_eq!((a.0, a.1), (b.0, b.1), "contended_{i}: one symbol, one id");
        assert_ne!(a.2, b.2, "contended_{i}: a PRINTNAME per isolate");
    }
}
