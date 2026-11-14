//! dotcode.c's `.Call` and Rdynload.c's `dyn.load` (docs/g3.md §14.2 M1):
//! one isolate's table of loaded shared objects (`LoadedDLL`, base first,
//! as `InitDynload` makes it), `dlopen` with GNU R's flags, routines found
//! by name with `dlsym` (dynamic lookup: robigo has no routine
//! registration yet), and the routine called through the heap's 65-argument
//! native entry (robigo-rtcore's `call_native65`) from a `CTXT_BUILTIN`
//! record, as GNU R's `eval` gives `PP_FOREIGN` builtins one.

use crate::errors::not_yet;
use robigo_rt::prims::{Inline, Kind, PrimDef};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, Scope, Sexp};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_void};

/// dotcode.c's `MAX_ARGS`.
const MAX_ARGS: usize = 65;
/// Rdynload.c's `MaxNumDLLs` default (`R_MAX_NUM_DLLS` unset, a large
/// enough file descriptor limit).
const MAX_NUM_DLLS: usize = 614;

/// One loaded shared object: GNU R's `DllInfo`'s fields robigo reads.
#[derive(Clone, Debug)]
pub struct DllInfo {
    /// The object's name: its file name without `.so`.
    pub name: String,
    /// The full path it was loaded from.
    pub path: String,
    /// `dlopen`'s handle (null for base).
    pub handle: *mut c_void,
    /// `useDynamicLookup`: routines are found by `dlsym` too.
    pub dynamic_lookup: bool,
    /// `forceSymbols`: only registered routines may be called by name.
    pub force_symbols: bool,
}

/// An isolate's loaded objects (`LoadedDLL`, `CountDLL`), in load order,
/// and the routines found in them by name.
#[derive(Default)]
struct Dlls {
    list: RefCell<Vec<DllInfo>>,
    /// `R_FindSymbol`'s results by (name, `PACKAGE`): what the search over
    /// `list` returns, so cleared whenever `list` changes. (GNU R caches
    /// only on Windows, `CACHE_DLL_SYM`; elsewhere every `.Call` searches
    /// with `dlsym`. Cached or not, the routine found is the same.)
    /// (by name: each `PACKAGE` it was asked with, and the routine)
    found: RefCell<std::collections::HashMap<Vec<u8>, Vec<(Option<String>, usize)>, robigo_heap::BuildFx>>,
}

/// The isolate's table of loaded objects: (name, path) in load order.
pub fn loaded_dlls(cx: &Cx) -> Vec<(String, String)> {
    cx.ext::<Dlls>()
        .map_or_else(Vec::new, |d| d.list.borrow().iter().map(|i| (i.name.clone(), i.path.clone())).collect())
}

/// `.Call` and `dyn.load` for `cx`'s isolate, and its table with base in it
/// (`InitDynload`).
pub(crate) fn install(cx: &mut Cx) {
    let base = DllInfo {
        name: "base".into(),
        path: "base".into(),
        handle: std::ptr::null_mut(),
        dynamic_lookup: false,
        force_symbols: false,
    };
    cx.set_ext(Dlls { list: RefCell::new(vec![base]), found: Default::default() });
    robigo_rt::define_builtins(
        cx,
        &[
            // names.c: {".Call", do_dotcall, 0, 1, -1, {PP_FOREIGN, PREC_FN, 0}}
            PrimDef { name: ".Call", kind: Kind::Builtin(do_dotcall, Inline::None), eval: 1, arity: -1 },
            // names.c: {"dyn.load", do_dynload, 0, 111, 4, ...}
            PrimDef { name: "dyn.load", kind: Kind::Builtin(do_dynload, Inline::None), eval: 111, arity: 4 },
        ],
    );
}

/// The first element of a character vector of length one, as bytes.
fn string1(cx: &Cx, x: Ref<'_>) -> Option<Vec<u8>> {
    let v = x.as_vector::<Char<'_>>()?;
    (!v.is_empty() && v.get(0) != cx.na_string()).then(|| v.get(0).bytes().to_vec())
}

/// GNU R's `GetFullDLLPath` (unix): a relative path is the working
/// directory's; `~` is expanded.
// R: unix/dynload.c:getFullDLLPath
fn full_dll_path(cx: &mut Cx, call: Root<'_>, path: &str) -> RResult<String> {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        return Ok(format!("{home}{rest}"));
    }
    if path.starts_with('/') {
        return Ok(path.to_owned());
    }
    match std::env::current_dir() {
        Ok(d) => Ok(format!("{}/{path}", d.display())),
        Err(_) => Err(cx.error_call(Some(call), "cannot get working directory!")),
    }
}

