//! Argument aliases: an S3 method's frame shares the arguments it gets from
//! its generic without making their promises ([`ALIAS`] slots).
//!
//! GNU R's `dispatchMethod` calls the method with the generic's `promargs`,
//! so the method's binding of an argument is the very promise the
//! generic's binding holds: one PROMSXP, whose value a fresh object is
//! counted by once. The generic's arguments here are slots (`promise.rs`),
//! and making each one's promise for the method is an allocation, a
//! residue and a `clearPromise` per argument per dispatch. An alias slot
//! of the method's frame stands for that promise instead, until something
//! needs it as an object.
//!
//! The rules:
//!
//! - An alias is made only by [`Cx::new_closure_frame`]
//!   ([`super::SlotInit::Alias`]), of a delayed or forced argument slot of a
//!   frame whose call is running (the generic's, below the method's); an
//!   alias of an alias is made of its target. Its `val` is the target's
//!   frame (traced, not counted), its `aux` the target's index, MISSING 0.
//!   The target slot has [`META_ALIASED`], and the residue list holds one
//!   entry per alias ([`super::promise::Residues`]).
//! - A read through an alias is a read of the target: forcing it forces
//!   the target in place, with the target's early-value bookkeeping
//!   ([`Cx::force_arg`]); `missing()` reads the target's expression.
//! - Before anything needs the binding as an object, or changes the target
//!   (a write or removal into it, its materialization, a cell made of it),
//!   the target becomes the argument's promise and every alias of it a value
//!   holding that promise ([`resolve`], from [`Cx::ensure_object`]): what
//!   GNU R's bindings held all along.
//! - A write or removal of the alias itself drops it ([`unalias`]),
//!   counting nothing, and so does `R_CleanupEnvir` of the method's frame.
//! - When the method's call ends ([`Cx::end_call`], by a return or a jump),
//!   every alias the cleanup did not drop becomes the promise. No alias
//!   outlives its method's call, so its target, whose call runs below the
//!   method's, outlives it.

#![allow(unsafe_code)]

use super::frame::layout::{self, ALIAS, DELAYED, FORCED, META_ALIASED, META_LOCKED, META_PENDING, SlotRef, VALUE};
use super::{Isolate, chain};
use crate::Cx;
use robigo_heap::Sexp;

/// The slot the alias `s` reads.
///
/// # Safety
/// `s` is a live slot of kind [`ALIAS`].
#[inline(always)]
pub(super) unsafe fn target(s: SlotRef) -> SlotRef {
    // SAFETY: the caller's contract: an alias's `val` is its target's frame
    // and its `aux` the target's index.
    unsafe { SlotRef { owner: s.val(), index: s.aux_bits() as usize } }
}

/// The slot a read of `s` reads: its target if it is an alias, else `s`.
///
/// # Safety
/// `s` is a live slot.
#[inline(always)]
pub(super) unsafe fn through(s: SlotRef) -> SlotRef {
    // SAFETY: the caller's contract.
    unsafe { if s.kind() == ALIAS { target(s) } else { s } }
}

/// Make the fresh slot `s` (keyed by `sym`, its words not yet written; of a
/// frame [`Cx::new_closure_frame`] is making) an alias of slot `index` of
/// the slot frame `g`, or of that slot's target if it is an alias itself,
/// with definition stamp `stamp`.
///
/// # Safety
/// `s` is a slot of a frame being made, whose words hold nothing counted;
/// `g` is a live slot frame of `iso`'s heap whose call is running, with
/// slot `index` delayed or forced (or an alias of such a slot).
pub(super) unsafe fn bind(iso: &Isolate, s: SlotRef, sym: Sexp, stamp: u64, g: Sexp, index: usize) {
    // SAFETY: the caller's contract.
    unsafe {
        assert!(layout::is_slots(g) && index < layout::capacity(g), "robigo: an alias of a slot that is not there");
        let t = through(SlotRef { owner: g, index });
        assert!(matches!(t.kind(), DELAYED | FORCED), "robigo: an alias of a slot of kind {}", t.kind());
        debug_assert!(t.owner.extra() & layout::CALL_LIVE != 0, "robigo: an alias of a frame whose call is over");
        s.set_meta(stamp);
        s.set_key(sym, ALIAS);
        s.store_val_uncounted(t.owner);
        s.set_aux_bits(t.index as u64);
        t.set_flag(META_ALIASED, true);
        iso.envs().residues.push_alias(s.owner, s.index as u32, t.owner, t.index as u32);
    }
}

