// The definitions of `robigo_heap::layout` (docs/g3.md §8, the appendix's
// layout row): constants and `const fn`s only, reading nothing but
// `crate::types`. robigo-heap's build.rs includes this file as a module of
// its own and writes `robigo_layout.h` from [`TABLE`], so C reads the same
// numbers (robigo-rtapi's api.c, robigo-rtcore's Defn.h).

use crate::types::*;

/// The allocation unit and every object's alignment: sizes are whole
/// granules, so a word at a multiple of 8 is aligned for a pointer. The
/// allocator's size classes (heap.rs) are multiples of it.
pub const GRANULE: usize = 16;
/// A pointer-sized field.
pub const WORD: usize = 8;

/// `bytes` rounded up to whole granules.
#[inline(always)]
pub const fn granule_round(bytes: usize) -> usize {
    bytes.div_ceil(GRANULE) * GRANULE
}

/// The header every object starts with (16 bytes):
///
/// ```text
/// +0 type u8   +1 flags u8   +2 gp u16   +4 refcnt u16   +6 extra u16
/// +8 ATTRIB (a CHARSXP keeps its hash here instead: charsxp::OFF_HASH)
/// ```
pub mod header {
    /// The SEXPTYPE.
    pub const OFF_TYPE: usize = 0;
    /// robigo's flags ([`super::flags`]).
    pub const OFF_FLAGS: usize = 1;
    /// GNU R's `sxpinfo.gp` ([`super::gp`]).
    pub const OFF_GP: usize = 2;
    /// GNU R's `REFCNT`.
    pub const OFF_REFCNT: usize = 4;
    /// robigo's per-type bits ([`super::extra`]).
    pub const OFF_EXTRA: usize = 6;
    /// The attribute pairlist.
    pub const OFF_ATTRIB: usize = 8;
    /// A dead object's link word: the next free slot of its block (heap.rs),
    /// or a moved object's new address (the debug mover). The type byte at
    /// [`OFF_TYPE`] stays readable.
    pub const OFF_LINK: usize = OFF_ATTRIB;
    /// The header's size; every body starts here.
    pub const SIZE: usize = 16;

    /// A fresh object's first word: type `t` and no flags, gp, count or
    /// `extra` bits, which share it. A fresh header is this word and
    /// ATTRIB.
    #[inline(always)]
    pub const fn first_word(t: u8) -> u64 {
        const _: () = assert!(
            OFF_TYPE < OFF_ATTRIB && OFF_FLAGS < OFF_ATTRIB && OFF_EXTRA + 2 <= OFF_ATTRIB && OFF_ATTRIB == super::WORD
        );
        let mut w = [0u8; super::WORD];
        w[OFF_TYPE] = t;
        u64::from_ne_bytes(w)
    }
}

/// A standard (not ALTREP) vector of any type but CHARSXP: the header,
/// LENGTH, TRUELENGTH, then the payload (or EXTDATA's words, [`super::extdata`]).
pub mod vector {
    pub const OFF_LEN: usize = super::header::SIZE;
    /// GNU R's TRUELENGTH: the growable capacity, a hash table's fill
    /// count (docs/g3.md §8: every variant keeps storage for it).
    pub const OFF_TRUELEN: usize = OFF_LEN + super::WORD;
    /// The payload: the vector header's size.
    pub const OFF_DATA: usize = OFF_TRUELEN + super::WORD;
}

/// A CHARSXP: its hash where attributes go (it has none), LENGTH,
/// TRUELENGTH (the hash of `HASHASH`, `radixsort.c`'s scratch), then the
/// bytes and a NUL. A layout of its own, which a vector-header variant
/// (D8's h24) need not change.
pub mod charsxp {
    /// The CHARSXP cache's hash (`crate::hash_bytes`), a `u32`.
    pub const OFF_HASH: usize = super::header::OFF_ATTRIB;
    pub const OFF_LEN: usize = super::header::SIZE;
    pub const OFF_TRUELEN: usize = OFF_LEN + super::WORD;
    pub const OFF_DATA: usize = OFF_TRUELEN + super::WORD;
}

/// An EXTDATA vector (`Core::alloc_extdata`): the vector header, then its
/// payload pointer where a standard vector's payload starts, its owner word
/// and its release function.
pub mod extdata {
    /// The payload pointer.
    pub const OFF_PTR: usize = super::vector::OFF_DATA;
    /// The owner word, the argument of the release function.
    pub const OFF_OWNER: usize = OFF_PTR + super::WORD;
    /// The release function, an `Option<ReleaseFn>`: null is `None`, which
    /// C code that disarms the call writes.
    pub const OFF_RELEASE: usize = OFF_OWNER + super::WORD;
    /// The bytes it is allocated with.
    pub const SIZE: usize = OFF_RELEASE + super::WORD;
}

