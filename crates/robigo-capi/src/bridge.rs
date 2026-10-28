//! The bridge (docs/core.md §6): until G3 rebuilds the runtime on the core,
//! robigo-runtime's `Value`s and core `SEXP`s live side by side and are
//! converted at the `.Call`/`.External`/`.C` boundary. This module is the
//! runtime's [`NativeBridge`] and the host side of `libR.so`'s callback
//! table: evaluation, environments and conditions stay in the runtime.
//!
//! Limits (council item 11; G3 removes them): arguments are copied in and
//! atomic vectors written back after the call (in-place C writes become
//! visible, as in GNU R), but `SET_STRING_ELT`/`SET_VECTOR_ELT` on an
//! argument do not reach the caller; environments, closures, promises and
//! builtins are HOST proxies; core objects without a runtime counterpart
//! (external pointers, weak references, S4 objects) are wrapped in a
//! runtime `ExtPtr` that keeps them preserved; cycles across the two heaps
//! leak; strings are converted to UTF-8.

use crate::host::{Host, robigo_capi_init, robigo_host_proxy, robigo_set_global};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp, flags, gp};
use robigo_runtime::env::Env;
use robigo_runtime::error::{Flow, RResult};
use robigo_runtime::interp::Interp;
use robigo_runtime::native::{Interface, NativeBridge, Routine};
use robigo_runtime::sym::{Sym, s};
use robigo_runtime::value::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::rc::{Rc, Weak};

pub struct Bridge;
pub static BRIDGE: Bridge = Bridge;

/// Register the bridge with the runtime (the executable calls this first).
pub fn install() {
    robigo_runtime::native::set_bridge(&BRIDGE);
}

thread_local! {
    static INTERP: Cell<*mut Interp> = const { Cell::new(std::ptr::null_mut()) };
    static FLOWS: RefCell<HashMap<usize, Flow>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<usize> = const { Cell::new(1) };
    /// identity of a runtime object -> its proxy's handle
    static PROXY_OF: RefCell<HashMap<usize, (usize, Sexp)>> = RefCell::new(HashMap::new());
    /// proxy handle -> (identity, the runtime object)
    static PROXIED: RefCell<HashMap<usize, (usize, Value)>> = RefCell::new(HashMap::new());
    static NEXT_HANDLE: Cell<usize> = const { Cell::new(1) };
    /// handles of proxies the collector found dead (dropped at a safe point)
    static DEAD: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// core objects wrapped for the runtime, for identity
    static WRAPPED: RefCell<HashMap<usize, Weak<ExtPtr>>> = RefCell::new(HashMap::new());
    static READY: Cell<bool> = const { Cell::new(false) };
    /// handler stacks hidden by R_ToplevelExec
    static TOPLEVEL: RefCell<Vec<(Vec<robigo_runtime::interp::Context>, robigo_runtime::conditions::HandlerStack)>> =
        const { RefCell::new(Vec::new()) };
    /// Identity within a native call: a runtime vector converted twice is
    /// the same core object (findVar(x) == R_getVar(x)). Entries are
    /// preserved until the outermost native call returns.
    static SEEN: RefCell<HashMap<usize, (Sexp, Value)>> = RefCell::new(HashMap::new());
    static CALL_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Run `f` with the interpreter that is currently calling into C.
pub fn with_interp<T>(f: impl FnOnce(&mut Interp) -> T) -> T {
    let p = INTERP.with(|c| c.get());
    assert!(!p.is_null(), "robigo: the C API was called outside an R evaluation");
    f(unsafe { &mut *p })
}

/// Make `interp` current while `f` runs (re-entrant). The outermost
/// entry also bounds the C heap's conservative stack scan to the frames
/// below it: the runtime's own frames above hold core objects only
/// through precise roots (protect stack, preserved wrappers, proxies), so
/// stale words there cannot keep garbage alive (docs/core.md §3).
fn enter<T>(interp: &mut Interp, f: impl FnOnce(&mut Interp) -> T) -> T {
    let prev = INTERP.with(|c| c.replace(interp as *mut Interp));
    let marker = 0usize;
    let here = std::hint::black_box(&marker) as *const usize as usize;
    let outer = prev.is_null();
    let old = if outer { core().set_scan_top(here) } else { 0 };
    let r = f(interp);
    if outer {
        core().set_scan_top(old);
    }
    INTERP.with(|c| c.set(prev));
    r
}

fn store_flow(f: Flow) -> usize {
    let t = NEXT_TOKEN.with(|n| {
        let t = n.get();
        n.set(t + 1);
        t
    });
    FLOWS.with(|m| m.borrow_mut().insert(t, f));
    t
}

fn take_flow(t: usize) -> Flow {
    FLOWS
        .with(|m| m.borrow_mut().remove(&t))
        .unwrap_or_else(|| Flow::error("robigo: a non-local exit from C code was lost"))
}

/// A host callback: catch panics so they never unwind into C, and turn
/// them into R errors.
fn guarded(f: impl FnOnce() -> usize) -> usize {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(t) => t,
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            store_flow(Flow::error(format!("robigo: internal error in a C API callback: {msg}")))
        }
    }
}

// ---- proxies -------------------------------------------------------------

unsafe extern "C" fn release_proxy(handle: usize) {
    DEAD.with(|d| d.borrow_mut().push(handle));
}

fn drain_dead() {
    let dead = DEAD.with(|d| std::mem::take(&mut *d.borrow_mut()));
    for h in dead {
        if let Some((key, _v)) = PROXIED.with(|m| m.borrow_mut().remove(&h)) {
            PROXY_OF.with(|m| {
                let mut m = m.borrow_mut();
                if m.get(&key).map(|e| e.0) == Some(h) {
                    m.remove(&key);
                }
            });
        }
    }
}

