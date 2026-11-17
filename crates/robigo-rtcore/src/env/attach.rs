//! `attach`'s environment (robigo-rt's `search`): `R_HashFrame` over the
//! binding cells `do_attach` made, then the resize loop.

#![allow(unsafe_code)]

use super::{Kind, env_error, hashtab, kind};
use crate::exit::RResult;
use crate::isolate::Cx;
use crate::root::Root;
use robigo_heap::env;
use robigo_heap::types::*;

impl Cx {
    /// `R_HashFrame(s)` over `attach`'s frame, then `while
    /// (R_HashSizeCheck(HASHTAB(s))) SET_HASHTAB(s, R_HashResize(...))`:
    /// `attach`'s way to fill the new hashed environment `env` (a shell
    /// from [`Cx::new_env`] of the table size GNU R asks for, nothing bound
    /// yet). Each binding cell of the pairlist `frame` goes, in order, to
    /// the head of its chain, counting only the chains it starts. The cells
    /// are relinked, not copied, so `frame` is consumed: GNU R's
    /// `SET_FRAME(s, frame)` before it has no counterpart, a hashed
    /// environment's bindings being its table's alone, and `frame` must not
    /// be used as a frame afterwards.
    // R: envir.c:R_HashFrame, do_attach
    pub fn hash_frame(&mut self, env: Root<'_>, frame: Root<'_>) -> RResult<()> {
        let iso = self.iso();
        let (e, f) = (env.bind(self).sexp(), frame.bind(self).sexp());
        iso.check_owns(e);
        iso.check_owns(f);
        let nil = iso.nil;
        // SAFETY: `e` is a rooted, live object of this isolate.
        let Kind::Hashed(t) = (unsafe { kind(iso, e) }) else {
            return Err(env_error(self, "robigo: hash_frame of an environment that is not hashed"));
        };
        // the frame's cells, each checked to be a binding cell before any
        // is relinked
        let mut cells = Vec::new();
        let mut c = f;
        while c != nil {
            // SAFETY: `c` is `frame` or the CDR of a cell of it: a live
            // object of the rooted pairlist, read without allocating.
            let (cell, next) = unsafe { (c.typ() == LISTSXP && c.tag().typ() == SYMSXP, c.cdr()) };
            if !cell {
                return Err(env_error(
                    self,
                    "robigo: hash_frame of a frame with a cell that is not a pairlist node tagged with a symbol",
                ));
            }
            cells.push(c);
            c = next;
        }
        // SAFETY: `t` is the live table of the rooted `env` and each cell a
        // binding cell of the rooted `frame`; nothing here allocates, so no
        // collection moves or frees what these raw words name before the
        // loop ends; `hashtab::index` is below the table's length, and the
        // stores go through `Sexp::store`'s barrier.
        unsafe {
            for c in cells {
                let i = hashtab::index(c.tag(), t);
                let chain = hashtab::chain_at(t, i);
                if chain == nil {
                    t.set_truelen(t.truelen() + 1);
                }
                c.set_cdr(chain);
                t.set_elt(i, c);
            }
        }
        // SAFETY: rooted.
        while unsafe { hashtab::needs_resize(env::hashtab(env.bind(self).sexp())) } {
            self.resize(env)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Kind, kind};
    use crate::{Cx, OwnedIsolate, Ref, Root, Scope};

    /// A pairlist of binding cells `a = 1`, `b = 2`, `c = 3`, in that order.
    fn cells<'s>(cx: &mut Cx, s: Scope<'s>) -> Root<'s> {
        s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut l = cx.nil();
                for (n, v) in [(b"c", 3.0), (b"b", 2.0), (b"a", 1.0)] {
                    let x: Ref<'_> = cx.alloc_copy(&[v])?.finish().into();
                    l = cx.cons_tagged(x, l, cx.install(n)?)?;
                }
                Ok(l)
            })
        })
        .unwrap()
    }

    fn value(cx: &mut Cx, s: Scope<'_>, env: Root<'_>, name: &[u8]) -> f64 {
        let sym = cx.install_rooted(s, name).unwrap();
        let v = s.capture(cx, |cx, o| cx.find_var_in_frame3(env, sym, true, o)).unwrap();
        v.bind(cx).as_scalar::<f64>().unwrap()
    }

    #[test]
    fn hashed_as_r_hash_frame_then_resized() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            // a one-chain table: all three cells in it, the last first,
            // one chain counted, then 1 > 0.85 * 1 grows it
            let env = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(cx.nil(), true, 1))).unwrap();
            let f = cells(cx, s);
            cx.hash_frame(env, f).unwrap();
            for (n, v) in [(&b"a"[..], 1.0), (b"b", 2.0), (b"c", 3.0)] {
                assert_eq!(value(cx, s, env, n), v);
            }
            let t = hashtab_of(cx, env.bind(cx));
            assert!(t.len().unwrap() > 1, "the table grew");
        });
    }

    #[test]
    fn a_large_table_keeps_insertion_order_per_chain() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            let env = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(cx.nil(), true, 29))).unwrap();
            let f = cells(cx, s);
            cx.hash_frame(env, f).unwrap();
            let t = hashtab_of(cx, env.bind(cx));
            assert_eq!(t.len(), Some(29));
            let filled = (0..29).filter(|&i| t.as_vector::<Ref<'_>>().unwrap().get(i) != cx.nil()).count();
            assert_eq!(t.truelength(), filled);
            assert_eq!(value(cx, s, env, b"b"), 2.0);
        });
    }

    /// HASHTAB(env), for this module's tests.
    fn hashtab_of<'g>(cx: &'g Cx, env: crate::Ref<'g>) -> crate::Ref<'g> {
        // SAFETY: a live environment of this isolate; its table is live.
        unsafe {
            match kind(cx.iso(), env.sexp()) {
                Kind::Hashed(t) => crate::Ref::from_sexp(t, cx),
                _ => cx.nil(),
            }
        }
    }
}