/// Node-shaped objects (`Core::alloc_node`): the header, then CAR, CDR, TAG
/// and the 4th word ([`OFF_AUX`]). Every type that is not a vector, and
/// ALTREP instances, has this body; ENVSXP slot frames ([`super::slots`])
/// extend it.
///
/// ```text
/// LISTSXP, LANGSXP, DOTSXP   CAR      CDR    TAG       aux: call-site cache
/// CLOSXP                     FORMALS  BODY   CLOENV    aux: code cache
/// PROMSXP                    PRVALUE  PRCODE PRENV     (unboxed: promise_tag)
/// ENVSXP                     FRAME    ENCLOS HASHTAB   aux: version box
/// SYMSXP                     PRINTNAME SYMVALUE INTERNAL  aux: id | hash (not an object)
/// EXTPTRSXP                  address  PROT   TAG
/// ALTREP                     data1    data2  class
/// ```
pub mod node {
    use crate::types::*;

    pub const OFF_CAR: usize = super::header::SIZE;
    pub const OFF_CDR: usize = OFF_CAR + super::WORD;
    pub const OFF_TAG: usize = OFF_CDR + super::WORD;
    /// The 4th node word (council item 20): R_NilValue when made, traced
    /// by every collection (but a symbol's, [`super::sym::OFF_ID`]). No C
    /// API reaches it: it is the runtime's (G3), for the environment's
    /// version box, a closure's compiled-code cache and a call's call-site
    /// cache (docs/core.md §2).
    pub const OFF_AUX: usize = OFF_TAG + super::WORD;
    /// The largest node body: header, car, cdr, tag and the 4th word (GNU
    /// R's cons cell is 56 bytes).
    pub const SIZE: usize = OFF_AUX + super::WORD;

    /// The bytes a node of type `t` is allocated with (D13's per-type node
    /// sizes: one size today).
    #[inline(always)]
    pub const fn size(t: u8) -> usize {
        let _ = t;
        SIZE
    }

    /// Whether a node of type `t` has the 4th word ([`OFF_AUX`]).
    #[inline(always)]
    pub const fn has_aux(t: u8) -> bool {
        size(t) > OFF_AUX
    }

    /// Whether a node may change from type `from` to `to` in place (GNU
    /// R's `SET_TYPEOF`): only between types of one body size.
    #[inline(always)]
    pub const fn same_body(from: u8, to: u8) -> bool {
        size(from) == size(to)
    }

    const _: () = assert!(size(LISTSXP) == SIZE && size(LANGSXP) == SIZE && has_aux(CLOSXP));
}

/// A symbol's node fields.
pub mod sym {
    pub const OFF_PRINTNAME: usize = super::node::OFF_CAR;
    pub const OFF_VALUE: usize = super::node::OFF_CDR;
    pub const OFF_INTERNAL: usize = super::node::OFF_TAG;
    /// A symbol's 4th word: its id (low 32 bits) and GNU R's
    /// `R_Newhashpjw` of its name (high 32 bits), set when the symbol is
    /// made and never changed. Not an object: the collector does not trace
    /// it. The id is dense, in the order the heap made its symbols
    /// (`R_UnboundValue` 0, `R_MissingArg` 1, then installs): the runtime's
    /// per-isolate binding tables are indexed by it (docs/g3.md §5.3; M2's
    /// shared symbol arena computes it instead).
    pub const OFF_ID: usize = super::node::OFF_AUX;
}

/// An environment's body (council item 20): only ENCLOS has a fixed
/// offset. FRAME and HASHTAB are shims over the frame words (`crate::env`),
/// so a frame representation is free to change; a slot frame's words are
/// [`super::slots`]'.
pub mod env {
    /// A hashed environment's FRAME (R_NilValue).
    pub const OFF_FRAME: usize = super::node::OFF_CAR;
    pub const OFF_ENCLOS: usize = super::node::OFF_CDR;
    /// A hashed environment's hash table (a VECSXP of chains).
    pub const OFF_HASHTAB: usize = super::node::OFF_TAG;
    /// The runtime's version box, or R_NilValue.
    pub const OFF_VERSION: usize = super::node::OFF_AUX;
}

/// A promise's node fields.
pub mod promise {
    /// PRVALUE: an object, or an unboxed value's raw bits under
    /// `PROMISE_TAG` ([`super::extra::promise::TAG_MASK`]).
    pub const OFF_VALUE: usize = super::node::OFF_CAR;
    pub const OFF_CODE: usize = super::node::OFF_CDR;
    pub const OFF_ENV: usize = super::node::OFF_TAG;
}

/// An external pointer's node fields.
pub mod extptr {
    /// The address: a word, not an object (never traced).
    pub const OFF_ADDR: usize = super::node::OFF_CAR;
    pub const OFF_PROT: usize = super::node::OFF_CDR;
    pub const OFF_TAG: usize = super::node::OFF_TAG;
}