fn identity(v: &Value) -> Option<(usize, u8)> {
    Some(match v {
        Value::Env(e) => (Rc::as_ptr(&e.0) as *const u8 as usize, ENVSXP),
        Value::Closure(c) => (Rc::as_ptr(c) as *const u8 as usize, CLOSXP),
        Value::Promise(p) => (Rc::as_ptr(p) as *const u8 as usize, PROMSXP),
        Value::Builtin(b) => (
            *b as *const Builtin as usize,
            if matches!(b.kind, BuiltinKind::Special) { SPECIALSXP } else { BUILTINSXP },
        ),
        Value::Dots(d) => (Rc::as_ptr(d) as *const u8 as usize, DOTSXP),
        Value::ExtPtr(p) => (Rc::as_ptr(p) as *const u8 as usize, EXTPTRSXP),
        _ => return None,
    })
}

unsafe fn proxy(v: &Value) -> Sexp {
    drain_dead();
    let (key, t) = identity(v).expect("proxy of a value without identity");
    if let Some((_, x)) = PROXY_OF.with(|m| m.borrow().get(&key).copied()) {
        return x;
    }
    let h = NEXT_HANDLE.with(|n| {
        let h = n.get();
        n.set(h + 1);
        h
    });
    let x = robigo_host_proxy(t as c_int, h, Some(release_proxy));
    PROXIED.with(|m| m.borrow_mut().insert(h, (key, v.clone())));
    PROXY_OF.with(|m| m.borrow_mut().insert(key, (h, x)));
    x
}

fn proxied(x: Sexp) -> Option<Value> {
    if !unsafe { x.has_flag(flags::HOST) } {
        return None;
    }
    let h = unsafe { robigo_core::Core::host_handle(x) };
    PROXIED.with(|m| m.borrow().get(&h).map(|e| e.1.clone()))
}

/// A core object kept alive while the runtime holds it.
struct CoreRef(Sexp);

impl Drop for CoreRef {
    fn drop(&mut self) {
        unsafe {
            if !crate::state::isolate().is_null() {
                core().release(self.0);
            }
        }
        WRAPPED.with(|m| {
            m.borrow_mut().remove(&self.0.addr());
        });
    }
}

fn core_ref_of(v: &Value) -> Option<Sexp> {
    match v {
        Value::ExtPtr(p) => p.payload.as_ref()?.downcast_ref::<CoreRef>().map(|c| c.0),
        _ => None,
    }
}

unsafe fn wrap_core(x: Sexp) -> Value {
    if let Some(p) = WRAPPED.with(|m| m.borrow().get(&x.addr()).and_then(|w| w.upgrade())) {
        return Value::ExtPtr(p);
    }
    core().preserve(x);
    let tag = if x.typ() == EXTPTRSXP { from_sexp(x.tag()) } else { Value::Null };
    let attrs = attrs_from(x);
    let p = Rc::new(ExtPtr { tag, attrs: RefCell::new(attrs), payload: Some(Rc::new(CoreRef(x))) });
    WRAPPED.with(|m| m.borrow_mut().insert(x.addr(), Rc::downgrade(&p)));
    Value::ExtPtr(p)
}

// ---- Value -> SEXP ---------------------------------------------------------

fn sym_sexp(s: Sym) -> Sexp {
    if s == s::EMPTY {
        return unsafe { R_MissingArg };
    }
    s.with_name(|n| core().install(n.as_bytes()))
}

unsafe fn char_sexp(s: &RStr) -> Sexp {
    match s {
        None => R_NaString,
        Some(t) => {
            let b = t.as_bytes();
            core().mk_char(b, if b.is_ascii() { 0 } else { gp::UTF8 })
        }
    }
}

unsafe fn atomic<T: Copy>(t: u8, data: &[T]) -> Sexp {
    let x = core().alloc_vector(t, data.len());
    std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, x.std_data(), std::mem::size_of_val(data));
    x
}

unsafe fn set_attrs(x: Sexp, attrs: Option<&Attrs>) {
    let Some(a) = attrs else { return };
    if a.0.is_empty() {
        return;
    }
    Rf_protect(x);
    let mut list = R_NilValue;
    for (k, v) in a.0.iter().rev() {
        Rf_protect(list);
        let val = to_sexp(v);
        Rf_unprotect(1);
        list = Rf_cons(val, list);
        SET_TAG(list, sym_sexp(*k));
    }
    SET_ATTRIB(x, list);
    if a.get(s::CLASS).is_some() {
        x.set_flag(flags::OBJECT, true);
    }
    Rf_unprotect(1);
}

unsafe fn args_list(args: &[Arg], t: u8) -> Sexp {
    let mut list = R_NilValue;
    for a in args.iter().rev() {
        Rf_protect(list);
        let v = to_sexp(&a.value);
        Rf_unprotect(1);
        list = core().alloc_node(t, v, list, R_NilValue);
        if let Some(tag) = a.tag {
            SET_TAG(list, sym_sexp(tag));
        }
    }
    list
}

