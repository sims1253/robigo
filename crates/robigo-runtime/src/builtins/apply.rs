//! The apply family: `lapply`, `vapply`, `rapply`, `eapply` (GNU R's
//! apply.c and envir.c) and `mapply` (mapply.c). `sapply`, `Map`,
//! `Reduce`, `Filter`, `outer`, `apply` and `tapply` are base R code on
//! top of these.

use std::rc::Rc;

use super::{arg, internal};
use crate::attrib::{get_attrib, set_attrib};
use crate::coerce::{as_logical, coerce_quiet};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("lapply", do_lapply, 2).special(),
    internal("vapply", do_vapply, 4).special(),
    internal("eapply", do_eapply, 4).special(),
    internal("rapply", do_rapply, 5),
    internal("mapply", do_mapply, 3),
    internal("islistfactor", do_islistfactor, 2),
];

/// R's `R_forceAndCall(e, n, rho)`: evaluate the call `e` in `rho`,
/// forcing the promises for the first `n` arguments of a closure before
/// applying it.
pub fn force_and_call(i: &mut Interp, e: &Value, n: usize, rho: &Env) -> RResult<Value> {
    let Value::Lang(l) = e else { return i.eval(e, rho) };
    let fun = match &l.fun {
        Value::Sym(sym) => i.find_fun(*sym, rho)?,
        other => i.eval(other, rho)?,
    };
    match &fun {
        Value::Builtin(b) => i.call_builtin(b, e, &l.args, rho),
        Value::Closure(c) => {
            let pargs = i.promise_args(&l.args, rho)?;
            for (k, a) in pargs.iter().take(n).enumerate() {
                match &a.value {
                    Value::Promise(p) => {
                        i.force_promise(p)?;
                    }
                    v if v.is_missing_arg() => {
                        return Err(Flow::error_call(e.clone(), format!("argument {} is empty", k + 1)));
                    }
                    _ => {}
                }
            }
            i.apply_closure(e, c, pargs, rho, None)
        }
        _ => Err(Flow::error("attempt to apply non-function")),
    }
}

fn check_arg_is_symbol(x: &Value) -> RResult<Value> {
    match x {
        Value::Sym(_) => Ok(x.clone()),
        _ => Err(Flow::error("argument must be a symbol")),
    }
}

/// `checkArity` for the `.Internal`s here: R: util.c:Rf_checkArityCall —
/// `.Internal`s name the internal as `.Internal(<name>)` and use error()
/// (the context call).
fn check_arity_internal(args: &[Arg], n: usize, name: &str) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            name,
            n
        )));
    }
    Ok(())
}

/// A closure and pre-evaluated arguments that can be executed inline per element.
struct FastClosure {
    clo: Rc<Closure>,
    args: Vec<Value>,
    x_slot: usize,
}

enum FastApply {
    One(Rc<Closure>),
    Multi(FastClosure),
}

