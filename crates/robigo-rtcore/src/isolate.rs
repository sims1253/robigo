//! The isolate and its capability (docs/g3.md §2.2, §11).
//!
//! An [`Isolate`] is all of one R session's mutable state. It is pinned
//! (boxed, never moved) and every field that changes lives in a `Cell` or
//! an `UnsafeCell`, so any number of shared references to it may coexist
//! with writes: a [`Cx`] holds a raw pointer to it, never a `&mut`. The
//! `UnsafeCell` fields are reached only through the accessors below, each
//! of which keeps its borrow inside one call that cannot reach the same
//! field again. That is the whole audit of the isolate's aliasing.
//!
//! Re-entry: [`Cx::call_out`] suspends a capability while foreign code runs
//! (C, a package ALTREP method), and [`Reentry::enter`] makes a fresh `Cx`
//! for a callback from it. The isolate's `active` flag allows one running
//! capability at a time, in every build, so no two `&mut Cx` of one isolate
//! are ever in use together (the rule `crates/robigo-capi/src/state.rs:124`
//! states in prose, enforced).

#![allow(unsafe_code)]

use crate::context::Contexts;
use crate::exit::{ExitRecord, PendingExits};
use crate::hooks::{Hooks, Warnings};
use crate::root::Globals;
use crate::vstack::{Slot, ValueStack};
use robigo_heap::types::{DOTSXP, LANGSXP, LISTSXP};
use robigo_heap::{Core, HeapId, Sexp, Trace};
use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::AtomicU32;

/// How to make an isolate.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Value-stack slots to reserve (16 bytes each; pages are committed as
    /// they are used).
    pub value_stack_slots: usize,
    /// GNU R's vector memory limit in bytes (`mem.maxVSize()`), over the
    /// one the heap took from the environment (`R_MAX_VSIZE`, as GNU R's
    /// `R_SizeFromEnv` reads it); None keeps that one.
    pub max_vsize: Option<usize>,
    /// The C library's locale as GNU R reads it.
    pub locale: Locale,
}

/// What GNU R reads from the C library's locale (`LC_CTYPE`,
/// `LC_COLLATE`). Per isolate (L6); GNU R reads it at startup and
/// `Sys.setlocale` changes it. The default is `C.UTF-8`, the oracle's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locale {
    /// A UTF-8 `LC_CTYPE`: GNU R's `utf8locale`, `mbcslocale` and
    /// `known_to_be_utf8`. False is `C`/`POSIX` (single-byte ASCII); other
    /// locales are not modelled.
    pub utf8: bool,
    /// `setlocale(LC_COLLATE)` is `C`: `Scollate` compares the strings'
    /// bytes. Otherwise it collates with ICU's root collator, `C.UTF-8`'s
    /// (unless the environment GNU R reads at the first `Scollate` says
    /// otherwise: robigo-rtbuiltins' `text::scollate`).
    pub collate_c: bool,
}

impl Default for Locale {
    fn default() -> Locale {
        Locale { utf8: true, collate_c: false }
    }
}

impl Default for Options {
    fn default() -> Options {
        // 16 MiB of address space
        Options { value_stack_slots: 1 << 20, max_vsize: None, locale: Locale::default() }
    }
}

/// One R session's state. Reached only through a [`Cx`].
pub struct Isolate {
    /// The heap. Boxed because `Core::new` hands it out so; §11's embedded
    /// heap ("allocation is two loads off `cx`") is a heap API change for
    /// M2's layout work.
    core: UnsafeCell<Box<Core>>,
    /// The poll word every safepoint loads (docs/g3.md §3 item 1): the heap
    /// sets `GC_DUE` here (`Core::set_poll_word`); interrupts and pending
    /// finalizers will add their bits. Declared after `core`, which points
    /// at it, so it outlives the core when the isolate is dropped.
    pub(crate) poll: AtomicU32,
    /// `core.id()`, for the owner checks
    pub(crate) heap: HeapId,
    // the heap's immortal constants, copied so that reading one does not
    // touch the core
    pub(crate) nil: Sexp,
    pub(crate) na_string: Sexp,
    pub(crate) blank_string: Sexp,
    pub(crate) unbound: Sexp,
    pub(crate) missing_arg: Sexp,
    pub(crate) restart_token: Sexp,
    /// GNU R's pre-installed symbols ([`crate::RSym`]), immortal.
    pub(crate) rsyms: [Sexp; crate::sym::RSym::ALL.len()],
    /// The string [`Cx::mk_char`] refused for an embedded NUL, and its
    /// encoding: raised by [`Cx::need_gc`] when it answers the refusal.
    pub(crate) embedded_nul: std::cell::RefCell<Option<(Vec<u8>, crate::Encoding)>>,
    /// The innermost open root scope's serial number; `scope_serial` the
    /// last one handed out (never reused).
    pub(crate) innermost: Cell<u64>,
    pub(crate) scope_serial: Cell<u64>,
    /// A capability of this isolate is running (not suspended by
    /// [`Cx::call_out`]).
    active: Cell<bool>,
    /// The layers' own per-isolate state (`robigo-rt`'s evaluator state,
    /// `robigo-rtapi`'s table of loaded objects), one value per type, which
    /// holds no object except through roots of this isolate (`Global`s,
    /// immortal symbols): dropped with the isolate ([`Cx::ext`]).
    ext: UnsafeCell<Vec<Box<dyn std::any::Any>>>,
    /// Each state's type and address, in `ext`'s order, set with it: `ext`'s
    /// read compares types and casts, with no `Any::type_id` call (the
    /// evaluator reads its state on every call).
    ext_index: UnsafeCell<Vec<(std::any::TypeId, *const ())>>,
    /// The evaluator, dispatcher and condition system (`hooks.rs`).
    pub(crate) hooks: Cell<Hooks>,
    pub(crate) locale: Cell<Locale>,
    /// The locale's names per category (`Sys.getlocale`).
    pub(crate) locale_names: std::cell::RefCell<crate::clib::LocaleNames>,
    /// GNU R's `R_TempDir` (and `Sys_TempDir`): the session's temporary
    /// directory, made at startup (`InitTempDir`), removed at the end
    /// (`R_CleanTempDir`, [`Cx::clean_temp_dir`]).
    pub(crate) temp_dir: std::cell::RefCell<Option<(u32, Vec<u8>)>>,
    /// The embedder's grant of the process environment's writes
    /// (`clib::EnvWrites`, [`OwnedIsolate::grant_env_writes`]): without
    /// it `Sys.setenv`/`Sys.unsetenv` refuse.
    pub(crate) env_writes: std::cell::OnceCell<crate::clib::EnvWrites>,
    /// GNU R's `R_warn_partial_match_args` (`options(warnPartialMatchArgs)`).
    pub(crate) warn_partial_match_args: Cell<bool>,
    /// GNU R's `R_warn_partial_match_attr` (`options(warnPartialMatchAttr)`).
    pub(crate) warn_partial_match_attr: Cell<bool>,
    /// `ROBIGO_NO_INPLACE=1` (docs/g3.md §16): every in-place write copies.
    pub(crate) no_inplace: Cell<bool>,
    /// `ROBIGO_RETRY_TORTURE=1` ([`Cx::retry_torture`]).
    pub(crate) retry_torture: Cell<bool>,
    /// The code stamps and floors ([`Cx::next_code_stamp`]).
    pub(crate) code: CodeStamps,
    /// The C state `options()` sets for the error and warning reports.
    pub(crate) report_options: ReportOptions,
    /// `R_NoEcho` ([`Cx::no_echo`]).
    pub(crate) no_echo: Cell<bool>,
    /// Where `Rprintf` and `REprintf` write (`console.rs`); holds no object.
    pub(crate) console: std::cell::RefCell<crate::console::Console>,
    /// Active `sink()` hooks installed by the runtime (`console.rs`); `None`
    /// while no sink is active so the common path pays only a null check.
    pub(crate) out_sink: Cell<Option<crate::console::SinkHook>>,
    pub(crate) err_sink: Cell<Option<crate::console::SinkHook>>,
    /// Every precise root the isolate holds: what a collection marks first
    /// (docs/g3.md §3, "What a collection marks", item 1). Derived `Trace`,
    /// so a root added here is traced or does not compile.
    roots: UnsafeCell<Roots>,
}

