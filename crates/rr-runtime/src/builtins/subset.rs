//! Subsetting: `[`, `[[`, `$`, `@`, `.subset` and `.subset2` (GNU R's
//! subset.c, plus `DropDims` from array.c and `do_AT` from attrib.c).
//!
//! `[`, `[[` and `$` are specials: they evaluate their first argument and
//! dispatch on it if it is an object (`R_DispatchOrEvalSP`); otherwise the
//! remaining arguments are evaluated keeping missing ones, which the
//! default code treats as "everything" (`x[, 1]`).

use std::rc::Rc;

use super::subscript::*;
use super::{check_arity, prim, special};
use crate::attrib::{get_attrib, set_attrib};
use crate::env::{Env, BIND_ACTIVE};
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim(".subset", do_dot_subset, -1),
    prim(".subset2", do_dot_subset2, -1),
    special("[", do_subset, -1),
    special("[[", do_subset2, -1),
    special("$", do_subset3, 2),
    special("@", do_at, 2),
];

// ----------------------------------------------------------------------
// argument evaluation and dispatch

/// R's `R_isMissing`: is `sym` (in `env`) a missing argument, possibly
/// through a chain of promises? Unlike for `missing()`, a formal that uses
/// its default value is not missing here (R marks it `MISSING = 2`).
// R: envir.c:R_isMissing
fn r_is_missing(interp: &Interp, sym: Sym, env: &Env, depth: usize) -> bool {
    if sym.is_empty() {
        return true;
    }
    if depth > 1000 {
        return true;
    }
    if env.ptr_eq(&interp.base) {
        return false;
    }
    let (lookup, ddv) = match sym.ddval() {
        Some(i) => (s::DOTS, Some(i)),
        None => (sym, None),
    };
    let binding = env.0.frame.borrow().get(lookup).map(|b| (b.value.clone(), b.flags));
    let Some((mut v, flags)) = binding else { return false };
    if let Some(i) = ddv {
        match &v {
            Value::Dots(d) if d.len() >= i => v = d[i - 1].value.clone(),
            _ => return true,
        }
    }
    if v.is_missing_arg() {
        return true;
    }
    if flags & BIND_ACTIVE != 0 {
        return false;
    }
    if let Value::Promise(p) = &v {
        let mut p = p.clone();
        while let Value::Promise(q) = &p.expr {
            let q = q.clone();
            p = q;
        }
        if p.value().is_none() {
            if let Value::Sym(s2) = &p.expr {
                let penv = p.env.borrow().clone();
                if let Some(penv) = penv {
                    return r_is_missing(interp, *s2, &penv, depth + 1);
                }
            }
        }
    }
    false
}

/// R's `evalListKeepMissing`: evaluate arguments, expanding `...` and
/// keeping missing arguments as the missing marker.
// R: eval.c:evalListKeepMissing
pub(crate) fn eval_keep_missing(interp: &mut Interp, args: &[Arg], env: &Env) -> RResult<Vec<Arg>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        match &a.value {
            Value::Sym(sym) if *sym == s::DOTS => match env.lookup(s::DOTS) {
                Some((Value::Dots(d), _)) => {
                    for item in d.iter() {
                        if item.value.is_missing_arg() {
                            out.push(Arg::new(item.tag, Value::missing_arg()));
                        } else {
                            let v = interp.eval(&item.value, env)?;
                            out.push(Arg::new(item.tag, v));
                        }
                    }
                }
                Some((v, _)) if v.is_null() || v.is_missing_arg() => {}
                _ => return Err(Flow::error("'...' used in an incorrect context")),
            },
            v if v.is_missing_arg() => out.push(Arg::new(a.tag, Value::missing_arg())),
            Value::Sym(sym) if r_is_missing(interp, *sym, env, 0) => {
                out.push(Arg::new(a.tag, Value::missing_arg()))
            }
            v => {
                let r = interp.eval(v, env)?;
                out.push(Arg::new(a.tag, r));
            }
        }
    }
    Ok(out)
}

/// Replace `...` arguments by the elements they are bound to.
fn expand_dots(args: Vec<Arg>, env: &Env) -> RResult<Vec<Arg>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if a.value.as_sym() == Some(s::DOTS) {
            match env.lookup(s::DOTS) {
                Some((Value::Dots(d), _)) => out.extend(d.iter().cloned()),
                Some((v, _)) if v.is_null() || v.is_missing_arg() => {}
                _ => return Err(Flow::error("'...' used in an incorrect context")),
            }
        } else {
            out.push(a);
        }
    }
    Ok(out)
}