/// The closure `FUN` when `FUN(X[[i]], ...)` (in `rho`, `X` being `xx`)
/// can run without a call (see `vm::inlinable`): `FUN` a pure closure,
/// `xx` a plain vector or list whose elements `[[` reads without dispatch,
/// and any `...` arguments are evaluated once before the loop.
fn inline_fun(i: &mut Interp, fcall: &Value, rho: &Env, xx: &Value) -> RResult<Option<FastApply>> {
    if xx.is_object() || !matches!(xx, Value::Real(_) | Value::Int(_) | Value::Lgl(_) | Value::Str(_) | Value::List(_))
    {
        return Ok(None);
    }
    let Value::Lang(l) = fcall else { return Ok(None) };
    let Value::Sym(f) = &l.fun else { return Ok(None) };
    let Ok(fun_val) = i.find_fun(*f, rho) else { return Ok(None) };
    let Value::Closure(c) = fun_val else { return Ok(None) };
    if !crate::vm::inlinable(&c) {
        return Ok(None);
    }
    let nf = c.formals.len();
    if nf == 0 {
        return Ok(None);
    }
    let dots = match rho.lookup(s::DOTS) {
        Some((Value::Dots(d), _)) => d,
        _ => Rc::new(Vec::new()),
    };
    if nf == 1 && dots.is_empty() && c.formals[0].tag != Some(s::DOTS) {
        return Ok(Some(FastApply::One(c)));
    }

    let mut supplied = Vec::with_capacity(1 + dots.len());
    supplied.push(Arg::pos(Value::missing_arg()));
    supplied.extend(dots.iter().cloned());

    let matched = match i.match_args(&c.formals, &supplied, fcall) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    if matched.len() != nf {
        return Ok(None);
    }

    let mut x_slot = None;
    let mut args = Vec::with_capacity(nf);
    for (idx, m) in matched.into_iter().enumerate() {
        let Some(actual) = m else { return Ok(None) };
        if actual.is_missing_arg() {
            if x_slot.is_some() {
                return Ok(None);
            }
            x_slot = Some(idx);
            args.push(Value::Null);
        } else {
            let val = match actual {
                Value::Promise(p) => i.force_promise(&p)?,
                v => v,
            };
            if val.is_missing_arg() {
                return Ok(None);
            }
            args.push(val);
        }
    }
    let Some(x_slot) = x_slot else { return Ok(None) };
    Ok(Some(FastApply::Multi(FastClosure { clo: c, args, x_slot })))
}

/// `xx[[k + 1]]` of a plain vector or list (see [`inline_fun`]).
fn element2(xx: &Value, k: usize) -> Value {
    match xx {
        Value::Real(v) => Value::real(v.data[k]),
        Value::Int(v) => Value::int(v.data[k]),
        Value::Lgl(v) => Value::lgl_raw(v.data[k]),
        Value::Str(v) => Value::strs(vec![v.data[k].clone()]),
        Value::List(v) => v.data[k].clone(),
        _ => unreachable!(),
    }
}

/// `FUN(X[[i]], ...)` for element `k`, inline when `fast` allows.
#[inline(always)]
fn call_fun(
    i: &mut Interp,
    fast: &mut Option<FastApply>,
    xx: &Value,
    k: usize,
    fcall: &Value,
    isym: Sym,
    rho: &Env,
) -> RResult<Value> {
    match fast {
        Some(FastApply::One(c)) => {
            if let Some(v) = crate::vm::call_inline(i, c, &[element2(xx, k)]) {
                return Ok(v);
            }
        }
        Some(FastApply::Multi(fc)) => {
            fc.args[fc.x_slot] = element2(xx, k);
            if let Some(v) = crate::vm::call_inline(i, &fc.clo, &fc.args) {
                return Ok(v);
            }
        }
        None => {}
    }
    rho.set_local(isym, Value::int(k as i32 + 1));
    force_and_call(i, fcall, 1, rho)
}

/// `FUN(X[[i]], ...)`
fn fun_x_i_call(fun: Value, x: Value) -> Value {
    let idx = Value::lang(Value::Sym(s::BRACKET2), vec![Arg::pos(x), Arg::pos(Value::sym("i"))]);
    Value::lang(fun, vec![Arg::pos(idx), Arg::pos(Value::Sym(s::DOTS))])
}

/// R: apply.c:do_lapply (a special .Internal).
fn do_lapply(i: &mut Interp, _: &Value, args: Vec<Arg>, rho: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "lapply")?;
    let x = check_arg_is_symbol(&arg(&args, 0))?;
    let xx = i.eval(&x, rho)?;
    let n = xx.len();
    let fun = check_arg_is_symbol(&arg(&args, 1))?;
    let names = get_attrib(&xx, s::NAMES);
    let fcall = fun_x_i_call(fun, x);
    let isym = Sym::new("i");
    let mut out = Vec::with_capacity(n);
    let fast = if n > 0 { inline_fun(i, &fcall, rho, &xx)? } else { None };
    match fast {
        Some(FastApply::One(c)) => {
            for k in 0..n {
                if let Some(v) = crate::vm::call_inline(i, &c, &[element2(&xx, k)]) {
                    out.push(v);
                } else {
                    rho.set_local(isym, Value::int(k as i32 + 1));
                    out.push(force_and_call(i, &fcall, 1, rho)?);
                }
            }
        }
        Some(FastApply::Multi(mut fc)) => {
            for k in 0..n {
                fc.args[fc.x_slot] = element2(&xx, k);
                if let Some(v) = crate::vm::call_inline(i, &fc.clo, &fc.args) {
                    out.push(v);
                } else {
                    rho.set_local(isym, Value::int(k as i32 + 1));
                    out.push(force_and_call(i, &fcall, 1, rho)?);
                }
            }
        }
        None => {
            for k in 0..n {
                rho.set_local(isym, Value::int(k as i32 + 1));
                out.push(force_and_call(i, &fcall, 1, rho)?);
            }
        }
    }
    let mut ans = Value::list(out);
    if !names.is_null() {
        ans = set_attrib(ans, s::NAMES, names)?;
    }
    Ok(ans)
}

