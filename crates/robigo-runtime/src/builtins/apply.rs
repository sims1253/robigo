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

/// Whether `X[[i]]`, evaluated in `rho` for a plain `X` (`xx`), needs no
/// code: `X` a list or an atomic vector that is not an object, and `[[`
/// base's (whose default method then takes the element: do_subset2).
fn plain_elements(i: &mut Interp, xx: &Value, rho: &Env) -> bool {
    !xx.is_object()
        && matches!(xx, Value::Real(_) | Value::Int(_) | Value::Lgl(_) | Value::Str(_) | Value::List(_))
        // a package ALTREP `X` is read by `X[[i]]`, through its class
        // (crate::altrep, #453), as in inline_fun
        && crate::altrep::lookup(xx).is_none()
        && i.resolves_to_base(s::BRACKET2, rho)
}

/// Whether `X` and `[[` in the `.Internal`'s frame are still what the
/// element reads assume (`xx`, base's). A `FUN` that runs R code may rebind
/// them (`assign("X", ..., parent.frame())`), and GNU R's next `X[[i]]`
/// reads the new binding; checked after every call that may have run code.
fn unchanged(rho: &Env, x: &Value, xx: &Value) -> bool {
    let Value::Sym(xsym) = x else { return false };
    let same = |v: &Value| match (v, xx) {
        (Value::Lgl(a), Value::Lgl(b)) => Rc::ptr_eq(a, b),
        _ => v.ptr_eq(xx),
    };
    let bound = {
        let frame = rho.0.frame().borrow();
        match frame.get(*xsym).map(|b| &b.value) {
            Some(Value::Promise(p)) => match &*p.value.borrow() {
                crate::value::PromiseVal::Boxed(v) => same(v),
                // A newly unboxed value cannot establish object identity;
                // let the general element-read path check it again.
                _ => false,
            },
            Some(v) => same(v),
            None => false,
        }
    };
    bound && !rho.local_may_bind_fun(s::BRACKET2)
}

/// `X[[k + 1]]` of a [`plain_elements`] `X`: the element, without the
/// vector's attributes.
fn plain_element(xx: &Value, k: usize) -> Value {
    match xx {
        Value::Real(v) => Value::real(v.data[k]),
        Value::Int(v) => Value::int(v.data[k]),
        Value::Lgl(v) => Value::lgl_raw(v.data[k]),
        Value::Str(v) => Value::strs(vec![v.data[k].clone()]),
        Value::List(v) => v.data[k].clone(),
        _ => unreachable!(),
    }
}

