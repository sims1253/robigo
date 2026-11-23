//! Evaluation and the rest of the API the capi cases use: eval.c's `eval`,
//! context.c's `R_tryEval`, source.c's `R_ParseVector`, options.c's
//! `GetOption1`, envir.c's `R_NewEnv` and `defineVar`, attrib.c's
//! `setAttrib`, coerce.c's `asInteger` & co, util.c's `nthcdr`, and the
//! slow paths of `api.c`'s `LENGTH`.

use crate::entry;
use crate::entry::OrNull;
use crate::errors::{not_yet, r_type_to_char};
use crate::memory::{alloc, safepoint};
use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, RError, RResult, Root, Scope, Sexp};
use std::ffi::{c_int, c_uint};

/// A symbol argument's name (`GetOption1`'s tag, `setAttrib`'s name), or
/// `installTrChar(STRING_ELT(name, 0))`'s for a string.
fn name_bytes(cx: &Cx, x: Root<'_>) -> Option<Vec<u8>> {
    let r = x.bind(cx);
    if let Some(s) = r.as_sym() {
        return Some(s.name_bytes().to_vec());
    }
    let v = r.as_vector::<robigo_rtcore::Char<'_>>()?;
    (!v.is_empty()).then(|| v.get(0).bytes().to_vec())
}

/// `eval(e, rho)` into a root of `s`.
fn eval<'s>(cx: &mut Cx, s: Scope<'s>, e: Root<'_>, rho: Root<'_>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    robigo_rt::eval(cx, e, rho, o)?;
    Ok(r)
}

/// `R_getVarEx(sym, rho, inherits, ifnotfound)` (`ifnotfound` Some: what
/// C passed, which may be a C NULL, returned as it is), and `R_getVar` over
/// it (None: R_UnboundValue, then its error). The checks' messages name
/// `R_getVarEx` (its `__func__`) for both, as GNU R's do.
// R: envir.c:R_getVarEx, R_getVar
fn get_var_ex(
    cx: &mut Cx,
    s: Scope<'_>,
    sym: Root<'_>,
    rho: Root<'_>,
    inherits: bool,
    ifnotfound: Option<Option<Root<'_>>>,
) -> RResult<Sexp> {
    if sym.bind(cx).sexptype() != SYMSXP {
        return Err(cx.error("first argument to 'R_getVarEx' must be a symbol"));
    }
    if rho.bind(cx).sexptype() != ENVSXP {
        return Err(cx.error("second argument to 'R_getVarEx' must be an environment"));
    }
    let o = s.out(cx);
    let val = o.root();
    if inherits {
        cx.find_var(sym, rho, o)?;
    } else {
        cx.find_var_in_frame3(rho, sym, true, o)?;
    }
    let v = val.bind(cx).sexp();
    if v == cx.missing_arg().sexp() {
        // R_MissingArgError_c(EncodeChar(PRINTNAME(sym)), getLexicalCall(rho), "getVarExError")
        let name = String::from_utf8_lossy(&name_bytes(cx, sym).unwrap_or_default()).into_owned();
        let call = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
        return Err(cx.missing_arg_error(&name, Some(call), "getVarExError"));
    }
    if v == cx.unbound().sexp() {
        return match ifnotfound {
            Some(x) => Ok(x.map_or(Sexp::NULL, |x| x.bind(cx).sexp())),
            None => {
                let name = String::from_utf8_lossy(&name_bytes(cx, sym).unwrap_or_default()).into_owned();
                Err(cx.error(&format!("object '{name}' not found")))
            }
        };
    }
    if val.bind(cx).sexptype() == PROMSXP {
        // (C code's eval of the promise: GNU R's eval(val, rho))
        let p = s.root(cx, val.bind(cx));
        let o = s.out(cx);
        let r = o.root();
        // (GNU R's PROTECT(val) around its eval)
        cx.with_pp_excess(1, |cx| cx.eval_promise(p, o))?;
        return Ok(r.bind(cx).sexp());
    }
    Ok(v)
}

