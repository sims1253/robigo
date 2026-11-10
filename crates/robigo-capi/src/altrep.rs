//! altrep.c: ALTREP classes defined by packages. A class is a RAWSXP
//! holding its method table (as in GNU R: `TYPEOF(ALTREP_CLASS(x))` is
//! "raw") whose attributes are list(class symbol, package symbol, base
//! type); an instance is a node (data1, data2, class) with the ALTREP
//! bit, which robigo-core traces like any node.

use crate::errors::{Res, Unwind, call_c2, call_d2, call_w6, error_str, ret};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// One of robigo's own classes (compact.rs: 1 intseq, 2 realseq),
    /// whose hot methods are called as Rust, not through a trampoline:
    /// they are not a package's code, and `INTEGER(1:n)` stays as cheap
    /// as GNU R's. 0 for a package's class.
    pub own: usize,
}

/// R_altrep_class_t: a struct holding the class object.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClassT {
    pub ptr: Sexp,
}

unsafe fn methods(class: Sexp) -> *mut Methods {
    class.std_data() as *mut Methods
}

#[inline]
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn class_of(x: Sexp) -> Sexp {
    x.tag()
}

/// The method table of an ALTREP object.
#[inline]
unsafe fn mt(x: Sexp) -> *const Methods {
    methods(class_of(x))
}

/// Whether `x` is an instance of a class a package made, not one of
/// robigo's own (the compact sequences, registered for package "base").
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn package_class(x: Sexp) -> bool {
    let info = class_of(x).attrib_raw();
    if info.is_null() || info == R_NilValue {
        return false;
    }
    let p = info.cdr().car();
    p.typ() == SYMSXP && p.car().char_bytes() != b"base"
}

/// The class's `Sum`, `Min` or `Max` (0: none).
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn summary_method(x: Sexp, which: robigo_runtime::altrep::Summary) -> usize {
    use robigo_runtime::altrep::Summary;
    let m = &*mt(x);
    match which {
        Summary::Sum => m.sum,
        Summary::Min => m.min,
        Summary::Max => m.max,
    }
}

/// The class's `Is_sorted` (`no_na`: `No_NA`) (0: none).
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn sorted_method(x: Sexp, no_na: bool) -> usize {
    let m = &*mt(x);
    if no_na { m.no_na } else { m.is_sorted }
}

/// The class's `Inspect` (0: none).
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn inspect_method(x: Sexp) -> usize {
    (*mt(x)).inspect
}

/// The class's `Serialized_state` (0: none).
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn serialized_state_method(x: Sexp) -> usize {
    (*mt(x)).serialized_state
}

/// The class's `Extract_subset` (0: none).
/// # Safety
/// x must be an ALTREP instance made by make_class.
pub unsafe fn extract_subset_method(x: Sexp) -> usize {
    (*mt(x)).extract_subset
}

/// ALTREP_ERROR_IN_CLASS: "msg [class: c, pkg: p]"
#[cold]
unsafe fn error_in_class(msg: &str, x: Sexp) -> Unwind {
    let info = class_of(x).attrib_raw();
    let name = |s: Sexp| String::from_utf8_lossy(s.car().char_bytes()).into_owned();
    let (c, p) = (name(info.car()), name(info.cdr().car()));
    error_str(&format!("{msg} [class: {c}, pkg: {p}]"))
}

/// A package instantiated an ALTREP class. From then on converting a core
/// object can run package code (Length, Dataptr, Elt) that may raise, so
/// the bridge converts inside a context; before that, compact sequences
/// are the only ALTREP objects and their methods reached by a conversion
/// cannot raise, and the normal path stays free of the context.
pub static PACKAGE_ALTREP: AtomicBool = AtomicBool::new(false);