/// The isolate's precise roots: the live object slots of the value stack,
/// the global table, the pending-exit stack, the context chain with the
/// globals it saves (`context.rs`), the environments' state (the fixed
/// environments, base bindings and the global cache) and the deferred
/// warnings. Only [`Cx::need_gc`] takes `&mut` of it, for the collection it
/// hands it to; everything else reads it through the accessors below.
// (the freeze roots, [`Roots::trace_freeze_roots`], are a part of these: the
// boot heap's lazy freeze walks them inside the first collection after the
// boot boundary, robigo-heap's `freeze.rs`)
#[derive(Trace)]
#[trace(freeze = "trace_freeze_roots", search_path = "trace_search_path")]
pub(crate) struct Roots {
    stack: ValueStack,
    globals: Globals,
    exits: PendingExits,
    contexts: Contexts,
    envs: crate::env::EnvState,
    warnings: Warnings,
    /// the C API's globals: the protect stack, R_alloc's stack, the
    /// constants and symbols C reads (`capi.rs`)
    capi: crate::capi::CApi,
}

impl Roots {
    /// The freeze roots (`Cx::freeze_boot_heap`): the environments' state,
    /// the C API's constants and the search path; not the `Globals` (roots
    /// only: what only they hold is never frozen, so releasing one takes no
    /// note, step 2), the value stack, the contexts, the pending exits or
    /// the warnings, which are the session's.
    /// The search path's environments reach the visitor as copies: an
    /// edge it writes back is not stored (a debug assertion refuses one),
    /// so the visitor must only read, as the freeze's does.
    pub(crate) fn trace_freeze_roots<V: robigo_heap::Visitor + ?Sized>(&mut self, v: &mut V) {
        self.envs.trace_freeze_roots(v);
        self.capi.trace_freeze_constants(v);
        self.trace_search_path(v);
    }

    /// The search path's environments after the global one (amendment 4:
    /// the attached packages a start with default packages loaded): a part
    /// of the freeze roots, and the walk the frozen-reachability pass's
    /// deferred search-path test compares (`Trace::trace_search_path`).
    /// The one walk of it, so the two cannot differ.
    pub(crate) fn trace_search_path<V: robigo_heap::Visitor + ?Sized>(&mut self, v: &mut V) {
        let base = self.envs.base_raw();
        let mut e = self.envs.global_raw();
        // (a rewired chain may cycle: each environment once, Sol r3 S9; the
        // global environment itself is seen from the start, so a chain led
        // back to it, `parent.env(globalenv()) <- e` with e's parent the
        // global one in a profile, never offers it as a root)
        // address-key: stop-the-world GC/freeze root walk on Roots (no &Cx); non-moving visitor asserted below
        let mut seen = std::collections::HashSet::from([e.addr()]);
        loop {
            // SAFETY: the global environment and its enclosures are live
            // environments of this isolate (the search path's chain ends at
            // base, then the empty environment).
            e = unsafe { robigo_heap::env::enclos(e) };
            if e.is_null() || e == base || e == self.envs.empty_raw() || !seen.insert(e.addr()) {
                break;
            }
            // the visitor gets a copy: the chain is walked, never rewritten
            // (the freeze's visitor reads; a moving one would be refused,
            // the freeze being off under ROBIGO_GC_MOVE)
            let mut x = e;
            v.edge(&mut x);
            debug_assert_eq!(x, e, "robigo: the freeze roots' search-path walk does not take a moved edge");
        }
    }
}

/// What a frame (a scope, a native call, a callback) restores when it ends,
/// on every path: the value-stack top and the innermost scope.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub(crate) top: NonNull<Slot>,
    pub(crate) innermost: u64,
    /// Where the frame was opened (`poison-dead` feature): the line its
    /// restore's poisons name (`Cx::scope`'s, `call_out`'s and
    /// `enter_raw`'s callers), not the line of the guard that ends it.
    #[cfg(feature = "poison-dead")]
    pub(crate) at: &'static std::panic::Location<'static>,
}

impl Isolate {
    #[inline(always)]
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub(crate) fn frame(&self) -> Frame {
        Frame {
            top: self.stack().top(),
            innermost: self.innermost.get(),
            #[cfg(feature = "poison-dead")]
            at: std::panic::Location::caller(),
        }
    }

    #[inline(always)]
    pub(crate) fn restore(&self, f: Frame) {
        #[cfg(feature = "poison-dead")]
        self.stack().restore_at(f.top, f.at);
        #[cfg(not(feature = "poison-dead"))]
        self.stack().restore(f.top);
        self.innermost.set(f.innermost);
    }

    /// The heap, for one call into it.
    ///
    /// # Safety
    /// The result is dropped before anything else reaches the core: the
    /// caller makes no call with it that could get to another `core()`
    /// (the core calls nothing of ours: this crate registers no root hook).
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub(crate) unsafe fn core(&self) -> &mut Core {
        // SAFETY: the caller's contract: no other reference into the core
        // is live while this one is.
        unsafe { &mut *self.core.get() }
    }

    /// The value stack. Its mutable parts are `Cell`s, so shared references
    /// to it may coexist.
    #[inline(always)]
    pub(crate) fn stack(&self) -> &ValueStack {
        // SAFETY: the only `&mut Roots` is `Cx::need_gc`'s, made while its
        // capability is exclusively borrowed (no reference from `iso()` of
        // it is live) and dropped before it returns; the collection it calls
        // reaches nothing of ours. References from guards are made and
        // dropped in their `drop`.
        unsafe { &(*self.roots.get()).stack }
    }

    /// The context chain and its globals. Every field is a `Cell` (as the
    /// value stack's), the detached segments' list a `RefCell` borrowed only
    /// for a push or a copy, so shared references may coexist.
    #[inline(always)]
    pub(crate) fn contexts(&self) -> &Contexts {
        // SAFETY: as in `stack`.
        unsafe { &(*self.roots.get()).contexts }
    }

    /// The C API's globals block (`capi.rs`). Its changing parts are cells,
    /// which C writes while no capability of the isolate runs.
    #[inline(always)]
    pub(crate) fn capi_globals(&self) -> &crate::capi::CGlobals {
        // SAFETY: as in `stack`: the only `&mut Roots` is `Cx::need_gc`'s
        // (a collection's), made while no reference from `iso()` is live.
        unsafe { &(*self.roots.get()).capi.g }
    }

