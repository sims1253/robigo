//! altrep.c's class API (docs/g3.md §8, §15.3 E6): the class constructors
//! (`R_make_alt*_class`, with GNU R's class registry), `R_new_altrep`, the
//! method setters and `R_altrep_inherits`. The tables and the dispatch of
//! their methods are robigo-rtcore's (`altrep`); data1, data2 and the class
//! are read by api.c.

#![allow(unsafe_code)]

use crate::entry;
use crate::memory::{alloc, safepoint};
use robigo_rtcore::altrep::*;
use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, Global, RResult, Ref, Root, Scope, Sexp};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::offset_of;

/// GNU R's `Registry`: a preserved pairlist whose CDR lists the classes'
/// entries, `list4(class, psym, stype, iptr)` tagged by the class symbol.
struct Registry(Global);

/// The registry's head cell, made at the first class (`if (Registry ==
/// NULL)`).
fn registry<'s>(cx: &mut Cx, s: Scope<'s>) -> RResult<Root<'s>> {
    if let Some(r) = cx.ext::<Registry>() {
        let head = r.0.bind(cx);
        return Ok(s.root(cx, head));
    }
    let head = s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let nil = cx.nil().sexp();
            // SAFETY: R_NilValue is live and immortal.
            unsafe { Ok(Ref::from_sexp(cx.alloc_node_raw(LISTSXP, nil, nil, nil)?, cx)) }
        })
    })?;
    let g = cx.global(head.bind(cx));
    cx.set_ext(Registry(g));
    Ok(head)
}

/// `install(name)` of a C string.
fn install<'s>(cx: &mut Cx, s: Scope<'s>, name: *const c_char) -> RResult<Root<'s>> {
    // SAFETY: C's contract: a NUL-terminated string.
    let bytes = unsafe { CStr::from_ptr(name) }.to_bytes().to_vec();
    s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(&bytes)?.into())))
}

/// A list cell `CONS(car, cdr)`, tagged `tag`.
fn cons<'s>(cx: &mut Cx, s: Scope<'s>, car: Root<'_>, cdr: Root<'_>, tag: Root<'_>) -> RResult<Root<'s>> {
    s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let (a, d, t) = (car.bind(cx).sexp(), cdr.bind(cx).sexp(), tag.bind(cx).sexp());
            // SAFETY: rooted objects of this isolate.
            unsafe { Ok(Ref::from_sexp(cx.alloc_node_raw(LISTSXP, a, d, t)?, cx)) }
        })
    })
}

/// `RegisterClass(class, type, cname, pname, dll)`: the class entered in
/// the registry (an entry of the same names updated), and its ATTRIB set
/// to `list3(csym, psym, stype)`.
// R: altrep.c:RegisterClass
fn register_class(
    cx: &mut Cx,
    s: Scope<'_>,
    class: Root<'_>,
    t: u8,
    cname: *const c_char,
    pname: *const c_char,
    dll: *mut c_void,
) -> RResult<()> {
    let head = registry(cx, s)?;
    let csym = install(cx, s, cname)?;
    let psym = install(cx, s, pname)?;
    let stype = s.capture(cx, |cx, o| cx.build_scalar(o, t as i32))?;
    let nil = s.root(cx, cx.nil());
    let iptr = s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let nil = cx.nil().sexp();
            // SAFETY: an EXTPTRSXP's car is its address (untraced).
            unsafe {
                let x = cx.alloc_node_raw(EXTPTRSXP, nil, nil, nil)?;
                x.set_extptr_addr(dll.cast());
                Ok(Ref::from_sexp(x, cx))
            }
        })
    })?;
    // LookupClassEntry(csym, psym)
    let found = {
        let (c, p) = (csym.bind(cx), psym.bind(cx));
        let mut chain = head.bind(cx).as_cons().unwrap().cdr();
        let mut found = None;
        while let Some(cell) = chain.as_cons() {
            let entry = cell.car().as_cons().unwrap();
            if entry.tag() == c && entry.cdr().as_cons().is_some_and(|e| e.car() == p) {
                found = Some(entry.as_ref());
                break;
            }
            chain = cell.cdr();
        }
        found.map(|e| s.root(cx, e))
    };
    match found {
        None => {
            let l4 = cons(cx, s, iptr, nil, nil)?;
            let l3 = cons(cx, s, stype, l4, nil)?;
            let l2 = cons(cx, s, psym, l3, nil)?;
            let entry = cons(cx, s, class, l2, csym)?;
            let rest = s.root(cx, head.bind(cx).as_cons().unwrap().cdr());
            let cell = cons(cx, s, entry, rest, nil)?;
            cx.set_cdr(head, cell);
        }
        Some(entry) => {
            cx.set_car(entry, class);
            let c3 = s.root(cx, entry.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr());
            cx.set_car(c3, stype);
            let c4 = s.root(cx, c3.bind(cx).as_cons().unwrap().cdr());
            cx.set_car(c4, iptr);
        }
    }
    // SET_ALTREP_CLASS_SERIALIZED_CLASS(class, csym, psym, stype)
    let a3 = cons(cx, s, stype, nil, nil)?;
    let a2 = cons(cx, s, psym, a3, nil)?;
    let a1 = cons(cx, s, csym, a2, nil)?;
    cx.set_attrib_list(class, a1);
    Ok(())
}

