//! The value stack (docs/g3.md §2.3, §9.2): one per isolate, in a reserved
//! address range with a guard page after it, so a slot's address never
//! changes; slots tagged object, real, int, logical or empty.
//!
//! D3's variant (branch g3/x1-d3-split): **split tags and payloads**. The
//! payloads are one dense array of 8-byte words and the tags another of
//! bytes (9 bytes a slot, against the default's 16). A slot's handle
//! ([`SlotPtr`]) is its payload word's address; its tag's address follows
//! from it alone: the payload array starts at an address aligned to
//! [`SPAN`], and the tag array ends where it starts, so the tag of the
//! payload at `a` is at `(a & !(SPAN - 1)) - SPAN/8 + ((a & (SPAN - 1)) >> 3)`.
//! The VM keeps an activation's tag base beside its payload base.
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
#[derive(Clone, Copy, Debug)]
#[repr(C, align(16))]
pub struct Slot {
    tag: u64,
    /// a pointer, so that an object slot keeps its object's provenance;
    /// unboxed values are stored as addresses without one
    word: *mut u8,
}

/// Two empty slots are equal whatever their payload words hold: the split
/// layout zeroes only an empty slot's tag (D3's variant).
impl PartialEq for Slot {
    #[inline(always)]
    fn eq(&self, o: &Slot) -> bool {
        self.tag == o.tag && (self.tag == Tag::Empty as u64 || self.word == o.word)
    }
}

impl Eq for Slot {}

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

/// The payload array's alignment, and its largest size in bytes: a stack
/// has at most `SPAN / 8` slots.
pub const SPAN: usize = 1 << 27;

/// A slot's handle: the address of its payload word (D3's split layout).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct SlotPtr(NonNull<u64>);

impl std::fmt::Pointer for SlotPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Pointer::fmt(&self.0, f)
    }
}

impl SlotPtr {
    /// The handle of the payload word at `p`.
    ///
    /// # Safety
    /// `p` is a payload word of a value stack.
    #[inline(always)]
    pub unsafe fn from_word(p: NonNull<u64>) -> SlotPtr {
        SlotPtr(p)
    }

    /// The payload word.
    #[inline(always)]
    pub fn as_ptr(self) -> *mut u64 {
        self.0.as_ptr()
    }

    /// The tag byte of this slot.
    #[inline(always)]
    pub fn tag_ptr(self) -> *mut u8 {
        // the tag array and the payload array are parts of one mapping, so
        // the address keeps the mapping's provenance
        self.0.as_ptr().cast::<u8>().map_addr(|a| (a & !(SPAN - 1)) - SPAN / 8 + ((a & (SPAN - 1)) >> 3))
    }

    /// The slot `n` above this one.
    ///
    /// # Safety
    /// That slot lies in the same stack's reservation (or one past its end).
    #[inline(always)]
    pub unsafe fn add(self, n: usize) -> SlotPtr {
        // SAFETY: the caller's contract.
        SlotPtr(unsafe { self.0.add(n) })
    }

    /// Slots from `o` up to this one.
    ///
    /// # Safety
    /// Both are slots of one stack.
    #[inline(always)]
    pub unsafe fn offset_from(self, o: SlotPtr) -> isize {
        // SAFETY: the caller's contract.
        unsafe { self.0.offset_from(o.0) }
    }

    /// Read the slot whose tag is at `tag` (its [`SlotPtr::tag_ptr`], which
    /// the VM keeps per activation).
    ///
    /// # Safety
    /// As for [`SlotPtr::read`]; `tag` is this slot's tag.
    #[inline(always)]
    pub unsafe fn read_at(self, tag: *const u8) -> Slot {
        // SAFETY: the caller's contract.
        unsafe { Slot { tag: *tag as u64, word: self.0.as_ptr().cast::<*mut u8>().read() } }
    }

    /// Write the slot whose tag is at `tag`.
    ///
    /// # Safety
    /// As for [`SlotPtr::write`]; `tag` is this slot's tag.
    #[inline(always)]
    pub unsafe fn write_at(self, tag: *mut u8, s: Slot) {
        // SAFETY: the caller's contract.
        unsafe {
            *tag = s.tag as u8;
            self.0.as_ptr().cast::<*mut u8>().write(s.word);
        }
    }

    /// Read the slot.
    ///
    /// # Safety
    /// A slot of a value stack (mapped, initialized: zeroed, then written
    /// whole).
    #[inline(always)]
    pub unsafe fn read(self) -> Slot {
        // SAFETY: the caller's contract.
        unsafe { Slot { tag: *self.tag_ptr() as u64, word: self.0.as_ptr().cast::<*mut u8>().read() } }
    }

    /// Write the slot.
    ///
    /// # Safety
    /// A slot of a value stack; an object in `s` is a live object of its
    /// isolate's heap.
    #[inline(always)]
    pub unsafe fn write(self, s: Slot) {
        // SAFETY: the caller's contract.
        unsafe {
            *self.tag_ptr() = s.tag as u8;
            self.0.as_ptr().cast::<*mut u8>().write(s.word);
        }
    }
}

