//! Rdynload.c: one isolate's table of loaded shared objects (`LoadedDLL`,
//! base first, as `InitDynload` makes it), the routines registered in them
//! (`R_init_base`'s, registration.c, and a package's `R_init_<pkg>` through
//! `R_registerRoutines`), `dyn.load` and `dyn.unload` with GNU R's `dlopen`
//! flags, symbol lookup (`R_FindSymbol`, `R_dlsym`: registered routines
//! first, then `dlsym` when the object allows dynamic lookup) and the R
//! objects that describe them: `getLoadedDLLs()`, `getRegisteredRoutines`
//! and `getSymbolInfo` (`getNativeSymbolInfo`).
//!
//! As GNU R does (`DLLInfoEptrs`), each object's `DLLInfoReference` is made
//! once and kept rooted while the object is loaded, and the weak list
//! `SymbolEptrs` holds the other external pointers handed out for it (its
//! handle's, its routines'): `dyn.unload` clears them all before it frees
//! the record, so a reference kept past the unload reads a null address.
//! A record is boxed: its address, which C holds as `DllInfo *` and the
//! references hold, does not move while it is loaded.

use crate::errors::not_yet;
use robigo_rt::prims::{Inline, Kind, PrimDef};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, NeedGc, Out, RResult, RSym, Ref, Root, Sexp, View};
use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};

/// Rdynload.c's `MaxNumDLLs` default (`R_MAX_NUM_DLLS` unset, a large
/// enough file descriptor limit).
const MAX_NUM_DLLS: usize = 614;

/// GNU R's `NativeSymbolType` (R_ext/Rdynload.h).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub enum SymType {
    Any = 0,
    C = 1,
    Call = 2,
    Fortran = 3,
    External = 4,
}

/// A registered routine (`Rf_DotCSymbol`, `Rf_DotCallSymbol`, ...): its
/// name, address and declared number of arguments (-1: any).
#[derive(Clone, Debug)]
pub struct Routine {
    pub name: Vec<u8>,
    pub fun: *const c_void,
    pub num_args: i32,
}

/// One interface's routines, as `R_registerRoutines` copied them.
pub type Routines = Box<[Routine]>;

/// The registered routines, by interface (`CSymbols`, `CallSymbols`,
/// `FortranSymbols`, `ExternalSymbols`).
#[derive(Default, Debug)]
struct Tables {
    c: Routines,
    call: Routines,
    fortran: Routines,
    external: Routines,
}

/// One loaded shared object: GNU R's `DllInfo`. C sees it as the opaque
/// `DllInfo *` (`R_ext/Rdynload.h`) that `R_init_<pkg>` gets: what C sets
/// through it (`R_registerRoutines`, `R_useDynamicSymbols`,
/// `R_forceSymbols`) is held in cells.
#[derive(Debug)]
pub struct DllInfo {
    /// The object's name: its file name without `.so`.
    pub name: String,
    /// The full path it was loaded from.
    pub path: String,
    /// `dlopen`'s handle (null for base).
    pub handle: *mut c_void,
    /// `useDynamicLookup`: routines are found by `dlsym` too.
    dynamic_lookup: Cell<bool>,
    /// `forceSymbols`: its routines are not found by name alone.
    force_symbols: Cell<bool>,
    tables: RefCell<Tables>,
    /// The tables a later `R_registerRoutines` replaced: GNU R leaks them,
    /// and a lookup's result may still point into one; they go with the
    /// record.
    retired: RefCell<Vec<Tables>>,
    /// Its `DLLInfoReference`, made once and rooted while it is loaded
    /// (GNU R's `DLLInfoEptrs`): every `DLLInfo` of it shares the object.
    eref: OnceCell<robigo_rtcore::Global>,
}

impl DllInfo {
    fn new(name: String, path: String, handle: *mut c_void, dynamic_lookup: bool, force_symbols: bool) -> DllInfo {
        DllInfo {
            name,
            path,
            handle,
            dynamic_lookup: Cell::new(dynamic_lookup),
            force_symbols: Cell::new(force_symbols),
            tables: Default::default(),
            retired: Default::default(),
            eref: Default::default(),
        }
    }

    /// `useDynamicLookup`.
    pub fn dynamic_lookup(&self) -> bool {
        self.dynamic_lookup.get()
    }

    /// `forceSymbols`.
    pub fn force_symbols(&self) -> bool {
        self.force_symbols.get()
    }

    /// The routines registered for interface `t` (none for `Any`). The
    /// elements stay where they are while the record is loaded: a lookup
    /// keeps their addresses.
    fn routines(&self, t: SymType) -> std::cell::Ref<'_, [Routine]> {
        std::cell::Ref::map(self.tables.borrow(), |tb| match t {
            SymType::C => &*tb.c,
            SymType::Call => &*tb.call,
            SymType::Fortran => &*tb.fortran,
            SymType::External => &*tb.external,
            SymType::Any => &[],
        })
    }
}

/// An isolate's loaded objects (`LoadedDLL`, `CountDLL`), in load order,
/// the weak list of the references handed out for them, and the `.Call`
/// routines found in them by name.
struct Dlls {
    /// Boxed: C and the `DLLInfoReference`s hold a record's address, which
    /// must not move when the list grows.
    #[allow(clippy::vec_box)]
    list: RefCell<Vec<Box<DllInfo>>>,
    /// `SymbolEptrs`: a cons whose cdr is the list of weak references, each
    /// keyed by an external pointer handed out (a handle's, a routine's)
    /// with the `DLLInfoReference` of its object as value.
    symbol_eptrs: robigo_rtcore::Global,
    /// `R_registerSymbolEptr`'s `cleancount`.
    clean_count: Cell<u32>,
    /// `R_FindSymbol`'s `.Call` results by (name, `PACKAGE`): what the
    /// search over `list` returns, so cleared whenever `list` changes. (GNU
    /// R caches only on Windows, `CACHE_DLL_SYM`; elsewhere every `.Call`
    /// searches. Cached or not, the routine found is the same.)
    #[allow(clippy::type_complexity)]
    found: RefCell<std::collections::HashMap<Vec<u8>, Vec<(Option<String>, usize, Found)>, robigo_heap::BuildFx>>,
}

/// `R_registerSymbolEptr`'s `MAXCOUNT`.
const MAX_COUNT: u32 = 10;

fn dlls(cx: &Cx) -> &Dlls {
    cx.ext::<Dlls>().expect("robigo-rtapi: install() was not called")
}

/// The isolate's table of loaded objects: (name, path) in load order.
pub fn loaded_dlls(cx: &Cx) -> Vec<(String, String)> {
    cx.ext::<Dlls>()
        .map_or_else(Vec::new, |d| d.list.borrow().iter().map(|i| (i.name.clone(), i.path.clone())).collect())
}

