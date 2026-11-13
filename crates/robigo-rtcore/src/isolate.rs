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

use crate::exit::{ExitRecord, PendingExits};
use crate::hooks::{Hooks, Warnings};
use crate::root::Globals;
use crate::vstack::{Slot, ValueStack};
use robigo_heap::{Core, HeapId, Sexp, Trace};
use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// How to make an isolate.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Value-stack slots to reserve (16 bytes each; pages are committed as
    /// they are used).
    pub value_stack_slots: usize,
    /// GNU R's vector memory limit in bytes (`R_MAX_VSIZE`,
    /// `mem.maxVSize()`); None: unlimited.
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
    /// `LC_COLLATE` is `C`/`POSIX`: `Scollate` compares the strings' bytes.
    /// Otherwise GNU R collates with ICU (the root collator in `C.UTF-8`).
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
    /// `core.id()`, for the owner checks
    pub(crate) heap: HeapId,
    // the heap's immortal constants, copied so that reading one does not
    // touch the core
    pub(crate) nil: Sexp,
    pub(crate) na_string: Sexp,
    pub(crate) blank_string: Sexp,
    pub(crate) unbound: Sexp,
    pub(crate) missing_arg: Sexp,
    /// The innermost open root scope's serial number; `scope_serial` the
    /// last one handed out (never reused).
    pub(crate) innermost: Cell<u64>,
    pub(crate) scope_serial: Cell<u64>,
    /// A capability of this isolate is running (not suspended by
    /// [`Cx::call_out`]).
    active: Cell<bool>,
    /// The evaluator, dispatcher and condition system (`hooks.rs`).
    pub(crate) hooks: Cell<Hooks>,
    pub(crate) locale: Cell<Locale>,
    /// Every precise root the isolate holds: what a collection marks first
    /// (docs/g3.md §3, "What a collection marks", item 1). Derived `Trace`,
    /// so a root added here is traced or does not compile.
    roots: UnsafeCell<Roots>,
}

/// The isolate's precise roots: the live object slots of the value stack,
/// the global table, the pending-exit stack, the environments' state (the
/// fixed environments, base bindings and the global cache) and the
/// deferred warnings. Only
/// [`Cx::need_gc`] takes `&mut` of it, for the collection it hands it to;
/// everything else reads it through the accessors below.
#[derive(Trace)]
pub(crate) struct Roots {
    stack: ValueStack,
    globals: Globals,
    exits: PendingExits,
    envs: crate::env::EnvState,
    warnings: Warnings,
}

/// What a frame (a scope, a native call, a callback) restores when it ends,
/// on every path: the value-stack top and the innermost scope.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub(crate) top: NonNull<Slot>,
    pub(crate) innermost: u64,
}

impl Isolate {
    #[inline(always)]
    pub(crate) fn frame(&self) -> Frame {
        Frame { top: self.stack().top(), innermost: self.innermost.get() }
    }

    #[inline(always)]
    pub(crate) fn restore(&self, f: Frame) {
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

    /// Whether `x`, a live object of some heap, belongs to this isolate's.
    #[inline(always)]
    pub(crate) fn owns(&self, x: Sexp) -> bool {
        // SAFETY: the callers pass objects from `Ref`s and `Char`s, which
        // are live by the minting rule (lib.rs).
        unsafe { x.heap_id() == self.heap }
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
    pub fn call_out<R>(&mut self, f: impl FnOnce(Reentry<'_>) -> R) -> R {
        let iso = self.iso();
        debug_assert!(iso.active.get(), "robigo: a suspended capability in use");
        let _restore = Suspend { iso: self.iso_ptr(), frame: iso.frame() };
        iso.active.set(false);
        f(Reentry { iso: self.iso_ptr(), _c: PhantomData })
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
    /// pending-exit stack. H2's typed root providers walk the same list.
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
        iso.envs().for_each_root(f);
        for i in 0..iso.warnings_len() {
            let (c, m) = iso.warnings_at(i);
            f(c);
            f(m);
        }
    }

    /// The locale GNU R reads ([`Locale`]).
    #[inline(always)]
    pub fn locale(&self) -> Locale {
        self.iso().locale.get()
    }

    /// Change the locale (`Sys.setlocale`).
    pub fn set_locale(&mut self, l: Locale) {
        self.iso().locale.set(l);
    }
}

/// Restores a suspended capability's frame and resumes it.
struct Suspend {
    iso: NonNull<Isolate>,
    frame: Frame,
}

impl Drop for Suspend {
    fn drop(&mut self) {
        // SAFETY: made in `call_out` from a live capability, whose borrow
        // keeps the isolate alive until this guard is dropped.
        let iso = unsafe { self.iso.as_ref() };
        debug_assert!(!iso.active.get(), "robigo: a callback did not end before its native call returned");
        iso.restore(self.frame);
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

/// An isolate and its first capability: what an embedder (the `robigo-rt`
/// binary, a test) creates. Dropping it frees the isolate and its heap.
pub struct OwnedIsolate {
    cx: Capability<[(); 0]>,
}

impl OwnedIsolate {
    pub fn new() -> OwnedIsolate {
        OwnedIsolate::with_options(Options::default())
    }

    pub fn with_options(opts: Options) -> OwnedIsolate {
        // The heap's allocation never collects (H2), and no C segment is open
        // until a native call opens one, so no Rust frame is ever scanned
        // (docs/g3.md §3).
        let mut core = Core::new();
        // R: memory.c:R_SetMaxVSize refuses a limit below the current size
        assert!(core.set_max_vsize(opts.max_vsize), "robigo: a vector memory limit below the vector heap's size");
        let envs = crate::env::EnvState::new(&mut core);
        let iso = Box::new(Isolate {
            heap: core.id(),
            nil: core.nil(),
            na_string: core.na_string(),
            blank_string: core.blank_string(),
            unbound: core.unbound(),
            missing_arg: core.missing_arg(),
            core: UnsafeCell::new(core),
            innermost: Cell::new(0),
            scope_serial: Cell::new(0),
            active: Cell::new(true),
            hooks: Cell::new(Hooks::default()),
            locale: Cell::new(opts.locale),
            roots: UnsafeCell::new(Roots {
                stack: ValueStack::new(opts.value_stack_slots),
                globals: Globals::default(),
                exits: PendingExits::default(),
                envs,
                warnings: Warnings::default(),
            }),
        });
        let iso = NonNull::from(Box::leak(iso));
        OwnedIsolate { cx: capability(iso) }
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
        // SAFETY: made by `Box::leak` in `with_options` and freed once, here.
        // A capability cannot be moved out of its storage (`Cx` is
        // unsized), so this owner's still names its isolate, and every
        // capability of the isolate was borrowed from this owner or lived
        // inside a callback of one that was: those have ended.
        drop(unsafe { Box::from_raw(self.cx.iso.as_ptr()) });
    }
}
