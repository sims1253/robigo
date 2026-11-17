//! Rdynload.c: one isolate's table of loaded shared objects (`LoadedDLL`,
//! base first, as `InitDynload` makes it), the routines registered in them
//! (`R_init_base`'s, registration.c), `dyn.load` with GNU R's `dlopen`
//! flags, symbol lookup (`R_FindSymbol`, `R_dlsym`: registered routines
//! first, then `dlsym` when the object allows dynamic lookup) and the R
//! objects that describe them: `getLoadedDLLs()`, `getRegisteredRoutines`
//! and `getSymbolInfo` (`getNativeSymbolInfo`).
//!
//! As GNU R does (`DLLInfoEptrs`), each object's `DLLInfoReference` is made
//! once and kept rooted. GNU R also keeps the weak list `SymbolEptrs`, so
//! that `dyn.unload` can clear the addresses of the references it handed
//! out. robigo-rt has no `dyn.unload` yet: an object's record never moves
//! or goes away (each is boxed), and every reference to it stays valid.

use crate::errors::not_yet;
use robigo_rt::prims::{Inline, Kind, PrimDef};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, NeedGc, Out, RResult, RSym, Ref, Root, Sexp, View};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_void};

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

/// One loaded shared object: GNU R's `DllInfo`.
#[derive(Debug)]
pub struct DllInfo {
    /// The object's name: its file name without `.so`.
    pub name: String,
    /// The full path it was loaded from.
    pub path: String,
    /// `dlopen`'s handle (null for base).
    pub handle: *mut c_void,
    /// `useDynamicLookup`: routines are found by `dlsym` too.
    pub dynamic_lookup: bool,
    /// `forceSymbols`: its routines are not found by name alone.
    pub force_symbols: bool,
    /// The registered routines, by interface (`CSymbols`, `CallSymbols`,
    /// `FortranSymbols`, `ExternalSymbols`).
    pub c: Vec<Routine>,
    pub call: Vec<Routine>,
    pub fortran: Vec<Routine>,
    pub external: Vec<Routine>,
    /// Its `DLLInfoReference`, made once and rooted while it is loaded
    /// (GNU R's `DLLInfoEptrs`): every `DLLInfo` of it shares the object.
    eref: std::cell::OnceCell<robigo_rtcore::Global>,
}

impl DllInfo {
    fn routines(&self, t: SymType) -> &[Routine] {
        match t {
            SymType::C => &self.c,
            SymType::Call => &self.call,
            SymType::Fortran => &self.fortran,
            SymType::External => &self.external,
            SymType::Any => &[],
        }
    }
}