/// The table with base in it, its routines registered (`InitDynload`,
/// `R_init_base`), and Rdynload.c's builtins bound.
// R: Rdynload.c:InitDynload, registration.c:R_init_base
pub(crate) fn install(cx: &mut Cx) {
    let call = |name: &[u8], fun: *const c_void, num_args| Routine { name: name.to_vec(), fun, num_args };
    // R_registerRoutines sets useDynamicLookup from the handle; R_init_base's
    // R_useDynamicSymbols(dll, FALSE) and R_forceSymbols(dll, TRUE)
    let base = DllInfo::new("base".into(), "base".into(), std::ptr::null_mut(), false, true);
    *base.tables.borrow_mut() = Tables {
        c: Box::default(),
        // registration.c's callMethods
        call: Box::new([
            call(b"R_addTaskCallback", robigo_rtapi_base_R_addTaskCallback as *const c_void, 4),
            call(b"R_getTaskCallbackNames", robigo_rtapi_base_R_getTaskCallbackNames as *const c_void, 0),
            call(b"R_removeTaskCallback", robigo_rtapi_base_R_removeTaskCallback as *const c_void, 1),
        ]),
        // registration.c's fortranMethods (LINPACK)
        fortran: Box::new([
            call(b"dqrcf", base_fortran_dqrcf as *const c_void, 8),
            call(b"dqrdc2", base_fortran_dqrdc2 as *const c_void, 9),
            call(b"dqrqty", base_fortran_dqrqty as *const c_void, 7),
            call(b"dqrqy", base_fortran_dqrqy as *const c_void, 7),
            call(b"dqrrsd", base_fortran_dqrrsd as *const c_void, 7),
            call(b"dqrxb", base_fortran_dqrxb as *const c_void, 7),
            call(b"dtrco", base_fortran_dtrco as *const c_void, 6),
        ]),
        external: Box::default(),
    };
    // SymbolEptrs = CONS(R_NilValue, R_NilValue), preserved
    let symbol_eptrs = cx.scope(|cx, s| {
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| cx.cons(cx.nil(), cx.nil())).expect("robigo-rtapi: allocating the DLL table's symbol list");
        cx.global(r.bind(cx))
    });
    cx.set_ext(Dlls {
        list: RefCell::new(vec![Box::new(base)]),
        symbol_eptrs,
        clean_count: Cell::new(MAX_COUNT),
        found: Default::default(),
    });
    let mut h = cx.hooks();
    h.has_native_symbol = has_symbol;
    cx.install_hooks(h);
    robigo_rt::define_builtins(
        cx,
        &[
            // names.c: {"dyn.load", do_dynload, 0, 111, 4, ...}
            PrimDef { name: "dyn.load", kind: Kind::Builtin(do_dynload, Inline::None), eval: 111, arity: 4 },
            // names.c: {"dyn.unload", do_dynunload, 0, 111, 1, ...}
            PrimDef { name: "dyn.unload", kind: Kind::Builtin(do_dynunload, Inline::None), eval: 111, arity: 1 },
            // names.c: {"getRegisteredRoutines", do_getRegisteredRoutines, 0, 11, 1, ...}
            PrimDef {
                name: "getRegisteredRoutines",
                kind: Kind::Builtin(do_get_registered_routines, Inline::None),
                eval: 11,
                arity: 1,
            },
            // names.c: {"getLoadedDLLs", do_getDllTable, 0, 11, 0, ...}
            PrimDef { name: "getLoadedDLLs", kind: Kind::Builtin(do_get_dll_table, Inline::None), eval: 11, arity: 0 },
            // names.c: {"getSymbolInfo", do_getSymbolInfo, 0, 11, 3, ...}
            PrimDef {
                name: "getSymbolInfo",
                kind: Kind::Builtin(do_get_symbol_info, Inline::None),
                eval: 11,
                arity: 3,
            },
        ],
    );
}

// --- base's routines ----------------------------------------------------
//
// R_init_base registers main.c's task-callback entries for `.Call` and
// LINPACK's QR routines for `.Fortran`. robigo-rt has neither yet: the
// first raise "not implemented" when called, and `.Fortran` itself is not
// implemented, so nothing reaches the second. Their names are robigo's:
// libR.so exports the LINPACK routines (`dqrdc2_`), which robigo does not
// implement, and not the task-callback ones.

crate::entry! {
    /// main.c's `R_addTaskCallback(f, data, useData, name)`, as base
    /// registers it for `.Call`.
    // R: main.c:R_addTaskCallback
    [4] fn robigo_rtapi_base_R_addTaskCallback(f: Sexp, data: Sexp, use_data: Sexp, name: Sexp) -> Sexp => |cx, _s| {
        let _ = (f, data, use_data, name);
        Err(not_yet(cx, "R_addTaskCallback"))
    }

    /// main.c's `R_getTaskCallbackNames()`.
    // R: main.c:R_getTaskCallbackNames
    [0] fn robigo_rtapi_base_R_getTaskCallbackNames() -> Sexp => |cx, _s| {
        Err(not_yet(cx, "R_getTaskCallbackNames"))
    }

    /// main.c's `R_removeTaskCallback(which)`.
    // R: main.c:R_removeTaskCallback
    [1] fn robigo_rtapi_base_R_removeTaskCallback(which: Sexp) -> Sexp => |cx, _s| {
        let _ = which;
        Err(not_yet(cx, "R_removeTaskCallback"))
    }
}

/// A LINPACK routine base registers: unreachable while `.Fortran` raises
/// "not implemented" before it calls a routine.
macro_rules! linpack {
    ($($f:ident),*) => {$(
        extern "C" fn $f() {
            eprintln!(concat!("robigo: LINPACK's '", stringify!($f), "' is not implemented yet"));
            std::process::abort();
        }
    )*};
}
linpack!(
    base_fortran_dqrcf,
    base_fortran_dqrdc2,
    base_fortran_dqrqty,
    base_fortran_dqrqy,
    base_fortran_dqrrsd,
    base_fortran_dqrxb,
    base_fortran_dtrco
);

// --- symbol lookup ------------------------------------------------------

/// What a lookup found (GNU R's `R_RegisteredNativeSymbol`): the routine's
/// interface (`Any` when found by `dlsym`), the registered routine, and the
/// object it is in.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Found {
    pub ty: SymType,
    /// The registered routine (`symbol.c`, ...), or null.
    pub routine: *const Routine,
    /// The object (`dll`), or null.
    pub dll: *const DllInfo,
}

impl Found {
    pub(crate) fn of(ty: SymType) -> Found {
        Found { ty, routine: std::ptr::null(), dll: std::ptr::null() }
    }
}

