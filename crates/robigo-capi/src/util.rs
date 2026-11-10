//! util.c and Rinlinedfuns.h: type names, predicates, dimensions; and the
//! dispatch points to ALTREP classes and finalizers.

use crate::errors::{Res, error_str, ret};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Sexp, flags, gp};
use std::ffi::{CStr, c_char, c_int, c_uint};

// R: util.c TypeTable / Type2Table
static TYPE_NAMES: &[(u8, &str)] = &[
    (NILSXP, "NULL"),
    (SYMSXP, "symbol"),
    (LISTSXP, "pairlist"),
    (CLOSXP, "closure"),
    (ENVSXP, "environment"),
    (PROMSXP, "promise"),
    (LANGSXP, "language"),
    (SPECIALSXP, "special"),
    (BUILTINSXP, "builtin"),
    (CHARSXP, "char"),
    (LGLSXP, "logical"),
    (INTSXP, "integer"),
    (REALSXP, "double"),
    (CPLXSXP, "complex"),
    (STRSXP, "character"),
    (DOTSXP, "..."),
    (ANYSXP, "any"),
    (VECSXP, "list"),
    (EXPRSXP, "expression"),
    (BCODESXP, "bytecode"),
    (EXTPTRSXP, "externalptr"),
    (WEAKREFSXP, "weakref"),
    (RAWSXP, "raw"),
    (OBJSXP, "object"),
];

pub fn type_name(t: u8) -> &'static str {
    TYPE_NAMES.iter().find(|(k, _)| *k == t).map(|(_, n)| *n).unwrap_or("unknown")
}

/// R_typeToChar: type2char, but S4 objects are "S4".
pub unsafe fn type_to_char(x: Sexp) -> &'static str {
    if x.typ() == OBJSXP { if x.gp() & gp::S4 != 0 { "S4" } else { "object" } } else { type_name(x.typ()) }
}

static TYPE_CNAMES: &[&CStr] = &[
    c"NULL",
    c"symbol",
    c"pairlist",
    c"closure",
    c"environment",
    c"promise",
    c"language",
    c"special",
    c"builtin",
    c"char",
    c"logical",
    c"",
    c"",
    c"integer",
    c"double",
    c"complex",
    c"character",
    c"...",
    c"any",
    c"list",
    c"expression",
    c"bytecode",
    c"externalptr",
    c"weakref",
    c"raw",
    c"object",
];

// R: util.c:type2char
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_type2char(t: c_uint) -> *const c_char {
    ret(type2char(t))
}

/// type2char.
pub unsafe fn type2char(t: c_uint) -> Res<*const c_char> {
    match TYPE_CNAMES.get(t as usize) {
        Some(s) if !s.is_empty() => Ok(s.as_ptr()),
        _ => {
            crate::errors::warning_str(&format!("type {} is unimplemented in '{}'", t as c_int, "type2char"))?;
            thread_local! {
                static BUF: std::cell::UnsafeCell<[u8; 50]> = const { std::cell::UnsafeCell::new([0; 50]) };
            }
            let s = format!("unknown type #{t}\0");
            let n = s.len().min(49);
            let buf = BUF.with(|b| b.get()).cast::<u8>();
            std::ptr::copy_nonoverlapping(s.as_ptr(), buf, n);
            Ok(buf as *const c_char)
        }
    }
}

// R: printutils.c:IndexWidth
#[unsafe(no_mangle)]
pub extern "C" fn Rf_IndexWidth(n: isize) -> c_int {
    ((n as f64 + 0.5).log10() + 1.0) as c_int
}

// R: printutils.c:Rstrlen (the width of the escaped, maybe quoted string)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rstrlen(s: Sexp, quote: c_int) -> c_int {
    let text = String::from_utf8_lossy(s.char_bytes()).into_owned();
    let q = if quote != 0 { Some(quote as u8 as char) } else { None };
    robigo_runtime::format::rstrlen(&text, q) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_typeToChar(x: Sexp) -> *const c_char {
    if x.typ() == OBJSXP {
        if x.gp() & gp::S4 != 0 { c"S4".as_ptr() } else { c"object".as_ptr() }
    } else {
        ret(type2char(x.typ() as c_uint))
    }
}

// R: util.c:type2str (a CHARSXP)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_type2str(t: c_uint) -> Sexp {
    ret(type2str(t))
}

