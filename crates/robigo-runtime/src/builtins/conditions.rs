//! Condition-system internals used by base R's conditions.R
//! (GNU R's errors.c `do_*` functions).

use std::rc::Rc;

use super::{arg, internal, lgl_arg};
use crate::conditions::{HandlerEntry, HandlerNode, RestartNode, handler_stack_from_value, handler_stack_value};
use crate::env::Env;
use crate::error::{Flow, RError, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("stop", do_stop, 2),
    internal("warning", do_warning, 4).invisible(),
    internal(".addCondHands", do_add_cond_hands, 5).invisible(),
    internal(".addGlobHands", do_add_global_hands, 5).invisible(),
    internal(".resetCondHands", do_reset_cond_hands, 1).invisible(),
    internal(".signalCondition", do_signal_condition, 3),
    internal(".dfltStop", do_dflt_stop, 2),
    // R: names.c registers .dfltWarn with eval=11 (visible).
    internal(".dfltWarn", do_dflt_warn, 2),
    internal(".addRestart", do_add_restart, 1).invisible(),
    internal(".getRestart", do_get_restart, 1),
    internal(".invokeRestart", do_invoke_restart, 2),
    internal("geterrmessage", do_geterrmessage, 0),
    internal("seterrmessage", do_seterrmessage, 1).invisible(),
    internal("printDeferredWarnings", do_print_deferred_warnings, 0).invisible(),
    internal(".addTryHandlers", do_add_try_handlers, 0).invisible(),
];

/// The call of the function that called `stop()`/`warning()`
/// (R's findCall: the next function frame below the current one).
fn find_call(i: &Interp) -> Value {
    let Some(cur) = i.current_frame() else { return Value::Null };
    for k in (0..cur).rev() {
        if !i.frames[k].builtin {
            return i.frames[k].call.clone();
        }
    }
    Value::Null
}

fn message_of(v: &Value) -> String {
    match v {
        Value::Str(s) if !s.data.is_empty() => s.data[0].as_deref().unwrap_or("NA").to_string(),
        Value::Null => String::new(),
        v => crate::coerce::to_str(v).first().cloned().flatten().map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn do_stop(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let call_flag = lgl_arg(&arg(&args, 0), "call.").unwrap_or(true);
    let c_call = if call_flag { find_call(i) } else { Value::Null };
    let msg = message_of(&arg(&args, 1));
    Err(Flow::error_call(c_call, msg))
}

fn do_warning(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let call_flag = lgl_arg(&arg(&args, 0), "call.").unwrap_or(true);
    let immediate = lgl_arg(&arg(&args, 1), "immediate.").unwrap_or(false);
    let c_call = if call_flag { find_call(i) } else { Value::Null };
    let msg = message_of(&arg(&args, 3));
    if immediate {
        let old = i.get_option("warn");
        let w = i.option_int("warn", 0);
        if w <= 0 {
            i.set_option("warn", Value::real(1.0));
        }
        let r = i.warningcall(&c_call, msg.clone());
        i.set_option("warn", old.unwrap_or(Value::real(0.0)));
        r?;
    } else {
        i.warningcall(&c_call, msg.clone())?;
    }
    Ok(Value::str(&msg))
}

fn do_add_cond_hands(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let classes = arg(&args, 0);
    let handlers = arg(&args, 1);
    let parentenv = arg(&args, 2);
    let target = arg(&args, 3);
    let calling = lgl_arg(&arg(&args, 4), "calling")?;
    let old = handler_stack_value(&i.handlers);
    if classes.is_null() || handlers.is_null() {
        return Ok(old);
    }
    let (Value::Str(cls), Value::List(hs)) = (&classes, &handlers) else {
        return Err(Flow::error_call(call.clone(), "bad handler data"));
    };
    if cls.data.len() != hs.data.len() {
        return Err(Flow::error_call(call.clone(), "bad handler data"));
    }
    let target_env = match target {
        Value::Env(e) => Some(e),
        _ => None,
    };
    let mut stack = i.handlers.clone();
    for k in (0..cls.data.len()).rev() {
        let entry = HandlerEntry {
            class: cls.data[k].clone().unwrap_or_else(|| Rc::from("NA")),
            parent_env: parentenv.clone(),
            handler: hs.data[k].clone(),
            target: target_env.clone(),
            calling,
        };
        stack = Some(Rc::new(HandlerNode { entry: Rc::new(entry), next: stack }));
    }
    i.handlers = stack;
    // R stores the new stack in the calling closure's context, so that it
    // is restored when that frame exits; our contexts saved the old stack
    // on entry, which has the same effect.
    Ok(old)
}

// R: errors.c:do_addGlobHands. Replace the top-level handler stack and
// every saved context that would otherwise restore the old global stack.
fn do_add_global_hands(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let old = i.global_handlers.clone();
    let same = |stack: &crate::conditions::HandlerStack| match (stack, &old) {
        (None, None) => true,
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        _ => false,
    };
    if i.frames.iter().any(|f| !same(&f.handlers)) {
        return Err(Flow::error("should not be called with handlers on the stack"));
    }
    i.handlers = None;
    do_add_cond_hands(i, call, args, env)?;
    for frame in &mut i.frames {
        frame.handlers = i.handlers.clone();
    }
    i.global_handlers = i.handlers.clone();
    Ok(Value::Null)
}

fn do_reset_cond_hands(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    i.handlers = handler_stack_from_value(&arg(&args, 0));
    Ok(Value::Null)
}

fn do_signal_condition(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cond = arg(&args, 0);
    let ecall = arg(&args, 2);
    i.signal_condition(&cond, &ecall)?;
    Ok(Value::Null)
}

fn do_dflt_stop(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let msg = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error_call(call.clone(), "bad error message")),
    };
    let ecall = arg(&args, 1);
    Err(i.default_error(&ecall, &msg))
}