/// `.Internal(dyn.load(x, local, now, DLLpath))`.
// R: Rdynload.c:do_dynload, AddDLL, R_RegisterDLL
fn do_dynload(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Some(x) = string1(cx, args.get(cx, 0)) else {
        return Err(cx.error("character argument expected"));
    };
    let local = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 1));
    let now = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    let path = full_dll_path(cx, call, &String::from_utf8_lossy(&x))?;
    let loaded = {
        let d = cx.ext::<Dlls>().expect("robigo-rtapi: install() was not called");
        d.found.borrow_mut().clear();
        let mut list = d.list.borrow_mut();
        // a path loaded already moves to the end of the list
        if let Some(k) = list.iter().position(|i| i.path == path) {
            let info = list.remove(k);
            list.push(info);
            Ok(list.len() - 1)
        } else if list.len() == MAX_NUM_DLLS {
            Err("`maximal number of DLLs reached...".to_owned())
        } else {
            // R: unix/dynload.c:computeDLOpenFlag
            let mut flags = if local != 0 { libc::RTLD_LOCAL } else { libc::RTLD_GLOBAL };
            flags |= if now != 0 { libc::RTLD_NOW } else { libc::RTLD_LAZY };
            let cpath = CString::new(path.clone()).map_err(|_| "embedded nul in the path".to_owned());
            match cpath {
                Err(m) => Err(m),
                Ok(cpath) => {
                    // SAFETY: a NUL-terminated path; the library's
                    // initializers run as GNU R's dlopen runs them.
                    let h = unsafe { libc::dlopen(cpath.as_ptr(), flags) };
                    if h.is_null() {
                        // SAFETY: dlerror returns the thread's message.
                        let e = unsafe { libc::dlerror() };
                        Err(if e.is_null() {
                            String::new()
                        } else {
                            // SAFETY: as above, a C string.
                            unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
                        })
                    } else {
                        // R_RegisterDLL: the basename without SHLIB_EXT
                        let base = path.rsplit('/').next().unwrap_or(&path);
                        let name = base.strip_suffix(".so").filter(|n| !n.is_empty()).unwrap_or(base).to_owned();
                        list.push(DllInfo {
                            name,
                            path: path.clone(),
                            handle: h,
                            dynamic_lookup: true,
                            force_symbols: false,
                        });
                        Ok(usize::MAX)
                    }
                }
            }
        }
    };
    let k = match loaded {
        Ok(k) => k,
        Err(m) => return Err(cx.error(&format!("unable to load shared object '{path}':\n  {m}"))),
    };
    if k == usize::MAX {
        // a new object: its R_init_<name>, if it has one
        let (name, h) = {
            let d = cx.ext::<Dlls>().unwrap();
            let l = d.list.borrow();
            let i = l.last().unwrap();
            (i.name.clone(), i.handle)
        };
        let init = |n: &str| -> *mut c_void {
            let s = CString::new(format!("R_init_{n}")).unwrap();
            // SAFETY: a live handle and a symbol name.
            unsafe { libc::dlsym(h, s.as_ptr()) }
        };
        if !init(&name).is_null() || !init(&name.replace('.', "_")).is_null() {
            return Err(not_yet(cx, &format!("R_init_{name} (routine registration)")));
        }
    }
    make_dll_info(cx, out)
}

/// `Rf_MakeDLLInfo(info)` of the table's last object: a list `name`,
/// `path`, `dynamicLookup`, `handle` (class "DLLHandle"), `info` (class
/// "DLLInfoReference") and `forceSymbols`, of class "DLLInfo".
// R: Rdynload.c:Rf_MakeDLLInfo, Rf_makeDllObject, Rf_makeDllInfoReference
fn make_dll_info(cx: &mut Cx, out: Out<'_>) -> RResult<()> {
    let info = cx.ext::<Dlls>().unwrap().list.borrow().last().cloned().unwrap();
    cx.scope(|cx, s| {
        let handle = extptr_of_class(cx, s, info.handle, b"DLLHandle", b"DLLHandle")?;
        let reference = extptr_of_class(cx, s, std::ptr::null_mut(), b"DLLInfo", b"DLLInfoReference")?;
        let names: [&[u8]; 6] = [b"name", b"path", b"dynamicLookup", b"handle", b"info", b"forceSymbols"];
        cx.build(out, |cx| {
            let str1 = |b: &[u8]| -> Result<Ref<'_>, robigo_rtcore::NeedGc> {
                let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                v.set(0, cx.mk_char(b, robigo_rtcore::Encoding::Native)?);
                Ok(v.finish().into())
            };
            let mut l = cx.alloc_vector::<Ref<'_>>(6)?;
            l.set(0, str1(info.name.as_bytes())?);
            l.set(1, str1(info.path.as_bytes())?);
            l.set(2, cx.logical_value(info.dynamic_lookup as i32));
            l.set(3, handle.bind(cx));
            l.set(4, reference.bind(cx));
            l.set(5, cx.logical_value(info.force_symbols as i32));
            let mut nm = cx.alloc_vector::<Char<'_>>(6)?;
            for (i, n) in names.iter().enumerate() {
                nm.set(i, cx.mk_char(n, robigo_rtcore::Encoding::Native)?);
            }
            let class = str1(b"DLLInfo")?;
            let attrs = cx.cons_tagged(class, cx.nil(), cx.install(b"class")?)?;
            let attrs = cx.cons_tagged(nm.finish().into(), attrs, cx.install(b"names")?)?;
            l.set_attrib(attrs);
            l.set_object(true);
            Ok(l.finish().into())
        })
    })
}

