//! What `serialize.c` reads and writes of an environment (robigo-rt's
//! `serialize`): `HASHTAB(env)` as GNU R has it, and the restore of an
//! environment `R_Unserialize` made before reading its contents (GNU R's
//! `allocSExp(ENVSXP)`, then `SET_FRAME`, `SET_HASHTAB` and
//! `R_RestoreHashCount`).
//!
//! GNU R registers the environment in its reference table before it reads
//! the contents, so the object exists first and the contents decide its
//! representation afterwards. robigo's two representations have different
//! shapes (a hashed node and a slot frame, `mod.rs`), so the reader looks
//! ahead in the stream for which one the contents are, makes that shell
//! with [`Cx::new_env`], and restores into it here:
//!
//! - a hashed shell takes the table as read: its chains of GNU R binding
//!   cells are GNU R's representation bit for bit (`hashtab.rs`), and the
//!   fill count is recounted as `R_RestoreHashCount` does;
//! - an unhashed shell (a slot frame with no shape) binds each cell of the
//!   pairlist FRAME as a cell-backed overflow slot, oldest first, so that
//!   the frame lists newest first as the pairlist did, and each binding
//!   keeps its cell: value, symbol and gp bits (`MISSING`, `BINDING_LOCK`,
//!   `ACTIVE_BINDING`, `ASSIGNMENT_PENDING`) as written.
//!
//! The cells are checked to be binding cells (LISTSXP with a symbol tag)
//! before anything is bound: the raw lookups read them as such.

#![allow(unsafe_code)]

use super::frame::{self, layout};
use super::hashtab;
use super::{Kind, MAX_SLOTS, env_error, kind};
use crate::exit::RResult;
use crate::handle::Ref;
use crate::isolate::Cx;
use crate::root::Root;
use robigo_heap::types::*;
use robigo_heap::{Sexp, env};

/// Whether `list` is a pairlist of binding cells (LISTSXP cells with a
/// symbol tag; R_NilValue, the empty one, included). Allocates nothing, so
/// a chain of a table is checked without a copy of it.
///
/// # Safety
/// `list` is live.
unsafe fn is_binding_list(list: Sexp, nil: Sexp) -> bool {
    let mut c = list;
    // SAFETY: the caller's contract; each cell is read only after its type
    // is checked.
    unsafe {
        while c != nil {
            if c.typ() != LISTSXP || c.tag().typ() != SYMSXP {
                return false;
            }
            c = c.cdr();
        }
    }
    true
}

/// The cells of a pairlist of binding cells ([`is_binding_list`]), newest
/// first.
///
/// # Safety
/// `list` is a live pairlist of binding cells.
unsafe fn binding_cells(list: Sexp, nil: Sexp) -> Vec<Sexp> {
    let mut cells = Vec::new();
    let mut c = list;
    // SAFETY: the caller's contract.
    unsafe {
        while c != nil {
            cells.push(c);
            c = c.cdr();
        }
    }
    cells
}

impl Cx {
    /// GNU R's `HASHTAB(env)`: the table of a hashed environment, and
    /// R_NilValue for a slot frame, the base environments and the empty
    /// one (whose HASHTAB GNU R leaves R_NilValue). For `serialize`.
    pub fn env_hashtab<'g>(&'g self, env: Ref<'g>) -> Ref<'g> {
        let iso = self.iso();
        // SAFETY: a live object; a hashed environment's table is a live
        // VECSXP reachable from it.
        unsafe {
            match kind(iso, env.sexp()) {
                Kind::Hashed(t) => Ref::from_sexp(t, self),
                _ => self.nil(),
            }
        }
    }

    /// Whether `env` is laid out as GNU R's hashed environment
    /// (`IS_HASHED`): a table, not a slot frame.
    pub fn env_is_hashed(&self, env: Ref<'_>) -> bool {
        // SAFETY: a live object.
        matches!(unsafe { kind(self.iso(), env.sexp()) }, Kind::Hashed(_))
    }