/// `make_altrep_class(type, cname, pname, dll)`: a method table of GNU R's
/// defaults (all null here: robigo runs them itself), registered.
// R: altrep.c:make_altrep_class
fn make_class(
    cx: &mut Cx,
    s: Scope<'_>,
    t: u8,
    cname: *const c_char,
    pname: *const c_char,
    dll: *mut c_void,
) -> RResult<Sexp> {
    let Some(size) = methods_size(t) else {
        return Err(cx.error("unsupported ALTREP class"));
    };
    safepoint(cx)?;
    // a RAWSXP with every entry null
    let class = s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let v = cx.alloc_vector_raw(RAWSXP, size)?;
            // SAFETY: a fresh vector of this isolate, `size` bytes of plain
            // data the heap left uninitialized.
            unsafe {
                std::ptr::write_bytes(v.std_data(), 0, size);
                Ok(Ref::from_sexp(v, cx))
            }
        })
    })?;
    register_class(cx, s, class, t, cname, pname, dll)?;
    Ok(class.bind(cx).sexp())
}

entry! {
    /// `R_make_altinteger_class(cname, pname, dll)`: an `R_altrep_class_t`
    /// is a struct of one SEXP, returned as the SEXP.
    // R: altrep.c:R_make_altinteger_class
    [3] fn R_make_altinteger_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, INTSXP, cname, pname, dll)
    }
    // R: altrep.c:R_make_altreal_class
    [3] fn R_make_altreal_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, REALSXP, cname, pname, dll)
    }
    // R: altrep.c:R_make_altlogical_class
    [3] fn R_make_altlogical_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, LGLSXP, cname, pname, dll)
    }
    // R: altrep.c:R_make_altraw_class
    [3] fn R_make_altraw_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, RAWSXP, cname, pname, dll)
    }
    // R: altrep.c:R_make_altcomplex_class
    [3] fn R_make_altcomplex_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, CPLXSXP, cname, pname, dll)
    }
    // R: altrep.c:R_make_altstring_class
    [3] fn R_make_altstring_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, STRSXP, cname, pname, dll)
    }
    // R: altrep.c:R_make_altlist_class
    [3] fn R_make_altlist_class(cname: *const c_char, pname: *const c_char, dll: *mut c_void) -> Sexp => |cx, s| {
        make_class(cx, s, VECSXP, cname, pname, dll)
    }

    /// `R_new_altrep(class, data1, data2)`: `CONS(data1, data2)` of the
    /// class's base type, the ALTREP bit set and the class its TAG.
    // R: altrep.c:R_new_altrep
    [3] fn R_new_altrep(class: Sexp, data1: Sexp, data2: Sexp) -> Sexp => |cx, s| {
        // SAFETY: a class R_make_alt*_class made (C's contract).
        let t = unsafe { class_base_type(class.bind(cx).sexp()) };
        alloc(cx, s, |cx| {
            let (c, a, d) = (class.bind(cx).sexp(), data1.bind(cx).sexp(), data2.bind(cx).sexp());
            // SAFETY: rooted objects of this isolate; the flag is set before
            // anything can collect, so every collection reads the object as
            // ALTREP (data1, data2, class).
            unsafe {
                let x = cx.alloc_node_raw(t, a, d, c)?;
                x.set_flag(robigo_heap::flags::ALTREP, true);
                Ok(Ref::from_sexp(x, cx))
            }
        })
    }
}