/// DispatchOrEval does not dispatch from `foo.default` calls.
pub(crate) fn call_is_default(call: &Value) -> bool {
    match call {
        Value::Lang(l) => l.fun_sym().is_some_and(|f| {
            let n = f.name();
            n.rfind('.').is_some_and(|p| &n[p..] == ".default")
        }),
        _ => false,
    }
}

pub(crate) enum Dispatched {
    /// A method was applied; its result.
    Done(Value),
    /// No method: the evaluated arguments.
    Args(Vec<Arg>),
}

/// R's `R_DispatchOrEvalSP` for `[`, `[[`, `$` and their replacement
/// forms. The object is taken out of `args` when it is a value rather
/// than an expression, so that replacement functions called from a
/// complex assignment hold the only reference and can modify in place.
// R: subset.c:R_DispatchOrEvalSP
pub(crate) fn dispatch_or_eval_sp(
    interp: &mut Interp,
    call: &Value,
    generic: &str,
    mut args: Vec<Arg>,
    env: &Env,
) -> RResult<Dispatched> {
    if args.is_empty() {
        return Ok(Dispatched::Args(args));
    }
    if args[0].value.as_sym() == Some(s::DOTS) {
        args = expand_dots(args, env)?;
        if args.is_empty() {
            return Ok(Dispatched::Args(args));
        }
    }
    let first = std::mem::replace(&mut args[0].value, Value::Null);
    let (x, expr) = match first {
        // An argument evaluated already (by compiled code): dispatch sees
        // its expression, as substitute() would through the promise.
        Value::Promise(p) if p.value().is_some() => (p.value().unwrap(), Some(p.expr.clone())),
        e @ (Value::Sym(_) | Value::Lang(_) | Value::Promise(_)) => {
            let v = interp.eval(&e, env)?;
            (v, Some(e))
        }
        v => (v, None),
    };
    let tag0 = args[0].tag;
    if !x.is_object() || call_is_default(call) {
        let mut out = Vec::with_capacity(args.len());
        out.push(Arg::new(tag0, x));
        out.extend(eval_keep_missing(interp, &args[1..], env)?);
        return Ok(Dispatched::Args(out));
    }
    let expr = expr.unwrap_or_else(|| x.clone());
    let mut pargs = Vec::with_capacity(args.len());
    pargs.push(Arg::new(tag0, Value::Promise(Rc::new(Promise::forced(expr, x.clone())))));
    pargs.extend(interp.promise_args(&args[1..], env)?);
    if let Some(ans) = interp.dispatch_with_temp_context(generic, &x, call, pargs, env)? {
        return Ok(Dispatched::Done(ans));
    }
    let mut out = Vec::with_capacity(args.len());
    out.push(Arg::new(tag0, x));
    out.extend(eval_keep_missing(interp, &args[1..], env)?);
    Ok(Dispatched::Args(out))
}

/// rr passes language values to specials wrapped in `quote()`; R would
/// see the value itself.
fn unwrap_quote(v: Value) -> Value {
    if let Value::Lang(l) = &v {
        if l.fun_sym() == Some(s::QUOTE) && l.args.len() == 1 {
            return l.args[0].value.clone();
        }
    }
    v
}

/// R's `fixSubset3Args`: the name argument of `$`, `$<-` and `@` as a
/// string (and the symbol, if it was one).
// R: subset.c:fixSubset3Args
pub(crate) fn fix_subset3_arg(
    interp: &mut Interp,
    call: &Value,
    nlist: &Value,
    env: &Env,
) -> RResult<(RStr, Option<Sym>)> {
    let nlist = match nlist {
        Value::Promise(_) => interp.eval(nlist, env)?,
        v => v.clone(),
    };
    let nlist = unwrap_quote(nlist);
    match &nlist {
        Value::Sym(sym) => Ok((Some(sym.name()), Some(*sym))),
        Value::Str(v) => {
            if v.data.len() != 1 {
                return Err(Flow::error("invalid subscript length"));
            }
            Ok((v.data[0].clone(), None))
        }
        _ => Err(Flow::error_call(call.clone(), format!("invalid subscript type '{}'", nlist.type_name()))),
    }
}