/// [`force_and_call`] of `FUN(X[[i]], ...)` (`e`) with the value of
/// `X[[i]]` already known (see [`plain_elements`]): a builtin `FUN` gets it
/// as its first argument, a closure a promise of `X[[i]]` forced to it, as
/// forcing the promise would have left it.
fn force_and_call_elem(i: &mut Interp, e: &Value, elem: Value, rho: &Env) -> RResult<Value> {
    let Value::Lang(l) = e else { return i.eval(e, rho) };
    let fun = match &l.fun {
        Value::Sym(sym) => i.find_fun(*sym, rho)?,
        other => i.eval(other, rho)?,
    };
    match &fun {
        Value::Builtin(b) if b.kind == BuiltinKind::Builtin => {
            let rest = i.eval_args(&l.args[1..], rho, e)?;
            let mut argv = Vec::with_capacity(rest.len() + 1);
            argv.push(Arg::pos(elem));
            argv.extend(rest);
            i.invoke_builtin(b, e, argv, rho)
        }
        Value::Closure(c) => {
            let x = Value::Promise(Rc::new(Promise::forced(l.args[0].value.clone(), elem)));
            let mut pargs = i.arg_vec(l.args.len());
            pargs.push(Arg::pos(x));
            pargs.extend(i.promise_args(&l.args[1..], rho)?);
            i.apply_closure(e, c, pargs, rho, None)
        }
        _ => force_and_call(i, e, 1, rho),
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
    args: Vec<crate::vm::RVal>,
    x_slot: usize,
}

enum FastApply {
    One(Rc<Closure>),
    Multi(FastClosure),
    /// A closure of one formal that may have native code (see
    /// `vm::call_native_rval`).
    Native(Rc<Closure>),
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
    // a package ALTREP `X` is read by `X[[i]]`, through its class
    // (crate::altrep): checked once per loop
    if crate::altrep::lookup(xx).is_some() {
        return Ok(None);
    }
    let Value::Lang(l) = fcall else { return Ok(None) };
    let Value::Sym(f) = &l.fun else { return Ok(None) };
    let Ok(fun_val) = i.find_fun(*f, rho) else { return Ok(None) };
    let Value::Closure(c) = fun_val else { return Ok(None) };
    let nf = c.formals.len();
    if nf == 0 {
        return Ok(None);
    }
    let dots = match rho.lookup(s::DOTS) {
        Some((Value::Dots(d), _)) => d,
        _ => Rc::new(Vec::new()),
    };
    // (Analysed now: a closure not known to be pure yet would take the
    // inline path, which declines it at every element.)
    if c.inline.get_or_init(|| crate::vm::inline_info(&c)).is_none() || !crate::vm::inlinable(&c) {
        let one = nf == 1 && dots.is_empty() && c.formals[0].tag != Some(s::DOTS);
        return Ok(one.then_some(FastApply::Native(c)));
    }
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
            args.push(crate::vm::RVal::V(Value::Null));
        } else {
            let val = match actual {
                Value::Promise(p) => i.force_promise(&p)?,
                v => v,
            };
            if val.is_missing_arg() {
                return Ok(None);
            }
            args.push(crate::vm::unbox(val));
        }
    }
    let Some(x_slot) = x_slot else { return Ok(None) };
    Ok(Some(FastApply::Multi(FastClosure { clo: c, args, x_slot })))
}

/// After a call of `FUN(X[[i]], ...)` was made: whether `FUN` and `X` are
/// still the closure `c` and the vector `xx`, as the native path assumes.
/// (A closure with native code can run any R code where it leaves native
/// code, rebinding them in `rho`, say with `assign(..., parent.frame())`.)
/// (Looked up in `rho`'s own frame, where lapply's formals and `FUN <-
/// match.fun(FUN)` are, without forcing anything: anything else is `false`.)
fn native_still_valid(fcall: &Value, rho: &Env, c: &Rc<Closure>, xx: &Value) -> bool {
    let Value::Lang(l) = fcall else { return false };
    let Value::Sym(f) = &l.fun else { return false };
    let Value::Lang(xi) = &l.args[0].value else { return false };
    let Value::Sym(x) = &xi.args[0].value else { return false };
    let value = |sym: Sym| match rho.get_local(sym)? {
        Value::Promise(p) => p.value(),
        v => Some(v),
    };
    matches!(value(*f), Some(Value::Closure(c2)) if Rc::ptr_eq(&c2, c)) && value(*x).is_some_and(|v| v.same_object(xx))
}

/// `xx[[k + 1]]` of a plain vector or list as an `RVal` (see [`inline_fun`]).
#[inline]
fn element_rval(xx: &Value, k: usize) -> crate::vm::RVal {
    match xx {
        Value::Real(v) => crate::vm::RVal::D(v.data[k]),
        Value::Int(v) => crate::vm::RVal::I(v.data[k]),
        Value::Lgl(v) => crate::vm::RVal::L(v.data[k]),
        Value::Str(v) => crate::vm::RVal::V(Value::strs(vec![v.data[k].clone()])),
        Value::List(v) => crate::vm::unbox_ref(&v.data[k]),
        _ => unreachable!(),
    }
}