/// `R_getDLLRegisteredSymbol(info, name, symbol)`: the routine `name`
/// registered in `info` for the interface `symbol.ty` (any interface for
/// `Any`, in GNU R's order .C, .Call, .Fortran, .External).
// R: Rdynload.c:R_getDLLRegisteredSymbol, Rf_lookupRegistered*Symbol
fn registered_symbol(info: &DllInfo, name: &[u8], symbol: &mut Found) -> *const c_void {
    for t in [SymType::C, SymType::Call, SymType::Fortran, SymType::External] {
        if symbol.ty != SymType::Any && symbol.ty != t {
            continue;
        }
        if let Some(r) = info.routines(t).iter().find(|r| r.name == name) {
            // (the element stays put while the record is loaded)
            *symbol = Found { ty: t, routine: r, dll: info };
            return r.fun;
        }
    }
    std::ptr::null()
}

/// `R_dlsym(info, name, symbol)`: a registered routine, else (when `info`
/// allows dynamic lookup) `dlsym`'s, with Fortran's trailing underscore
/// (HAVE_F77_UNDERSCORE; ELF symbols have no leading one).
// R: Rdynload.c:R_dlsym, unix/dynload.c:R_local_dlsym
fn dlsym(info: &DllInfo, name: &[u8], symbol: &mut Found) -> *const c_void {
    let f = registered_symbol(info, name, symbol);
    if !f.is_null() {
        return f;
    }
    if !info.dynamic_lookup() || info.handle.is_null() {
        return std::ptr::null();
    }
    let sym = |buf: &[u8]| -> *const c_void {
        let Ok(c) = CString::new(buf) else { return std::ptr::null() };
        // SAFETY: a live handle and a symbol name.
        unsafe { libc::dlsym(info.handle, c.as_ptr()) }.cast_const()
    };
    let mut buf = name.to_vec();
    if symbol.ty == SymType::Fortran {
        buf.push(b'_');
    }
    let mut f = sym(&buf);
    if f.is_null() && symbol.ty == SymType::Any {
        buf.push(b'_');
        f = sym(&buf);
    }
    if !f.is_null() {
        symbol.dll = info;
    }
    f
}

/// `R_FindSymbol(name, pkg, symbol)`: the newest loaded object (only the one
/// named `pkg` when it is not empty) that has `name`, skipping objects that
/// force symbols.
// R: Rdynload.c:R_FindSymbol
fn find_symbol_in(list: &[Box<DllInfo>], name: &[u8], pkg: &str, symbol: &mut Found) -> *const c_void {
    let all = pkg.is_empty();
    for info in list.iter().rev() {
        let named = !all && pkg == info.name;
        if (all || named) && !info.force_symbols() {
            let f = dlsym(info, name, symbol);
            if !f.is_null() {
                symbol.dll = &**info;
                return f;
            }
        }
        if named {
            return std::ptr::null();
        }
    }
    std::ptr::null()
}

/// `R_FindSymbol(name, "", NULL) != NULL`: the isolate's `has_native_symbol`
/// hook (RNG.c's `user_unif_rand`, `user_norm_rand`).
// R: Rdynload.c:R_FindSymbol
fn has_symbol(cx: &Cx, name: &[u8]) -> bool {
    let Some(d) = cx.ext::<Dlls>() else { return false };
    let mut symbol = Found::of(SymType::Any);
    !find_symbol_in(&d.list.borrow(), name, "", &mut symbol).is_null()
}

/// `R_FindSymbol(name, pkg, &symbol)` for `resolveNativeRoutine`: the
/// routine `name` for interface `ty` in the loaded objects (`pkg`:
/// `PACKAGE`, if given), and what the lookup found. `.Call`'s results are
/// cached (`Dlls::found`).
// R: Rdynload.c:R_FindSymbol, dotcode.c:resolveNativeRoutine
pub(crate) fn find_symbol(cx: &Cx, name: &[u8], pkg: Option<&str>, ty: SymType) -> Option<(*const c_void, Found)> {
    let d = cx.ext::<Dlls>()?;
    let cached = ty == SymType::Call;
    // (a hit allocates nothing)
    if cached
        && let Some(v) = d.found.borrow().get(name)
        && let Some(&(_, f, found)) = v.iter().find(|(p, _, _)| p.as_deref() == pkg)
    {
        return Some((f as *const c_void, found));
    }
    let mut symbol = Found::of(ty);
    let f = find_symbol_in(&d.list.borrow(), name, pkg.unwrap_or(""), &mut symbol);
    if f.is_null() {
        return None;
    }
    if cached {
        d.found.borrow_mut().entry(name.to_vec()).or_default().push((pkg.map(str::to_owned), f as usize, symbol));
    }
    Some((f, symbol))
}

/// `R_FindNativeSymbolFromDLL`'s lookup in the object a "DLLInfo" names:
/// `addr` is its `DLLInfoReference`'s address (`VECTOR_ELT(obj, 4)`).
// R: dotcode.c:R_FindNativeSymbolFromDLL
pub(crate) fn find_symbol_in_dll(
    cx: &mut Cx,
    addr: *mut c_void,
    name: &[u8],
    ty: SymType,
) -> RResult<Option<(*const c_void, Found)>> {
    if addr.is_null() {
        return Err(cx.error("NULL value for DLLInfoReference when looking for DLL"));
    }
    let info = match dll_of(cx, addr) {
        Some(p) => p,
        None => return Err(not_yet(cx, "a DllInfo reference to an object that is not loaded")),
    };
    // SAFETY: a record of the table (dll_of), not freed while this runs.
    let info = unsafe { &*info };
    if info.force_symbols() {
        return Err(cx.error("DLL requires the use of native symbols"));
    }
    let mut symbol = Found::of(ty);
    let f = dlsym(info, name, &mut symbol);
    Ok((!f.is_null()).then_some((f, symbol)))
}

/// The record a `DLLInfoReference`'s address names, if it is one of the
/// table's (`dyn.unload` clears the references of a record it frees; an
/// address C code made up is refused rather than read).
fn dll_of(cx: &Cx, addr: *mut c_void) -> Option<*const DllInfo> {
    let list = dlls(cx).list.borrow();
    list.iter().map(|b| &**b as *const DllInfo).find(|&p| p == addr.cast_const().cast())
}

// --- the R objects ------------------------------------------------------

/// A character vector of length one (`mkString`).
fn str1<'g>(cx: &'g Cx, b: &[u8]) -> Result<Ref<'g>, NeedGc> {
    let mut v = cx.alloc_vector::<Char<'_>>(1)?;
    v.set(0, cx.mk_char(b, robigo_rtcore::Encoding::Native)?);
    Ok(v.finish().into())
}