fn vtype_ok(common: VType, val: VType) -> bool {
    match common {
        VType::Cplx => matches!(val, VType::Real | VType::Int | VType::Lgl),
        VType::Real => matches!(val, VType::Int | VType::Lgl),
        VType::Int => val == VType::Lgl,
        _ => false,
    }
}

/// R: apply.c:do_vapply (a special .Internal).
fn do_vapply(i: &mut Interp, _: &Value, args: Vec<Arg>, rho: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "vapply")?;
    let x = arg(&args, 0);
    let xx = i.eval(&x, rho)?;
    let fun = arg(&args, 1);
    let value = i.eval(&arg(&args, 2), rho)?;
    if !value.is_vector() {
        return Err(Flow::error("'FUN.VALUE' must be a vector"));
    }
    let use_names = as_logical(&i.eval(&arg(&args, 3), rho)?);
    if use_names == NA_LOGICAL {
        return Err(Flow::error("invalid 'USE.NAMES' value"));
    }
    let use_names = use_names != 0;
    let n = xx.len();
    let common_len = value.len();
    let common_type = value.vtype().unwrap();
    if !matches!(
        common_type,
        VType::Cplx | VType::Real | VType::Int | VType::Lgl | VType::Raw | VType::Str | VType::List
    ) {
        return Err(Flow::error(format!("type '{}' is not supported", value.type_name())));
    }
    let dim_v = value.get_attr(s::DIM);
    let array_value = matches!(&dim_v, Some(Value::Int(d)) if !d.data.is_empty());
    let mut names = Value::Null;
    let mut row_names = Value::Null;
    if use_names {
        names = get_attrib(&xx, s::NAMES);
        if names.is_null() && matches!(xx, Value::Str(_)) {
            names = xx.clone();
        }
        row_names = get_attrib(&value, if array_value { s::DIMNAMES } else { s::NAMES });
    }
    let fcall = fun_x_i_call(fun, x);
    let isym = Sym::new("i");
    let mut parts: Vec<Value> = Vec::with_capacity(n);
    let mut fast = if n > 0 { inline_fun(i, &fcall, rho, &xx)? } else { None };
    for k in 0..n {
        let mut val = call_fun(i, &mut fast, &xx, k, &fcall, isym, rho)?;
        if val.len() != common_len {
            return Err(Flow::error(format!(
                "values must be length {},\n but FUN(X[[{}]]) result is length {}",
                common_len,
                k + 1,
                val.len()
            )));
        }
        let vt = val.vtype();
        if vt != Some(common_type) {
            let ok = vt.is_some_and(|t| vtype_ok(common_type, t));
            if !ok {
                return Err(Flow::error(format!(
                    "values must be type '{}',\n but FUN(X[[{}]]) result is type '{}'",
                    value.type_name(),
                    k + 1,
                    val.type_name()
                )));
            }
            val = coerce_quiet(&val, common_type);
        }
        if k == 0 && use_names && row_names.is_null() {
            row_names = get_attrib(&val, if array_value { s::DIMNAMES } else { s::NAMES });
        }
        parts.push(val);
    }
    let mut ans = concat_values(common_type, &parts, n * common_len);
    let mut rnk_v = 0usize;
    if common_len != 1 {
        let mut dim: Vec<i32> = match (&dim_v, array_value) {
            (Some(Value::Int(d)), true) => d.data.clone(),
            _ => vec![common_len as i32],
        };
        rnk_v = dim.len();
        dim.push(n as i32);
        ans.set_attr_raw(s::DIM, Value::ints(dim));
    }
    if use_names {
        if common_len == 1 {
            if !names.is_null() {
                ans = set_attrib(ans, s::NAMES, names)?;
            }
        } else if !names.is_null() || !row_names.is_null() {
            let mut dn = vec![Value::Null; rnk_v + 1];
            if array_value && !row_names.is_null() {
                match &row_names {
                    Value::List(l) if l.data.len() == rnk_v => {
                        dn[..rnk_v].clone_from_slice(&l.data[..rnk_v]);
                    }
                    _ => {
                        return Err(Flow::error(format!(
                            "dimnames(<value>) is neither NULL nor list of length {}",
                            rnk_v
                        )));
                    }
                }
            } else {
                dn[0] = row_names;
            }
            dn[rnk_v] = names;
            ans = set_attrib(ans, s::DIMNAMES, Value::list(dn))?;
        }
    }
    Ok(ans)
}

