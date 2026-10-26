//! Type predicates and conversions: `as.*`, `is.*`, `typeof`,
//! `as.vector`, `call`, `do.call` (GNU R's coerce.c).

use std::rc::Rc;

use super::{arg, check_arity, internal, prim, special};
use crate::coerce::*;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("as.character", do_as_character, -1),
    prim("as.integer", do_as_integer, -1),
    prim("as.double", do_as_double, -1),
    prim("as.numeric", do_as_double, -1),
    prim("as.complex", do_as_complex, -1),
    prim("as.logical", do_as_logical, -1),
    prim("as.raw", do_as_raw, 1),
    prim("as.call", do_ascall, 1),
    prim("storage.mode<-", do_storage_mode, 2),
    internal("as.vector", do_asvector, 2),
    internal("typeof", do_typeof, 1),
    internal("is.vector", do_isvector, 2),
    prim("is.null", is_null, 1),
    prim("is.logical", is_logical, 1),
    prim("is.integer", is_integer, 1),
    prim("is.double", is_double, 1),
    prim("is.complex", is_complex, 1),
    prim("is.character", is_character, 1),
    prim("is.symbol", is_symbol, 1),
    prim("is.name", is_symbol, 1),
    prim("is.environment", is_environment, 1),
    prim("is.list", is_list, 1),
    prim("is.pairlist", is_pairlist, 1),
    prim("is.expression", is_expression, 1),
    prim("is.raw", is_raw, 1),
    prim("is.object", is_object, 1),
    prim("isS4", is_s4, 1),
    prim("is.numeric", is_numeric, 1),
    prim("is.matrix", is_matrix, 1),
    prim("is.array", is_array, 1),
    prim("is.atomic", is_atomic, 1),
    prim("is.recursive", is_recursive, 1),
    prim("is.call", is_call, 1),
    prim("is.language", is_language, 1),
    prim("is.function", is_function, 1),
    prim("is.single", is_single, 1),
    prim("is.na", do_isna, 1),
    prim("is.nan", do_isnan, 1),
    prim("is.finite", do_isfinite, 1),
    prim("is.infinite", do_isinfinite, 1),
    special("call", do_call, -1),
    internal("do.call", do_docall, 3).custom_vis(),
    internal("as.function.default", do_asfunction, 2),
];

fn one(args: &[Arg], name: &str, call: &Value) -> RResult<Value> {
    check_arity(args, 1, name, call)?;
    Ok(args[0].value.clone())
}

/// `ascommon` for the atomic types: coerce and drop all attributes.
fn as_atomic(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, generic: &str, t: VType) -> RResult<Value> {
    if args.is_empty() {
        return Ok(Value::empty_of(t));
    }
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch(generic, call, &args, env)? {
            return Ok(r);
        }
    }
    let x = args.into_iter().next().unwrap().value;
    if x.vtype() == Some(t) && !x.has_attrs() {
        return Ok(x);
    }
    match &x {
        Value::Env(_) | Value::Closure(_) | Value::Builtin(_) | Value::ExtPtr(_) => {
            return Err(Flow::error(format!(
                "cannot coerce type '{}' to vector of type '{}'",
                x.type_name(),
                t.name()
            )));
        }
        Value::Sym(sy) if t == VType::Str => return Ok(Value::strs(vec![Some(sy.name())])),
        Value::Sym(_) => {
            return Err(Flow::error(format!("cannot coerce type 'symbol' to vector of type '{}'", t.name())));
        }
        Value::List(l) | Value::Expr(l) if t != VType::Str => {
            for e in &l.data {
                if !(e.is_atomic() && e.len() == 1) && !e.is_null() {
                    return Err(Flow::error(format!(
                        "'list' object cannot be coerced to type '{}'",
                        t.name()
                    )));
                }
            }
        }
        _ => {}
    }
    let mut r = i.coerce(&x, t)?;
    r.clear_attrs();
    Ok(r)
}

