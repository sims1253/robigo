//! dotcode.c's `.Call` (docs/g3.md §14.2 M1): the routine found by name
//! in the table of loaded objects (rdynload.rs: `R_FindSymbol`, registered
//! routines first, then `dlsym`), and called through the heap's 65-argument
//! native entry (robigo-rtcore's `call_native65`) from a `CTXT_BUILTIN`
//! record, as GNU R's `eval` gives `PP_FOREIGN` builtins one.

use crate::errors::not_yet;
use robigo_rt::prims::{Inline, Kind, PrimDef};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, Sexp};
use std::ffi::c_void;

// dotcode.c's `MAX_ARGS`: robigo-rtcore's, the native entry's own bound
use robigo_rtcore::NATIVE_MAX_ARGS as MAX_ARGS;

/// `.Call` bound in `cx`'s isolate.
pub(crate) fn install(cx: &mut Cx) {
    robigo_rt::define_builtins(
        cx,
        &[
            // names.c: {".Call", do_dotcall, 0, 1, -1, {PP_FOREIGN, PREC_FN, 0}}
            PrimDef { name: ".Call", kind: Kind::Builtin(do_dotcall, Inline::None), eval: 1, arity: -1 },
        ],
    );
}

/// dotcode.c's `pkgtrim`: the `PACKAGE` argument's DLL name (or none),
/// and the indices of the arguments the routine gets (`None`: all after the
/// name, the usual case).
// R: dotcode.c:pkgtrim, setDLLname
fn pkgtrim(cx: &mut Cx, args: Args<'_>) -> RResult<(Option<String>, Option<Vec<usize>>)> {
    let named = (1..args.len()).any(|i| args.name(cx, i).is_some_and(|n| n.name().bytes() == b"PACKAGE"));
    if !named {
        return Ok((None, None));
    }
    let mut dll = None;
    let mut used = 0;
    let mut keep = Vec::new();
    for i in 1..args.len() {
        let is_pkg = args.name(cx, i).is_some_and(|n| n.name().bytes() == b"PACKAGE");
        if !is_pkg {
            keep.push(i);
            continue;
        }
        if used == 1 {
            cx.warning("'PACKAGE' used more than once")?;
        }
        used += 1;
        let Some(name) =
            args.get(cx, i).as_vector::<Char<'_>>().filter(|v| v.len() == 1).map(|v| v.get(0).bytes().to_vec())
        else {
            return Err(cx.error("PACKAGE argument must be a single character string"));
        };
        let name = String::from_utf8_lossy(&name).into_owned();
        let name = name.strip_prefix("package:").map(str::to_owned).unwrap_or(name);
        if name.len() > 4095 {
            return Err(cx.error("PACKAGE argument is too long"));
        }
        dll = Some(name);
    }
    Ok((dll, Some(keep)))
}

/// `.Call(.NAME, ...)`.
// R: dotcode.c:do_dotcall, resolveNativeRoutine, R_doDotCall; eval.c's
// PP_FOREIGN builtin call (vmaxset; its check_stack_balance is the
// evaluator's, where GNU R's `eval` makes it)
fn do_dotcall(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.is_empty() {
        return Err(cx.error_call(Some(call), "'.NAME' is missing"));
    }
    if args.name(cx, 0).is_some() {
        return Err(cx.error_call(Some(call), "the first argument should not be named"));
    }
    let op = args.get(cx, 0);
    let valid = op.as_vector::<Char<'_>>().is_some_and(|v| !v.is_empty());
    let opt = op.sexptype();
    if !valid {
        if opt == EXTPTRSXP || opt == VECSXP {
            return Err(not_yet(cx, ".Call of a native symbol reference"));
        }
        return Err(
            cx.error_call(Some(call), "first argument must be a string (of length 1) or native symbol reference")
        );
    }
    let (dll, keep) = pkgtrim(cx, args)?;
    if dll.as_deref() == Some("") {
        return Err(cx.error_call(Some(call), "PACKAGE = \"\" is invalid"));
    }
    // the name (MaxSymbolBytes): bytes of the first element, read in
    // this interval
    let mut buf = [0u8; 1024];
    let name: &[u8] = {
        let b = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(0).bytes();
        if b.len() >= buf.len() {
            let n = String::from_utf8_lossy(b).into_owned();
            return Err(cx.error(&format!("symbol '{n}' is too long")));
        }
        buf[..b.len()].copy_from_slice(b);
        &buf[..b.len()]
    };
    // called from a namespace: only its own DLL (R_FindNativeSymbolFromDLL)
    let ns = {
        let e = env.bind(cx).sexp();
        // SAFETY: a builtin's environment: a live ENVSXP, whose ENCLOS is a
        // fixed field.
        e != cx.nil().sexp() && unsafe { robigo_heap::env::enclos(e) } == cx.base_namespace().sexp()
    };
    let shown = || String::from_utf8_lossy(name).into_owned();
    let f = if dll.is_none() && ns {
        crate::rdynload::find_call_symbol(cx, name, Some("base")).ok_or_else(|| {
            cx.error_call(Some(call), &format!("\"{}\" not resolved from current namespace (base)", shown()))
        })
    } else {
        match crate::rdynload::find_call_symbol(cx, name, dll.as_deref()) {
            Some(f) => Ok(f),
            None => Err(match &dll {
                Some(p) => {
                    cx.error_call(Some(call), &format!("\"{}\" not available for .Call() for package \"{p}\"", shown()))
                }
                None => cx.error_call(Some(call), &format!("C symbol name \"{}\" not in load table", shown())),
            }),
        }
    };
    let f = f?;
    let nargs = keep.as_ref().map_or(args.len() - 1, |k| k.len());
    if nargs > MAX_ARGS {
        return Err(cx.error_call(Some(call), "too many arguments in foreign function call"));
    }
    let nil = cx.nil().sexp();
    let mut a = [nil.as_ptr().cast::<c_void>(); MAX_ARGS];
    for k in 0..nargs {
        let i = keep.as_ref().map_or(k + 1, |v| v[k]);
        a[k] = args.get(cx, i).sexp().as_ptr().cast();
    }
    // R: eval.c (the PP_FOREIGN builtin's context, `vmax`)
    let vmax = cx.vmax();
    let r = cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        // SAFETY: `f` is a routine of a loaded object, called as `.Call`
        // calls one (SEXP f(SEXP, ...)); every argument is in `args`,
        // rooted by the caller. The result word is rooted right away.
        let w = unsafe { cx.call_native65(call, base, f.cast(), &mut a, nargs) }?;
        // SAFETY: the routine's result, a SEXP (C's contract).
        Ok(unsafe { Sexp::from_ptr(w.cast()) })
    });
    let w = r?;
    // R: dotcode.c:check_retval
    if w.is_null() {
        out.set(cx, cx.nil());
        cx.warning_call(Some(call), "converting NULL pointer to R NULL")?;
    } else {
        // SAFETY: the routine's result: a live object of this isolate (C's
        // contract), held by nothing else now; nothing collected since it
        // returned.
        out.set(cx, unsafe { Ref::from_sexp(w, cx) });
    }
    // SAFETY: `vmax` was the R_alloc stack's top when the call began, a
    // frame that is still live.
    unsafe { cx.set_vmax(vmax) };
    Ok(())
}