/// `R_MakeExternalPtr(p, install(tag), R_NilValue)` with class `class`.
fn extptr_of_class<'s>(cx: &mut Cx, s: Scope<'s>, p: *mut c_void, tag: &[u8], class: &[u8]) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let nil = cx.nil().sexp();
        let t = cx.install(tag)?.as_ref().sexp();
        let mut c = cx.alloc_vector::<Char<'_>>(1)?;
        c.set(0, cx.mk_char(class, robigo_rtcore::Encoding::Native)?);
        let attrs = cx.cons_tagged(c.finish().into(), cx.nil(), cx.install(b"class")?)?;
        // SAFETY: live objects of this heap; an EXTPTRSXP's car is its
        // address (untraced); the fresh object takes its attributes without
        // a barrier.
        unsafe {
            let x = cx.alloc_node_raw(EXTPTRSXP, nil, nil, t)?;
            x.set_extptr_addr(p.cast());
            x.set_attrib(attrs.sexp());
            x.set_object(true);
            Ok(Ref::from_sexp(x, cx))
        }
    })?;
    Ok(r)
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

/// `R_FindSymbol(name, pkg, symbol)` by dynamic lookup: the newest loaded
/// object that has `name` (only `pkg` when it is named).
// R: Rdynload.c:R_FindSymbol, R_dlsym
fn find_symbol(cx: &Cx, name: &[u8], pkg: Option<&str>) -> Option<*const c_void> {
    let d = cx.ext::<Dlls>()?;
    // (a hit allocates nothing)
    if let Some(v) = d.found.borrow().get(name)
        && let Some(&(_, f)) = v.iter().find(|(p, _)| p.as_deref() == pkg)
    {
        return Some(f as *const c_void);
    }
    let list = d.list.borrow();
    let c = CString::new(name).ok()?;
    for info in list.iter().rev() {
        if pkg.is_some_and(|p| p != info.name) || info.force_symbols || !info.dynamic_lookup || info.handle.is_null() {
            continue;
        }
        // SAFETY: a live handle and a symbol name.
        let f = unsafe { libc::dlsym(info.handle, c.as_ptr()) };
        if !f.is_null() {
            d.found.borrow_mut().entry(name.to_vec()).or_default().push((pkg.map(str::to_owned), f as usize));
            return Some(f.cast_const());
        }
    }
    None
}

/// `.Call(.NAME, ...)`.
// R: dotcode.c:do_dotcall, resolveNativeRoutine, R_doDotCall; eval.c's
// PP_FOREIGN builtin call (check_stack_balance, vmaxset)
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
        find_symbol(cx, name, Some("base")).ok_or_else(|| {
            cx.error_call(Some(call), &format!("\"{}\" not resolved from current namespace (base)", shown()))
        })
    } else {
        match find_symbol(cx, name, dll.as_deref()) {
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
    let mut a = [nil.0.cast::<c_void>(); MAX_ARGS];
    for k in 0..nargs {
        let i = keep.as_ref().map_or(k + 1, |v| v[k]);
        a[k] = args.get(cx, i).sexp().0.cast();
    }
    // R: eval.c (the PP_FOREIGN builtin's context, `save`, `vmax`)
    let save = cx.pp_top();
    let vmax = cx.vmax();
    let r = cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        // SAFETY: `f` is a routine of a loaded object, called as `.Call`
        // calls one (SEXP f(SEXP, ...)); every argument is in `args`,
        // rooted by the caller. The result word is rooted right away.
        let w = unsafe { cx.call_native65(call, base, f.cast(), &mut a) }?;
        Ok(Sexp(w.cast()))
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
    // R: eval.c:check_stack_balance
    if cx.pp_top() != save {
        let msg = format!("Warning: stack imbalance in '.Call', {save} then {}\n", cx.pp_top());
        cx.reprintf(msg.as_bytes());
    }
    // SAFETY: `vmax` was the R_alloc stack's top when the call began, a
    // frame that is still live.
    unsafe { cx.set_vmax(vmax) };
    Ok(())
}