fn do_as_character(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    as_atomic(i, c, a, e, "as.character", VType::Str)
}
fn do_as_integer(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    as_atomic(i, c, a, e, "as.integer", VType::Int)
}
fn do_as_double(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    let generic = match c {
        Value::Lang(l) if l.fun_sym().is_some_and(|s| &*s.name() == "as.numeric") => "as.numeric",
        _ => "as.double",
    };
    as_atomic(i, c, a, e, generic, VType::Real)
}
fn do_as_complex(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    as_atomic(i, c, a, e, "as.complex", VType::Cplx)
}
fn do_as_logical(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    as_atomic(i, c, a, e, "as.logical", VType::Lgl)
}
fn do_as_raw(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    if !a.is_empty() && !a[0].value.is_numeric_type() && !matches!(a[0].value, Value::Raw(_) | Value::Str(_) | Value::Cplx(_) | Value::Null | Value::List(_)) {
        return Err(Flow::error(format!("cannot coerce type '{}' to vector of type 'raw'", a[0].value.type_name())));
    }
    let r = as_atomic(i, c, a, e, "as.raw", VType::Raw)?;
    Ok(r)
}

fn do_ascall(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = one(&args, "as.call", call)?;
    match &x {
        Value::Lang(_) => Ok(x),
        Value::List(l) | Value::Expr(l) => {
            if l.data.is_empty() {
                return Err(Flow::error_call(call.clone(), "invalid argument list"));
            }
            let names = crate::attrib::names_vec(&x);
            let items: Vec<Arg> = l
                .data
                .iter()
                .enumerate()
                .map(|(k, v)| {
                    let tag = names
                        .as_ref()
                        .and_then(|n| n.get(k).cloned().flatten())
                        .filter(|s| !s.is_empty())
                        .map(|s| Sym::new(&s));
                    Arg::new(tag, v.clone())
                })
                .collect();
            Ok(Value::Lang(Rc::new(Lang::from_items(items, None))))
        }
        Value::Pairlist(p) => Ok(Value::Lang(Rc::new(Lang::from_items(p.items.clone(), None)))),
        _ => Err(Flow::error_call(call.clone(), "invalid argument list")),
    }
}

fn do_storage_mode(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "storage.mode<-", call)?;
    let x = args[0].value.clone();
    let mode = match &args[1].value {
        Value::Str(m) if m.data.len() == 1 => m.data[0].clone().unwrap_or_else(|| Rc::from("")),
        _ => return Err(Flow::error_call(call.clone(), "'value' must be non-null character string")),
    };
    let t = match &*mode {
        "real" | "single" => return Err(Flow::error_call(call.clone(), "use of 'real' is defunct: use 'double' instead")),
        m => VType::from_name(m).ok_or_else(|| Flow::error_call(call.clone(), "invalid value"))?,
    };
    if x.vtype() == Some(t) {
        return Ok(x);
    }
    if x.inherits("factor") {
        return Err(Flow::error_call(call.clone(), "invalid to change the storage mode of a factor"));
    }
    i.coerce(&x, t)
}

fn do_asvector(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let mode = match arg(&args, 1) {
        Value::Str(m) if m.data.len() == 1 => m.data[0].clone().unwrap_or_else(|| Rc::from("")),
        _ => return Err(Flow::error_call(call.clone(), "invalid 'mode' argument")),
    };
    as_vector(i, x, &mode, call)
}

