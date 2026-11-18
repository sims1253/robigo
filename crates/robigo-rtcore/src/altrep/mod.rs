//! Package ALTREP classes (docs/g3.md §8: "methods where GNU R calls them,
//! behind `&mut Cx`, in GNU R's call order"; §2.4): the method tables of
//! GNU R's `altrep.c`, which the C API's `R_make_alt*_class` makes
//! (robigo-rtapi), and the dispatch of their methods to package C.
//!
//! A class is GNU R's class object: a RAWSXP whose data is the method
//! table, in the field order of GNU R's `altrep_methods_t` and its type's
//! extension ([`AltrepMethods`], [`AltvecMethods`], ...), its ATTRIB
//! `list3(class symbol, package symbol, base type)`. An entry is the
//! package's C function, or null for GNU R's default method, which robigo
//! runs itself. An object of the class is a node of the class's base type
//! with the ALTREP flag: data1 (CAR), data2 (CDR), the class (TAG), as
//! robigo-heap lays it out.
//!
//! A method is C: it may allocate, re-enter R and leave by an R error. It
//! runs as a native entry from a hidden record ([`Cx::call_native6`]),
//! with the object an argument C has seen.

#![allow(unsafe_code)]

pub mod own;

use crate::{Cx, RResult, Ref, Root};
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_heap::unwind::Ret6;
use std::ffi::c_void;

/// A method table entry: a C function, or null for GNU R's default.
pub type Method = *const c_void;

/// GNU R's `altrep_methods_t` (`ALTREP_METHODS`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltrepMethods {
    pub unserialize_ex: Method,
    pub unserialize: Method,
    pub serialized_state: Method,
    pub duplicate_ex: Method,
    pub duplicate: Method,
    pub coerce: Method,
    pub inspect: Method,
    pub length: Method,
}

/// GNU R's `altvec_methods_t` (`ALTVEC_METHODS`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltvecMethods {
    pub altrep: AltrepMethods,
    pub dataptr: Method,
    pub dataptr_or_null: Method,
    pub extract_subset: Method,
}

/// GNU R's `altinteger_methods_t` and `altreal_methods_t` (the same
/// fields).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltnumMethods {
    pub altvec: AltvecMethods,
    pub elt: Method,
    pub get_region: Method,
    pub is_sorted: Method,
    pub no_na: Method,
    pub sum: Method,
    pub min: Method,
    pub max: Method,
}

/// GNU R's `altlogical_methods_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltlogicalMethods {
    pub altvec: AltvecMethods,
    pub elt: Method,
    pub get_region: Method,
    pub is_sorted: Method,
    pub no_na: Method,
    pub sum: Method,
}

/// GNU R's `altraw_methods_t` and `altcomplex_methods_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltrawMethods {
    pub altvec: AltvecMethods,
    pub elt: Method,
    pub get_region: Method,
}

/// GNU R's `altstring_methods_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltstringMethods {
    pub altvec: AltvecMethods,
    pub elt: Method,
    pub set_elt: Method,
    pub is_sorted: Method,
    pub no_na: Method,
}

/// GNU R's `altlist_methods_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AltlistMethods {
    pub altvec: AltvecMethods,
    pub elt: Method,
    pub set_elt: Method,
}

/// The size of the method table of a class of base type `t`
/// (`make_altrep_class`'s types), or None: GNU R's "unsupported ALTREP
/// class".
pub fn methods_size(t: u8) -> Option<usize> {
    Some(match t {
        INTSXP | REALSXP => size_of::<AltnumMethods>(),
        LGLSXP => size_of::<AltlogicalMethods>(),
        RAWSXP | CPLXSXP => size_of::<AltrawMethods>(),
        STRSXP => size_of::<AltstringMethods>(),
        VECSXP => size_of::<AltlistMethods>(),
        _ => return None,
    })
}

impl Ref<'_> {
    /// The class's `altrep_methods_t` part, read: the entries every class
    /// has.
    fn altrep_methods(self) -> AltrepMethods {
        let c = self.altrep_class();
        assert!(c.sexptype() == RAWSXP, "robigo: an ALTREP class that is not a method table");
        // SAFETY: a class object is a RAWSXP of at least an
        // `altrep_methods_t` (`methods_size`), its data 16-aligned; read
        // by value, so no borrow outlives a method that rewrites it.
        unsafe { (c.sexp().std_data() as *const AltrepMethods).read() }
    }
}

/// `ALTREP_ERROR_IN_CLASS(msg, x)`'s message: `msg` with the class's and
/// package's names (`ALTREP_SERIALIZED_CLASS`'s symbols).
// R: altrep.c:ALTREP_ERROR_IN_CLASS
fn error_in_class(msg: &str, x: Ref<'_>) -> String {
    let c = x.altrep_class().attrib().and_then(|i| i.as_cons());
    let name = |r: Option<Ref<'_>>| {
        r.and_then(|s| s.as_sym()).map_or_else(String::new, |s| String::from_utf8_lossy(s.name_bytes()).into_owned())
    };
    let (cls, pkg) = (name(c.map(|c| c.car())), name(c.and_then(|c| c.cdr().as_cons()).map(|c| c.car())));
    format!("{msg} [class: {cls}, pkg: {pkg}]")
}

impl Cx {
    /// GNU R's `ALTREP_LENGTH(x)` of a package ALTREP object: its class's
    /// Length method, C that may allocate, re-enter R or leave by an
    /// error; GNU R's default (no method set) is an error.
    // R: altrep.c:ALTREP_LENGTH, altrep_Length_default
    pub fn altrep_length(&mut self, x: Root<'_>) -> RResult<usize> {
        let f = x.bind(self).altrep_methods().length;
        if f.is_null() {
            let msg = error_in_class("no ALTREP Length method defined", x.bind(self));
            return Err(self.error(&msg));
        }
        let w = x.bind(self).sexp().as_ptr() as usize;
        // SAFETY: `f` is the class's Length method, `R_xlen_t (*)(SEXP)`,
        // which the package set; the object is rooted by `x` for the call.
        let n = match unsafe { self.call_native6(f, [w, 0, 0, 0, 0, 0], 1, Ret6::Word) }? {
            crate::Native6::Word(n) => n as isize,
            _ => unreachable!(),
        };
        if n < 0 {
            return Err(self.error_call(None, "robigo: 'a negative ALTREP length' is not implemented yet"));
        }
        Ok(n as usize)
    }
}

/// A class object's base type (`ALTREP_CLASS_BASE_TYPE`): the third
/// element of its ATTRIB.
///
/// # Safety
/// `class` is a live class object made by [`methods_size`]'s caller, its
/// ATTRIB `list3(csym, psym, stype)`.
pub unsafe fn class_base_type(class: Sexp) -> u8 {
    // SAFETY: the caller's contract: a three-cell list whose third CAR is
    // an integer scalar.
    unsafe {
        let stype = class.attrib_raw().cdr().cdr().car();
        *(stype.std_data() as *const i32) as u8
    }
}