fn vector_identity(v: &Value) -> Option<usize> {
    Some(match v {
        Value::Lgl(x) | Value::Int(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Real(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Cplx(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Str(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Raw(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::List(x) | Value::Expr(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Lang(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Pairlist(x) => Rc::as_ptr(x) as *const u8 as usize,
        _ => return None,
    })
}

/// Convert a runtime value to a core object (a copy for vectors; the
/// same copy while a native call runs).
pub unsafe fn to_sexp(v: &Value) -> Sexp {
    let key = if CALL_DEPTH.with(|d| d.get()) > 0 { vector_identity(v) } else { None };
    if let Some(k) = key {
        if let Some(x) = SEEN.with(|m| m.borrow().get(&k).map(|e| e.0)) {
            return x;
        }
    }
    let x = to_sexp_new(v);
    if let Some(k) = key {
        core().preserve(x);
        // the Value is kept too, so its address is not reused meanwhile
        SEEN.with(|m| m.borrow_mut().insert(k, (x, v.clone())));
    }
    x
}

unsafe fn to_sexp_new(v: &Value) -> Sexp {
    let x = match v {
        Value::Null => return R_NilValue,
        Value::Sym(s) => return sym_sexp(*s),
        Value::Lgl(x) => atomic(LGLSXP, &x.data),
        Value::Int(x) => atomic(INTSXP, &x.data),
        Value::Real(x) => atomic(REALSXP, &x.data),
        Value::Cplx(x) => atomic(CPLXSXP, &x.data),
        Value::Raw(x) => atomic(RAWSXP, &x.data),
        Value::Str(x) => {
            let r = Rf_protect(core().alloc_vector(STRSXP, x.data.len()));
            for (i, s) in x.data.iter().enumerate() {
                SET_STRING_ELT(r, i as isize, char_sexp(s));
            }
            Rf_unprotect(1);
            r
        }
        Value::List(x) | Value::Expr(x) => {
            let t = if matches!(v, Value::List(_)) { VECSXP } else { EXPRSXP };
            let r = Rf_protect(core().alloc_vector(t, x.data.len()));
            for (i, e) in x.data.iter().enumerate() {
                SET_VECTOR_ELT(r, i as isize, to_sexp(e));
            }
            Rf_unprotect(1);
            r
        }
        Value::Pairlist(p) => {
            let r = args_list(&p.items, LISTSXP);
            set_attrs(r, p.attrs.as_deref());
            return r;
        }
        Value::Lang(l) => {
            let rest = Rf_protect(args_list(&l.args, LISTSXP));
            let f = to_sexp(&l.fun);
            let r = core().alloc_node(LANGSXP, f, rest, R_NilValue);
            Rf_unprotect(1);
            set_attrs(r, l.attrs.as_deref());
            return r;
        }
        Value::ExtPtr(_) if core_ref_of(v).is_some() => return core_ref_of(v).unwrap(),
        // `...`: a DOTSXP chain, R_MissingArg when empty (as GNU R binds it)
        Value::Dots(d) => {
            if d.is_empty() {
                return R_MissingArg;
            }
            return args_list(d, DOTSXP);
        }
        Value::Env(_) | Value::Closure(_) | Value::Promise(_) | Value::Builtin(_) | Value::ExtPtr(_) => {
            return proxy(v);
        }
    };
    set_attrs(x, v.attrs());
    x
}

// ---- SEXP -> Value ---------------------------------------------------------

unsafe fn attrs_from(x: Sexp) -> AttrBox {
    if x.typ() == CHARSXP {
        return None;
    }
    let mut a = x.attrib_raw();
    if a == R_NilValue || a.is_null() {
        return None;
    }
    let mut out = Vec::new();
    while a != R_NilValue {
        let t = a.tag();
        if t.typ() == SYMSXP {
            out.push((sym_of(t), from_sexp(a.car())));
        }
        a = a.cdr();
    }
    if out.is_empty() { None } else { Some(Box::new(Attrs(out))) }
}

unsafe fn sym_of(x: Sexp) -> Sym {
    if x == R_MissingArg {
        return s::EMPTY;
    }
    Sym::new(&String::from_utf8_lossy(x.car().char_bytes()))
}

/// A CHARSXP as a runtime string (UTF-8; Latin-1 converted).
pub unsafe fn rstr_of(c: Sexp) -> RStr {
    if c == R_NaString {
        return None;
    }
    let b = c.char_bytes();
    if c.gp() & gp::LATIN1 != 0 {
        return Some(Rc::from(b.iter().map(|&ch| ch as char).collect::<String>().as_str()));
    }
    Some(Rc::from(String::from_utf8_lossy(b).as_ref()))
}

unsafe fn copy_data<T: Copy>(x: Sexp) -> Vec<T> {
    let n = crate::memory::xlength(x) as usize;
    if n == 0 {
        return Vec::new();
    }
    let p = DATAPTR_RO(x) as *const T;
    std::slice::from_raw_parts(p, n).to_vec()
}

unsafe fn items_of(mut p: Sexp) -> Vec<Arg> {
    let mut items = Vec::new();
    while p != R_NilValue && matches!(p.typ(), LISTSXP | LANGSXP | DOTSXP) {
        let tag = if p.tag().typ() == SYMSXP { Some(sym_of(p.tag())) } else { None };
        items.push(Arg::new(tag, from_sexp(p.car())));
        p = p.cdr();
    }
    items
}

/// Convert a core object to a runtime value.
pub unsafe fn from_sexp(x: Sexp) -> Value {
    if x.is_null() || x == R_NilValue {
        return Value::Null;
    }
    if let Some(v) = proxied(x) {
        return v;
    }
    let mut v = match x.typ() {
        SYMSXP => {
            if x == R_MissingArg {
                return Value::missing_arg();
            }
            return Value::Sym(sym_of(x));
        }
        LGLSXP => Value::Lgl(Rc::new(Vector::new(copy_data::<i32>(x)))),
        INTSXP => Value::Int(Rc::new(Vector::new(copy_data::<i32>(x)))),
        REALSXP => Value::Real(Rc::new(Vector::new(copy_data::<f64>(x)))),
        CPLXSXP => Value::Cplx(Rc::new(Vector::new(
            copy_data::<Rcomplex>(x).into_iter().map(|z| Complex::new(z.r, z.i)).collect(),
        ))),
        RAWSXP => Value::Raw(Rc::new(Vector::new(copy_data::<u8>(x)))),
        STRSXP => {
            let n = crate::memory::xlength(x);
            Value::Str(Rc::new(Vector::new((0..n).map(|i| rstr_of(STRING_ELT(x, i))).collect())))
        }
        VECSXP | EXPRSXP => {
            let n = crate::memory::xlength(x);
            let items = (0..n).map(|i| from_sexp(VECTOR_ELT(x, i))).collect();
            if x.typ() == VECSXP {
                Value::List(Rc::new(Vector::new(items)))
            } else {
                Value::Expr(Rc::new(Vector::new(items)))
            }
        }
        CHARSXP => Value::Str(Rc::new(Vector::new(vec![rstr_of(x)]))),
        LISTSXP | DOTSXP => {
            let items = items_of(x);
            return Value::Pairlist(Rc::new(Pairlist { items, attrs: attrs_from(x) }));
        }
        LANGSXP => {
            let fun = from_sexp(x.car());
            let args = items_of(x.cdr());
            return Value::Lang(Rc::new(Lang { fun, args, attrs: attrs_from(x) }));
        }
        _ => return wrap_core(x),
    };
    v.set_attrs(attrs_from(x));
    v
}

// ---- host callbacks ------------------------------------------------------------

unsafe fn env_of(x: Sexp) -> Result<Env, Flow> {
    match from_sexp(x) {
        Value::Env(e) => Ok(e),
        Value::Null => Err(Flow::error("use of NULL environment is defunct")),
        _ => Err(Flow::error("argument is not an environment")),
    }
}

unsafe extern "C" fn h_eval(expr: Sexp, env: Sexp, out: *mut Sexp) -> usize {
    guarded(|| unsafe {
        let e = from_sexp(expr);
        let rho = match env_of(env) {
            Ok(r) => r,
            Err(f) => return store_flow(f),
        };
        match with_interp(|i| i.eval(&e, &rho)) {
            Ok(v) => {
                *out = to_sexp(&v);
                0
            }
            Err(f) => store_flow(f),
        }
    })
}

unsafe extern "C" fn h_write(stream: c_int, buf: *const u8, len: usize) {
    let s = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(buf, len) }).into_owned();
    with_interp(|i| if stream == 0 { i.write_out(&s) } else { i.write_err(&s) });
}

unsafe extern "C" fn h_warning(call: Sexp, msg: *const c_char, _immediate: c_int) -> usize {
    guarded(|| unsafe {
        let m = CStr::from_ptr(msg).to_string_lossy().into_owned();
        let r = if call.is_null() {
            with_interp(|i| i.warning(m))
        } else {
            let c = from_sexp(call);
            with_interp(|i| i.warningcall(&c, m))
        };
        match r {
            Ok(()) => 0,
            Err(f) => store_flow(f),
        }
    })
}

/// `Rf_error`/`Rf_errorcall`: signal the condition now, as errors.c
/// does, so calling handlers run while the C frames are live and the
/// default handler's traceback includes them.
unsafe extern "C" fn h_error(call: Sexp, msg: *const c_char) -> usize {
    guarded(|| unsafe {
        let m = CStr::from_ptr(msg).to_string_lossy().into_owned();
        let f = if call.is_null() { Flow::error(m) } else { Flow::error_call(from_sexp(call), m) };
        let f = match f {
            Flow::Error(e) => with_interp(|i| i.signal_error(e)),
            other => other,
        };
        store_flow(f)
    })
}

/// R_ToplevelExec's error handling: outer handlers are hidden, the
/// default handler prints the message.
unsafe extern "C" fn h_report(t: usize) {
    let f = take_flow(t);
    if let Flow::Error(e) = f {
        if !e.signaled {
            with_interp(|i| {
                let saved = i.handlers.take();
                let _ = i.signal_error(e);
                i.handlers = saved;
            });
        }
    }
}

unsafe extern "C" fn h_discard(t: usize) {
    FLOWS.with(|m| {
        m.borrow_mut().remove(&t);
    });
}

unsafe fn sym_arg(x: Sexp) -> RResult<Sym> {
    match from_sexp(x) {
        Value::Sym(s) => Ok(s),
        Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => Ok(Sym::new(v.data[0].as_deref().unwrap())),
        _ => Err(Flow::error("invalid symbol")),
    }
}

/// The value bound to `sym` in this frame, as findVarInFrame sees it:
/// promises unforced, active bindings called.
fn binding(env: &Env, sym: Sym) -> RResult<Option<Value>> {
    let Some(flags) = env.binding_flags(sym) else { return Ok(None) };
    let v = env.get_local(sym).unwrap_or(Value::Null);
    if flags & robigo_runtime::env::BIND_ACTIVE != 0 {
        let g = with_interp(|i| i.global.clone());
        return with_interp(|i| i.call_value(&v, vec![], &g)).map(Some);
    }
    Ok(Some(v))
}

fn vtype_of(t: i32) -> Option<VType> {
    Some(match t as u8 {
        NILSXP => VType::Null,
        LGLSXP => VType::Lgl,
        INTSXP => VType::Int,
        REALSXP => VType::Real,
        CPLXSXP => VType::Cplx,
        STRSXP => VType::Str,
        VECSXP => VType::List,
        EXPRSXP => VType::Expr,
        RAWSXP => VType::Raw,
        _ => return None,
    })
}

unsafe fn int_arg(x: Sexp) -> i32 {
    *(x.std_data() as *const i32)
}

/// Host operations by name (`host::op`).
unsafe fn op(name: &str, a: &[Sexp]) -> RResult<Sexp> {
    Ok(match name {
        "envlength" => {
            let e = env_of(a[0])?;
            crate::list::Rf_ScalarInteger(e.len() as i32)
        }
        "getOption" => {
            let n = String::from_utf8_lossy(a[0].car().char_bytes()).into_owned();
            let v = with_interp(|i| i.options.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone()));
            to_sexp(&v.unwrap_or(Value::Null))
        }
        "asChar" => {
            let v = from_sexp(a[0]);
            let first = match robigo_runtime::coerce::coerce_quiet(&v, VType::Str) {
                Value::Str(s) => s.data.first().cloned().unwrap_or(None),
                _ => None,
            };
            char_sexp(&first)
        }
        "coerceVector" => {
            let t = int_arg(a[1]);
            let v = from_sexp(a[0]);
            match vtype_of(t) {
                Some(vt) => to_sexp(&with_interp(|i| i.coerce(&v, vt))?),
                None => {
                    return Err(Flow::error(format!(
                        "robigo: coerceVector to type {} is not implemented yet",
                        crate::util::type_name(t as u8)
                    )));
                }
            }
        }
        "asCharacterFactor" => to_sexp(&robigo_runtime::builtins::envir::as_character_factor(&from_sexp(a[0]))),
        "field" => field(a[0], int_arg(a[1]))?,
        "connectionNumber" => {
            crate::list::Rf_ScalarInteger(robigo_runtime::builtins::io::connection_number(&from_sexp(a[0]))?)
        }
        "writeConnection" => {
            let n = int_arg(a[0]);
            let bytes = std::slice::from_raw_parts(a[1].std_data(), crate::memory::xlength(a[1]) as usize).to_vec();
            with_interp(|i| robigo_runtime::builtins::io::con_write_bytes(i, n, &bytes))?;
            R_NilValue
        }
        "readConnection" => {
            let (n, size) = (int_arg(a[0]), int_arg(a[1]).max(0) as usize);
            let mut buf = vec![0u8; size];
            let got = with_interp(|i| robigo_runtime::builtins::io::con_read_bytes(i, n, &mut buf))?;
            buf.truncate(got);
            to_sexp(&Value::Raw(Rc::new(Vector::new(buf))))
        }
        "connGetc" => {
            let n = int_arg(a[0]);
            crate::list::Rf_ScalarInteger(with_interp(|i| robigo_runtime::builtins::io::conn_getc(i, n))?)
        }
        "serialize" => {
            let v = from_sexp(a[0]);
            let (fmt, version) = (int_arg(a[1]), int_arg(a[2]));
            let bytes = if fmt == 2 {
                robigo_runtime::serialize::serialize_binary(&v, version)
            } else {
                robigo_runtime::serialize::serialize_version(&v, version)
            };
            to_sexp(&Value::Raw(Rc::new(Vector::new(bytes))))
        }
        "getRNGstate" => {
            with_interp(robigo_runtime::builtins::random::get_rng_state)?;
            R_NilValue
        }
        "putRNGstate" => {
            with_interp(robigo_runtime::builtins::random::put_rng_state)?;
            R_NilValue
        }
        "newEnv" => {
            let parent = env_of(a[0])?;
            to_sexp(&Value::Env(Env::new(Some(parent))))
        }
        "defineVar" => {
            let (sym, val, env) = (sym_arg(a[0])?, from_sexp(a[1]), env_of(a[2])?);
            with_interp(|i| i.define(sym, val, &env))?;
            R_NilValue
        }
        "setVar" => {
            // R: envir.c:setVar — assign where bound, else in the global env
            let (sym, val, env) = (sym_arg(a[0])?, from_sexp(a[1]), env_of(a[2])?);
            let mut cur = Some(env);
            let mut target = None;
            while let Some(e) = cur {
                if e.has_local(sym) {
                    target = Some(e);
                    break;
                }
                cur = e.parent();
            }
            let target = target.unwrap_or_else(|| with_interp(|i| i.global.clone()));
            with_interp(|i| i.define(sym, val, &target))?;
            R_NilValue
        }
        "findVar" => {
            let (sym, mut env) = (sym_arg(a[0])?, Some(env_of(a[1])?));
            while let Some(e) = env {
                if let Some(v) = binding(&e, sym)? {
                    return Ok(to_sexp(&v));
                }
                env = e.parent();
            }
            R_UnboundValue
        }
        "findVarInFrame" => {
            let (env, sym) = (env_of(a[0])?, sym_arg(a[1])?);
            match binding(&env, sym)? {
                Some(v) => to_sexp(&v),
                None => R_UnboundValue,
            }
        }
        "existsVarInFrame" => crate::list::Rf_ScalarLogical(env_of(a[0])?.has_local(sym_arg(a[1])?) as c_int),
        "removeVar" => {
            let (sym, env) = (sym_arg(a[0])?, env_of(a[1])?);
            if env.0.locked.get() {
                return Err(Flow::error("cannot remove bindings from a locked environment"));
            }
            env.remove(sym);
            R_NilValue
        }
        "findFun" => {
            let (sym, env) = (sym_arg(a[0])?, env_of(a[1])?);
            to_sexp(&with_interp(|i| i.find_fun(sym, &env))?)
        }
        "getVar" => {
            // R: envir.c:R_getVarEx — forces promises
            let (sym, env, inherits) = (sym_arg(a[0])?, env_of(a[1])?, *LOGICAL(a[2]) == 1);
            let mut cur = Some(env);
            let mut found = None;
            while let Some(e) = cur {
                if let Some(v) = binding(&e, sym)? {
                    found = Some(v);
                    break;
                }
                cur = if inherits { e.parent() } else { None };
            }
            match found {
                Some(v) => to_sexp(&with_interp(|i| i.force(v))?),
                None if a[3] != R_UnboundValue => a[3],
                None => return Err(Flow::error(format!("object '{}' not found", sym.name()))),
            }
        }
        "parse" => {
            let n = crate::memory::xlength(a[0]);
            let lines: Vec<String> =
                (0..n).map(|k| String::from_utf8_lossy(STRING_ELT(a[0], k).char_bytes()).into_owned()).collect();
            let exprs = robigo_runtime::builtins::parsing::parse_text(&lines.join("\n"), "<text>")?;
            to_sexp(&Value::Expr(Rc::new(Vector::new(exprs))))
        }
        "printValue" => {
            let v = from_sexp(a[0]);
            with_interp(|i| {
                let g = i.global.clone();
                robigo_runtime::print::print_value(i, &v, &g)
            })?;
            R_NilValue
        }
        // a builtin of the host (.Internal or .Primitive) with evaluated
        // arguments: the C API functions that GNU R implements with the
        // same code as an R builtin
        "internal" => {
            let name = String::from_utf8_lossy(STRING_ELT(a[0], 0).char_bytes()).into_owned();
            let b = robigo_runtime::builtins::lookup_internal(&name)
                .ok_or_else(|| Flow::error(format!("robigo: no internal '{name}'")))?;
            let args: Vec<Arg> = a[1..].iter().map(|&x| Arg::pos(from_sexp(x))).collect();
            let call = Value::Null;
            let v = with_interp(|i| {
                let env = i.global.clone();
                i.invoke_builtin(b, &call, args, &env)
            })?;
            to_sexp(&v)
        }
        // R_ToplevelExec hides the handler stack of its caller
        // R_ToplevelExec/R_tryEval start a new top-level context: the
        // caller's frames (sys.calls, the call stop() reports) and
        // handlers are hidden until it returns
        "toplevelBegin" => {
            with_interp(|i| {
                let saved = (std::mem::take(&mut i.frames), i.handlers.take());
                TOPLEVEL.with(|t| t.borrow_mut().push(saved))
            });
            R_NilValue
        }
        "toplevelEnd" => {
            if let Some((frames, h)) = TOPLEVEL.with(|t| t.borrow_mut().pop()) {
                with_interp(|i| {
                    i.frames = frames;
                    i.handlers = h;
                });
            }
            R_NilValue
        }
        // R: errors.c:onintr with nothing to resume: signal an interrupt
        // condition, then jump to top level (non-interactive: halt)
        "onintr" => {
            let mut cond = Value::list(vec![]);
            cond.set_attr_raw(s::CLASS, Value::strs_from(["interrupt", "condition"]));
            with_interp(|i| i.signal_condition(&cond, &Value::Null))?;
            with_interp(|i| i.write_err("\n"));
            return Err(Flow::Error(Box::new(robigo_runtime::error::RError {
                message: String::new(),
                call: Some(Value::Null),
                signaled: true,
                classes: vec![],
            })));
        }
        _ => return Err(Flow::error(format!("robigo: C API host operation '{name}' is not implemented yet"))),
    })
}

/// CAR/CDR/TAG of a proxy: FORMALS/BODY/CLOENV, PRVALUE/PRCODE/PRENV,
/// FRAME/ENCLOS/HASHTAB.
unsafe fn field(x: Sexp, k: i32) -> RResult<Sexp> {
    let v = proxied(x).unwrap_or(Value::Null);
    Ok(match (&v, k) {
        (Value::Closure(c), 0) => {
            to_sexp(&Value::Pairlist(Rc::new(Pairlist { items: c.formals.to_vec(), attrs: None })))
        }
        (Value::Closure(c), 1) => to_sexp(&c.body),
        (Value::Closure(c), 2) => to_sexp(&Value::Env(c.env.clone())),
        (Value::Promise(p), 0) => match p.value.borrow().clone() {
            Some(v) => to_sexp(&v),
            None => R_UnboundValue,
        },
        (Value::Promise(p), 1) => to_sexp(&p.expr),
        (Value::Promise(p), 2) => match p.env.borrow().clone() {
            Some(e) => to_sexp(&Value::Env(e)),
            None => R_NilValue,
        },
        (Value::Env(e), 1) => match e.parent() {
            Some(p) => to_sexp(&Value::Env(p)),
            None => R_EmptyEnv,
        },
        (Value::Env(_), _) => R_NilValue,
        _ => R_NilValue,
    })
}

unsafe extern "C" fn h_op(name: *const c_char, args: *const Sexp, n: c_int, out: *mut Sexp) -> usize {
    guarded(|| unsafe {
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let a = std::slice::from_raw_parts(args, n.max(0) as usize);
        match op(&name, a) {
            Ok(x) => {
                *out = x;
                0
            }
            Err(f) => store_flow(f),
        }
    })
}

static HOST: Host = Host {
    eval: h_eval,
    write: h_write,
    warning: h_warning,
    op: h_op,
    error: h_error,
    report: h_report,
    discard: h_discard,
};

// ---- the stub libR.so (package objects built by GNU R ask for it) ---------------

static STUB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libR-stub.so"));

/// Load a soname-only `libR.so` so that objects linked with `-lR` (GNU R's
/// Makeconf) bind their API symbols to this executable's exports. The
/// loader matches `-lR` by the stub's SONAME, not its path, so the stub is
/// loaded from an anonymous memory file: a file in a shared temporary
/// directory could be replaced by another user before `dlopen` reads it.
fn load_stub() {
    // SAFETY: memfd_create with a NUL-terminated name; the fd is ours alone.
    let fd = unsafe { libc::memfd_create(c"libR.so".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return;
    }
    let mut written = 0;
    while written < STUB.len() {
        // SAFETY: writing the remaining bytes of a static slice to our fd.
        let n = unsafe { libc::write(fd, STUB[written..].as_ptr().cast(), STUB.len() - written) };
        if n <= 0 {
            // SAFETY: closing the fd we opened.
            unsafe { libc::close(fd) };
            return;
        }
        written += n as usize;
    }
    let path = CString::new(format!("/proc/self/fd/{}", fd)).unwrap();
    // SAFETY: dlopen of the stub we just wrote; the mapping outlives the fd.
    unsafe {
        libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        libc::close(fd);
    }
}

unsafe fn ensure_ready(interp: &mut Interp) {
    if READY.with(|r| r.get()) {
        return;
    }
    READY.with(|r| r.set(true));
    load_stub();
    robigo_capi_init(&HOST);
    let globals = [
        Value::Env(interp.global.clone()),
        Value::Env(interp.base.clone()),
        Value::Env(interp.empty.clone()),
        Value::Env(interp.base_ns.clone()),
        Value::Env(interp.ns_registry.clone()),
    ];
    for (k, g) in globals.iter().enumerate() {
        let x = proxy(g);
        core().preserve(x);
        robigo_set_global(k as c_int, x);
    }
}

// ---- calling native code ---------------------------------------------------------

fn iface_code(i: Option<Interface>) -> c_int {
    match i {
        None => 0,
        Some(Interface::C) => 1,
        Some(Interface::Call) => 2,
        Some(Interface::Fortran) => 3,
        Some(Interface::External) => 4,
    }
}

fn iface_of(k: c_int) -> Interface {
    match k {
        1 => Interface::C,
        3 => Interface::Fortran,
        4 => Interface::External,
        _ => Interface::Call,
    }
}

/// Copy back atomic arguments the routine wrote into (GNU R passes the
/// caller's own vectors, so in-place writes are visible to it).
unsafe fn write_back(v: &Value, x: Sexp) {
    fn back<T: Copy + PartialEq>(rc: &Rc<Vector<T>>, x: Sexp) {
        unsafe {
            let n = rc.data.len();
            if n == 0 || crate::memory::xlength(x) as usize != n {
                return;
            }
            let src = std::slice::from_raw_parts(x.std_data() as *const T, n);
            if src != rc.data.as_slice() {
                // R semantics: the routine mutated the caller's object
                let p = Rc::as_ptr(rc) as *mut Vector<T>;
                (*p).data.copy_from_slice(src);
            }
        }
    }
    match v {
        Value::Lgl(rc) | Value::Int(rc) => back(rc, x),
        Value::Real(rc) => back(rc, x),
        Value::Raw(rc) => back(rc, x),
        Value::Cplx(rc) => {
            let n = rc.data.len();
            if n > 0 && crate::memory::xlength(x) as usize == n {
                let src = std::slice::from_raw_parts(x.std_data() as *const Complex, n);
                if src != rc.data.as_slice() {
                    let p = Rc::as_ptr(rc) as *mut Vector<Complex>;
                    (*p).data.copy_from_slice(src);
                }
            }
        }
        _ => {}
    }
}

/// References to an argument's object besides the argument list itself.
fn sharing(v: &Value) -> u16 {
    let n = match v {
        Value::Lgl(x) | Value::Int(x) => Rc::strong_count(x),
        Value::Real(x) => Rc::strong_count(x),
        Value::Cplx(x) => Rc::strong_count(x),
        Value::Str(x) => Rc::strong_count(x),
        Value::Raw(x) => Rc::strong_count(x),
        Value::List(x) | Value::Expr(x) => Rc::strong_count(x),
        _ => return 0,
    };
    n.saturating_sub(1).min(u16::MAX as usize) as u16
}

unsafe fn result(t: usize, out: Sexp) -> RResult<Value> {
    if t != 0 {
        return Err(take_flow(t));
    }
    Ok(from_sexp(out))
}

unsafe fn stack_imbalance(interp: &mut Interp, name: &str, info: &crate::dotcode::CallInfo) {
    // R: eval.c:check_stack_balance
    if info.pp_before != info.pp_after {
        interp
            .write_err(&format!("Warning: stack imbalance in '{}', {} then {}\n", name, info.pp_before, info.pp_after));
    }
}

/// The foreign call runs in a builtin context, as GNU R gives PP_FOREIGN
/// primitives one (eval.c): tracebacks show it, `error()` skips it.
unsafe fn call_routine(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    CALL_DEPTH.with(|d| d.set(d.get() + 1));
    let res = call_routine_ctx(interp, call, r, args, env);
    if CALL_DEPTH.with(|d| {
        d.set(d.get() - 1);
        d.get() == 0
    }) {
        let seen = SEEN.with(|m| std::mem::take(&mut *m.borrow_mut()));
        for (_, (x, _)) in seen {
            core().release(x);
        }
    }
    res
}

unsafe fn call_routine_ctx(
    interp: &mut Interp,
    call: &Value,
    r: &Routine,
    args: Vec<Arg>,
    env: &Env,
) -> RResult<Value> {
    interp.frames.push(robigo_runtime::interp::Context {
        call: call.clone(),
        function: Value::Null,
        cloenv: env.clone(),
        sysparent: env.clone(),
        promargs: vec![],
        on_exit: vec![],
        handlers: interp.handlers.clone(),
        restarts: interp.restarts.clone(),
        generic: false,
        builtin: true,
    });
    let r = call_routine_in(interp, call, r, args, env);
    let r = match r {
        Err(Flow::Error(e)) if !e.signaled => Err(interp.signal_error(e)),
        other => other,
    };
    let ctx = interp.frames.pop().unwrap();
    interp.handlers = ctx.handlers;
    interp.restarts = ctx.restarts;
    r
}

unsafe fn call_routine_in(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    ensure_ready(interp);
    let base = R_PPStackTop;
    let res = match r.interface {
        Interface::Call => {
            let mut xs = Vec::with_capacity(args.len());
            for a in &args {
                // the caller's sharing: a value bound elsewhere (a
                // variable) is referenced, as in GNU R, so code following
                // the NAMED/REFCNT protocol duplicates before writing
                let refs = sharing(&a.value);
                let x = Rf_protect(to_sexp(&a.value));
                if refs > 0 && x.refcnt() < refs {
                    x.set_refcnt(refs);
                }
                xs.push(x);
            }
            let mut out = Sexp::NULL;
            let mut info = crate::dotcode::CallInfo { pp_before: 0, pp_after: 0 };
            let t = crate::dotcode::robigo_dotcall(
                r.addr as *const c_void,
                xs.as_ptr(),
                xs.len() as c_int,
                &mut out,
                &mut info,
            );
            for (a, &x) in args.iter().zip(&xs) {
                write_back(&a.value, x);
            }
            stack_imbalance(interp, ".Call", &info);
            result(t, out)
        }
        Interface::External => {
            // R: dotcode.c:do_External — the routine gets the whole
            // argument list, its .NAME first
            let mut all = vec![Arg::pos(Value::str(&r.name))];
            all.extend(args.iter().cloned());
            let list = Rf_protect(args_list(&all, LISTSXP));
            let mut out = Sexp::NULL;
            let mut info = crate::dotcode::CallInfo { pp_before: 0, pp_after: 0 };
            let t = crate::dotcode::robigo_dotexternal(r.addr as *const c_void, list, &mut out, &mut info);
            stack_imbalance(interp, ".External", &info);
            result(t, out)
        }
        Interface::C | Interface::Fortran => dot_c(interp, call, r, args, env),
    };
    R_PPStackTop = base;
    drain_dead();
    res
}

/// `.C`/`.Fortran` (dotcode.c do_dotCode): every argument is copied into a
/// C array, the routine works on the copies, and the result is the list
/// of (copied back) arguments.
unsafe fn dot_c(_interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    enum Buf {
        I(Vec<i32>),
        D(Vec<f64>),
        Z(Vec<Complex>),
        R(Vec<u8>),
        S(#[allow(dead_code)] Vec<CString>, Vec<*mut c_char>),
    }
    let mut bufs = Vec::with_capacity(args.len());
    for a in &args {
        bufs.push(match &a.value {
            Value::Lgl(v) | Value::Int(v) => Buf::I(v.data.clone()),
            Value::Real(v) => Buf::D(v.data.clone()),
            Value::Cplx(v) => Buf::Z(v.data.clone()),
            Value::Raw(v) => Buf::R(v.data.clone()),
            Value::Str(v) => {
                let cs: Vec<CString> =
                    v.data.iter().map(|s| CString::new(s.as_deref().unwrap_or("NA")).unwrap_or_default()).collect();
                let ps = cs.iter().map(|c| c.as_ptr() as *mut c_char).collect();
                Buf::S(cs, ps)
            }
            other => {
                return Err(Flow::error_call(
                    call.clone(),
                    format!("invalid mode ({}) to pass to C or Fortran (arg {})", other.type_name(), bufs.len() + 1),
                ));
            }
        });
    }
    let ptrs: Vec<*mut c_void> = bufs
        .iter_mut()
        .map(|b| match b {
            Buf::I(v) => v.as_mut_ptr() as *mut c_void,
            Buf::D(v) => v.as_mut_ptr() as *mut c_void,
            Buf::Z(v) => v.as_mut_ptr() as *mut c_void,
            Buf::R(v) => v.as_mut_ptr() as *mut c_void,
            Buf::S(_, p) => p.as_mut_ptr() as *mut c_void,
        })
        .collect();
    let t = crate::dotcode::robigo_dotc(r.addr as *const c_void, ptrs.as_ptr(), ptrs.len() as c_int);
    if t != 0 {
        return Err(take_flow(t));
    }
    let replaced: Vec<(usize, Value)> = bufs
        .into_iter()
        .enumerate()
        .map(|(k, b)| {
            let v = match (b, &args[k].value) {
                (Buf::I(d), Value::Lgl(_)) => Value::Lgl(Rc::new(Vector::new(d))),
                (Buf::I(d), _) => Value::Int(Rc::new(Vector::new(d))),
                (Buf::D(d), _) => Value::Real(Rc::new(Vector::new(d))),
                (Buf::Z(d), _) => Value::Cplx(Rc::new(Vector::new(d))),
                (Buf::R(d), _) => Value::Raw(Rc::new(Vector::new(d))),
                (Buf::S(_, p), _) => Value::Str(Rc::new(Vector::new(
                    p.iter().map(|&q| Some(Rc::from(CStr::from_ptr(q).to_string_lossy().as_ref()))).collect(),
                ))),
            };
            (k, v)
        })
        .collect();
    Ok(robigo_runtime::native::dotc_result(args, replaced))
}

impl NativeBridge for Bridge {
    fn load(&self, interp: &mut Interp, path: &str, name: &str, local: bool, now: bool) -> RResult<usize> {
        enter(interp, |interp| unsafe {
            ensure_ready(interp);
            let p = CString::new(path).unwrap_or_default();
            let n = CString::new(name).unwrap_or_default();
            let mut err = [0 as c_char; 2048];
            let mut token = 0usize;
            let info = crate::rdynload::robigo_dll_open(
                p.as_ptr(),
                n.as_ptr(),
                local as c_int,
                now as c_int,
                err.as_mut_ptr(),
                err.len(),
                &mut token,
            );
            if info.is_null() {
                let e = CStr::from_ptr(err.as_ptr()).to_string_lossy().into_owned();
                return Err(Flow::error(format!("unable to load shared object '{path}':\n  {e}")));
            }
            if token != 0 {
                return Err(take_flow(token));
            }
            Ok(info as usize)
        })
    }

    fn unload(&self, interp: &mut Interp, handle: usize) {
        enter(interp, |_| unsafe { crate::rdynload::robigo_dll_close(handle as *mut crate::rdynload::DllInfo) })
    }

    fn flags(&self, handle: usize) -> (bool, bool) {
        let f = unsafe { crate::rdynload::robigo_dll_flags(handle as *mut crate::rdynload::DllInfo) };
        (f & 1 != 0, f & 2 != 0)
    }

    fn routines(&self, handle: usize) -> Vec<(Interface, String, usize, i32)> {
        let info = handle as *mut crate::rdynload::DllInfo;
        let mut out = Vec::new();
        for k in 1..=4 {
            let n = unsafe { crate::rdynload::robigo_dll_routine_count(info, k) };
            for i in 0..n {
                let mut name: *const c_char = std::ptr::null();
                let mut fun: *const c_void = std::ptr::null();
                let mut nargs = 0;
                unsafe { crate::rdynload::robigo_dll_routine(info, k, i, &mut name, &mut fun, &mut nargs) };
                let nm = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
                out.push((iface_of(k), nm, fun as usize, nargs));
            }
        }
        out
    }

    fn lookup(&self, handle: usize, name: &str, interface: Option<Interface>) -> Option<(usize, i32, Interface)> {
        let n = CString::new(name).ok()?;
        let mut fun: *const c_void = std::ptr::null();
        let mut nargs = -1;
        let mut found = 0;
        let ok = unsafe {
            crate::rdynload::robigo_dll_find(
                handle as *mut crate::rdynload::DllInfo,
                n.as_ptr(),
                iface_code(interface),
                &mut fun,
                &mut nargs,
                &mut found,
            )
        };
        if ok == 0 {
            return None;
        }
        let iface = if found == 0 { interface.unwrap_or(Interface::Call) } else { iface_of(found) };
        Some((fun as usize, nargs, iface))
    }

    fn invoke(&self, interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &Env) -> RResult<Value> {
        enter(interp, |interp| unsafe { call_routine(interp, call, r, args, env) })
    }

    fn gc(&self, interp: &mut Interp) {
        if READY.with(|r| r.get()) {
            let outer = INTERP.with(|c| c.get().is_null());
            enter(interp, |_| unsafe {
                // gc() from R with no C code active: nothing on the
                // machine stack can hold a core object
                let old = if outer { core().set_scan_top(0) } else { 0 };
                crate::host::robigo_gc();
                if outer {
                    core().set_scan_top(old);
                }
            });
            drain_dead();
        }
    }
}
