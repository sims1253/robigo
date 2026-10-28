//! Rdynload.c: loading shared objects, routine registration, C callables.
//! The host keeps its own DLL table for the R-level objects (`DLLInfo`,
//! `NativeSymbolInfo`); it opens and queries libraries through the
//! `robigo_dll_*` entry points here.

use crate::errors::error_str_owned;
use robigo_core::Sexp;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};

pub type DlFunc = *const c_void;

#[repr(C)]
pub struct R_CMethodDef {
    pub name: *const c_char,
    pub fun: DlFunc,
    pub num_args: c_int,
    pub types: *const c_void,
}

#[repr(C)]
pub struct R_CallMethodDef {
    pub name: *const c_char,
    pub fun: DlFunc,
    pub num_args: c_int,
}

/// Interfaces, in R's NativeSymbolType order.
pub const ANY_SYM: c_int = 0;
pub const C_SYM: c_int = 1;
pub const CALL_SYM: c_int = 2;
pub const FORTRAN_SYM: c_int = 3;
pub const EXTERNAL_SYM: c_int = 4;

pub struct Routine {
    pub name: CString,
    pub fun: DlFunc,
    pub nargs: c_int,
}

/// GNU R's DllInfo (opaque to packages).
pub struct DllInfo {
    pub path: CString,
    pub name: CString,
    pub handle: *mut c_void,
    pub use_dynamic_lookup: bool,
    pub force_symbols: bool,
    /// registered routines, by interface (C, Call, Fortran, External)
    pub routines: [Vec<Routine>; 4],
}

/// R_RegisteredNativeSymbol (Rdynpriv.h)
#[repr(C)]
pub struct RegisteredNativeSymbol {
    pub type_: c_int,
    pub symbol: *mut c_void,
    pub dll: *mut DllInfo,
}

static mut DLLS: Vec<*mut DllInfo> = Vec::new();
static mut CCALLABLES: Option<HashMap<(Vec<u8>, Vec<u8>), DlFunc>> = None;

pub unsafe fn mark_roots(_c: &mut robigo_core::Core) {}

unsafe fn add_routines<T>(
    info: &mut DllInfo,
    k: usize,
    defs: *const T,
    get: impl Fn(&T) -> (*const c_char, DlFunc, c_int),
) {
    if defs.is_null() {
        return;
    }
    let mut v = Vec::new();
    let mut i = 0;
    loop {
        let (name, fun, nargs) = get(&*defs.add(i));
        if name.is_null() {
            break;
        }
        v.push(Routine { name: CStr::from_ptr(name).into(), fun, nargs });
        i += 1;
    }
    info.routines[k] = v;
}

