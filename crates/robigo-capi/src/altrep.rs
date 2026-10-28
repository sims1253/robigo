//! altrep.c: ALTREP classes defined by packages. A class is a RAWSXP
//! holding its method table (as in GNU R: `TYPEOF(ALTREP_CLASS(x))` is
//! "raw") whose attributes are list(class symbol, package symbol, base
//! type); an instance is a node (data1, data2, class) with the ALTREP
//! bit, which robigo-core traces like any node.

use crate::errors::error_str_owned;
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp};
use std::ffi::{CStr, c_char, c_int, c_void};

/// One table for every kind of class; unused slots stay empty. A slot
/// holds a C function pointer (0: altrep.c's default).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Methods {
    pub unserialize_ex: usize,
    pub unserialize: usize,
    pub serialized_state: usize,
    pub duplicate_ex: usize,
    pub duplicate: usize,
    pub coerce: usize,
    pub inspect: usize,
    pub length: usize,
    pub dataptr: usize,
    pub dataptr_or_null: usize,
    pub extract_subset: usize,
    pub elt: usize,
    pub set_elt: usize,
    pub get_region: usize,
    pub is_sorted: usize,
    pub no_na: usize,
    pub sum: usize,
    pub min: usize,
    pub max: usize,
}

/// R_altrep_class_t: a struct holding the class object.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClassT {
    pub ptr: Sexp,
}

unsafe fn methods(class: Sexp) -> &'static mut Methods {
    &mut *(class.std_data() as *mut Methods)
}

#[inline]
pub unsafe fn class_of(x: Sexp) -> Sexp {
    x.tag()
}

unsafe fn mt(x: Sexp) -> &'static Methods {
    methods(class_of(x))
}

/// ALTREP_ERROR_IN_CLASS: "msg [class: c, pkg: p]"
#[cold]
unsafe fn error_in_class(msg: &str, x: Sexp) -> ! {
    let info = class_of(x).attrib_raw();
    let name = |s: Sexp| String::from_utf8_lossy(s.car().char_bytes()).into_owned();
    let (c, p) = (name(info.car()), name(info.cdr().car()));
    error_str_owned(format!("{msg} [class: {c}, pkg: {p}]"))
}

static mut REGISTRY: Vec<Sexp> = Vec::new();

// R: altrep.c:make_altrep_class / RegisterClass
unsafe fn make_class(t: u8, cname: *const c_char, pname: *const c_char) -> ClassT {
    let c = core();
    let class = c.alloc_vector(RAWSXP, std::mem::size_of::<Methods>());
    c.preserve(class);
    *methods(class) = Methods::default();
    let csym = c.install(CStr::from_ptr(cname).to_bytes());
    let psym = c.install(CStr::from_ptr(pname).to_bytes());
    let stype = crate::list::Rf_ScalarInteger(t as c_int);
    Rf_protect(class);
    let info = crate::list::Rf_list3(csym, psym, stype);
    SET_ATTRIB(class, info);
    Rf_unprotect(1);
    // a class registered again (a reloaded package) replaces the old one
    REGISTRY.retain(|&k| {
        let i = k.attrib_raw();
        !(i.car() == csym && i.cdr().car() == psym)
    });
    REGISTRY.push(class);
    ClassT { ptr: class }
}

macro_rules! make_class_fns {
    ($($name:ident => $t:expr),* $(,)?) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(cname: *const c_char, pname: *const c_char, _dll: *mut c_void) -> ClassT {
                make_class($t, cname, pname)
            }
        )*
    };
}

make_class_fns! {
    R_make_altinteger_class => INTSXP,
    R_make_altreal_class => REALSXP,
    R_make_altlogical_class => LGLSXP,
    R_make_altraw_class => RAWSXP,
    R_make_altcomplex_class => CPLXSXP,
    R_make_altstring_class => STRSXP,
    R_make_altlist_class => VECSXP,
}

macro_rules! setters {
    ($($name:ident => $field:ident),* $(,)?) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(cls: ClassT, f: usize) {
                methods(cls.ptr).$field = f;
            }
        )*
    };
}