/// Concatenate the data of same-typed vectors.
fn concat_values(t: VType, parts: &[Value], len: usize) -> Value {
    macro_rules! cat {
        ($variant:ident, $ctor:ident) => {{
            let mut out = Vec::with_capacity(len);
            for p in parts {
                if let Value::$variant(v) = p {
                    out.extend(v.data.iter().cloned());
                }
            }
            Value::$ctor(out)
        }};
    }
    match t {
        VType::Lgl => cat!(Lgl, lgls),
        VType::Int => cat!(Int, ints),
        VType::Real => cat!(Real, reals),
        VType::Cplx => cat!(Cplx, cplxs),
        VType::Str => cat!(Str, strs),
        VType::Raw => cat!(Raw, raws),
        VType::List => cat!(List, list),
        _ => Value::Null,
    }
}

/// Workhorse of `rapply()` (R: apply.c:do_one).
fn rapply_one(
    i: &mut Interp,
    x: &Value,
    fun: &Value,
    classes: &[RStr],
    deflt: &Value,
    replace: bool,
    rho: &Env,
) -> RResult<Value> {
    if x.is_null() || x.is_vector_list() {
        let items: Vec<Value> = x.as_list_vec().map(|l| l.data.clone()).unwrap_or_default();
        let mut out = Vec::with_capacity(items.len());
        for e in &items {
            out.push(rapply_one(i, e, fun, classes, deflt, replace, rho)?);
        }
        if replace {
            let mut ans = x.clone();
            match &mut ans {
                Value::List(l) | Value::Expr(l) => Rc::make_mut(l).data = out,
                _ => {}
            }
            return Ok(ans);
        }
        let mut ans = Value::list(out);
        let names = get_attrib(x, s::NAMES);
        if !names.is_null() {
            ans = set_attrib(ans, s::NAMES, names)?;
        }
        return Ok(ans);
    }
    let matched = if classes.first().and_then(|c| c.as_deref()) == Some("ANY") {
        true
    } else {
        let klass = crate::attrib::class_of(x);
        let kl: Vec<RStr> = klass.as_str_vec().map(|v| v.data.clone()).unwrap_or_default();
        kl.iter().any(|k| classes.iter().any(|c| c == k))
    };
    if matched {
        let xsym = Sym::new("X");
        rho.set_local(xsym, x.clone());
        let fcall = Value::lang(fun.clone(), vec![Arg::pos(Value::Sym(xsym)), Arg::pos(Value::Sym(s::DOTS))]);
        force_and_call(i, &fcall, 1, rho)
    } else if replace {
        Ok(x.clone())
    } else {
        Ok(deflt.clone())
    }
}

