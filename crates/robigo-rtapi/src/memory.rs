//! memory.c's allocating API (and its neighbours: `mkChar*` from envir.c,
//! `install` from names.c, `duplicate`, `lengthgets`, the pairlist
//! conversions), external pointers, weak references, finalizers and the
//! precious set, with L8's allocation parity: each export collects on entry
//! when a collection is due exactly on the paths where GNU R's same
//! function allocates, and allocates nothing on the paths where GNU R's
//! does not (a known symbol, a cached string, `ScalarLogical`).

use crate::entry;
use crate::errors::not_yet;
use robigo_heap::LARGE;
use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, Encoding, NeedGc, RResult, Ref, Root, Safepoint, Scope, Sexp};
use std::ffi::{c_char, c_int, c_uint, c_void};

/// GNU R's `MAXIDSIZE` (Defn.h).
const MAXIDSIZE: usize = 10000;
/// `R_XLEN_T_MAX` (Rinternals.h).
const R_XLEN_T_MAX: isize = 4503599627370496;

/// The collection an allocating export makes on entry when one is due
/// (docs/g3.md §3 item 6): before its first allocation, where GNU R's
/// allocation could collect.
#[inline(always)]
pub(crate) fn safepoint(cx: &mut Cx) -> RResult<()> {
    cx.safepoint(Safepoint::Export)
}

/// Build an export's result under `&Cx` (rerun after a collection if the
/// heap refused an allocation, then GNU R's "cannot allocate" error) and
/// hand it back raw: C holds it from here.
#[inline]
pub(crate) fn build<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    f: impl for<'g> FnMut(&'g Cx) -> Result<Ref<'g>, NeedGc>,
) -> RResult<Sexp> {
    let out = s.out(cx);
    let r = out.root();
    cx.build(out, f)?;
    Ok(r.bind(cx).sexp())
}

/// [`safepoint`] then [`build`]: an export that always allocates.
#[inline]
pub(crate) fn alloc<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    f: impl for<'g> FnMut(&'g Cx) -> Result<Ref<'g>, NeedGc>,
) -> RResult<Sexp> {
    safepoint(cx)?;
    build(cx, s, f)
}

/// A vector of type `t` and length `n` for C: `allocVector3`'s errors and
/// its non-vector types (NULL, pairlists), then the allocation, through
/// `alloc_rooted` for [`LARGE`] requests.
// R: memory.c:allocVector3
fn alloc_vector(cx: &mut Cx, s: Scope<'_>, t: c_uint, n: isize) -> RResult<Sexp> {
    let t8 = u8::try_from(t).unwrap_or(u8::MAX);
    // the scalars GNU R allocates first collect before its checks
    if n == 1 && matches!(t8, REALSXP | INTSXP | LGLSXP) {
        return alloc(cx, s, |cx| fresh_vector(cx, t8, 1));
    }
    if n > R_XLEN_T_MAX {
        return Err(cx.error(&format!("cannot allocate vector of length {n}")));
    }
    if n < 0 {
        return Err(cx.error("negative length vectors are not allowed"));
    }
    let elt = match t8 {
        NILSXP => return Ok(cx.nil().sexp()),
        CHARSXP => return Err(cx.error("use of allocVector(CHARSXP ...) is defunct\n")),
        LANGSXP if n == 0 => return Ok(cx.nil().sexp()),
        LANGSXP | LISTSXP => {
            if n > i32::MAX as isize {
                return Err(cx.error("invalid length for pairlist"));
            }
            return alloc_list(cx, s, n as usize, t8 == LANGSXP);
        }
        RAWSXP => 1,
        LGLSXP | INTSXP => 4,
        REALSXP => 8,
        CPLXSXP => 16,
        STRSXP | EXPRSXP | VECSXP => 8,
        _ => {
            let msg =
                format!("invalid type/length ({}/{n}) in vector allocation", robigo_rtbuiltins::util::type2char(t8));
            return Err(cx.error(&msg));
        }
    };
    let n = n as usize;
    if n.saturating_mul(elt) >= LARGE {
        // GNU R collects before a large allocation (memory.c:2709-2714)
        return cx.alloc_vector_rooted_raw(t8, n);
    }
    alloc(cx, s, |cx| fresh_vector(cx, t8, n))
}

