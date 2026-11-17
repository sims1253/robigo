//! dotcode.c's `.Call`, `.External` and `.External2` (docs/g3.md §14.2 M1):
//! the routine `resolveNativeRoutine` finds (a native symbol reference, or a
//! name in the table of loaded objects: rdynload.rs's `R_FindSymbol`,
//! registered routines first, then `dlsym`), called through the heap's
//! 65-argument native entry (robigo-rtcore's `call_native65`) from a
//! `CTXT_BUILTIN` record, as GNU R's `eval` gives `PP_FOREIGN` builtins one.

use crate::errors::not_yet;
use crate::rdynload::{Found, SymType};
use robigo_rt::prims::{Inline, Kind, PrimDef};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, Sexp, View};
use std::ffi::c_void;

// dotcode.c's `MAX_ARGS`: robigo-rtcore's, the native entry's own bound
use robigo_rtcore::NATIVE_MAX_ARGS as MAX_ARGS;

/// dotcode.c's `MaxSymbolBytes`: an entry point's name, with its nul.
const MAX_SYMBOL_BYTES: usize = 1024;

/// `.Call`, `.External` and `.External2` bound in `cx`'s isolate.
pub(crate) fn install(cx: &mut Cx) {
    robigo_rt::define_builtins(
        cx,
        &[
            // names.c: {".Call", do_dotcall, 0, 1, -1, {PP_FOREIGN, PREC_FN, 0}}
            PrimDef { name: ".Call", kind: Kind::Builtin(do_dotcall, Inline::None), eval: 1, arity: -1 },
            // names.c: {".External", do_External, 0, 1, -1, {PP_FOREIGN, PREC_FN, 0}}
            PrimDef { name: ".External", kind: Kind::Builtin(do_external, Inline::None), eval: 1, arity: -1 },
            // names.c: {".External2", do_External, 1, 201, -1, {PP_FOREIGN, PREC_FN, 0}}
            PrimDef { name: ".External2", kind: Kind::Builtin(do_external2, Inline::None), eval: 201, arity: -1 },
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

/// What `resolveNativeRoutine` resolved: the routine, what its lookup found
/// (its registration, for the argument count), its name as `buf` holds it,
/// and the arguments after `.NAME` the routine gets (`None`: all of them,
/// the usual case).
struct Resolved {
    fun: *const c_void,
    symbol: Found,
    name: Vec<u8>,
    keep: Option<Vec<usize>>,
}

/// What `.NAME` is (`checkValidSymbolId`).
enum SymId {
    /// A name: its first string in the native encoding (`translateChar`),
    /// or the translation's error, which `resolveNativeRoutine` raises
    /// where GNU R translates it (after `PACKAGE` and the namespace).
    Name(Result<Vec<u8>, String>),
    /// A native symbol reference: its routine and, for a registered one,
    /// the routine's name.
    Ref(*const c_void, Option<Vec<u8>>),
}

/// `checkValidSymbolId(op, call, &fun, &symbol, buf)`: for a string, the
/// name; for a native symbol reference (an external pointer, or a
/// "NativeSymbolInfo" holding one) its routine, the registration it carries
/// and the routine's name.
// R: dotcode.c:checkValidSymbolId
fn check_valid_symbol_id(cx: &mut Cx, call: Root<'_>, op: Root<'_>, symbol: &mut Found) -> RResult<SymId> {
    enum Kind {
        Str(Result<Vec<u8>, String>),
        Ptr(Option<Vec<u8>>, *mut c_void),
        Info,
        Other,
    }
    let kind = {
        let x = op.bind(cx);
        if let Some(c) = x.as_vector::<Char<'_>>().filter(|v| !v.is_empty()).map(|v| v.get(0)) {
            // isValidString: the name, translated now and raised later
            Kind::Str(
                robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                    .map(|b| b.into_owned())
                    .map_err(|e| e.to_string()),
            )
        } else if let View::ExtPtr(e) = x.view() {
            Kind::Ptr(e.tag().as_sym().map(|t| t.name().bytes().to_vec()), e.addr())
        } else if x.as_vector::<Ref<'_>>().is_some_and(|v| v.len() >= 2 && v.get(1).sexptype() == EXTPTRSXP) {
            // isNativeSymbolInfo: a list of at least two whose second is a
            // pointer
            Kind::Info
        } else {
            Kind::Other
        }
    };
    match kind {
        Kind::Str(name) => Ok(SymId::Name(name)),
        Kind::Ptr(tag, addr) => {
            let mut fun = std::ptr::null::<c_void>();
            let mut name = None;
            match tag.as_deref() {
                Some(b"native symbol") => fun = addr.cast_const(),
                Some(b"registered native symbol") => {
                    let tmp = addr.cast_const().cast::<Found>();
                    if !tmp.is_null() {
                        // SAFETY: a "registered native symbol" reference's
                        // address is the copy rdynload.rs made of a lookup
                        // (cleared before its object is unloaded).
                        let tmp = unsafe { *tmp };
                        if symbol.ty != SymType::Any && symbol.ty != tmp.ty {
                            return Err(cx.error_call(Some(call), "NULL value passed as symbol address"));
                        }
                        if !tmp.routine.is_null() {
                            // SAFETY: as above: a routine of a loaded record.
                            let r = unsafe { &*tmp.routine };
                            fun = r.fun;
                            name = Some(r.name.clone());
                        }
                        *symbol = tmp;
                    }
                }
                _ => {}
            }
            if fun.is_null() {
                return Err(cx.error_call(Some(call), "NULL value passed as symbol address"));
            }
            if let Some(n) = &name
                && n.len() >= MAX_SYMBOL_BYTES
            {
                return Err(cx.error(&format!("symbol '{}' is too long", String::from_utf8_lossy(n))));
            }
            Ok(SymId::Ref(fun, name))
        }
        Kind::Info => cx.scope(|cx, s| {
            let a = op.bind(cx).as_vector::<Ref<'_>>().unwrap().get(1);
            let a = s.root(cx, a);
            check_valid_symbol_id(cx, call, a, symbol)
        }),
        Kind::Other => {
            Err(cx.error_call(Some(call), "first argument must be a string (of length 1) or native symbol reference"))
        }
    }
}

/// `R_FindNativeSymbolFromDLL(name, &dll, symbol, env2)` for a call from the
/// namespace `env2`: its first DLL (base's `getCallingDLLe(env2)`), and the
/// routine `name` in it.
// R: dotcode.c:R_FindNativeSymbolFromDLL
fn find_native_symbol_from_dll(
    cx: &mut Cx,
    name: &[u8],
    ty: SymType,
    env2: Root<'_>,
) -> RResult<Option<(*const c_void, Found)>> {
    let obj = cx.scope(|cx, s| -> RResult<Option<*mut c_void>> {
        // eval(lang2(install("getCallingDLLe"), env), R_BaseEnv)
        let e = s.out(cx);
        let er = e.root();
        cx.build(e, |cx| {
            let f = cx.install(b"getCallingDLLe")?;
            cx.lang2(f.as_ref(), env2.bind(cx))
        })?;
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let obj = o.root();
        robigo_rt::eval(cx, er, base, o)?;
        let obj = obj.bind(cx);
        if !robigo_rt::vm::inherits(cx, obj, b"DLLInfo") {
            return Ok(None);
        }
        // VECTOR_ELT(obj, 4): the DLLInfoReference
        let r = obj.as_vector::<Ref<'_>>().filter(|v| v.len() > 4).map(|v| v.get(4));
        Ok(Some(match r.map(|r| r.view()) {
            Some(View::ExtPtr(p)) => p.addr(),
            _ => std::ptr::null_mut(),
        }))
    })?;
    match obj {
        None => Ok(None),
        Some(addr) => crate::rdynload::find_symbol_in_dll(cx, addr, name, ty),
    }
}

/// `resolveNativeRoutine(args, &fun, &symbol, buf, ...)` for `.Call` and
/// `.External` (`symbol.type` is `ty`): the routine `.NAME` names, as a
/// reference or by name (in `PACKAGE`, else the calling namespace's DLL,
/// else any loaded object), and the arguments without `PACKAGE`.
// R: dotcode.c:resolveNativeRoutine
fn resolve_native_routine(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    ty: SymType,
) -> RResult<Resolved> {
    let mut symbol = Found::of(ty);
    let id = check_valid_symbol_id(cx, call, args.root(0), &mut symbol)?;
    let (dll, keep) = pkgtrim(cx, args)?;
    let translated = match id {
        SymId::Ref(fun, name) => return Ok(Resolved { fun, symbol, name: name.unwrap_or_default(), keep }),
        SymId::Name(t) => t,
    };
    if dll.as_deref() == Some("") {
        return Err(cx.error_call(Some(call), "PACKAGE = \"\" is invalid"));
    }
    // called from a namespace? (ENCLOS(env))
    let (ns, env2) = cx.scope(|cx, s| -> RResult<(Option<Vec<u8>>, Option<Sexp>)> {
        let e = env.bind(cx);
        if e.sexptype() != ENVSXP {
            return Ok((None, None));
        }
        // SAFETY: a builtin's environment: a live ENVSXP, whose ENCLOS is a
        // fixed field.
        let enclos = unsafe { robigo_heap::env::enclos(e.sexp()) };
        // SAFETY: the enclosure of a live environment.
        let env2 = s.root(cx, unsafe { Ref::from_sexp(enclos, cx) });
        if env2.bind(cx).sexptype() != ENVSXP {
            return Ok((None, None));
        }
        let ns = robigo_rt::print::namespace_spec(cx, env2)?;
        Ok((ns.clone(), ns.map(|_| env2.bind(cx).sexp())))
    })?;
    // the name, translated (MaxSymbolBytes)
    let name = {
        let b = match translated {
            Ok(b) => b,
            Err(m) => return Err(cx.error(&m)),
        };
        if b.len() >= MAX_SYMBOL_BYTES {
            return Err(cx.error(&format!("symbol '{}' is too long", String::from_utf8_lossy(&b))));
        }
        if ty == SymType::Fortran { b.to_ascii_lowercase() } else { b }
    };
    let shown = String::from_utf8_lossy(&name).into_owned();
    if dll.is_none()
        && let Some(ns) = ns
    {
        // no PACKAGE: the DLL of the namespace the call comes from
        let env2 = env2.unwrap();
        let found = cx.scope(|cx, s| {
            // SAFETY: the enclosure found above, live (the call's frame
            // holds it) and nothing has collected since.
            let env2 = s.root(cx, unsafe { Ref::from_sexp(env2, cx) });
            find_native_symbol_from_dll(cx, &name, ty, env2)
        })?;
        return match found {
            Some((fun, symbol)) => Ok(Resolved { fun, symbol, name, keep }),
            None => Err(cx.error_call(
                Some(call),
                &format!("\"{shown}\" not resolved from current namespace ({})", String::from_utf8_lossy(&ns)),
            )),
        };
    }
    if let Some((fun, symbol)) = crate::rdynload::find_symbol(cx, &name, dll.as_deref(), ty) {
        return Ok(Resolved { fun, symbol, name, keep });
    }
    Err(match &dll {
        Some(p) => {
            let iface = match ty {
                SymType::C => ".C",
                SymType::Fortran => ".Fortran",
                SymType::Call => ".Call",
                SymType::External => ".External",
                SymType::Any => {
                    return Err(cx.error_call(
                        Some(call),
                        &format!("C/Fortran symbol name \"{shown}\" not in DLL for package \"{p}\""),
                    ));
                }
            };
            cx.error_call(Some(call), &format!("\"{shown}\" not available for {iface}() for package \"{p}\""))
        }
        None => cx.error_call(
            Some(call),
            &format!(
                "{} symbol name \"{shown}\" not in load table",
                if ty == SymType::Fortran { "Fortran" } else { "C" }
            ),
        ),
    })
}

/// The routine's declared argument count against the `n` it gets.
// R: dotcode.c:do_dotcall, do_External ("Incorrect number of arguments")
fn check_num_args(cx: &mut Cx, call: Root<'_>, r: &Resolved, n: usize) -> RResult<()> {
    if r.symbol.routine.is_null() {
        return Ok(());
    }
    // SAFETY: a routine of a loaded record (what the lookup found).
    let want = unsafe { &*r.symbol.routine }.num_args;
    if want > -1 && want as usize != n {
        return Err(cx.error_call(
            Some(call),
            &format!(
                "Incorrect number of arguments ({n}), expecting {want} for '{}'",
                String::from_utf8_lossy(&r.name)
            ),
        ));
    }
    Ok(())
}

/// `check_retval(call, val)`: a NULL result is R NULL, with a warning.
// R: dotcode.c:check_retval
fn set_result(cx: &mut Cx, call: Root<'_>, out: Out<'_>, w: Sexp) -> RResult<()> {
    if w.is_null() {
        out.set(cx, cx.nil());
        cx.warning_call(Some(call), "converting NULL pointer to R NULL")?;
    } else {
        // SAFETY: the routine's result: a live object of this isolate (C's
        // contract), held by nothing else now; nothing collected since it
        // returned.
        out.set(cx, unsafe { Ref::from_sexp(w, cx) });
    }
    Ok(())
}

/// Call `f(a[..n])` as `.Call` and `.External` call a routine: from a
/// `CTXT_BUILTIN` record for `call`, `R_alloc`'s stack restored after.
// R: eval.c (the PP_FOREIGN builtin's context, `vmax`), dotcode.c's
// vmaxset
fn call_routine(
    cx: &mut Cx,
    call: Root<'_>,
    f: *const c_void,
    a: &mut [*mut c_void; MAX_ARGS],
    n: usize,
) -> RResult<Sexp> {
    let vmax = cx.vmax();
    let r = cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        // SAFETY: `f` is a routine of a loaded object (or a reference's
        // address), called as its interface calls one (SEXP f(SEXP, ...));
        // every argument is rooted by the caller. The result word is
        // rooted right away.
        let w = unsafe { cx.call_native65(call, base, f.cast(), a, n) }?;
        // SAFETY: the routine's result, a SEXP (C's contract).
        Ok(unsafe { Sexp::from_ptr(w.cast()) })
    });
    // SAFETY: `vmax` was the R_alloc stack's top when the call began, a
    // frame that is still live.
    unsafe { cx.set_vmax(vmax) };
    r
}

