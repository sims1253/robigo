//! The process's shared immutable objects (H5, docs/g3.md §5.3):
//! `R_NilValue`, `NA_STRING`, `R_BlankString`, `R_UnboundValue` and
//! `R_MissingArg`, one of each for every heap of the process.
//!
//! They live in one segment, made once and never freed or written after it
//! is published. The segment is 64 KiB-aligned, as a block is. Its first
//! bytes are a pseudo block header whose remembered-set word
//! ([`REMEMBERED_OFF`]) holds [`SHARED_ID`]'s address, so that
//! [`Sexp::heap_id`] answers "shared" for them. Its magic word is not a
//! block's, so no heap takes it for one of its blocks.
//!
//! Every object is flagged `IMMORTAL | BARRIER` and points only at the
//! others:
//! - Marking and the mover stop at `IMMORTAL`, and no heap sweeps memory
//!   outside its own chunks.
//! - Reference counts, `SEEN_BY_C` and `MARK_NOT_MUTABLE` skip immortal
//!   objects.
//! - A store into one reaches the barrier's slow path, which refuses it
//!   (`barrier_slow`) before it writes anything.
//!
//! The objects are built as `Core::new` built its own until H5: the same
//! headers, gp bits, hashes and symbol ids.

use crate::heap::{BLOCK, DATA_OFF, REMEMBERED_OFF};
use crate::layout::{GRANULE, charsxp, charsxp_bytes, flags, gp, header, node, sym};
use crate::types::*;
use crate::{HeapId, Sexp, hash_bytes, heapsize::vec_cells, sym_hash};
use std::sync::OnceLock;

/// The address that stands for "shared" in a pseudo block header's
/// remembered-set word: no heap's remembered set, never read.
static SHARED_ID: u8 = 0;

/// The shared objects.
pub(crate) struct Shared {
    pub(crate) nil: Sexp,
    pub(crate) na_string: Sexp,
    pub(crate) blank_string: Sexp,
    pub(crate) unbound: Sexp,
    pub(crate) missing_arg: Sexp,
}

// SAFETY: the objects are written only by `make`, before `SHARED` publishes
// them (`OnceLock` orders those writes before every read), and never again.
// Every writer of a published object skips immortal ones (reference counts,
// `SEEN_BY_C`, `MARK_NOT_MUTABLE`, marking, moving), refuses shared ones
// (the barrier's slow path; rtcore's `set_object_bit` and `set_s4_bit`,
// which also leave an unchanged bit unwritten), or never reaches them
// (`install` does not mark the shared "" as a PRINTNAME).
unsafe impl Sync for Shared {}
// SAFETY: as for `Sync`: the pointers are to memory no thread frees.
unsafe impl Send for Shared {}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// How many objects the segment holds: every heap counts them among its
/// nodes in use, as GNU R counts its constants.
pub(crate) const OBJECTS: usize = 5;

/// The symbols among them (`R_UnboundValue` and `R_MissingArg`) take ids 0
/// and 1: a heap's own symbols start here.
pub(crate) const FIRST_SYMBOL_ID: u32 = 2;

/// The shared objects, made on first use.
#[inline]
pub(crate) fn shared() -> &'static Shared {
    SHARED.get_or_init(make)
}

/// The heap id of every shared object.
pub(crate) fn id() -> HeapId {
    HeapId(id_word() as usize)
}

/// What a shared segment's pseudo block header holds in its remembered-set
/// word (this segment's, and the symbol arena's).
pub(crate) fn id_word() -> *const u8 {
    &raw const SHARED_ID
}

impl HeapId {
    /// Whether this is the shared objects' id ([`crate::shared`]): an object
    /// every heap may hold.
    #[inline]
    pub fn is_shared(self) -> bool {
        self == id()
    }
}

/// Whether `x` is a shared object, from its pseudo block header.
///
/// # Safety
/// `x` is a live object.
#[inline]
pub(crate) unsafe fn is_shared(x: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { x.heap_id() }.is_shared()
}

/// Bytes the segment needs: the pseudo header, then the objects, each
/// granule-aligned (the node's size and the strings' are granule-rounded).
const SEGMENT: usize = DATA_OFF + 3 * node::SIZE + charsxp_bytes(2) + charsxp_bytes(0);

const _: () = assert!(SEGMENT <= BLOCK && node::SIZE.is_multiple_of(GRANULE));