/// `allocList(n)` (`allocLang` when `lang`: the first cell a call): `n`
/// cells of R_NilValue.
// R: memory.c:allocList, allocLang
fn alloc_list(cx: &mut Cx, s: Scope<'_>, n: usize, lang: bool) -> RResult<Sexp> {
    if n == 0 {
        return Ok(cx.nil().sexp());
    }
    alloc(cx, s, |cx| {
        let nil = cx.nil();
        let mut r = nil;
        for i in 0..n {
            r = if lang && i == n - 1 { cx.lcons(nil, r)? } else { cx.cons(nil, r)? };
        }
        Ok(r)
    })
}

/// `mkCharLenCE(name, len, enc)`: the cached CHARSXP, allocating (and
/// collecting first if due) only for a string the cache lacks.
// R: envir.c:mkCharLenCE
fn mk_char_len_ce(cx: &mut Cx, s: Scope<'_>, name: *const c_char, len: c_int, enc: c_int) -> RResult<Sexp> {
    let enc = match enc {
        0 | 5 | 99 => Encoding::Native,
        1 => Encoding::Utf8,
        2 => Encoding::Latin1,
        3 => Encoding::Bytes,
        _ => return Err(cx.error(&format!("unknown encoding: {enc}"))),
    };
    let bytes: &[u8] = if len <= 0 || name.is_null() {
        &[]
    } else {
        // SAFETY: C's contract: `len` bytes at `name`.
        unsafe { std::slice::from_raw_parts(name.cast::<u8>(), len as usize) }
    };
    if bytes.contains(&0) {
        // GNU R's EncodeString(c, 0, 0, Rprt_adj_none): a NUL is "\0",
        // printable ASCII itself; robigo has the rest of it for printing
        // only (robigo-text's needs print parameters)
        if !bytes.iter().all(|&b| b == 0 || (0x20..0x7f).contains(&b) && b != b'\\') {
            return Err(not_yet(cx, "the message of an embedded nul in a string with other escapes"));
        }
        let shown: String =
            bytes.iter().map(|&b| if b == 0 { "\\0".to_owned() } else { (b as char).to_string() }).collect();
        return Err(cx.error(&format!("embedded nul in string: '{shown}'")));
    }
    if let Some(c) = cx.find_char(bytes, enc) {
        return Ok(c.as_ref().sexp());
    }
    alloc(cx, s, |cx| Ok(cx.mk_char(bytes, enc)?.as_ref()))
}

/// `strlen(name)` for `mkChar` and `mkCharCE`, with GNU R's limit.
fn c_len(cx: &mut Cx, name: *const c_char) -> RResult<c_int> {
    // SAFETY: C's contract: a NUL-terminated string.
    let n = unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes().len();
    c_int::try_from(n).map_err(|_| cx.error("R character strings are limited to 2^31-1 bytes"))
}

/// `install(name)`: the symbol, allocating (and collecting first if due)
/// only for a name the isolate has not interned.
// R: names.c:install
fn install(cx: &mut Cx, s: Scope<'_>, name: &[u8]) -> RResult<Sexp> {
    if let Some(y) = cx.find_symbol(name) {
        return Ok(y.as_ref().sexp());
    }
    if name.is_empty() {
        return Err(cx.error("attempt to use zero-length variable name"));
    }
    if name.len() > MAXIDSIZE {
        return Err(cx.error(&format!("variable names are limited to {MAXIDSIZE} bytes")));
    }
    alloc(cx, s, |cx| Ok(cx.install(name)?.as_ref()))
}

