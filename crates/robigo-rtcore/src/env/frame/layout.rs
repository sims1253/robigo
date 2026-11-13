//! The slot frame's word map, as the runtime reads it: `robigo_heap::slots`
//! (where the collector reads it too) and typed accessors over it. Nothing
//! else in the runtime knows an offset; a measured variant (m1-plan X1, D2)
//! swaps the heap module and these accessors.

#![allow(unsafe_code)]

pub(crate) use robigo_heap::slots::*;
use robigo_heap::{Sexp, flags};

/// Whether `x` (a live ENVSXP) is a slot frame or an overflow block.
///
/// # Safety
/// `x` is live.
#[inline(always)]
pub(crate) unsafe fn is_slots(x: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { x.has_flag(flags::SLOTS) }
}

/// The word at `off` of a slot frame, as a plain integer.
///
/// # Safety
/// `f` is a live slot frame or overflow block and `off` one of its words.
#[inline(always)]
unsafe fn word(f: Sexp, off: usize) -> *mut u64 {
    // SAFETY: the caller's contract.
    unsafe { f.0.add(off) as *mut u64 }
}

/// The number of slots in the object.
///
/// # Safety
/// `f` is a live slot frame or overflow block.
#[inline(always)]
pub(crate) unsafe fn capacity(f: Sexp) -> usize {
    // SAFETY: the caller's contract: the meta word's low half.
    unsafe { *word(f, OFF_META) as u32 as usize }
}

/// The meta word's high half: a frame's next definition stamp, an overflow
/// block's slots in use.
///
/// # Safety
/// As for [`capacity`].
#[inline(always)]
pub(crate) unsafe fn counter(f: Sexp) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { (*word(f, OFF_META) >> 32) as u32 }
}

/// # Safety
/// As for [`capacity`].
#[inline(always)]
pub(crate) unsafe fn set_counter(f: Sexp, n: u32) {
    // SAFETY: the caller's contract; the low half (capacity) is kept.
    unsafe {
        let w = word(f, OFF_META);
        *w = (*w & 0xFFFF_FFFF) | (n as u64) << 32;
    }
}

/// A frame word that holds an object (`OFF_OVERFLOW`, `OFF_SHAPE`,
/// `OFF_CALLER`).
///
/// # Safety
/// As for [`capacity`].
#[inline(always)]
pub(crate) unsafe fn get(f: Sexp, off: usize) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe { f.sexp_at(off) }
}

/// Store an object into a frame word, through the barrier and counts.
///
/// # Safety
/// As for [`capacity`]; `v` is live (or R_NilValue).
#[inline(always)]
pub(crate) unsafe fn set(f: Sexp, off: usize, v: Sexp) {
    // SAFETY: the caller's contract: a traced word of the object's body.
    unsafe { f.store(f.0.add(off) as *mut Sexp, v) }
}

/// One slot: its owner (a frame or an overflow block) and index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SlotRef {
    pub(crate) owner: Sexp,
    pub(crate) index: usize,
}

impl SlotRef {
    #[inline(always)]
    fn base(self) -> *mut u8 {
        // the address arithmetic stays inside the owner's allocation for
        // every index below its capacity (the callers' contract)
        self.owner.0.wrapping_add(OFF_SLOTS + self.index * SLOT_SIZE)
    }

    /// # Safety
    /// The owner is live and `index` below its capacity.
    #[inline(always)]
    pub(crate) unsafe fn key(self) -> u64 {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_KEY) as *const u64) }
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn kind(self) -> u64 {
        // SAFETY: the caller's contract.
        unsafe { self.key() & KIND_MASK }
    }

    /// The slot's symbol (null for a slot never keyed).
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn sym(self) -> Sexp {
        // SAFETY: the caller's contract.
        Sexp((unsafe { self.key() } & !KIND_MASK) as usize as *mut u8)
    }

    /// Set the key: `sym` and `kind`. The words the new kind traces must be
    /// valid by the next collection (the callers set them in the same
    /// interval).
    ///
    /// # Safety
    /// As for [`SlotRef::key`]; `sym` is a symbol (16-byte aligned) or null.
    #[inline(always)]
    pub(crate) unsafe fn set_key(self, sym: Sexp, kind: u64) {
        debug_assert!(sym.addr() as u64 & KIND_MASK == 0);
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_KEY) as *mut u64) = sym.addr() as u64 | kind }
    }

    /// `val` as an object.
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn val(self) -> Sexp {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_VAL) as *const Sexp) }
    }

    /// `val` as raw bits (an unboxed scalar).
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn val_bits(self) -> u64 {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_VAL) as *const u64) }
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn aux(self) -> Sexp {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_AUX) as *const Sexp) }
    }

    /// Store an object (or null) into `val`, through the owner's barrier
    /// and counts. The old word must be an object or null: a slot holding
    /// raw bits is cleared with [`SlotRef::clear_raw_val`] first.
    ///
    /// # Safety
    /// As for [`SlotRef::key`]; `v` null or live.
    #[inline(always)]
    pub(crate) unsafe fn store_val(self, v: Sexp) {
        // SAFETY: the caller's contract: a traced word of the owner's body.
        unsafe { self.owner.store(self.base().add(SLOT_VAL) as *mut Sexp, v) }
    }

    /// As [`SlotRef::store_val`] for `aux`.
    ///
    /// # Safety
    /// As for [`SlotRef::store_val`].
    #[inline(always)]
    pub(crate) unsafe fn store_aux(self, v: Sexp) {
        // SAFETY: as in `store_val`.
        unsafe { self.owner.store(self.base().add(SLOT_AUX) as *mut Sexp, v) }
    }

    /// Write raw bits into `val` (an unboxed scalar; the slot's kind says
    /// so). The old word must hold no object (or one whose count the caller
    /// has already dropped).
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn set_val_bits(self, bits: u64) {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_VAL) as *mut u64) = bits }
    }

    /// Zero `val` without touching any count (it held raw bits).
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn clear_raw_val(self) {
        // SAFETY: the caller's contract.
        unsafe { self.set_val_bits(0) }
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn meta(self) -> u64 {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_META) as *const u64) }
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn set_meta(self, m: u64) {
        // SAFETY: the caller's contract.
        unsafe { *(self.base().add(SLOT_META) as *mut u64) = m }
    }

    /// The definition stamp.
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn stamp(self) -> u32 {
        // SAFETY: the caller's contract.
        unsafe { self.meta() as u32 }
    }

    /// GNU R's `MISSING` of the binding.
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn missing(self) -> u16 {
        // SAFETY: the caller's contract.
        ((unsafe { self.meta() } >> META_MISSING_SHIFT) & 3) as u16
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn set_missing(self, m: u16) {
        // SAFETY: the caller's contract.
        unsafe {
            let w = self.meta() & !(3 << META_MISSING_SHIFT);
            self.set_meta(w | ((m as u64 & 3) << META_MISSING_SHIFT));
        }
    }

    /// GNU R's `PRSEEN` of a delayed slot.
    ///
    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn prseen(self) -> u16 {
        // SAFETY: the caller's contract.
        ((unsafe { self.meta() } >> META_PRSEEN_SHIFT) & 3) as u16
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn has(self, flag: u64) -> bool {
        // SAFETY: the caller's contract.
        unsafe { self.meta() & flag != 0 }
    }

    /// # Safety
    /// As for [`SlotRef::key`].
    #[inline(always)]
    pub(crate) unsafe fn set_flag(self, flag: u64, on: bool) {
        // SAFETY: the caller's contract.
        unsafe {
            let m = self.meta();
            self.set_meta(if on { m | flag } else { m & !flag });
        }
    }
}
