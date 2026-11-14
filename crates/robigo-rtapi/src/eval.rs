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
use robigo_rtcore::{Cx, RResult, Root, Scope, Sexp};
use std::ffi::c_int;

/// A symbol argument's name (`GetOption1`'s tag, `setAttrib`'s name), or
/// `installTrChar(STRING_ELT(name, 0))`'s for a string.
fn name_bytes(cx: &Cx, x: Root<'_>) -> Option<Vec<u8>> {
    let r = x.bind(cx);
    if let Some(s) = r.as_sym() {
        return Some(s.name().bytes().to_vec());
    }
    let v = r.as_vector::<robigo_rtcore::Char<'_>>()?;
    (v.len() > 0).then(|| v.get(0).bytes().to_vec())
}

/// `eval(e, rho)` into a root of `s`.
fn eval<'s>(cx: &mut Cx, s: Scope<'s>, e: Root<'_>, rho: Root<'_>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    robigo_rt::eval(cx, e, rho, o)?;
    Ok(r)
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
    let ok = cx.toplevel(global, base, |cx| robigo_rt::eval(cx, e, env, o))?;
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
        let Some(name) = tag.bind(cx).as_sym().map(|y| y.name().bytes().to_vec()) else {
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
            t if robigo_heap::is_vector_type(t) => Err(not_yet(cx, "the length of an ALTREP vector from C")),
            _ => {
                let t = r_type_to_char(r);
                Err(cx.error(&format!("LENGTH or similar applied to {t} object")))
            }
        }
    }
}
