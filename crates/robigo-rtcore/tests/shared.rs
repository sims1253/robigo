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
                    // SAFETY: a live symbol (immortal), read in its header and 4th word.
                    let (shared, id) = unsafe { (s.as_ref().sexp().heap_id().is_shared(), s.as_ref().sexp().sym_id()) };
                    assert!(shared, "{name}: the process's symbol");
                    seen.push((s.as_ref().sexp().addr(), id, pn.sexp().addr()));
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

/// Adoption under contention (H5): isolate A installs fresh names while
/// isolate B, on another thread, meets each symbol for the first time,
/// adopts it and reads its PRINTNAME. A hands each symbol over through a
/// rendezvous channel, then installs names of its own (never handed over)
/// until B says it is done with that symbol; B starts only once A has
/// completed an install after the hand-off. So A's install loop spans B's
/// work by construction (the arena's lock contended whenever both threads
/// run). Before adoption B refuses the name; after it,
/// B's PRINTNAME is what B's `mkChar` gives for the name (the address
/// `match` hashes strings by, unique.c), made before the adoption for even
/// names and after it for odd ones, and B's `install` gives the same
/// symbol. A collection in B between the first and the second read leaves
/// it in place: the PRINTNAME is immortal in B, and the second read gives
/// the same CHARSXP. Under Miri as well, without the collections (the
/// collector calls C) and the refusal (Miri does not unwind through
/// `catch_unwind` cheaply).
#[test]
fn a_symbol_another_isolate_made_is_adopted_under_contention() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    const NAMES: usize = if cfg!(miri) { 6 } else { 300 };
    let installs = Arc::new(AtomicUsize::new(0));
    let b_done = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = std::sync::mpsc::sync_channel::<(String, usize)>(0);
    let a = {
        let (installs, b_done) = (Arc::clone(&installs), Arc::clone(&b_done));
        std::thread::spawn(move || {
            let mut iso = OwnedIsolate::new();
            let cx = iso.cx();
            let mut k = 0;
            for i in 0..NAMES {
                let name = format!("adopted_{i}");
                let s = cx.install(name.as_bytes()).unwrap();
                tx.send((name, s.as_ref().sexp().addr())).unwrap();
                // names of A's own, which B never sees, until B is done
                while b_done.load(Ordering::SeqCst) <= i {
                    let _ = cx.install(format!("adopted_a_only_{k}").as_bytes()).unwrap();
                    installs.fetch_add(1, Ordering::SeqCst);
                    k += 1;
                }
            }
        })
    };
    let b = {
        let (installs, b_done) = (Arc::clone(&installs), Arc::clone(&b_done));
        std::thread::spawn(move || {
            let mut iso = OwnedIsolate::new();
            let mut n = 0;
            for (name, addr) in rx {
                // A is in its install loop before B starts
                let start = installs.load(Ordering::SeqCst);
                while installs.load(Ordering::SeqCst) == start {
                    std::thread::yield_now();
                }
                let cx = iso.cx();
                // B interns names of its own meanwhile
                let _ = cx.install(format!("adopted_b_only_{n}").as_bytes()).unwrap();
                let early = (n % 2 == 0)
                    .then(|| robigo_rtcore::Ref::from(cx.mk_char(name.as_bytes(), Encoding::Native).unwrap()).sexp());
                // SAFETY: a symbol of the process's arena: immortal, never
                // moved, and every isolate may hold it (what a hand-over
                // between isolates gives).
                let s = unsafe { robigo_rtcore::Ref::from_sexp(robigo_rtcore::Sexp::from_ptr(addr as *mut u8), cx) }
                    .as_sym()
                    .unwrap();
                if !cfg!(miri) && n % 50 == 0 {
                    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = s.name(cx);
                    }));
                    assert!(refused.is_err(), "{name}: refused before adoption");
                }
                let s = cx.adopt_symbol(s).unwrap();
                assert_eq!(s.as_ref().sexp().addr(), addr, "{name}: adoption keeps the symbol");
                let first = robigo_rtcore::Ref::from(s.name(cx)).sexp();
                let made = early.unwrap_or_else(|| {
                    robigo_rtcore::Ref::from(cx.mk_char(name.as_bytes(), Encoding::Native).unwrap()).sexp()
                });
                assert_eq!(first, made, "{name}: B's PRINTNAME is B's mkChar (match finds it)");
                assert_eq!(cx.install(name.as_bytes()).unwrap().as_ref().sexp().addr(), addr, "{name}: one symbol");
                assert_eq!(
                    cx.adopt_symbol(s).unwrap().as_ref().sexp().addr(),
                    addr,
                    "{name}: adopting twice is a no-op"
                );
                if !cfg!(miri) && n % 10 == 0 {
                    iso.cx().collect();
                }
                let cx = iso.cx();
                // SAFETY: as above.
                let s = unsafe { robigo_rtcore::Ref::from_sexp(robigo_rtcore::Sexp::from_ptr(addr as *mut u8), cx) }
                    .as_sym()
                    .unwrap();
                let again = robigo_rtcore::Ref::from(s.name(cx)).sexp();
                assert_eq!(again, first, "{name}: the same PRINTNAME after a collection");
                let made = robigo_rtcore::Ref::from(cx.mk_char(name.as_bytes(), Encoding::Native).unwrap()).sexp();
                assert_eq!(made, first, "{name}: and B's cache still gives it");
                n += 1;
                b_done.store(n, Ordering::SeqCst);
            }
            n
        })
    };
    // B first: a failed assertion there is the failure, not A's SendError
    // (and A, waiting on B, would not end)
    let n = b.join().unwrap();
    a.join().unwrap();
    assert_eq!(n, NAMES);
    assert!(installs.load(Ordering::SeqCst) >= NAMES, "A installed after every hand-off");
}