    /// `R_FalseValue`, `R_TrueValue`, `R_LogicalNAValue`.
    #[inline(always)]
    pub(crate) fn capi_logicals(&self) -> [Sexp; 3] {
        // SAFETY: as in `capi_globals`.
        unsafe { (*self.roots.get()).capi.logicals }
    }

    /// The environments' state (`env/mod.rs`). Its changing parts are
    /// cells, so shared references to it may coexist with their writes.
    #[inline(always)]
    pub(crate) fn envs(&self) -> &crate::env::EnvState {
        // SAFETY: as in `stack`: the only `&mut Roots` is `Cx::need_gc`'s,
        // made while no reference from `iso()` is live.
        unsafe { &(*self.roots.get()).envs }
    }

    /// All the roots, for a collection.
    ///
    /// # Safety
    /// No other reference into the roots is live while the result is, and
    /// nothing reaches this isolate meanwhile.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub(crate) unsafe fn roots_mut(&self) -> &mut Roots {
        // SAFETY: the caller's contract.
        unsafe { &mut *self.roots.get() }
    }

    // The global table and the pending-exit stack: each method borrows its
    // table for its own body only and calls nothing that reaches the isolate.

    // (out of line: its callers' fast paths reach it only on a miss, and
    // its table growth inlined there costs them registers)
    #[inline(never)]
    pub(crate) fn global_add(&self, x: Sexp) -> u32 {
        // SAFETY: the borrow ends with this call, which calls nothing of ours.
        unsafe { (*self.roots.get()).globals.add(x) }
    }
    pub(crate) fn global_get(&self, idx: u32) -> Sexp {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).globals.get(idx) }
    }
    pub(crate) fn global_remove(&self, idx: u32) {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).globals.remove(idx) }
    }
    /// The global table's `i`th entry (None: free), of `global_slots()`.
    pub(crate) fn global_slot(&self, i: usize) -> Option<Sexp> {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).globals.slot(i) }
    }
    pub(crate) fn global_slots(&self) -> usize {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).globals.slots() }
    }
    pub(crate) fn exit_push(&self, r: ExitRecord) {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).exits.records.push(r) }
    }
    pub(crate) fn exit_pop(&self) -> Option<ExitRecord> {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).exits.records.pop() }
    }
    /// The `i`th record from the bottom.
    pub(crate) fn exit_at(&self, i: usize) -> Option<ExitRecord> {
        // SAFETY: as in `global_add`.
        unsafe {
            let records = &(*self.roots.get()).exits.records;
            records.get(i).copied()
        }
    }
    pub(crate) fn exit_count(&self) -> usize {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).exits.records.len() }
    }
    /// Set the top record's leg stop (`context.rs`).
    pub(crate) fn exit_set_stop(&self, stop: usize) {
        // SAFETY: as in `global_add`.
        unsafe {
            if let Some(r) = (*self.roots.get()).exits.records.last_mut() {
                r.stop = stop;
            }
        }
    }

    /// The layer state of type `T`, if one is installed.
    #[inline(always)]
    pub(crate) fn ext<T: 'static>(&self) -> Option<&T> {
        let t = std::any::TypeId::of::<T>();
        // SAFETY: the list and its index change only in `set_ext`, which
        // takes the capability's `&mut Cx`: no `&Cx`, so no reference from
        // here, is live then (a suspended capability's `&Cx` ended with
        // `call_out`'s `&mut`). An entry's address is that of the boxed `T`
        // the list holds at its position (set together by `set_ext`, its
        // type compared here); the values are boxed, so it stays valid while
        // the list grows.
        unsafe { (*self.ext_index.get()).iter().find(|e| e.0 == t).map(|e| &*(e.1 as *const T)) }
    }

    /// Install a layer state, replacing the one of its type.
    ///
    /// # Safety
    /// No reference returned by [`Isolate::ext`] is live.
    pub(crate) unsafe fn set_ext<T: 'static>(&self, v: Box<T>) {
        let t = std::any::TypeId::of::<T>();
        // The new box in (replacing the state of its type, or added), its
        // address taken from where it is stored (a pointer taken before the
        // box moved would not be valid for it: moving a box reborrows its
        // contents), the index updated, and only then the old box dropped:
        // a destructor that panics leaves the index describing the boxes the
        // list holds.
        // SAFETY: the caller's contract.
        let old = unsafe {
            let list = &mut *self.ext.get();
            let index = &mut *self.ext_index.get();
            let at = index.iter().position(|e| e.0 == t);
            let old = match at {
                Some(i) => Some(std::mem::replace(&mut list[i], v as Box<dyn std::any::Any>)),
                None => {
                    list.push(v as Box<dyn std::any::Any>);
                    None
                }
            };
            let i = at.unwrap_or(list.len() - 1);
            let p = &*list[i] as *const dyn std::any::Any as *const ();
            match at {
                Some(i) => index[i].1 = p,
                None => index.push((t, p)),
            }
            old
        };
        drop(old);
    }

    pub(crate) fn warnings_len(&self) -> usize {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).warnings.len() }
    }
    pub(crate) fn warnings_push(&self, call: Sexp, msg: Sexp) {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).warnings.push(call, msg) }
    }
    pub(crate) fn warnings_at(&self, i: usize) -> (Sexp, Sexp) {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).warnings.at(i) }
    }
    pub(crate) fn warnings_clear(&self) {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).warnings.clear() }
    }
    pub(crate) fn warnings_limit(&self) -> usize {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).warnings.limit() }
    }
    pub(crate) fn warnings_set_limit(&self, k: usize) {
        // SAFETY: as in `global_add`.
        unsafe { (*self.roots.get()).warnings.set_limit(k) }
    }

    /// Whether `x`, a live object of some heap, belongs to this isolate's.
    #[inline(always)]
    pub(crate) fn owns(&self, x: Sexp) -> bool {
        // SAFETY: the callers pass objects from `Ref`s and `Char`s, which
        // are live by the minting rule (lib.rs).
        let h = unsafe { x.heap_id() };
        if h == self.heap {
            return true;
        }
        // the shared constants are every isolate's (H5): R_NilValue, the
        // commonest object checked, by its address, the others by their
        // heap id (no call, which would cost every caller registers). A
        // symbol is the process's, so its identity is every isolate's; its
        // name is the isolate's only once it took it (`Cx::adopt_symbol`).
        x == self.nil || h.is_shared()
    }

    /// Panics unless `x` (a live object) belongs to this isolate: an object
    /// of another isolate must never be rooted or stored here, because that
    /// isolate's collector does not see our roots.
    #[inline(always)]
    pub(crate) fn check_owns(&self, x: Sexp) {
        if !self.owns(x) {
            foreign_object();
        }
    }
}

#[cold]
#[inline(never)]
fn foreign_object() -> ! {
    panic!("robigo: an object of another isolate")
}

#[cold]
#[inline(never)]
pub(crate) fn foreign_handle() -> ! {
    panic!("robigo: a handle of another isolate")
}

/// The capability to run on an isolate: `!Send`, `!Sync`, never `Copy`.
///
/// `&Cx` is a read interval: reads, allocation (which never collects),
/// rooting into an open scope, writing an `Out`. `&mut Cx` is everything
/// that may collect, run R or C code, signal, or mutate an existing object,
/// and taking it ends every [`crate::Ref`] and slice borrowed from the
/// `&Cx` (docs/g3.md §2.2).
///
/// A `Cx` holds only its isolate's pointer and caches nothing, so a
/// capability suspended by [`Cx::call_out`] needs no reload when it resumes.
///
/// `Cx` is unsized (a zero-length slice tail), so `&mut Cx` cannot be
/// swapped, replaced or taken (`std::mem::swap` and its kin need `Sized`):
/// a capability stays where it was made, in its owner or in a callback's
/// frame, and can never be moved to another owner or outlive its isolate.
/// The price is a two-word `&mut Cx` whose length word is always 0.
pub type Cx = Capability<[()]>;