/// `.Call(.NAME, ...)`.
// R: dotcode.c:do_dotcall, R_doDotCall; eval.c's PP_FOREIGN builtin call
// (its check_stack_balance is the evaluator's, where GNU R's `eval` makes
// it)
fn do_dotcall(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.is_empty() {
        return Err(cx.error_call(Some(call), "'.NAME' is missing"));
    }
    if args.name(cx, 0).is_some() {
        return Err(cx.error_call(Some(call), "the first argument should not be named"));
    }
    let r = resolve_native_routine(cx, call, args, env, SymType::Call)?;
    let nargs = r.keep.as_ref().map_or(args.len() - 1, |k| k.len());
    if nargs > MAX_ARGS {
        return Err(cx.error_call(Some(call), "too many arguments in foreign function call"));
    }
    check_num_args(cx, call, &r, nargs)?;
    let nil = cx.nil().sexp();
    let mut a = [nil.as_ptr().cast::<c_void>(); MAX_ARGS];
    for k in 0..nargs {
        let i = r.keep.as_ref().map_or(k + 1, |v| v[k]);
        a[k] = args.get(cx, i).sexp().as_ptr().cast();
    }
    let w = call_routine(cx, call, r.fun, &mut a, nargs)?;
    set_result(cx, call, out, w)
}