/// Drop the alias in slot `s` (a write or removal replaces the binding, or
/// the cleanup releases it): its entry goes, its target loses
/// [`META_ALIASED`] if no other alias reads it, and `s` is left a value
/// slot holding null (its stamp, MISSING and lock kept) for the caller to
/// store over or unbind. Nothing is counted.
///
/// # Safety
/// `s` is a live slot of kind [`ALIAS`] of `iso`'s heap.
pub(super) unsafe fn unalias(iso: &Isolate, s: SlotRef) {
    // SAFETY: the caller's contract; the target is live (the alias's rule).
    unsafe {
        let t = target(s);
        let r = &iso.envs().residues;
        let gone = r.remove_alias(s.owner, s.index as u32);
        debug_assert_eq!(gone, Some((t.owner, t.index as u32)), "robigo: an alias without its entry");
        if !r.targeted(t.owner, t.index as u32) {
            t.set_flag(META_ALIASED, false);
        }
        // (ASSIGNMENT_PENDING is a cell's: a complex assignment makes the
        // binding cell-backed, which resolves the alias first)
        debug_assert!(!s.has(META_PENDING), "robigo: an assignment pending on an alias");
        s.clear_raw_val();
        s.clear_raw_aux();
        s.set_key(s.sym(), VALUE);
    }
}

/// The aliased slot `t` was just made a value holding the argument's
/// promise `p` ([`Cx::ensure_object`]): every alias of it becomes a value
/// holding `p` too (counted, as GNU R's bindings count it; a locked one
/// marks it not mutable), and `t` loses [`META_ALIASED`]. Nothing is
/// allocated.
///
/// # Safety
/// `t` is a live slot of `iso`'s heap that had [`META_ALIASED`], `p` a live
/// promise.
pub(super) unsafe fn resolve(iso: &Isolate, t: SlotRef, p: Sexp) {
    // SAFETY: the caller's contract; each alias is a live slot of a live
    // method's frame (the residue list roots it).
    unsafe {
        iso.envs().residues.take_targeting(t.owner, t.index as u32, |m, i| {
            let s = SlotRef { owner: m, index: i as usize };
            debug_assert_eq!(s.kind(), ALIAS, "robigo: an alias entry of a slot that is no alias");
            debug_assert_eq!(target(s), t, "robigo: an alias entry of another target");
            s.clear_raw_val();
            s.clear_raw_aux();
            s.set_key(s.sym(), VALUE);
            s.store_val(p);
            if s.has(META_LOCKED) {
                chain::mark_not_mutable(p);
            }
        });
        t.set_flag(META_ALIASED, false);
    }
}

impl Cx {
    /// [`Cx::end_call`]'s step for a method's frame `f` that still has
    /// aliases (the cleanup did not drop them: the frame outlives the call,
    /// or the call was left by a jump): each target is made the argument's
    /// promise, which every alias of it then holds. An allocation refused
    /// even after a collection ends the process: an alias must not outlive
    /// the call.
    #[cold]
    #[inline(never)]
    pub(super) fn end_aliases(&mut self, f: crate::Root<'_>) {
        loop {
            let m = f.bind(self).sexp();
            let Some((_, g, t)) = self.iso().envs().residues.alias_of(m) else { break };
            // SAFETY: the target is a live slot frame (the entry roots it)
            // whose slot `t` is delayed or forced: the alias's rule.
            let r = self.scope(|cx, sc| {
                let g = sc.root(cx, unsafe { crate::Ref::from_sexp(g, cx) });
                // SAFETY: a slot of the rooted target frame.
                let s = SlotRef { owner: g.bind(cx).sexp(), index: t as usize };
                cx.ensure_object(s).map(|_| ())
            });
            if r.is_err() {
                eprintln!("robigo: cannot allocate an S3 method argument's promise at the end of its call");
                std::process::abort();
            }
        }
    }

    /// The checks of every read through the alias `s` (debug builds): its
    /// entry is there, its target is a delayed or forced slot of a slot
    /// frame whose call is running, and that call's record is live and
    /// below the method's (when the method's record is pushed yet).
    #[cfg(debug_assertions)]
    pub(super) fn check_alias(&self, s: SlotRef) {
        // SAFETY: a live alias slot (the caller's), whose target's frame is
        // live (traced by the slot); records are read only.
        unsafe {
            let t = target(s);
            assert!(layout::is_slots(t.owner), "robigo: an alias read of a frame that is no slot frame (poisoned?)");
            assert!(t.index < layout::capacity(t.owner), "robigo: an alias past its target's slots");
            assert!(matches!(t.kind(), DELAYED | FORCED), "robigo: an alias of a slot of kind {}", t.kind());
            assert!(t.has(META_ALIASED), "robigo: an alias of a slot not marked aliased");
            assert!(t.owner.extra() & layout::CALL_LIVE != 0, "robigo: an alias read after its target's call");
            let entry = self.iso().envs().residues.alias_target(s.owner, s.index as u32);
            assert_eq!(entry, Some((t.owner, t.index as u32)), "robigo: an alias without its entry");
            // the records' frames, innermost first: the generic's is below
            // the method's innermost one (an `eval` in the generic's frame
            // may run above the method)
            let envs: Vec<Sexp> = self.ctxs().chain().map(|r| r.cloenv()).collect();
            let from = envs.iter().position(|&e| e == s.owner).unwrap_or(0);
            assert!(
                envs[from..].contains(&t.owner),
                "robigo: an alias read whose target's record is gone or above its method's"
            );
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub(super) fn check_alias(&self, _s: SlotRef) {}
}