pub fn as_vector(i: &mut Interp, x: Value, mode: &str, call: &Value) -> RResult<Value> {
    if mode == "function" || mode == "closure" {
        return Err(Flow::error_call(call.clone(), "cannot coerce to function"));
    }
    let t = match mode {
        "any" => None,
        "name" | "symbol" => {
            return match &x {
                Value::Sym(_) => Ok(x),
                _ => {
                    let s = as_string(&x).ok_or_else(|| Flow::error("invalid type/length (symbol/0) in vector allocation"))?;
                    Ok(Value::Sym(Sym::new(&s)))
                }
            };
        }
        "pairlist" => {
            let items = to_list(&x);
            let names = crate::attrib::names_vec(&x);
            let args = items
                .into_iter()
                .enumerate()
                .map(|(k, v)| {
                    let tag = names.as_ref().and_then(|n| n.get(k).cloned().flatten()).filter(|s| !s.is_empty()).map(|s| Sym::new(&s));
                    Arg::new(tag, v)
                })
                .collect();
            return Ok(Value::pairlist(args));
        }
        m => Some(VType::from_name(m).ok_or_else(|| Flow::error_call(call.clone(), "invalid 'mode' argument"))?),
    };
    match t {
        None => match &x {
            v if v.is_atomic() => {
                if !v.has_attrs() {
                    return Ok(x);
                }
                let mut r = x;
                r.clear_attrs();
                Ok(r)
            }
            Value::List(_) | Value::Expr(_) => Ok(x),
            Value::Pairlist(_) | Value::Lang(_) => {
                let mut l = Value::list(to_list(&x));
                if let Some(n) = x.names() {
                    l.set_attr_raw(s::NAMES, n);
                }
                Ok(l)
            }
            _ => Ok(x),
        },
        Some(t) => {
            if x.vtype() == Some(t) {
                if matches!(t, VType::List | VType::Expr) {
                    return Ok(x);
                }
                let mut r = x;
                r.clear_attrs();
                return Ok(r);
            }
            match t {
                VType::List => {
                    let mut l = Value::list(to_list(&x));
                    if let Some(n) = x.names() {
                        l.set_attr_raw(s::NAMES, n);
                    }
                    if x.is_atomic() {
                        if let Some(d) = x.get_attr(s::DIM) {
                            l.set_attr_raw(s::DIM, d);
                        }
                        if let Some(d) = x.get_attr(s::DIMNAMES) {
                            l.set_attr_raw(s::DIMNAMES, d);
                        }
                    }
                    Ok(l)
                }
                VType::Expr => {
                    let items = match &x {
                        Value::List(l) => l.data.clone(),
                        other => vec![other.clone()],
                    };
                    Ok(Value::expr_vec(items))
                }
                _ => {
                    if matches!(x, Value::Env(_) | Value::Closure(_) | Value::Builtin(_)) {
                        return Err(Flow::error(format!(
                            "cannot coerce type '{}' to vector of type '{}'",
                            x.type_name(),
                            t.name()
                        )));
                    }
                    if let Value::List(l) = &x {
                        for e in &l.data {
                            if !(e.is_atomic() && e.len() == 1) && !(t == VType::Str) {
                                return Err(Flow::error(format!(
                                    "'list' object cannot be coerced to type '{}'",
                                    t.name()
                                )));
                            }
                        }
                    }
                    let mut r = i.coerce(&x, t)?;
                    r.clear_attrs();
                    Ok(r)
                }
            }
        }
    }
}

fn do_typeof(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::str(arg(&args, 0).type_name()))
}

fn do_isvector(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let mode = match arg(&args, 1) {
        Value::Str(m) if m.data.len() == 1 => m.data[0].clone().unwrap_or_else(|| Rc::from("")),
        _ => return Err(Flow::error_call(call.clone(), "invalid 'mode' argument")),
    };
    let type_ok = match &*mode {
        "any" => x.is_vector(),
        "numeric" => matches!(x, Value::Int(_) | Value::Real(_)) && !x.inherits("factor"),
        "name" | "symbol" => matches!(x, Value::Sym(_)),
        "list" => matches!(x, Value::List(_)),
        m => x.type_name() == m,
    };
    if !type_ok {
        return Ok(Value::lgl(false));
    }
    if let Some(a) = x.attrs() {
        if a.0.iter().any(|(k, _)| *k != s::NAMES) {
            return Ok(Value::lgl(false));
        }
    }
    Ok(Value::lgl(true))
}

macro_rules! is_fn {
    ($name:ident, $rname:expr, |$x:ident| $body:expr) => {
        fn $name(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
            let $x = one(&args, $rname, call)?;
            Ok(Value::lgl($body))
        }
    };
}

is_fn!(is_null, "is.null", |x| x.is_null());
is_fn!(is_logical, "is.logical", |x| matches!(x, Value::Lgl(_)));
is_fn!(is_integer, "is.integer", |x| matches!(x, Value::Int(_)));
is_fn!(is_double, "is.double", |x| matches!(x, Value::Real(_)));
is_fn!(is_complex, "is.complex", |x| matches!(x, Value::Cplx(_)));
is_fn!(is_character, "is.character", |x| matches!(x, Value::Str(_)));
is_fn!(is_symbol, "is.symbol", |x| matches!(x, Value::Sym(_)));
is_fn!(is_environment, "is.environment", |x| matches!(x, Value::Env(_)));
is_fn!(is_list, "is.list", |x| matches!(x, Value::List(_) | Value::Pairlist(_)));
is_fn!(is_pairlist, "is.pairlist", |x| matches!(x, Value::Pairlist(_) | Value::Null));
is_fn!(is_expression, "is.expression", |x| matches!(x, Value::Expr(_)));
is_fn!(is_raw, "is.raw", |x| matches!(x, Value::Raw(_)));
is_fn!(is_object, "is.object", |x| x.is_object());
is_fn!(is_s4, "isS4", |x| { let _ = x; false });
is_fn!(is_array, "is.array", |x| x.get_attr(s::DIM).is_some_and(|d| !d.is_empty()));
is_fn!(is_atomic, "is.atomic", |x| x.is_atomic());
is_fn!(is_recursive, "is.recursive", |x| matches!(
    x,
    Value::List(_)
        | Value::Expr(_)
        | Value::Pairlist(_)
        | Value::Lang(_)
        | Value::Closure(_)
        | Value::Builtin(_)
        | Value::Env(_)
        | Value::Promise(_)
        | Value::Dots(_)
        | Value::ExtPtr(_)
));
is_fn!(is_call, "is.call", |x| matches!(x, Value::Lang(_)));
is_fn!(is_language, "is.language", |x| matches!(x, Value::Sym(_) | Value::Lang(_) | Value::Expr(_)));
is_fn!(is_function, "is.function", |x| x.is_function());