/// The storage of a [`Cx`]; only this crate makes one, always with an
/// empty array tail, and hands it out unsized.
pub struct Capability<T: ?Sized> {
    /// (a raw pointer: this also makes `Cx` `!Send` and `!Sync`)
    iso: NonNull<Isolate>,
    _unsized: T,
}

/// A capability for `iso`, sized, to be handed out as `&mut Cx`.
#[inline(always)]
fn capability(iso: NonNull<Isolate>) -> Capability<[(); 0]> {
    Capability { iso, _unsized: [] }
}

impl Cx {
    /// The isolate.
    #[inline(always)]
    pub(crate) fn iso(&self) -> &Isolate {
        // SAFETY: a `Cx` is only reachable while its isolate is live: an
        // owner's is handed out only while it points at the owner's isolate,
        // which lives until the owner is dropped, and a callback's lives
        // within one call that `enter_raw`'s contract keeps the isolate alive
        // for. Shared references to the isolate may coexist with writes
        // because everything that changes is in a `Cell` or `UnsafeCell`.
        unsafe { self.iso.as_ref() }
    }

    #[inline(always)]
    pub(crate) fn iso_ptr(&self) -> NonNull<Isolate> {
        self.iso
    }

    /// Whether `other` is a capability of the same isolate.
    pub fn same_isolate(&self, other: &Cx) -> bool {
        self.iso == other.iso
    }

