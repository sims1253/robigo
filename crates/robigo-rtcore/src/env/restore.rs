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

/// Whether `list` is a pairlist of binding cells (R_NilValue included),
/// and its cells newest first.
///
/// # Safety
/// `list` is live.
unsafe fn binding_cells(list: Sexp, nil: Sexp) -> Option<Vec<Sexp>> {
    let mut cells = Vec::new();
    let mut c = list;
    // SAFETY: the caller's contract; each cell is read only after its type
    // is checked.
    unsafe {
        while c != nil {
            if c.typ() != LISTSXP || c.tag().typ() != SYMSXP {
                return None;
            }
            cells.push(c);
            c = c.cdr();
        }
    }
    Some(cells)
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
        // SAFETY: rooted, live objects of this heap.
        unsafe {
            match kind(iso, e) {
                Kind::Hashed(_) => {
                    if t.typ() != VECSXP || t.is_altrep() || t.std_len() == 0 {
                        return Err(env_error(self, "robigo: an environment's serialized hash table is not a list"));
                    }
                    let n = t.std_len() as usize;
                    let mut count = 0;
                    for i in 0..n {
                        let chain = hashtab::chain_at(t, i);
                        if binding_cells(chain, nil).is_none() {
                            return Err(env_error(
                                self,
                                "robigo: an environment's serialized hash chain is not a frame",
                            ));
                        }
                        if chain != nil {
                            count += 1;
                        }
                    }
                    // GNU R stores FRAME as read (R_NilValue for a hashed
                    // environment GNU R wrote)
                    if f != nil {
                        if binding_cells(f, nil).is_none() {
                            return Err(env_error(self, "robigo: an environment's serialized frame is not a frame"));
                        }
                        env::set_frame(e, f);
                    }
                    env::set_hashtab(e, t);
                    t.set_truelen(count);
                    Ok(())
                }
                Kind::Frame => {
                    if t != nil {
                        return Err(env_error(
                            self,
                            "robigo: an unhashed environment serialized with a hash table is not implemented yet",
                        ));
                    }
                    let Some(cells) = binding_cells(f, nil) else {
                        return Err(env_error(self, "robigo: an environment's serialized frame is not a frame"));
                    };
                    if cells.len() >= MAX_SLOTS {
                        return Err(env_error(
                            self,
                            &format!(
                                "robigo: an environment of more than {MAX_SLOTS} variables is not implemented yet"
                            ),
                        ));
                    }
                    let n = cells.len();
                    let mut seen = std::collections::HashSet::with_capacity(n);
                    if !cells.iter().all(|c| seen.insert(c.tag().addr())) {
                        return Err(env_error(
                            self,
                            "robigo: a serialized frame that binds a variable twice is not implemented yet",
                        ));
                    }
                    if n == 0 {
                        return Ok(());
                    }
                    // one block for them all (the shell has none yet)
                    // (one call into the core)
                    let block = self.retry_alloc(|cx| cx.iso().core().alloc_slot_frame(n, cx.iso().nil))?;
                    let (e, f) = (env.bind(self).sexp(), frame.bind(self).sexp());
                    frame::adopt_overflow(e, block, nil);
                    let cells = binding_cells(f, nil).expect("checked above");
                    for &c in cells.iter().rev() {
                        let sym = c.tag();
                        let s =
                            frame::overflow_slot(e, sym, nil).expect("robigo: a restored frame's block without room");
                        let stamp = frame::next_stamp(e, nil);
                        s.set_meta(stamp as u64 & layout::META_STAMP);
                        s.set_key(sym, layout::CELL);
                        s.store_val(c);
                    }
                    Ok(())
                }
                _ => Err(env_error(self, "robigo: restoring a special environment is not implemented yet")),
            }
        }
    }
}