/// R's `ExtractArg`: remove the first argument tagged `sym`, returning its
/// value (`NULL` if there is none). As in R, an argument at the head of the
/// list is not removed (only `ExtractArg`'s local copy of the list head
/// moves), so the first argument always remains the object.
// R: subset.c:ExtractArg
fn extract_arg(args: &mut Vec<Arg>, sym: Sym) -> Value {
    match args.iter().position(|a| a.tag == Some(sym)) {
        Some(0) => args[0].value.clone(),
        Some(i) => args.remove(i).value,
        None => Value::Null,
    }
}

/// `drop=` (default TRUE; NA means TRUE).
fn extract_drop_arg(args: &mut Vec<Arg>) -> bool {
    let d = crate::coerce::as_logical(&extract_arg(args, s::DROP));
    d != FALSE
}

/// `exact=`: 1 exact, 0 partial, -1 partial with a warning (NA).
fn extract_exact_arg(args: &mut Vec<Arg>) -> i32 {
    let v = extract_arg(args, s::EXACT);
    if v.is_null() {
        return 1;
    }
    match crate::coerce::as_logical(&v) {
        NA_LOGICAL => -1,
        x => x,
    }
}

/// `R_findVarInFrame` for `[[` and `$` on environments: forces promises
/// and calls active bindings.
fn find_var_in_frame(interp: &mut Interp, e: &Env, sym: Sym) -> RResult<Option<Value>> {
    let b = e.0.frame.borrow().get(sym).map(|b| (b.value.clone(), b.flags));
    match b {
        None => Ok(None),
        Some((f, flags)) if flags & BIND_ACTIVE != 0 => {
            let global = interp.global.clone();
            interp.call_value(&f, vec![], &global).map(Some)
        }
        Some((v, _)) => interp.force(v).map(Some),
    }
}

// ----------------------------------------------------------------------
// extraction

fn extract<T: Elt>(src: &[T], idx: &[i64]) -> Vec<T> {
    let nx = src.len() as i64;
    idx.iter().map(|&i| if i >= 1 && i <= nx { src[(i - 1) as usize].clone() } else { T::NA }).collect()
}

/// R's `ExtractSubset`: elements at 1-based positions, NA (or `NULL`)
/// where a position is NA or out of range. No attributes.
// R: subset.c:ExtractSubset
pub fn extract_subset(x: &Value, idx: &[i64], call: ECall) -> RResult<Value> {
    Ok(match x {
        Value::Null => Value::Null,
        Value::Lgl(v) => Value::lgls(extract(&v.data, idx)),
        Value::Int(v) => Value::ints(extract(&v.data, idx)),
        Value::Real(v) => Value::reals(extract(&v.data, idx)),
        Value::Cplx(v) => Value::cplxs(extract(&v.data, idx)),
        Value::Str(v) => Value::strs(extract(&v.data, idx)),
        Value::Raw(v) => Value::raws(extract(&v.data, idx)),
        Value::List(v) => Value::list(extract(&v.data, idx)),
        Value::Expr(v) => Value::expr_vec(extract(&v.data, idx)),
        _ => return Err(not_subsettable(x, call)),
    })
}

/// A pairlist or call as a list for `[` (names from the tags).
fn pairlike_as_list(x: &Value) -> Value {
    let mut ax = Value::list(crate::coerce::to_list(x));
    let dim = x.get_attr(s::DIM);
    if let Some(d) = dim.filter(|d| d.len() > 1) {
        ax.set_attr_raw(s::DIM, d);
        if let Some(dn) = x.get_attr(s::DIMNAMES) {
            ax.set_attr_raw(s::DIMNAMES, dn);
        }
    } else if let Some(n) = x.names() {
        ax.set_attr_raw(s::NAMES, n);
    }
    ax
}

/// Names as call/pairlist tags.
pub(crate) fn tag_of(n: Option<&RStr>) -> Option<Sym> {
    match n {
        Some(Some(s)) if !s.is_empty() => Some(Sym::new(s)),
        Some(None) => Some(Sym::new("NA")),
        _ => None,
    }
}