    /// Run `f` with this capability suspended: foreign code (C, a package
    /// ALTREP method) that may call back into the isolate through the
    /// [`Reentry`] it is given. Like every native dispatch, the value-stack
    /// top and the innermost scope are restored when `f` returns or
    /// unwinds, so a callback's roots die with it.
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub fn call_out<R>(&mut self, f: impl FnOnce(Reentry<'_>) -> R) -> R {
        let iso = self.iso();
        debug_assert!(iso.active.get(), "robigo: a suspended capability in use");
        // the C API's thread-local names this isolate while its foreign code
        // runs (docs/g3.md §11), and the previous one again after
        let g = self.cglobals_ptr();
        let cg = crate::capi::set_current(g);
        let _restore = Suspend { iso: self.iso_ptr(), frame: iso.frame(), cg, g };
        iso.active.set(false);
        f(Reentry { iso: self.iso_ptr(), _c: PhantomData })
    }

    /// GNU R's `R_Visible`: whether the value just computed prints at top
    /// level. The builtin dispatch sets it from names.c's eval column around
    /// each call; a builtin whose column says it decides
    /// (`Visibility::Custom`, X = 2) sets it itself before returning, as
    /// GNU R's `do_invisible` sets `R_Visible = FALSE`. One flag, in the
    /// context state, which exits save and restore as GNU R's
    /// `R_jumpctxt` and `endcontext` do (`context.rs`).
    #[inline(always)]
    pub fn visible(&self) -> bool {
        self.ctxs().visible.get()
    }

    /// Set `R_Visible`.
    #[inline(always)]
    pub fn set_visible(&mut self, on: bool) {
        self.ctxs().visible.set(on);
    }

    /// Value-stack slots in use.
    pub fn stack_depth(&self) -> usize {
        self.iso().stack().depth()
    }

    /// Value-stack slots still free: R entries compare this with their
    /// margin to raise GNU R's depth errors before the stack overflows.
    pub fn stack_headroom(&self) -> usize {
        self.iso().stack().headroom()
    }

    /// The value stack's reserved slot range `[lo, hi)`; the guard page
    /// starts at `hi`.
    pub fn stack_bounds(&self) -> (usize, usize) {
        self.iso().stack().bounds()
    }

    /// Every object this isolate's precise roots reach directly, in §3's
    /// order of kinds: live value-stack object slots, globals, the
    /// pending-exit stack, the context chain. H2's typed root providers
    /// walk the same list.
    pub fn for_each_root(&self, f: &mut dyn FnMut(Sexp)) {
        let iso = self.iso();
        // `f` may use this isolate (push a root, add a global): every table
        // is read one entry at a time, with no borrow across the call
        iso.stack().for_each_object(f);
        for i in 0..iso.global_slots() {
            if let Some(x) = iso.global_slot(i) {
                f(x);
            }
        }
        for i in 0..iso.exit_count() {
            if let Some(r) = iso.exit_at(i) {
                f(r.value);
                f(r.condition);
                f(r.call);
            }
        }
        iso.contexts().for_each_object(f);
        iso.envs().for_each_root(f);
        for i in 0..iso.warnings_len() {
            let (c, m) = iso.warnings_at(i);
            f(c);
            f(m);
        }
    }

    /// The embedder's grant of the process environment's writes, if it
    /// gave this isolate one ([`OwnedIsolate::grant_env_writes`]).
    pub fn env_writes(&self) -> Option<&crate::clib::EnvWrites> {
        self.iso().env_writes.get()
    }

    /// The locale GNU R reads ([`Locale`]).
    #[inline(always)]
    pub fn locale(&self) -> Locale {
        self.iso().locale.get()
    }

    /// Change the locale (`Sys.setlocale`).
    pub fn set_locale(&mut self, l: Locale) {
        self.iso().locale.set(l);
        // the C globals that read it (Defn.h's mbcslocale, R_MB_CUR_MAX)
        self.cglobals().set_locale(l);
    }

    /// The locale's names per category, as `setlocale(category, NULL)`
    /// gives them (`Sys.getlocale`).
    pub fn locale_names(&self) -> crate::clib::LocaleNames {
        self.iso().locale_names.borrow().clone()
    }

    /// One category's locale name, `setlocale(cat, NULL)`
    /// ([`crate::clib::LocaleNames::get`]).
    pub fn locale_name(&self, cat: crate::clib::category::c_int) -> Vec<u8> {
        self.iso().locale_names.borrow().get(cat)
    }

    /// Set the locale's names (startup's, `Sys.setlocale`'s).
    pub fn set_locale_names(&mut self, n: crate::clib::LocaleNames) {
        *self.iso().locale_names.borrow_mut() = n;
    }

    /// GNU R's `R_TempDir`: the session's temporary directory, if made.
    pub fn temp_dir(&self) -> Option<Vec<u8>> {
        self.iso().temp_dir.borrow().as_ref().map(|(_, d)| d.clone())
    }

    /// Set `R_TempDir` (and `Sys_TempDir`) to the directory `dir` this
    /// process made, or forget it (`R_TempDir = NULL`, for
    /// `tempdir(check = TRUE)`'s remaking: the old one is then left as it is,
    /// as GNU R leaves it). The current one is removed at the end
    /// ([`Cx::clean_temp_dir`]: robigo-rt's R_CleanUp, which every orderly
    /// end goes through, or the isolate's drop; a panic past the session's
    /// reset or an abort removes nothing, as a crash of GNU R's does not).
    pub fn set_temp_dir(&self, dir: Option<Vec<u8>>) {
        *self.iso().temp_dir.borrow_mut() = dir.map(|d| (std::process::id(), d));
    }

    /// `R_CleanTempDir()`: the session's temporary directory removed, once
    /// (a second call does nothing), and only by the process that made it:
    /// a forked child shares its parent's session directory
    /// (sysutils.c's note on `R_tmpnam2`) and leaves it.
    // R: platform.c:R_CleanTempDir
    pub fn clean_temp_dir(&self) {
        let d = self.iso().temp_dir.borrow_mut().take();
        if let Some((pid, d)) = d
            && pid == std::process::id()
        {
            crate::clib::remove_temp_dir(&d);
        }
    }

    /// GNU R's `R_warn_partial_match_args`: argument matching warns of
    /// partial matches (`options(warnPartialMatchArgs = TRUE)`).
    #[inline(always)]
    pub fn warn_partial_match_args(&self) -> bool {
        self.iso().warn_partial_match_args.get()
    }

    /// Set `R_warn_partial_match_args` (`options()` does).
    pub fn set_warn_partial_match_args(&mut self, on: bool) {
        self.iso().warn_partial_match_args.set(on);
    }

    /// GNU R's `R_warn_partial_match_attr`: `attr()` warns of a partial
    /// match of the attribute's name (`options(warnPartialMatchAttr = TRUE)`).
    #[inline(always)]
    pub fn warn_partial_match_attr(&self) -> bool {
        self.iso().warn_partial_match_attr.get()
    }

    /// Set `R_warn_partial_match_attr` (`options()` does).
    pub fn set_warn_partial_match_attr(&mut self, on: bool) {
        self.iso().warn_partial_match_attr.set(on);
    }

    /// `ROBIGO_NO_INPLACE=1` (docs/g3.md §16): every write that would
    /// modify an object in place copies it first, so a run compared with a
    /// normal one finds an in-place write that an uncounted reference (a
    /// value on the stack, a C local) sees.
    #[inline(always)]
    pub fn no_inplace(&self) -> bool {
        self.iso().no_inplace.get()
    }

    /// Turn [`Cx::no_inplace`] mode on or off (`robigo-rt` reads
    /// `ROBIGO_NO_INPLACE`; tests run both modes).
    pub fn set_no_inplace(&mut self, on: bool) {
        self.iso().no_inplace.set(on);
    }

    /// `ROBIGO_RETRY_TORTURE=1` (docs/g3.md §16, E9): the first allocation
    /// of every [`Cx::build_with`] and [`Cx::retry_alloc`] call is refused
    /// once, so each build's retry runs (a collection, then the attempt
    /// again from the start). It catches what a refusal at that point
    /// would: an object read outside the build and used inside it after
    /// the collection (unrooted: freed; under `ROBIGO_GC_MOVE=1`: moved),
    /// and an effect a retried attempt repeats.
    ///
    /// Each refusal is one E9's `ROBIGO_FAIL_ALLOC_AT=n` makes: the knob's,
    /// refusing that allocation as a request past a limit is refused; so
    /// the mode reaches no state the heap cannot. It makes many in one run,
    /// as a run near a vector limit does, where `ROBIGO_FAIL_ALLOC_AT` makes
    /// one. It shares the knob, so it is not combined with
    /// `ROBIGO_FAIL_ALLOC_AT` ([`Cx::set_retry_torture`] refuses both).
    ///
    /// It is compiled in only with the crate's `retry-torture` feature
    /// (`cargo build --release -p robigo-rtapi --features
    /// robigo-rtcore/retry-torture`): without it this is false, and the
    /// builds' check is compiled out.
    #[inline(always)]
    pub fn retry_torture(&self) -> bool {
        cfg!(feature = "retry-torture") && self.iso().retry_torture.get()
    }

    /// Turn [`Cx::retry_torture`] mode on or off (`robigo-rt` reads
    /// `ROBIGO_RETRY_TORTURE`). Turning it on panics in a build without
    /// the `retry-torture` feature, which could not honor it, and with
    /// `ROBIGO_FAIL_ALLOC_AT` set (as the heap reads it), whose knob it
    /// would take over.
    pub fn set_retry_torture(&mut self, on: bool) {
        assert!(
            !on || cfg!(feature = "retry-torture"),
            "robigo: ROBIGO_RETRY_TORTURE needs robigo-rtcore built with its retry-torture feature \
             (cargo build --release -p robigo-rtapi --features robigo-rtcore/retry-torture)"
        );
        assert!(
            !on || !self.fail_alloc_set(),
            "robigo: ROBIGO_RETRY_TORTURE and ROBIGO_FAIL_ALLOC_AT both use E9's knob: set one"
        );
        self.iso().retry_torture.set(on);
    }

    /// GNU R's `MAYBE_SHARED(x)` where the answer decides whether `x` is
    /// written in place: always true under [`Cx::no_inplace`].
    #[inline(always)]
    pub fn shared(&self, x: crate::Ref<'_>) -> bool {
        self.no_inplace() || x.maybe_shared()
    }

    /// GNU R's `MAYBE_REFERENCED(x)` where the answer decides whether `x` is
    /// written in place: always true under [`Cx::no_inplace`].
    #[inline(always)]
    pub fn referenced(&self, x: crate::Ref<'_>) -> bool {
        self.no_inplace() || x.maybe_referenced()
    }

    /// The C state `options()` sets for the error and warning reports
    /// (errors.c's globals), which the condition system reads.
    #[inline(always)]
    pub fn report_options(&self) -> &ReportOptions {
        &self.iso().report_options
    }

    /// GNU R's `R_NoEcho`: whether the console, when R is not interactive,
    /// writes neither its prompt nor the lines it reads. The start-up
    /// options set it (`--no-echo`; true for Rscript, this default, false
    /// for `R -f FILE`), `options()`' initial `echo` is its negation, and
    /// `options(echo = )` sets it from then on.
    #[inline(always)]
    pub fn no_echo(&self) -> bool {
        self.iso().no_echo.get()
    }

    /// Set `R_NoEcho` ([`Cx::no_echo`]).
    pub fn set_no_echo(&self, on: bool) {
        self.iso().no_echo.set(on);
    }

    /// A new stamp for a code object an evaluator made from R objects:
    /// never 0, never repeated in the isolate, increasing.
    #[inline]
    pub fn next_code_stamp(&self) -> u64 {
        let c = &self.iso().code;
        let s = c.last.get() + 1;
        c.last.set(s);
        s
    }

    /// Whether the code object stamped `stamp` still stands for what GNU R
    /// would run (docs/g3.md §9.3). `ast`: it was made for GNU R's AST walk
    /// of its source, so a write into a cell of the source
    /// ([`Cx::mark_code_source`] with [`CodeSource::Ast`]) makes it stale.
    /// `closure`: it is a closure's (its frame and argument handling come
    /// from the formals), so a write into any closure's formals
    /// ([`CodeSource::Formals`]) makes it stale. Code GNU R holds as byte
    /// code is never made stale by a write into its source: GNU R's byte
    /// code is an object of its own.
    #[inline(always)]
    pub fn code_is_current(&self, stamp: u64, ast: bool, closure: bool) -> bool {
        let c = &self.iso().code;
        // one load and compare: the floor of each kind is kept
        stamp >= c.floors[(ast as usize) | ((closure as usize) << 1)].get()
    }

    /// Mark the cells `x`'s code is made from as a source of code: from now
    /// on a write into one through a setter (`set_car`, `set_cdr`,
    /// `set_tag`, the C API's `SETCAR`, `SETCDR`, `SET_TAG`) makes the code
    /// [`Cx::code_is_current`] says depends on it stale.
    ///
    /// - [`CodeSource::Ast`]: `x` is an expression compiled for GNU R's AST
    ///   walk. Marked are a call's own cells (its function and argument
    ///   cells), and those of the calls among its arguments, at any depth,
    ///   `quote()`'s too: a write of a new head can make a quoted call code.
    ///   Not a pairlist or any other object a call holds as a value, which
    ///   code holds as a constant (a write into it is the same object's in
    ///   the AST and in the code, and §9.3's untracked raw writes cover the
    ///   rest).
    /// - [`CodeSource::Formals`]: `x` is a closure's formals. Marked are its
    ///   cells, and the cells of the calls among the defaults (as above).
    ///
    /// A marked call's whole tree is marked, always: a tracked write into a
    /// marked cell marks what it stores ([`Cx::note_code_write`]). So the
    /// walk stops at a call already marked: code made again from the same
    /// expression walks nothing, and a tree that reaches itself ends.
    pub fn mark_code_source(&self, x: crate::Ref<'_>, kind: CodeSource) {
        let bit = kind.bit();
        // SAFETY: `x` is a live object of this isolate; the cells reached
        // are live (traced fields of live cells), and the walk writes only
        // their `extra` header word, which no reader holds a borrow of.
        unsafe {
            match kind {
                CodeSource::Ast => mark_call(x.sexp(), bit),
                CodeSource::Formals => mark_cells(x.sexp(), bit),
            }
        }
    }

    /// The write of `v` into cell `x` that a setter is about to make
    /// (rtcore's `set_car`, `set_cdr`, `set_tag`; the C API's setters): if
    /// code was made from `x`, the code it stands for becomes stale, and
    /// `v`, now part of the same source, is marked as `x` is.
    #[inline(always)]
    pub fn note_code_write(&self, x: crate::Ref<'_>, v: crate::Ref<'_>) {
        // SAFETY: a `Ref`'s object is live: its header is readable.
        let e = unsafe { x.sexp().extra() };
        if e & (robigo_heap::extra::cell::AST_SOURCE | robigo_heap::extra::cell::FORMALS_SOURCE) != 0 {
            self.code_write_slow(e, v.sexp());
        }
    }

    #[cold]
    #[inline(never)]
    fn code_write_slow(&self, e: u16, v: Sexp) {
        let c = &self.iso().code;
        let floor = c.last.get() + 1;
        // a kind's floor rises with a write into a source it depends on
        for (kind, f) in c.floors.iter().enumerate() {
            let (ast, closure) = (kind & 1 != 0, kind & 2 != 0);
            if (ast && e & robigo_heap::extra::cell::AST_SOURCE != 0)
                || (closure && e & robigo_heap::extra::cell::FORMALS_SOURCE != 0)
            {
                f.set(floor);
            }
        }
        for bit in [robigo_heap::extra::cell::AST_SOURCE, robigo_heap::extra::cell::FORMALS_SOURCE] {
            if e & bit != 0 {
                // SAFETY: `v` is a live object (a `Ref`'s, from the caller);
                // as in `mark_code_source`. A new CAR is a call or a value,
                // a new CDR further cells of the same list: either is marked
                // throughout.
                unsafe { mark_cells(v, bit) }
            }
        }
    }

    /// The floors [`Cx::code_is_current`] compares with, (AST, closure): a
    /// test reads them to check that a run made no tracked write.
    pub fn code_floors(&self) -> (u64, u64) {
        let c = &self.iso().code;
        (c.floors[1].get(), c.floors[2].get())
    }
}

/// [`Cx::mark_code_source`]'s walk of the call `e` (anything else is not
/// walked): [`mark_cells`] of it, if it is not marked already.
///
/// # Safety
/// `e` is a live object; no reader holds a borrow of the headers of the
/// cells reached.
#[inline]
unsafe fn mark_call(e: Sexp, bit: u16) {
    // SAFETY: the caller's contract.
    unsafe {
        if e.typ() == LANGSXP && e.extra() & bit == 0 {
            mark_cells(e, bit);
        }
    }
}

/// The cells of the list from `c` along the CDRs (to the first one marked
/// already, or the end; `c` may be a call, a new CAR, or any value), and of
/// the calls among their CARs at any depth. A call already
/// marked is marked throughout and is not entered, so a call that reaches
/// itself ends the walk. The calls still to walk are a worklist, not the
/// native stack: data C nested deeply (and quoted) cannot overflow it, and
/// the worklist stays on the stack for the depths code has.
///
/// # Safety
/// As [`mark_call`]; `c` is any live object.
unsafe fn mark_cells(c: Sexp, bit: u16) {
    const INLINE: usize = 32;
    let mut inline = [Sexp::NULL; INLINE];
    let (mut n, mut spill) = (0usize, Vec::new());
    let mut next = Some(c);
    // SAFETY: the caller's contract; a cell's CAR and CDR are live.
    unsafe {
        while let Some(mut c) = next {
            while matches!(c.typ(), LANGSXP | LISTSXP | DOTSXP) && c.extra() & bit == 0 {
                c.set_extra(c.extra() | bit);
                let a = c.car();
                if a.typ() == LANGSXP && a.extra() & bit == 0 {
                    if n < INLINE {
                        inline[n] = a;
                        n += 1;
                    } else {
                        spill.push(a);
                    }
                }
                c = c.cdr();
            }
            next = spill.pop().or_else(|| {
                n = n.checked_sub(1)?;
                Some(inline[n])
            });
        }
    }
}

/// What a cell marked by [`Cx::mark_code_source`] is a source of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSource {
    /// An expression compiled for GNU R's AST walk.
    Ast,
    /// A closure's formals.
    Formals,
}