/// The cons of `car` and `cdr` (a call's cell when `lang`).
fn cons(cx: &mut Cx, s: Scope<'_>, car: Root<'_>, cdr: Root<'_>, lang: bool) -> RResult<Sexp> {
    alloc(cx, s, |cx| if lang { cx.lcons(car.bind(cx), cdr.bind(cx)) } else { cx.cons(car.bind(cx), cdr.bind(cx)) })
}

/// `listN(a...)` or `langN(a...)`: the cells of `items`, the first a call's
/// for `lang`.
fn list_of(cx: &mut Cx, s: Scope<'_>, items: &[Root<'_>], lang: bool) -> RResult<Sexp> {
    alloc(cx, s, |cx| {
        let mut r = cx.nil();
        for (i, x) in items.iter().enumerate().rev() {
            r = if lang && i == 0 { cx.lcons(x.bind(cx), r)? } else { cx.cons(x.bind(cx), r)? };
        }
        Ok(r)
    })
}

/// A length-one vector of `t` holding `v`.
fn scalar<T: robigo_rtcore::Plain + Copy>(cx: &mut Cx, s: Scope<'_>, v: T) -> RResult<Sexp> {
    alloc(cx, s, |cx| Ok(cx.alloc_copy(&[v])?.finish().into()))
}

/// Whether GNU R's `duplicate1` copies an object of type `t` (the others
/// it returns as they are, allocating nothing).
// R: duplicate.c:duplicate1
fn duplicates(t: u8) -> bool {
    !matches!(
        t,
        NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP | CHARSXP | PROMSXP
    )
}

/// `R_MakeWeakRef`'s and `R_MakeWeakRefC`'s `NewWeakRef(key, val, fin,
/// onexit)`: GNU R's key check, the value duplicated if it is referenced,
/// then the reference, registered with the collector.
// R: memory.c:NewWeakRef
fn new_weak_ref<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    key: Root<'_>,
    val: Root<'_>,
    fin: Root<'_>,
    onexit: bool,
) -> RResult<Sexp> {
    if !matches!(key.bind(cx).sexptype(), NILSXP | ENVSXP | EXTPTRSXP | BCODESXP) {
        return Err(cx.error("can only weakly reference/finalize reference objects"));
    }
    safepoint(cx)?;
    let val = if val.bind(cx).maybe_referenced() {
        let o = s.out(cx);
        let r = o.root();
        cx.duplicate(val, o)?;
        r
    } else {
        val
    };
    let mut retried = None;
    loop {
        let (k, v, f) = (key.bind(cx).sexp(), val.bind(cx).sexp(), fin.bind(cx).sexp());
        // SAFETY: rooted objects of this isolate; the reference is rooted by
        // the collector's list once made (and by C, which gets it).
        match unsafe { cx.new_weak_ref_raw(k, v, f, onexit) } {
            Ok(w) => return Ok(w),
            // (need_gc raises GNU R's error when a collection cannot make
            // the room; a second refusal after one that did is the same)
            Err(e) if retried == Some(e) => {
                cx.need_gc(e)?;
                return Err(cx.error_call(None, "robigo: a weak reference the heap refused twice"));
            }
            Err(e) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
        }
    }
}

/// `MakeCFinalizer(fun)`: GNU R keeps a C finalizer's address in a raw
/// vector, which `R_RunWeakRefFinalizer` recognizes.
// R: memory.c:MakeCFinalizer
fn c_finalizer<'s>(cx: &mut Cx, s: Scope<'s>, fun: *const c_void) -> RResult<Root<'s>> {
    let bytes = (fun as usize).to_ne_bytes();
    let out = s.out(cx);
    let r = out.root();
    cx.build(out, |cx| Ok(cx.alloc_copy(&bytes)?.finish().into()))?;
    Ok(r)
}

/// `R_RegisterFinalizerEx(s, fun, onexit)` of an R function.
// R: memory.c:R_RegisterFinalizerEx
fn register_finalizer(cx: &mut Cx, s: Root<'_>, fun: Root<'_>, onexit: bool) -> RResult<()> {
    robigo_rt::fin::register_finalizer(cx, s, fun, onexit)
}