/// An ALTREP instance (a node whatever its type): data1, data2, the class
/// and the 4th word (R_NilValue).
pub mod altrep {
    pub const OFF_DATA1: usize = super::node::OFF_CAR;
    pub const OFF_DATA2: usize = super::node::OFF_CDR;
    pub const OFF_CLASS: usize = super::node::OFF_TAG;
}

/// The word map of a slot frame (an ENVSXP with [`super::flags::SLOTS`],
/// `crate::slots`; docs/g3.md §5.1): a node's words, reused, then the
/// caller, the meta word and the slots.
///
/// ```text
/// ENVSXP with flags::SLOTS: 64 + 32n bytes
/// +0   header      flags SLOTS; gp ENVFLAGS (FRAME_LOCK, GLOBAL_FRAME, NO_SPECIAL_SYMBOLS)
/// +8   ATTRIB
/// +16  overflow    R_NilValue, or an overflow block (the same map, no shape)
/// +24  ENCLOS      (the heap's one fixed environment field)
/// +32  shape       R_NilValue, or the shape: a VECSXP of the inline slots' symbols
/// +40  version     R_NilValue, or the frame's version box (fn_version)
/// +48  caller      R_NilValue, or the environment the delayed slots of ORIGIN caller evaluate in
/// +56  meta        capacity u32 (inline slots) | frame: next stamp u32; overflow block: slots used u32
/// +64  slot[n]     32 bytes each:
///        +0  key   symbol address | kind (bits 0-3)
///        +8  val   by kind: an object, raw scalar bits, or null
///        +16 aux   by kind: the expression of a delayed or forced slot, else null
///        +24 meta  definition stamp (bits 0-31) | flags (bits 32-40)
/// ```
pub mod slots {
    pub const OFF_OVERFLOW: usize = super::node::OFF_CAR;
    /// ENCLOS, where every environment has it.
    pub const OFF_ENCLOS: usize = super::env::OFF_ENCLOS;
    pub const OFF_SHAPE: usize = super::node::OFF_TAG;
    pub const OFF_VERSION: usize = super::env::OFF_VERSION;
    pub const OFF_CALLER: usize = super::node::SIZE;
    pub const OFF_META: usize = OFF_CALLER + super::WORD;
    pub const OFF_SLOTS: usize = OFF_META + super::WORD;
    pub const SLOT_SIZE: usize = 4 * super::WORD;
    pub const SLOT_KEY: usize = 0;
    pub const SLOT_VAL: usize = SLOT_KEY + super::WORD;
    pub const SLOT_AUX: usize = SLOT_VAL + super::WORD;
    pub const SLOT_META: usize = SLOT_AUX + super::WORD;

    /// The bytes of a slot frame (or overflow block) with `capacity` slots.
    #[inline]
    pub const fn frame_bytes(capacity: usize) -> usize {
        OFF_SLOTS + SLOT_SIZE * capacity
    }

    const _: () = assert!(OFF_SLOTS.is_multiple_of(super::GRANULE) && SLOT_SIZE.is_multiple_of(super::GRANULE));
}

/// Bytes per element of a vector type (0: not a vector).
#[inline]
pub const fn elt_size(t: u8) -> usize {
    match t {
        LGLSXP | INTSXP => 4,
        REALSXP => 8,
        CPLXSXP => 16,
        STRSXP | VECSXP | EXPRSXP | WEAKREFSXP => 8,
        RAWSXP | CHARSXP => 1,
        _ => 0,
    }
}

/// Where a standard vector of type `t` keeps its payload.
#[inline(always)]
pub const fn data_off(t: u8) -> usize {
    if t == CHARSXP { charsxp::OFF_DATA } else { vector::OFF_DATA }
}

/// The bytes a standard vector of type `t` and length `n` is allocated
/// with (a CHARSXP's without its NUL: [`charsxp_bytes`]), or None when that
/// overflows.
#[inline]
pub fn vector_bytes(t: u8, n: usize) -> Option<usize> {
    Some(n.checked_mul(elt_size(t))?.checked_add(data_off(t) + GRANULE - 1)? / GRANULE * GRANULE)
}

/// The bytes a CHARSXP of `n` bytes is allocated with (with its NUL).
#[inline]
pub const fn charsxp_bytes(n: usize) -> usize {
    granule_round(charsxp::OFF_DATA + n + 1)
}

/// Whether no two of `bits` share a bit: the no-overlap check of each map.
const fn disjoint(bits: &[u64]) -> bool {
    let mut seen = 0u64;
    let mut i = 0;
    while i < bits.len() {
        if seen & bits[i] != 0 {
            return false;
        }
        seen |= bits[i];
        i += 1;
    }
    true
}

#[cfg(test)]
pub(super) fn disjoint_for_tests(bits: &[u64]) -> bool {
    disjoint(bits)
}

