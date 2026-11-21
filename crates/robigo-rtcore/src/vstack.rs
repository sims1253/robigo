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

/// A slot's tag. [`Slot::mark`]'s tag word lies past the last variant
/// (a compile-time assert checks it against `Logical`: a new variant moves
/// the assert).
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

/// One value-stack slot: a tag word and a payload word. Only this module
/// makes slots (the fields are private), so a slot's tag word is one of
/// [`Tag`]'s, [`Slot::mark`]'s, or (the `poison-dead` feature) a poison's,
/// whose payload is a `&'static Location` with its low bit set: a new kind
/// of slot takes a tag word past the poison's.
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

    /// A slot holding the object `x`, which every collection traces and the
    /// slot's readers dereference.
    ///
    /// # Safety
    /// `x` is null or a live object of the isolate whose stack the slot is
    /// stored in.
    #[inline(always)]
    pub unsafe fn object(x: Sexp) -> Slot {
        Slot { tag: Tag::Object as u64, word: x.as_ptr() }
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
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub fn tag(self) -> Tag {
        match self.tag {
            0 => Tag::Empty,
            1 => Tag::Object,
            2 => Tag::Real,
            3 => Tag::Int,
            4 => Tag::Logical,
            t => {
                self.check_live();
                unreachable!("robigo: value-stack slot with tag {t}")
            }
        }
    }
    /// The unboxed double, if the slot holds one.
    #[inline(always)]
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub fn as_real(self) -> Option<f64> {
        self.check_live();
        (self.tag == Tag::Real as u64).then(|| f64::from_bits(self.word.addr() as u64))
    }
    /// The unboxed integer or logical, if the slot holds one.
    #[inline(always)]
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub fn as_int(self) -> Option<i32> {
        self.check_live();
        matches!(self.tag, 3 | 4).then(|| self.word.addr() as u32 as i32)
    }
    /// The object, if the slot holds one.
    #[inline(always)]
    pub fn as_object(self) -> Option<Sexp> {
        // SAFETY: (taken only for an object slot) an object slot holds an
        // object's word ([`Slot::object`]'s contract).
        (self.tag == Tag::Object as u64).then(|| unsafe { Sexp::from_ptr(self.word) })
    }
    /// A mark the evaluator leaves on its own slots: a word that is neither
    /// an object (nothing traces it) nor a value ([`Slot::tag`] names no
    /// such slot, and no register or root holds one). robigo-rt's VM heads
    /// each activation's registers with marks (its link walk reads them).
    #[inline(always)]
    pub fn mark(w: u64) -> Slot {
        Slot { tag: MARK, word: std::ptr::without_provenance_mut(w as usize) }
    }
    /// The mark's word, if the slot holds one ([`Slot::mark`]).
    #[inline(always)]
    pub fn as_mark(self) -> Option<u64> {
        (self.tag == MARK).then(|| self.word.addr() as u64)
    }

    /// What a dead slot holds: register `reg` of an activation, read for
    /// the last time by instruction `pc` (`u32::MAX`: no instruction), D9.
    /// Empty, but with the `poison-dead` feature and `ROBIGO_POISON_DEAD=1`
    /// a poison that names the slot, the pc and the caller's source line,
    /// which [`Slot::check_live`] panics on. Like a mark, nothing traces
    /// it and no reader takes it for a value.
    #[inline(always)]
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub fn dead(reg: u16, pc: u32) -> Slot {
        #[cfg(feature = "poison-dead")]
        if poison::on() {
            return poison::slot(u64::from(reg), pc, std::panic::Location::caller());
        }
        let _ = (reg, pc);
        Slot::EMPTY
    }

    /// Panic if the slot is a poison ([`Slot::dead`]): a read of a slot
    /// after its last use. Nothing without the `poison-dead` feature.
    #[inline(always)]
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub fn check_live(self) {
        #[cfg(feature = "poison-dead")]
        if poison::is(self.tag, self.word) {
            poison::read(self.tag, self.word)
        }
    }
}

/// Whether dead slots are poisoned (`ROBIGO_POISON_DEAD=1` in a build with
/// the `poison-dead` feature; always false without it): evaluators that keep
/// registers of their own apply D9's deaths to them only then.
#[inline(always)]
pub fn poison_dead() -> bool {
    #[cfg(feature = "poison-dead")]
    let on = poison::on();
    #[cfg(not(feature = "poison-dead"))]
    let on = false;
    on
}