entry! {
    /// `allocVector(type, length)`.
    // R: memory.c:allocVector3
    [2] fn Rf_allocVector(t: c_uint, n: isize) -> Sexp => |cx, s| {
        alloc_vector(cx, s, t, n)
    }

    /// `allocList(n)`.
    // R: memory.c:allocList
    [1] fn Rf_allocList(n: c_int) -> Sexp => |cx, s| {
        alloc_list(cx, s, n.max(0) as usize, false)
    }

    /// `cons(car, cdr)` (`CONS`).
    // R: memory.c:cons
    [2] fn Rf_cons(car: Sexp, cdr: Sexp) -> Sexp => |cx, s| {
        cons(cx, s, car, cdr, false)
    }

    /// `lcons(car, cdr)` (`LCONS`).
    // R: Rinlinedfuns.h:lcons
    [2] fn Rf_lcons(car: Sexp, cdr: Sexp) -> Sexp => |cx, s| {
        cons(cx, s, car, cdr, true)
    }

    // R: Rinlinedfuns.h:list1..list4, lang1..lang4
    [1] fn Rf_list1(a: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a], false) }
    [2] fn Rf_list2(a: Sexp, b: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a, b], false) }
    [3] fn Rf_list3(a: Sexp, b: Sexp, c: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a, b, c], false) }
    [4] fn Rf_list4(a: Sexp, b: Sexp, c: Sexp, d: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a, b, c, d], false) }
    [1] fn Rf_lang1(a: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a], true) }
    [2] fn Rf_lang2(a: Sexp, b: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a, b], true) }
    [3] fn Rf_lang3(a: Sexp, b: Sexp, c: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a, b, c], true) }
    [4] fn Rf_lang4(a: Sexp, b: Sexp, c: Sexp, d: Sexp) -> Sexp => |cx, s| { list_of(cx, s, &[a, b, c, d], true) }

    /// `mkChar(name)`.
    // R: envir.c:mkChar
    [1] fn Rf_mkChar(name: *const c_char) -> Sexp => |cx, s| {
        let n = c_len(cx, name)?;
        mk_char_len_ce(cx, s, name, n, 0)
    }

    /// `mkCharCE(name, enc)`.
    // R: envir.c:mkCharCE
    [2] fn Rf_mkCharCE(name: *const c_char, enc: c_int) -> Sexp => |cx, s| {
        let n = c_len(cx, name)?;
        mk_char_len_ce(cx, s, name, n, enc)
    }

    /// `mkCharLen(name, len)`.
    // R: envir.c:mkCharLen
    [2] fn Rf_mkCharLen(name: *const c_char, len: c_int) -> Sexp => |cx, s| {
        mk_char_len_ce(cx, s, name, len, 0)
    }

    /// `mkCharLenCE(name, len, enc)`.
    // R: envir.c:mkCharLenCE
    [3] fn Rf_mkCharLenCE(name: *const c_char, len: c_int, enc: c_int) -> Sexp => |cx, s| {
        mk_char_len_ce(cx, s, name, len, enc)
    }

    /// `mkString(s)`: the character vector first (which collects if due),
    /// then the string.
    // R: Rinlinedfuns.h:mkString
    [1] fn Rf_mkString(name: *const c_char) -> Sexp => |cx, s| {
        // SAFETY: C's contract: a NUL-terminated string.
        let bytes = unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes();
        alloc(cx, s, |cx| {
            let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
            v.set(0, cx.mk_char(bytes, Encoding::Native)?);
            Ok(v.finish().into())
        })
    }

    /// `ScalarInteger(x)`.
    // R: Rinlinedfuns.h:ScalarInteger
    [1] fn Rf_ScalarInteger(x: c_int) -> Sexp => |cx, s| { scalar(cx, s, x) }

    /// `ScalarReal(x)`.
    // R: Rinlinedfuns.h:ScalarReal
    [0] fn Rf_ScalarReal(x: f64) -> Sexp => |cx, s| { scalar(cx, s, x) }

    /// `ScalarLogical(x)`: GNU R's shared `R_TrueValue`, `R_FalseValue` or
    /// `R_LogicalNAValue`, allocating nothing.
    // R: Rinlinedfuns.h:ScalarLogical
    [1] fn Rf_ScalarLogical(x: c_int) -> Sexp => |cx, _s| {
        Ok(cx.logical_value(x).sexp())
    }

    /// `ScalarString(x)` of a CHARSXP.
    // R: Rinlinedfuns.h:ScalarString
    [1] fn Rf_ScalarString(x: Sexp) -> Sexp => |cx, s| {
        alloc(cx, s, |cx| {
            let Some(c) = x.bind(cx).as_char() else {
                // SET_STRING_ELT's check
                return Ok(cx.nil());
            };
            let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
            v.set(0, c);
            Ok(v.finish().into())
        })
        .and_then(|r| {
            if r == cx.nil().sexp() {
                Err(crate::errors::charsxp_error(cx, x))
            } else {
                Ok(r)
            }
        })
    }

    /// `install(name)`.
    // R: names.c:install
    [1] fn Rf_install(name: *const c_char) -> Sexp => |cx, s| {
        // SAFETY: C's contract: a NUL-terminated string.
        let bytes = unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes();
        install(cx, s, bytes)
    }

    /// `duplicate(x)`.
    // R: duplicate.c:duplicate
    [1] fn Rf_duplicate(x: Sexp) -> Sexp => |cx, s| {
        if !duplicates(x.bind(cx).sexptype()) {
            return Ok(x.bind(cx).sexp());
        }
        safepoint(cx)?;
        let o = s.out(cx);
        let r = o.root();
        cx.duplicate(x, o)?;
        Ok(r.bind(cx).sexp())
    }

    /// `shallow_duplicate(x)`.
    // R: duplicate.c:shallow_duplicate
    [1] fn Rf_shallow_duplicate(x: Sexp) -> Sexp => |cx, s| {
        if !duplicates(x.bind(cx).sexptype()) {
            return Ok(x.bind(cx).sexp());
        }
        safepoint(cx)?;
        let o = s.out(cx);
        let r = o.root();
        cx.shallow_duplicate(x, o)?;
        Ok(r.bind(cx).sexp())
    }

    /// `lengthgets(x, len)`.
    // R: builtin.c:lengthgets
    [2] fn Rf_lengthgets(x: Sexp, len: c_int) -> Sexp => |cx, s| {
        lengthgets(cx, s, x, len as isize)
    }

    /// `xlengthgets(x, len)`.
    // R: builtin.c:xlengthgets
    [2] fn Rf_xlengthgets(x: Sexp, len: isize) -> Sexp => |cx, s| {
        lengthgets(cx, s, x, len)
    }

    /// `PairToVectorList(x)`.
    // R: coerce.c:PairToVectorList
    [1] fn Rf_PairToVectorList(x: Sexp) -> Sexp => |cx, s| {
        alloc(cx, s, |cx| robigo_rtbuiltins::coerce::vector::pair_to_vector_list(cx, x.bind(cx)))
    }

    /// `VectorToPairList(x)`.
    // R: coerce.c:VectorToPairList
    [1] fn Rf_VectorToPairList(x: Sexp) -> Sexp => |cx, s| {
        alloc(cx, s, |cx| robigo_rtbuiltins::coerce::vector::vector_to_pair_list(cx, x.bind(cx)))
    }

    /// `R_PreserveObject(x)`: GNU R conses onto its precious list, so this
    /// collects first if due.
    // R: memory.c:R_PreserveObject
    [1] fn R_PreserveObject(x: Sexp) -> () => |cx, _s| {
        safepoint(cx)?;
        let o = x.bind(cx).sexp();
        // SAFETY: a rooted object of this isolate.
        unsafe { cx.preserve_raw(o) };
        Ok(())
    }

    /// `R_alloc(nelem, eltsize)`: a block of the R_alloc stack, freed by
    /// `vmaxset` (a `.Call`'s end, an exit's stop).
    // R: memory.c:R_alloc
    [2] fn R_alloc(nelem: usize, eltsize: c_int) -> *mut c_void => |cx, s| {
        let size = (nelem as f64) * (eltsize.max(0) as f64);
        if size <= 0.0 {
            return Ok(std::ptr::null_mut());
        }
        if size >= R_XLEN_T_MAX as f64 {
            return Err(cx.error(&format!("cannot allocate memory block of size {:.1} Gb", size / 1024.0 / 1024.0 / 1024.0)));
        }
        // a raw vector of size + 1 bytes, or doubles for alignment past
        // 8 bytes (GNU R: a REALSXP when size + 1 does not fit a raw
        // vector's 8-byte alignment)
        let bytes = size as usize + 1;
        let x = if bytes >= LARGE {
            cx.alloc_vector_rooted_raw(RAWSXP, bytes)?
        } else {
            safepoint(cx)?;
            build(cx, s, |cx| fresh_vector(cx, RAWSXP, bytes))?
        };
        // C keeps the block's address, wherever it likes: under
        // ROBIGO_GC_MOVE=1 the block must not move
        cx.hand_to_c(x);
        // SAFETY: a fresh raw vector of this heap, with no attributes.
        unsafe {
            cx.push_vmax(x);
            Ok(x.std_data().cast())
        }
    }

    /// `R_gc()`: a full collection, then `R_RunPendingFinalizers()`.
    // R: memory.c:R_gc
    [0] fn R_gc() -> () => |cx, _s| {
        robigo_rt::fin::gc(cx);
        Ok(())
    }

    /// `R_RunPendingFinalizers()`.
    // R: memory.c:R_RunPendingFinalizers
    [0] fn R_RunPendingFinalizers() -> () => |cx, _s| {
        robigo_rt::fin::run_pending_finalizers(cx);
        Ok(())
    }

    /// `R_MakeExternalPtr(p, tag, prot)`.
    // R: memory.c:R_MakeExternalPtr
    [3] fn R_MakeExternalPtr(p: *mut c_void, tag: Sexp, prot: Sexp) -> Sexp => |cx, s| {
        alloc(cx, s, |cx| {
            let (nil, t, pr) = (cx.nil().sexp(), tag.bind(cx).sexp(), prot.bind(cx).sexp());
            // SAFETY: rooted objects of this isolate; an EXTPTRSXP's car is
            // its address (untraced), its cdr the protected value, its tag
            // the tag (memory.c's EXTPTR_PROT and EXTPTR_TAG).
            unsafe {
                let x = cx.alloc_node_raw(EXTPTRSXP, nil, pr, t)?;
                x.set_extptr_addr(p.cast());
                Ok(Ref::from_sexp(x, cx))
            }
        })
    }

    /// `R_MakeWeakRef(key, val, fin, onexit)`.
    // R: memory.c:R_MakeWeakRef
    [4] fn R_MakeWeakRef(key: Sexp, val: Sexp, fin: Sexp, onexit: c_int) -> Sexp => |cx, s| {
        if !matches!(fin.bind(cx).sexptype(), NILSXP | CLOSXP | BUILTINSXP | SPECIALSXP) {
            return Err(cx.error("finalizer must be a function or NULL"));
        }
        new_weak_ref(cx, s, key, val, fin, onexit != 0)
    }

    /// `R_MakeWeakRefC(key, val, fin, onexit)`.
    // R: memory.c:R_MakeWeakRefC
    [4] fn R_MakeWeakRefC(key: Sexp, val: Sexp, fin: *const c_void, onexit: c_int) -> Sexp => |cx, s| {
        safepoint(cx)?;
        let f = c_finalizer(cx, s, fin)?;
        new_weak_ref(cx, s, key, val, f, onexit != 0)
    }

    /// `R_RegisterCFinalizerEx(s, fun, onexit)`: `R_MakeWeakRefC(s,
    /// R_NilValue, fun, onexit)`.
    // R: memory.c:R_RegisterCFinalizerEx
    [3] fn R_RegisterCFinalizerEx(x: Sexp, fun: *const c_void, onexit: c_int) -> () => |cx, s| {
        safepoint(cx)?;
        let f = c_finalizer(cx, s, fun)?;
        let nil = s.root(cx, cx.nil());
        new_weak_ref(cx, s, x, nil, f, onexit != 0).map(|_| ())
    }

    /// `R_RegisterCFinalizer(s, fun)`.
    // R: memory.c:R_RegisterCFinalizer
    [2] fn R_RegisterCFinalizer(x: Sexp, fun: *const c_void) -> () => |cx, s| {
        safepoint(cx)?;
        let f = c_finalizer(cx, s, fun)?;
        let nil = s.root(cx, cx.nil());
        new_weak_ref(cx, s, x, nil, f, false).map(|_| ())
    }

    /// `R_RegisterFinalizerEx(s, fun, onexit)` of an R function.
    // R: memory.c:R_RegisterFinalizerEx
    [3] fn R_RegisterFinalizerEx(x: Sexp, fun: Sexp, onexit: c_int) -> () => |cx, _s| {
        safepoint(cx)?;
        register_finalizer(cx, x, fun, onexit != 0)
    }

    /// `R_RegisterFinalizer(s, fun)`.
    // R: memory.c:R_RegisterFinalizer
    [2] fn R_RegisterFinalizer(x: Sexp, fun: Sexp) -> () => |cx, _s| {
        safepoint(cx)?;
        register_finalizer(cx, x, fun, false)
    }
}