/// The result of `[` on a call, turned back into a call.
fn list_as_lang(ans: Value) -> Value {
    let Value::List(l) = &ans else { return ans };
    if l.data.is_empty() {
        return Value::Null;
    }
    let names = match get_attrib(&ans, s::NAMES) {
        Value::Str(n) => Some(n.data.clone()),
        _ => None,
    };
    let items: Vec<Arg> = l
        .data
        .iter()
        .enumerate()
        .map(|(i, v)| Arg::new(names.as_ref().and_then(|n| tag_of(n.get(i))), v.clone()))
        .collect();
    Value::Lang(Rc::new(Lang::from_items(items, None)))
}

/// R's `VectorSubset`: `x[s]` with a single subscript.
// R: subset.c:VectorSubset
fn vector_subset(x: &Value, s: &Value, call: &Value) -> RResult<Value> {
    if s.is_missing_arg() {
        return Ok(x.clone());
    }
    let mat = matrix_subscript(x, s, Some(call), Some(call))?;
    let s = mat.as_ref().unwrap_or(s);
    let mut stretch = 1;
    let indx = make_subscript(x, s, &mut stretch, Some(call))?;
    let mut result = extract_subset(x, &indx.idx, Some(call))?;
    if !result.is_null() {
        let mut names = get_attrib(x, s::NAMES);
        if names.is_null() && is_array(x) {
            if let Value::List(dn) = get_attrib(x, s::DIMNAMES) {
                if dn.data.len() == 1 {
                    names = dn.data[0].clone();
                }
            }
        }
        if !names.is_null() {
            let nn = extract_subset(&names, &indx.idx, Some(call))?;
            result.set_attr_raw(s::NAMES, nn);
        }
        if let Some(sr @ Value::List(_)) = x.get_attr(s::SRCREF) {
            let nsr = extract_subset(&sr, &indx.idx, Some(call))?;
            result.set_attr_raw(s::SRCREF, nsr);
        }
    }
    Ok(result)
}

fn dim_names_attr(x: &Value) -> Option<Value> {
    x.get_attr(s::DIM).and_then(|d| d.get_attr(s::NAMES))
}

/// R's `MatrixSubset`.
// R: subset.c:MatrixSubset
fn matrix_subset(interp: &mut Interp, x: &Value, s0: &Value, s1: &Value, call: &Value, drop: bool) -> RResult<Value> {
    let dim = dim_of(x).unwrap_or_default();
    let nr = dim[0] as i64;
    let nc = dim[1] as i64;
    let sr = array_subscript(interp, 0, s0, &dim, x, Some(call))?;
    let sc = array_subscript(interp, 1, s1, &dim, x, Some(call))?;
    let mut flat = Vec::with_capacity(sr.len() * sc.len());
    for &jj in &sc {
        if jj != NA_IDX && (jj < 1 || jj > nc) {
            return Err(out_of_bounds(Some(call), None));
        }
        for &ii in &sr {
            if ii != NA_IDX && (ii < 1 || ii > nr) {
                return Err(out_of_bounds(Some(call), None));
            }
            flat.push(if ii == NA_IDX || jj == NA_IDX { NA_IDX } else { ii + (jj - 1) * nr });
        }
    }
    let mut result = match x {
        x if x.is_vector() => extract_subset(x, &flat, Some(call))?,
        _ => return Err(Flow::error_call(call.clone(), "matrix subscripting not handled for this type")),
    };
    let mut d = Value::ints(vec![sr.len() as i32, sc.len() as i32]);
    if let Some(n) = dim_names_attr(x) {
        d.set_attr_raw(s::NAMES, n);
    }
    result.set_attr_raw(s::DIM, d);
    let dimnames = get_attrib(x, s::DIMNAMES);
    if !dimnames.is_null() {
        let mut nd = Value::list(vec![
            extract_subset(&generic_elt(&dimnames, 0), &sr, Some(call))?,
            extract_subset(&generic_elt(&dimnames, 1), &sc, Some(call))?,
        ]);
        if let Some(n) = dimnames.get_attr(s::NAMES) {
            nd.set_attr_raw(s::NAMES, n);
        }
        result = set_attrib(result, s::DIMNAMES, nd)?;
    }
    if drop {
        result = drop_dims(result)?;
    }
    Ok(result)
}