/// `R_altrep_inherits(x, class)`: `x` is an ALTREP object of `class`.
// R: altrep.c:R_altrep_inherits
///
/// # Safety
/// C's contract: `x` is a live object of the calling isolate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_altrep_inherits(x: Sexp, class: Sexp) -> c_int {
    // SAFETY: the caller's contract: a live object; an ALTREP object's TAG
    // is its class.
    unsafe { (x.is_altrep() && x.tag() == class) as c_int }
}

/// Write entry `off` of the method table of `class` (GNU R's
/// `DEFINE_METHOD_SETTER`: no check of the class).
///
/// # Safety
/// `class` is a class object whose table has an entry at `off`.
unsafe fn set_method(class: Sexp, off: usize, fun: *const c_void) {
    // SAFETY: the caller's contract: a RAWSXP of at least `off + 8` bytes,
    // 16-aligned data; a function pointer is plain data (not traced).
    unsafe { (class.std_data().add(off) as *mut *const c_void).write(fun) }
}

macro_rules! setters {
    ($($name:ident => $ty:ident . $($field:ident).+;)*) => {$(
        // R: altrep.c:DEFINE_METHOD_SETTER
        ///
        /// # Safety
        /// C's contract: `class` is a class object of a type whose method
        /// table has this entry (GNU R casts the table the same way).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(class: Sexp, fun: *const c_void) {
            // SAFETY: the caller's contract.
            unsafe { set_method(class, offset_of!($ty, $($field).+), fun) }
        }
    )*};
}

setters! {
    R_set_altrep_UnserializeEX_method => AltrepMethods.unserialize_ex;
    R_set_altrep_Unserialize_method => AltrepMethods.unserialize;
    R_set_altrep_Serialized_state_method => AltrepMethods.serialized_state;
    R_set_altrep_DuplicateEX_method => AltrepMethods.duplicate_ex;
    R_set_altrep_Duplicate_method => AltrepMethods.duplicate;
    R_set_altrep_Coerce_method => AltrepMethods.coerce;
    R_set_altrep_Inspect_method => AltrepMethods.inspect;
    R_set_altrep_Length_method => AltrepMethods.length;
    R_set_altvec_Dataptr_method => AltvecMethods.dataptr;
    R_set_altvec_Dataptr_or_null_method => AltvecMethods.dataptr_or_null;
    R_set_altvec_Extract_subset_method => AltvecMethods.extract_subset;
    R_set_altinteger_Elt_method => AltnumMethods.elt;
    R_set_altinteger_Get_region_method => AltnumMethods.get_region;
    R_set_altinteger_Is_sorted_method => AltnumMethods.is_sorted;
    R_set_altinteger_No_NA_method => AltnumMethods.no_na;
    R_set_altinteger_Sum_method => AltnumMethods.sum;
    R_set_altinteger_Min_method => AltnumMethods.min;
    R_set_altinteger_Max_method => AltnumMethods.max;
    R_set_altreal_Elt_method => AltnumMethods.elt;
    R_set_altreal_Get_region_method => AltnumMethods.get_region;
    R_set_altreal_Is_sorted_method => AltnumMethods.is_sorted;
    R_set_altreal_No_NA_method => AltnumMethods.no_na;
    R_set_altreal_Sum_method => AltnumMethods.sum;
    R_set_altreal_Min_method => AltnumMethods.min;
    R_set_altreal_Max_method => AltnumMethods.max;
    R_set_altlogical_Elt_method => AltlogicalMethods.elt;
    R_set_altlogical_Get_region_method => AltlogicalMethods.get_region;
    R_set_altlogical_Is_sorted_method => AltlogicalMethods.is_sorted;
    R_set_altlogical_No_NA_method => AltlogicalMethods.no_na;
    R_set_altlogical_Sum_method => AltlogicalMethods.sum;
    R_set_altraw_Elt_method => AltrawMethods.elt;
    R_set_altraw_Get_region_method => AltrawMethods.get_region;
    R_set_altcomplex_Elt_method => AltrawMethods.elt;
    R_set_altcomplex_Get_region_method => AltrawMethods.get_region;
    R_set_altstring_Elt_method => AltstringMethods.elt;
    R_set_altstring_Set_elt_method => AltstringMethods.set_elt;
    R_set_altstring_Is_sorted_method => AltstringMethods.is_sorted;
    R_set_altstring_No_NA_method => AltstringMethods.no_na;
    R_set_altlist_Elt_method => AltlistMethods.elt;
    R_set_altlist_Set_elt_method => AltlistMethods.set_elt;
}