/// type2str.
pub unsafe fn type2str(t: c_uint) -> Res<Sexp> {
    match TYPE_CNAMES.get(t as usize) {
        Some(s) if !s.is_empty() => Ok(core().mk_char(s.to_bytes(), 0)),
        _ => {
            crate::errors::warning_str(&format!("type {} is unimplemented in '{}'", t as c_int, "type2str"))?;
            Ok(core().mk_char(format!("unknown type #{t}").as_bytes(), 0))
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_type2str_nowarn(t: c_uint) -> Sexp {
    match TYPE_CNAMES.get(t as usize) {
        Some(s) if !s.is_empty() => core().mk_char(s.to_bytes(), 0),
        _ => R_NilValue,
    }
}

// R: util.c:str2type (the TypeTable, with its aliases)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_str2type(s: *const c_char) -> c_uint {
    let s = CStr::from_ptr(s).to_bytes();
    let t: i32 = match s {
        b"NULL" => 0,
        b"symbol" => 1,
        b"pairlist" => 2,
        b"closure" => 3,
        b"environment" => 4,
        b"promise" => 5,
        b"language" => 6,
        b"special" => 7,
        b"builtin" => 8,
        b"char" => 9,
        b"logical" => 10,
        b"integer" => 13,
        b"double" => 14,
        b"complex" => 15,
        b"character" => 16,
        b"..." => 17,
        b"any" => 18,
        b"expression" => 20,
        b"list" => 19,
        b"externalptr" => 22,
        b"bytecode" => 21,
        b"weakref" => 23,
        b"raw" => 24,
        b"S4" | b"object" => 25,
        b"numeric" => 14,
        b"name" => 1,
        _ => -1,
    };
    t as c_uint
}

// ---- predicates (Rinlinedfuns.h, util.c) ------------------------------------

pub unsafe fn is_vector(x: Sexp) -> bool {
    matches!(x.typ(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | VECSXP | EXPRSXP)
}

/// inherits (a class vector that is an ALTREP object can fail).
pub unsafe fn inherits(x: Sexp, name: &[u8]) -> Res<bool> {
    if !x.has_flag(flags::OBJECT) {
        return Ok(false);
    }
    let klass = crate::attrib::get_class(x);
    if klass.typ() != STRSXP {
        return Ok(false);
    }
    let n = xlength(klass)?;
    for i in 0..n {
        if string_elt(klass, i)?.char_bytes() == name {
            return Ok(true);
        }
    }
    Ok(false)
}

macro_rules! pred {
    ($($name:ident(|$x:ident| $e:expr);)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($x: Sexp) -> c_int {
                ($e) as c_int
            }
        )*
    };
}

pred! {
    Rf_isNull(|s| s.typ() == NILSXP);
    Rf_isSymbol(|s| s.typ() == SYMSXP);
    Rf_isLogical(|s| s.typ() == LGLSXP);
    Rf_isReal(|s| s.typ() == REALSXP);
    Rf_isComplex(|s| s.typ() == CPLXSXP);
    Rf_isExpression(|s| s.typ() == EXPRSXP);
    Rf_isEnvironment(|s| s.typ() == ENVSXP);
    Rf_isString(|s| s.typ() == STRSXP);
    Rf_isObject(|s| s.has_flag(flags::OBJECT));
    Rf_isRaw(|s| s.typ() == RAWSXP);
    Rf_isS4(|s| s.gp() & gp::S4 != 0);
    Rf_isNewList(|s| s.typ() == NILSXP || s.typ() == VECSXP);
    Rf_isList(|s| s.typ() == NILSXP || s.typ() == LISTSXP);
    Rf_isPairList(|s| matches!(s.typ(), NILSXP | LISTSXP | LANGSXP | DOTSXP));
    Rf_isVectorList(|s| matches!(s.typ(), VECSXP | EXPRSXP));
    Rf_isVectorAtomic(|s| matches!(s.typ(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP));
    Rf_isVector(|s| is_vector(s));
    Rf_isPrimitive(|s| matches!(s.typ(), BUILTINSXP | SPECIALSXP));
    Rf_isFunction(|s| matches!(s.typ(), CLOSXP | BUILTINSXP | SPECIALSXP));
    Rf_isLanguage(|s| s.typ() == NILSXP || s.typ() == LANGSXP);
}

/// Predicates that read attributes or elements, which can fail (an
/// ALTREP class or dim vector): `Rf_x` wraps `x`.
macro_rules! pred_res {
    ($($name:ident, $inner:ident(|$x:ident| $e:expr);)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($x: Sexp) -> c_int {
                ret($inner($x)) as c_int
            }
            #[doc = concat!("`", stringify!($name), "()`.")]
            pub unsafe fn $inner($x: Sexp) -> Res<bool> {
                Ok($e)
            }
        )*
    };
}

pred_res! {
    Rf_isInteger, is_integer(|s| s.typ() == INTSXP && !inherits(s, b"factor")?);
    Rf_isFactor, is_factor(|s| s.typ() == INTSXP && inherits(s, b"factor")?);
    Rf_isNumeric, is_numeric(|s| match s.typ() {
        INTSXP => !inherits(s, b"factor")?,
        LGLSXP | REALSXP => true,
        _ => false,
    });
    Rf_isNumber, is_number(|s| match s.typ() {
        INTSXP => !inherits(s, b"factor")?,
        LGLSXP | REALSXP | CPLXSXP => true,
        _ => false,
    });
    Rf_isFrame, is_frame(|s| inherits(s, b"data.frame")?);
    Rf_isDataFrame, is_data_frame(|s| inherits(s, b"data.frame")?);
    Rf_isMatrix, is_matrix(|s| is_vector(s) && {
        let t = crate::attrib::get_attrib(s, R_DimSymbol)?;
        t.typ() == INTSXP && length(t)? == 2
    });
    Rf_isArray, is_array(|s| is_vector(s) && {
        let t = crate::attrib::get_attrib(s, R_DimSymbol)?;
        t.typ() == INTSXP && length(t)? > 0
    });
    Rf_isTs, is_ts(|s| is_vector(s) && crate::attrib::get_attrib(s, R_TspSymbol)? != R_NilValue);
    Rf_isValidString, is_valid_string(|s| s.typ() == STRSXP && length(s)? > 0 && string_elt(s, 0)?.typ() != NILSXP);
    Rf_isUnordered, is_unordered(|s| s.typ() == INTSXP && inherits(s, b"factor")? && !inherits(s, b"ordered")?);
    Rf_isOrdered, is_ordered(|s| s.typ() == INTSXP && inherits(s, b"factor")? && inherits(s, b"ordered")?);
    Rf_isValidStringF, is_valid_string_f(|s| is_valid_string(s)? && !string_elt(s, 0)?.char_bytes().is_empty());
}

// R: Rinlinedfuns.h:nlevels
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_nlevels(s: Sexp) -> c_int {
    ret(nlevels(s))
}

unsafe fn nlevels(s: Sexp) -> Res<c_int> {
    if is_factor(s)? { length(crate::attrib::get_attrib(s, R_LevelsSymbol)?) } else { Ok(0) }
}

// R: util.c:isBlankString
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_isBlankString(s: *const c_char) -> c_int {
    CStr::from_ptr(s).to_bytes().iter().all(|c| c.is_ascii_whitespace()) as c_int
}

// R: util.c:StringBlank
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_StringBlank(x: Sexp) -> c_int {
    (x == R_NilValue || x.char_bytes().is_empty()) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_StringFalse(name: *const c_char) -> c_int {
    matches!(CStr::from_ptr(name).to_bytes(), b"F" | b"False" | b"FALSE" | b"false") as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_StringTrue(name: *const c_char) -> c_int {
    matches!(CStr::from_ptr(name).to_bytes(), b"T" | b"True" | b"TRUE" | b"true") as c_int
}

// R: util.c:inherits (via Rinlinedfuns)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_inherits(x: Sexp, name: *const c_char) -> c_int {
    ret(inherits(x, CStr::from_ptr(name).to_bytes())) as c_int
}

// R: util.c:nrows
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_nrows(s: Sexp) -> c_int {
    ret(nrows(s))
}

/// nrows.
pub unsafe fn nrows(s: Sexp) -> Res<c_int> {
    if is_vector(s) || Rf_isList(s) != 0 {
        let t = crate::attrib::get_attrib(s, R_DimSymbol)?;
        if t == R_NilValue {
            return r_length(s);
        }
        return Ok(*integer(t)?);
    } else if is_frame(s)? {
        return nrows(s.car());
    }
    Err(error_str("object is not a matrix"))
}

// R: util.c:ncols
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_ncols(s: Sexp) -> c_int {
    ret(ncols(s))
}

/// ncols.
pub unsafe fn ncols(s: Sexp) -> Res<c_int> {
    if is_vector(s) || Rf_isList(s) != 0 {
        let t = crate::attrib::get_attrib(s, R_DimSymbol)?;
        if t == R_NilValue {
            return Ok(1);
        }
        if length(t)? >= 2 {
            return Ok(*integer(t)?.add(1));
        }
        return Ok(1);
    } else if is_frame(s)? {
        return r_length(s);
    }
    Err(error_str("object is not a matrix"))
}

// R: util.c:conformable
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_conformable(x: Sexp, y: Sexp) -> c_int {
    ret(conformable(x, y)) as c_int
}

unsafe fn conformable(x: Sexp, y: Sexp) -> Res<bool> {
    let dx = crate::attrib::get_attrib(x, R_DimSymbol)?;
    let dy = crate::attrib::get_attrib(y, R_DimSymbol)?;
    let n = r_length(dx)?;
    if n != r_length(dy)? {
        return Ok(false);
    }
    for i in 0..n as usize {
        if *integer(dx)?.add(i) != *integer(dy)?.add(i) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_envlength(rho: Sexp) -> c_int {
    ret(env_length(rho))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_envxlength(rho: Sexp) -> isize {
    ret(env_xlength(rho))
}

/// envlength.
pub unsafe fn env_length(rho: Sexp) -> Res<c_int> {
    Ok(env_xlength(rho)? as c_int)
}

/// envxlength.
pub unsafe fn env_xlength(rho: Sexp) -> Res<isize> {
    let r = crate::host::op("envlength\0", &[rho])?;
    Ok(*(r.std_data() as *const i32) as isize)
}

// R: memory.c:IS_SCALAR
#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_SCALAR(x: Sexp, ty: c_int) -> c_int {
    if x.is_null() {
        return 0;
    }
    // GNU R's scalar bit is set for vectors of length 1 only
    (x.typ() as c_int == ty && !matches!(x.typ(), NILSXP | LISTSXP | LANGSXP | DOTSXP) && ret(checked_xlength(x)) == 1)
        as c_int
}

// R: Rinlinedfuns.h:XTRUELENGTH
#[unsafe(no_mangle)]
pub unsafe extern "C" fn XTRUELENGTH(x: Sexp) -> isize {
    ret(truelength(x))
}

/// A safe point (memory.c R_RunPendingFinalizers, called by GNU R from
/// the evaluator and gc()): run the release callbacks of dead host
/// proxies, then the finalizers of weak references whose keys died. Never
/// called from inside an allocation or a collection.
pub unsafe fn safe_point() {
    for (f, h) in core().take_releases() {
        f(h);
    }
    run_finalizers();
}

thread_local! {
    static RUNNING_FINALIZERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// R: memory.c:RunFinalizers -- nested GC may queue more finalizers but
/// never recursively enters the finalizer runner.
pub unsafe fn run_finalizers() {
    // R: memory.c:RunFinalizers walks the references present at entry.
    // References queued by a nested GC wait for a later safe point.
    let count = core().pending_finalizer_count();
    if count == 0 || RUNNING_FINALIZERS.with(|r| r.replace(true)) {
        return;
    }
    for _ in 0..count {
        let Some(w) = core().pop_finalizable() else { break };
        // R: memory.c:RunFinalizers -- isolate errors from the caller and
        // restore its contexts, condition stacks and visibility afterwards.
        // Each finalizer has its own context, so a jump stops here (the
        // caller may have none) and the flag below is always reset.
        let visible = R_Visible;
        finalizer_context("finalizerBegin\0");
        let r = crate::errors::in_context(|| {
            protect(w)?;
            crate::extptr::run_finalizer(w)?;
            unprotect(1)
        });
        if let Err(u) = r {
            (crate::host::host().report)(u.into_token());
        }
        finalizer_context("finalizerEnd\0");
        R_Visible = visible;
    }
    RUNNING_FINALIZERS.with(|r| r.set(false));
}

/// The host's finalizer context operations; they have nothing to raise.
unsafe fn finalizer_context(op: &'static str) {
    let mut out = Sexp::NULL;
    let t = crate::host::try_op(op, &[], &mut out);
    if t != 0 {
        (crate::host::host().discard)(t);
    }
}

/// C's `%g` (messages that GNU R formats with it).
pub fn fmt_g(x: f64) -> String {
    let mut buf = [0u8; 64];
    // SAFETY: the size is the buffer's, and %g takes the one double passed.
    let n = unsafe { libc::snprintf(buf.as_mut_ptr() as *mut c_char, 64, c"%g".as_ptr(), x) };
    String::from_utf8_lossy(&buf[..n.clamp(0, 63) as usize]).into_owned()
}