/// R's `ArraySubset`.
// R: subset.c:ArraySubset
fn array_subset(interp: &mut Interp, x: &Value, subs: &[Value], call: &Value, drop: bool) -> RResult<Value> {
    let xdims = dim_of(x).unwrap_or_default();
    let k = xdims.len();
    let mut sub_idx: Vec<Vec<i64>> = Vec::with_capacity(k);
    let mut n: usize = 1;
    for i in 0..k {
        let si = array_subscript(interp, i, &subs[i], &xdims, x, Some(call))?;
        n *= si.len();
        sub_idx.push(si);
    }
    let bound: Vec<usize> = sub_idx.iter().map(|v| v.len()).collect();
    let mut offset = vec![1i64; k];
    for i in 1..k {
        offset[i] = offset[i - 1] * xdims[i - 1] as i64;
    }
    for i in 0..k {
        for &jj in &sub_idx[i] {
            if jj > xdims[i] as i64 || (jj < 1 && jj != NA_IDX) {
                return Err(out_of_bounds(Some(call), None));
            }
        }
    }
    let mut flat = Vec::with_capacity(n);
    let mut indx = vec![0usize; k];
    for _ in 0..n {
        let mut ii: i64 = 0;
        let mut na = false;
        for j in 0..k {
            let jj = sub_idx[j][indx[j]];
            if jj == NA_IDX {
                na = true;
                break;
            }
            ii += (jj - 1) * offset[j];
        }
        flat.push(if na { NA_IDX } else { ii + 1 });
        if n > 1 {
            let mut j = 0;
            loop {
                indx[j] += 1;
                if indx[j] < bound[j] {
                    break;
                }
                indx[j] = 0;
                j = (j + 1) % k;
            }
        }
    }
    let mut result = match x {
        x if x.is_vector() => extract_subset(x, &flat, Some(call))?,
        _ => return Err(Flow::error_call(call.clone(), "array subscripting not handled for this type")),
    };
    let mut nd = Value::ints(bound.iter().map(|&b| b as i32).collect());
    if let Some(n) = dim_names_attr(x) {
        nd.set_attr_raw(s::NAMES, n);
    }
    result.set_attr_raw(s::DIM, nd);
    let dimnames = get_attrib(x, s::DIMNAMES);
    if !dimnames.is_null() {
        let mut items = Vec::with_capacity(k);
        for i in 0..k {
            if bound[i] > 0 {
                items.push(extract_subset(&generic_elt(&dimnames, i), &sub_idx[i], Some(call))?);
            } else {
                items.push(Value::Null);
            }
        }
        let mut ndn = Value::list(items);
        if let Some(n) = dimnames.get_attr(s::NAMES) {
            ndn.set_attr_raw(s::NAMES, n);
        }
        result = set_attrib(result, s::DIMNAMES, ndn)?;
    }
    if drop {
        result = drop_dims(result)?;
    }
    Ok(result)
}

/// R's `DropDims`: drop extents of length one.
// R: array.c:DropDims
pub fn drop_dims(mut x: Value) -> RResult<Value> {
    let Some(dim) = dim_of(&x) else { return Ok(x) };
    let ndims = dim.len();
    let n = dim.iter().filter(|&&d| d != 1).count();
    if n == ndims {
        return Ok(x);
    }
    let dimnames = get_attrib(&x, s::DIMNAMES);
    if n <= 1 {
        let mut newnames = Value::Null;
        if !dimnames.is_null() {
            if x.len() != 1 {
                for (i, &d) in dim.iter().enumerate() {
                    if d != 1 {
                        newnames = generic_elt(&dimnames, i);
                        break;
                    }
                }
            } else {
                let cnt = (0..ndims).filter(|&i| !generic_elt(&dimnames, i).is_null()).count();
                if cnt == 1 {
                    for i in 0..ndims {
                        newnames = generic_elt(&dimnames, i);
                        if !newnames.is_null() {
                            break;
                        }
                    }
                }
            }
        }
        x.set_attr_raw(s::DIMNAMES, Value::Null);
        x.set_attr_raw(s::DIM, Value::Null);
        x = set_attrib(x, s::NAMES, newnames)?;
    } else {
        let dnn = dimnames.get_attr(s::NAMES);
        let keep: Vec<usize> = (0..ndims).filter(|&i| dim[i] != 1).collect();
        let mut newdims = Value::ints(keep.iter().map(|&i| dim[i]).collect());
        if let Some(Value::Str(nms)) = dim_names_attr(&x) {
            newdims.set_attr_raw(s::NAMES, Value::strs(keep.iter().map(|&i| nms.data[i].clone()).collect()));
        }
        let mut havenames = false;
        if !dimnames.is_null() {
            havenames = keep.iter().any(|&i| !generic_elt(&dimnames, i).is_null());
        }
        let newnames = if havenames {
            let mut nn = Value::list(keep.iter().map(|&i| generic_elt(&dimnames, i)).collect());
            if let Some(Value::Str(d)) = &dnn {
                nn.set_attr_raw(s::NAMES, Value::strs(keep.iter().map(|&i| d.data[i].clone()).collect()));
            }
            Some(nn)
        } else {
            None
        };
        x.set_attr_raw(s::DIMNAMES, Value::Null);
        x.set_attr_raw(s::DIM, newdims);
        if let Some(nn) = newnames {
            x = set_attrib(x, s::DIMNAMES, nn)?;
        }
    }
    Ok(x)
}