/// A character vector of `names`.
fn strs<'g>(cx: &'g Cx, names: &[&[u8]]) -> Result<Ref<'g>, NeedGc> {
    let mut v = cx.alloc_vector::<Char<'_>>(names.len())?;
    for (i, n) in names.iter().enumerate() {
        v.set(i, cx.mk_char(n, robigo_rtcore::Encoding::Native)?);
    }
    Ok(v.finish().into())
}

/// `R_MakeExternalPtr(p, install(tag), R_NilValue)` with class `class`.
fn extptr<'g>(cx: &'g Cx, p: *mut c_void, tag: &[u8], class: &[u8]) -> Result<Ref<'g>, NeedGc> {
    let nil = cx.nil().sexp();
    let t = cx.install(tag)?.as_ref().sexp();
    let class = str1(cx, class)?;
    let attrs = cx.cons_tagged(class, cx.nil(), cx.sym(RSym::Class))?;
    // SAFETY: live objects of this heap; an EXTPTRSXP's car is its address
    // (untraced); the fresh object takes its attributes without a barrier.
    unsafe {
        let x = cx.alloc_node_raw(EXTPTRSXP, nil, nil, t)?;
        x.set_extptr_addr(p.cast());
        x.set_attrib(attrs.sexp());
        x.set_object(true);
        Ok(Ref::from_sexp(x, cx))
    }
}

/// A list of `elts`, its attributes `attrs` (tag, value) in that order, an
/// object when one of them is the class.
fn list_with<'g>(cx: &'g Cx, elts: &[Ref<'g>], attrs: &[(RSym, Ref<'g>)]) -> Result<Ref<'g>, NeedGc> {
    let mut l = cx.alloc_vector::<Ref<'_>>(elts.len())?;
    for (i, &e) in elts.iter().enumerate() {
        l.set(i, e);
    }
    let mut a = cx.nil();
    for &(tag, v) in attrs.iter().rev() {
        a = cx.cons_tagged(v, a, cx.sym(tag))?;
    }
    l.set_attrib(a);
    if attrs.iter().any(|(t, _)| *t == RSym::Class) {
        l.set_object(true);
    }
    Ok(l.finish().into())
}

/// `Rf_MakeDLLInfo(info)`: a list `name`, `path`, `dynamicLookup`, `handle`
/// (class "DLLHandle"), `info` (class "DLLInfoReference") and
/// `forceSymbols`, of class "DLLInfo".
// R: Rdynload.c:Rf_MakeDLLInfo, Rf_makeDllObject, Rf_makeDllInfoReference
fn dll_info<'g>(cx: &'g Cx, info: &DllInfo, made: &mut Made) -> Result<Ref<'g>, NeedGc> {
    let ehandle = extptr(cx, info.handle, b"DLLHandle", b"DLLHandle")?;
    // R: Rdynload.c:Rf_makeDllInfoReference (DLLInfoEptrs)
    let einfo = match info.eref.get() {
        Some(g) => g.bind(cx),
        None => {
            let e = extptr(cx, (info as *const DllInfo).cast_mut().cast(), b"DLLInfo", b"DLLInfoReference")?;
            // (a build retried after a collection reuses it)
            let _ = info.eref.set(cx.global(e));
            e
        }
    };
    // R_registerSymbolEptr(ehandle, einfo), once the build is done
    made.eptrs.push((ehandle.sexp(), einfo.sexp()));
    let elts = [
        str1(cx, info.name.as_bytes())?,
        str1(cx, info.path.as_bytes())?,
        cx.logical_value(info.dynamic_lookup() as i32),
        ehandle,
        einfo,
        cx.logical_value(info.force_symbols() as i32),
    ];
    let names = strs(cx, &[b"name", b"path", b"dynamicLookup", b"handle", b"info", b"forceSymbols"])?;
    let class = str1(cx, b"DLLInfo")?;
    list_with(cx, &elts, &[(RSym::Names, names), (RSym::Class, class)])
}

/// The address of a `RegisteredNativeSymbol`: GNU R's malloc'd copy of the
/// lookup's result, freed by the reference's C finalizer.
// R: Rdynload.c:freeRegisteredNativeSymbolCopy
extern "C" fn free_registered_symbol(r: Sexp) {
    // SAFETY: the collector runs this once, on the external pointer
    // `registered_symbol_ref` made, whose address is a `Box<Found>` (or null).
    unsafe {
        let p = r.car().as_ptr().cast::<Found>();
        if !p.is_null() {
            drop(Box::from_raw(p));
        }
    }
}

/// `createRSymbolObject(sname, f, symbol, withRegistrationInfo)`: a list
/// `name`, `address`, `dll` and, for a registered routine, `numParameters`,
/// of class "NativeSymbolInfo" (after "CallRoutine", ... for a registered
/// one). Its address is a "RegisteredNativeSymbol" holding a copy of
/// `symbol` when registration information is asked for, else a
/// "NativeSymbol" holding `f`. `copy` is that copy's address, once
/// allocated (the C finalizer is registered after the object is built).
// R: Rdynload.c:createRSymbolObject, Rf_MakeNativeSymbolRef,
// Rf_MakeRegisteredNativeSymbol
fn symbol_object<'g>(
    cx: &'g Cx,
    sname: Option<Ref<'g>>,
    f: *const c_void,
    symbol: &Found,
    copy: Option<*mut Found>,
    made: &mut Made,
) -> Result<Ref<'g>, NeedGc> {
    let registered = symbol.ty != SymType::Any;
    let sname = match sname {
        Some(s) => s,
        // SAFETY: a lookup's routine is an element of a loaded record.
        None => str1(cx, &unsafe { &*symbol.routine }.name)?,
    };
    let address = match copy {
        Some(p) => {
            let e = extptr(cx, p.cast(), b"registered native symbol", b"RegisteredNativeSymbol")?;
            made.regs.push(e.sexp());
            e
        }
        None => extptr(cx, f.cast_mut(), b"native symbol", b"NativeSymbol")?,
    };
    let dll = if symbol.dll.is_null() {
        cx.nil()
    } else {
        // SAFETY: as above: the record is loaded.
        let rinfo = dll_info(cx, unsafe { &*symbol.dll }, made)?;
        // R_registerSymbolEptr(eaddress, einfo), once the build is done
        let einfo = rinfo.as_vector::<Ref<'_>>().unwrap().get(4);
        made.eptrs.push((address.sexp(), einfo.sexp()));
        rinfo
    };
    let mut elts = vec![sname, address, dll];
    let mut names: Vec<&[u8]> = vec![b"name", b"address", b"dll"];
    let mut class: Vec<&[u8]> = vec![b"NativeSymbolInfo"];
    if registered {
        let class_name: &[u8] = match symbol.ty {
            SymType::C => b"CRoutine",
            SymType::Call => b"CallRoutine",
            SymType::Fortran => b"FortranRoutine",
            SymType::External => b"ExternalRoutine",
            SymType::Any => unreachable!(),
        };
        // SAFETY: as above: the record is loaded.
        let n = if symbol.routine.is_null() { -1 } else { unsafe { &*symbol.routine }.num_args };
        elts.push(cx.alloc_copy(&[n])?.finish().into());
        names.push(b"numParameters");
        class.insert(0, class_name);
    }
    let class = strs(cx, &class)?;
    let names = strs(cx, &names)?;
    list_with(cx, &elts, &[(RSym::Class, class), (RSym::Names, names)])
}