/// Bind the loop index `i` to `k + 1`, writing into the index object it
/// has unless that has been captured (GNU R allocates it once and writes
/// it in place; the index is its own object, never a shared small integer).
// R: apply.c:do_lapply (`INTEGER(ind)[0] = i + 1`, duplicated when
// `MAYBE_SHARED(ind)`)
fn set_index(rho: &Env, isym: Sym, k: usize) {
    let index = k as i32 + 1;
    {
        let mut frame = rho.0.frame().borrow_mut();
        if let Some(b) = frame.get_mut(isym)
            && b.flags == 0
            && let Value::Int(rc) = &mut b.value
            && let Some(v) = Rc::get_mut(rc)
            && v.data.len() == 1
            && v.attrs.is_none()
        {
            v.data[0] = index;
            return;
        }
    }
    rho.set_local(isym, Value::int_fresh(index));
}

/// R_execClosure steps into `FUN` when `rho`'s browser was left with `s`
/// (`Interp::steps_into`); a call made inline would not, so then there is
/// no fast path, for this call and the rest of the loop's (the browser may
/// rebind `FUN`). Checked before the loop and after each call that ran R
/// code: only R code reaches a browser prompt, and a closure run inline or
/// as native code runs none (see `crate::jit::closure`).
// R: eval.c:R_execClosure
#[inline(always)]
fn unless_stepping<T>(i: &Interp, rho: &Env, fast: Option<T>) -> Option<T> {
    if i.steps_into(rho) { None } else { fast }
}