setters! {
    R_set_altrep_UnserializeEX_method => unserialize_ex,
    R_set_altrep_Unserialize_method => unserialize,
    R_set_altrep_Serialized_state_method => serialized_state,
    R_set_altrep_DuplicateEX_method => duplicate_ex,
    R_set_altrep_Duplicate_method => duplicate,
    R_set_altrep_Coerce_method => coerce,
    R_set_altrep_Inspect_method => inspect,
    R_set_altrep_Length_method => length,
    R_set_altvec_Dataptr_method => dataptr,
    R_set_altvec_Dataptr_or_null_method => dataptr_or_null,
    R_set_altvec_Extract_subset_method => extract_subset,
    R_set_altinteger_Elt_method => elt,
    R_set_altinteger_Get_region_method => get_region,
    R_set_altinteger_Is_sorted_method => is_sorted,
    R_set_altinteger_No_NA_method => no_na,
    R_set_altinteger_Sum_method => sum,
    R_set_altinteger_Min_method => min,
    R_set_altinteger_Max_method => max,
    R_set_altreal_Elt_method => elt,
    R_set_altreal_Get_region_method => get_region,
    R_set_altreal_Is_sorted_method => is_sorted,
    R_set_altreal_No_NA_method => no_na,
    R_set_altreal_Sum_method => sum,
    R_set_altreal_Min_method => min,
    R_set_altreal_Max_method => max,
    R_set_altlogical_Elt_method => elt,
    R_set_altlogical_Get_region_method => get_region,
    R_set_altlogical_Is_sorted_method => is_sorted,
    R_set_altlogical_No_NA_method => no_na,
    R_set_altlogical_Sum_method => sum,
    R_set_altraw_Elt_method => elt,
    R_set_altraw_Get_region_method => get_region,
    R_set_altcomplex_Elt_method => elt,
    R_set_altcomplex_Get_region_method => get_region,
    R_set_altstring_Elt_method => elt,
    R_set_altstring_Set_elt_method => set_elt,
    R_set_altstring_Is_sorted_method => is_sorted,
    R_set_altstring_No_NA_method => no_na,
    R_set_altlist_Elt_method => elt,
    R_set_altlist_Set_elt_method => set_elt,
}

// R: altrep.c:R_new_altrep
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_new_altrep(cls: ClassT, data1: Sexp, data2: Sexp) -> Sexp {
    let class = cls.ptr;
    let t = *(class.attrib_raw().cdr().cdr().car().std_data() as *const i32) as u8;
    let x = core().alloc_node(t, data1, data2, class);
    x.set_flag(robigo_core::flags::ALTREP, true);
    x
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_altrep_inherits(x: Sexp, cls: ClassT) -> c_int {
    (x.is_altrep() && class_of(x) == cls.ptr) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTREP_CLASS(x: Sexp) -> Sexp {
    class_of(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_altrep_data1(x: Sexp) -> Sexp {
    x.car()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_altrep_data2(x: Sexp) -> Sexp {
    x.cdr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_set_altrep_data1(x: Sexp, v: Sexp) {
    fix_refcnt(x, x.car(), v);
    x.set_car_raw(v);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_set_altrep_data2(x: Sexp, v: Sexp) {
    fix_refcnt(x, x.cdr(), v);
    x.set_cdr_raw(v);
}

// ---- dispatch (the API functions call these for ALTREP objects) -----------

pub unsafe fn length(x: Sexp) -> isize {
    match mt(x).length {
        0 => error_in_class("no ALTREP Length method defined", x),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp) -> isize>(f)(x),
    }
}

pub unsafe fn dataptr(x: Sexp, writable: bool) -> *mut u8 {
    match mt(x).dataptr {
        0 => error_in_class("cannot access data pointer for this ALTVEC object", x),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, c_int) -> *mut u8>(f)(x, writable as c_int),
    }
}

pub unsafe fn dataptr_or_null(x: Sexp) -> *mut u8 {
    match mt(x).dataptr_or_null {
        0 => std::ptr::null_mut(),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp) -> *mut u8>(f)(x),
    }
}

macro_rules! elt_fn {
    ($name:ident, $t:ty, $ptr:ident) => {
        pub unsafe fn $name(x: Sexp, i: isize) -> $t {
            match mt(x).elt {
                0 => *($ptr(x) as *const $t).offset(i),
                f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, isize) -> $t>(f)(x, i),
            }
        }
    };
}

elt_fn!(integer_elt, c_int, INTEGER);
elt_fn!(logical_elt, c_int, LOGICAL);
elt_fn!(real_elt, f64, REAL);
elt_fn!(raw_elt, u8, RAW);
elt_fn!(complex_elt, Rcomplex, COMPLEX);

pub unsafe fn string_elt(x: Sexp, i: isize) -> Sexp {
    match mt(x).elt {
        0 => error_in_class("No Elt method found for ALTSTRING class", x),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, isize) -> Sexp>(f)(x, i),
    }
}

pub unsafe fn string_set_elt(x: Sexp, i: isize, v: Sexp) {
    match mt(x).set_elt {
        0 => error_in_class("No Set_elt found for ALTSTRING class", x),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, isize, Sexp)>(f)(x, i, v),
    }
}

pub unsafe fn list_elt(x: Sexp, i: isize) -> Sexp {
    match mt(x).elt {
        0 => error_in_class("ALTLIST classes must provide an Elt method", x),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, isize) -> Sexp>(f)(x, i),
    }
}