/// [`Slot::dead`]'s poison (`poison-dead` feature): tag word
/// `TAG | reg << 8 | pc << 32`, payload the `Location` of the source line
/// that killed the slot.
#[cfg(feature = "poison-dead")]
mod poison {
    use std::panic::Location;
    use std::sync::LazyLock;

    pub(super) const TAG: u64 = super::MARK + 1;
    const _: () = assert!(TAG < 0x100, "robigo: a poison's tag word must fit the low byte `check_live` masks");
    /// [`Slot::dead`]'s `reg` for a slot a scope's restore freed (its
    /// index from the stack's base is in the pc field).
    pub(super) const RESTORED: u64 = 0xffffff;

    /// `ROBIGO_POISON_DEAD`: "1" on, unset, empty or "0" off, anything else
    /// refused (as robigo-rt's other `ROBIGO_*` switches are).
    static ON: LazyLock<bool> = LazyLock::new(|| match std::env::var("ROBIGO_POISON_DEAD").as_deref() {
        Ok("1") => true,
        Ok("0") | Ok("") | Err(_) => false,
        Ok(other) => panic!("robigo: ROBIGO_POISON_DEAD={other:?} is neither \"0\" nor \"1\""),
    });

    /// The low bit of a poison's payload: a `Location` is aligned past it,
    /// so a poison names itself twice (its tag byte and this bit) before
    /// [`read`] takes the payload for a `Location`.
    pub(super) const PAYLOAD: usize = 1;
    const _: () = assert!(align_of::<Location<'static>>() > PAYLOAD, "robigo: a Location must leave its low bit free");

    /// Whether the slot words `tag` and `word` are a poison [`slot`] made.
    #[inline(always)]
    pub(super) fn is(tag: u64, word: *mut u8) -> bool {
        tag & 0xff == TAG && word.addr() & PAYLOAD != 0
    }

    #[inline(always)]
    pub(super) fn on() -> bool {
        *ON
    }

    pub(super) fn slot(reg: u64, pc: u32, at: &'static Location<'static>) -> super::Slot {
        super::Slot {
            tag: TAG | (reg & 0xffffff) << 8 | u64::from(pc) << 32,
            word: std::ptr::from_ref(at).cast_mut().cast::<u8>().map_addr(|a| a | PAYLOAD),
        }
    }

    #[cold]
    #[inline(never)]
    #[track_caller]
    pub(super) fn read(tag: u64, word: *mut u8) -> ! {
        // SAFETY: a poison (`is`: only `slot` writes its tag byte, and with
        // this bit set) holds the `&'static Location` `slot` stored, its
        // low bit set.
        let at: &Location<'static> = unsafe { &*word.map_addr(|a| a & !PAYLOAD).cast::<Location<'static>>() };
        let (reg, pc) = ((tag >> 8) & 0xffffff, (tag >> 32) as u32);
        let what = if reg == RESTORED {
            format!("value-stack slot {pc}, freed by the end of the frame opened")
        } else if pc == u32::MAX {
            format!("register {reg}, cleared")
        } else {
            format!("register {reg}, cleared by instruction {pc}")
        };
        panic!("robigo: ROBIGO_POISON_DEAD: a read of a dead slot: {what} at {at}")
    }
}

/// [`Slot::mark`]'s tag word: past [`Tag`]'s.
const MARK: u64 = 5;
const _: () = assert!(MARK > Tag::Logical as u64, "robigo: a mark's tag word must be past every Tag's");

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
/// `log2(size_of::<Slot>())`, for [`ValueStack::owns`].
const SLOT_SHIFT: u32 = size_of::<Slot>().trailing_zeros();
const _: () = assert!(size_of::<Slot>().is_power_of_two() && size_of::<Slot>() == 1 << SLOT_SHIFT);