    /// `R_Unserialize`'s `SET_FRAME(s, frame); SET_HASHTAB(s, table);
    /// R_RestoreHashCount(s)` on the environment shell `env`, which the
    /// reader made with [`Cx::new_env`] (hashed if the stream's table is
    /// not R_NilValue, else unhashed) and nothing has bound into yet. The
    /// caller sets ENCLOS, the attributes and the lock afterwards, as GNU R
    /// does.
    // R: serialize.c:ReadItem_Recursive (ENVSXP), envir.c:R_RestoreHashCount
    pub fn restore_env(&mut self, env: Root<'_>, frame: Root<'_>, table: Root<'_>) -> RResult<()> {
        let iso = self.iso();
        let (e, f, t) = (env.bind(self).sexp(), frame.bind(self).sexp(), table.bind(self).sexp());
        iso.check_owns(e);
        iso.check_owns(f);
        iso.check_owns(t);
        let nil = iso.nil;
        // SAFETY: `e` is rooted and live.
        match unsafe { kind(iso, e) } {
            Kind::Hashed(_) => self.restore_hashed(env, frame, table),
            Kind::Frame => {
                // restore_frame's precondition: a shell with nothing bound
                // and no overflow block, whose block it makes
                // SAFETY: `e` is a live slot frame.
                let fresh =
                    unsafe { frame::bindings(e, nil).is_empty() && layout::get(e, layout::OFF_OVERFLOW) == nil };
                if !fresh {
                    return Err(env_error(self, "robigo: restore_env of an environment that is not a new shell"));
                }
                self.restore_frame(env, frame, t == nil)
            }
            _ => Err(env_error(self, "robigo: restoring a special environment is not implemented yet")),
        }
    }

    /// [`Cx::restore_env`] of a hashed shell: the table as read, its fill
    /// count recounted.
    fn restore_hashed(&mut self, env: Root<'_>, frame: Root<'_>, table: Root<'_>) -> RResult<()> {
        let iso = self.iso();
        let (e, f, t) = (env.bind(self).sexp(), frame.bind(self).sexp(), table.bind(self).sexp());
        let nil = iso.nil;
        // SAFETY: `t` is rooted and live; its header is read.
        let shape = unsafe {
            if t.typ() != VECSXP {
                Some("robigo: an environment's serialized hash table is not a list")
            } else if t.is_altrep() {
                Some("robigo: an environment's serialized hash table that is an ALTREP list is not implemented yet")
            } else if t.std_len() == 0 {
                Some("robigo: an environment's serialized hash table has no chains")
            } else {
                None
            }
        };
        if let Some(msg) = shape {
            return Err(env_error(self, msg));
        }
        // SAFETY: as above: a VECSXP in standard storage.
        let n = unsafe { hashtab::size(t) };
        let mut count = 0;
        for i in 0..n {
            // SAFETY: `i` is below the table's length.
            let chain = unsafe { hashtab::chain_at(t, i) };
            // SAFETY: an element of the rooted table, live.
            if !unsafe { is_binding_list(chain, nil) } {
                return Err(env_error(self, "robigo: an environment's serialized hash chain is not a frame"));
            }
            if chain != nil {
                count += 1;
            }
        }
        // GNU R stores FRAME as read (R_NilValue for a hashed environment
        // GNU R wrote)
        if f != nil {
            // SAFETY: `f` is rooted and live.
            if !unsafe { is_binding_list(f, nil) } {
                return Err(env_error(self, "robigo: an environment's serialized frame is not a frame"));
            }
            // SAFETY: rooted, live objects; the store goes through the barrier.
            unsafe { env::set_frame(e, f) };
        }
        // SAFETY: as above; `t` is a checked table of binding chains, so the
        // lookups read it as one.
        unsafe {
            env::set_hashtab(e, t);
            t.set_truelen(count);
        }
        Ok(())
    }

