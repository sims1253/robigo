//! The isolate's context-record stack (X9-1c): where the records the Rust
//! drivers push live ([`crate::Cx`]'s `with_record`), so that an R call
//! keeps none on the C stack. GNU R keeps a byte-code call's `RCNTXT` off
//! the C stack too (`BCFRAME_CNTXT` on `R_BCNodeStack`, eval.c:7302).
//!
//! A chunked arena of *slots*, used strictly last in, first out: a slot is
//! taken when a record is pushed (or reserved for the record its holder
//! will push, [`Reserved`]) and released when the record is popped, on
//! every path: a return, an `Err` (an exit) passing, a panic unwinding
//! through the guard's `Drop`. Chunks of [`CHUNK_SLOTS`] slots are
//! allocated when the stack first gets that deep and kept (the high-water
//! mark, which the C stack limit and `expressions` bound) until the
//! isolate is dropped. A chunk never moves, so a record keeps its address
//! for its life, as C and the chain need.
//!
//! Each slot has a fill area of [`FILL_SIZE`] bytes beside its record, for
//! the record's owner: robigo-rt keeps a call's lazy-`promargs` fill data
//! there, which the record points at (`filldata`) and its filler reads
//! while the record lives. This crate never reads the area; the owner
//! says what type it holds ([`Reserved::fill`], checked to fit).
//!
//! Records are traced through the chain (`nextcontext` from
//! `R_GlobalContext`, and from the records `Cx::reset_to_session` cut off
//! it), never through this storage: a slot that holds no such record holds
//! nothing a collection must see. Nothing relates
//! a record's address to the C stack (GNU R's `R_CStackStart` arithmetic
//! compares locals only; the X9-1a ledger entry lists the grep).
//!
//! Under `debug_assertions` a released slot is filled with [`POISON`], so
//! that a stale pointer to a popped record reads an unmistakable pattern
//! (`nextcontext` 0xdddd…, a non-canonical address) instead of a plausible
//! old record, and the chain walks assert they never meet one.

#![allow(unsafe_code)]

use crate::context::Context;
use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr::{NonNull, addr_of, addr_of_mut, null_mut};

/// The bytes beside each record for its owner (robigo-rt's `FillData`, 392
/// bytes on x86-64, asserts it fits).
pub const FILL_SIZE: usize = 400;
/// The fill area's alignment.
pub const FILL_ALIGN: usize = 8;

/// Slots per chunk: 64 × 936 bytes.
pub(crate) const CHUNK_SLOTS: usize = 64;

/// The byte a released slot is filled with under `debug_assertions`.
pub(crate) const POISON: u8 = 0xdd;

#[repr(C, align(8))]
struct FillArea([MaybeUninit<u8>; FILL_SIZE]);

const _: () = assert!(align_of::<FillArea>() == FILL_ALIGN);

/// A place for one record: the record (at offset 0, so a record's address
/// is its slot's), its owner's fill area, and the slot after it.
#[repr(C)]
pub(crate) struct Slot {
    rec: MaybeUninit<Context>,
    fill: FillArea,
    /// The slot after this one: the next of its chunk, the first of the
    /// next chunk, or null (the last slot of the last chunk). Written when
    /// a chunk is made or the next one linked, read by [`Records::take`].
    succ: *mut Slot,
}

const _: () = assert!(std::mem::offset_of!(Slot, rec) == 0);

type Chunk = [Slot; CHUNK_SLOTS];

/// The arena. Its fast paths touch `next` and one slot's `succ`; chunks
/// are made and freed only in [`Records::grow`] and `Drop`.
pub(crate) struct Records {
    /// The slot the next record takes.
    next: Cell<NonNull<Slot>>,
    /// Every chunk made, in order: owned (from `Box::into_raw`), freed
    /// when the arena is dropped.
    chunks: UnsafeCell<Vec<NonNull<Chunk>>>,
}

impl Records {
    /// An arena with its first chunk.
    pub(crate) fn new() -> Records {
        let first = new_chunk();
        Records { next: Cell::new(first.cast()), chunks: UnsafeCell::new(vec![first]) }
    }

    /// Take the next slot (uninitialized memory for a record and its fill
    /// area, at a stable address until it is released).
    #[inline(always)]
    pub(crate) fn take(&self) -> NonNull<Slot> {
        let p = self.next.get();
        // SAFETY: `next` is always a slot of a live chunk; `succ` is
        // initialized in every slot.
        let succ = unsafe { addr_of!((*p.as_ptr()).succ).read() };
        match NonNull::new(succ) {
            Some(s) => self.next.set(s),
            None => self.grow(p),
        }
        p
    }