fn make() -> Shared {
    let layout = std::alloc::Layout::from_size_align(SEGMENT, BLOCK).expect("robigo: the shared segment's layout");
    // SAFETY: a non-zero size. The memory is zeroed, so the pseudo header's
    // magic is not a block's.
    let base = unsafe { std::alloc::alloc_zeroed(layout) };
    if base.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    // SAFETY: every write below is inside the fresh segment, at granule-
    // aligned offsets past the pseudo header, and nothing else can see the
    // segment until `SHARED` publishes it.
    unsafe {
        *(base.add(REMEMBERED_OFF) as *mut *const u8) = id_word();
        let mut next = base.add(DATA_OFF);
        let mut take = |size: usize| {
            let p = next;
            next = next.add(size);
            p
        };
        let obj = |p: *mut u8, t: u8| {
            *(p as *mut u64) = header::first_word(t);
            Sexp(p)
        };
        // R_NilValue: a node whose fields point at itself (memory.c InitMemory)
        let nil = obj(take(node::size(NILSXP)), NILSXP);
        nil.set_attrib_raw(nil);
        nil.set_car_raw(nil);
        nil.set_cdr_raw(nil);
        nil.set_tag_raw(nil);
        nil.set_sexp_at(node::OFF_AUX, nil);
        // NA_STRING is not in the cache (names.c: mkChar("NA") is a different object)
        let na = string(obj(take(charsxp_bytes(2)), CHARSXP), b"NA", 0);
        // R_BlankString: the cached "", which every heap's cache is seeded with
        let blank = string(obj(take(charsxp_bytes(0)), CHARSXP), b"", gp::charsxp::ASCII | gp::charsxp::CACHED);
        // R_UnboundValue and R_MissingArg are symbols with empty names
        // that are not in the symbol table (names.c InitNames)
        let unbound = obj(take(node::size(SYMSXP)), SYMSXP);
        symbol(unbound, nil, blank, unbound, 0);
        let missing = obj(take(node::size(SYMSXP)), SYMSXP);
        symbol(missing, nil, blank, unbound, 1);
        debug_assert!(next <= base.add(SEGMENT));
        for x in [nil, na, blank, unbound, missing] {
            x.set_flag_raw(flags::IMMORTAL | flags::BARRIER, true);
        }
        Shared { nil, na_string: na, blank_string: blank, unbound, missing_arg: missing }
    }
}

/// A CHARSXP's fields, as `Core::alloc_char` writes them.
///
/// # Safety
/// `x` is a fresh CHARSXP with room for `bytes` and their NUL.
unsafe fn string(x: Sexp, bytes: &[u8], gpbits: u16) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe {
        *(x.0.add(charsxp::OFF_HASH) as *mut u64) = hash_bytes(bytes) as u64;
        x.set_std_len(bytes.len() as isize);
        x.set_truelen(0);
        let d = x.0.add(charsxp::OFF_DATA);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), d, bytes.len());
        *d.add(bytes.len()) = 0;
        x.set_gp(gpbits);
    }
    x
}

/// A symbol's fields, as `Core::new_symbol` writes them.
///
/// # Safety
/// `s` is a fresh SYMSXP; the others are objects of the segment.
unsafe fn symbol(s: Sexp, nil: Sexp, name: Sexp, value: Sexp, id: u32) {
    // SAFETY: the caller's contract; the 4th word is not traced for a
    // symbol, so it may hold plain bits.
    unsafe {
        s.set_attrib_raw(nil);
        s.set_car_raw(name);
        s.set_cdr_raw(value);
        s.set_tag_raw(nil);
        *(s.0.add(sym::OFF_ID) as *mut u64) = (sym_hash(name.char_bytes()) as u64) << 32 | id as u64;
    }
}

/// What a heap counts for the shared objects when it is made, as if it had
/// allocated them (`Core::alloc_object`'s count): their bytes and their
/// vector cells.
/// Each is (type, length, bytes, vector cells).
pub(crate) fn counts() -> [(u8, usize, usize, usize); OBJECTS] {
    let node = |t| node::size(t).max(GRANULE);
    [
        (NILSXP, 1, node(NILSXP), 0),
        (CHARSXP, 2, charsxp_bytes(2), vec_cells(CHARSXP, 2)),
        (CHARSXP, 0, charsxp_bytes(0), vec_cells(CHARSXP, 0)),
        (SYMSXP, 1, node(SYMSXP), 0),
        (SYMSXP, 1, node(SYMSXP), 0),
    ]
}