impl CodeSource {
    fn bit(self) -> u16 {
        match self {
            CodeSource::Ast => robigo_heap::extra::cell::AST_SOURCE,
            CodeSource::Formals => robigo_heap::extra::cell::FORMALS_SOURCE,
        }
    }
}

/// The isolate's code stamps (docs/g3.md §9.3): the last one handed out,
/// and the floors below which code of each kind is stale, indexed by
/// `ast | closure << 1` (the floor of AST closure code is the higher of the
/// other two).
#[derive(Default)]
pub(crate) struct CodeStamps {
    last: Cell<u64>,
    floors: [Cell<u64>; 4],
}

/// errors.c's globals that `options()` sets (options.c's `do_options`) and
/// the error and warning reports read: `R_WarnLength`
/// (`warning.length`), `R_ShowErrorMessages` (`show.error.messages`;
/// `R_tryEvalSilent` turns it off for its evaluation), `R_ShowErrorCalls`
/// (`showErrorCalls`), `R_ShowWarnCalls` (`showWarnCalls`) and
/// `R_NShowCalls` (`showNCalls`). The defaults are those of an Rscript
/// session (`options()`' initial list).
#[derive(Debug)]
pub struct ReportOptions {
    pub warn_length: Cell<usize>,
    pub show_error_messages: Cell<bool>,
    pub show_error_calls: Cell<bool>,
    pub show_warn_calls: Cell<bool>,
    pub n_show_calls: Cell<usize>,
}