/// `R_getRoutineSymbols(type, info)`: the routines of one interface, a list
/// of class "NativeRoutineList".
// R: Rdynload.c:R_getRoutineSymbols
fn routine_list<'g>(cx: &'g Cx, info: &DllInfo, t: SymType, made: &mut Made) -> Result<Ref<'g>, NeedGc> {
    // the routines' addresses first: building runs no C, but holds no
    // borrow of the table while it allocates
    let routines: Vec<*const Routine> = info.routines(t).iter().map(|r| r as *const Routine).collect();
    let mut elts = Vec::new();
    for r in routines {
        let symbol = Found { ty: t, routine: r, dll: info };
        // SAFETY: an element of the loaded record's table.
        let fun = unsafe { &*r }.fun;
        let copy = made.next(symbol);
        elts.push(symbol_object(cx, None, fun, &symbol, Some(copy), made)?);
    }
    let class = str1(cx, b"NativeRoutineList")?;
    list_with(cx, &elts, &[(RSym::Class, class)])
}

/// What a build hands out that needs work once it is done: the copies
/// "RegisteredNativeSymbol" references hold, the references made in the
/// build's last (successful) run, and the external pointers to enter in
/// `SymbolEptrs`, each with its object's `DLLInfoReference`. A run that
/// needs a collection is retried from the start: its copies are reused in
/// order.
#[derive(Default)]
struct Made {
    all: Vec<*mut Found>,
    used: usize,
    regs: Vec<Sexp>,
    eptrs: Vec<(Sexp, Sexp)>,
}