/// Header flag bits (byte +1): one map for every type.
pub mod flags {
    /// GNU R's `obj` bit: the object has a class attribute.
    pub const OBJECT: u8 = 0x01;
    pub const ALTREP: u8 = 0x02;
    /// The payload is elsewhere: a data pointer at `extdata::OFF_PTR`,
    /// owned by the words after it ([`super::extdata`]).
    pub const EXTDATA: u8 = 0x04;
    /// Never marked, moved or freed (`Core::make_immortal`, and the
    /// process's shared constants). The contract that comes with it:
    /// - The collector does not trace an immortal object, so what it points
    ///   at must be immortal too, or kept alive some other way, until a
    ///   store into it passes the barrier ([`BARRIER`], set with this flag),
    ///   which makes it a root of its heap.
    /// - Reference counts, `SEEN_BY_C` and `MARK_NOT_MUTABLE` skip it: its
    ///   header is not written after it is made, beyond that one barrier bit.
    /// - A shared constant (H5, `shared.rs`) takes no store at all: the
    ///   barrier's slow path refuses it before writing anything.
    pub const IMMORTAL: u8 = 0x08;
    /// The barrier's state bit (council item 8): a store into this object
    /// must take the barrier's slow path (`Sexp::store`), which logs the
    /// object in its heap's remembered set and clears the bit, so each
    /// object is logged once. Set today on every immortal object (made a
    /// root by its first store); a generational or incremental collector
    /// sets it on the objects it promotes (sticky marks, docs/core.md §11),
    /// so the inline fast path stays one test of this byte.
    pub const BARRIER: u8 = 0x10;
    /// Free. Was the second header mark color, retired when marks moved to
    /// the side bitmaps (docs/core.md §10). Kept for the barrier work: Ruby
    /// RGenGC's "write-barrier unprotected" bit for objects whose payload
    /// pointer C has (`DATAPTR` on a STRSXP or VECSXP: docs/core.md §11).
    pub const SPARE: u8 = 0x20;
    /// An ENVSXP laid out as a slot frame or an overflow block
    /// ([`super::slots`]): its body is the slot map, not node fields.
    pub const SLOTS: u8 = 0x40;
    /// Do not maintain reference counts of the children: GNU R's `spare`
    /// bit (TRACKREFS(x) is `!spare`, and closures always track).
    pub const NO_TRACKREFS: u8 = 0x80;

    const _: () = assert!(super::disjoint(&[
        OBJECT as u64,
        ALTREP as u64,
        EXTDATA as u64,
        IMMORTAL as u64,
        BARRIER as u64,
        SPARE as u64,
        SLOTS as u64,
        NO_TRACKREFS as u64,
    ]));
}

/// GNU R's `sxpinfo.gp` bits (header +2; `Defn.h` is the spec): what a bit
/// means depends on the type, so each type has its map, and [`S4`] is in
/// all of them. Each map's bits are checked not to overlap.
pub mod gp {
    /// `S4_OBJECT_MASK` (Defn.h:367), on any object.
    pub const S4: u16 = 1 << 4;

    /// A CHARSXP's encoding and cache bits (Defn.h:934-940).
    pub mod charsxp {
        /// `HASHASH_MASK`: TRUELENGTH holds `R_Newhashpjw` of the bytes
        /// (set on every symbol's PRINTNAME by install).
        pub const HASHASH: u16 = 1;
        pub const BYTES: u16 = 1 << 1;
        pub const LATIN1: u16 = 1 << 2;
        pub const UTF8: u16 = 1 << 3;
        pub const CACHED: u16 = 1 << 5;
        pub const ASCII: u16 = 1 << 6;
        const _: () = assert!(super::super::disjoint(&[
            super::S4 as u64,
            HASHASH as u64,
            BYTES as u64,
            LATIN1 as u64,
            UTF8 as u64,
            CACHED as u64,
            ASCII as u64
        ]));
    }

    /// Any other vector.
    pub mod vector {
        /// `GROWABLE_MASK` (Defn.h:382): TRUELENGTH is the capacity.
        pub const GROWABLE: u16 = 1 << 5;
        const _: () = assert!(super::super::disjoint(&[super::S4 as u64, GROWABLE as u64]));
    }

    /// A weak reference (memory.c).
    pub mod weakref {
        /// `READY_TO_FINALIZE_MASK`.
        pub const READY_TO_FINALIZE: u16 = 1;
        /// `FINALIZE_ON_EXIT_MASK`.
        pub const FINALIZE_ON_EXIT: u16 = 2;
        const _: () =
            assert!(super::super::disjoint(&[super::S4 as u64, READY_TO_FINALIZE as u64, FINALIZE_ON_EXIT as u64]));
    }