/// R: apply.c:do_rapply
fn do_rapply(i: &mut Interp, _: &Value, args: Vec<Arg>, rho: &Env) -> RResult<Value> {
    check_arity_internal(&args, 5, "rapply")?;
    let x = arg(&args, 0);
    if !x.is_vector_list() {
        return Err(Flow::error("'object' must be a list or expression"));
    }
    let fun = arg(&args, 1);
    if !fun.is_function() {
        return Err(Flow::error("invalid 'f' argument"));
    }
    let classes = match arg(&args, 2) {
        Value::Str(c) => c.data.clone(),
        _ => return Err(Flow::error("invalid 'classes' argument")),
    };
    let deflt = arg(&args, 3);
    let how = match arg(&args, 4) {
        Value::Str(h) if !h.data.is_empty() => h.data[0].clone(),
        _ => return Err(Flow::error("invalid 'how' argument")),
    };
    let replace = how.as_deref() == Some("replace");
    let items = x.as_list_vec().unwrap().data.clone();
    let mut out = Vec::with_capacity(items.len());
    for e in &items {
        out.push(rapply_one(i, e, &fun, &classes, &deflt, replace, rho)?);
    }
    if replace {
        let mut ans = x.clone();
        if let Value::List(l) | Value::Expr(l) = &mut ans {
            Rc::make_mut(l).data = out;
        }
        return Ok(ans);
    }
    let mut ans = Value::list(out);
    let names = get_attrib(&x, s::NAMES);
    if !names.is_null() {
        ans = set_attrib(ans, s::NAMES, names)?;
    }
    Ok(ans)
}

/// R: envir.c:do_eapply (a special .Internal).
fn do_eapply(i: &mut Interp, _: &Value, args: Vec<Arg>, rho: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "eapply")?;
    let env = i.eval(&arg(&args, 0), rho)?;
    if env.is_null() {
        return Err(Flow::error("use of NULL environment is defunct"));
    }
    let Value::Env(env) = env else {
        return Err(Flow::error("argument must be an environment"));
    };
    let fun = arg(&args, 1);
    if !matches!(fun, Value::Sym(_)) {
        return Err(Flow::error("arguments must be symbolic"));
    }
    let all = as_logical(&i.eval(&arg(&args, 2), rho)?);
    let all = all != NA_LOGICAL && all != 0;
    let use_nms = as_logical(&i.eval(&arg(&args, 3), rho)?);
    let use_nms = use_nms != NA_LOGICAL && use_nms != 0;
    let syms: Vec<Sym> = env.symbols().into_iter().filter(|sy| all || !sy.with_name(|n| n.starts_with('.'))).collect();
    let mut vals = Vec::with_capacity(syms.len());
    for sy in &syms {
        let v = env.get_local(*sy).unwrap_or(Value::Null);
        // R: envir.c:FrameValues (BINDING_VALUE calls an active binding)
        if env.binding_flags(*sy).is_some_and(|f| f & crate::env::BIND_ACTIVE != 0) {
            vals.push(i.call_value(&v, vec![], &env)?);
            continue;
        }
        vals.push(i.force(v)?);
    }
    let xsym = Sym::new("X");
    let isym = Sym::new("i");
    rho.set_local(xsym, Value::list(vals));
    let fcall = fun_x_i_call(fun, Value::Sym(xsym));
    let mut out = Vec::with_capacity(syms.len());
    for k in 0..syms.len() {
        rho.set_local(isym, Value::int(k as i32 + 1));
        out.push(force_and_call(i, &fcall, 1, rho)?);
    }
    let mut ans = Value::list(out);
    if use_nms {
        ans.set_attr_raw(s::NAMES, Value::strs(syms.iter().map(|s| Some(s.name().into())).collect()));
    }
    Ok(ans)
}