/// `R_tryEval(e, env, ErrorOccurred)`: `e` evaluated in a top-level
/// context; an error is reported there (GNU R's default handler, before
/// the jump) and gives NULL with `*err` set.
// R: context.c:R_tryEval, protectedEval
fn try_eval(cx: &mut Cx, s: Scope<'_>, e: Root<'_>, env: Option<Root<'_>>, err: *mut c_int) -> RResult<Sexp> {
    let env = match env {
        Some(env) => env,
        None => s.root(cx, cx.global_env()),
    };
    let o = s.out(cx);
    let val = o.root();
    let global = s.root(cx, cx.global_env());
    let base = s.root(cx, cx.base_env());
    // R: context.c:protectedEval (eval, then R_PreserveObject of the value:
    // an allocation, where a collection may come; the value is rooted)
    let ok = cx.toplevel(global, base, |cx| {
        robigo_rt::eval(cx, e, env, o)?;
        safepoint(cx)
    })?;
    if !err.is_null() {
        // SAFETY: C's contract: an int to write, or NULL.
        unsafe { *err = (!ok) as c_int };
    }
    Ok(if ok { val.bind(cx).sexp() } else { Sexp::NULL })
}

entry! {
    /// `eval(e, rho)`.
    // R: eval.c:eval
    [2] fn Rf_eval(e: Sexp, rho: Sexp) -> Sexp => |cx, s| {
        Ok(eval(cx, s, e, rho)?.bind(cx).sexp())
    }

    /// `R_tryEval(e, env, ErrorOccurred)`.
    // R: context.c:R_tryEval
    [3] fn R_tryEval(e: Sexp, env: OrNull, err: *mut c_int) -> Sexp => |cx, s| {
        try_eval(cx, s, e, env, err)
    }

    /// `R_ParseVector(text, n, status, srcfile)`: robigo-rt is Rscript,
    /// where `keep.source` is FALSE, so `srcfile` adds no references.
    // R: source.c:R_ParseVector
    [4] fn R_ParseVector(text: Sexp, n: c_int, status: *mut c_int, srcfile: Sexp) -> Sexp => |cx, s| {
        let _ = srcfile;
        if text.bind(cx).sexptype() != STRSXP {
            return Err(not_yet(cx, "R_ParseVector of text that is not a character vector"));
        }
        safepoint(cx)?;
        let o = s.out(cx);
        let r = o.root();
        let (st, _) = robigo_rt::reader::parse_vector(cx, text, n, o)?;
        if !status.is_null() {
            // SAFETY: C's contract: a ParseStatus to write.
            unsafe { *status = st as c_int };
        }
        Ok(r.bind(cx).sexp())
    }

    /// `GetOption1(tag)`: the option's value, or NULL; allocates nothing.
    // R: options.c:GetOption1
    [1] fn Rf_GetOption1(tag: Sexp) -> Sexp => |cx, s| {
        let Some(name) = tag.bind(cx).as_sym().map(|y| y.name_bytes().to_vec()) else {
            return Err(not_yet(cx, "GetOption1 of a tag that is not a symbol"));
        };
        Ok(robigo_rtbuiltins::options::get_option(cx, s, &name)?.bind(cx).sexp())
    }

    /// `R_NewEnv(enclos, hash, size)`.
    // R: envir.c:R_NewEnv
    [3] fn R_NewEnv(enclos: Sexp, hash: c_int, size: c_int) -> Sexp => |cx, s| {
        alloc(cx, s, |cx| cx.new_env(enclos.bind(cx), hash != 0, size))
    }

    /// `defineVar(sym, val, rho)`: GNU R conses a new binding (and may
    /// resize a hashed frame) only for a symbol the frame does not bind,
    /// so only then does this collect first.
    // R: envir.c:defineVar
    [3] fn Rf_defineVar(sym: Sexp, val: Sexp, rho: Sexp) -> () => |cx, _s| {
        if rho.bind(cx).sexptype() == ENVSXP && sym.bind(cx).sexptype() == SYMSXP && !cx.exists_var_in_frame(rho, sym)? {
            safepoint(cx)?;
        }
        cx.define_var(sym, val, rho)
    }

    /// `setAttrib(vec, name, val)`: GNU R allocates the attribute's cell
    /// for an attribute `vec` does not have (the name is interned already
    /// for every name the cases use: `install` of a known name allocates
    /// nothing), so only then does this collect first.
    // R: attrib.c:setAttrib
    [3] fn Rf_setAttrib(vec: Sexp, name: Sexp, val: Sexp) -> Sexp => |cx, _s| {
        let Some(n) = name_bytes(cx, name) else {
            return Err(not_yet(cx, "setAttrib with a name that is neither a symbol nor a string"));
        };
        let has = robigo_rtbuiltins::attrib::get(vec.bind(cx), &n).is_some();
        if !has && val.bind(cx).sexptype() != NILSXP {
            safepoint(cx)?;
        }
        robigo_rtbuiltins::attrib::set::set_attrib(cx, vec, &n, val)?;
        Ok(val.bind(cx).sexp())
    }

    /// `getAttrib(vec, name)`: GNU R builds the result only for a
    /// pairlist's, call's or `...`'s names and for compact `c(NA, n)` row
    /// names, so only then does this collect first.
    // R: attrib.c:getAttrib
    [2] fn Rf_getAttrib(vec: Sexp, name: Sexp) -> Sexp => |cx, s| {
        let Some(n) = name_bytes(cx, name) else {
            return Err(not_yet(cx, "getAttrib with a name that is neither a symbol nor a string"));
        };
        let v = vec.bind(cx);
        let builds = if n == b"row.names" {
            robigo_rtbuiltins::attrib::get(v, b"row.names").is_some_and(|a| {
                a.value.as_vector::<i32>().is_some_and(|r| r.len() == 2 && r.get(0) == robigo_heap::NA_INTEGER)
            })
        } else {
            matches!(v.sexptype(), LISTSXP | LANGSXP | DOTSXP) && n == b"names"
        };
        if builds {
            safepoint(cx)?;
        }
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| {
            let v = robigo_rtbuiltins::attrib::get_attrib(cx, vec.bind(cx), &n)?;
            Ok(v.unwrap_or(cx.nil()))
        })?;
        Ok(r.bind(cx).sexp())
    }

    /// `asInteger(x)`, with coerce.c's warnings.
    // R: coerce.c:asInteger
    [1] fn Rf_asInteger(x: Sexp) -> c_int => |cx, _s| {
        robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, x)
    }

    /// `asLogical(x)`.
    // R: coerce.c:asLogical
    [1] fn Rf_asLogical(x: Sexp) -> c_int => |cx, _s| {
        Ok(robigo_rtbuiltins::coerce::vector::as_logical(cx, x.bind(cx)))
    }

    /// `asReal(x)`, with coerce.c's warnings.
    // R: coerce.c:asReal
    [1] fn Rf_asReal(x: Sexp) -> f64 => |cx, _s| {
        robigo_rtbuiltins::coerce::vector::as_real_warn(cx, x)
    }

    /// `nthcdr(s, n)`.
    // R: util.c:nthcdr
    [2] fn Rf_nthcdr(x: Sexp, n: c_int) -> Sexp => |cx, _s| {
        let r = x.bind(cx);
        let t = r.sexptype();
        let frame = t == VECSXP && robigo_rtbuiltins::attrib::inherits(r, b"data.frame");
        if !matches!(t, NILSXP | LISTSXP | LANGSXP | DOTSXP) && !frame {
            return Err(cx.error("'nthcdr' needs a list to CDR down"));
        }
        if frame {
            return Err(not_yet(cx, "nthcdr of a data frame"));
        }
        let mut c = r.sexp();
        let mut n = n;
        while n > 0 {
            n -= 1;
            if c == cx.nil().sexp() {
                return Err(cx.error(&format!("'nthcdr' list shorter than {n}")));
            }
            // SAFETY: a live pairlist cell (checked above for the first,
            // and every cdr of a pairlist is a cell or R_NilValue).
            c = unsafe { c.cdr() };
        }
        Ok(c)
    }

    /// `LENGTH`/`XLENGTH`/`length` of what `api.c` does not read itself:
    /// GNU R's "LENGTH or similar applied to %s object" for a non-vector, a
    /// package ALTREP's Length method, an environment's binding count.
    // R: memory.c:CHK2, Rinlinedfuns.h:length
    [1] fn robigo_rtapi_length_slow(x: Sexp) -> isize => |cx, _s| {
        let r = x.bind(cx);
        match r.sexptype() {
            ENVSXP => Err(not_yet(cx, "length of an environment from C")),
            // ALTREP_LENGTH: one of robigo's classes has its `Length` in
            // Rust (it runs no code); a package class's is not dispatched
            // from C yet
            t if robigo_heap::is_vector_type(t) => match r.len() {
                Some(n) => Ok(n as isize),
                None => Err(not_yet(cx, "the length of an ALTREP vector from C")),
            },
            _ => {
                let t = r_type_to_char(r);
                Err(cx.error(&format!("LENGTH or similar applied to {t} object")))
            }
        }
    }
}