    /// A symbol. GNU R "use[s] the same bits (15 and 14) in symbols and
    /// bindings" (Defn.h:1305).
    pub mod sym {
        /// `DDVAL_MASK` (Defn.h:526): the symbol is `..1`, `..2`, ...
        pub const DDVAL: u16 = 1;
        /// `SPECIAL_SYMBOL_MASK` (Defn.h:1331).
        pub const SPECIAL_SYMBOL: u16 = 1 << 12;
        /// `BINDING_LOCK_MASK`: a base binding is locked.
        pub const BINDING_LOCK: u16 = 1 << 14;
        /// `ACTIVE_BINDING_MASK`: a base binding is active.
        pub const ACTIVE_BINDING: u16 = 1 << 15;
        const _: () = assert!(super::super::disjoint(&[
            super::S4 as u64,
            DDVAL as u64,
            SPECIAL_SYMBOL as u64,
            BINDING_LOCK as u64,
            ACTIVE_BINDING as u64
        ]));
    }

    /// A pairlist cell: a binding (a frame's cell), or an argument.
    pub mod cell {
        /// `MISSING_MASK` (Defn.h:456): four bits, values 0-2.
        pub const MISSING: u16 = 15;
        /// `ASSIGNMENT_PENDING_MASK` (Defn.h:320): a complex assignment to
        /// this binding is in progress, and the next store keeps the old
        /// value's count.
        pub const ASSIGNMENT_PENDING: u16 = 1 << 11;
        /// `BINDING_LOCK_MASK` (Defn.h:1309).
        pub const BINDING_LOCK: u16 = 1 << 14;
        /// `ACTIVE_BINDING_MASK` (Defn.h:1308).
        pub const ACTIVE_BINDING: u16 = 1 << 15;
        const _: () = assert!(super::super::disjoint(&[
            super::S4 as u64,
            MISSING as u64,
            ASSIGNMENT_PENDING as u64,
            BINDING_LOCK as u64,
            ACTIVE_BINDING as u64
        ]));
    }

    /// A call (LANGSXP).
    pub mod call {
        /// `MARK_ASSIGNMENT_CALL` sets `ASSIGNMENT_PENDING_MASK` on a call
        /// (Defn.h:320-326).
        pub const ASSIGNMENT_CALL: u16 = 1 << 11;
        const _: () = assert!(super::super::disjoint(&[super::S4 as u64, ASSIGNMENT_CALL as u64]));
    }

    /// An environment's ENVFLAGS.
    pub mod env {
        /// `NO_SPECIAL_SYMBOLS` (Defn.h:1335): no special symbol was ever
        /// bound here (the symbol's `SPECIAL_SYMBOL_MASK` bit).
        pub const NO_SPECIAL_SYMBOLS: u16 = 1 << 12;
        /// `FRAME_LOCK_MASK`: the frame is locked.
        pub const FRAME_LOCK: u16 = 1 << 14;
        /// `GLOBAL_FRAME_MASK`: the global environment and attached
        /// frames, whose bindings the global cache may hold.
        pub const GLOBAL_FRAME: u16 = 1 << 15;
        const _: () = assert!(super::super::disjoint(&[
            super::S4 as u64,
            NO_SPECIAL_SYMBOLS as u64,
            FRAME_LOCK as u64,
            GLOBAL_FRAME as u64
        ]));
    }

    /// A closure's JIT state.
    pub mod closure {
        /// `NOJIT_MASK` (Defn.h:373).
        pub const NOJIT: u16 = 1 << 5;
        /// `MAYBEJIT_MASK` (Defn.h:376).
        pub const MAYBEJIT: u16 = 1 << 6;
        const _: () = assert!(super::super::disjoint(&[super::S4 as u64, NOJIT as u64, MAYBEJIT as u64]));
    }

    // GNU R reads these as one bit on either type: the binding bits of
    // symbols and cells (Defn.h:1305), SPECIAL_SYMBOL_MASK on an
    // environment (NO_SPECIAL_SYMBOLS), ASSIGNMENT_PENDING_MASK on a call.
    const _: () = assert!(
        sym::ACTIVE_BINDING == cell::ACTIVE_BINDING
            && sym::BINDING_LOCK == cell::BINDING_LOCK
            && env::NO_SPECIAL_SYMBOLS == sym::SPECIAL_SYMBOL
            && call::ASSIGNMENT_CALL == cell::ASSIGNMENT_PENDING
    );

    /// A promise's `PRSEEN` (GNU R keeps it in the whole gp field).
    pub mod promise {
        /// The promise is being evaluated.
        pub const PRSEEN_EVALUATING: u16 = 1;
        /// Its evaluation was interrupted.
        pub const PRSEEN_INTERRUPTED: u16 = 2;
        const _: () =
            assert!(super::super::disjoint(&[super::S4 as u64, PRSEEN_EVALUATING as u64, PRSEEN_INTERRUPTED as u64]));
    }
}