/// `FUN(X[[i]], ...)` for element `k`, inline when `fast` allows.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn call_fun(
    i: &mut Interp,
    fast: &mut Option<FastApply>,
    xx: &Value,
    k: usize,
    fcall: &Value,
    isym: Sym,
    rho: &Env,
    plain: &mut bool,
    x: &Value,
) -> RResult<Value> {
    match fast {
        Some(FastApply::One(c)) => {
            let rk = element_rval(xx, k);
            if let Some(v) = crate::vm::call_inline_rval(i, c, &[rk]) {
                return Ok(v.into_value());
            }
        }
        Some(FastApply::Multi(fc)) => {
            fc.args[fc.x_slot] = element_rval(xx, k);
            if let Some(v) = crate::vm::call_inline_rval(i, &fc.clo, &fc.args) {
                return Ok(v.into_value());
            }
        }
        Some(FastApply::Native(c)) => {
            if let Some(v) = crate::vm::call_native_rval(i, c, &[element_rval(xx, k)]) {
                return Ok(v.into_value());
            }
            set_index(rho, isym, k);
            let v = force_and_call(i, fcall, 1, rho);
            *fast = unless_stepping(i, rho, fast.take());
            let v = v?;
            if let Some(FastApply::Native(c)) = fast
                && (crate::vm::native_never(c) || !native_still_valid(fcall, rho, c, xx))
            {
                *fast = None;
            }
            // (The call may have rebound `X`: the plain element reads of the
            // next elements assume it did not, as below.)
            if *plain && !unchanged(rho, x, xx) {
                *plain = false;
                *fast = None;
            }
            return Ok(v);
        }
        None => {}
    }
    set_index(rho, isym, k);
    let r = if *plain {
        force_and_call_elem(i, fcall, plain_element(xx, k), rho)
    } else {
        force_and_call(i, fcall, 1, rho)
    };
    if fast.is_some() {
        *fast = unless_stepping(i, rho, fast.take());
    }
    if (*plain || fast.is_some()) && !unchanged(rho, x, xx) {
        *plain = false;
        *fast = None;
    }
    r
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
    let fcall = fun_x_i_call(fun, x.clone());
    let isym = Sym::new("i");
    let mut out = Vec::with_capacity(n);
    // (Checked before inline_fun, which forces `...`, and after it: resolving
    // FUN can run R and reach a browser.)
    let fast = if n > 0 && !i.steps_into(rho) { inline_fun(i, &fcall, rho, &xx)? } else { None };
    let mut fast = unless_stepping(i, rho, fast);
    let mut plain = n > 0 && plain_elements(i, &xx, rho);
    for k in 0..n {
        out.push(call_fun(i, &mut fast, &xx, k, &fcall, isym, rho, &mut plain, &x)?);
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
    let fcall = fun_x_i_call(fun, x.clone());
    let isym = Sym::new("i");
    let mut parts: Vec<Value> = Vec::with_capacity(n);
    // (Checked before inline_fun, which forces `...`, and after it: resolving
    // FUN can run R and reach a browser.)
    let fast = if n > 0 && !i.steps_into(rho) { inline_fun(i, &fcall, rho, &xx)? } else { None };
    let mut fast = unless_stepping(i, rho, fast);

    if common_len == 1 && !array_value {
        match fast {
            Some(FastApply::One(ref c)) => {
                if common_type == VType::Real {
                    let mut out_data = Vec::with_capacity(n);
                    for k in 0..n {
                        let rk = element_rval(&xx, k);
                        match crate::vm::call_inline_rval(i, c, &[rk]) {
                            Some(crate::vm::RVal::D(d)) => out_data.push(d),
                            Some(crate::vm::RVal::I(crate::value::NA_INTEGER)) => out_data.push(crate::value::NA_REAL),
                            Some(crate::vm::RVal::I(v)) => out_data.push(v as f64),
                            Some(crate::vm::RVal::L(l)) if l != NA_LOGICAL => out_data.push(l as f64),
                            _ => break,
                        }
                    }
                    if out_data.len() == n {
                        let mut ans = Value::reals(out_data);
                        if use_names && !names.is_null() {
                            ans = set_attrib(ans, s::NAMES, names)?;
                        }
                        return Ok(ans);
                    }
                    for v in out_data {
                        parts.push(Value::real(v));
                    }
                } else if common_type == VType::Int {
                    let mut out_data = Vec::with_capacity(n);
                    for k in 0..n {
                        let rk = element_rval(&xx, k);
                        match crate::vm::call_inline_rval(i, c, &[rk]) {
                            Some(crate::vm::RVal::I(v)) => out_data.push(v),
                            Some(crate::vm::RVal::L(l)) => out_data.push(l),
                            _ => break,
                        }
                    }
                    if out_data.len() == n {
                        let mut ans = Value::ints(out_data);
                        if use_names && !names.is_null() {
                            ans = set_attrib(ans, s::NAMES, names)?;
                        }
                        return Ok(ans);
                    }
                    for v in out_data {
                        parts.push(Value::int(v));
                    }
                }
            }
            Some(FastApply::Multi(ref mut fc)) => {
                if common_type == VType::Real {
                    let mut out_data = Vec::with_capacity(n);
                    for k in 0..n {
                        fc.args[fc.x_slot] = element_rval(&xx, k);
                        match crate::vm::call_inline_rval(i, &fc.clo, &fc.args) {
                            Some(crate::vm::RVal::D(d)) => out_data.push(d),
                            Some(crate::vm::RVal::I(crate::value::NA_INTEGER)) => out_data.push(crate::value::NA_REAL),
                            Some(crate::vm::RVal::I(v)) => out_data.push(v as f64),
                            Some(crate::vm::RVal::L(l)) if l != NA_LOGICAL => out_data.push(l as f64),
                            _ => break,
                        }
                    }
                    if out_data.len() == n {
                        let mut ans = Value::reals(out_data);
                        if use_names && !names.is_null() {
                            ans = set_attrib(ans, s::NAMES, names)?;
                        }
                        return Ok(ans);
                    }
                    for v in out_data {
                        parts.push(Value::real(v));
                    }
                } else if common_type == VType::Int {
                    let mut out_data = Vec::with_capacity(n);
                    for k in 0..n {
                        fc.args[fc.x_slot] = element_rval(&xx, k);
                        match crate::vm::call_inline_rval(i, &fc.clo, &fc.args) {
                            Some(crate::vm::RVal::I(v)) => out_data.push(v),
                            Some(crate::vm::RVal::L(l)) => out_data.push(l),
                            _ => break,
                        }
                    }
                    if out_data.len() == n {
                        let mut ans = Value::ints(out_data);
                        if use_names && !names.is_null() {
                            ans = set_attrib(ans, s::NAMES, names)?;
                        }
                        return Ok(ans);
                    }
                    for v in out_data {
                        parts.push(Value::int(v));
                    }
                }
            }
            Some(FastApply::Native(_)) | None => {}
        }
    }

    let start_k = parts.len();
    let mut plain = start_k < n && plain_elements(i, &xx, rho);
    for k in start_k..n {
        let mut val = call_fun(i, &mut fast, &xx, k, &fcall, isym, rho, &mut plain, &x)?;
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
            vals.push(i.get_active_value(&v)?);
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
        set_index(rho, isym, k);
        out.push(force_and_call(i, &fcall, 1, rho)?);
    }
    let mut ans = Value::list(out);
    if use_nms {
        ans.set_attr_raw(s::NAMES, Value::strs(syms.iter().map(|s| Some(s.name())).collect()));
    }
    Ok(ans)
}

/// An inlinable pure `FUN` for mapply with the argument slots matched
/// once (see [`inline_fun`]): `vary[j]` is the formal slot fed from
/// varying argument `j`; every other slot holds its invariant `MoreArgs`
/// value.
struct MFastClosure {
    clo: Rc<Closure>,
    args: Vec<crate::vm::RVal>,
    vary: Vec<usize>,
}

/// Match `FUN(dots[[1]][[k]], ..., <MoreArgs>)` once so each element can
/// run `FUN` inline (`vm::call_inline_rval`). The `inner[j]` calls stand
/// in for the varying arguments during matching and are recognized by
/// pointer, so no `MoreArgs` value can collide with them; a formal that
/// would take its default gives up the fast path.
fn mapply_inline(
    i: &mut Interp,
    f: &Value,
    inner: &[Value],
    tags: &[Option<Sym>],
    more: &[Arg],
) -> RResult<Option<MFastClosure>> {
    let Value::Closure(c) = f else { return Ok(None) };
    if !crate::vm::inlinable(c) {
        return Ok(None);
    }
    let m = inner.len();
    let mut supplied: Vec<Arg> = Vec::with_capacity(m + more.len());
    for (j, e) in inner.iter().enumerate() {
        supplied.push(Arg::new(tags[j], e.clone()));
    }
    supplied.extend(more.iter().cloned());
    let dummy = Value::lang(f.clone(), vec![]);
    let Ok(matched) = i.match_args(&c.formals, &supplied, &dummy) else {
        return Ok(None);
    };
    if matched.len() != c.formals.len() {
        return Ok(None);
    }
    let mut args: Vec<crate::vm::RVal> = Vec::with_capacity(matched.len());
    let mut vary = vec![usize::MAX; m];
    for v in matched.into_iter() {
        let Some(v) = v else { return Ok(None) };
        if v.is_missing_arg() {
            return Ok(None);
        }
        let j = inner.iter().position(|e| matches!((e, &v), (Value::Lang(a), Value::Lang(b)) if Rc::ptr_eq(a, b)));
        match j {
            Some(j) => {
                vary[j] = args.len();
                args.push(crate::vm::RVal::V(Value::Null));
            }
            None => args.push(crate::vm::unbox(v)),
        }
    }
    if vary.contains(&usize::MAX) {
        return Ok(None);
    }
    Ok(Some(MFastClosure { clo: c.clone(), args, vary }))
}

/// [`force_and_call`] of mapply's `f(dots[[1L]][[k1]], ...)` call where
/// every `dots[[j]]` is a plain vector (no `[[` dispatch): the elements are
/// read directly and passed already forced, while the call keeps GNU R's
/// `dots[[j]][[k]]` expressions (what `sys.call()`, `match.call()`,
/// `substitute()` and error calls show).
fn force_and_call_elems(i: &mut Interp, e: &Value, vitems: &[Value], counters: &[usize], rho: &Env) -> RResult<Value> {
    let Value::Lang(l) = e else { return i.eval(e, rho) };
    let m = vitems.len();
    let elem = |j: usize| element_rval(&vitems[j], counters[j] - 1);
    let fun = match &l.fun {
        Value::Sym(sym) => i.find_fun(*sym, rho)?,
        other => i.eval(other, rho)?,
    };
    match &fun {
        Value::Builtin(b) if b.kind == BuiltinKind::Builtin => {
            let rest = i.eval_args(&l.args[m..], rho, e)?;
            let mut argv = Vec::with_capacity(m + rest.len());
            for (j, a) in l.args[..m].iter().enumerate() {
                argv.push(Arg::new(a.tag, elem(j).into_value()));
            }
            argv.extend(rest);
            i.invoke_builtin(b, e, argv, rho)
        }
        Value::Closure(c) => {
            let mut pargs = i.arg_vec(l.args.len());
            for (j, a) in l.args[..m].iter().enumerate() {
                let k = counters[j] - 1;
                let expr = a.value.clone();
                let p = match &vitems[j] {
                    Value::Real(v) => Promise::forced_real(expr, v.data[k]),
                    Value::Int(v) => Promise::forced_int(expr, v.data[k]),
                    Value::Lgl(v) => Promise::forced_lgl(expr, v.data[k]),
                    x => Promise::forced(expr, element_rval(x, k).into_value()),
                };
                pargs.push(Arg::new(a.tag, Value::Promise(Rc::new(p))));
            }
            pargs.extend(i.promise_args(&l.args[m..], rho)?);
            i.apply_closure(e, c, pargs, rho, None)
        }
        _ => force_and_call(i, e, m, rho),
    }
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
    // R: mapply.c:do_mapply builds `f(dots[[1]][[k1]], ...)` once and
    // mutates the index cells per iteration. Where no varying element is
    // an object (so `[[` cannot dispatch), the element is read directly
    // and passed as an already-forced promise carrying the same
    // `dots[[j]][[k]]` expression (what `substitute` in `FUN` sees), and
    // an inlinable pure `FUN` runs without a call at all (see
    // [`inline_fun`]).
    let direct = vitems.iter().all(|x| {
        matches!(x, Value::Real(_) | Value::Int(_) | Value::Lgl(_) | Value::Str(_) | Value::List(_)) && !x.is_object()
    });
    let tags: Vec<Option<Sym>> =
        (0..m).map(|j| vnames.as_ref().and_then(|n| n[j].as_deref().filter(|s| !s.is_empty()).map(Sym::new))).collect();
    let inner: Vec<Value> = (0..m)
        .map(|j| Value::lang(Value::Sym(s::BRACKET2), vec![Arg::pos(dots.clone()), Arg::pos(Value::int(j as i32 + 1))]))
        .collect();
    // (Stepping into `FUN`: see unless_stepping.)
    let mfast =
        if direct && longest > 0 && !i.steps_into(rho) { mapply_inline(i, &f, &inner, &tags, &more)? } else { None };
    let mut mfast = unless_stepping(i, rho, mfast);
    let mut counters = vec![0usize; m];
    let mut out = Vec::with_capacity(longest);
    for _ in 0..longest {
        for j in 0..m {
            counters[j] += 1;
            if counters[j] > lengths[j] {
                counters[j] = 1;
            }
        }
        if let Some(mf) = &mut mfast {
            for j in 0..m {
                mf.args[mf.vary[j]] = element_rval(&vitems[j], counters[j] - 1);
            }
            if let Some(v) = crate::vm::call_inline_rval(i, &mf.clo, &mf.args) {
                out.push(v.into_value());
                continue;
            }
        }
        let mut fargs = Vec::with_capacity(m + more.len());
        for j in 0..m {
            let outer = Value::lang(
                Value::Sym(s::BRACKET2),
                vec![Arg::pos(inner[j].clone()), Arg::pos(Value::int(counters[j] as i32))],
            );
            fargs.push(Arg::new(tags[j], outer));
        }
        fargs.extend(more.iter().cloned());
        let fcall = Value::lang(f.clone(), fargs);
        let r = if direct {
            force_and_call_elems(i, &fcall, &vitems, &counters, rho)
        } else {
            force_and_call(i, &fcall, m, rho)
        };
        if mfast.is_some() {
            mfast = unless_stepping(i, rho, mfast.take());
        }
        out.push(r?);
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