/// `xlengthgets(x, len)` with builtin.c's checks.
// R: builtin.c:xlengthgets
fn lengthgets(cx: &mut Cx, s: Scope<'_>, x: Root<'_>, len: isize) -> RResult<Sexp> {
    let r = x.bind(cx);
    let t = r.sexptype();
    let vector = robigo_heap::is_vector_type(t) && t != CHARSXP;
    if !vector && !matches!(t, NILSXP | LISTSXP) {
        return Err(cx.error("cannot set length of non-(vector or list)"));
    }
    if len < 0 {
        return Err(cx.error("invalid value"));
    }
    if t == NILSXP && len > 0 {
        cx.warning("length of NULL cannot be changed")?;
    }
    if x.bind(cx).is_altrep() {
        return Err(not_yet(cx, "lengthgets of an ALTREP vector"));
    }
    let lenx = match t {
        NILSXP => 0,
        LISTSXP => std::iter::successors(x.bind(cx).as_cons(), |c| c.cdr().as_cons()).count(),
        _ => x.bind(cx).len().unwrap_or(0),
    };
    let len = len as usize;
    if lenx == len {
        return Ok(x.bind(cx).sexp());
    }
    alloc(cx, s, |cx| xlengthgets(cx, x.bind(cx), lenx, len))
}