impl Default for ReportOptions {
    fn default() -> ReportOptions {
        ReportOptions {
            warn_length: Cell::new(1000),
            show_error_messages: Cell::new(true),
            show_error_calls: Cell::new(true),
            show_warn_calls: Cell::new(false),
            n_show_calls: Cell::new(50),
        }
    }
}

/// Restores a suspended capability's frame and the C API's thread-local,
/// and resumes it.
struct Suspend {
    iso: NonNull<Isolate>,
    frame: Frame,
    /// the thread-local's value before, and this isolate's block
    cg: *mut crate::capi::CGlobals,
    g: *mut crate::capi::CGlobals,
}

impl Drop for Suspend {
    fn drop(&mut self) {
        // SAFETY: made in `call_out` from a live capability, whose borrow
        // keeps the isolate alive until this guard is dropped.
        let iso = unsafe { self.iso.as_ref() };
        debug_assert!(!iso.active.get(), "robigo: a callback did not end before its native call returned");
        iso.restore(self.frame);
        crate::capi::restore_current(self.cg, self.g);
        iso.active.set(true);
    }
}

/// The way back into an isolate whose capability is suspended by
/// [`Cx::call_out`]: what a C export uses to obtain its `Cx` (A1 reaches the
/// isolate through the one C-export thread-local, L6). `Copy`, `!Send`.
#[derive(Clone, Copy)]
pub struct Reentry<'c> {
    iso: NonNull<Isolate>,
    /// the suspended capability's borrow (a `fn` type: only the lifetime
    /// matters; `iso` makes `Reentry` `!Send`)
    _c: PhantomData<fn() -> &'c mut Cx>,
}

impl Reentry<'_> {
    /// Run `f` with a fresh capability (a callback). Panics if a capability
    /// of the isolate is running: a callback needs the outer one suspended.
    pub fn enter<R>(self, f: impl FnOnce(&mut Cx) -> R) -> R {
        // SAFETY: the suspended capability's borrow (`'c`) keeps the isolate
        // alive, and `Reentry` is `!Send`, so this is its thread.
        unsafe { enter_raw(self.iso.as_ptr(), f) }
    }

    /// The isolate, for the C side: what [`enter_raw`] takes back.
    pub fn as_ptr(self) -> *mut Isolate {
        self.iso.as_ptr()
    }
}

/// [`Reentry::enter`] from a raw isolate pointer: the C exports' way in.
///
/// # Safety
/// `iso` comes from [`Reentry::as_ptr`] of a capability that stays
/// suspended for this whole call (so the isolate is live), on this thread.
#[cfg_attr(feature = "poison-dead", track_caller)]
pub unsafe fn enter_raw<R>(iso: *mut Isolate, f: impl FnOnce(&mut Cx) -> R) -> R {
    // SAFETY: the caller's contract: `iso` is a live isolate.
    let iso = unsafe { NonNull::new_unchecked(iso) };
    // SAFETY: as above.
    let i = unsafe { iso.as_ref() };
    if i.active.replace(true) {
        reentered_while_active();
    }
    let _end = EndCallback { iso, frame: i.frame() };
    // No scope is open in the callback: a scope of the suspended capability
    // cannot root anything here, where the callback's end would drop it.
    i.innermost.set(0);
    let mut cx = capability(iso);
    f(&mut cx)
}

#[cold]
#[inline(never)]
fn reentered_while_active() -> ! {
    panic!("robigo: re-entry into an isolate whose capability is running (suspend it with Cx::call_out)")
}

/// Ends a callback: restores its frame and suspends the isolate again.
struct EndCallback {
    iso: NonNull<Isolate>,
    frame: Frame,
}

impl Drop for EndCallback {
    fn drop(&mut self) {
        // SAFETY: `enter_raw`'s contract keeps the isolate alive for the call
        // this guard belongs to.
        let iso = unsafe { self.iso.as_ref() };
        iso.restore(self.frame);
        iso.active.set(false);
    }
}

/// The process's live isolates, counted under this lock (an isolate is
/// counted from before its construction does anything fallible until its
/// teardown is over: [`Registration`]). `clib::env_writer` holds it, with
/// clib's environment lock, for a whole `Sys.setenv`/`Sys.unsetenv` call,
/// so on a glibc older than `clib::MIN_GLIBC` no second isolate starts (and
/// so no other robigo thread can be inside `getenv`) while the environment
/// changes. It is first in the process's lock order, and the fork handlers
/// hold it across a `fork` (robigo-heap's `fork.rs`). In a forked child
/// the count still includes the other threads' isolates, which the child
/// does not have; on an older glibc that only refuses environment writes
/// there.
static LIVE_ISOLATES: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

/// [`LIVE_ISOLATES`] locked (a panic while it was held left a count that
/// is still right: nothing panics between its reads and writes).
/// [`LIVE_ISOLATES`]' address, for a fork test's report of a hung child.
#[cfg(test)]
pub(crate) fn live_isolates_addr() -> *const () {
    (&raw const LIVE_ISOLATES).cast()
}

pub(crate) fn live_isolates() -> std::sync::MutexGuard<'static, usize> {
    LIVE_ISOLATES.lock().unwrap_or_else(|e| e.into_inner())
}

/// The guards `fork_take` holds across a fork, released by `fork_release`
/// on both sides: only the forking thread touches them, between the two
/// handlers.
type ForkHeld = (
    std::sync::MutexGuard<'static, usize>,
    std::sync::RwLockWriteGuard<'static, ()>,
    (std::sync::MutexGuard<'static, ()>, libc::pid_t),
);

struct ForkGuards(std::cell::UnsafeCell<Option<ForkHeld>>);

// SAFETY: written by a fork's `prepare` handler and read by the same fork's
// `parent` or `child` handler, on the forking thread (the C library runs
// them around `fork`). Two forks on two threads never overlap here:
// robigo-heap's `FORK` lock, taken first in `prepare` and released last
// after `parent`/`child`, serializes whole forks, so one fork's handlers
// alone reach the slot at a time.
unsafe impl Sync for ForkGuards {}

static FORK_GUARDS: ForkGuards = ForkGuards(std::cell::UnsafeCell::new(None));

/// The fork handlers' `prepare` for this crate's locks, in the process's
/// order (robigo-heap's `fork.rs`): the live isolates, the environment's
/// lock exclusive, then the time zone session's (a leaf).
fn fork_take() {
    let live = live_isolates();
    let env = crate::clib::env_write_lock();
    let tz = crate::clib::tz_session_fork_take();
    // SAFETY: as for `ForkGuards`: this fork alone, under `FORK`; the slot
    // is empty (the previous fork's `fork_release` took it). That relies on
    // the C library running `parent` after every `prepare`, a fork that
    // fails included: glibc does (verified on 2.44: a fork refused with
    // EAGAIN runs prepare, then parent; `fork_tests`'
    // `a_failed_fork_releases_the_locks`).
    unsafe { *FORK_GUARDS.0.get() = Some((live, env, tz)) };
}

/// The fork handlers' `parent` and `child` for this crate's locks: the
/// reverse order (the time zone session's, `ENV`, then the live isolates).
/// In the child the forking thread is the guards' owner, so unlocking is
/// its own.
fn fork_release() {
    // SAFETY: as for `ForkGuards`.
    if let Some((live, env, (tz, pid))) = unsafe { (*FORK_GUARDS.0.get()).take() } {
        crate::clib::tz_session_fork_release(tz, pid);
        drop(env);
        drop(live);
    }
}

/// The constructor: this crate's locks given to the fork handlers before
/// `main`, while the process has one thread, so no fork can see one held
/// before the handlers know it.
#[cfg(all(target_os = "linux", not(miri)))]
extern "C" fn set_fork_locks_at_load() {
    robigo_heap::set_fork_locks(fork_take, fork_release);
}

// SAFETY: an `.init_array` entry is a function of no arguments that the
// loader calls once, before `main`; `set_fork_locks_at_load` stores two
// atomics and needs nothing of std's runtime.
#[cfg(all(target_os = "linux", not(miri)))]
#[used]
#[unsafe(link_section = ".init_array")]
static SET_FORK_LOCKS_AT_LOAD: extern "C" fn() = set_fork_locks_at_load;

/// [`LIVE_ISOLATES`] if no one holds it (a test's probe).
#[cfg(test)]
pub(crate) fn live_isolates_try_lock() -> Option<std::sync::MutexGuard<'static, usize>> {
    LIVE_ISOLATES.try_lock().ok()
}