fn is_numeric(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.numeric", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("is.numeric", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    Ok(Value::lgl(matches!(x, Value::Int(_) | Value::Real(_)) && !x.inherits("factor")))
}

fn is_matrix(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.matrix", call)?;
    let _ = (i, env);
    Ok(Value::lgl(args[0].value.get_attr(s::DIM).is_some_and(|d| d.len() == 2)))
}

fn is_single(_: &mut Interp, call: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Err(Flow::error_call(call.clone(), "type \"single\" unimplemented in R"))
}

/// Copy names, dim and dimnames (for is.na and friends).
fn copy_dim_and_names(from: &Value, to: &mut Value) {
    if from.is_vector() || matches!(from, Value::Pairlist(_)) {
        if let Some(d) = from.get_attr(s::DIM) {
            to.set_attr_raw(s::DIM, d);
            if let Some(dn) = from.get_attr(s::DIMNAMES) {
                to.set_attr_raw(s::DIMNAMES, dn);
            }
        } else if let Some(n) = crate::attrib::get_attrib(from, s::NAMES).as_str_vec().map(|_| crate::attrib::get_attrib(from, s::NAMES)) {
            to.set_attr_raw(s::NAMES, n);
        }
    }
}

fn do_isna(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.na", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("is.na", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    let out: Vec<Lgl> = match x {
        Value::Lgl(v) | Value::Int(v) => v.data.iter().map(|&e| (e == NA_INTEGER) as Lgl).collect(),
        Value::Real(v) => v.data.iter().map(|e| e.is_nan() as Lgl).collect(),
        Value::Cplx(v) => v.data.iter().map(|z| (z.re.is_nan() || z.im.is_nan()) as Lgl).collect(),
        Value::Str(v) => v.data.iter().map(|e| e.is_none() as Lgl).collect(),
        Value::Raw(v) => vec![FALSE; v.data.len()],
        Value::List(l) | Value::Expr(l) => l
            .data
            .iter()
            .map(|e| {
                (e.len() == 1
                    && match e {
                        Value::Lgl(v) | Value::Int(v) => v.data[0] == NA_INTEGER,
                        Value::Real(v) => v.data[0].is_nan(),
                        Value::Str(v) => v.data[0].is_none(),
                        Value::Cplx(v) => v.data[0].re.is_nan() || v.data[0].im.is_nan(),
                        _ => false,
                    }) as Lgl
            })
            .collect(),
        Value::Pairlist(p) => p.items.iter().map(|_| FALSE).collect(),
        Value::Null => vec![],
        other => {
            i.warningcall(
                call,
                format!("is.na() applied to non-(list or vector) of type '{}'", other.type_name()),
            )?;
            vec![FALSE]
        }
    };
    let mut r = Value::lgls(out);
    copy_dim_and_names(x, &mut r);
    Ok(r)
}

fn do_isnan(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.nan", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("is.nan", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    let out: Vec<Lgl> = match x {
        Value::Lgl(v) | Value::Int(v) => vec![FALSE; v.data.len()],
        Value::Real(v) => v.data.iter().map(|e| (e.is_nan() && !is_na_real(*e)) as Lgl).collect(),
        Value::Cplx(v) => v
            .data
            .iter()
            .map(|z| ((z.re.is_nan() && !is_na_real(z.re)) || (z.im.is_nan() && !is_na_real(z.im))) as Lgl)
            .collect(),
        Value::Null => vec![],
        other => {
            return Err(Flow::error_call(
                call.clone(),
                format!("default method not implemented for type '{}'", other.type_name()),
            ))
        }
    };
    let mut r = Value::lgls(out);
    copy_dim_and_names(x, &mut r);
    Ok(r)
}