/// robigo's bits of the header's `extra` word (+6): what a bit means
/// depends on the type, so each type has its map, and [`SEEN_BY_C`] is in
/// all of them. A fresh allocation writes the whole header, so every bit
/// starts clear. Each map's bits are checked not to overlap; the bits a
/// map leaves free are listed with it.
pub mod extra {
    /// The object was handed to C (a native routine's argument, an export's
    /// result, a word a C frame pinned): sticky, since C may keep the
    /// address where no collection looks. `ROBIGO_GC_MOVE=1` never moves
    /// such an object. Set it with `Sexp::mark_seen_by_c`, which keeps the
    /// other bits. On every type.
    pub const SEEN_BY_C: u16 = 1 << 4;
    /// The bits code tests without knowing the type: [`SEEN_BY_C`], and the
    /// code-source marks of [`cell`], which every setter's write barrier
    /// tests on whatever node it writes into (robigo-rtcore's
    /// `note_code_write`, the C API's setters). Every map but the one that
    /// sets a bit leaves it clear, so these reads see 0 on other types.
    pub const TYPE_BLIND: u16 = SEEN_BY_C | cell::AST_SOURCE | cell::FORMALS_SOURCE;

    /// A closure. Free: 6, 8-12, 15.
    pub mod closure {
        /// GNU R's RDEBUG (`sxpinfo.debug`).
        pub const DEBUG: u16 = 1 << 0;
        /// GNU R's RTRACE (`sxpinfo.trace`).
        pub const TRACE: u16 = 1 << 1;
        /// GNU R's RSTEP (`sxpinfo.spare` on a closure).
        pub const STEP: u16 = 1 << 2;
        /// Kept with GNU R's three for debug.c's port.
        pub const DEBUG_RESERVED: u16 = 1 << 3;
        /// The debug bits: a closure with any of them set runs where GNU
        /// R's debugger can see it.
        pub const DEBUG_BITS: u16 = DEBUG | TRACE | STEP | DEBUG_RESERVED;
        /// GNU R holds this closure's BODY as byte code (a BCODESXP:
        /// installed with base, or compiled by GNU R's JIT), which robigo
        /// does not make in M1 (docs/g3.md §6.1, §9.1). Set and read by
        /// robigo-rt's JIT port; `duplicate` copies it with the body.
        pub const BYTECODE: u16 = 1 << 5;
        /// robigo-rt's verdict on its attached byte code is that a compiled
        /// caller cannot run it inline (`robigo-rt`'s `inline`). Set once,
        /// never cleared: a stale mark only costs an inline run, never a
        /// result.
        pub const NO_INLINE: u16 = 1 << 14;
        const _: () = assert!(super::super::disjoint(&[
            super::TYPE_BLIND as u64,
            DEBUG_BITS as u64,
            BYTECODE as u64,
            NO_INLINE as u64
        ]));
    }

    /// A promise. Free: 0-3, 6, 14, 15.
    pub mod promise {
        /// PRCODE is byte code: compiled code made the promise.
        pub const BYTECODE: u16 = 1 << 5;
        /// Where a promise keeps GNU R's `PROMISE_TAG`
        /// (`IMMEDIATE_PROMISE_VALUES`, `Defn.h:34-39`): bits 8-12, the type
        /// of the unboxed value in PRVALUE (REALSXP, INTSXP or LGLSXP), 0 for
        /// none.
        pub const TAG_SHIFT: u16 = 8;
        pub const TAG_MASK: u16 = 0x1F << TAG_SHIFT;
        const _: () = assert!(super::super::disjoint(&[super::TYPE_BLIND as u64, BYTECODE as u64, TAG_MASK as u64]));
    }

    /// A pairlist or call cell (LISTSXP, LANGSXP, DOTSXP). Free: 0-3, 6,
    /// 8-12, 14, 15.
    pub mod cell {
        /// On a `function` call that base's loader binds: the closure it
        /// makes is installed byte code (robigo-rt's `jit::installed_literal`).
        pub const BYTECODE: u16 = 1 << 5;
        /// robigo-rt compiled code for GNU R's AST (its `Eval` profile)
        /// from an expression this cell is part of. GNU R walks such an
        /// expression live, so a write into the cell through a setter
        /// (`SETCAR`, `SETCDR`, `SET_TAG`) makes that code stale
        /// (robigo-rtcore's code floors, docs/g3.md §9.3). `duplicate` does
        /// not copy it.
        pub const AST_SOURCE: u16 = 1 << 7;
        /// Part of the formals (the spine or a default) of a closure
        /// robigo-rt compiled. GNU R reads a closure's formals at every
        /// call, whatever runs its body, so a write into the cell makes
        /// every closure's code stale. `duplicate` does not copy it.
        pub const FORMALS_SOURCE: u16 = 1 << 13;
        const _: () = assert!(super::super::disjoint(&[
            super::SEEN_BY_C as u64,
            BYTECODE as u64,
            AST_SOURCE as u64,
            FORMALS_SOURCE as u64
        ]));
    }