/// builtin.c's `xlengthgets` after its checks: a new object of `x`'s type
/// and length `len`, `x`'s first elements, then NA (raw 0, list and
/// pairlist NULL); the names likewise, padded with "" (no other attribute
/// is kept). `lenx` is `xlength(x)`.
// R: builtin.c:xlengthgets
fn xlengthgets<'g>(cx: &'g Cx, x: Ref<'g>, lenx: usize, len: usize) -> Result<Ref<'g>, NeedGc> {
    use robigo_rtcore::{Char, Lgl, Rcomplex, View};
    let t = x.sexptype();
    if t == NILSXP {
        return fresh_vector(cx, NILSXP, len);
    }
    if t == LISTSXP {
        let cells: Vec<_> = std::iter::successors(x.as_cons(), |c| c.cdr().as_cons()).collect();
        let nil = cx.nil();
        let mut r = nil;
        for i in (0..len).rev() {
            r = match cells.get(i) {
                Some(c) => match c.tag().as_sym() {
                    Some(tag) => cx.cons_tagged(c.car(), r, tag)?,
                    None => cx.cons(c.car(), r)?,
                },
                None => cx.cons(nil, r)?,
            };
        }
        return Ok(r);
    }
    let names = robigo_rtbuiltins::attrib::names(x).and_then(|n| n.as_vector::<Char<'g>>());
    macro_rules! plain {
        ($ty:ty, $v:expr, $na:expr) => {{
            let v = $v;
            cx.alloc_with::<$ty>(len, |i| if i < lenx { v.get(i) } else { $na })?.finish().into()
        }};
    }
    let r: Ref<'g> = match x.view() {
        View::Lgl(v) => plain!(Lgl, v, Lgl::NA),
        View::Int(v) => plain!(i32, v, robigo_heap::NA_INTEGER),
        View::Real(v) => plain!(f64, v, robigo_heap::NA_REAL),
        View::Cplx(v) => plain!(Rcomplex, v, Rcomplex { r: robigo_heap::NA_REAL, i: robigo_heap::NA_REAL }),
        View::Raw(v) => plain!(u8, v, 0),
        View::Str(v) => {
            let mut r = cx.alloc_vector::<Char<'g>>(len)?;
            for i in 0..len {
                r.set(i, if i < lenx { v.get(i) } else { cx.na_string() });
            }
            r.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let mut r = if t == EXPRSXP { cx.alloc_expression(len)? } else { cx.alloc_vector::<Ref<'g>>(len)? };
            for i in 0..lenx.min(len) {
                r.set(i, v.get(i));
            }
            r.finish().into()
        }
        _ => return Ok(x),
    };
    if let Some(nm) = names {
        let mut n = cx.alloc_vector::<Char<'g>>(len)?;
        for i in 0..lenx.min(len) {
            n.set(i, nm.get(i));
        }
        let names = n.finish();
        let sym = cx.install(b"names")?;
        let attr = cx.cons_tagged(names.into(), cx.nil(), sym)?;
        // SAFETY: `r` is fresh: its attribute slot takes no barrier, and
        // the pairlist is fresh too.
        unsafe { r.sexp().set_attrib(attr.sexp()) };
    }
    Ok(r)
}

/// A fresh vector of type `t` and length `n` under `&Cx` (NULL for NILSXP).
fn fresh_vector(cx: &Cx, t: u8, n: usize) -> Result<Ref<'_>, NeedGc> {
    if t == NILSXP {
        return Ok(cx.nil());
    }
    let x = cx.alloc_vector_raw(t, n)?;
    // SAFETY: fresh, of this heap.
    Ok(unsafe { Ref::from_sexp(x, cx) })
}