impl Made {
    fn start(&mut self) {
        self.used = 0;
        self.regs.clear();
        self.eptrs.clear();
    }
    fn next(&mut self, symbol: Found) -> *mut Found {
        if self.used == self.all.len() {
            self.all.push(Box::into_raw(Box::new(symbol)));
        }
        let p = self.all[self.used];
        // SAFETY: a box of ours, not yet handed to a reference.
        unsafe { *p = symbol };
        self.used += 1;
        p
    }
    /// Register each made reference's finalizer, free the copies no
    /// reference took, and enter the external pointers in `SymbolEptrs`.
    fn finish(mut self, cx: &mut Cx, regs: &[Root<'_>], eptrs: &[(Root<'_>, Root<'_>)]) -> RResult<()> {
        for p in self.all.drain(self.used..) {
            // SAFETY: ours, never handed out.
            drop(unsafe { Box::from_raw(p) });
        }
        // the rest belong to the references now: their finalizers free them
        // (one whose registration fails below is leaked, never freed twice)
        self.all.clear();
        for &r in regs {
            crate::memory::register_c_finalizer(cx, r, free_registered_symbol as *const c_void)?;
        }
        for &(e, i) in eptrs {
            register_symbol_eptr(cx, e, i)?;
        }
        Ok(())
    }
}

impl Drop for Made {
    fn drop(&mut self) {
        // a build that failed: no reference holds a copy
        for p in self.all.drain(..) {
            // SAFETY: ours; `finish` forgot the ones references took.
            drop(unsafe { Box::from_raw(p) });
        }
    }
}

/// Build `f`'s result into `out`, then do what [`Made::finish`] does for
/// what it handed out.
fn build_made(
    cx: &mut Cx,
    out: Out<'_>,
    mut f: impl for<'g> FnMut(&'g Cx, &mut Made) -> Result<Ref<'g>, NeedGc>,
) -> RResult<()> {
    let mut made = Made::default();
    cx.scope(|cx, s| {
        let res = s.out(cx);
        let r = res.root();
        cx.build(res, |cx| {
            made.start();
            f(cx, &mut made)
        })?;
        // root what the result holds while the work allocates
        let held = |cx: &mut Cx, x: Sexp| {
            // SAFETY: an object of the result just built, rooted by `r`, and
            // nothing has collected since.
            s.root(cx, unsafe { Ref::from_sexp(x, cx) })
        };
        let regs: Vec<Root<'_>> = std::mem::take(&mut made.regs).into_iter().map(|x| held(cx, x)).collect();
        let eptrs: Vec<(Root<'_>, Root<'_>)> =
            std::mem::take(&mut made.eptrs).into_iter().map(|(e, i)| (held(cx, e), held(cx, i))).collect();
        std::mem::take(&mut made).finish(cx, &regs, &eptrs)?;
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// A weak reference's key and value (`WEAKREF_KEY`, `WEAKREF_VALUE`).
///
/// # Safety
/// `w` is a live WEAKREFSXP of this isolate.
unsafe fn weak_key_value(w: Sexp) -> (Sexp, Sexp) {
    // SAFETY: the caller's contract: a weak reference's elements 0 and 1.
    unsafe {
        let d = w.std_data().cast::<Sexp>();
        (*d, *d.add(1))
    }
}

/// `R_registerSymbolEptr(eptr, einfo)`: every `MAX_COUNT`-th call drops the
/// entries whose key is gone, then a weak reference keyed by `eptr` with
/// `einfo` as value goes at the head of `SymbolEptrs`.
// R: Rdynload.c:R_registerSymbolEptr
fn register_symbol_eptr(cx: &mut Cx, eptr: Root<'_>, einfo: Root<'_>) -> RResult<()> {
    let d = dlls(cx);
    let c = d.clean_count.get() - 1;
    if c == 0 {
        d.clean_count.set(MAX_COUNT);
        let nil = cx.nil().sexp();
        let head = d.symbol_eptrs.bind(cx).sexp();
        cx.scope(|cx, s| {
            // (nothing below allocates: the words stay put)
            // SAFETY: SymbolEptrs is a pairlist of weak references.
            unsafe {
                let mut last = head;
                let mut next = head.cdr();
                while next != nil {
                    if weak_key_value(next.car()).0 == nil {
                        let l = s.root(cx, Ref::from_sexp(last, cx));
                        let rest = s.root(cx, Ref::from_sexp(next.cdr(), cx));
                        cx.set_cdr(l, rest);
                    } else {
                        last = next;
                    }
                    next = next.cdr();
                }
            }
        });
    } else {
        d.clean_count.set(c);
    }
    cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        let w = crate::memory::new_weak_ref(cx, s, eptr, einfo, nil, false)?;
        // SAFETY: the fresh reference, rooted by the collector's list.
        let w = s.root(cx, unsafe { Ref::from_sexp(w, cx) });
        let head = s.root(cx, dlls(cx).symbol_eptrs.bind(cx));
        let rest = s.root(cx, head.bind(cx).as_cons().unwrap().cdr());
        let o = s.out(cx);
        let cell = o.root();
        cx.build(o, |cx| cx.cons(w.bind(cx), rest.bind(cx)))?;
        cx.set_cdr(head, cell);
        Ok(())
    })
}

/// `.Internal(getLoadedDLLs())`: a list of the loaded objects' "DLLInfo",
/// named by their names, of class "DLLInfoList".
// R: Rdynload.c:do_getDllTable, R_getDllTable
fn do_get_dll_table(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    robigo_rt::prims::check_arity(cx, call, args.len(), "getLoadedDLLs", 0, true)?;
    build_made(cx, out, |cx, made| {
        let list = dlls(cx).list.borrow();
        let elts = list.iter().map(|i| dll_info(cx, i, made)).collect::<Result<Vec<_>, _>>()?;
        let names: Vec<&[u8]> = list.iter().map(|i| i.name.as_bytes()).collect();
        let class = str1(cx, b"DLLInfoList")?;
        let names = strs(cx, &names)?;
        list_with(cx, &elts, &[(RSym::Class, class), (RSym::Names, names)])
    })
}

/// The address of a `DLLInfoReference` (`R_ExternalPtrAddr`), checked as
/// `R_getRegisteredRoutines` and `R_getSymbolInfo` check it.
fn dll_reference(cx: &mut Cx, addr: *mut c_void) -> RResult<*const DllInfo> {
    if addr.is_null() {
        return Err(cx.error("NULL value passed for DllInfo"));
    }
    match dll_of(cx, addr) {
        Some(p) => Ok(p),
        None => Err(not_yet(cx, "a DllInfo reference to an object that is not loaded")),
    }
}

/// `.Internal(getRegisteredRoutines(dll))`: the routines registered in the
/// object `dll` refers to, a list `.C`, `.Call`, `.Fortran`, `.External`.
// R: Rdynload.c:do_getRegisteredRoutines, R_getRegisteredRoutines
fn do_get_registered_routines(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    robigo_rt::prims::check_arity(cx, call, args.len(), "getRegisteredRoutines", 1, true)?;
    let dll = args.get(cx, 0);
    // GNU R tests `TYPEOF(dll) != EXTPTRSXP && R_ExternalPtrTag(dll) !=
    // install("DLLInfo")`: the tag is read only when `dll` is not an
    // external pointer, so R_ExternalPtrTag's own check raises
    if dll.sexptype() != EXTPTRSXP {
        let t = robigo_rtbuiltins::util::sexptype2char(dll.sexptype());
        return Err(cx.error(&format!("R_ExternalPtrTag: argument of type {t} is not an external pointer")));
    }
    let View::ExtPtr(e) = dll.view() else { unreachable!("an external pointer") };
    let addr = e.addr();
    let info = dll_reference(cx, addr)?;
    build_made(cx, out, |cx, made| {
        // SAFETY: a record of the table (dll_reference), loaded while this
        // runs.
        let info = unsafe { &*info };
        let elts = [
            routine_list(cx, info, SymType::C, made)?,
            routine_list(cx, info, SymType::Call, made)?,
            routine_list(cx, info, SymType::Fortran, made)?,
            routine_list(cx, info, SymType::External, made)?,
        ];
        let names = strs(cx, &[b".C", b".Call", b".Fortran", b".External"])?;
        list_with(cx, &elts, &[(RSym::Names, names)])
    })
}

/// `translateCharFP(s)`: `s` in the native encoding, or the message of its
/// error.
// R: sysutils.c:translateCharFP
fn translate_fp(cx: &Cx, s: Char<'_>) -> Result<Vec<u8>, String> {
    robigo_rtbuiltins::text::to_native_fp(robigo_rtbuiltins::coerce::text(s), cx.locale()).map(|b| b.into_owned())
}

/// `.Internal(getSymbolInfo(name, package, withRegistrationInfo))`: the
/// routine `name` (in `package`, a name or a `DLLInfoReference`; any loaded
/// object when it is empty), as `createRSymbolObject` describes it, or NULL.
// R: Rdynload.c:do_getSymbolInfo, R_getSymbolInfo
fn do_get_symbol_info(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    robigo_rt::prims::check_arity(cx, call, args.len(), "getSymbolInfo", 3, true)?;
    let sname = args.get(cx, 0);
    let name = match sname.as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => translate_fp(cx, v.get(0)),
        _ => return Err(cx.error("invalid 'name' argument")),
    };
    let name = match name {
        Ok(n) => n,
        Err(m) => return Err(cx.error(&m)),
    };
    let spackage = args.get(cx, 1);
    let mut symbol = Found::of(SymType::Any);
    // `length(spackage)`: a name or a reference here
    let f = if spackage.sexptype() == NILSXP || spackage.len() == Some(0) {
        find_symbol_in(&dlls(cx).list.borrow(), &name, "", &mut symbol)
    } else if let Some(v) = spackage.as_vector::<Char<'_>>() {
        let pkg = match translate_fp(cx, v.get(0)) {
            Ok(p) => p,
            Err(m) => return Err(cx.error(&m)),
        };
        let pkg = String::from_utf8_lossy(&pkg).into_owned();
        find_symbol_in(&dlls(cx).list.borrow(), &name, &pkg, &mut symbol)
    } else if let View::ExtPtr(e) = spackage.view()
        && e.tag().as_sym().is_some_and(|t| t.name().bytes() == b"DLLInfo")
    {
        let addr = e.addr();
        let info = dll_reference(cx, addr)?;
        // SAFETY: a record of the table (dll_reference), loaded while this
        // runs.
        dlsym(unsafe { &*info }, &name, &mut symbol)
    } else {
        return Err(cx.error("must pass package name or DllInfo reference"));
    };
    if f.is_null() {
        out.set(cx, cx.nil());
        return Ok(());
    }
    // asBool2(withRegistrationInfo, R_NilValue)
    let with_reg = {
        let x = args.get(cx, 2);
        let (v, n) = robigo_rtbuiltins::util::as_logical_n(cx, x);
        if robigo_rtbuiltins::util::is_vector_atomic(x) && n > 1 {
            return Err(cx.error_call(None, &format!("'length = {n}' in coercion to 'logical(1)'")));
        }
        if v == robigo_heap::NA_LOGICAL {
            return Err(cx.error_call(None, "NA in coercion to boolean"));
        }
        v != 0
    };
    let sname = args.root(0);
    build_made(cx, out, |cx, made| {
        let copy = (with_reg && !symbol.routine.is_null() && !symbol.dll.is_null()).then(|| made.next(symbol));
        symbol_object(cx, Some(sname.bind(cx)), f, &symbol, copy, made)
    })
}