    /// An environment. Free: 0-3, 5, 8-12, 14.
    pub mod env {
        /// On a slot frame: the closure call that made the frame is still
        /// running, so its supplied arguments are also held by the call's
        /// argument list as GNU R's are (robigo-rtcore's `env::promise`).
        pub const CALL_LIVE: u16 = 1 << 6;
        /// On a slot frame: R code or C had the frame as a value (a
        /// context's `cloenv` or `sysparent` read as one), where no count
        /// shows it, so the frame is never recycled (robigo-rtcore's
        /// `env::recycle`, D5). Sticky until the frame is reused.
        pub const ESCAPED: u16 = 1 << 15;
        const _: () = assert!(super::super::disjoint(&[super::TYPE_BLIND as u64, CALL_LIVE as u64, ESCAPED as u64]));
    }

    /// A vector. Free: 0, 5, 6, 8-12, 14, 15.
    pub mod vector {
        /// GNU R's RTRACE (`sxpinfo.trace`): `tracemem` is on.
        pub const TRACE: u16 = 1 << 1;
        /// An ALTREP vector (`flags::ALTREP`) that is an instance of one of
        /// robigo-rt's own classes (robigo-rtcore's `altrep::own`), set by
        /// the one maker of such instances: the hot accessors tell it from
        /// a package class's with this bit, without reading the class.
        /// Bit 2 is free on main and in all four X3 layout cells (a code
        /// object takes bit 8, the h24 cells bit 0): H, 2026-11-17.
        pub const OWN_ALTREP: u16 = 1 << 2;
        /// The boot heap's freeze leaves this object ordinary (robigo-heap's
        /// `gc/freeze.rs`): set on a RAWSXP that is replaced soon after the
        /// boot, so that its replacement writes over no frozen object. C1's
        /// translation-cache stub sets it. The freeze tests it on RAWSXPs only.
        pub const NO_FREEZE: u16 = 1 << 3;
        const _: () = assert!(super::super::disjoint(&[
            super::TYPE_BLIND as u64,
            TRACE as u64,
            OWN_ALTREP as u64,
            NO_FREEZE as u64
        ]));
    }

    /// A code object (robigo-rt's `code.rs`: a VECSXP), which has the
    /// vector bits too. Free: 0, 5, 6, 9-12, 14, 15.
    pub mod code {
        /// The code object holds weak cache words (inline caches that
        /// name an object without keeping it alive: docs/g3.md §6.1, the
        /// appendix's `trace` row), which a collection clears after marking
        /// when their object died. Reserved for C1: nothing sets it yet,
        /// and the trace arm that reads it comes with the first cache.
        pub const WEAK_CACHE: u16 = 1 << 8;
        const _: () = assert!(super::super::disjoint(&[
            super::TYPE_BLIND as u64,
            super::vector::TRACE as u64,
            super::vector::OWN_ALTREP as u64,
            super::vector::NO_FREEZE as u64,
            WEAK_CACHE as u64
        ]));
    }
}