thread_local! {
    static CLASSES: std::cell::RefCell<Vec<Sexp>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// # Safety
/// info must be a serialized ALTREP class-info pairlist.
pub unsafe fn lookup_class(info: Sexp) -> Option<Sexp> {
    CLASSES.with(|classes| {
        classes.borrow().iter().rev().copied().find(|class| {
            let registered = class.attrib_raw();
            registered.car() == info.car() && registered.cdr().car() == info.cdr().car()
        })
    })
}

/// # Safety
/// class must be a class returned by make_class.
pub unsafe fn unserialize_methods(class: Sexp) -> (usize, usize) {
    let m = &*methods(class);
    (m.unserialize_ex, m.unserialize)
}

// R: altrep.c:make_altrep_class / RegisterClass
pub(crate) unsafe fn make_class(t: u8, cname: *const c_char, pname: *const c_char) -> Res<ClassT> {
    let c = core();
    let class = c.alloc_vector(RAWSXP, std::mem::size_of::<Methods>());
    c.preserve(class);
    *methods(class) = Methods::default();
    let csym = c.install(CStr::from_ptr(cname).to_bytes());
    let psym = c.install(CStr::from_ptr(pname).to_bytes());
    let stype = crate::list::Rf_ScalarInteger(t as c_int);
    protect(class)?;
    let info = crate::list::list3(csym, psym, stype)?;
    set_attrib_list(class, info)?;
    unprotect(1)?;
    CLASSES.with(|classes| classes.borrow_mut().push(class));
    Ok(ClassT { ptr: class })
}

macro_rules! make_class_fns {
    ($($name:ident => $t:expr),* $(,)?) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(cname: *const c_char, pname: *const c_char, _dll: *mut c_void) -> ClassT {
                ret(make_class($t, cname, pname))
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
                (*methods(cls.ptr)).$field = f;
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
    PACKAGE_ALTREP.store(true, Ordering::Relaxed);
    new_instance(cls, data1, data2)
}

/// An instance of a class of ours (compact sequences), which leaves
/// [`PACKAGE_ALTREP`] alone.
pub(crate) unsafe fn new_instance(cls: ClassT, data1: Sexp, data2: Sexp) -> Sexp {
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
    x.set_car(v);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_set_altrep_data2(x: Sexp, v: Sexp) {
    x.set_cdr(v);
}

// ---- dispatch (the API functions call these for ALTREP objects) -----------
//
// A class's methods are C functions (a package's, or compact.rs's): each
// is called through a C trampoline, so an error it raises comes back as
// an `Err` (docs/core.md §5).

#[inline]
fn w(x: Sexp) -> usize {
    x.0 as usize
}

#[inline]
fn sx(v: usize) -> Sexp {
    Sexp(v as *mut u8)
}

/// The data of an expanded object of one of robigo's own classes (a
/// compact sequence whose data pointer was taken): the hot case of
/// `INTEGER(1:n)`, read without a call.
#[inline(always)]
pub unsafe fn own_expanded_data(x: Sexp) -> Option<*mut u8> {
    if (*mt(x)).own == 0 {
        return None;
    }
    let e = x.cdr();
    if e == R_NilValue { None } else { Some(e.std_data()) }
}

/// Mark `cls` as one of robigo's own classes (see [`Methods::own`]).
pub(crate) unsafe fn set_own(cls: ClassT, kind: usize) {
    (*methods(cls.ptr)).own = kind;
}

pub unsafe fn length(x: Sexp) -> Res<isize> {
    if (*mt(x)).own != 0 {
        return Ok(crate::compact::own_length(x));
    }
    match (*mt(x)).length {
        0 => Err(error_in_class("no ALTREP Length method defined", x)),
        f => Ok(call_w6(f, [w(x), 0, 0, 0, 0, 0])? as isize),
    }
}

#[inline]
pub unsafe fn dataptr(x: Sexp, writable: bool) -> Res<*mut u8> {
    match (*mt(x)).own {
        0 => {}
        kind => return crate::compact::own_dataptr(x, kind),
    }
    match (*mt(x)).dataptr {
        0 => Err(error_in_class("cannot access data pointer for this ALTVEC object", x)),
        f => Ok(call_w6(f, [w(x), writable as usize, 0, 0, 0, 0])? as *mut u8),
    }
}

pub unsafe fn dataptr_or_null(x: Sexp) -> Res<*mut u8> {
    if (*mt(x)).own != 0 {
        return Ok(crate::compact::own_dataptr_or_null(x));
    }
    match (*mt(x)).dataptr_or_null {
        0 => Ok(std::ptr::null_mut()),
        f => Ok(call_w6(f, [w(x), 0, 0, 0, 0, 0])? as *mut u8),
    }
}

/// The class's Elt for element `i`, if it has one.
#[inline]
unsafe fn elt_word(x: Sexp, i: isize) -> Res<Option<usize>> {
    match (*mt(x)).elt {
        0 => Ok(None),
        f => Ok(Some(call_w6(f, [w(x), i as usize, 0, 0, 0, 0])?)),
    }
}

pub unsafe fn integer_elt(x: Sexp, i: isize) -> Res<c_int> {
    if (*mt(x)).own == 1 {
        return Ok(crate::compact::own_int_elt(x, i));
    }
    match elt_word(x, i)? {
        Some(v) => Ok(v as c_int),
        None => Ok(*integer(x)?.offset(i)),
    }
}

pub unsafe fn logical_elt(x: Sexp, i: isize) -> Res<c_int> {
    match elt_word(x, i)? {
        Some(v) => Ok(v as c_int),
        None => Ok(*logical(x)?.offset(i)),
    }
}

pub unsafe fn raw_elt(x: Sexp, i: isize) -> Res<u8> {
    match elt_word(x, i)? {
        Some(v) => Ok(v as u8),
        None => Ok(*raw(x)?.offset(i)),
    }
}

pub unsafe fn real_elt(x: Sexp, i: isize) -> Res<f64> {
    if (*mt(x)).own == 2 {
        return Ok(crate::compact::own_real_elt(x, i));
    }
    match (*mt(x)).elt {
        0 => Ok(*real(x)?.offset(i)),
        f => call_d2(f, w(x), i as usize),
    }
}

pub unsafe fn complex_elt(x: Sexp, i: isize) -> Res<Rcomplex> {
    match (*mt(x)).elt {
        0 => Ok(*complex(x)?.offset(i)),
        f => call_c2(f, w(x), i as usize),
    }
}

pub unsafe fn string_elt(x: Sexp, i: isize) -> Res<Sexp> {
    match elt_word(x, i)? {
        Some(v) => Ok(sx(v)),
        None => Err(error_in_class("No Elt method found for ALTSTRING class", x)),
    }
}

pub unsafe fn string_set_elt(x: Sexp, i: isize, v: Sexp) -> Res<()> {
    match (*mt(x)).set_elt {
        0 => Err(error_in_class("No Set_elt found for ALTSTRING class", x)),
        f => call_w6(f, [w(x), i as usize, w(v), 0, 0, 0]).map(drop),
    }
}

pub unsafe fn list_elt(x: Sexp, i: isize) -> Res<Sexp> {
    match elt_word(x, i)? {
        Some(v) => Ok(sx(v)),
        None => Err(error_in_class("ALTLIST classes must provide an Elt method", x)),
    }
}

pub unsafe fn list_set_elt(x: Sexp, i: isize, v: Sexp) -> Res<()> {
    match (*mt(x)).set_elt {
        0 => Err(error_in_class("ALTLIST classes must provide a Set_elt method", x)),
        f => call_w6(f, [w(x), i as usize, w(v), 0, 0, 0]).map(drop),
    }
}

/// Call method `f` (nonzero) of an ALTREP object with word arguments.
///
/// # Safety
/// `f` must be the class's method of that arity, returning a word or
/// nothing.
pub unsafe fn call_method(f: usize, args: &[usize]) -> Res<usize> {
    let mut a = [0usize; 6];
    a[..args.len()].copy_from_slice(args);
    call_w6(f, a)
}

/// A standard copy of an ALTREP vector, element by element.
unsafe fn materialize(x: Sexp) -> Res<Sexp> {
    let t = x.typ();
    let n = length(x)?;
    let r = protect(crate::memory::alloc_vector(t as u32, n)?)?;
    for i in 0..n {
        match t {
            INTSXP => *(r.std_data() as *mut c_int).offset(i) = integer_elt(x, i)?,
            LGLSXP => *(r.std_data() as *mut c_int).offset(i) = logical_elt(x, i)?,
            REALSXP => *(r.std_data() as *mut f64).offset(i) = real_elt(x, i)?,
            RAWSXP => *r.std_data().offset(i) = raw_elt(x, i)?,
            CPLXSXP => *(r.std_data() as *mut Rcomplex).offset(i) = complex_elt(x, i)?,
            STRSXP => set_string_elt(r, i, string_elt(x, i)?)?,
            VECSXP => {
                set_vector_elt(r, i, list_elt(x, i)?)?;
            }
            _ => {}
        }
    }
    unprotect(1)?;
    Ok(r)
}

// R: altrep.c:altrep_DuplicateEX_default, then duplicate.c's fallback
pub unsafe fn duplicate(x: Sexp, deep: bool) -> Res<Sexp> {
    let ans = match (*mt(x)).duplicate {
        0 => Sexp::NULL,
        f => sx(call_w6(f, [w(x), deep as usize, 0, 0, 0, 0])?),
    };
    let ans = if ans.is_null() { materialize(x)? } else { ans };
    if ans != x {
        let attr = ATTRIB(x);
        if attr != R_NilValue {
            protect(ans)?;
            set_attrib_list(ans, if deep { crate::memory::duplicate(attr)? } else { shallow_duplicate(attr)? })?;
            SET_OBJECT(ans, OBJECT(x));
            if IS_S4_OBJECT(x) != 0 {
                SET_S4_OBJECT(ans)
            } else {
                UNSET_S4_OBJECT(ans)
            }
            unprotect(1)?;
        } else if ATTRIB(ans) != R_NilValue {
            set_attrib_list(ans, R_NilValue)?;
            SET_OBJECT(ans, 0);
            UNSET_S4_OBJECT(ans);
        }
    }
    Ok(ans)
}

// ---- the exported accessors GNU R's Defn.h declares ------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTREP_LENGTH(x: Sexp) -> isize {
    ret(length(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTVEC_DATAPTR(x: Sexp) -> *mut c_void {
    ret(dataptr(x, true)) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTVEC_DATAPTR_RO(x: Sexp) -> *const c_void {
    ret(dataptr(x, false)) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTVEC_DATAPTR_OR_NULL(x: Sexp) -> *const c_void {
    ret(dataptr_or_null(x)) as *const c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTSTRING_ELT(x: Sexp, i: isize) -> Sexp {
    ret(string_elt(x, i))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTSTRING_SET_ELT(x: Sexp, i: isize, v: Sexp) {
    ret(string_set_elt(x, i, v))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTINTEGER_ELT(x: Sexp, i: isize) -> c_int {
    ret(integer_elt(x, i))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTREAL_ELT(x: Sexp, i: isize) -> f64 {
    ret(real_elt(x, i))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTLOGICAL_ELT(x: Sexp, i: isize) -> c_int {
    ret(logical_elt(x, i))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTRAW_ELT(x: Sexp, i: isize) -> u8 {
    ret(raw_elt(x, i))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTCOMPLEX_ELT(x: Sexp, i: isize) -> Rcomplex {
    ret(complex_elt(x, i))
}

/// R: altrep.c's sortedness queries for any vector (UNKNOWN_SORTEDNESS
/// is NA_INTEGER).
pub unsafe fn is_sorted(x: Sexp) -> Res<c_int> {
    if x.is_altrep() && (*mt(x)).is_sorted != 0 {
        return Ok(call_w6((*mt(x)).is_sorted, [w(x), 0, 0, 0, 0, 0])? as c_int);
    }
    Ok(i32::MIN)
}

/// R: altrep.c's NA queries for any vector.
pub unsafe fn no_na(x: Sexp) -> Res<c_int> {
    if x.is_altrep() && (*mt(x)).no_na != 0 {
        return Ok(call_w6((*mt(x)).no_na, [w(x), 0, 0, 0, 0, 0])? as c_int);
    }
    Ok(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn INTEGER_IS_SORTED(x: Sexp) -> c_int {
    ret(is_sorted(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn INTEGER_NO_NA(x: Sexp) -> c_int {
    ret(no_na(x))
}

// R: altrep.c:ALTINTEGER_MIN/MAX (the default method gives NULL)
unsafe fn min_max(f: usize, x: Sexp, narm: c_int) -> Res<Sexp> {
    if f != 0 {
        return Ok(sx(call_w6(f, [w(x), narm as usize, 0, 0, 0, 0])?));
    }
    Ok(crate::state::R_NilValue)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTINTEGER_MIN(x: Sexp, narm: c_int) -> Sexp {
    ret(min_max((*mt(x)).min, x, narm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ALTINTEGER_MAX(x: Sexp, narm: c_int) -> Sexp {
    ret(min_max((*mt(x)).max, x, narm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REAL_IS_SORTED(x: Sexp) -> c_int {
    ret(is_sorted(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn REAL_NO_NA(x: Sexp) -> c_int {
    ret(no_na(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_IS_SORTED(x: Sexp) -> c_int {
    ret(is_sorted(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn STRING_NO_NA(x: Sexp) -> c_int {
    ret(no_na(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LOGICAL_IS_SORTED(x: Sexp) -> c_int {
    ret(is_sorted(x))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LOGICAL_NO_NA(x: Sexp) -> c_int {
    ret(no_na(x))
}