/// An isolate's loaded objects (`LoadedDLL`, `CountDLL`), in load order,
/// and the `.Call` routines found in them by name.
#[derive(Default)]
struct Dlls {
    /// Boxed: a `DLLInfoReference` holds a record's address.
    list: RefCell<Vec<Box<DllInfo>>>,
    /// `R_FindSymbol`'s `.Call` results by (name, `PACKAGE`): what the
    /// search over `list` returns, so cleared whenever `list` changes. (GNU
    /// R caches only on Windows, `CACHE_DLL_SYM`; elsewhere every `.Call`
    /// searches. Cached or not, the routine found is the same.)
    found: RefCell<std::collections::HashMap<Vec<u8>, Vec<(Option<String>, usize)>, robigo_heap::BuildFx>>,
}

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
    let base = DllInfo {
        name: "base".into(),
        path: "base".into(),
        handle: std::ptr::null_mut(),
        // R_registerRoutines sets it from the handle; R_init_base's
        // R_useDynamicSymbols(dll, FALSE) and R_forceSymbols(dll, TRUE)
        dynamic_lookup: false,
        force_symbols: true,
        c: Vec::new(),
        // registration.c's callMethods
        call: vec![
            call(b"R_addTaskCallback", robigo_rtapi_base_R_addTaskCallback as *const c_void, 4),
            call(b"R_getTaskCallbackNames", robigo_rtapi_base_R_getTaskCallbackNames as *const c_void, 0),
            call(b"R_removeTaskCallback", robigo_rtapi_base_R_removeTaskCallback as *const c_void, 1),
        ],
        // registration.c's fortranMethods (LINPACK)
        fortran: vec![
            call(b"dqrcf", base_fortran_dqrcf as *const c_void, 8),
            call(b"dqrdc2", base_fortran_dqrdc2 as *const c_void, 9),
            call(b"dqrqty", base_fortran_dqrqty as *const c_void, 7),
            call(b"dqrqy", base_fortran_dqrqy as *const c_void, 7),
            call(b"dqrrsd", base_fortran_dqrrsd as *const c_void, 7),
            call(b"dqrxb", base_fortran_dqrxb as *const c_void, 7),
            call(b"dtrco", base_fortran_dtrco as *const c_void, 6),
        ],
        external: Vec::new(),
        eref: Default::default(),
    };
    cx.set_ext(Dlls { list: RefCell::new(vec![Box::new(base)]), found: Default::default() });
    robigo_rt::define_builtins(
        cx,
        &[
            // names.c: {"dyn.load", do_dynload, 0, 111, 4, ...}
            PrimDef { name: "dyn.load", kind: Kind::Builtin(do_dynload, Inline::None), eval: 111, arity: 4 },
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
    fn of(ty: SymType) -> Found {
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
    if !info.dynamic_lookup || info.handle.is_null() {
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
        if (all || named) && !info.force_symbols {
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

/// `.Call`'s `R_FindSymbol(name, pkg, &symbol)` with a `.Call` symbol, by
/// name (`pkg`: `PACKAGE`, if given).
// R: dotcode.c:resolveNativeRoutine (R_FindSymbol with R_CALL_SYM)
pub(crate) fn find_call_symbol(cx: &Cx, name: &[u8], pkg: Option<&str>) -> Option<*const c_void> {
    let d = cx.ext::<Dlls>()?;
    // (a hit allocates nothing)
    if let Some(v) = d.found.borrow().get(name)
        && let Some(&(_, f)) = v.iter().find(|(p, _)| p.as_deref() == pkg)
    {
        return Some(f as *const c_void);
    }
    let mut symbol = Found::of(SymType::Call);
    let f = find_symbol_in(&d.list.borrow(), name, pkg.unwrap_or(""), &mut symbol);
    if f.is_null() {
        return None;
    }
    d.found.borrow_mut().entry(name.to_vec()).or_default().push((pkg.map(str::to_owned), f as usize));
    Some(f)
}

/// The record a `DLLInfoReference`'s address names, if it is one of the
/// table's (robigo-rt never frees one, see the module's documentation; an
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
fn dll_info<'g>(cx: &'g Cx, info: &DllInfo) -> Result<Ref<'g>, NeedGc> {
    let elts = [
        str1(cx, info.name.as_bytes())?,
        str1(cx, info.path.as_bytes())?,
        cx.logical_value(info.dynamic_lookup as i32),
        extptr(cx, info.handle, b"DLLHandle", b"DLLHandle")?,
        // R: Rdynload.c:Rf_makeDllInfoReference (DLLInfoEptrs)
        match info.eref.get() {
            Some(g) => g.bind(cx),
            None => {
                let e = extptr(cx, (info as *const DllInfo).cast_mut().cast(), b"DLLInfo", b"DLLInfoReference")?;
                // (a build retried after a collection reuses it)
                let _ = info.eref.set(cx.global(e));
                e
            }
        },
        cx.logical_value(info.force_symbols as i32),
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
) -> Result<(Ref<'g>, Option<Ref<'g>>), NeedGc> {
    let registered = symbol.ty != SymType::Any;
    let sname = match sname {
        Some(s) => s,
        // SAFETY: a lookup's routine is an element of a live record
        // (records are never freed).
        None => str1(cx, &unsafe { &*symbol.routine }.name)?,
    };
    let (address, reg) = match copy {
        Some(p) => {
            let e = extptr(cx, p.cast(), b"registered native symbol", b"RegisteredNativeSymbol")?;
            (e, Some(e))
        }
        None => (extptr(cx, f.cast_mut(), b"native symbol", b"NativeSymbol")?, None),
    };
    // SAFETY: as above: the record is live.
    let dll = if symbol.dll.is_null() { cx.nil() } else { dll_info(cx, unsafe { &*symbol.dll })? };
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
        // SAFETY: as above: the record is live.
        let n = if symbol.routine.is_null() { -1 } else { unsafe { &*symbol.routine }.num_args };
        elts.push(cx.alloc_copy(&[n])?.finish().into());
        names.push(b"numParameters");
        class.insert(0, class_name);
    }
    let class = strs(cx, &class)?;
    let names = strs(cx, &names)?;
    Ok((list_with(cx, &elts, &[(RSym::Class, class), (RSym::Names, names)])?, reg))
}

/// `R_getRoutineSymbols(type, info)`: the routines of one interface, a list
/// of class "NativeRoutineList".
// R: Rdynload.c:R_getRoutineSymbols
fn routine_list<'g>(cx: &'g Cx, info: &DllInfo, t: SymType, copies: &mut Copies) -> Result<Ref<'g>, NeedGc> {
    let mut elts = Vec::new();
    for r in info.routines(t) {
        let symbol = Found { ty: t, routine: r, dll: info };
        let (o, reg) = symbol_object(cx, None, r.fun, &symbol, Some(copies.next(symbol)))?;
        copies.made.extend(reg.map(|r| r.sexp()));
        elts.push(o);
    }
    let class = str1(cx, b"NativeRoutineList")?;
    list_with(cx, &elts, &[(RSym::Class, class)])
}

/// The copies a build hands to "RegisteredNativeSymbol" references, and the
/// references made in the build's last (successful) run. A run that needs a
/// collection is retried from the start: its copies are reused in order.
#[derive(Default)]
struct Copies {
    all: Vec<*mut Found>,
    used: usize,
    made: Vec<Sexp>,
}

impl Copies {
    fn start(&mut self) {
        self.used = 0;
        self.made.clear();
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
    /// Register each made reference's finalizer, and free the copies no
    /// reference took.
    fn finish(mut self, cx: &mut Cx, made: &[Root<'_>]) -> RResult<()> {
        for p in self.all.drain(self.used..) {
            // SAFETY: ours, never handed out.
            drop(unsafe { Box::from_raw(p) });
        }
        // the rest belong to the references now: their finalizers free them
        // (one whose registration fails below is leaked, never freed twice)
        self.all.clear();
        for &r in made {
            crate::memory::register_c_finalizer(cx, r, free_registered_symbol as *const c_void)?;
        }
        Ok(())
    }
}

impl Drop for Copies {
    fn drop(&mut self) {
        // a build that failed: no reference holds a copy
        for p in self.all.drain(..) {
            // SAFETY: ours; `finish` forgot the ones references took.
            drop(unsafe { Box::from_raw(p) });
        }
    }
}

/// Build `f`'s result into `out`, then register the finalizers of the
/// "RegisteredNativeSymbol" references it made.
fn build_with_copies(
    cx: &mut Cx,
    out: Out<'_>,
    mut f: impl for<'g> FnMut(&'g Cx, &mut Copies) -> Result<Ref<'g>, NeedGc>,
) -> RResult<()> {
    let mut copies = Copies::default();
    cx.scope(|cx, s| {
        let res = s.out(cx);
        let r = res.root();
        cx.build(res, |cx| {
            copies.start();
            f(cx, &mut copies)
        })?;
        // root the references while finalizers are registered
        let made: Vec<Sexp> = std::mem::take(&mut copies.made);
        let roots: Vec<Root<'_>> = made
            .iter()
            .map(|&x| {
                // SAFETY: an object of the result just built, rooted by `r`.
                let x = unsafe { Ref::from_sexp(x, cx) };
                s.root(cx, x)
            })
            .collect();
        std::mem::take(&mut copies).finish(cx, &roots)?;
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// `.Internal(getLoadedDLLs())`: a list of the loaded objects' "DLLInfo",
/// named by their names, of class "DLLInfoList".
// R: Rdynload.c:do_getDllTable, R_getDllTable
fn do_get_dll_table(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    robigo_rt::prims::check_arity(cx, call, args.len(), "getLoadedDLLs", 0, true)?;
    cx.build(out, |cx| {
        let list = dlls(cx).list.borrow();
        let elts = list.iter().map(|i| dll_info(cx, i)).collect::<Result<Vec<_>, _>>()?;
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
    build_with_copies(cx, out, |cx, copies| {
        // SAFETY: a record of the table, never freed.
        let info = unsafe { &*info };
        let elts = [
            routine_list(cx, info, SymType::C, copies)?,
            routine_list(cx, info, SymType::Call, copies)?,
            routine_list(cx, info, SymType::Fortran, copies)?,
            routine_list(cx, info, SymType::External, copies)?,
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
        // SAFETY: a record of the table, never freed.
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
    build_with_copies(cx, out, |cx, copies| {
        let copy = (with_reg && !symbol.routine.is_null() && !symbol.dll.is_null()).then(|| copies.next(symbol));
        let (o, reg) = symbol_object(cx, Some(sname.bind(cx)), f, &symbol, copy)?;
        copies.made.extend(reg.map(|r| r.sexp()));
        Ok(o)
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
                        list.push(Box::new(DllInfo {
                            name,
                            path: path.clone(),
                            handle: h,
                            dynamic_lookup: true,
                            force_symbols: false,
                            c: Vec::new(),
                            call: Vec::new(),
                            fortran: Vec::new(),
                            external: Vec::new(),
                            eref: Default::default(),
                        }));
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
            let l = dlls(cx).list.borrow();
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
    cx.build(out, |cx| {
        let list = dlls(cx).list.borrow();
        dll_info(cx, list.last().unwrap())
    })
}
