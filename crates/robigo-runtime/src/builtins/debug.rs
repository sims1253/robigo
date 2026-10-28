//! Debugging and memory-tracing primitives (GNU R's debug.c).

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
    // Duplication reporting needs object-header state and allocator hooks.
    Err(Flow::error_call(call.clone(), "robigo: memory tracing is not implemented yet"))
}

// R: debug.c:do_untracemem. No object can have a memory trace flag yet.
fn do_untracemem(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "untracemem", call)?;
    check_x(&args, call)?;
    if matches!(args[0].value, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
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
    if !previous.is_null() {
        if !matches!(&previous, Value::Str(s) if s.data.len() == 1) {
            return Err(Flow::error_call(call.clone(), "invalid 'previous' argument"));
        }
        return Err(Flow::error_call(call.clone(), "robigo: memory tracing is not implemented yet"));
    }
    i.visible = false;
    Ok(Value::Null)
}
