//! Debugging and memory-tracing primitives (GNU R's debug.c).

use std::rc::Rc;

use super::{check_arity, match_args, prim};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("tracemem", do_tracemem, 1),
    prim("untracemem", do_untracemem, 1).invisible(),
    prim("retracemem", do_retracemem, -1).custom_vis(),
];

// R: util.c:check1arg.
fn check_x(args: &[Arg], call: &Value) -> RResult<()> {
    if let Some(tag) = args.first().and_then(|a| a.tag) {
        if !"x".starts_with(&*tag.name()) {
            return Err(Flow::error_call(call.clone(), format!("supplied argument name '{}' does not match 'x'", tag)));
        }
    }
    Ok(())
}

/// debug.c's RTRACE bit: which objects `tracemem()` marked. Copy
/// reporting (`memtrace_report`) needs allocator hooks and prints raw
/// addresses no recording can match; the flag and the address strings
/// are what R-level code observes.
thread_local! {
    static TRACED: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The object's address, debug.c's `snprintf(buffer, 21, "<%p>", object)`.
fn object_ptr(v: &Value) -> usize {
    macro_rules! ptr {
        ($x:expr) => {
            Rc::as_ptr($x) as usize
        };
    }
    match v {
        // Symbols are interned: their name allocation has the same lifetime
        // and identity as the symbol. Keep NULL's zero identity separate.
        Value::Sym(s) => s.name().as_ptr() as usize,
        Value::Env(e) => ptr!(&e.0),
        Value::Lgl(x) => ptr!(x),
        Value::Int(x) => ptr!(x),
        Value::Real(x) => ptr!(x),
        Value::Cplx(x) => ptr!(x),
        Value::Str(x) => ptr!(x),
        Value::Raw(x) => ptr!(x),
        Value::List(x) => ptr!(x),
        Value::Expr(x) => ptr!(x),
        Value::Pairlist(x) => ptr!(x),
        Value::Lang(x) => ptr!(x),
        Value::Closure(x) => ptr!(x),
        Value::Promise(x) => ptr!(x),
        Value::Dots(x) => ptr!(x),
        Value::ExtPtr(x) => ptr!(x),
        Value::Obj(x) => ptr!(x),
        _ => 0,
    }
}

fn address_string(p: usize) -> Value {
    Value::str(&format!("<{:p}>", p as *const ()))
}

// R: debug.c:do_tracemem (R_MEMORY_PROFILING).
fn do_tracemem(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "tracemem", call)?;
    check_x(&args, call)?;
    match &args[0].value {
        Value::Closure(_) | Value::Builtin(_) => {
            return Err(Flow::error_call(call.clone(), "argument must not be a function"));
        }
        Value::Null => return Err(Flow::error_call(call.clone(), "cannot trace NULL")),
        Value::Env(_) | Value::Promise(_) => {
            return Err(Flow::error_call(call.clone(), "'tracemem' is not useful for promise and environment objects"));
        }
        Value::ExtPtr(_) => {
            return Err(Flow::error_call(
                call.clone(),
                "'tracemem' is not useful for weak reference or external pointer objects",
            ));
        }
        _ => {}
    }
    let p = object_ptr(&args[0].value);
    // R: debug.c:do_tracemem sets RTRACE unconditionally, even if set.
    TRACED.with(|t| t.borrow_mut().insert(p));
    Ok(address_string(p))
}

// R: debug.c:do_untracemem.
fn do_untracemem(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "untracemem", call)?;
    check_x(&args, call)?;
    if matches!(args[0].value, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
    TRACED.with(|t| t.borrow_mut().remove(&object_ptr(&args[0].value)));
    Ok(Value::Null)
}

// R: debug.c:do_retracemem (R_MEMORY_PROFILING).
fn do_retracemem(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut matched = match_args(args, &["x", "previous"], call)?;
    let object = matched.or_null(0);
    if matches!(object, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
    let previous = matched.or_null(1);
    if !previous.is_null() && !matches!(&previous, Value::Str(s) if s.data.len() == 1) {
        return Err(Flow::error_call(call.clone(), "invalid 'previous' argument"));
    }
    let p = object_ptr(&object);
    let traced = TRACED.with(|t| t.borrow().contains(&p));
    // A traced object returns its address visibly; otherwise NULL,
    // invisibly (debug.c sets R_Visible before returning).
    let ans = if traced {
        i.visible = true;
        address_string(p)
    } else {
        i.visible = false;
        Value::Null
    };
    if !previous.is_null() {
        // SET_RTRACE(object, 1); the "tracemem[<previous> -> new]" report
        // prints raw addresses and is left unimplemented.
        TRACED.with(|t| t.borrow_mut().insert(p));
    }
    Ok(ans)
}