    /// [`Cx::restore_env`] of an unhashed shell: each cell of FRAME bound,
    /// oldest first, as a cell-backed overflow slot.
    fn restore_frame(&mut self, env: Root<'_>, frame: Root<'_>, no_table: bool) -> RResult<()> {
        if !no_table {
            return Err(env_error(
                self,
                "robigo: an unhashed environment serialized with a hash table is not implemented yet",
            ));
        }
        let nil = self.iso().nil;
        let f = frame.bind(self).sexp();
        // SAFETY: `frame` is rooted and live.
        if !unsafe { is_binding_list(f, nil) } {
            return Err(env_error(self, "robigo: an environment's serialized frame is not a frame"));
        }
        // SAFETY: as above, now checked to be a pairlist of binding cells.
        let cells = unsafe { binding_cells(f, nil) };
        // a slot's location holds an index below MAX_SLOTS: MAX_SLOTS slots
        // at most
        let n = cells.len();
        if n > MAX_SLOTS {
            return Err(env_error(
                self,
                &format!("robigo: an environment of more than {MAX_SLOTS} variables is not implemented yet"),
            ));
        }
        let dup = {
            let mut seen: std::collections::HashSet<Ref<'_>> = std::collections::HashSet::with_capacity(n);
            // SAFETY: the cells are live binding cells (checked); their tags
            // are live symbols for this shared borrow of `self`.
            !cells.iter().all(|c| seen.insert(unsafe { Ref::from_sexp(c.tag(), self) }))
        };
        if dup {
            return Err(env_error(
                self,
                "robigo: a serialized frame that binds a variable twice is not implemented yet",
            ));
        }
        if n == 0 {
            return Ok(());
        }
        // one block for them all (the shell has none yet)
        // SAFETY: one call into the core, which calls nothing of ours.
        let block = self.retry_alloc(|cx| unsafe { cx.iso().core().alloc_slot_frame(n, cx.iso().nil) })?;
        let (e, f) = (env.bind(self).sexp(), frame.bind(self).sexp());
        // SAFETY: `e` is the rooted shell, a slot frame with no overflow
        // block; `block` is fresh with room for `n` slots, and nothing has
        // collected since it was made.
        unsafe { frame::adopt_overflow(e, block, nil) };
        // the cells again: the block's allocation may have collected, and a
        // collection may move them. `f` is rooted and nothing changed it, so
        // it is still the pairlist of binding cells checked above.
        // SAFETY: as stated.
        let cells = unsafe { binding_cells(f, nil) };
        for &c in cells.iter().rev() {
            // SAFETY: a live binding cell.
            let sym = unsafe { c.tag() };
            // The block has room for `n` slots and the `n` cells bind `n`
            // distinct symbols (checked), so every cell finds a slot: None
            // would be a bug in this function, not a property of the input.
            // SAFETY: `e` is the rooted frame, which binds no symbol twice
            // (checked) and has room for every cell.
            let s =
                unsafe { frame::overflow_slot(e, sym, nil) }.expect("robigo: a restored frame's block without room");
            // SAFETY: `s` is the unbound slot just keyed by `sym` in `e`'s
            // block; a CELL slot's value is the binding cell, counted by the
            // store.
            unsafe {
                let stamp = frame::next_stamp(e, nil);
                s.set_meta(stamp as u64 & layout::META_STAMP);
                s.set_key(sym, layout::CELL);
                s.store_val(c);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::sym::BINDING_LOCK;
    use crate::{Cx, OwnedIsolate, Ref, Root, Scope};

    /// A pairlist of binding cells, newest first: `b = 2`, then `a = 1`
    /// with its binding locked.
    fn frame<'s>(cx: &mut Cx, s: Scope<'s>) -> Root<'s> {
        let o = s.out(cx);
        let f = o.root();
        cx.build(o, |cx| {
            let (a, b) = (cx.install(b"a")?, cx.install(b"b")?);
            let one: Ref<'_> = cx.alloc_copy(&[1.0])?.finish().into();
            let two: Ref<'_> = cx.alloc_copy(&[2.0])?.finish().into();
            let ca = cx.cons_tagged(one, cx.nil(), a)?;
            // SAFETY: a fresh cell.
            #[allow(unsafe_code)]
            unsafe {
                ca.sexp().set_gp(BINDING_LOCK)
            };
            cx.cons_tagged(two, ca, b)
        })
        .unwrap();
        f
    }

    fn value(cx: &mut Cx, s: Scope<'_>, env: Root<'_>, name: &[u8]) -> f64 {
        let sym = cx.install_rooted(s, name).unwrap();
        let o = s.out(cx);
        let v = o.root();
        cx.find_var_in_frame3(env, sym, true, o).unwrap();
        v.bind(cx).as_scalar::<f64>().unwrap()
    }

    #[test]
    fn an_unhashed_frame_keeps_its_cells_and_order() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let env = o.root();
            cx.build(o, |cx| cx.new_env(cx.nil(), false, 0)).unwrap();
            let f = frame(cx, s);
            let nil = s.root(cx, cx.nil());
            cx.restore_env(env, f, nil).unwrap();
            assert_eq!(value(cx, s, env, b"a"), 1.0);
            assert_eq!(value(cx, s, env, b"b"), 2.0);
            let a = cx.install_rooted(s, b"a").unwrap();
            assert!(cx.binding_is_locked(a, env).unwrap());
            let b = cx.install_rooted(s, b"b").unwrap();
            assert!(!cx.binding_is_locked(b, env).unwrap());
            // FRAME() lists them as the pairlist did, newest first
            let o = s.out(cx);
            let snap = o.root();
            cx.frame_snapshot(env, o).unwrap();
            let c = snap.bind(cx).as_cons().unwrap();
            assert_eq!(c.tag().as_sym().unwrap().name_bytes(), b"b");
            assert_eq!(c.cdr().as_cons().unwrap().tag().as_sym().unwrap().name_bytes(), b"a");
            assert!(c.cdr().as_cons().unwrap().cdr() == cx.nil());
            // a new binding goes on top
            let z = cx.install_rooted(s, b"z").unwrap();
            let v = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[3.0])?.finish().into()))).unwrap();
            cx.define_var(z, v, env).unwrap();
            assert_eq!(value(cx, s, env, b"z"), 3.0);
        });
    }

    #[test]
    fn a_hashed_table_is_taken_and_recounted() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let env = o.root();
            cx.build(o, |cx| cx.new_env(cx.nil(), true, 0)).unwrap();
            let chain = frame(cx, s);
            // a 5-chain table with both cells in chain 3 (lookups find
            // a binding in whichever chain it is in only by hash, so each
            // goes where `R_Newhashpjw % 5` puts it)
            let o = s.out(cx);
            let t = o.root();
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<Ref<'_>>(5)?;
                let c = chain.bind(cx).as_cons().unwrap();
                let a = c.cdr();
                // SAFETY: the cells are live, and fresh enough to unlink.
                #[allow(unsafe_code)]
                unsafe {
                    let ia = crate::sym::hash_of(a.as_cons().unwrap().tag().sexp()) as usize % 5;
                    let ib = crate::sym::hash_of(c.tag().sexp()) as usize % 5;
                    c.as_ref().sexp().set_cdr(cx.nil().sexp());
                    if ia == ib {
                        c.as_ref().sexp().set_cdr(a.sexp());
                        v.set(ib, c.as_ref());
                    } else {
                        v.set(ia, a);
                        v.set(ib, c.as_ref());
                    }
                }
                Ok(v.finish().into())
            })
            .unwrap();
            let nil = s.root(cx, cx.nil());
            cx.restore_env(env, nil, t).unwrap();
            assert_eq!(value(cx, s, env, b"a"), 1.0);
            assert_eq!(value(cx, s, env, b"b"), 2.0);
            let table = cx.env_hashtab(env.bind(cx));
            let filled = (0..5).filter(|&i| table.as_vector::<Ref<'_>>().unwrap().get(i) != cx.nil()).count();
            assert_eq!(table.truelength(), filled);
        });
    }

    /// The message of the error `r` is, resolved.
    fn err(cx: &mut Cx, r: crate::exit::RResult<()>) -> String {
        let e = r.expect_err("an error");
        cx.scope(|cx, s| {
            let out = s.out(cx);
            let m = out.root();
            cx.resolve_exit(e, Some(out));
            String::from_utf8(m.bind(cx).as_vector::<crate::Char<'_>>().unwrap().get(0).bytes().to_vec()).unwrap()
        })
    }

    /// A new shell, hashed or not.
    fn shell<'s>(cx: &mut Cx, s: Scope<'s>, hashed: bool) -> Root<'s> {
        let o = s.out(cx);
        let env = o.root();
        cx.build(o, |cx| cx.new_env(cx.nil(), hashed, 0)).unwrap();
        env
    }

    /// A list of `n` elements, each `elt` (R_NilValue: an empty chain).
    fn list<'s>(cx: &mut Cx, s: Scope<'s>, n: usize, elt: Option<Root<'_>>) -> Root<'s> {
        let o = s.out(cx);
        let l = o.root();
        cx.build(o, |cx| {
            let mut v = cx.alloc_vector::<Ref<'_>>(n)?;
            for i in 0..n {
                v.set(i, elt.map_or(cx.nil(), |e| e.bind(cx)));
            }
            Ok(v.finish().into())
        })
        .unwrap();
        l
    }

    /// A pairlist cell whose tag is not a symbol (an integer).
    fn bad_cell<'s>(cx: &mut Cx, s: Scope<'s>) -> Root<'s> {
        let o = s.out(cx);
        let c = o.root();
        cx.build(o, |cx| {
            let one: Ref<'_> = cx.alloc_copy(&[1i32])?.finish().into();
            let cell = cx.cons(one, cx.nil())?;
            // SAFETY: a fresh cell; its tag an integer vector.
            #[allow(unsafe_code)]
            unsafe {
                cell.sexp().set_tag(one.sexp())
            };
            Ok(cell)
        })
        .unwrap();
        c
    }

    #[test]
    fn each_rejection_says_what_it_rejects() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            // a special environment
            let base = s.root(cx, cx.base_env());
            let r = cx.restore_env(base, nil, nil);
            assert_eq!(err(cx, r), "robigo: restoring a special environment is not implemented yet");
            // a shell that has a binding already
            let used = shell(cx, s, false);
            let z = cx.install_rooted(s, b"z").unwrap();
            cx.define_var(z, nil, used).unwrap();
            let f = frame(cx, s);
            let r = cx.restore_env(used, f, nil);
            assert_eq!(err(cx, r), "robigo: restore_env of an environment that is not a new shell");
            // hashed: the table's shape, a chain, the frame
            let h = shell(cx, s, true);
            let ints = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[1i32])?.finish().into()))).unwrap();
            let r = cx.restore_env(h, nil, ints);
            assert_eq!(err(cx, r), "robigo: an environment's serialized hash table is not a list");
            let empty = list(cx, s, 0, None);
            let r = cx.restore_env(h, nil, empty);
            assert_eq!(err(cx, r), "robigo: an environment's serialized hash table has no chains");
            let bad = bad_cell(cx, s);
            let t = list(cx, s, 3, Some(bad));
            let r = cx.restore_env(h, nil, t);
            assert_eq!(err(cx, r), "robigo: an environment's serialized hash chain is not a frame");
            let t = list(cx, s, 3, None);
            let r = cx.restore_env(h, bad, t);
            assert_eq!(err(cx, r), "robigo: an environment's serialized frame is not a frame");
            // unhashed: a table with it, the frame, twice a name, too many
            let u = shell(cx, s, false);
            let r = cx.restore_env(u, nil, t);
            assert_eq!(
                err(cx, r),
                "robigo: an unhashed environment serialized with a hash table is not implemented yet"
            );
            let r = cx.restore_env(u, bad, nil);
            assert_eq!(err(cx, r), "robigo: an environment's serialized frame is not a frame");
            let twice = s
                .capture(cx, |cx, o| {
                    cx.build(o, |cx| {
                        let a = cx.install(b"a")?;
                        let c1 = cx.cons_tagged(cx.nil(), cx.nil(), a)?;
                        cx.cons_tagged(cx.nil(), c1, a)
                    })
                })
                .unwrap();
            let r = cx.restore_env(u, twice, nil);
            assert_eq!(err(cx, r), "robigo: a serialized frame that binds a variable twice is not implemented yet");
            // (a table that is an ALTREP list: robigo-rtcore makes none to
            // test with)
        });
    }

    /// More cells than a slot frame's locations can index.
    #[test]
    #[cfg_attr(miri, ignore = "65,537 cells under Miri")]
    fn a_frame_beyond_the_slot_limit_is_rejected() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            let u = shell(cx, s, false);
            let many = s
                .capture(cx, |cx, o| {
                    cx.build(o, |cx| {
                        let a = cx.install(b"a")?;
                        let mut l = cx.nil();
                        for _ in 0..=super::MAX_SLOTS {
                            l = cx.cons_tagged(cx.nil(), l, a)?;
                        }
                        Ok(l)
                    })
                })
                .unwrap();
            let r = cx.restore_env(u, many, nil);
            assert_eq!(
                err(cx, r),
                format!("robigo: an environment of more than {} variables is not implemented yet", super::MAX_SLOTS)
            );
        });
    }
}
