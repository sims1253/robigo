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
/// collection in B between the first and the second run. Half the names B
/// makes with `mkChar` before it adopts the symbol, half after.
#[test]
fn match_finds_an_adopted_symbols_name_under_contention() {
    const NAMES: usize = 200;
    let (tx, rx) = std::sync::mpsc::channel::<(String, usize)>();
    let a = std::thread::spawn(move || {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        for i in 0..NAMES {
            let name = format!("matched_{i}");
            let s = cx.install(name.as_bytes()).unwrap();
            tx.send((name, s.as_ref().sexp().addr())).unwrap();
        }
    });
    let b = std::thread::spawn(move || {
        let mut iso = OwnedIsolate::new();
        let mut n = 0;
        for (name, addr) in rx {
            // SAFETY: the address of a symbol of the process's arena.
            let sym = unsafe { Sexp::from_ptr(addr as *mut u8) };
            if n % 2 == 0 {
                let _ = iso.cx().mk_char(name.as_bytes(), Encoding::Native).unwrap();
            }
            {
                let cx = iso.cx();
                // SAFETY: as in `match_name`.
                let y = unsafe { Ref::from_sexp(sym, cx) }.as_sym().unwrap();
                cx.adopt_symbol(y).unwrap();
            }
            assert_eq!(match_name(iso.cx(), name.as_bytes(), sym), 1, "{name}: match finds it");
            if n % 10 == 0 {
                iso.cx().collect();
            }
            assert_eq!(match_name(iso.cx(), name.as_bytes(), sym), 1, "{name}: and after a collection");
            n += 1;
        }
        n
    });
    a.join().unwrap();
    assert_eq!(b.join().unwrap(), NAMES);
}