    /// Release `p`, the slot taken last (strictly LIFO: every slot taken
    /// after it was released first). Its contents are dead: under
    /// `debug_assertions` they are overwritten with [`POISON`].
    #[inline(always)]
    pub(crate) fn release(&self, p: NonNull<Slot>) {
        #[cfg(debug_assertions)]
        {
            // SAFETY: `p` is a slot of a live chunk (the guard that releases
            // it took it); `succ` is initialized in every slot.
            let succ = unsafe { addr_of!((*p.as_ptr()).succ).read() };
            assert!(succ == self.next.get().as_ptr(), "robigo: a context slot released out of order");
            poison(p);
        }
        self.next.set(p);
    }

    /// `last` is the last slot of the last chunk: make the next chunk, link
    /// it after `last`, and make its first slot the next one.
    #[cold]
    #[inline(never)]
    fn grow(&self, last: NonNull<Slot>) {
        let c = new_chunk();
        // SAFETY: only `grow` and `Drop` touch `chunks`, never reentrantly;
        // `last` is a live slot.
        unsafe {
            (*self.chunks.get()).push(c);
            addr_of_mut!((*last.as_ptr()).succ).write(c.as_ptr().cast());
        }
        self.next.set(c.cast());
    }

    /// How many chunks the arena holds (the high-water mark, in chunks).
    pub(crate) fn chunks(&self) -> usize {
        // SAFETY: a read of the vector's length; nothing else touches it
        // during the call.
        unsafe { (*self.chunks.get()).len() }
    }

    /// How many slots are taken.
    pub(crate) fn depth(&self) -> usize {
        let next = self.next.get().as_ptr() as usize;
        // SAFETY: as in `chunks`.
        let chunks = unsafe { &*self.chunks.get() };
        let (i, c) = chunks
            .iter()
            .enumerate()
            .find(|(_, c)| (next.wrapping_sub(c.as_ptr() as usize)) < size_of::<Chunk>())
            .expect("robigo: the next context slot is in no chunk");
        i * CHUNK_SLOTS + (next - c.as_ptr() as usize) / size_of::<Slot>()
    }
}

impl Drop for Records {
    fn drop(&mut self) {
        debug_assert!(
            self.depth() == 0 || std::thread::panicking(),
            "robigo: an isolate dropped with context records pushed"
        );
        for c in self.chunks.get_mut().drain(..) {
            // SAFETY: made by `Box::into_raw` of a `Box<MaybeUninit<Chunk>>`
            // in `new_chunk`, freed once, here; no slot is in use (the
            // isolate is going).
            drop(unsafe { Box::from_raw(c.as_ptr().cast::<MaybeUninit<Chunk>>()) });
        }
    }
}

/// A chunk whose slots link to each other in order, the last to nothing.
/// Its memory is only ever reached through the raw pointer returned (the
/// links are made from it, never from a `Box` that is moved after).
fn new_chunk() -> NonNull<Chunk> {
    let raw = Box::into_raw(Box::<Chunk>::new_uninit());
    let base = raw.cast::<Slot>();
    for i in 0..CHUNK_SLOTS {
        let succ = if i + 1 < CHUNK_SLOTS { base.wrapping_add(i + 1) } else { null_mut() };
        // SAFETY: slot `i` of the allocation; `succ` is its one field that
        // must be initialized (the record and the fill area are
        // `MaybeUninit`).
        unsafe { addr_of_mut!((*base.add(i)).succ).write(succ) };
    }
    // SAFETY: a `Box`'s pointer is never null.
    unsafe { NonNull::new_unchecked(base.cast::<Chunk>()) }
}

/// Fill the record and the fill area of the released slot `p` with
/// [`POISON`] (its link stays).
#[cfg(debug_assertions)]
fn poison(p: NonNull<Slot>) {
    // SAFETY: `p` is a live chunk's slot that holds nothing any more; the
    // two writes stay inside its record and fill area.
    unsafe {
        addr_of_mut!((*p.as_ptr()).rec).cast::<u8>().write_bytes(POISON, size_of::<Context>());
        addr_of_mut!((*p.as_ptr()).fill).cast::<u8>().write_bytes(POISON, FILL_SIZE);
    }
}

/// A slot reserved for the next record its holder pushes
/// ([`crate::Cx::with_reserved`], [`crate::Cx::function_context_raw`]), and
/// its fill area, which holds a `T` for the record's owner while the
/// reservation lasts (`'r`). One record is pushed in it at most: pushing
/// consumes the reservation.
pub struct Reserved<'r, T> {
    pub(crate) slot: NonNull<Slot>,
    _r: PhantomData<(&'r mut (), *mut T)>,
}

