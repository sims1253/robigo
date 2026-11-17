//! The object layout (docs/g3.md §8, the appendix's layout row): every
//! per-type size, offset and header bit, in one module. The rest of the
//! heap, the rt crates and the C that reads objects (robigo-rtapi's
//! `api.c`, robigo-rtcore's `Defn.h`, through the generated
//! `robigo_layout.h`) read them from here, so a measured layout variant
//! (X3's D8 × D13) is a change inside this module and the allocator's size
//! classes (`heap.rs`), and nothing else.
//!
//! - `defs`: the offsets per body ([`header`], [`vector`], [`charsxp`],
//!   [`extdata`], [`node`] and its per-type views [`sym`], [`env`],
//!   [`promise`], [`extptr`], [`altrep`], [`slots`]), the sizes
//!   ([`GRANULE`], [`node::size`], [`vector_bytes`], [`charsxp_bytes`]),
//!   and the header bits: [`flags`] (one map), and per-type maps for GNU
//!   R's [`gp`] and robigo's [`extra`] bits, each checked at compile time
//!   for overlaps. build.rs includes this file to write `robigo_layout.h`
//!   from [`TABLE`].
//! - `access`: `Sexp`'s field accessors over those offsets.

mod access;
mod defs;
pub use defs::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    /// X3's cell h24-nT: D8's 24-byte vector header (a CHARSXP keeps its
    /// 32), with D13's 8-byte granules and per-type node sizes. A variant
    /// changes these with the definitions.
    #[test]
    fn the_layout_is_h24_nt() {
        assert_eq!((header::SIZE, vector::OFF_LEN, vector::OFF_DATA), (16, 16, 24));
        assert_eq!((charsxp::OFF_HASH, charsxp::OFF_TRUELEN, charsxp::OFF_DATA), (8, 24, 32));
        assert_eq!((node::OFF_CAR, node::OFF_CDR, node::OFF_TAG, node::OFF_AUX, node::SIZE), (16, 24, 32, 40, 48));
        assert_eq!((extdata::OFF_PTR, extdata::OFF_OWNER, extdata::OFF_RELEASE, extdata::SIZE), (24, 32, 40, 48));
        assert_eq!((slots::OFF_CALLER, slots::OFF_META, slots::OFF_SLOTS, slots::SLOT_SIZE), (48, 56, 64, 32));
        assert_eq!(GRANULE, 8);
        for t in [NILSXP, SYMSXP, CLOSXP, ENVSXP, LANGSXP, BCODESXP, OBJSXP, INTSXP, VECSXP] {
            assert_eq!(node::size(t), 48, "SEXPTYPE {t}");
            assert!(node::has_aux(t));
        }
        for t in [LISTSXP, DOTSXP, PROMSXP, EXTPTRSXP] {
            assert_eq!(node::size(t), 40, "SEXPTYPE {t}");
            assert!(!node::has_aux(t));
        }
    }

    /// The allocation sizes are the header plus the payload in whole
    /// granules, as the allocator was written against.
    #[test]
    fn allocation_sizes_round_to_granules() {
        for (t, n) in [(REALSXP, 0), (REALSXP, 1), (INTSXP, 3), (CPLXSXP, 2), (RAWSXP, 17), (VECSXP, 5)] {
            let want = (vector::OFF_DATA + n * elt_size(t)).div_ceil(GRANULE) * GRANULE;
            assert_eq!(vector_bytes(t, n), Some(want), "SEXPTYPE {t}, length {n}");
        }
        assert_eq!(vector_bytes(REALSXP, usize::MAX / 4), None);
        assert_eq!(vector_bytes(RAWSXP, usize::MAX - 8), None);
        assert_eq!(charsxp_bytes(0), 40);
        assert_eq!(charsxp_bytes(15), 48);
        assert_eq!(charsxp_bytes(16), 56);
        assert_eq!(data_off(CHARSXP), charsxp::OFF_DATA);
        assert_eq!(data_off(REALSXP), vector::OFF_DATA);
        assert_eq!(slots::frame_bytes(0), slots::OFF_SLOTS);
        // a scalar is 32 bytes (h32: 48), 40 with a TRUELENGTH word; an
        // empty vector is its header
        assert_eq!((vector_bytes(REALSXP, 1), vector_bytes_tl(REALSXP, 1)), (Some(32), Some(40)));
        assert_eq!((vector_bytes(RAWSXP, 0), vector_bytes_tl(RAWSXP, 0)), (Some(24), Some(32)));
        assert_eq!(vector_bytes_tl(CHARSXP, 5), vector_bytes(CHARSXP, 5));
        assert_eq!(vector_bytes_tl(REALSXP, usize::MAX / 8), None);
    }

    /// The no-overlap check rejects a map whose bits overlap.
    #[test]
    fn overlapping_bits_are_refused() {
        assert!(super::defs::disjoint_for_tests(&[1, 2, 4, 0x18]));
        assert!(!super::defs::disjoint_for_tests(&[1, 2, 3]));
        assert!(!super::defs::disjoint_for_tests(&[extra::SEEN_BY_C as u64, extra::SEEN_BY_C as u64]));
    }

    /// The header's table names each value once, under a C identifier.
    #[test]
    fn the_table_names_are_unique_c_identifiers() {
        let mut seen = std::collections::HashSet::new();
        for (name, _) in TABLE {
            assert!(seen.insert(name), "{name} twice");
            assert!(name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'), "{name}");
        }
    }

    /// The `#[repr(C)]` views read the words the accessors do.
    #[test]
    fn the_views_alias_the_accessors() {
        let mut w = [0u64; 6];
        let p = w.as_mut_ptr() as *mut u8;
        let x = crate::Sexp(p);
        // SAFETY: `w` is 48 bytes, 8-aligned, a node's words; the accessors
        // read only those.
        // (read through raw pointers: a reference held across the
        // accessors' writes would be invalidated by them)
        unsafe {
            *(p as *mut u64) = header::first_word(CLOSXP);
            let h = p as *const HeaderView;
            assert_eq!(((*h).typ, (*h).flags, (*h).gp, (*h).refcnt, (*h).extra), (CLOSXP, 0, 0, 0, 0));
            x.set_typ(LANGSXP);
            x.set_gp(0x1234);
            x.set_extra(0x0042);
            let v = p as *const NodeView;
            assert_eq!(((*v).header.typ, (*v).header.gp, (*v).header.extra), (LANGSXP, 0x1234, 0x0042));
            x.set_car_raw(crate::Sexp(std::ptr::without_provenance_mut(16)));
            x.set_tag_raw(crate::Sexp(std::ptr::without_provenance_mut(32)));
            assert_eq!(((*v).car as usize, (*v).tag as usize), (16, 32));
            let vv = p as *const VectorView;
            *(p as *mut u64) = header::first_word(INTSXP);
            x.set_std_len(7);
            assert_eq!(((*vv).header.typ, (*vv).length), (INTSXP, 7));
            // a vector without the word: its TRUELENGTH is 0, which it keeps
            x.set_truelen(0);
            assert_eq!(x.truelen(), 0);
            *(p as *mut u64) = header::first_word(CHARSXP);
            x.set_truelen(9);
            assert_eq!((x.truelen(), *(p.add(charsxp::OFF_TRUELEN) as *const isize)), (9, 9));
        }
    }

    /// `set_typ`'s debug assertion holds it to its contract: retags within
    /// one body shape pass, a widening one does not.
    #[cfg(debug_assertions)]
    #[test]
    fn retags_stay_within_one_body_shape() {
        let mut w = [0u64; 6];
        let p = w.as_mut_ptr();
        let x = crate::Sexp(p as *mut u8);
        // SAFETY: `w` is a node's words; the retags read only the header.
        unsafe {
            for (from, to) in
                [(LISTSXP, DOTSXP), (LANGSXP, CLOSXP), (INTSXP, LGLSXP), (REALSXP, INTSXP), (LANGSXP, INTSXP)]
            {
                *p = header::first_word(from);
                x.set_typ(to);
            }
            // D13's nT: a 40-byte cell has no room for a call's 4th word
            *p = header::first_word(LISTSXP);
            assert!(std::panic::catch_unwind(|| x.set_typ(LANGSXP)).is_err(), "a pairlist cell is smaller than a call");
            assert!(std::panic::catch_unwind(|| x.set_typ(INTSXP)).is_err(), "an ALTREP instance needs a 4th word");
            *p = header::first_word(RAWSXP);
            let widen = std::panic::catch_unwind(|| x.set_typ(CPLXSXP));
            assert!(widen.is_err(), "RAWSXP to CPLXSXP reads past the payload");
            *p = header::first_word(CHARSXP);
            assert!(std::panic::catch_unwind(|| x.set_typ(RAWSXP)).is_err(), "a CHARSXP keeps its type");
            *p = header::first_word(INTSXP);
            assert!(std::panic::catch_unwind(|| x.set_typ(LISTSXP)).is_err(), "a vector's length is no car");
        }
    }
}
