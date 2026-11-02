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
use robigo_runtime::error::{Flow, RResult};
use robigo_runtime::interp::Interp;
use robigo_runtime::native::{Interface, NativeBridge, Routine};
use robigo_runtime::value::{Arg, Value};
use std::cell::Cell;
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

// ---- writes that replace what the evaluator may be reading ----------------

unsafe extern "C" fn set_body(f: Sexp, body: Sexp) -> Sexp {
    unsafe {
        capi::memory::SET_BODY(f, body);
        R_NilValue
    }
}

unsafe extern "C" fn set_formals(f: Sexp, formals: Sexp) -> Sexp {
    unsafe {
        capi::memory::SET_FORMALS(f, formals);
        R_NilValue
    }
}

unsafe extern "C" fn shorten(x: Sexp) -> Sexp {
    unsafe {
        capi::memory::SETLENGTH(x, 1);
        R_NilValue
    }
}

unsafe extern "C" fn set_tag_attr(x: Sexp, value: Sexp) -> Sexp {
    unsafe {
        capi::attrib::Rf_setAttrib(x, capi::strings::Rf_install(c"tag".as_ptr()), value);
        R_NilValue
    }
}

unsafe extern "C" fn cut_spine(x: Sexp) -> Sexp {
    unsafe {
        capi::memory::SETCDR(x, R_NilValue);
        R_NilValue
    }
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

fn eval_text(i: &mut Interp, text: &str) -> Value {
    let exprs = robigo_runtime::builtins::parsing::parse_text(text, "<text>").ok().expect("parses");
    let global = i.global.clone();
    i.eval(&exprs[0], &global).ok().expect("evaluates")
}

/// What a write-back replaces stays alive until R is back at top level:
/// the frames walking it can be above the outermost native call
/// (`f <- function() h(f)` with `h` calling SET_BODY).
fn replaced_objects_outlive_the_evaluation(i: &mut Interp) {
    let global = i.global.clone();
    let top_level = |i: &mut Interp| {
        i.eval_toplevel(&Value::Null, &global, false);
    };

    // SET_BODY
    let f = eval_text(i, "function(x) x + 1");
    let Value::Closure(clo) = &f else { panic!("not a closure") };
    let Value::Lang(old_body) = &clo.body else { panic!("not a call") };
    let old_body = Rc::downgrade(old_body);
    let new_body = eval_text(i, "quote(x * 2)");
    let addr = set_body as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, addr, vec![f.clone(), new_body])), "(no error)");
    let Value::Lang(body) = &clo.body else { panic!("not a call") };
    assert_eq!(body.args.len(), 2, "the closure has its new body");
    assert!(old_body.upgrade().is_some(), "the old body outlives the native call");
    top_level(i);
    assert!(old_body.upgrade().is_none(), "the old body is released at top level");

    // SET_FORMALS: the first formals are shared with the `function` call
    let pairlist = |name: &str| {
        let y = Arg::new(Some(robigo_runtime::sym::Sym::new(name)), Value::missing_arg());
        Value::Pairlist(Rc::new(robigo_runtime::value::Pairlist { items: vec![y], attrs: None }))
    };
    let addr = set_formals as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, addr, vec![f.clone(), pairlist("y")])), "(no error)");
    let formals = Rc::downgrade(&clo.formals);
    assert_eq!(message(call(i, Interface::Call, addr, vec![f.clone(), pairlist("z")])), "(no error)");
    assert_eq!(&*clo.formals[0].tag.unwrap().name(), "z");
    assert!(formals.upgrade().is_some(), "the old formals outlive the native call");
    top_level(i);
    assert!(formals.upgrade().is_none(), "the old formals are released at top level");

    // SETLENGTH on a list: its buffer is replaced
    let (first, second) = (Value::reals(vec![1.0, 2.0]), Value::reals(vec![3.0, 4.0]));
    let Value::Real(rc) = &second else { unreachable!() };
    let element = Rc::downgrade(rc);
    let list = Value::list(vec![first, second]);
    let addr = shorten as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, addr, vec![list.clone()])), "(no error)");
    let Value::List(l) = &list else { unreachable!() };
    assert_eq!(l.data.len(), 1);
    assert!(element.upgrade().is_some(), "the old buffer outlives the native call");
    top_level(i);
    assert!(element.upgrade().is_none(), "the old buffer is released at top level");

    // attributes: the box is replaced
    let x = Value::reals(vec![1.0]);
    let addr = set_tag_attr as *const () as usize;
    let old = Value::reals(vec![2.0]);
    let Value::Real(rc) = &old else { unreachable!() };
    let old_attr = Rc::downgrade(rc);
    assert_eq!(message(call(i, Interface::Call, addr, vec![x.clone(), old])), "(no error)");
    assert_eq!(message(call(i, Interface::Call, addr, vec![x.clone(), Value::Null])), "(no error)");
    assert!(x.attrs().is_none_or(|a| a.0.is_empty()), "the attribute is gone");
    assert!(old_attr.upgrade().is_some(), "the old attributes outlive the native call");
    top_level(i);
    assert!(old_attr.upgrade().is_none(), "the old attributes are released at top level");

    // a pairlist whose spine got shorter: the runtime's write-back swaps it
    let second = Value::reals(vec![3.0, 4.0]);
    let Value::Real(rc) = &second else { unreachable!() };
    let element = Rc::downgrade(rc);
    let cells = vec![Arg::pos(Value::reals(vec![1.0])), Arg::pos(second)];
    let pairlist = Value::Pairlist(Rc::new(robigo_runtime::value::Pairlist { items: cells, attrs: None }));
    let addr = cut_spine as *const () as usize;
    assert_eq!(message(call(i, Interface::Call, addr, vec![pairlist.clone()])), "(no error)");
    let Value::Pairlist(p) = &pairlist else { unreachable!() };
    assert_eq!(p.items.len(), 1);
    assert!(element.upgrade().is_some(), "the old spine outlives the native call");
    top_level(i);
    assert!(element.upgrade().is_none(), "the old spine is released at top level");
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
    replaced_objects_outlive_the_evaluation(i);
    protect_overflow(i);
    raising_altrep_conversions(i);
}

const OTHER_THREAD_CHILD: &str = "ROBIGO_OTHER_THREAD_CHILD";

/// Child process: the API's exported data is process-wide, so a second
/// thread must not initialize it again.
#[test]
fn other_thread_child() {
    if std::env::var_os(OTHER_THREAD_CHILD).is_none() {
        return;
    }
    let nil = unsafe {
        init_state();
        R_NilValue
    };
    let _ = std::thread::spawn(|| unsafe { core() as *mut robigo_core::Core as usize }).join();
    assert!(nil == unsafe { R_NilValue }, "R_NilValue was overwritten");
    eprintln!("the other thread got through");
}

#[test]
fn the_api_stays_on_its_thread() {
    if std::env::var_os(OTHER_THREAD_CHILD).is_some() {
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "other_thread_child", "--nocapture"])
        .env(OTHER_THREAD_CHILD, "1")
        .output()
        .expect("run child test");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("robigo: the R heap was used from a thread other than the R thread"), "{err}");
    assert!(!out.status.success(), "{err}");
}