impl<T> Reserved<'_, T> {
    /// The fill area, sized and aligned for a `T` (checked when the slot
    /// is reserved): uninitialized when reserved, valid to write and read
    /// until the reservation ends, and never read nor dropped by this
    /// crate. A `T` written there must be dropped by its writer before
    /// then, or it leaks.
    #[inline(always)]
    pub fn fill(&self) -> NonNull<T> {
        // SAFETY: a field address of the reserved slot, never null.
        unsafe { NonNull::new_unchecked(addr_of_mut!((*self.slot.as_ptr()).fill).cast::<T>()) }
    }

    /// Reserve `slot` (taken from the isolate's arena for this reservation).
    #[inline(always)]
    pub(crate) fn new(slot: NonNull<Slot>) -> Self {
        const { assert!(size_of::<T>() <= FILL_SIZE && align_of::<T>() <= FILL_ALIGN) };
        Reserved { slot, _r: PhantomData }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec_word(p: NonNull<Slot>) -> usize {
        // SAFETY: the slot's first word, initialized (written or poisoned)
        // by the test.
        unsafe { p.as_ptr().cast::<usize>().read() }
    }

    /// Slots come in order, cross into a new chunk past [`CHUNK_SLOTS`],
    /// come back LIFO, and the chunks stay (the high-water mark).
    #[test]
    fn slots_are_lifo_across_chunks() {
        let r = Records::new();
        assert_eq!((r.chunks(), r.depth()), (1, 0));
        let n = 2 * CHUNK_SLOTS + 3;
        let mut taken = Vec::new();
        for i in 0..n {
            let p = r.take();
            // SAFETY: a taken slot's record memory.
            unsafe { p.as_ptr().cast::<usize>().write(i) };
            taken.push(p);
            assert_eq!(r.depth(), i + 1);
        }
        assert_eq!(r.chunks(), 3);
        for (i, p) in taken.iter().enumerate() {
            assert_eq!(rec_word(*p), i, "slot {i} kept its contents");
        }
        // within a chunk, slots are adjacent; across, they are not
        // (compared, never read)
        let addr = |i: usize| taken[i].as_ptr() as usize;
        assert_eq!(addr(1) - addr(0), size_of::<Slot>());
        for i in (0..n).rev() {
            r.release(taken[i]);
            assert_eq!(r.depth(), i);
        }
        assert_eq!(r.chunks(), 3, "the chunks stay");
        // the same slots again, in the same order
        for (i, &q) in taken.iter().enumerate() {
            assert_eq!(r.take(), q, "slot {i}");
        }
        assert_eq!(r.chunks(), 3);
        for &q in taken.iter().rev() {
            r.release(q);
        }
    }

    /// Back and forth over a chunk boundary: no new chunk, no slot lost.
    #[test]
    fn a_chunk_boundary_is_crossed_both_ways() {
        let r = Records::new();
        let below: Vec<_> = (0..CHUNK_SLOTS).map(|_| r.take()).collect();
        for _ in 0..5 {
            let a = r.take();
            let b = r.take();
            assert_eq!(r.depth(), CHUNK_SLOTS + 2);
            r.release(b);
            r.release(a);
            assert_eq!(r.depth(), CHUNK_SLOTS);
        }
        assert_eq!(r.chunks(), 2);
        for &q in below.iter().rev() {
            r.release(q);
        }
        assert_eq!(r.depth(), 0);
    }

    /// A released slot reads as poison under `debug_assertions`.
    #[test]
    #[cfg(debug_assertions)]
    fn a_released_slot_is_poisoned() {
        let r = Records::new();
        let p = r.take();
        // SAFETY: a taken slot's record memory.
        unsafe { p.as_ptr().cast::<usize>().write(42) };
        r.release(p);
        assert_eq!(rec_word(p), usize::from_ne_bytes([POISON; 8]));
        // SAFETY: the released slot's fill area, poisoned (initialized).
        let last = unsafe { addr_of!((*p.as_ptr()).fill).cast::<u8>().add(FILL_SIZE - 1).read() };
        assert_eq!(last, POISON);
    }

    /// Releasing anything but the last slot taken is caught.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "released out of order")]
    fn an_out_of_order_release_panics() {
        let r = Records::new();
        let a = r.take();
        let _b = r.take();
        r.release(a);
    }

    /// The fill area takes any type that fits, at its alignment.
    #[test]
    fn the_fill_area_fits_its_type() {
        let r = Records::new();
        let p = r.take();
        let res = Reserved::<[u64; FILL_SIZE / 8]>::new(p);
        let f = res.fill();
        assert!(f.as_ptr().is_aligned());
        // SAFETY: the fill area holds the type (checked by `new`).
        unsafe { f.as_ptr().write([7; FILL_SIZE / 8]) };
        // SAFETY: written above.
        assert_eq!(unsafe { f.as_ptr().read() }[FILL_SIZE / 8 - 1], 7);
        r.release(p);
    }
}