/// `R_ReleaseObject(x)`: cannot collect (GNU R's `DeleteFromList` allocates
/// nothing).
// R: memory.c:R_ReleaseObject
#[unsafe(no_mangle)]
pub extern "C" fn R_ReleaseObject(x: Sexp) {
    // SAFETY: called from C code of the thread's isolate; releasing does
    // not collect.
    unsafe { robigo_rtcore::enter_leaf_mut(|cx| cx.release_raw(x)) }
}

/// `vmaxget()`: R_alloc's stack top.
// R: memory.c:vmaxget
#[unsafe(no_mangle)]
pub extern "C" fn vmaxget() -> *mut c_void {
    // SAFETY: called from C code of the thread's isolate.
    unsafe { robigo_rtcore::enter_leaf(|cx| cx.vmax().0.cast()) }
}

/// `vmaxset(v)`: drop the R_alloc blocks made since `vmaxget()` returned
/// `v`.
// R: memory.c:vmaxset
#[unsafe(no_mangle)]
pub extern "C" fn vmaxset(v: *const c_void) {
    // SAFETY: called from C code of the thread's isolate; `v` is what
    // vmaxget returned in a frame still live (C's contract), or null.
    unsafe {
        robigo_rtcore::enter_leaf_mut(|cx| {
            let x = if v.is_null() { cx.nil().sexp() } else { Sexp(v.cast_mut().cast()) };
            cx.set_vmax(x);
        })
    }
}
