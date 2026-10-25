//! Function introspection: `formals`, `body`, `args`, `match.call`
//! (GNU R's builtin.c and unique.c).

use std::rc::Rc;

use super::{arg, internal};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("formals", do_formals, 1),
    internal("body", do_body, 1),
    internal("args", do_args, 1),
    internal("match.call", do_matchcall, 4),
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
        }))),
        Value::Builtin(b) => {
            // Primitives: look up their argument template in .ArgsEnv or
            // .GenericArgsEnv, as R does.
            for name in [".ArgsEnv", ".GenericArgsEnv"] {
                if let Some(Value::Env(e)) = i.base.get_local(Sym::new(name)) {
                    if let Some(Value::Closure(t)) = e.get_local(Sym::new(b.name)) {
                        return Ok(Value::Closure(Rc::new(Closure {
                            formals: t.formals.clone(),
                            body: Value::Null,
                            env: i.global.clone(),
                            attrs: None,
                        })));
                    }
                }
            }
            Ok(Value::Null)
        }
        _ => Ok(Value::Null),
    }
}

/// R's `subDots`: the `...` of `rho`, with symbol/call elements replaced
/// by `..1`, `..2`, ...
fn sub_dots(rho: &Env) -> RResult<Option<Vec<Arg>>> {
    let dots = match rho.lookup(s::DOTS) {
        None => return Err(Flow::error("... used in a situation where it does not exist")),
        Some((v, _)) => v,
    };
    if dots.is_missing_arg() {
        return Ok(None);
    }
    let Value::Dots(d) = dots else { return Err(Flow::error("... is not a pairlist")) };
    let out = d
        .iter()
        .enumerate()
        .map(|(k, a)| {
            let mut t = a.value.clone();
            while let Value::Promise(p) = t {
                t = p.expr.clone();
            }
            let v = if matches!(t, Value::Sym(_) | Value::Lang(_)) {
                Value::Sym(Sym::new(&format!("..{}", k + 1)))
            } else {
                t
            };
            Arg::new(a.tag, v)
        })
        .collect();
    Ok(Some(out))
}

fn do_matchcall(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut funcall = arg(&args, 1);
    if let Value::Expr(e) = &funcall {
        funcall = e.data.first().cloned().unwrap_or(Value::Null);
    }
    let Value::Lang(fl) = &funcall else { return Err(Flow::error("invalid 'call' argument")) };
    let Value::Closure(def) = arg(&args, 0) else { return Err(Flow::error("invalid 'definition' argument")) };
    let expdots = crate::coerce::as_logical(&arg(&args, 2));
    if expdots == NA_LOGICAL {
        return Err(Flow::error("invalid 'expand.dots' argument"));
    }
    let Value::Env(sysp) = arg(&args, 3) else { return Err(Flow::error("'envir' must be an environment")) };
    // Expand `...` in the actuals from sysp.
    let mut actuals: Vec<Arg> = Vec::new();
    let has_dots = fl.args.iter().any(|a| a.value.as_sym() == Some(s::DOTS));
    let expanded = if has_dots { sub_dots(&sysp)? } else { None };
    for a in fl.args.iter() {
        if a.value.as_sym() == Some(s::DOTS) {
            if let Some(e) = &expanded {
                actuals.extend(e.iter().cloned());
            }
        } else {
            actuals.push(a.clone());
        }
    }
    let matched = i.match_args(&def.formals, actuals, call)?;
    let mut rlist: Vec<Arg> = Vec::new();
    for (f, m) in def.formals.iter().zip(matched) {
        let tag = f.tag.unwrap();
        let Some(v) = m else { continue };
        if tag == s::DOTS {
            if let Value::Dots(d) = &v {
                if expdots != 0 {
                    rlist.extend(d.iter().cloned());
                } else {
                    rlist.push(Arg::new(Some(s::DOTS), Value::pairlist((**d).clone())));
                }
            }
            continue;
        }
        if v.is_missing_arg() {
            continue;
        }
        rlist.push(Arg::new(Some(tag), v));
    }
    Ok(Value::Lang(Rc::new(Lang { fun: fl.fun.clone(), args: rlist, attrs: None })))
}