pub(crate) struct ValueStack {
    base: NonNull<Slot>,
    /// one past the last slot (the guard page starts here)
    limit: NonNull<Slot>,
    /// `limit - base` in slots, for [`ValueStack::owns`]
    nslots: usize,
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
        ValueStack { base, limit, nslots: n, top: Cell::new(base), _mem: mem }
    }

    #[inline(always)]
    pub(crate) fn top(&self) -> NonNull<Slot> {
        self.top.get()
    }

    /// Restore the top to `t`, which an earlier [`ValueStack::top`] of this
    /// stack returned: every slot from `t` up dies.
    #[inline(always)]
    #[cfg(not(feature = "poison-dead"))]
    pub(crate) fn restore(&self, t: NonNull<Slot>) {
        debug_assert!(self.base <= t && t <= self.limit);
        self.top.set(t);
    }

    /// [`ValueStack::restore`] (`poison-dead` feature): the slots freed are
    /// poisons naming `at`, the line that opened the frame ending here.
    #[cfg(feature = "poison-dead")]
    pub(crate) fn restore_at(&self, t: NonNull<Slot>, at: &'static std::panic::Location<'static>) {
        debug_assert!(self.base <= t && t <= self.limit);
        if poison::on() {
            let mut p = t;
            while p < self.top.get() {
                // SAFETY: a slot of the reservation (from `t` to the top);
                // its index from the base fits the poison's pc field for
                // any stack this module maps.
                unsafe {
                    let i = p.offset_from(self.base) as u32;
                    p.write(poison::slot(poison::RESTORED, i, at));
                    p = p.add(1);
                }
            }
        }
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
    ///
    /// One compare: the byte offset from `base`, rotated right by the slot
    /// size's bits, is the slot index for an aligned offset and has its
    /// high bits set for a misaligned one (or one below `base`, which
    /// wraps), so it is below `nslots` exactly when `p` is in range and
    /// aligned, as `off < limit - base && off % size_of::<Slot>() == 0`.
    #[inline(always)]
    pub(crate) fn owns(&self, p: NonNull<Slot>) -> bool {
        let off = (p.as_ptr() as usize).wrapping_sub(self.base.as_ptr() as usize);
        off.rotate_right(SLOT_SHIFT) < self.nslots
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

    /// Push `n` empty slots, returning the first: a VM activation's
    /// registers. Panics on overflow, as [`ValueStack::push`] does.
    #[inline(always)]
    pub(crate) fn push_empty(&self, n: usize) -> NonNull<Slot> {
        let t = self.top.get();
        if self.headroom() < n {
            overflow();
        }
        // SAFETY: `n` slots from `t` lie below `limit` (checked), and no
        // reference to slot memory exists. Only the tag word needs writing:
        // an empty slot's payload is never read.
        unsafe {
            for i in 0..n {
                t.add(i).write(Slot::EMPTY);
            }
            self.top.set(t.add(n));
        }
        t
    }

    /// Read slot `p`.
    ///
    /// # Safety
    /// `p` is a slot of this stack ([`ValueStack::owns`]).
    #[inline(always)]
    #[cfg_attr(feature = "poison-dead", track_caller)]
    pub(crate) unsafe fn read(&self, p: NonNull<Slot>) -> Slot {
        // SAFETY: the caller's contract: a slot of the reservation, which
        // is always mapped and initialized (zeroed, then written whole).
        let s = unsafe { p.read() };
        s.check_live();
        s
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

    /// The live slots `from..to` (indices from the base), read in place.
    /// Panics unless `from <= to <= depth`.
    ///
    /// # Safety
    /// No slot of the range is written while the slice lives.
    pub(crate) unsafe fn slots(&self, from: usize, to: usize) -> &[Slot] {
        assert!(from <= to && to <= self.depth(), "robigo: slots {from}..{to} of a stack of {}", self.depth());
        // SAFETY: `from..to` lies below the depth: live, initialized slots
        // of this reservation; the caller's contract keeps them unwritten
        // while the slice lives.
        unsafe { std::slice::from_raw_parts(self.base.add(from).as_ptr(), to - from) }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// [`ValueStack::owns`]' one compare is the range-and-alignment test it
    /// replaces, at every edge.
    #[test]
    fn owns_is_the_aligned_range() {
        let st = ValueStack::new(64);
        let b = st.base.as_ptr() as usize;
        let n = st.nslots;
        let sz = size_of::<Slot>();
        // (compared, never read: no provenance needed)
        let p = |a: usize| NonNull::new(std::ptr::without_provenance_mut::<Slot>(a)).unwrap();
        let old = |a: usize| {
            let off = a.wrapping_sub(b);
            off < n * sz && off.is_multiple_of(sz)
        };
        let mut cases =
            vec![b, b + sz, b + (n - 1) * sz, b + n * sz, b + (n + 1) * sz, b - sz, b - 1, usize::MAX & !15];
        for k in [0, 1, n - 1] {
            for d in 1..sz {
                cases.push(b + k * sz + d);
            }
        }
        for a in cases {
            assert_eq!(st.owns(p(a)), old(a), "address {a:#x} (base {b:#x}, {n} slots)");
        }
        assert!(st.owns(p(b)) && st.owns(p(b + (n - 1) * sz)) && !st.owns(p(b + n * sz)));
    }
}
