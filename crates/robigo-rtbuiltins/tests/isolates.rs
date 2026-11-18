//! Two isolates in one process (H5): a symbol is the process's, its
//! PRINTNAME each isolate's own, so `match` in an isolate that adopted
//! another's symbol finds the name, as GNU R's one CHARSXP per string
//! requires (unique.c hashes strings by address).

use robigo_rtbuiltins::registry;
use robigo_rtcore::{Char, Cx, Encoding, OwnedIsolate, Ref, Sexp};

/// `match(x, table)` with both as character vectors of one element:
/// `x` from this isolate's `mkChar`, `table` as.character of the symbol.
fn match_name(cx: &mut Cx, name: &[u8], sym: Sexp) -> i32 {
    let m = registry::by_name("match").unwrap();
    cx.scope(|cx, s| {
        let x = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let c = cx.mk_char(name, Encoding::Native)?;
                let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                v.set(0, c);
                Ok(v.finish().into())
            })
        });
        let t = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                // SAFETY: a symbol of the process's arena, immortal.
                let y = unsafe { Ref::from_sexp(sym, cx) }.as_sym().unwrap();
                let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                v.set(0, y.name(cx));
                Ok(v.finish().into())
            })
        });
        let (x, t) = (x.unwrap(), t.unwrap());
        let nomatch = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[i32::MIN])?.finish().into()))).unwrap();
        let refs = [x.bind(cx), t.bind(cx), nomatch.bind(cx), cx.nil()];
        let args = s.args(cx, &refs, None);
        let call = s.root(cx, cx.nil());
        let env = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let r = out.root();
        m.invoke(cx, call, args, env, out).unwrap();
        r.bind(cx).as_vector::<i32>().unwrap().get(0)
    })
}

/// Isolate A installs fresh names while isolate B, on another thread,
/// adopts each symbol, reads its PRINTNAME and runs `match` on it, with a
/// collection in B between the first and the second run. A hands each
/// symbol over through a rendezvous channel, then installs fresh names of
/// its own (never handed over) until B says it is done with that symbol;
/// B starts only once A has completed an install after the hand-off. So
/// A's install loop spans B's adoption, PRINTNAME read and `match` by
/// construction (the arena's lock contended whenever both threads run).
/// B makes the name with `mkChar` before it adopts the symbol for even
/// names and after it for odd ones: the two orders, each checked to give
/// one CHARSXP.
#[test]
fn match_finds_an_adopted_symbol_name_under_contention() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    const NAMES: usize = 200;
    let installs = std::sync::Arc::new(AtomicUsize::new(0));
    let b_done = std::sync::Arc::new(AtomicUsize::new(0));
    let (tx, rx) = std::sync::mpsc::sync_channel::<(String, usize)>(0);
    let a = {
        let (installs, b_done) = (std::sync::Arc::clone(&installs), std::sync::Arc::clone(&b_done));
        std::thread::spawn(move || {
            let mut iso = OwnedIsolate::new();
            let cx = iso.cx();
            let mut k = 0;
            for i in 0..NAMES {
                let name = format!("matched_{i}");
                let s = cx.install(name.as_bytes()).unwrap();
                tx.send((name, s.as_ref().sexp().addr())).unwrap();
                // fresh names of A's own until B is done with this one
                while b_done.load(Ordering::SeqCst) <= i {
                    let _ = cx.install(format!("matched_a_only_{k}").as_bytes()).unwrap();
                    installs.fetch_add(1, Ordering::SeqCst);
                    k += 1;
                }
            }
        })
    };
    let b = {
        let (installs, b_done) = (std::sync::Arc::clone(&installs), std::sync::Arc::clone(&b_done));
        std::thread::spawn(move || {
            let mut iso = OwnedIsolate::new();
            let mut n = 0;
            for (name, addr) in rx {
                // A is in its install loop before B starts
                let start = installs.load(Ordering::SeqCst);
                while installs.load(Ordering::SeqCst) == start {
                    std::thread::yield_now();
                }
                // SAFETY: the address of a symbol of the process's arena.
                let sym = unsafe { Sexp::from_ptr(addr as *mut u8) };
                let early = (n % 2 == 0)
                    .then(|| Ref::from(iso.cx().mk_char(name.as_bytes(), Encoding::Native).unwrap()).sexp());
                let pn = {
                    let cx = iso.cx();
                    // SAFETY: as in `match_name`.
                    let y = unsafe { Ref::from_sexp(sym, cx) }.as_sym().unwrap();
                    let y = cx.adopt_symbol(y).unwrap();
                    Ref::from(y.name(cx)).sexp()
                };
                let made = match early {
                    Some(c) => c,
                    None => Ref::from(iso.cx().mk_char(name.as_bytes(), Encoding::Native).unwrap()).sexp(),
                };
                assert_eq!(
                    pn,
                    made,
                    "{name}: one CHARSXP, mkChar {} adoption",
                    if n % 2 == 0 { "before" } else { "after" }
                );
                assert_eq!(match_name(iso.cx(), name.as_bytes(), sym), 1, "{name}: match finds it");
                if n % 10 == 0 {
                    iso.cx().collect();
                }
                assert_eq!(match_name(iso.cx(), name.as_bytes(), sym), 1, "{name}: and after a collection");
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