// ----------------------------------------------------------------------
// [

fn do_subset(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    match dispatch_or_eval_sp(interp, call, "[", args, env)? {
        Dispatched::Done(v) => Ok(v),
        Dispatched::Args(a) => subset_dflt(interp, call, a),
    }
}

fn do_dot_subset(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    subset_dflt(interp, call, args)
}

/// R's `do_subset_dflt`: the default method of `[` (also `.subset`).
// R: subset.c:do_subset_dflt
pub fn subset_dflt(interp: &mut Interp, call: &Value, mut args: Vec<Arg>) -> RResult<Value> {
    let drop = extract_drop_arg(&mut args);
    if args.is_empty() {
        return Ok(Value::Null);
    }
    let x = std::mem::replace(&mut args[0].value, Value::Null);
    if x.is_null() {
        return Ok(x);
    }
    let subs: Vec<Value> = args.drain(1..).map(|a| a.value).collect();
    let nsubs = subs.len();
    let is_lang = matches!(x, Value::Lang(_));
    let converted;
    let ax: &Value = if x.is_vector() {
        &x
    } else if matches!(x, Value::Pairlist(_) | Value::Lang(_)) {
        converted = pairlike_as_list(&x);
        &converted
    } else {
        return Err(not_subsettable(&x, Some(call)));
    };
    let mut ans = if nsubs < 2 {
        let dim = x.get_attr(s::DIM);
        let ndim = dim.as_ref().map(|d| d.len()).unwrap_or(0);
        let missing = Value::missing_arg();
        let mut ans = vector_subset(ax, if nsubs == 1 { &subs[0] } else { &missing }, call)?;
        if ndim == 1 {
            let len = ans.len();
            if !drop || len > 1 {
                let nm = get_attrib(&ans, s::NAMES);
                let mut attr = Value::int(len as i32);
                if let Some(n) = dim.as_ref().and_then(|d| d.get_attr(s::NAMES)) {
                    attr.set_attr_raw(s::NAMES, n);
                }
                ans.set_attr_raw(s::DIMNAMES, Value::Null);
                ans.set_attr_raw(s::DIM, attr);
                if let Value::List(dn) = get_attrib(&x, s::DIMNAMES) {
                    let mut ndn = (*dn).clone();
                    ndn.data[0] = nm;
                    ans = set_attrib(ans, s::DIMNAMES, Value::List(Rc::new(ndn)))?;
                    ans.set_attr_raw(s::NAMES, Value::Null);
                }
            }
        }
        ans
    } else {
        let ndim = x.get_attr(s::DIM).map(|d| d.len()).unwrap_or(0);
        if nsubs != ndim {
            return Err(Flow::error_call(call.clone(), "incorrect number of dimensions"));
        }
        if nsubs == 2 {
            matrix_subset(interp, ax, &subs[0], &subs[1], call, drop)?
        } else {
            array_subset(interp, ax, &subs, call, drop)?
        }
    };
    if is_lang {
        ans = list_as_lang(ans);
    }
    if ans.has_attrs() {
        ans.set_attr_raw(s::TSP, Value::Null);
        ans.set_attr_raw(s::CLASS, Value::Null);
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// [[

fn do_subset2(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    match dispatch_or_eval_sp(interp, call, "[[", args, env)? {
        Dispatched::Done(v) => Ok(v),
        Dispatched::Args(a) => subset2_dflt(interp, call, a),
    }
}

fn do_dot_subset2(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    subset2_dflt(interp, call, args)
}

/// R's `do_subset2_dflt`: the default method of `[[` (also `.subset2`).
// R: subset.c:do_subset2_dflt
pub fn subset2_dflt(interp: &mut Interp, call: &Value, mut args: Vec<Arg>) -> RResult<Value> {
    extract_drop_arg(&mut args);
    let exact = extract_exact_arg(&mut args);
    let pok = if exact == -1 { -1 } else { (exact == 0) as i32 };
    let x = if args.is_empty() { Value::Null } else { std::mem::replace(&mut args[0].value, Value::Null) };
    let subs: Vec<Value> = if args.is_empty() { vec![] } else { args.drain(1..).map(|a| a.value).collect() };
    let nsubs = subs.len();
    if nsubs == 0 {
        return Err(Flow::error_call(call.clone(), "no index specified"));
    }
    if x.is_null() {
        if subs[0].is_missing_arg() {
            return Err(missing_subscript(Some(call)));
        }
        return Ok(x);
    }
    let dims = dim_of(&x).unwrap_or_default();
    let ndims = dims.len();
    if nsubs > 1 && nsubs != ndims {
        return Err(Flow::error_call(call.clone(), "incorrect number of subscripts"));
    }
    if let Value::Env(e) = &x {
        let name = match &subs[0] {
            Value::Str(v) if nsubs == 1 && v.data.len() == 1 => char_of(&v.data[0]).to_string(),
            _ => return Err(Flow::error_call(call.clone(), "wrong arguments for subsetting an environment")),
        };
        return Ok(find_var_in_frame(interp, e, Sym::new(&name))?.unwrap_or(Value::Null));
    }
    if !is_subsettable(&x) {
        return Err(not_subsettable(&x, Some(call)));
    }
    let mut x = x;
    let offset: i64;
    if nsubs == 1 {
        let thesub = &subs[0];
        let len = thesub.len();
        if len > 1 {
            x = vector_index(interp, &x, thesub, 0, len - 1, pok, call)?;
        }
        let xnames = get_attrib(&x, s::NAMES);
        let xlen = x.len() as i64;
        offset = get1index(interp, thesub, &xnames, xlen, pok, if len > 1 { len as i64 - 1 } else { -1 }, Some(call))?;
        if offset < 0 || offset >= xlen {
            if offset < 0
                && matches!(x, Value::Null | Value::List(_) | Value::Expr(_) | Value::Pairlist(_) | Value::Lang(_))
            {
                return Ok(Value::Null);
            }
            return Err(out_of_bounds(Some(call), None));
        }
    } else {
        let dimnames = get_attrib(&x, s::DIMNAMES);
        let ndn = dimnames.len();
        let mut pindx = vec![0i64; nsubs];
        for i in 0..nsubs {
            let names = if i < ndn { generic_elt(&dimnames, i) } else { Value::Null };
            // R passes the (zero-initialised) index slot as the length.
            pindx[i] = get1index(interp, &subs[i], &names, 0, pok, -1, Some(call))?;
            if pindx[i] < 0 || pindx[i] >= dims[i] as i64 {
                return Err(out_of_bounds(Some(call), None));
            }
        }
        let mut off: i64 = 0;
        for i in (1..nsubs).rev() {
            off = (off + pindx[i]) * dims[i - 1] as i64;
        }
        offset = off + pindx[0];
    }
    let o = offset as usize;
    Ok(match &x {
        Value::Pairlist(_) | Value::Lang(_) | Value::List(_) | Value::Expr(_) => generic_elt(&x, o),
        Value::Lgl(v) => Value::lgls(vec![v.data[o]]),
        Value::Int(v) => Value::ints(vec![v.data[o]]),
        Value::Real(v) => Value::reals(vec![v.data[o]]),
        Value::Cplx(v) => Value::cplxs(vec![v.data[o]]),
        Value::Str(v) => Value::strs(vec![v.data[o].clone()]),
        Value::Raw(v) => Value::raws(vec![v.data[o]]),
        _ => return Err(Flow::error(format!("unimplemented type '{}' in 'do_subset2'\n", x.type_name()))),
    })
}

// ----------------------------------------------------------------------
// $ and @

fn do_subset3(interp: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "$", call)?;
    let (input, _) = fix_subset3_arg(interp, call, &args[1].value, env)?;
    args[1].value = Value::strs(vec![input.clone()]);
    match dispatch_or_eval_sp(interp, call, "$", args, env)? {
        Dispatched::Done(v) => Ok(v),
        Dispatched::Args(a) => {
            let x = a.into_iter().next().map(|a| a.value).unwrap_or(Value::Null);
            subset3_dflt(interp, &x, &input, call)
        }
    }
}

#[derive(PartialEq)]
enum PMatch {
    No,
    Exact,
    Partial,
}

// R: subset.c:pstrmatch
fn pstrmatch(target: &str, input: &str) -> PMatch {
    if !input.is_empty() && target.starts_with(input) {
        if target.len() == input.len() {
            PMatch::Exact
        } else {
            PMatch::Partial
        }
    } else {
        PMatch::No
    }
}

/// R's `R_subset3_dflt`: `x$name` with partial matching.
// R: subset.c:R_subset3_dflt
pub fn subset3_dflt(interp: &mut Interp, x: &Value, input: &RStr, call: &Value) -> RResult<Value> {
    let input = char_of(input).to_string();
    let warn = matches!(interp.get_option("warnPartialMatchDollar"), Some(v) if crate::coerce::as_logical(&v) == TRUE);
    let candidates: Option<Vec<(Option<String>, usize)>> = match x {
        Value::Null => Some(vec![]),
        Value::Pairlist(p) => Some(p.items.iter().enumerate().map(|(i, a)| (a.tag.map(|t| t.name().to_string()), i)).collect()),
        Value::Lang(l) => Some(
            std::iter::once((None, 0))
                .chain(l.args.iter().enumerate().map(|(i, a)| (a.tag.map(|t| t.name().to_string()), i + 1)))
                .collect(),
        ),
        Value::List(_) | Value::Expr(_) => {
            let names = match get_attrib(x, s::NAMES) {
                Value::Str(n) => n.data.clone(),
                _ => vec![],
            };
            Some(names.iter().enumerate().map(|(i, n)| (Some(char_of(n).to_string()), i)).collect())
        }
        _ => None,
    };
    if let Some(cands) = candidates {
        let mut havematch = 0;
        let mut imatch: Option<(String, usize)> = None;
        for (tag, i) in cands {
            let Some(tag) = tag else { continue };
            match pstrmatch(&tag, &input) {
                PMatch::Exact => return Ok(generic_elt(x, i)),
                PMatch::Partial => {
                    havematch += 1;
                    imatch = Some((tag, i));
                }
                PMatch::No => {}
            }
        }
        if havematch == 1 {
            let (tag, i) = imatch.unwrap();
            if warn {
                interp.warningcall(call, format!("partial match of '{}' to '{}'", input, tag))?;
            }
            return Ok(generic_elt(x, i));
        }
        return Ok(Value::Null);
    }
    match x {
        Value::Env(e) => Ok(find_var_in_frame(interp, e, Sym::new(&input))?.unwrap_or(Value::Null)),
        x if x.is_atomic() => Err(Flow::error_call(call.clone(), "$ operator is invalid for atomic vectors")),
        _ => Err(not_subsettable(x, Some(call))),
    }
}

/// `@`: S3 dispatch for objects; formal (S4) classes need the methods
/// package, which rr does not provide.
// R: attrib.c:do_AT
fn do_at(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "@", call)?;
    let object = interp.eval(&args[0].value, env)?;
    if object.is_object() && !call_is_default(call) {
        let (input, _) = fix_subset3_arg(interp, call, &args[1].value, env)?;
        let prom = Value::Promise(Rc::new(Promise::forced(args[0].value.clone(), object.clone())));
        let pargs = vec![Arg::new(args[0].tag, prom), Arg::new(args[1].tag, Value::strs(vec![input]))];
        if let Some(ans) = interp.dispatch_with_temp_context("@", &object, call, pargs, env)? {
            return Ok(ans);
        }
    }
    Err(Flow::error("formal classes cannot be used without the 'methods' package"))
}