fn do_dflt_warn(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let msg = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error_call(call.clone(), "bad error message")),
    };
    let ecall = arg(&args, 1);
    i.warning_default(&ecall, &msg)?;
    Ok(Value::Null)
}

/// Build an R `restart` object.
pub fn make_restart(name: &str, exit: Value) -> Value {
    let mut v = Vector::new(vec![Value::str(name), exit, Value::Null, Value::str(""), Value::Null, Value::lgl(true)]);
    v.attrs = Some(Box::new(Attrs(vec![
        (s::NAMES, Value::strs_from(["name", "exit", "handler", "description", "test", "interactive"])),
        (s::CLASS, Value::str("restart")),
    ])));
    Value::List(Rc::new(v))
}

fn do_add_restart(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let r = arg(&args, 0);
    if !matches!(&r, Value::List(l) if l.data.len() >= 2) {
        return Err(Flow::error_call(call.clone(), "bad restart"));
    }
    i.restarts = Some(Rc::new(RestartNode { restart: r, next: i.restarts.clone() }));
    Ok(Value::Null)
}

fn do_get_restart(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut n = crate::coerce::as_integer(&arg(&args, 0));
    let mut cur = i.restarts.clone();
    while let Some(node) = cur {
        if n <= 1 {
            return Ok(node.restart.clone());
        }
        n -= 1;
        cur = node.next.clone();
    }
    if n == 1 {
        return Ok(make_restart("abort", Value::Null));
    }
    Ok(Value::Null)
}

fn do_invoke_restart(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let r = arg(&args, 0);
    let arglist = arg(&args, 1);
    let Value::List(l) = &r else { return Err(Flow::error_call(call.clone(), "bad restart")) };
    if l.data.len() < 2 {
        return Err(Flow::error_call(call.clone(), "bad restart"));
    }
    let exit = l.data[1].clone();
    match exit {
        Value::Null => {
            // "abort": jump to top level quietly.
            i.restarts = None;
            Err(Flow::Error(Box::new(RError {
                message: String::new(),
                call: Some(Value::Null),
                signaled: true,
                classes: vec![],
            })))
        }
        Value::Env(e) => {
            let mut cur = i.restarts.clone();
            while let Some(node) = cur {
                let matches = match &node.restart {
                    Value::List(rl) if rl.data.len() >= 2 => match &rl.data[1] {
                        Value::Env(x) => x.ptr_eq(&e),
                        _ => false,
                    },
                    _ => false,
                };
                cur = node.next.clone();
                if matches {
                    i.restarts = cur;
                    return Err(Flow::Return(arglist, e));
                }
            }
            Err(Flow::error_call(call.clone(), "restart not on stack"))
        }
        _ => Err(Flow::error_call(call.clone(), "restart not on stack")),
    }
}

fn do_geterrmessage(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::str(&i.errmsg))
}

fn do_seterrmessage(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    i.errmsg = message_of(&arg(&args, 0));
    Ok(Value::Null)
}

fn do_print_deferred_warnings(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    i.print_warnings();
    Ok(Value::Null)
}

fn do_add_try_handlers(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}