/// R: mapply.c:do_mapply
fn do_mapply(i: &mut Interp, _: &Value, args: Vec<Arg>, rho: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "mapply")?;
    let f = arg(&args, 0);
    let mut varying = arg(&args, 1);
    let constant = arg(&args, 2);
    if !matches!(varying, Value::List(_)) {
        varying = i.coerce(&varying, VType::List)?;
    }
    let vitems: Vec<Value> = varying.as_list_vec().map(|l| l.data.clone()).unwrap_or_default();
    let m = vitems.len();
    let mut lengths = vec![0usize; m];
    let mut longest = 0usize;
    let mut zero = 0;
    for (k, x) in vitems.iter().enumerate() {
        lengths[k] = x.len();
        if x.is_object() {
            // possibly dispatch on length()
            let lcall = Value::lang(Value::sym("length"), vec![Arg::pos(x.clone())]);
            if let Some(ans) = i.try_dispatch("length", &lcall, &[Arg::pos(x.clone())], rho)? {
                lengths[k] = match &ans {
                    Value::Real(r) if !r.data.is_empty() => r.data[0] as usize,
                    _ => crate::coerce::as_integer(&ans).max(0) as usize,
                };
            }
        }
        if lengths[k] == 0 {
            zero += 1;
        }
        longest = longest.max(lengths[k]);
        if zero > 0 && longest > 0 {
            return Ok(Value::list(vec![]));
        }
    }
    let vnames: Option<Vec<RStr>> = match get_attrib(&varying, s::NAMES) {
        Value::Str(n) => Some(n.data.clone()),
        _ => None,
    };
    let mut more: Vec<Arg> = match &constant {
        Value::Null => vec![],
        Value::List(l) | Value::Expr(l) => {
            let nm = get_attrib(&constant, s::NAMES);
            l.data
                .iter()
                .enumerate()
                .map(|(k, v)| {
                    let tag = match &nm {
                        Value::Str(n) => n.data[k].as_deref().filter(|s| !s.is_empty()).map(Sym::new),
                        _ => None,
                    };
                    Arg::new(tag, v.clone())
                })
                .collect()
        }
        Value::Pairlist(p) => p.items.clone(),
        _ => return Err(Flow::error("argument 'MoreArgs' of 'mapply' is not a list or pairlist")),
    };
    let dots = Value::sym("dots");
    let mut counters = vec![0usize; m];
    let mut out = Vec::with_capacity(longest);
    for _ in 0..longest {
        let mut fargs = Vec::with_capacity(m + more.len());
        for j in 0..m {
            counters[j] += 1;
            if counters[j] > lengths[j] {
                counters[j] = 1;
            }
            let inner =
                Value::lang(Value::Sym(s::BRACKET2), vec![Arg::pos(dots.clone()), Arg::pos(Value::int(j as i32 + 1))]);
            let outer =
                Value::lang(Value::Sym(s::BRACKET2), vec![Arg::pos(inner), Arg::pos(Value::int(counters[j] as i32))]);
            let tag = vnames.as_ref().and_then(|n| n[j].as_deref().filter(|s| !s.is_empty()).map(Sym::new));
            fargs.push(Arg::new(tag, outer));
        }
        fargs.extend(more.iter().cloned());
        let fcall = Value::lang(f.clone(), fargs);
        out.push(force_and_call(i, &fcall, m, rho)?);
    }
    more.clear();
    for j in 0..m {
        if counters[j] != lengths[j] {
            i.warning("longer argument not a multiple of length of shorter")?;
        }
    }
    Ok(Value::list(out))
}

fn is_factor(x: &Value) -> bool {
    matches!(x, Value::Int(_)) && x.inherits("factor")
}

/// R: apply.c:islistfactor
fn islistfactor(x: &Value) -> Lgl {
    match x {
        Value::List(l) | Value::Expr(l) => {
            let mut ans = NA_LOGICAL;
            for e in &l.data {
                let is_lf = islistfactor(e);
                if is_lf == FALSE {
                    return FALSE;
                } else if is_lf == TRUE {
                    ans = TRUE;
                }
            }
            ans
        }
        _ => is_factor(x) as Lgl,
    }
}

/// R: apply.c:do_islistfactor
/// R: apply.c:do_islistfactor
fn do_islistfactor(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "islistfactor")?;
    let x = arg(&args, 0);
    let recursive = as_logical(&arg(&args, 1));
    if recursive == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    let n = x.len();
    if n == 0 || !x.is_vector_list() {
        return Ok(Value::lgl(false));
    }
    let items = &x.as_list_vec().unwrap().data;
    if recursive == 0 {
        return Ok(Value::lgl(items.iter().all(is_factor)));
    }
    Ok(Value::lgl(islistfactor(&x) == TRUE))
}
