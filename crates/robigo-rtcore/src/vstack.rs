//! The value stack (docs/g3.md §2.3, §9.2): one per isolate, in a reserved
//! address range with a guard page after it, so a slot's address never
//! changes; 16-byte slots tagged object, real, int, logical or empty (D3's
//! default layout; X1 measures split tags and payloads behind this module).
//!
//! Roots, `Out` slots and `Args` windows are slots of this stack, never
//! `R_PPStack`: C may `UNPROTECT` across a callback, and unwinding restores
//! `R_PPStackTop`. Value-stack slots do not count towards `REFCNT` (§4).
//! Only slots below the top are live, and the collector traces exactly the
//! live object slots ([`crate::Cx::for_each_root`]).

#![allow(unsafe_code)]

use robigo_heap::{Sexp, Trace, Visitor};
use std::cell::Cell;
use std::ptr::NonNull;

/// A slot's tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Tag {
    /// No value. Fresh (zeroed) memory reads as empty.
    Empty = 0,
    /// The payload is a `Sexp`, traced.
    Object = 1,
    /// An unboxed double.
    Real = 2,
    /// An unboxed integer (low 32 bits).
    Int = 3,
    /// An unboxed logical (low 32 bits, GNU R's int encoding).
    Logical = 4,
}

/// One value-stack slot: a tag word and a payload word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct Slot {
    tag: u64,
    /// a pointer, so that an object slot keeps its object's provenance;
    /// unboxed values are stored as addresses without one
    word: *mut u8,
}

impl Slot {
    pub const EMPTY: Slot = Slot { tag: Tag::Empty as u64, word: std::ptr::null_mut() };

    #[inline(always)]
    pub fn object(x: Sexp) -> Slot {
        Slot { tag: Tag::Object as u64, word: x.0 }
    }
    #[inline(always)]
    pub fn real(v: f64) -> Slot {
        Slot { tag: Tag::Real as u64, word: std::ptr::without_provenance_mut(v.to_bits() as usize) }
    }
    #[inline(always)]
    pub fn int(v: i32) -> Slot {
        Slot { tag: Tag::Int as u64, word: std::ptr::without_provenance_mut(v as u32 as usize) }
    }
    #[inline(always)]
    pub fn logical(v: i32) -> Slot {
        Slot { tag: Tag::Logical as u64, word: std::ptr::without_provenance_mut(v as u32 as usize) }
    }
    #[inline(always)]
    pub fn tag(self) -> Tag {
        match self.tag {
            0 => Tag::Empty,
            1 => Tag::Object,
            2 => Tag::Real,
            3 => Tag::Int,
            4 => Tag::Logical,
            t => unreachable!("robigo: value-stack slot with tag {t}"),
        }
    }
    /// The unboxed double, if the slot holds one.
    #[inline(always)]
    pub fn as_real(self) -> Option<f64> {
        (self.tag == Tag::Real as u64).then(|| f64::from_bits(self.word.addr() as u64))
    }
    /// The unboxed integer or logical, if the slot holds one.
    #[inline(always)]
    pub fn as_int(self) -> Option<i32> {
        matches!(self.tag, 3 | 4).then(|| self.word.addr() as u32 as i32)
    }
    /// The object, if the slot holds one.
    #[inline(always)]
    pub fn as_object(self) -> Option<Sexp> {
        (self.tag == Tag::Object as u64).then_some(Sexp(self.word))
    }
}

/// The reserved memory of one value stack.
struct Reservation {
    base: NonNull<Slot>,
    /// bytes of slots, then (outside Miri) one guard page
    slot_bytes: usize,
    #[cfg(not(miri))]
    map_bytes: usize,
}

impl Reservation {
    #[cfg(not(miri))]
    fn new(slots: usize) -> Reservation {
        let page = crate::sys::page_size();
        let slot_bytes = (slots.max(1) * size_of::<Slot>()).next_multiple_of(page);
        let map_bytes = slot_bytes + page;
        let base = crate::sys::reserve(map_bytes, slot_bytes);
        Reservation { base: base.cast(), slot_bytes, map_bytes }
    }

    /// Miri cannot map memory: the slots come from the allocator, zeroed,
    /// and there is no guard page.
    #[cfg(miri)]
    fn new(slots: usize) -> Reservation {
        let slot_bytes = slots.max(1) * size_of::<Slot>();
        let layout = std::alloc::Layout::from_size_align(slot_bytes, align_of::<Slot>()).unwrap();
        // SAFETY: the layout is not zero-sized (one slot at least).
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        let base = NonNull::new(p).unwrap_or_else(|| std::alloc::handle_alloc_error(layout));
        Reservation { base: base.cast(), slot_bytes }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        #[cfg(not(miri))]
        crate::sys::release(self.base.cast(), self.map_bytes);
        #[cfg(miri)]
        // SAFETY: allocated in `new` with this layout, and freed once.
        unsafe {
            std::alloc::dealloc(
                self.base.as_ptr().cast(),
                std::alloc::Layout::from_size_align(self.slot_bytes, align_of::<Slot>()).unwrap(),
            )
        }
    }
}