/// `.External(.NAME, ...)`: the routine gets the arguments as a pairlist,
/// `.NAME` first.
// R: dotcode.c:do_External (PRIMVAL 0)
fn do_external(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    external(cx, call, args, env, out, false)
}

/// `.External2(.NAME, ...)`: the routine gets `(call, op, args, env)`.
// R: dotcode.c:do_External (PRIMVAL 1)
fn do_external2(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    external(cx, call, args, env, out, true)
}

// R: dotcode.c:do_External
fn external(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>, two: bool) -> RResult<()> {
    if args.is_empty() {
        return Err(cx.error_call(Some(call), "'.NAME' is missing"));
    }
    if args.name(cx, 0).is_some() {
        return Err(cx.error_call(Some(call), "the first argument should not be named"));
    }
    let r = resolve_native_routine(cx, call, args, env, SymType::External)?;
    let keep: Vec<usize> = match &r.keep {
        Some(k) => std::iter::once(0).chain(k.iter().copied()).collect(),
        None => (0..args.len()).collect(),
    };
    check_num_args(cx, call, &r, keep.len() - 1)?;
    cx.scope(|cx, s| {
        // the evaluated arguments as GNU R's builtin gets them: a pairlist,
        // tagged by the names given, `.NAME` first (PACKAGE dropped)
        let o = s.out(cx);
        let list = o.root();
        cx.build(o, |cx| {
            let mut l = cx.nil();
            for &i in keep.iter().rev() {
                let v = args.get(cx, i);
                l = match args.name(cx, i) {
                    Some(t) => cx.cons_tagged(v, l, t)?,
                    None => cx.cons(v, l)?,
                };
            }
            Ok(l)
        })?;
        let nil = cx.nil().sexp();
        let mut a = [nil.as_ptr().cast::<c_void>(); MAX_ARGS];
        let n = if two {
            let op = match robigo_rt::prims::primitive(cx, ".External2") {
                Some(op) => s.root(cx, op),
                None => return Err(not_yet(cx, "'.External2' without its builtin object")),
            };
            a[0] = call.bind(cx).sexp().as_ptr().cast();
            a[1] = op.bind(cx).sexp().as_ptr().cast();
            a[2] = list.bind(cx).sexp().as_ptr().cast();
            a[3] = env.bind(cx).sexp().as_ptr().cast();
            4
        } else {
            a[0] = list.bind(cx).sexp().as_ptr().cast();
            1
        };
        let w = call_routine(cx, call, r.fun, &mut a, n)?;
        set_result(cx, call, out, w)
    })
}
