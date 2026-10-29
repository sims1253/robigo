//! Function introspection: `formals`, `body`, `args` (GNU R's builtin.c).
//! `match.call` lives in unique.rs, as in R's unique.c.

use std::rc::Rc;

use super::{arg, internal};
use crate::env::Env;
use crate::error::RResult;
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("formals", do_formals, 1),
    internal("body", do_body, 1),
    internal("args", do_args, 1),
    internal("bodyCode", do_body, 1),
];

fn do_formals(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(match arg(&args, 0) {
        Value::Closure(c) => Value::pairlist(c.formals.to_vec()),
        _ => Value::Null,
    })
}

fn do_body(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match arg(&args, 0) {
        Value::Closure(c) => Ok(c.body.clone()),
        v => {
            if !v.is_function() {
                i.warningcall(call, "argument is not a function")?;
            }
            Ok(Value::Null)
        }
    }
}

/// `args(f)`: a closure with the formals of `f` and a NULL body.
fn do_args(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let mut f = arg(&args, 0);
    if let Value::Str(s) = &f {
        if s.data.len() == 1 {
            if let Some(name) = &s.data[0] {
                f = i.find_fun(Sym::new(name), env)?;
            }
        }
    }
    match &f {
        Value::Closure(c) => Ok(Value::Closure(Rc::new(Closure {
            formals: c.formals.clone(),
            body: Value::Null,
            env: i.global.clone(),
            attrs: None,
            code: Default::default(),
            inline: Default::default(),
            defaults: Default::default(),
            jit: Default::default(),
        }))),
        Value::Builtin(b) => {
            // Primitives: look up their argument template in .ArgsEnv or
            // .GenericArgsEnv, as R does.
            for name in [".ArgsEnv", ".GenericArgsEnv"] {
                if let Some(Value::Env(e)) = i.base_binding(Sym::new(name)) {
                    if let Some(Value::Closure(t)) = e.get_local(Sym::new(b.name)) {
                        return Ok(Value::Closure(Rc::new(Closure {
                            formals: t.formals.clone(),
                            body: Value::Null,
                            env: i.global.clone(),
                            attrs: None,
                            code: Default::default(),
                            inline: Default::default(),
                            defaults: Default::default(),
                            jit: Default::default(),
                        })));
                    }
                }
            }
            Ok(Value::Null)
        }
        _ => Ok(Value::Null),
    }
}