/// `coerceVector(v, type)`'s COERCE_ERROR for a type code too large for any
/// SEXPTYPE, with the code the caller gave: type2char's fallback, its
/// warning and then "unknown type #N" (GNU R itself reads past its
/// type table for such a code).
// R: coerce.c:coerceVector (COERCE_ERROR), util.c:type2char
#[cold]
fn coerce_to_unknown(cx: &mut Cx, v: Root<'_>, t: c_uint) -> RError {
    let to = match robigo_rtbuiltins::util::type2char_warn(cx, t) {
        Ok(to) => to,
        Err(e) => return e,
    };
    let from = r_type_to_char(v.bind(cx));
    cx.error(&format!("cannot coerce type '{from}' to vector of type '{to}'"))
}

entry! {
    /// `coerceVector(v, type)`.
    // R: coerce.c:coerceVector
    [2] fn Rf_coerceVector(v: Sexp, t: c_uint) -> Sexp => |cx, s| {
        let o = s.out(cx);
        let r = o.root();
        let Ok(t8) = u8::try_from(t) else {
            return Err(coerce_to_unknown(cx, v, t));
        };
        // A code no type names: GNU R's type2char warns where the
        // coercion's error names the target (the error itself says
        // "unknown type #N", type2char's text), so the warning comes first,
        // here, for the coercions whose error does name it.
        if robigo_rtbuiltins::util::type_name(t8).is_none()
            && robigo_rtbuiltins::coerce::vector::error_names_target(v.bind(cx), t8)
        {
            robigo_rtbuiltins::util::type2char_warn(cx, t)?;
        }
        robigo_rtbuiltins::coerce::vector::coerce_vector(cx, v, t8, o)?;
        Ok(r.bind(cx).sexp())
    }

    /// `asBool(x)`.
    // R: coerce.c:asBool
    [1] fn Rf_asBool(x: Sexp) -> bool => |cx, _s| {
        robigo_rtbuiltins::util::as_bool(cx, x)
    }

    /// `findVar(sym, rho)`: the value bound to `sym` in `rho` or an
    /// enclosure (a promise as it is: unforced), R_UnboundValue if none.
    // R: envir.c:findVar
    [2] fn Rf_findVar(sym: Sexp, rho: Sexp) -> Sexp => |cx, s| {
        let o = s.out(cx);
        let r = o.root();
        cx.find_var(sym, rho, o)?;
        Ok(r.bind(cx).sexp())
    }

    /// `R_getVarEx(sym, rho, inherits, ifnotfound)`: `sym`'s value in
    /// `rho` (and its enclosures with `inherits`), a promise forced,
    /// `ifnotfound` when it is unbound; a missing argument is the
    /// `getVarExError` reported against `rho`'s lexical call.
    // R: envir.c:R_getVarEx
    [4] fn R_getVarEx(sym: Sexp, rho: Sexp, inherits: c_int, ifnotfound: OrNull) -> Sexp => |cx, s| {
        get_var_ex(cx, s, sym, rho, inherits != 0, Some(ifnotfound))
    }

    /// `R_getVar(sym, rho, inherits)`: [`R_getVarEx`]'s value, "object
    /// '%s' not found" when it is unbound.
    // R: envir.c:R_getVar
    [3] fn R_getVar(sym: Sexp, rho: Sexp, inherits: c_int) -> Sexp => |cx, s| {
        get_var_ex(cx, s, sym, rho, inherits != 0, None)
    }

    /// `R_NewHashedEnv(enclos, size)`: a hashed environment (`size` the
    /// table's initial size).
    // R: envir.c:R_NewHashedEnv
    [2] fn R_NewHashedEnv(enclos: Sexp, size: c_int) -> Sexp => |cx, s| {
        alloc(cx, s, |cx| cx.new_env(enclos.bind(cx), true, size))
    }
}

/// `isInteger(s)`: an integer vector that is not a factor.
///
/// # Safety
/// Called from C code a native entry of the thread's isolate runs, with
/// `s` a live object of that isolate (C's contract for the API).
// R: Rinlinedfuns.h:isInteger
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_isInteger(s: Sexp) -> c_int {
    // SAFETY: the caller's contract; `s` is read without allocating.
    unsafe {
        robigo_rtcore::enter_leaf(|cx| {
            let x = robigo_rtcore::Ref::from_sexp(s, cx);
            (x.sexptype() == INTSXP && !robigo_rtbuiltins::attrib::inherits(x, b"factor")) as c_int
        })
    }
}