// R: Rdynload.c:R_registerRoutines
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_registerRoutines(
    info: *mut DllInfo,
    c: *const R_CMethodDef,
    call: *const R_CallMethodDef,
    fortran: *const R_CMethodDef,
    external: *const R_CallMethodDef,
) -> c_int {
    if info.is_null() {
        crate::errors::error_str("R_RegisterRoutines called with invalid DllInfo object.")
    }
    let info = &mut *info;
    info.use_dynamic_lookup = !info.handle.is_null();
    info.force_symbols = false;
    add_routines(info, 0, c, |d| (d.name, d.fun, d.num_args));
    add_routines(info, 1, call, |d| (d.name, d.fun, d.num_args));
    add_routines(info, 2, fortran, |d| (d.name, d.fun, d.num_args));
    add_routines(info, 3, external, |d| (d.name, d.fun, d.num_args));
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_useDynamicSymbols(info: *mut DllInfo, value: c_int) -> c_int {
    let old = (*info).use_dynamic_lookup;
    (*info).use_dynamic_lookup = value != 0;
    old as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_forceSymbols(info: *mut DllInfo, value: c_int) -> c_int {
    let old = (*info).force_symbols;
    (*info).force_symbols = value != 0;
    old as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getDllInfo(path: *const c_char) -> *mut DllInfo {
    let p = CStr::from_ptr(path);
    DLLS.iter().copied().find(|&d| (*d).path.as_c_str() == p).unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_getEmbeddingDllInfo() -> *mut DllInfo {
    let d = R_getDllInfo(c"(embedding)".as_ptr());
    if !d.is_null() {
        return d;
    }
    let d = Box::into_raw(Box::new(DllInfo {
        path: c"(embedding)".into(),
        name: c"(embedding)".into(),
        handle: std::ptr::null_mut(),
        use_dynamic_lookup: false,
        force_symbols: false,
        routines: Default::default(),
    }));
    DLLS.push(d);
    d
}

/// The registered routine `name` for interface `iface` (ANY_SYM: any).
unsafe fn registered(info: &DllInfo, name: &CStr, iface: c_int) -> Option<(DlFunc, c_int, c_int)> {
    let order = [C_SYM, CALL_SYM, FORTRAN_SYM, EXTERNAL_SYM];
    for &k in &order {
        if iface != ANY_SYM && iface != k {
            continue;
        }
        if let Some(r) = info.routines[(k - 1) as usize].iter().find(|r| r.name.as_c_str() == name) {
            return Some((r.fun, r.nargs, k));
        }
    }
    None
}

// R: Rdynload.c:R_dlsym
unsafe fn dlsym_info(info: &DllInfo, name: &CStr, iface: c_int) -> Option<(DlFunc, c_int, c_int)> {
    if let Some(r) = registered(info, name, iface) {
        return Some(r);
    }
    if !info.use_dynamic_lookup || info.handle.is_null() {
        return None;
    }
    let mut buf = name.to_bytes().to_vec();
    if iface == FORTRAN_SYM {
        buf.push(b'_');
    }
    let c = CString::new(buf.clone()).unwrap();
    let f = libc::dlsym(info.handle, c.as_ptr());
    if !f.is_null() {
        return Some((f, -1, iface));
    }
    if iface == ANY_SYM {
        buf.push(b'_');
        let c = CString::new(buf).unwrap();
        let f = libc::dlsym(info.handle, c.as_ptr());
        if !f.is_null() {
            return Some((f, -1, iface));
        }
    }
    None
}

// R: Rdynload.c:R_FindSymbol
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_FindSymbol(
    name: *const c_char,
    pkg: *const c_char,
    symbol: *mut RegisteredNativeSymbol,
) -> DlFunc {
    let name = CStr::from_ptr(name);
    let pkg = CStr::from_ptr(pkg).to_bytes();
    let all = pkg.is_empty();
    let iface = if symbol.is_null() { ANY_SYM } else { (*symbol).type_ };
    for &d in DLLS.iter().rev() {
        let info = &*d;
        let mut doit = if all { 1 } else { 0 };
        if doit == 0 && info.name.as_bytes() == pkg {
            doit = 2;
        }
        if doit != 0 && info.force_symbols {
            doit = 0;
        }
        if doit != 0 {
            if let Some((f, _, _)) = dlsym_info(info, name, iface) {
                if !symbol.is_null() {
                    (*symbol).dll = d;
                }
                return f;
            }
        }
        if doit > 1 {
            return std::ptr::null();
        }
    }
    std::ptr::null()
}

// ---- C callables -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_RegisterCCallable(package: *const c_char, name: *const c_char, fptr: DlFunc) {
    let t = CCALLABLES.get_or_insert_with(HashMap::new);
    t.insert((CStr::from_ptr(package).to_bytes().to_vec(), CStr::from_ptr(name).to_bytes().to_vec()), fptr);
}

// R: Rdynload.c:R_GetCCallable
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_GetCCallable(package: *const c_char, name: *const c_char) -> DlFunc {
    let p = CStr::from_ptr(package).to_bytes().to_vec();
    let n = CStr::from_ptr(name).to_bytes().to_vec();
    if let Some(&f) = CCALLABLES.as_ref().and_then(|t| t.get(&(p.clone(), n.clone()))) {
        return f;
    }
    error_str_owned(format!(
        "function '{}' not provided by package '{}'",
        String::from_utf8_lossy(&n),
        String::from_utf8_lossy(&p)
    ))
}

// ---- entry points for the host ----------------------------------------------

/// Open a shared object and run its `R_init_<name>` (Rdynload.c AddDLL).
/// Returns the DllInfo, or NULL with the loader's message in `err`
/// (`errlen` bytes). `*token` is set to a flow token if `R_init_` raised
/// an R error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dll_open(
    path: *const c_char,
    name: *const c_char,
    local: c_int,
    now: c_int,
    err: *mut c_char,
    errlen: usize,
    token: *mut usize,
) -> *mut DllInfo {
    *token = 0;
    let p = CStr::from_ptr(path);
    if let Some(k) = DLLS.iter().position(|&d| (*d).path.as_c_str() == p) {
        let d = DLLS.remove(k);
        DLLS.push(d);
        return d;
    }
    let flags = (if now != 0 { libc::RTLD_NOW } else { libc::RTLD_LAZY })
        | (if local != 0 { libc::RTLD_LOCAL } else { libc::RTLD_GLOBAL });
    let handle = libc::dlopen(path, flags);
    if handle.is_null() {
        let e = CStr::from_ptr(libc::dlerror()).to_bytes();
        let n = e.len().min(errlen.saturating_sub(1));
        std::ptr::copy_nonoverlapping(e.as_ptr(), err as *mut u8, n);
        *err.add(n) = 0;
        return std::ptr::null_mut();
    }
    let info = Box::into_raw(Box::new(DllInfo {
        path: p.into(),
        name: CStr::from_ptr(name).into(),
        handle,
        use_dynamic_lookup: true,
        force_symbols: false,
        routines: Default::default(),
    }));
    DLLS.push(info);
    let init = |sym: &str| {
        let c = CString::new(sym).unwrap();
        libc::dlsym(handle, c.as_ptr())
    };
    let nm = CStr::from_ptr(name).to_string_lossy().into_owned();
    let mut f = init(&format!("R_init_{nm}"));
    if f.is_null() {
        f = init(&format!("R_init_{}", nm.replace('.', "_")));
    }
    if !f.is_null() {
        let f: unsafe extern "C" fn(*mut DllInfo) = std::mem::transmute(f);
        if let Some(t) = crate::errors::protect_run(|| f(info)) {
            *token = t;
        }
    }
    info
}

/// Unload (Rdynload.c DeleteDLL): runs `R_unload_<name>`, then dlclose.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dll_close(info: *mut DllInfo) {
    if let Some(k) = DLLS.iter().position(|&d| d == info) {
        DLLS.remove(k);
        let i = Box::from_raw(info);
        if !i.handle.is_null() {
            let nm = i.name.to_string_lossy().replace('.', "_");
            let c = CString::new(format!("R_unload_{nm}")).unwrap();
            let f = libc::dlsym(i.handle, c.as_ptr());
            if !f.is_null() {
                let f: unsafe extern "C" fn(*mut DllInfo) = std::mem::transmute(f);
                crate::errors::protect_run(|| f(info));
            }
            libc::dlclose(i.handle);
        }
    }
}

/// Look `name` up in a DLL for interface `iface` (0 any, 1 C, 2 Call,
/// 3 Fortran, 4 External): registered routines first, then dlsym if
/// dynamic lookup is on. Returns 1 if found.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dll_find(
    info: *mut DllInfo,
    name: *const c_char,
    iface: c_int,
    fun: *mut DlFunc,
    nargs: *mut c_int,
    found_iface: *mut c_int,
) -> c_int {
    match dlsym_info(&*info, CStr::from_ptr(name), iface) {
        Some((f, n, k)) => {
            *fun = f;
            *nargs = n;
            *found_iface = k;
            1
        }
        None => 0,
    }
}

/// The flags of a DLL: bit 0 useDynamicLookup, bit 1 forceSymbols.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dll_flags(info: *mut DllInfo) -> c_int {
    (*info).use_dynamic_lookup as c_int | ((*info).force_symbols as c_int) << 1
}

/// The number of routines registered for interface `iface` (1..4).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dll_routine_count(info: *mut DllInfo, iface: c_int) -> c_int {
    let info = &*info;
    info.routines[(iface - 1) as usize].len() as c_int
}

/// Routine `i` registered for interface `iface`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn robigo_dll_routine(
    info: *mut DllInfo,
    iface: c_int,
    i: c_int,
    name: *mut *const c_char,
    fun: *mut DlFunc,
    nargs: *mut c_int,
) {
    let info = &*info;
    let r = &info.routines[(iface - 1) as usize][i as usize];
    *name = r.name.as_ptr();
    *fun = r.fun;
    *nargs = r.nargs;
}

#[allow(dead_code)]
fn _unused(_: Sexp) {}
