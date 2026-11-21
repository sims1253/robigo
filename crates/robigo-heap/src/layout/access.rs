//! Where each field of an object lives: `Sexp`'s accessors for the
//! header, vector, node, symbol, promise and CHARSXP words, over
//! [`super::defs`]' offsets. With the definitions, the one place that knows
//! the layout: a layout variant (D8's vector header, D13's node sizes)
//! changes these two files and the allocator's size classes.

use super::*;
use crate::Sexp;
use crate::types::*;

impl Sexp {
    // -- header
    /// The SEXPTYPE.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn typ(self) -> u8 {
        // SAFETY: the first byte of every live object's header.
        unsafe { *self.0.add(header::OFF_TYPE) }
    }
    /// Change the type in place (GNU R's SET_TYPEOF).
    ///
    /// # Safety
    /// `self` must be live, and its allocation and contents must be valid
    /// for `t`: the collector and every accessor read the body by type. So
    /// a change stays within one body shape (LISTSXP to LANGSXP, one atomic
    /// vector type to another of the same element size or a smaller one). A
    /// vector made a node would have its length traced as a car; a CHARSXP
    /// made anything else would leave the cache containing it.
    #[inline(always)]
    pub unsafe fn set_typ(self, t: u8) {
        // SAFETY: the first header byte of a live object.
        unsafe {
            debug_assert!(
                self.retype_keeps_body(t),
                "robigo: SET_TYPEOF from {} to {t} outside one body shape",
                self.typ()
            );
            *self.0.add(header::OFF_TYPE) = t
        }
    }
    /// [`Sexp::set_typ`]'s contract, for its debug assertion: a standard
    /// vector becomes a vector type of the same element size or a smaller
    /// one, and neither is a CHARSXP; a node (or an ALTREP instance, a node
    /// whatever its type) becomes a node type of the same size, or a vector
    /// type when it has a full node's words (an ALTREP instance in the
    /// making: data1, data2, class and the 4th word).
    ///
    /// # Safety
    /// `self` must be live.
    unsafe fn retype_keeps_body(self, t: u8) -> bool {
        // SAFETY: the caller's contract: a live header.
        let (old, altrep) = unsafe { (self.typ(), self.has_flag(flags::ALTREP)) };
        if t == old {
            true
        } else if crate::is_vector_type(old) && !altrep {
            crate::is_vector_type(t) && old != CHARSXP && t != CHARSXP && elt_size(t) <= elt_size(old)
        } else if crate::is_vector_type(t) {
            node::size(old) == node::SIZE
        } else {
            node::same_body(old, t)
        }
    }
    /// The header flags (the `flags` module).
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn flags(self) -> u8 {
        // SAFETY: the flags byte of a live object.
        unsafe { *self.0.add(header::OFF_FLAGS) }
    }
    /// Overwrite the header flags.
    ///
    /// # Safety
    /// `self` must be live, and the new flags must describe its body:
    /// ALTREP, EXTDATA and SLOTS change how it is read, by accessors and by
    /// the collector. IMMORTAL and BARRIER are the heap's and must not
    /// change: IMMORTAL goes through [`Core::make_immortal`], which also
    /// counts it for the sweep, and BARRIER is the store barrier's state
    /// (clearing it on an immortal object skips the remembered set, and
    /// what is stored into it next is not kept alive). Debug builds check
    /// both.
    #[inline(always)]
    pub unsafe fn set_flags(self, f: u8) {
        // SAFETY: the flags byte of a live object.
        unsafe {
            debug_assert_eq!(
                (self.flags() ^ f) & (flags::IMMORTAL | flags::BARRIER),
                0,
                "robigo: set_flags changes IMMORTAL or BARRIER (the heap's bits)"
            );
            self.set_flags_raw(f)
        }
    }
    /// [`Sexp::set_flags`] for the heap's own bits too: `make_immortal`,
    /// the barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_flags`], and the heap's bookkeeping follows the
    /// change (the immortal count, the remembered set).
    #[inline(always)]
    pub(crate) unsafe fn set_flags_raw(self, f: u8) {
        // SAFETY: the flags byte of a live object.
        unsafe { *self.0.add(header::OFF_FLAGS) = f }
    }
    /// [`Sexp::set_flag`] for the heap's own bits too.
    ///
    /// # Safety
    /// As for [`Sexp::set_flags_raw`].
    #[inline(always)]
    pub(crate) unsafe fn set_flag_raw(self, f: u8, on: bool) {
        // SAFETY: the caller's contract.
        unsafe {
            let v = self.flags();
            self.set_flags_raw(if on { v | f } else { v & !f });
        }
    }
    /// Whether any of the bits `f` is set.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn has_flag(self, f: u8) -> bool {
        // SAFETY: `self` is live, all `flags` needs.
        unsafe { self.flags() & f != 0 }
    }
    /// Set or clear the bits `f`.
    ///
    /// # Safety
    /// As for [`Sexp::set_flags`].
    #[inline(always)]
    pub unsafe fn set_flag(self, f: u8, on: bool) {
        // SAFETY: `self` is live, and the caller vouches for the new flags.
        unsafe {
            let v = self.flags();
            self.set_flags(if on { v | f } else { v & !f });
        }
    }
    /// GNU R's OBJECT.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn is_object(self) -> bool {
        // SAFETY: `self` is live, all `has_flag` needs.
        unsafe { self.has_flag(flags::OBJECT) }
    }
    /// GNU R's SET_OBJECT.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn set_object(self, on: bool) {
        // SAFETY: `self` is live, and OBJECT does not change how it is read.
        unsafe { self.set_flag(flags::OBJECT, on) }
    }
    /// GNU R's TRACKREFS.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn tracks_refs(self) -> bool {
        // SAFETY: `self` is live, all these header reads need.
        unsafe { self.typ() == CLOSXP || !self.has_flag(flags::NO_TRACKREFS) }
    }
    /// GNU R's LEVELS.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn gp(self) -> u16 {
        // SAFETY: the gp word of a live, aligned object.
        unsafe { *(self.0.add(header::OFF_GP) as *const u16) }
    }
    /// GNU R's SETLEVELS.
    ///
    /// # Safety
    /// `self` must be live. On a CHARSXP the bits are part of the cache's
    /// bookkeeping: CACHED must stay true of the object (every cached
    /// CHARSXP is in the table, and the prune counts on it), and the encoding
    /// bits must stay the ones it was interned under.
    #[inline(always)]
    pub unsafe fn set_gp(self, v: u16) {
        // SAFETY: the gp word of a live, aligned object.
        unsafe { *(self.0.add(header::OFF_GP) as *mut u16) = v }
    }
    /// GNU R's REFCNT.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn refcnt(self) -> u16 {
        // SAFETY: the count word of a live, aligned object.
        unsafe { *(self.0.add(header::OFF_REFCNT) as *const u16) }
    }
    /// Overwrite the count.
    ///
    /// # Safety
    /// `self` must be live. Too low a count is a logic error with teeth: code
    /// that trusts `MAYBE_SHARED` then modifies a shared object in place.
    #[inline(always)]
    pub unsafe fn set_refcnt(self, v: u16) {
        // SAFETY: the count word of a live, aligned object.
        unsafe { *(self.0.add(header::OFF_REFCNT) as *mut u16) = v }
    }
    /// The debug, trace and step bits.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn extra(self) -> u16 {
        // SAFETY: the `extra` word of a live, aligned object.
        unsafe { *(self.0.add(header::OFF_EXTRA) as *const u16) }
    }
    /// Overwrite the `extra` bits (the type's map: [`extra`]).
    ///
    /// # Safety
    /// `self` must be live, and `v` must keep two kinds of bit as they are:
    /// [`extra::SEEN_BY_C`], which is sticky once set (a moving collection
    /// must never move what C may hold: [`Sexp::mark_seen_by_c`] sets it);
    /// and a promise's `PROMISE_TAG`, which says whether the collector reads
    /// PRVALUE as an object ([`Sexp::set_promise_unboxed`] and
    /// [`Sexp::set_promise_value`] change it with PRVALUE). Debug builds
    /// check both.
    #[inline(always)]
    pub unsafe fn set_extra(self, v: u16) {
        // SAFETY: the `extra` word of a live, aligned object.
        unsafe {
            let old = self.extra();
            debug_assert!(
                old & extra::SEEN_BY_C == 0 || v & extra::SEEN_BY_C != 0,
                "robigo: set_extra clears SEEN_BY_C (sticky)"
            );
            debug_assert!(
                self.typ() != PROMSXP || (old ^ v) & extra::promise::TAG_MASK == 0,
                "robigo: set_extra changes a promise's tag (set_promise_unboxed, set_promise_value)"
            );
            self.set_extra_raw(v)
        }
    }
    /// [`Sexp::set_extra`] for the bits the heap keeps in step with PRVALUE.
    ///
    /// # Safety
    /// `self` must be live; a promise's PRVALUE matches the tag written.
    #[inline(always)]
    pub(crate) unsafe fn set_extra_raw(self, v: u16) {
        // SAFETY: the `extra` word of a live, aligned object.
        unsafe { *(self.0.add(header::OFF_EXTRA) as *mut u16) = v }
    }
    /// GNU R's `PROMISE_TAG` of a promise (`IMMEDIATE_PROMISE_VALUES`,
    /// `Defn.h:34-39, 1268-1275`): 0, or the type (REALSXP, INTSXP, LGLSXP)
    /// of the unboxed value whose raw bits PRVALUE (`promise::OFF_VALUE`) holds. A tagged
    /// promise is evaluated, and the collector does not trace it
    /// ([`Sexp::trace_fields`]).
    ///
    /// # Safety
    /// `self` must be a live PROMSXP.
    #[inline(always)]
    pub unsafe fn promise_tag(self) -> u8 {
        // SAFETY: the caller's contract.
        unsafe { ((self.extra() & extra::promise::TAG_MASK) >> extra::promise::TAG_SHIFT) as u8 }
    }
    /// Set `PROMISE_TAG`. The caller writes PRVALUE to match in the same
    /// interval: raw bits for a tag, an object (its count dropped from the
    /// raw bits it replaces: none) for 0.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP; `t` is 0, REALSXP, INTSXP or LGLSXP;
    /// when the tag goes from 0 to a type, PRVALUE holds no object whose count
    /// is still needed (the caller dropped it).
    #[inline(always)]
    pub(crate) unsafe fn set_promise_tag(self, t: u8) {
        debug_assert!(matches!(t, 0 | REALSXP | INTSXP | LGLSXP));
        // SAFETY: the caller's contract.
        unsafe {
            let e = self.extra() & !extra::promise::TAG_MASK;
            self.set_extra_raw(e | ((t as u16) << extra::promise::TAG_SHIFT));
        }
    }
    /// Make PRVALUE the unboxed value `bits` of type `t` (REALSXP, INTSXP
    /// or LGLSXP): the tag and the bits in one step, so that the collector
    /// never sees a tag that disagrees with PRVALUE. GNU R's
    /// `SET_BNDCELL_DVAL`/`IVAL`/`LVAL` on a promise, with its tag.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP whose PRVALUE holds no object whose
    /// count is still needed (an unboxed value, R_UnboundValue, or an
    /// object whose count the caller dropped); `t` is one of the three.
    // R: Defn.h:SET_BNDCELL_DVAL, SET_BNDCELL_IVAL, SET_BNDCELL_LVAL
    #[inline(always)]
    pub unsafe fn set_promise_unboxed(self, t: u8, bits: u64) {
        debug_assert!(matches!(t, REALSXP | INTSXP | LGLSXP));
        // SAFETY: the caller's contract.
        unsafe {
            self.set_promise_bits(bits);
            self.set_promise_tag(t);
        }
    }
    /// The raw bits of PRVALUE (`promise::OFF_VALUE`): a tagged promise's unboxed value.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP.
    #[inline(always)]
    pub unsafe fn promise_bits(self) -> u64 {
        // SAFETY: PRVALUE is a node word.
        unsafe { *(self.word(promise::OFF_VALUE) as *const u64) }
    }
    /// Make PRVALUE (`promise::OFF_VALUE`) the object `v` (or R_UnboundValue), whatever it
    /// held: GNU R's `SET_PRVALUE` on a promise, with `CLEAR_BNDCELL_TAG`
    /// first when the value was unboxed (`PROMISE_TAG`). Raw bits are no
    /// object, so no count is dropped for them (and they are never read as
    /// a pointer); an object it held loses its count as any store's does.
    /// `v` is counted and the store takes the barrier.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP and `v` a live object of its heap.
    // R: memory.c:SET_PRVALUE, Defn.h:CLEAR_BNDCELL_TAG
    #[inline]
    pub unsafe fn set_promise_value(self, v: Sexp) {
        // SAFETY: the caller's contract; PRVALUE is a node field.
        unsafe {
            if self.promise_tag() != 0 {
                self.set_promise_tag(0);
                // a null word: the store below drops no count for it
                self.set_sexp_at(promise::OFF_VALUE, Sexp::NULL);
            }
            self.store(self.field(promise::OFF_VALUE), v);
        }
    }
    /// Write raw bits into PRVALUE (`promise::OFF_VALUE`), touching no count: the store of
    /// GNU R's `SET_BNDCELL_DVAL`/`IVAL`/`LVAL` on a promise.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP whose tag is (or is being set to) a
    /// type, and whose PRVALUE held no counted object (or the caller dropped it).
    #[inline(always)]
    pub(crate) unsafe fn set_promise_bits(self, bits: u64) {
        // SAFETY: the caller's contract.
        unsafe { *(self.word(promise::OFF_VALUE) as *mut u64) = bits }
    }
    /// Mark the object as handed to C ([`extra::SEEN_BY_C`]): from now on a
    /// moving collection leaves it where it is.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn mark_seen_by_c(self) {
        // SAFETY: the caller's contract. An immortal object never moves, so
        // it needs no mark, and is not written (H5: shared objects are
        // immutable).
        unsafe {
            if !self.has_flag(flags::IMMORTAL) {
                self.set_extra(self.extra() | extra::SEEN_BY_C)
            }
        }
    }
    /// Whether the object was ever handed to C ([`extra::SEEN_BY_C`]).
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn seen_by_c(self) -> bool {
        // SAFETY: the caller's contract.
        unsafe { self.extra() & extra::SEEN_BY_C != 0 }
    }
    /// The attribute pairlist (CHARSXPs have none: their slot contains the hash).
    ///
    /// # Safety
    /// `self` must be live. On a CHARSXP the result is the hash, not an
    /// object, and must not be dereferenced or stored as one.
    #[inline(always)]
    pub unsafe fn attrib_raw(self) -> Sexp {
        // SAFETY: ATTRIB is part of every object's header.
        unsafe { self.sexp_at(header::OFF_ATTRIB) }
    }
    /// Store the attribute pairlist, with no reference count and no barrier.
    ///
    /// # Safety
    /// `self` must be live and not a CHARSXP: this would overwrite its hash,
    /// and the cache could no longer find it or place it when it grows. `v`
    /// must be live (a pairlist or R_NilValue): every collection traces
    /// it. On an immortal `self` the store also needs the barrier
    /// ([`Sexp::store`]), or nothing keeps `v` alive.
    #[inline(always)]
    pub(crate) unsafe fn set_attrib_raw(self, v: Sexp) {
        // SAFETY: ATTRIB is part of every object's header; what `v` must
        // be is the caller's contract.
        unsafe { self.set_sexp_at(header::OFF_ATTRIB, v) }
    }
    /// GNU R's ALTREP.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn is_altrep(self) -> bool {
        // SAFETY: `self` is live, all `has_flag` needs.
        unsafe { self.has_flag(flags::ALTREP) }
    }

    // -- vectors
    /// The length of a standard vector.
    ///
    /// # Safety
    /// `self` must be a live vector or CHARSXP that is not ALTREP. On an
    /// ALTREP object the length word is `data1`, a pointer that reads as a nonsense
    /// length.
    #[inline(always)]
    pub unsafe fn std_len(self) -> isize {
        // SAFETY: the caller's contract puts a length at `vector::OFF_LEN`.
        unsafe { *(self.word(vector::OFF_LEN) as *const isize) }
    }
    /// Store the length of a standard vector.
    ///
    /// # Safety
    /// As for [`Sexp::std_len`], and `n` must not exceed the capacity of the
    /// payload: the collector and every reader trust it, and a STRSXP or
    /// VECSXP has its first `n` elements traced as objects.
    #[inline(always)]
    pub unsafe fn set_std_len(self, n: isize) {
        // SAFETY: the caller's contract puts a length at `vector::OFF_LEN`.
        unsafe { *(self.word(vector::OFF_LEN) as *mut isize) = n }
    }
    /// GNU R's TRUELENGTH.
    ///
    /// # Safety
    /// As for [`Sexp::std_len`]: the true length's word is `data2` or a node field otherwise.
    #[inline(always)]
    pub unsafe fn truelen(self) -> isize {
        // SAFETY: the caller's contract puts a true length at `vector::OFF_TRUELEN`.
        unsafe { *(self.word(vector::OFF_TRUELEN) as *const isize) }
    }
    /// GNU R's SET_TRUELENGTH.
    ///
    /// # Safety
    /// As for [`Sexp::std_len`].
    #[inline(always)]
    pub unsafe fn set_truelen(self, n: isize) {
        // SAFETY: the caller's contract puts a true length at `vector::OFF_TRUELEN`.
        unsafe { *(self.word(vector::OFF_TRUELEN) as *mut isize) = n }
    }
    /// The payload of a non-ALTREP vector (inline, or EXTDATA's pointer).
    ///
    /// # Safety
    /// As for [`Sexp::std_len`]: an ALTREP object's payload word is its class, and a
    /// node has no payload. The pointer stays valid while the object is
    /// live and until the next collection, which may move the object
    /// ([`Sexp`]'s address rule) unless it is immortal or handed to C; an
    /// EXTDATA pointer is as good as its owner's promise to
    /// [`Core::alloc_extdata`].
    #[inline(always)]
    pub unsafe fn std_data(self) -> *mut u8 {
        // SAFETY: the caller's contract makes this a standard vector, whose
        // payload starts at `data_off` or, for EXTDATA, is stored there.
        unsafe {
            if self.has_flag(flags::EXTDATA) {
                *(self.word(extdata::OFF_PTR) as *const *mut u8)
            } else {
                self.0.add(data_off(self.typ()))
            }
        }
    }

    /// The payload of a vector that keeps it inline: [`Sexp::std_data`]
    /// without the EXTDATA test, for an object whose maker is known (only
    /// [`Core::alloc_extdata`] makes EXTDATA vectors, for the C API).
    ///
    /// # Safety
    /// As for [`Sexp::std_data`], and `self` was not made by
    /// [`Core::alloc_extdata`].
    #[inline(always)]
    pub unsafe fn inline_data(self) -> *mut u8 {
        // SAFETY: the caller's contract: a standard vector with its payload
        // at `data_off`.
        unsafe {
            debug_assert!(!self.has_flag(flags::EXTDATA), "robigo: inline_data of an EXTDATA vector");
            self.0.add(data_off(self.typ()))
        }
    }

    // -- nodes
    /// GNU R's CAR, and the fields that share its slot.
    ///
    /// # Safety
    /// `self` must be a live object with node fields: a node or an ALTREP
    /// object. Not a vector, where the car's word is the length. The car of
    /// an EXTPTRSXP or a primitive is not an object.
    #[inline(always)]
    pub unsafe fn car(self) -> Sexp {
        // A symbol's CAR is the arena's copy of its name, which no R value
        // may hold: its PRINTNAME is the isolate's (`Core::printname`), its
        // bytes `sym_name_bytes`.
        // SAFETY: the caller's contract (a live object).
        debug_assert!(!unsafe { self.is_symbol() }, "robigo: the CAR of a symbol (its PRINTNAME is the isolate's)");
        // SAFETY: the caller's contract puts a field at `node::OFF_CAR`.
        unsafe { self.sexp_at(node::OFF_CAR) }
    }
    /// GNU R's CDR, and the fields that share its slot.
    ///
    /// # Safety
    /// As for [`Sexp::car`]: on a vector the cdr's word is the true length.
    #[inline(always)]
    pub unsafe fn cdr(self) -> Sexp {
        // SAFETY: the caller's contract puts a field at `node::OFF_CDR`.
        unsafe { self.sexp_at(node::OFF_CDR) }
    }
    /// GNU R's TAG, and the fields that share its slot.
    ///
    /// # Safety
    /// As for [`Sexp::car`]: on a vector the tag's word is payload.
    #[inline(always)]
    pub unsafe fn tag(self) -> Sexp {
        // A symbol has no TAG: INTERNAL is per isolate (rtcore's `SymCell`).
        // SAFETY: the caller's contract (a live object).
        debug_assert!(!unsafe { self.is_symbol() }, "robigo: the TAG of a symbol");
        // SAFETY: the caller's contract puts a field at `node::OFF_TAG`.
        unsafe { self.sexp_at(node::OFF_TAG) }
    }
    /// Store the car, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::car`]. `v` must be live unless `self`'s car is not an
    /// object (EXTPTRSXP, BUILTINSXP, SPECIALSXP), and a live `v` needs the
    /// barrier ([`Sexp::store`]) if `self` is immortal, or nothing keeps it
    /// alive. Reference counts are the caller's to keep.
    #[inline(always)]
    pub(crate) unsafe fn set_car_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at `node::OFF_CAR` and vouches for `v`.
        unsafe { self.set_sexp_at(node::OFF_CAR, v) }
    }
    /// Store the cdr, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_car_raw`], except that `v` must always be live.
    #[inline(always)]
    pub(crate) unsafe fn set_cdr_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at `node::OFF_CDR` and vouches for `v`.
        unsafe { self.set_sexp_at(node::OFF_CDR, v) }
    }
    /// Store the tag, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_car_raw`], except that `v` must always be live.
    #[inline(always)]
    pub(crate) unsafe fn set_tag_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at `node::OFF_TAG` and vouches for `v`.
        unsafe { self.set_sexp_at(node::OFF_TAG, v) }
    }

    /// The 4th node word ([`node::OFF_AUX`]).
    ///
    /// # Safety
    /// `self` must be a live node made by [`Core::alloc_node`] (not a vector).
    #[inline(always)]
    pub unsafe fn aux(self) -> Sexp {
        // SAFETY: the caller's contract puts the 4th word at `node::OFF_AUX`.
        unsafe { self.sexp_at(node::OFF_AUX) }
    }

    /// A symbol's name, as bytes: the arena's own copy (H5), which no R value
    /// holds. A PRINTNAME (a CHARSXP R code may see) is an isolate's
    /// (`Core::printname`).
    ///
    /// # Safety
    /// `self` must be a live SYMSXP, and the bytes are read while it is (a
    /// symbol is immortal).
    #[inline(always)]
    pub unsafe fn sym_name_bytes<'a>(self) -> &'a [u8] {
        // SAFETY: the caller's contract: a symbol's CAR is a CHARSXP (the
        // arena's name, or the shared "").
        unsafe { self.sexp_at(sym::OFF_PRINTNAME).char_bytes() }
    }
    /// Whether a symbol's name is ASCII: the ASCII bit of its PRINTNAME in
    /// every isolate (each makes it from the same bytes), read from the
    /// arena's name (one load less than the isolate's PRINTNAME).
    ///
    /// # Safety
    /// `self` must be a live SYMSXP.
    #[inline(always)]
    pub unsafe fn sym_name_is_ascii(self) -> bool {
        // SAFETY: as for `sym_name_bytes`.
        unsafe { self.sexp_at(sym::OFF_PRINTNAME).gp() & crate::gp::charsxp::ASCII != 0 }
    }
    /// A symbol's id ([`sym::OFF_ID`]): the process's, dense, in the order
    /// the symbol arena made its symbols (`arena.rs`; H5).
    ///
    /// # Safety
    /// `self` must be a live SYMSXP.
    #[inline(always)]
    pub unsafe fn sym_id(self) -> u32 {
        // SAFETY: the caller's contract: a symbol's 4th word holds its id in
        // the low half (little or big endian, read as a whole word).
        unsafe { *(self.word(sym::OFF_ID) as *const u64) as u32 }
    }
    /// GNU R's `R_Newhashpjw` of a symbol's name ([`sym_hash`]), stored
    /// when the symbol was made ([`sym::OFF_ID`]).
    ///
    /// # Safety
    /// `self` must be a live SYMSXP.
    #[inline(always)]
    pub unsafe fn sym_hash(self) -> u32 {
        // SAFETY: as for `sym_id`: the high half of the same word.
        unsafe { (*(self.word(sym::OFF_ID) as *const u64) >> 32) as u32 }
    }

    /// The field at byte offset `off`, as [`Sexp::store`] takes it.
    ///
    /// # Safety
    /// `self` must be live and `off` inside its allocation.
    #[inline(always)]
    pub(crate) unsafe fn field(self, off: usize) -> *mut Sexp {
        // SAFETY: the caller's contract.
        unsafe { self.word(off) as *mut Sexp }
    }
    /// GNU R's SET_ATTRIB store (without its checks).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` is not a CHARSXP and `v` is a
    /// pairlist or R_NilValue.
    #[inline(always)]
    pub unsafe fn set_attrib(self, v: Sexp) {
        // SAFETY: the caller's contract (a live object).
        debug_assert!(!unsafe { self.is_symbol() }, "robigo: a store into a symbol");
        // SAFETY: the attribute slot of every non-CHARSXP header.
        unsafe { self.store(self.field(header::OFF_ATTRIB), v) }
    }
    /// A symbol is immutable once installed (H5): the barriered node stores
    /// never take one.
    ///
    /// # Safety
    /// `self` is a live object.
    #[inline(always)]
    unsafe fn is_symbol(self) -> bool {
        // SAFETY: the caller's contract.
        unsafe { self.typ() == crate::types::SYMSXP }
    }
    /// GNU R's SETCAR store (also SET_PRVALUE, SET_FORMALS, ALTREP data1).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` has node fields and its car is an
    /// object (not an EXTPTRSXP, BUILTINSXP or SPECIALSXP).
    #[inline(always)]
    pub unsafe fn set_car(self, v: Sexp) {
        // SAFETY: the caller's contract (a live object).
        debug_assert!(!unsafe { self.is_symbol() }, "robigo: a store into a symbol");
        // SAFETY: the caller's contract puts a traced field at `node::OFF_CAR`.
        unsafe { self.store(self.field(node::OFF_CAR), v) }
    }
    /// GNU R's SETCDR store.
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` has node fields.
    #[inline(always)]
    pub unsafe fn set_cdr(self, v: Sexp) {
        // SAFETY: the caller's contract (a live object).
        debug_assert!(!unsafe { self.is_symbol() }, "robigo: a store into a symbol");
        // SAFETY: the caller's contract puts a traced field at `node::OFF_CDR`.
        unsafe { self.store(self.field(node::OFF_CDR), v) }
    }
    /// GNU R's SET_TAG store.
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` has node fields.
    #[inline(always)]
    pub unsafe fn set_tag(self, v: Sexp) {
        // SAFETY: the caller's contract (a live object).
        debug_assert!(!unsafe { self.is_symbol() }, "robigo: a store into a symbol");
        // SAFETY: the caller's contract puts a traced field at `node::OFF_TAG`.
        unsafe { self.store(self.field(node::OFF_TAG), v) }
    }
    /// Store the 4th node word.
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` is as for [`Sexp::aux`].
    #[inline(always)]
    pub unsafe fn set_aux(self, v: Sexp) {
        // SAFETY: the caller's contract puts the traced 4th word at `node::OFF_AUX`.
        unsafe { self.store(self.field(node::OFF_AUX), v) }
    }
    /// GNU R's SET_VECTOR_ELT / SET_STRING_ELT store (without the checks).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` is a standard (not ALTREP) STRSXP,
    /// VECSXP, EXPRSXP or WEAKREFSXP and `i` is below its length.
    #[inline(always)]
    pub unsafe fn set_elt(self, i: usize, v: Sexp) {
        // SAFETY: the caller's contract: element `i` of a pointer payload.
        unsafe { self.store((self.std_data() as *mut Sexp).add(i), v) }
    }
    /// Store an external pointer's address (its car, which is not an
    /// object): no count and no barrier, as GNU R's R_SetExternalPtrAddr.
    ///
    /// # Safety
    /// `self` must be a live EXTPTRSXP.
    #[inline(always)]
    pub unsafe fn set_extptr_addr(self, p: *mut u8) {
        // SAFETY: an EXTPTRSXP's address is an untraced word.
        unsafe { self.set_sexp_at(extptr::OFF_ADDR, Sexp(p)) }
    }

    // -- CHARSXP
    /// The hash stored in a CHARSXP's header (the attribute slot).
    ///
    /// # Safety
    /// `self` must be a live CHARSXP. On anything else this reads the low
    /// half of the attribute pointer.
    #[inline(always)]
    pub unsafe fn char_hash(self) -> u32 {
        // SAFETY: the hash word is in the header of every object, and 8-byte aligned.
        unsafe { *(self.0.add(charsxp::OFF_HASH) as *const u32) }
    }
    /// The bytes of a CHARSXP (without the terminating NUL).
    ///
    /// # Safety
    /// `self` must be a live CHARSXP that stays live and in place for `'a`.
    /// Nothing ties `'a` to the object: the caller picks it, and the slice
    /// dangles as soon as the CHARSXP is swept, or moved by a collection
    /// ([`Sexp`]'s address rule: a CHARSXP that is immortal or handed to C
    /// stays put). If it is not rooted, that is the next allocation. The
    /// bytes must not change for `'a` either.
    #[inline(always)]
    pub unsafe fn char_bytes<'a>(self) -> &'a [u8] {
        // SAFETY: a CHARSXP has `std_len` initialized bytes at `charsxp::OFF_DATA`, and
        // the caller keeps them there for `'a`.
        unsafe { std::slice::from_raw_parts(self.0.add(charsxp::OFF_DATA), self.std_len() as usize) }
    }
}