pub unsafe fn list_set_elt(x: Sexp, i: isize, v: Sexp) {
    match mt(x).set_elt {
        0 => error_in_class("ALTLIST classes must provide a Set_elt method", x),
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, isize, Sexp)>(f)(x, i, v),
    }
}

/// A standard copy of an ALTREP vector, element by element.
unsafe fn materialize(x: Sexp) -> Sexp {
    let t = x.typ();
    let n = length(x);
    let r = Rf_protect(crate::memory::alloc_vector(t as u32, n));
    for i in 0..n {
        match t {
            INTSXP => *(r.std_data() as *mut c_int).offset(i) = integer_elt(x, i),
            LGLSXP => *(r.std_data() as *mut c_int).offset(i) = logical_elt(x, i),
            REALSXP => *(r.std_data() as *mut f64).offset(i) = real_elt(x, i),
            RAWSXP => *r.std_data().offset(i) = raw_elt(x, i),
            CPLXSXP => *(r.std_data() as *mut Rcomplex).offset(i) = complex_elt(x, i),
            STRSXP => SET_STRING_ELT(r, i, string_elt(x, i)),
            VECSXP => {
                SET_VECTOR_ELT(r, i, list_elt(x, i));
            }
            _ => {}
        }
    }
    Rf_unprotect(1);
    r
}

// R: altrep.c:altrep_DuplicateEX_default, then duplicate.c's fallback
pub unsafe fn duplicate(x: Sexp, deep: bool) -> Sexp {
    let m = mt(x);
    let ans = match m.duplicate {
        0 => Sexp::NULL,
        f => std::mem::transmute::<usize, unsafe extern "C" fn(Sexp, c_int) -> Sexp>(f)(x, deep as c_int),
    };
    let ans = if ans.is_null() { materialize(x) } else { ans };
    if ans != x {
        let attr = ATTRIB(x);
        if attr != R_NilValue {
            Rf_protect(ans);
            SET_ATTRIB(ans, if deep { Rf_duplicate(attr) } else { Rf_shallow_duplicate(attr) });
            SET_OBJECT(ans, OBJECT(x));
            if IS_S4_OBJECT(x) != 0 {
                SET_S4_OBJECT(ans)
            } else {
                UNSET_S4_OBJECT(ans)
            }
            Rf_unprotect(1);
        } else if ATTRIB(ans) != R_NilValue {
            SET_ATTRIB(ans, R_NilValue);
            SET_OBJECT(ans, 0);
            UNSET_S4_OBJECT(ans);
        }
    }
    ans
}

// ---- the exported accessors GNU R's Defn.h declares ------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTREP_LENGTH(x: Sexp) -> isize {
    length(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTVEC_DATAPTR(x: Sexp) -> *mut c_void {
    dataptr(x, true) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTVEC_DATAPTR_RO(x: Sexp) -> *const c_void {
    dataptr(x, false) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTVEC_DATAPTR_OR_NULL(x: Sexp) -> *const c_void {
    dataptr_or_null(x) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTSTRING_ELT(x: Sexp, i: isize) -> Sexp {
    string_elt(x, i)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTSTRING_SET_ELT(x: Sexp, i: isize, v: Sexp) {
    string_set_elt(x, i, v)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTINTEGER_ELT(x: Sexp, i: isize) -> c_int {
    integer_elt(x, i)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTREAL_ELT(x: Sexp, i: isize) -> f64 {
    real_elt(x, i)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTLOGICAL_ELT(x: Sexp, i: isize) -> c_int {
    logical_elt(x, i)
}

/// R: altrep.c's sortedness/NA queries for any vector (UNKNOWN_SORTEDNESS
/// is NA_INTEGER).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn INTEGER_IS_SORTED(x: Sexp) -> c_int {
    if x.is_altrep() && mt(x).is_sorted != 0 {
        return std::mem::transmute::<usize, unsafe extern "C" fn(Sexp) -> c_int>(mt(x).is_sorted)(x);
    }
    i32::MIN
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn INTEGER_NO_NA(x: Sexp) -> c_int {
    if x.is_altrep() && mt(x).no_na != 0 {
        return std::mem::transmute::<usize, unsafe extern "C" fn(Sexp) -> c_int>(mt(x).no_na)(x);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REAL_IS_SORTED(x: Sexp) -> c_int {
    INTEGER_IS_SORTED(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REAL_NO_NA(x: Sexp) -> c_int {
    INTEGER_NO_NA(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_IS_SORTED(x: Sexp) -> c_int {
    INTEGER_IS_SORTED(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_NO_NA(x: Sexp) -> c_int {
    INTEGER_NO_NA(x)
}
