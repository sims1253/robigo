//! The bridge driven from Rust with a real interpreter, for what
//! `tests/capi.rs`'s stub host cannot reach: conversions outside a context,
//! `gc()`, write-backs into runtime objects. The API state is bound to one
//! thread (docs/core.md §3, §10), so everything runs in one test; a case
//! that must abort runs in a child process.

use R as capi;
use capi::Sexp;
use capi::bridge::BRIDGE;
use capi::state::*;
use robigo_core::types::*;
use robigo_runtime::env::Env;
use robigo_runtime::error::{Flow, RResult};
use robigo_runtime::interp::Interp;
use robigo_runtime::native::{Interface, NativeBridge, Routine};
use robigo_runtime::value::{Arg, Value};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void};
use std::rc::Rc;

fn routine(interface: Interface, addr: usize) -> Routine {
    Routine { package: "test".into(), name: "test".into(), interface, nargs: -1, addr }
}

fn call(interp: &mut Interp, interface: Interface, addr: usize, args: Vec<Value>) -> RResult<Value> {
    let env = interp.global.clone();
    let call = Value::lang(Value::sym(".Call"), vec![]);
    let args = args.into_iter().map(Arg::pos).collect();
    BRIDGE.invoke(interp, &call, &routine(interface, addr), args, &env)
}

fn message(r: RResult<Value>) -> String {
    match r {
        Err(Flow::Error(e)) => e.message.clone(),
        Err(_) => "(a flow other than an error)".into(),
        Ok(_) => "(no error)".into(),
    }
}

// ---- ALTREP classes whose methods raise ---------------------------------

fn class() -> capi::altrep::ClassT {
    thread_local! {
        static CLASS: Cell<*mut u8> = const { Cell::new(std::ptr::null_mut()) };
    }
    if CLASS.get().is_null() {
        let c = unsafe {
            capi::altrep::R_make_altinteger_class(c"raising".as_ptr(), c"bridge_test".as_ptr(), std::ptr::null_mut())
        };
        unsafe {
            capi::altrep::R_set_altrep_Length_method(c, raising_length as *const () as usize);
        }
        CLASS.set(c.ptr.0);
    }
    capi::altrep::ClassT { ptr: Sexp(CLASS.get()) }
}

unsafe extern "C" fn raising_length(_: Sexp) -> isize {
    unsafe { capi::errors::Rf_error(c"Length raised %d".as_ptr(), 7) }
}

unsafe extern "C" fn altrep_result() -> Sexp {
    unsafe { capi::altrep::R_new_altrep(class(), R_NilValue, R_NilValue) }
}

/// The ALTREP object inside a list: only the conversion of the element
/// reaches its method.
unsafe extern "C" fn altrep_in_list() -> Sexp {
    unsafe {
        let l = capi::memory::Rf_protect(capi::memory::Rf_allocVector(VECSXP as u32, 1));
        capi::memory::SET_VECTOR_ELT(l, 0, altrep_result());
        capi::memory::Rf_unprotect(1);
        l
    }
}

/// C stores an object in an argument: the write-back converts it.
unsafe extern "C" fn store_altrep(x: Sexp) -> Sexp {
    unsafe {
        capi::memory::SET_VECTOR_ELT(x, 0, altrep_result());
        R_NilValue
    }
}

unsafe extern "C" fn nothing() -> Sexp {
    unsafe { R_NilValue }
}

// ---- finalizers -----------------------------------------------------------

thread_local! {
    /// `Core::scan_top` when the last C finalizer ran (usize::MAX: none ran).
    static SCAN_TOP: Cell<usize> = const { Cell::new(usize::MAX) };
}

unsafe extern "C" fn note_scan_top(_: Sexp) {
    unsafe {
        let top = core().set_scan_top(0);
        core().set_scan_top(top);
        SCAN_TOP.set(top);
    }
}

unsafe extern "C" fn finalized_pointer() -> Sexp {
    unsafe {
        let p = capi::memory::Rf_protect(capi::extptr::R_MakeExternalPtr(std::ptr::null_mut(), R_NilValue, R_NilValue));
        capi::extptr::R_RegisterCFinalizer(p, note_scan_top);
        capi::memory::Rf_unprotect(1);
        R_NilValue
    }
}

// ---- the tests ------------------------------------------------------------

/// A protect stack that is full raises GNU R's error, also where no
/// context is active: the jump would abort.
fn protect_overflow(i: &mut Interp) {
    let list = Value::list(vec![Value::int(1)]);
    let top = unsafe { R_PPStackTop };
    for depth in [2, 5] {
        unsafe {
            let size = R_PPStackSize;
            for k in top..size {
                *R_PPStack.add(k as usize) = R_NilValue;
            }
            R_PPStackTop = size - depth;
        }
        let m = message(call(i, Interface::Call, nothing as *const () as usize, vec![list.clone()]));
        unsafe { R_PPStackTop = top };
        assert_eq!(m, "protect(): protection stack overflow", "{depth} slots free");
    }
    assert_eq!(capi::unwind_depth(), 0);
}

fn raising_altrep_conversions(i: &mut Interp) {
    let f = altrep_result as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, f, vec![])), "Length raised 7");
    let f = altrep_in_list as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, f, vec![])), "Length raised 7");
    let f = store_altrep as *const () as usize;
    let list = Value::list(vec![Value::int(1)]);
    assert_eq!(message(call(i, Interface::Call, f, vec![list])), "Length raised 7");
    assert_eq!(unsafe { R_PPStackTop }, 0, "the protect stack is restored");
    assert_eq!(capi::unwind_depth(), 0);
}

/// A finalizer is C code like any other: gc() must not leave the stack
/// scan off while it runs, or a collection inside it misses its locals.
fn finalizers_run_with_the_stack_scanned(i: &mut Interp) {
    let f = finalized_pointer as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, f, vec![])), "(no error)");
    BRIDGE.gc(i);
    let top = SCAN_TOP.get();
    assert_ne!(top, usize::MAX, "the finalizer ran");
    assert_ne!(top, 0, "the stack was scanned while it ran");
}

#[test]
fn the_bridge() {
    capi::bridge::install();
    let mut interp = Interp::new();
    let i = &mut interp;
    // the first native call initializes libR
    assert_eq!(message(call(i, Interface::Call, nothing as *const () as usize, vec![])), "(no error)");
    if std::env::var_os("ROBIGO_GCTORTURE").is_some() {
        unsafe { core().set_torture(true) };
    }
    finalizers_run_with_the_stack_scanned(i);
    protect_overflow(i);
    raising_altrep_conversions(i);
}

#[allow(dead_code)]
fn _unused(_: &Env, _: *const c_char, _: c_int, _: *mut c_void, _: Rc<()>) {}