// --- dyn.load -----------------------------------------------------------

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
        let d = dlls(cx);
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
                        // R_RegisterDLL: old-style dynamic lookup until the
                        // object's R_init_<name> says otherwise
                        list.push(Box::new(DllInfo::new(name, path.clone(), h, true, false)));
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
    // the record AddDLL returns: boxed, so its address stays while loaded
    let rec: *const DllInfo = {
        let l = dlls(cx).list.borrow();
        &**l.last().unwrap()
    };
    if k == usize::MAX {
        // a new object: its R_init_<name>, or with `.` as `_` (a package
        // name's `.` is not valid in a symbol), called with the record
        // SAFETY: the record just pushed, loaded.
        let (name, h) = unsafe { ((*rec).name.clone(), (*rec).handle) };
        let init = |n: &str| -> *mut c_void {
            let Ok(s) = CString::new(format!("R_init_{n}")) else { return std::ptr::null_mut() };
            // SAFETY: a live handle and a symbol name.
            unsafe { libc::dlsym(h, s.as_ptr()) }
        };
        let mut f = init(&name);
        if f.is_null() {
            f = init(&name.replace('.', "_"));
        }
        if !f.is_null() {
            // SAFETY: GNU R's contract for R_init_<name>: `void f(DllInfo *)`,
            // a word argument returning nothing (Ret6::Word reads a register
            // the callee may leave anything in, and the value is dropped).
            // GNU R calls it directly, with no context: a hidden record.
            unsafe { cx.call_native6(f, [rec as usize, 0, 0, 0, 0, 0], 0, robigo_rtcore::Ret6::Word) }?;
        }
    }
    // SAFETY: loaded: R_init_<name> runs C that may load or unload others,
    // not this one (GNU R reads `info` the same way after the call).
    build_made(cx, out, |cx, made| dll_info(cx, unsafe { &*rec }, made))
}

/// `R_callDLLUnload(info)`: the object's `R_unload_<name>`, as `R_dlsym`
/// finds it (registered routines first; `dlsym` only when the object
/// allows dynamic lookup), called with the record.
// R: Rdynload.c:R_callDLLUnload
fn call_dll_unload(cx: &mut Cx, rec: *const DllInfo) -> RResult<()> {
    // SAFETY: a loaded record (the caller's).
    let info = unsafe { &*rec };
    let mut symbol = Found::of(SymType::Any);
    let f = dlsym(info, format!("R_unload_{}", info.name).as_bytes(), &mut symbol);
    if !f.is_null() {
        // SAFETY: as R_init_<name>'s call in do_dynload.
        unsafe { cx.call_native6(f, [rec as usize, 0, 0, 0, 0, 0], 0, robigo_rtcore::Ret6::Word) }?;
    }
    Ok(())
}

/// `DeleteDLL(path)`: the object loaded from `path` unloaded (its
/// `R_unload_<name>` called, its ALTREP classes reset, the library closed)
/// and its record freed, after every reference handed out for it is
/// cleared (`DLLInfoEptrs`, `SymbolEptrs`). False if no object was loaded
/// from `path`.
// R: Rdynload.c:DeleteDLL
fn delete_dll(cx: &mut Cx, path: &str) -> RResult<bool> {
    let rec: *const DllInfo = {
        let l = dlls(cx).list.borrow();
        match l.iter().find(|i| i.path == path) {
            Some(i) => &**i,
            None => return Ok(false),
        }
    };
    crate::altrep::reinit_classes(cx, rec.cast_mut().cast())?;
    call_dll_unload(cx, rec)?;
    // no cached lookup may name the record once it goes (`R_unload_<name>`
    // may have looked routines up by name)
    dlls(cx).found.borrow_mut().clear();
    // the record leaves the table now (`R_unload_<name>` may have changed
    // it: found again by address)
    let mut info = {
        let mut l = dlls(cx).list.borrow_mut();
        match l.iter().position(|i| std::ptr::eq(&**i, rec)) {
            Some(k) => l.remove(k),
            None => return Ok(true),
        }
    };
    if !info.handle.is_null() {
        // SAFETY: the handle dlopen gave this record, closed once.
        unsafe { libc::dlclose(info.handle) };
    }
    // zero the pointers to the record and its handle: its DLLInfoReference,
    // then each reference SymbolEptrs holds for it
    // (the symbol first: making it may collect, which may move the words
    // read below)
    let registered = cx.scope(|cx, s| -> RResult<Sexp> {
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| Ok(cx.install(b"registered native symbol")?.into()))?;
        Ok(r.bind(cx).sexp())
    })?;
    let e = info.eref.get().map(|g| g.bind(cx).sexp());
    if let Some(e) = e {
        let nil = cx.nil().sexp();
        let head = dlls(cx).symbol_eptrs.bind(cx).sexp();
        cx.scope(|cx, s| {
            // (nothing below allocates: the words stay put)
            // SAFETY: e and the list's keys are external pointers (their
            // addresses untraced words); SymbolEptrs is a pairlist of weak
            // references.
            unsafe {
                e.set_extptr_addr(std::ptr::null_mut());
                let mut last = head;
                let mut next = head.cdr();
                while next != nil {
                    let (p, v) = weak_key_value(next.car());
                    if v == e {
                        let l = s.root(cx, Ref::from_sexp(last, cx));
                        let rest = s.root(cx, Ref::from_sexp(next.cdr(), cx));
                        cx.set_cdr(l, rest);
                        if p.typ() == EXTPTRSXP && !p.car().as_ptr().is_null() {
                            if p.tag() == registered {
                                // freeRegisteredNativeSymbolCopy
                                drop(Box::from_raw(p.car().as_ptr().cast::<Found>()));
                            }
                            p.set_extptr_addr(std::ptr::null_mut());
                        }
                    } else {
                        last = next;
                    }
                    next = next.cdr();
                }
            }
        });
    }
    // the record goes, and its DLLInfoReference's root (DLLInfoEptrs' slot)
    if let Some(g) = info.eref.take() {
        cx.release_global(g);
    }
    drop(info);
    Ok(true)
}