/// One isolate's value stack. `base` and `limit` never change; `top` is the
/// next free slot. Only this module reads or writes slots.
pub(crate) struct ValueStack {
    base: NonNull<Slot>,
    /// one past the last slot (the guard page starts here)
    limit: NonNull<Slot>,
    top: Cell<NonNull<Slot>>,
    _mem: Reservation,
}

impl ValueStack {
    pub(crate) fn new(slots: usize) -> ValueStack {
        let mem = Reservation::new(slots);
        let base = mem.base;
        let n = mem.slot_bytes / size_of::<Slot>();
        // SAFETY: `n` slots fit in the reservation, so one past the last is
        // in bounds or one past the end of it.
        let limit = unsafe { base.add(n) };
        ValueStack { base, limit, top: Cell::new(base), _mem: mem }
    }

    #[inline(always)]
    pub(crate) fn top(&self) -> NonNull<Slot> {
        self.top.get()
    }

    /// Restore the top to `t`, which an earlier [`ValueStack::top`] of this
    /// stack returned: every slot from `t` up dies.
    #[inline(always)]
    pub(crate) fn restore(&self, t: NonNull<Slot>) {
        debug_assert!(self.base <= t && t <= self.limit);
        self.top.set(t);
    }

    /// How many slots are live.
    pub(crate) fn depth(&self) -> usize {
        // SAFETY: both are slots of this reservation, `top` not below `base`.
        unsafe { self.top.get().offset_from(self.base) as usize }
    }

    /// How many more slots fit.
    pub(crate) fn headroom(&self) -> usize {
        // SAFETY: as in `depth`; `top` is never above `limit`.
        unsafe { self.limit.offset_from(self.top.get()) as usize }
    }

    pub(crate) fn bounds(&self) -> (usize, usize) {
        (self.base.as_ptr() as usize, self.limit.as_ptr() as usize)
    }

    /// Whether `p` is a slot of this stack, live or not: the owner check of
    /// every root, `Out` and `Args` (docs/g3.md §2.3). Slots of one stack
    /// are never another's, because each stack has its own reservation.
    #[inline(always)]
    pub(crate) fn owns(&self, p: NonNull<Slot>) -> bool {
        let off = (p.as_ptr() as usize).wrapping_sub(self.base.as_ptr() as usize);
        off < self.limit.as_ptr() as usize - self.base.as_ptr() as usize && off.is_multiple_of(size_of::<Slot>())
    }

    /// Push `s`, returning its slot. Panics on overflow, the last resort:
    /// R code reaches GNU R's stack-depth errors first, at the R entries
    /// that check [`ValueStack::headroom`] (C2, V1).
    #[inline(always)]
    pub(crate) fn push(&self, s: Slot) -> NonNull<Slot> {
        let t = self.top.get();
        if t == self.limit {
            overflow();
        }
        // SAFETY: `t` is below `limit`, so a slot of the reservation, and no
        // reference to slot memory exists (this module writes it through
        // raw pointers only).
        unsafe {
            t.write(s);
            self.top.set(t.add(1));
        }
        t
    }

    /// Read slot `p`.
    ///
    /// # Safety
    /// `p` is a slot of this stack ([`ValueStack::owns`]).
    #[inline(always)]
    pub(crate) unsafe fn read(&self, p: NonNull<Slot>) -> Slot {
        // SAFETY: the caller's contract: a slot of the reservation, which
        // is always mapped and initialized (zeroed, then written whole).
        unsafe { p.read() }
    }

    /// Write slot `p`.
    ///
    /// # Safety
    /// `p` is a slot of this stack, and if `s` is an object it is a live
    /// object of this isolate's heap.
    #[inline(always)]
    pub(crate) unsafe fn write(&self, p: NonNull<Slot>, s: Slot) {
        // SAFETY: the caller's contract.
        unsafe { p.write(s) }
    }

    /// Every object in a live slot.
    pub(crate) fn for_each_object(&self, f: &mut dyn FnMut(Sexp)) {
        let mut p = self.base;
        let top = self.top.get();
        while p < top {
            // SAFETY: `p` is a live slot of this stack.
            if let Some(x) = unsafe { p.read() }.as_object() {
                f(x);
            }
            // SAFETY: `p` is below `top`, so one past it is in bounds.
            p = unsafe { p.add(1) };
        }
    }
}

/// A collection marks the live object slots (tag-checked), and a moving one
/// rewrites their payloads through the same edges.
impl Trace for ValueStack {
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        let mut p = self.base;
        let top = self.top.get();
        while p < top {
            // SAFETY: `p` is a live slot of this stack. `&mut self` (only a
            // collection has it) means no other code reads or writes slots
            // meanwhile, and the payload of an object slot is a pointer word,
            // which `Sexp` (a transparent pointer) may stand for.
            unsafe {
                let slot = p.as_ptr();
                if (*slot).tag == Tag::Object as u64 {
                    v.edge(&mut *(&raw mut (*slot).word).cast::<Sexp>());
                }
                p = p.add(1);
            }
        }
    }
}

#[cold]
#[inline(never)]
fn overflow() -> ! {
    panic!("robigo: the value stack is full")
}