/// How many isolates the process has now (constructions under way
/// included).
pub fn live_isolate_count() -> usize {
    *live_isolates()
}

/// One isolate's place in [`LIVE_ISOLATES`]: made before the isolate's
/// construction does anything that can fail and owned by the isolate, so
/// that a panic in construction or teardown takes it out exactly once,
/// and the isolate stays counted throughout its teardown.
struct Registration(());

impl Registration {
    fn new() -> Registration {
        *live_isolates() += 1;
        Registration(())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        *live_isolates() -= 1;
    }
}

/// An isolate and its first capability: what an embedder (the `robigo-rt`
/// binary, a test) creates. Dropping it frees the isolate and its heap.
pub struct OwnedIsolate {
    // dropped after `Drop::drop`'s teardown (or during its unwinding)
    _registration: Registration,
    cx: Capability<[(); 0]>,
}

impl OwnedIsolate {
    pub fn new() -> OwnedIsolate {
        OwnedIsolate::with_options(Options::default())
    }

    pub fn with_options(opts: Options) -> OwnedIsolate {
        // the fork handlers know this crate's locks before this isolate
        // takes one. The constructor did it before `main`; this repeats it
        // in case a linker dropped the constructor. That fallback is
        // best-effort: another thread that already held one of these locks
        // before this first call could still be caught by a fork.
        robigo_heap::set_fork_locks(fork_take, fork_release);
        // counted before it can run anything (an environment write waits);
        // a panic below takes the count back
        let registration = Registration::new();
        // a new symbol's bits come from its name (H5): before any install
        robigo_heap::set_symbol_bits(crate::sym::symbol_bits);
        // The heap's allocation never collects (H2), and no C segment is open
        // until a native call opens one, so no Rust frame is ever scanned
        // (docs/g3.md §3).
        let mut core = Core::new();
        // R: memory.c:R_SetMaxVSize refuses a limit below the current size
        if let Some(limit) = opts.max_vsize {
            assert!(core.set_max_vsize(Some(limit)), "robigo: a vector memory limit below the vector heap's size");
        }
        let contexts = Contexts::new(core.nil(), core.stack_bounds());
        let envs = crate::env::EnvState::new(&mut core);
        let rsyms = crate::sym::install_rsyms(&mut core);
        // R: names.c:InitNames, R_RestartToken = mkSymMarker(mkChar(""))
        let restart_token = core.new_marker_symbol().expect("robigo: no room for R_RestartToken");
        crate::capi::check_layout();
        let capi = crate::capi::CApi::new(
            crate::capi::constants(&mut core, &envs),
            crate::capi::symbols(&rsyms),
            crate::capi::logicals(&mut core),
        );
        let iso = Box::new(Isolate {
            heap: core.id(),
            nil: core.nil(),
            na_string: core.na_string(),
            blank_string: core.blank_string(),
            unbound: core.unbound(),
            missing_arg: core.missing_arg(),
            restart_token,
            rsyms,
            embedded_nul: Default::default(),
            core: UnsafeCell::new(core),
            poll: AtomicU32::new(0),
            innermost: Cell::new(0),
            scope_serial: Cell::new(0),
            active: Cell::new(true),
            ext: UnsafeCell::new(Vec::new()),
            ext_index: UnsafeCell::new(Vec::new()),
            hooks: Cell::new(Hooks::default()),
            locale: Cell::new(opts.locale),
            locale_names: Default::default(),
            temp_dir: Default::default(),
            env_writes: std::cell::OnceCell::new(),
            warn_partial_match_args: Cell::new(false),
            warn_partial_match_attr: Cell::new(false),
            no_inplace: Cell::new(false),
            retry_torture: Cell::new(false),
            code: CodeStamps::default(),
            report_options: ReportOptions::default(),
            no_echo: Cell::new(true),
            console: std::cell::RefCell::new(crate::console::Console::std()),
            out_sink: Cell::new(None),
            err_sink: Cell::new(None),
            roots: UnsafeCell::new(Roots {
                stack: ValueStack::new(opts.value_stack_slots),
                globals: Globals::default(),
                exits: PendingExits::default(),
                contexts,
                envs,
                warnings: Warnings::default(),
                capi,
            }),
        });
        let iso = NonNull::from(Box::leak(iso));
        // SAFETY: the isolate is boxed and never moves; nothing else refers
        // to its roots yet.
        // (ROBIGO_GC_MOVE=1, read by the core when it was made: the exports
        // mark what they hand to C from the start)
        unsafe {
            let i = iso.as_ref();
            let moving = i.core().gc_move();
            let capi = &mut (*i.roots.get()).capi;
            capi.set_isolate(iso.as_ptr());
            capi.g.gc_move.set(moving as std::ffi::c_int);
            capi.g.set_locale(i.locale.get());
        }
        // SAFETY: the isolate is boxed and never moves, and its `poll` field
        // outlives its core (declared after it); nothing else refers to the
        // core yet.
        unsafe {
            let i = iso.as_ref();
            i.core().set_poll_word(NonNull::from(&i.poll));
        }
        OwnedIsolate { _registration: registration, cx: capability(iso) }
    }

    /// Give this isolate the process environment's writes (`Sys.setenv`,
    /// `Sys.unsetenv`): the embedder's [`crate::clib::EnvWrites`], whose
    /// `assume` stated the contract. Once: a second grant is ignored.
    pub fn grant_env_writes(&mut self, w: crate::clib::EnvWrites) {
        let cx: &Cx = &self.cx;
        let _ = cx.iso().env_writes.set(w);
    }

    /// The capability. One at a time: it is borrowed from the owner.
    pub fn cx(&mut self) -> &mut Cx {
        &mut self.cx
    }
}

impl Default for OwnedIsolate {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OwnedIsolate {
    fn drop(&mut self) {
        // Rf_endEmbeddedR's R_CleanTempDir
        self.cx().clean_temp_dir();
        // SAFETY: made by `Box::leak` in `with_options` and freed once, here.
        // A capability cannot be moved out of its storage (`Cx` is
        // unsized), so this owner's still names its isolate, and every
        // capability of the isolate was borrowed from this owner or lived
        // inside a callback of one that was: those have ended.
        drop(unsafe { Box::from_raw(self.cx.iso.as_ptr()) });
        // (the registration is dropped after this, with the fields)
    }
}
