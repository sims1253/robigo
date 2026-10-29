//! The S4 primitives and internals of GNU R's objects.c and attrib.c:
//! `standardGeneric`, `.isMethodsDispatchOn`, `setS4Object`, `@<-`,
//! `.cache_class`, `.class2`. The machinery behind them is in `crate::s4`.

use super::{arg, check_arity, internal, prim, special};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::s4::data_class;
use crate::sym::Sym;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("standardGeneric", do_standard_generic, -1).custom_vis(),
    prim(".isMethodsDispatchOn", do_s4on, -1),
    internal("setS4Object", do_set_s4_object, 3),
    special("@<-", do_slot_gets, 3),
    prim(".cache_class", do_cache_class, 2),
    prim(".class2", do_class2, 1),
    // methods' trace() machinery (`.doTrace` checks tracingState()).
    internal("traceOnOff", |i, c, a, e| do_trace_on_off(i, c, a, e, true), 1),
    internal("debugOnOff", |i, c, a, e| do_trace_on_off(i, c, a, e, false), 1),
];

thread_local! {
    /// debug.c's `tracing_state` and `debugging_state`.
    static TRACE_DEBUG_STATE: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((true, true)) };
}

// R: debug.c:do_traceOnOff
fn do_trace_on_off(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env, trace: bool) -> RResult<Value> {
    let (t, d) = TRACE_DEBUG_STATE.with(|s| s.get());
    let prev = if trace { t } else { d };
    let on_off = arg(&args, 0);
    if !on_off.is_empty() {
        match crate::coerce::as_logical(&on_off) {
            v @ (TRUE | FALSE) => {
                let v = v == TRUE;
                TRACE_DEBUG_STATE.with(|s| s.set(if trace { (v, d) } else { (t, v) }));
            }
            _ => {
                return Err(Flow::error(format!(
                    "Value for '{}' must be TRUE or FALSE",
                    if trace { "tracingState" } else { "debuggingState" }
                )));
            }
        }
    }
    Ok(Value::lgl(prev))
}

// R: objects.c:do_standardGeneric
fn do_standard_generic(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    i.visible = true;
    i.standard_generic(call, args, env)
}

/// `.isMethodsDispatchOn(onOff)`.
// R: objects.c:do_S4on, R_isMethodsDispatchOn
fn do_s4on(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let on = i.methods_dispatch_on();
    let Some(a) = args.first() else { return Ok(Value::lgl(on)) };
    if !a.value.is_empty() {
        let l = crate::coerce::as_logical(&a.value);
        if l == NA_LOGICAL {
            return Err(Flow::error("'onOff' must be TRUE or FALSE"));
        } else if l == FALSE {
            let g = i.global.clone();
            i.set_standard_generic(crate::s4::StdGeneric::Unset, Some(g));
        } else if !on {
            i.warning("R_isMethodsDispatchOn(TRUE) called -- may not work correctly")?;
            let ns = i.methods_namespace();
            i.eval(&Value::call_sym("initMethodDispatch", vec![]), &ns)?;
        }
    }
    Ok(Value::lgl(on))
}

// R: objects.c:do_setS4Object
fn do_set_s4_object(i: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let flag_arg = arg(&args, 1);
    let flag = crate::builtins::character::as_bool2(&flag_arg, call)?;
    let complete = crate::coerce::as_integer(&arg(&args, 2));
    if flag_arg.len() != 1 || crate::coerce::as_logical(&flag_arg) == NA_LOGICAL {
        return Err(Flow::error("invalid 'flag' argument"));
    }
    if complete == NA_INTEGER {
        return Err(Flow::error("invalid 'complete' argument"));
    }
    let object = std::mem::replace(&mut args[0].value, Value::Null);
    if flag == object.is_s4() {
        return Ok(object);
    }
    i.as_s4(object, flag, complete)
}

/// `obj@name <- value`.
// R: attrib.c:do_attrgets (PRIMVAL 1)
fn do_slot_gets(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 3, "@<-", call)?;
    let nlist = match &args[1].value {
        p @ Value::Promise(_) => i.eval(p, env)?,
        v => v.clone(),
    };
    let input = match crate::builtins::subset::unwrap_quote(nlist) {
        Value::Sym(s) => Value::str(&s.name()),
        Value::Str(v) => {
            if v.data.len() != 1 {
                return Err(Flow::error("invalid slot name length"));
            }
            Value::Str(v)
        }
        other => return Err(Flow::error(format!("invalid type '{}' for slot name", other.type_name()))),
    };
    args[1].value = input.clone();
    // DispatchOrEval: the object first, then the other arguments.
    let first = std::mem::replace(&mut args[0].value, Value::Null);
    let (obj, expr) = match first {
        Value::Promise(p) if p.value().is_some() => (p.value().unwrap(), p.expr.clone()),
        e @ (Value::Sym(_) | Value::Lang(_) | Value::Promise(_)) => (i.eval(&e, env)?, e),
        v => (v.clone(), v),
    };
    let value = match &args[2].value {
        v @ (Value::Sym(_) | Value::Lang(_) | Value::Promise(_)) => i.eval(v, env)?,
        v => v.clone(),
    };
    let evaluated =
        vec![Arg::new(args[0].tag, obj), Arg::new(args[1].tag, input.clone()), Arg::new(args[2].tag, value)];
    if evaluated[0].value.is_object() {
        let mut pargs = i.forced_promargs(call, &evaluated, env);
        pargs[0].value = Value::Promise(std::rc::Rc::new(Promise::forced(expr, evaluated[0].value.clone())));
        if let Some(ans) = i.dispatch_promised("@<-", &evaluated[0].value.clone(), call, pargs, env)? {
            return Ok(ans);
        }
    }
    let mut evaluated = evaluated;
    let value = std::mem::replace(&mut evaluated[2].value, Value::Null);
    let obj = std::mem::replace(&mut evaluated[0].value, Value::Null);
    check_slot_assign(i, &obj, &input, &value, env)?;
    i.do_slot_assign(obj, &input, value)
}

/// `check_slot_assign`: `methods:::checkAtAssignment(class(obj), name,
/// class(value))`.
// R: attrib.c:check_slot_assign
fn check_slot_assign(i: &mut Interp, obj: &Value, input: &Value, value: &Value, env: &Env) -> RResult<()> {
    let value_class = data_class(value, false);
    let obj_class = data_class(obj, false);
    let ns = i.methods_namespace();
    if !i.methods_dispatch_on() {
        i.eval(&Value::call_sym("initMethodDispatch", vec![]), &ns)?;
    }
    let check_at = i.find_fun(Sym::new("checkAtAssignment"), &ns)?;
    let e = Value::lang(check_at, vec![Arg::pos(obj_class), Arg::pos(input.clone()), Arg::pos(value_class)]);
    i.eval(&e, env)?;
    Ok(())
}

// R: attrib.c:R_do_data_class (PRIMVAL 1)
fn do_cache_class(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, ".cache_class", call)?;
    crate::builtins::character::check1arg(&args, call, "class")?;
    let klass = arg(&args, 0);
    let class = match &klass {
        Value::Str(v) if !v.data.is_empty() => v.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid class argument to internal .class_cache")),
    };
    Ok(i.cache_class(&class, arg(&args, 1)))
}

// R: attrib.c:R_do_data_class (PRIMVAL 2)
fn do_class2(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, ".class2", call)?;
    crate::builtins::character::check1arg(&args, call, "x")?;
    let c = i.data_class2(&arg(&args, 0))?;
    Ok(Value::strs(c.into_iter().map(Some).collect()))
}