/// Everything C reads, by the name `robigo_layout.h` defines it under
/// (`ROBIGO_<name>`), in one list: build.rs writes the header from it, and
/// robigo-rtapi checks the table api.c was compiled with against it.
pub const TABLE: &[(&str, u64)] = &[
    ("GRANULE", GRANULE as u64),
    ("HDR_OFF_TYPE", header::OFF_TYPE as u64),
    ("HDR_OFF_FLAGS", header::OFF_FLAGS as u64),
    ("HDR_OFF_GP", header::OFF_GP as u64),
    ("HDR_OFF_REFCNT", header::OFF_REFCNT as u64),
    ("HDR_OFF_EXTRA", header::OFF_EXTRA as u64),
    ("HDR_OFF_ATTRIB", header::OFF_ATTRIB as u64),
    ("HDR_SIZE", header::SIZE as u64),
    ("VEC_OFF_LEN", vector::OFF_LEN as u64),
    ("VEC_OFF_TRUELEN", vector::OFF_TRUELEN as u64),
    ("VEC_OFF_DATA", vector::OFF_DATA as u64),
    ("CHAR_OFF_HASH", charsxp::OFF_HASH as u64),
    ("CHAR_OFF_LEN", charsxp::OFF_LEN as u64),
    ("CHAR_OFF_TRUELEN", charsxp::OFF_TRUELEN as u64),
    ("CHAR_OFF_DATA", charsxp::OFF_DATA as u64),
    ("EXTDATA_OFF_PTR", extdata::OFF_PTR as u64),
    ("NODE_OFF_CAR", node::OFF_CAR as u64),
    ("NODE_OFF_CDR", node::OFF_CDR as u64),
    ("NODE_OFF_TAG", node::OFF_TAG as u64),
    ("FLAG_OBJECT", flags::OBJECT as u64),
    ("FLAG_ALTREP", flags::ALTREP as u64),
    ("FLAG_EXTDATA", flags::EXTDATA as u64),
    ("FLAG_IMMORTAL", flags::IMMORTAL as u64),
    ("EXTRA_SEEN_BY_C", extra::SEEN_BY_C as u64),
    ("GP_S4", gp::S4 as u64),
    ("GP_CHAR_HASHASH", gp::charsxp::HASHASH as u64),
    ("GP_CHAR_BYTES", gp::charsxp::BYTES as u64),
    ("GP_CHAR_LATIN1", gp::charsxp::LATIN1 as u64),
    ("GP_CHAR_UTF8", gp::charsxp::UTF8 as u64),
    ("GP_CHAR_CACHED", gp::charsxp::CACHED as u64),
    ("GP_CHAR_ASCII", gp::charsxp::ASCII as u64),
    ("GP_VEC_GROWABLE", gp::vector::GROWABLE as u64),
    ("GP_WEAKREF_READY_TO_FINALIZE", gp::weakref::READY_TO_FINALIZE as u64),
    ("GP_WEAKREF_FINALIZE_ON_EXIT", gp::weakref::FINALIZE_ON_EXIT as u64),
    ("GP_SYM_DDVAL", gp::sym::DDVAL as u64),
    ("GP_SYM_SPECIAL_SYMBOL", gp::sym::SPECIAL_SYMBOL as u64),
    ("GP_SYM_BINDING_LOCK", gp::sym::BINDING_LOCK as u64),
    ("GP_SYM_ACTIVE_BINDING", gp::sym::ACTIVE_BINDING as u64),
    ("GP_CELL_MISSING", gp::cell::MISSING as u64),
    ("GP_CELL_ASSIGNMENT_PENDING", gp::cell::ASSIGNMENT_PENDING as u64),
    ("GP_CELL_BINDING_LOCK", gp::cell::BINDING_LOCK as u64),
    ("GP_CELL_ACTIVE_BINDING", gp::cell::ACTIVE_BINDING as u64),
    ("GP_ENV_NO_SPECIAL_SYMBOLS", gp::env::NO_SPECIAL_SYMBOLS as u64),
    ("GP_ENV_FRAME_LOCK", gp::env::FRAME_LOCK as u64),
    ("GP_ENV_GLOBAL_FRAME", gp::env::GLOBAL_FRAME as u64),
    ("GP_CLOSURE_NOJIT", gp::closure::NOJIT as u64),
    ("GP_CLOSURE_MAYBEJIT", gp::closure::MAYBEJIT as u64),
];

/// The C view of the layout, asserted here and, from the same numbers, in
/// api.c: a `#[repr(C)]` struct per body whose field offsets must be the
/// table's, so neither language can read a field the other does not.
#[repr(C)]
pub struct HeaderView {
    pub typ: u8,
    pub flags: u8,
    pub gp: u16,
    pub refcnt: u16,
    pub extra: u16,
    pub attrib: *mut u8,
}

/// A vector's header words.
#[repr(C)]
pub struct VectorView {
    pub header: HeaderView,
    pub length: isize,
    pub truelength: isize,
}

/// A node's words.
#[repr(C)]
pub struct NodeView {
    pub header: HeaderView,
    pub car: *mut u8,
    pub cdr: *mut u8,
    pub tag: *mut u8,
    pub aux: *mut u8,
}

const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(offset_of!(HeaderView, typ) == header::OFF_TYPE);
    assert!(offset_of!(HeaderView, flags) == header::OFF_FLAGS);
    assert!(offset_of!(HeaderView, gp) == header::OFF_GP);
    assert!(offset_of!(HeaderView, refcnt) == header::OFF_REFCNT);
    assert!(offset_of!(HeaderView, extra) == header::OFF_EXTRA);
    assert!(offset_of!(HeaderView, attrib) == header::OFF_ATTRIB);
    assert!(size_of::<HeaderView>() == header::SIZE);
    assert!(offset_of!(VectorView, length) == vector::OFF_LEN);
    assert!(offset_of!(VectorView, truelength) == vector::OFF_TRUELEN);
    assert!(size_of::<VectorView>() == vector::OFF_DATA);
    assert!(offset_of!(NodeView, car) == node::OFF_CAR);
    assert!(offset_of!(NodeView, cdr) == node::OFF_CDR);
    assert!(offset_of!(NodeView, tag) == node::OFF_TAG);
    assert!(offset_of!(NodeView, aux) == node::OFF_AUX);
    assert!(size_of::<NodeView>() == node::SIZE);
    // a CHARSXP's hash is a u64 word (the u32 hash, widened) where ATTRIB
    // would be
    assert!(charsxp::OFF_HASH + size_of::<u64>() <= header::SIZE);
    // every body and field boundary is granule- or word-aligned
    assert!(header::SIZE.is_multiple_of(GRANULE) && node::SIZE.is_multiple_of(GRANULE));
    assert!(vector::OFF_DATA.is_multiple_of(WORD) && charsxp::OFF_DATA.is_multiple_of(WORD));
};