fn do_isfinite(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.finite", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("is.finite", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    let out: Vec<Lgl> = match x {
        Value::Lgl(v) | Value::Int(v) => v.data.iter().map(|&e| (e != NA_INTEGER) as Lgl).collect(),
        Value::Real(v) => v.data.iter().map(|e| e.is_finite() as Lgl).collect(),
        Value::Cplx(v) => v.data.iter().map(|z| (z.re.is_finite() && z.im.is_finite()) as Lgl).collect(),
        Value::Null => vec![],
        Value::Str(v) => vec![FALSE; v.data.len()],
        Value::Raw(v) => vec![FALSE; v.data.len()],
        other => {
            return Err(Flow::error_call(
                call.clone(),
                format!("default method not implemented for type '{}'", other.type_name()),
            ))
        }
    };
    let mut r = Value::lgls(out);
    copy_dim_and_names(x, &mut r);
    Ok(r)
}

fn do_isinfinite(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.infinite", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("is.infinite", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    let out: Vec<Lgl> = match x {
        Value::Real(v) => v.data.iter().map(|e| e.is_infinite() as Lgl).collect(),
        Value::Cplx(v) => v.data.iter().map(|z| (z.re.is_infinite() || z.im.is_infinite()) as Lgl).collect(),
        Value::Null => vec![],
        Value::Lgl(v) | Value::Int(v) => vec![FALSE; v.data.len()],
        Value::Str(v) => vec![FALSE; v.data.len()],
        Value::Raw(v) => vec![FALSE; v.data.len()],
        other => {
            return Err(Flow::error_call(
                call.clone(),
                format!("default method not implemented for type '{}'", other.type_name()),
            ))
        }
    };
    let mut r = Value::lgls(out);
    copy_dim_and_names(x, &mut r);
    Ok(r)
}

fn do_call(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.is_empty() {
        return Err(Flow::error_call(call.clone(), "first argument must be a character string"));
    }
    let name = i.eval(&args[0].value, env)?;
    let fname = match &name {
        Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => v.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "first argument must be a character string")),
    };
    let rest = i.eval_args(&args[1..], env, call)?;
    Ok(Value::lang(Value::Sym(Sym::new(&fname)), rest))
}

fn do_docall(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let what = arg(&args, 0);
    let fargs = arg(&args, 1);
    let envir = match arg(&args, 2) {
        Value::Env(e) => e,
        _ => return Err(Flow::error_call(call.clone(), "'envir' must be an environment")),
    };
    let f = match &what {
        Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => Value::Sym(Sym::new(v.data[0].as_ref().unwrap())),
        Value::Closure(_) | Value::Builtin(_) => what.clone(),
        _ => return Err(Flow::error_call(call.clone(), "'what' must be a function or character string")),
    };
    if !matches!(fargs, Value::List(_)) && !fargs.is_null() {
        return Err(Flow::error_call(call.clone(), "second argument must be a list"));
    }
    let names = crate::attrib::names_vec(&fargs);
    let items: Vec<Arg> = to_list(&fargs)
        .into_iter()
        .enumerate()
        .map(|(k, v)| {
            let tag = names.as_ref().and_then(|n| n.get(k).cloned().flatten()).filter(|s| !s.is_empty()).map(|s| Sym::new(&s));
            Arg::new(tag, v)
        })
        .collect();
    let c = Value::lang(f, items);
    i.eval(&c, &envir)
}

fn do_asfunction(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let envir = match arg(&args, 1) {
        Value::Env(e) => e,
        _ => return Err(Flow::error_call(call.clone(), "invalid environment")),
    };
    let items: Vec<Value> = to_list(&x);
    if items.is_empty() {
        return Err(Flow::error_call(call.clone(), "argument must have length at least 1"));
    }
    let names = crate::attrib::names_vec(&x);
    let n = items.len();
    let formals: Vec<Arg> = items[..n - 1]
        .iter()
        .enumerate()
        .map(|(k, v)| {
            let tag = names.as_ref().and_then(|nm| nm.get(k).cloned().flatten()).map(|s| Sym::new(&s));
            Arg::new(tag, v.clone())
        })
        .collect();
    Ok(Value::Closure(Rc::new(Closure { formals: formals.into(), body: items[n - 1].clone(), env: envir, attrs: None, code: Default::default() })))
}
