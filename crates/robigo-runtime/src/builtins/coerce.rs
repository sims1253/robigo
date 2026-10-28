//! Type predicates and conversions: `as.*`, `is.*`, `typeof`,
//! `as.vector`, `call`, `do.call` (GNU R's coerce.c).

use std::rc::Rc;

use super::{arg, check_arity, internal, prim, special};
use crate::coerce::*;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
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
                    return Err(Flow::error(format!("'list' object cannot be coerced to type '{}'", t.name())));
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
    // R: base's as.raw has no default argument, so the arity check of
    // do_asatomic's checkArity is observable (as.integer etc. supply
    // their defaults at the R level first).
    check_arity(&a, 1, "as.raw", c)?;
    if !a.is_empty()
        && !a[0].value.is_numeric_type()
        && !matches!(a[0].value, Value::Raw(_) | Value::Str(_) | Value::Cplx(_) | Value::Null | Value::List(_))
    {
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
        "real" | "single" => {
            return Err(Flow::error_call(call.clone(), "use of 'real' is defunct: use 'double' instead"));
        }
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

fn do_asvector(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // DispatchOrEval: as.vector is an internal generic.
    if let Some(r) = i.try_dispatch("as.vector", call, &args, env)? {
        return Ok(r);
    }
    let x = arg(&args, 0);
    let mode = match arg(&args, 1) {
        Value::Str(m) if m.data.len() == 1 => m.data[0].clone().unwrap_or_else(|| Rc::from("")),
        // R: error_return(R_MSG_mode) — plain error against the context.
        _ => return Err(Flow::error("invalid 'mode' argument")),
    };
    as_vector(i, x, &mode, call)
}

/// R: coerceVectorList's gate for comparison operands (relop.c's chain):
/// every element must be a vector of length at most 1 — an expression
/// never qualifies (isNewList() is FALSE for EXPRSXP) — else the
/// coercion fails as a plain error.
pub fn relop_list_coerce(v: &Value, t: VType, w: &mut CoerceWarnings) -> RResult<Value> {
    let items = match v {
        Value::List(l) | Value::Expr(l) => l.data.clone(),
        _ => unreachable!("relop_list_coerce on a non-list"),
    };
    if matches!(v, Value::Expr(_)) || !items.iter().all(|e| e.is_atomic() && e.len() <= 1) {
        return Err(Flow::error(format!("'list' object cannot be coerced to type '{}'", t.name())));
    }
    // The names travel with the coerced vector (do_relop_dflt reads them
    // from the original operand and applies them to the result).
    let mut list = Value::list(items);
    if let Some(n) = v.names() {
        list.set_attr_raw(s::NAMES, n);
    }
    crate::coerce::coerce_vector(&list, t, w)
}

pub fn as_vector(i: &mut Interp, x: Value, mode: &str, call: &Value) -> RResult<Value> {
    // R: coerce.c:do_asvector + ascommon. "function"/"closure" makes a
    // zero-formal closure with x as the body (asFunction/makeClosure).
    if mode == "function" || mode == "closure" {
        return Ok(match &x {
            Value::Closure(_) | Value::Builtin(_) => x,
            _ => Value::Closure(Rc::new(Closure {
                formals: Vec::new().into(),
                body: x.clone(),
                env: i.global.clone(),
                attrs: None,
                code: Default::default(),
                inline: Default::default(),
                defaults: Default::default(),
            })),
        });
    }
    // R: ascommon's tail — sources that are neither vectors, lists nor
    // language objects fail against the internal call.
    if matches!(x, Value::Env(_) | Value::Closure(_) | Value::Builtin(_)) {
        let type_name = match mode {
            "any" => "any",
            "name" | "symbol" => "symbol",
            "pairlist" => "pairlist",
            m => VType::from_name(m).map(|t| t.name()).unwrap_or(m),
        };
        return Err(Flow::error_call(
            call.clone(),
            format!("cannot coerce type '{}' to vector of type '{}'", x.type_name(), type_name),
        ));
    }
    let t = match mode {
        "any" => None,
        "name" | "symbol" => {
            // R: coerceToSymbol for atomic vectors (first element, or
            // "invalid data of mode 'x' (too short)"); vectorizable
            // lists fail in allocVector(SYMSXP, n) with the length.
            return match &x {
                Value::Sym(_) => Ok(x),
                v if v.is_atomic() && !v.is_empty() => {
                    let s = as_string(v).expect("non-empty atomic has a first element");
                    Ok(Value::Sym(Sym::new(&s)))
                }
                v if v.is_atomic() => Err(Flow::error(format!("invalid data of mode '{}' (too short)", v.type_name()))),
                Value::Null => Err(Flow::error("invalid type/length (symbol/0) in vector allocation")),
                Value::List(l) if l.data.iter().all(|e| e.is_atomic() && e.len() <= 1) => {
                    Err(Flow::error(format!("invalid type/length (symbol/{}) in vector allocation", l.data.len())))
                }
                Value::List(_) => Err(Flow::error("'list' object cannot be coerced to type 'symbol'")),
                // R: coercePairList — a vectorizable pairlist fails in
                // allocVector(SYMSXP, n) for any n (symbols cannot be
                // allocated as vectors); a call is never vectorizable.
                Value::Pairlist(p) if p.items.iter().all(|a| a.value.is_atomic() && a.value.len() <= 1) => {
                    Err(Flow::error(format!("invalid type/length (symbol/{}) in vector allocation", p.items.len())))
                }
                Value::Pairlist(_) => Err(Flow::error("'pairlist' object cannot be coerced to type 'symbol'")),
                Value::Lang(_) => Err(Flow::error("'language' object cannot be coerced to type 'symbol'")),
                // R: coerceVectorList — isNewList() is FALSE for
                // expressions, so they are never vectorizable.
                Value::Expr(_) => Err(Flow::error("'list' object cannot be coerced to type 'symbol'")),
                _ => Err(Flow::error_call(
                    call.clone(),
                    format!("cannot coerce type '{}' to vector of type 'symbol'", x.type_name()),
                )),
            };
        }
        "pairlist" => {
            // R: do_asvector returns a pairlist of the right type
            // unchanged (attributes kept: the final switch does not
            // clear LISTSXP).
            if let Value::Pairlist(_) = &x {
                return Ok(x);
            }
            // R: coerce.c:coercePairList — language objects whose
            // elements are not all scalars cannot be coerced.
            if let Value::Lang(l) = &x {
                let fn_scalar = l.fun.is_atomic() && l.fun.len() == 1;
                let all_scalar = fn_scalar && l.args.iter().all(|a| a.value.is_atomic() && a.value.len() == 1);
                if !all_scalar {
                    return Err(Flow::error(format!(
                        "'{}' object cannot be coerced to type 'pairlist'",
                        x.type_name()
                    )));
                }
            }
            let items = to_list(&x);
            let names = crate::attrib::names_vec(&x);
            let args = items
                .into_iter()
                .enumerate()
                .map(|(k, v)| {
                    let tag = names
                        .as_ref()
                        .and_then(|n| n.get(k).cloned().flatten())
                        .filter(|s| !s.is_empty())
                        .map(|s| Sym::new(&s));
                    Arg::new(tag, v)
                })
                .collect();
            let mut r = Value::pairlist(args);
            // R: VectorToPairList copies most attributes (the tags carry
            // the names); ascommon strips them for other sources.
            if matches!(x, Value::List(_) | Value::Expr(_)) {
                crate::attrib::copy_most_attrib(&x, &mut r);
            }
            return Ok(r);
        }
        // R's str2type + do_asvector's valid-type switch: modes like
        // "NULL", "environment", "language" or "single" are invalid
        // (plain error against the context).
        m => Some(
            VType::from_name(m)
                .filter(|t| !matches!(t, VType::Null))
                .ok_or_else(|| Flow::error("invalid 'mode' argument"))?,
        ),
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
            // ascommon: lists, expressions, pairlists and language
            // objects pass through unchanged for mode "any".
            Value::List(_) | Value::Expr(_) | Value::Pairlist(_) | Value::Lang(_) => Ok(x),
            Value::Null => Ok(x),
            Value::Env(_) | Value::Closure(_) | Value::Builtin(_) => Err(Flow::error_call(
                call.clone(),
                format!("cannot coerce type '{}' to vector of type 'any'", x.type_name()),
            )),
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
                    // R: as.vector(x, "list") on an atomic drops dim and
                    // dimnames (as.list(matrix) is a flat list) but keeps
                    // names. An expression keeps everything (the duplicate
                    // only changes the type). Pairlists and calls go
                    // through PairToVectorList: names from the tags, then
                    // copyMostAttrib (which keeps the other attributes).
                    let mut l = Value::list(to_list(&x));
                    if let Some(n) = x.names() {
                        l.set_attr_raw(s::NAMES, n);
                    }
                    if let Value::Expr(_) = &x {
                        l.set_attrs(x.attrs().cloned().map(Box::new));
                    } else if matches!(x, Value::Pairlist(_) | Value::Lang(_)) {
                        crate::attrib::copy_most_attrib(&x, &mut l);
                    }
                    Ok(l)
                }
                VType::Expr => {
                    // R: coerceToExpression wraps non-lists; a pairlist or
                    // call is wrapped whole (coercePairList's EXPRSXP case).
                    let items = match &x {
                        Value::List(l) => l.data.clone(),
                        Value::Pairlist(_) | Value::Lang(_) => vec![x.clone()],
                        other => vec![other.clone()],
                    };
                    Ok(Value::expr_vec(items))
                }
                _ => {
                    if matches!(x, Value::Env(_) | Value::Closure(_) | Value::Builtin(_)) {
                        return Err(Flow::error_call(
                            call.clone(),
                            format!("cannot coerce type '{}' to vector of type '{}'", x.type_name(), t.name()),
                        ));
                    }
                    // R: coerceVectorList (VECSXP/EXPRSXP sources) and
                    // coercePairList (LISTSXP/LANGSXP sources): every
                    // element must be a vector of length at most 1, else
                    // the coercion fails as a plain error. Character is
                    // exempt (the deparse path comes first), and an
                    // expression never qualifies (isNewList() is FALSE for
                    // EXPRSXP, so even the empty expression errors).
                    let src_name = match &x {
                        Value::List(_) | Value::Expr(_) => "list",
                        Value::Pairlist(_) => "pairlist",
                        Value::Lang(_) => "language",
                        _ => "",
                    };
                    if !src_name.is_empty() && t != VType::Str {
                        let items: Vec<Value> = match &x {
                            Value::List(l) | Value::Expr(l) => l.data.clone(),
                            Value::Pairlist(p) => p.items.iter().map(|a| a.value.clone()).collect(),
                            Value::Lang(l) => l.items().into_iter().map(|a| a.value).collect(),
                            _ => unreachable!(),
                        };
                        let vectorizable =
                            !matches!(x, Value::Expr(_)) && items.iter().all(|e| e.is_atomic() && e.len() <= 1);
                        if !vectorizable {
                            return Err(Flow::error(format!(
                                "'{}' object cannot be coerced to type '{}'",
                                src_name,
                                t.name()
                            )));
                        }
                        // Element-wise (asLogical, asInteger, ...); the tag
                        // names are cleared with the attributes after.
                        let mut r = i.coerce(&Value::list(items), t)?;
                        r.clear_attrs();
                        return Ok(r);
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
is_fn!(is_s4, "isS4", |x| {
    let _ = x;
    false
});
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
        } else if let Some(n) =
            crate::attrib::get_attrib(from, s::NAMES).as_str_vec().map(|_| crate::attrib::get_attrib(from, s::NAMES))
        {
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
            i.warningcall(call, format!("is.na() applied to non-(list or vector) of type '{}'", other.type_name()))?;
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
            ));
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
            ));
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
            ));
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
            let tag = names
                .as_ref()
                .and_then(|n| n.get(k).cloned().flatten())
                .filter(|s| !s.is_empty())
                .map(|s| Sym::new(&s));
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
    Ok(Value::Closure(Rc::new(Closure {
        formals: formals.into(),
        body: items[n - 1].clone(),
        env: envir,
        attrs: None,
        code: Default::default(),
        inline: Default::default(),
        defaults: Default::default(),
    })))
}