/// `.Internal(dyn.unload(x))`.
// R: Rdynload.c:do_dynunload
fn do_dynunload(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Some(x) = string1(cx, args.get(cx, 0)) else {
        return Err(cx.error("character argument expected"));
    };
    let path = full_dll_path(cx, call, &String::from_utf8_lossy(&x))?;
    if !delete_dll(cx, &path)? {
        return Err(cx.error(&format!("shared object '{path}' was not loaded")));
    }
    out.set(cx, cx.nil());
    Ok(())
}

// --- the registration API (R_ext/Rdynload.h) ----------------------------

/// `R_CMethodDef` and `R_FortranMethodDef` (`R_ext/Rdynload.h`): GNU R's
/// layout, checked here and against GNU R's header in the C API tests.
#[repr(C)]
pub struct CMethodDef {
    pub name: *const c_char,
    pub fun: *const c_void,
    pub num_args: c_int,
    pub types: *const c_void,
}

/// `R_CallMethodDef` and `R_ExternalMethodDef` (`R_ext/Rdynload.h`).
#[repr(C)]
pub struct CallMethodDef {
    pub name: *const c_char,
    pub fun: *const c_void,
    pub num_args: c_int,
}

// GNU R's layout on x86-64 (LP64): the C API tests assert the same numbers
// with `static_assert` against R_ext/Rdynload.h
const _: () = {
    assert!(std::mem::size_of::<CMethodDef>() == 32);
    assert!(std::mem::offset_of!(CMethodDef, name) == 0);
    assert!(std::mem::offset_of!(CMethodDef, fun) == 8);
    assert!(std::mem::offset_of!(CMethodDef, num_args) == 16);
    assert!(std::mem::offset_of!(CMethodDef, types) == 24);
    assert!(std::mem::size_of::<CallMethodDef>() == 24);
    assert!(std::mem::offset_of!(CallMethodDef, name) == 0);
    assert!(std::mem::offset_of!(CallMethodDef, fun) == 8);
    assert!(std::mem::offset_of!(CallMethodDef, num_args) == 16);
};

/// The routines of a NULL-terminated table (`R_addCRoutine`, ...: the
/// name copied, the address, `numArgs` with anything below -1 as -1).
///
/// # Safety
/// `defs` is null or points at a table of `T` whose last entry has a null
/// name, each name a C string.
unsafe fn copy_table<T>(
    defs: *const T,
    name: impl Fn(&T) -> *const c_char,
    entry: impl Fn(&T) -> (*const c_void, c_int),
) -> Routines {
    let mut out = Vec::new();
    if defs.is_null() {
        return out.into_boxed_slice();
    }
    // SAFETY: the caller's contract.
    unsafe {
        let mut p = defs;
        while !name(&*p).is_null() {
            let (fun, n) = entry(&*p);
            out.push(Routine { name: CStr::from_ptr(name(&*p)).to_bytes().to_vec(), fun, num_args: n.max(-1) });
            p = p.add(1);
        }
    }
    out.into_boxed_slice()
}

crate::entry! {
    /// `R_registerRoutines(info, croutines, callRoutines, fortranRoutines,
    /// externalRoutines)`: the tables copied into the record (a table given
    /// before is kept until the record goes, as GNU R leaks it), dynamic
    /// lookup on when the object has a handle, symbols not forced.
    // R: Rdynload.c:R_registerRoutines, R_addCRoutine, R_addCallRoutine,
    // R_addFortranRoutine, R_addExternalRoutine
    [5] fn R_registerRoutines(
        info: *mut c_void,
        croutines: *const c_void,
        call_routines: *const c_void,
        fortran_routines: *const c_void,
        external_routines: *const c_void
    ) -> c_int => |cx, _s| {
        if info.is_null() {
            return Err(cx.error("R_RegisterRoutines called with invalid DllInfo object."));
        }
        // SAFETY: C's contract: the DllInfo * robigo handed out (R_init_<name>'s
        // argument), a loaded record; the tables as R_ext/Rdynload.h has them.
        unsafe {
            let info = &*info.cast_const().cast::<DllInfo>();
            info.dynamic_lookup.set(!info.handle.is_null());
            info.force_symbols.set(false);
            let c = |d: &CMethodDef| (d.fun, d.num_args);
            let k = |d: &CallMethodDef| (d.fun, d.num_args);
            let mut tables = info.tables.borrow_mut();
            let mut old = std::mem::take(&mut *tables);
            if !croutines.is_null() {
                tables.c = copy_table(croutines.cast::<CMethodDef>(), |d| d.name, c);
            } else {
                tables.c = std::mem::take(&mut old.c);
            }
            if !fortran_routines.is_null() {
                tables.fortran = copy_table(fortran_routines.cast::<CMethodDef>(), |d| d.name, c);
            } else {
                tables.fortran = std::mem::take(&mut old.fortran);
            }
            if !call_routines.is_null() {
                tables.call = copy_table(call_routines.cast::<CallMethodDef>(), |d| d.name, k);
            } else {
                tables.call = std::mem::take(&mut old.call);
            }
            if !external_routines.is_null() {
                tables.external = copy_table(external_routines.cast::<CallMethodDef>(), |d| d.name, k);
            } else {
                tables.external = std::mem::take(&mut old.external);
            }
            drop(tables);
            info.retired.borrow_mut().push(old);
        }
        dlls(cx).found.borrow_mut().clear();
        Ok(1)
    }
}

/// `R_useDynamicSymbols(info, value)`: the old setting.
// R: Rdynload.c:R_useDynamicSymbols
#[unsafe(no_mangle)]
pub extern "C" fn R_useDynamicSymbols(info: *mut c_void, value: c_int) -> c_int {
    // SAFETY: C's contract: the DllInfo * robigo handed out, a loaded record.
    let info = unsafe { &*info.cast_const().cast::<DllInfo>() };
    let old = info.dynamic_lookup.replace(value != 0);
    forget_lookups();
    old as c_int
}

/// `R_forceSymbols(info, value)`: the old setting.
// R: Rdynload.c:R_forceSymbols
#[unsafe(no_mangle)]
pub extern "C" fn R_forceSymbols(info: *mut c_void, value: c_int) -> c_int {
    // SAFETY: C's contract: the DllInfo * robigo handed out, a loaded record.
    let info = unsafe { &*info.cast_const().cast::<DllInfo>() };
    let old = info.force_symbols.replace(value != 0);
    forget_lookups();
    old as c_int
}

/// The cached `.Call` lookups dropped: a record's lookup settings changed,
/// which may change what any lookup finds (GNU R caches none on Linux).
fn forget_lookups() {
    // SAFETY: called from C code a native entry of the thread's isolate
    // runs (the setters' contract, as GNU R's: `R_init_<name>`'s).
    unsafe { robigo_rtcore::enter_leaf(|cx| dlls(cx).found.borrow_mut().clear()) }
}