/// The reserved memory of one value stack: `SPAN / 8` bytes of tags
/// ending where the `SPAN`-aligned payload array starts, the payloads, and
/// the rest of the span as a guard.
struct Reservation {
    /// the first payload word
    base: NonNull<u64>,
    /// bytes of payloads in use
    slot_bytes: usize,
    map: NonNull<u8>,
    map_bytes: usize,
}

impl Reservation {
    #[cfg(not(miri))]
    fn new(slots: usize) -> Reservation {
        let page = crate::sys::page_size();
        let slot_bytes = (slots.max(1) * 8).next_multiple_of(page);
        assert!(slot_bytes <= SPAN, "robigo: a value stack of more than {} slots", SPAN / 8);
        let tag_bytes = (slot_bytes / 8).next_multiple_of(page);
        // room for an aligned span with its tags before it
        let map_bytes = 3 * SPAN;
        let map = crate::sys::reserve_none(map_bytes);
        let lo = map.as_ptr() as usize + SPAN / 8;
        let start = lo.next_multiple_of(SPAN);
        // SAFETY: `start - SPAN/8 .. start + SPAN` lies in the mapping: `start`
        // is at most SPAN past `lo`, which is SPAN/8 past its start.
        let base = unsafe { map.add(start - map.as_ptr() as usize) };
        // SAFETY: as above.
        unsafe {
            crate::sys::allow(base.sub(SPAN / 8), tag_bytes);
            crate::sys::allow(base, slot_bytes);
        }
        Reservation { base: base.cast(), slot_bytes, map, map_bytes }
    }

    /// Miri cannot map memory, and the split layout needs a SPAN-aligned
    /// span: not supported on this branch.
    #[cfg(miri)]
    fn new(_slots: usize) -> Reservation {
        unimplemented!("robigo: the D3 split value stack under Miri")
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        crate::sys::release(self.map, self.map_bytes);
    }
}

/// One isolate's value stack. `base` and `limit` never change; `top` is the
/// next free slot. Only this module reads or writes slots.
pub(crate) struct ValueStack {
    base: SlotPtr,
    /// one past the last slot (the guard page starts here)
    limit: SlotPtr,
    top: Cell<SlotPtr>,
    _mem: Reservation,
}

impl ValueStack {
    pub(crate) fn new(slots: usize) -> ValueStack {
        let mem = Reservation::new(slots);
        let base = SlotPtr(mem.base);
        let n = mem.slot_bytes / 8;
        // SAFETY: `n` slots fit in the reservation, so one past the last is
        // in bounds or one past the end of it.
        let limit = unsafe { base.add(n) };
        ValueStack { base, limit, top: Cell::new(base), _mem: mem }
    }

    #[inline(always)]
    pub(crate) fn top(&self) -> SlotPtr {
        self.top.get()
    }

    /// Restore the top to `t`, which an earlier [`ValueStack::top`] of this
    /// stack returned: every slot from `t` up dies.
    #[inline(always)]
    pub(crate) fn restore(&self, t: SlotPtr) {
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
    pub(crate) fn owns(&self, p: SlotPtr) -> bool {
        let off = (p.as_ptr() as usize).wrapping_sub(self.base.as_ptr() as usize);
        off < self.limit.as_ptr() as usize - self.base.as_ptr() as usize && off.is_multiple_of(8)
    }

    /// Push `s`, returning its slot. Panics on overflow, the last resort:
    /// R code reaches GNU R's stack-depth errors first, at the R entries
    /// that check [`ValueStack::headroom`] (C2, V1).
    #[inline(always)]
    pub(crate) fn push(&self, s: Slot) -> SlotPtr {
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

    /// Push `n` empty slots, returning the first: a VM activation's
    /// registers. Panics on overflow, as [`ValueStack::push`] does.
    #[inline(always)]
    pub(crate) fn push_empty(&self, n: usize) -> SlotPtr {
        let t = self.top.get();
        if self.headroom() < n {
            overflow();
        }
        // SAFETY: `n` slots from `t` lie below `limit` (checked), and no
        // reference to slot memory exists. Only the tag word needs writing:
        // an empty slot's payload is never read.
        unsafe {
            // empty: the tags zeroed (a payload of an empty slot is never read)
            std::ptr::write_bytes(t.tag_ptr(), 0, n);
            self.top.set(t.add(n));
        }
        t
    }

    /// Read slot `p`.
    ///
    /// # Safety
    /// `p` is a slot of this stack ([`ValueStack::owns`]).
    #[inline(always)]
    pub(crate) unsafe fn read(&self, p: SlotPtr) -> Slot {
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
    pub(crate) unsafe fn write(&self, p: SlotPtr, s: Slot) {
        // SAFETY: the caller's contract.
        unsafe { p.write(s) }
    }

    /// Every object in the live slots `from..to` (indices from the base).
    /// Panics unless `from <= to <= depth`.
    pub(crate) fn for_each_object_in(&self, from: usize, to: usize, f: &mut dyn FnMut(Sexp)) {
        assert!(from <= to && to <= self.depth(), "robigo: slots {from}..{to} of a stack of {}", self.depth());
        for i in from..to {
            // SAFETY: `i` is below the depth: a live slot of this stack.
            if let Some(x) = unsafe { self.base.add(i).read() }.as_object() {
                f(x);
            }
        }
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
                if *p.tag_ptr() == Tag::Object as u8 {
                    v.edge(&mut *p.as_ptr().cast::<Sexp>());
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
