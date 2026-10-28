//! Subassignment: `[<-`, `[[<-` and `$<-` (GNU R's subassign.c).
//!
//! The object arrives by value (see `subset::dispatch_or_eval_sp`): in a
//! complex assignment `x[i] <- v` the binding of `x` has been emptied so
//! that we hold the only reference and `Rc::make_mut` does not copy. The
//! default methods work on `&mut Value` and check everything that can fail
//! before the first mutation, so on error the object is unchanged; it is
//! then left in [`take_restore`] for the complex assignment to rebind.

use std::cell::RefCell;
use std::rc::Rc;

use super::subscript::*;
use super::subset::{Dispatched, dispatch_or_eval_sp, fix_subset3_arg, tag_of};
use super::{check_arity, special};
use crate::attrib::get_attrib;
use crate::coerce::{CoerceWarnings, to_cplx, to_int, to_lgl, to_list, to_real, to_str};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] =
    &[special("[<-", do_subassign, -1), special("[[<-", do_subassign2, -1), special("$<-", do_subassign3, 3)];

// R: subassign.c:errorNotSubsettable/errorMissingSubscript/errorOutOfBoundsSEXP.
// R_signalErrorCondition receives R_CurrentExpression explicitly, which is
// printed as the symbol below rather than the enclosing replacement call.
fn replacement_error_call() -> Value {
    Value::Sym(Sym::new("<current-expression>"))
}

// ----------------------------------------------------------------------
// handing the object back on error

thread_local! {
    static RESTORE: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Forget any object left by a failed replacement.
pub fn clear_restore() {
    RESTORE.with(|r| *r.borrow_mut() = None);
}

/// The (unchanged) object of the last replacement call that failed.
pub fn take_restore() -> Option<Value> {
    RESTORE.with(|r| r.borrow_mut().take())
}

fn stash(x: Value) {
    RESTORE.with(|r| *r.borrow_mut() = Some(x));
}

/// Stash the object argument of a replacement call that fails before it
/// is taken apart (it may have been moved out of its binding).
fn stash_first(args: &mut [Arg]) {
    if let Some(a) = args.first_mut() {
        if !matches!(a.value, Value::Sym(_) | Value::Lang(_) | Value::Promise(_)) {
            stash(std::mem::replace(&mut a.value, Value::Null));
        }
    }
}

// ----------------------------------------------------------------------
// helpers

/// R's SEXPTYPE codes, used for the `which` codes of SubassignTypeFix.
fn sexptype(v: &Value) -> u32 {
    match v {
        Value::Null => 0,
        Value::Sym(_) => 1,
        Value::Pairlist(_) => 2,
        Value::Closure(_) => 3,
        Value::Env(_) => 4,
        Value::Promise(_) => 5,
        Value::Lang(_) => 6,
        Value::Builtin(b) => {
            if b.kind == BuiltinKind::Special {
                7
            } else {
                8
            }
        }
        Value::Lgl(_) => 10,
        Value::Int(_) => 13,
        Value::Real(_) => 14,
        Value::Cplx(_) => 15,
        Value::Str(_) => 16,
        Value::Dots(_) => 17,
        Value::List(_) => 19,
        Value::Expr(_) => 20,
        Value::ExtPtr(_) => 22,
        Value::Raw(_) => 24,
        Value::Obj(_) => 25,
    }
}

/// R's `type2char`.
fn type_char(t: u32) -> &'static str {
    match t {
        0 => "NULL",
        1 => "symbol",
        2 => "pairlist",
        3 => "closure",
        4 => "environment",
        5 => "promise",
        6 => "language",
        7 => "special",
        8 => "builtin",
        10 => "logical",
        13 => "integer",
        14 => "double",
        15 => "complex",
        16 => "character",
        17 => "...",
        19 => "list",
        20 => "expression",
        22 => "externalptr",
        24 => "raw",
        25 => "S4",
        _ => "unknown",
    }
}

/// Attributes of a vector rebuilt from `x` (EnlargeVector,
/// DeleteListElements, ...): the new names first, then `copyMostAttrib`.
fn rebuilt_attrs(x: &Value, names: Option<Value>) -> AttrBox {
    let mut out = Vec::new();
    if let Some(n) = names {
        out.push((s::NAMES, n));
    }
    if let Some(a) = x.attrs() {
        for (k, v) in &a.0 {
            if *k != s::NAMES && *k != s::DIM && *k != s::DIMNAMES {
                out.push((*k, v.clone()));
            }
        }
    }
    if out.is_empty() { None } else { Some(Box::new(Attrs(out, false))) }
}

/// R's `PairToVectorList`.
fn pair_to_vector_list(x: &Value) -> Value {
    let mut v = Value::list(to_list(x));
    if let Some(n) = x.names() {
        v.set_attr_raw(s::NAMES, n);
    }
    crate::attrib::copy_most_attrib(x, &mut v);
    v
}

/// R's `VectorToPairList` followed by `SET_TYPEOF(x, LANGSXP)`.
fn vector_to_lang(x: &Value) -> Value {
    let names = match get_attrib(x, s::NAMES) {
        Value::Str(n) => Some(n.data.clone()),
        _ => None,
    };
    let items: Vec<Arg> = to_list(x)
        .into_iter()
        .enumerate()
        .map(|(i, v)| Arg::new(names.as_ref().and_then(|n| tag_of(n.get(i))), v))
        .collect();
    let mut lang = Value::Lang(Rc::new(Lang::from_items(items, None)));
    crate::attrib::copy_most_attrib(x, &mut lang);
    lang
}

/// The cells of a pairlist or call (a call's function is cell 0).
fn pairlike_items(x: &Value) -> Vec<Arg> {
    match x {
        Value::Pairlist(p) => p.items.clone(),
        Value::Lang(l) => l.items(),
        _ => vec![],
    }
}

/// Mutable access to element `i` of a list, pairlist or call.
fn elt_mut(x: &mut Value, i: usize) -> &mut Value {
    match x {
        Value::List(l) | Value::Expr(l) => &mut Rc::make_mut(l).data[i],
        Value::Pairlist(p) => &mut Rc::make_mut(p).items[i].value,
        Value::Lang(l) => {
            let l = Rc::make_mut(l);
            if i == 0 { &mut l.fun } else { &mut l.args[i - 1].value }
        }
        _ => unreachable!("elt_mut on a non-list"),
    }
}

/// `coerceVector(x, type)` for the left-hand side in SubassignTypeFix and
/// `$<-`: atomic results keep all attributes, lists only the names,
/// expressions none.
// R: coerce.c:coerceVector
fn coerce_lhs(x: &Value, to: u32) -> Value {
    let mut w = CoerceWarnings::default();
    let keep = |mut v: Value| {
        v.set_attrs(x.attrs().cloned().map(Box::new));
        v
    };
    match to {
        10 => keep(Value::lgls(to_lgl(x, &mut w))),
        13 => keep(Value::ints(to_int(x, &mut w))),
        14 => keep(Value::reals(to_real(x, &mut w))),
        15 => keep(Value::cplxs(to_cplx(x, &mut w))),
        16 => keep(Value::strs(to_str(x))),
        19 => {
            let mut v = Value::list(to_list(x));
            let n = get_attrib(x, s::NAMES);
            if !n.is_null() {
                v.set_attr_raw(s::NAMES, n);
            }
            v
        }
        20 => Value::expr_vec(to_list(x)),
        _ => unreachable!("coerce_lhs to type {}", to),
    }
}

/// `coerceVector(y, VECSXP)` for the right-hand side.
// R: coerce.c:coerceVector
fn coerce_rhs_to_list(interp: &mut Interp, y: Value) -> RResult<Value> {
    match &y {
        Value::List(_) | Value::Null => Ok(y),
        Value::Expr(v) => Ok(Value::List(v.clone())),
        v if v.is_atomic() => Ok(coerce_lhs(v, 19)),
        Value::Sym(_) => {
            interp.warning("'symbol' object cannot be coerced to type 'list'")?;
            Ok(Value::Null)
        }
        Value::Pairlist(_) | Value::Lang(_) => Ok(pair_to_vector_list(&y)),
        Value::Env(_) => Err(Flow::error("environments cannot be coerced to other types")),
        _ => Err(Flow::error(format!("cannot coerce type '{}' to vector of type 'list'", y.type_name()))),
    }
}

/// The outcome of SubassignTypeFix: the type `x` must be coerced to (done
/// by the caller once nothing can fail any more), the possibly coerced
/// `y`, and the type code `100 * TYPEOF(x) + TYPEOF(y)` after coercion.
struct TypeFix {
    x_to: Option<u32>,
    y: Value,
    which: u32,
}

// R: subassign.c:SubassignTypeFix
fn type_fix(interp: &mut Interp, call: &Value, x: &Value, y: Value, level: u8) -> RResult<TypeFix> {
    let tx = sexptype(x);
    let ty = sexptype(&y);
    let which = 100 * tx + ty;
    let mut x_to = None;
    let mut y = y;
    match which {
        1000 | 1300 | 1400 | 1500 | 1600 | 1900 | 2000 | 2400 | 1010 | 1310 | 1410 | 1510 | 1313 | 1413 | 1513
        | 1414 | 1514 | 1515 | 1616 | 1919 | 2020 | 2424 => return Ok(TypeFix { x_to, y, which }),
        1013 => x_to = Some(13),
        1014 | 1314 => x_to = Some(14),
        1015 | 1315 | 1415 => x_to = Some(15),
        1610 | 1613 | 1614 | 1615 => y = Value::strs(to_str(&y)),
        1016 | 1316 | 1416 | 1516 => x_to = Some(16),
        1901 | 1902 | 1904 | 1905 | 1906 | 1910 | 1913 | 1914 | 1915 | 1916 | 1920 | 1921 | 1922 | 1923 | 1924
        | 1903 | 1907 | 1908 | 1999 => {
            if level == 1 {
                y = coerce_rhs_to_list(interp, y)?;
            } else {
                return Ok(TypeFix { x_to, y, which });
            }
        }
        // R: subassign.c:embedInVector
        1925 | 2025 => {
            if level == 1 {
                if y.is_s4() {
                    interp.warningcall(call, "implicit list embedding of S4 objects is deprecated")?;
                } else {
                    return Err(Flow::error_call(
                        call.clone(),
                        "implicit list embedding of \"object\" is not possible",
                    ));
                }
                y = Value::list(vec![y]);
            } else {
                return Ok(TypeFix { x_to, y, which });
            }
        }
        1019 | 1319 | 1419 | 1519 | 1619 | 2419 => x_to = Some(19),
        1020 | 1320 | 1420 | 1520 | 1620 | 2420 => x_to = Some(20),
        2001 | 2002 | 2006 | 2010 | 2013 | 2014 | 2015 | 2016 | 2019 => {
            if level == 1 {
                y = coerce_rhs_to_list(interp, y)?;
            } else {
                return Ok(TypeFix { x_to, y, which });
            }
        }
        _ => {
            return Err(Flow::error(format!(
                "incompatible types (from {} to {}) in subassignment type fix",
                type_char(tx),
                type_char(ty)
            )));
        }
    }
    let nx = x_to.unwrap_or(tx);
    let which = 100 * nx + sexptype(&y);
    Ok(TypeFix { x_to, y, which })
}

/// The `check.bounds` warning of EnlargeVector (and robigo's length limit).
fn check_bounds(interp: &mut Interp, len: usize, newlen: i64) -> RResult<()> {
    if newlen > i32::MAX as i64 {
        return Err(Flow::error("robigo: long vectors are not supported"));
    }
    if let Some(v) = interp.get_option("check.bounds") {
        if crate::coerce::as_logical(&v) == TRUE {
            interp.warning(format!("assignment outside vector/list limits (extending from {} to {})", len, newlen))?;
        }
    }
    Ok(())
}

/// R's `EnlargeVector`: extend with NAs; names are extended with "", dim
/// and dimnames are dropped.
// R: subassign.c:EnlargeVector
fn enlarge_vector(x: &mut Value, newlen: usize) -> RResult<()> {
    if !x.is_vector() {
        return Err(Flow::error("attempt to enlarge non-vector"));
    }
    let len = x.len();
    let names = match get_attrib(x, s::NAMES) {
        Value::Null => None,
        Value::Str(n) if n.data.len() == len => {
            let mut d = n.data.clone();
            d.resize(newlen.max(len), Some(Rc::from("")));
            Some(Value::strs(d))
        }
        _ => return Err(Flow::error("bad names attribute")),
    };
    let attrs = rebuilt_attrs(x, names);
    match x {
        Value::Lgl(v) => Rc::make_mut(v).data.resize(newlen, NA_LOGICAL),
        Value::Int(v) => Rc::make_mut(v).data.resize(newlen, NA_INTEGER),
        Value::Real(v) => Rc::make_mut(v).data.resize(newlen, NA_REAL),
        Value::Cplx(v) => Rc::make_mut(v).data.resize(newlen, Complex::NA),
        Value::Str(v) => Rc::make_mut(v).data.resize(newlen, None),
        Value::Raw(v) => Rc::make_mut(v).data.resize(newlen, 0),
        Value::List(v) | Value::Expr(v) => Rc::make_mut(v).data.resize(newlen, Value::Null),
        _ => unreachable!(),
    }
    x.set_attrs(attrs);
    Ok(())
}

fn assign_elts<T: Clone>(dst: &mut [T], idx: &[i64], src: &[T]) {
    let ny = src.len();
    if ny == 0 {
        return;
    }
    for (i, &ii) in idx.iter().enumerate() {
        if ii == NA_IDX {
            continue;
        }
        dst[(ii - 1) as usize] = src[i % ny].clone();
    }
}

/// Is `which` (after the type fix) an atomic combination the assignment
/// loops handle?
fn atomic_which_ok(which: u32) -> bool {
    matches!(
        which,
        1010 | 1310 | 1313 | 1410 | 1413 | 1414 | 1510 | 1513 | 1514 | 1515 | 1610 | 1613 | 1614 | 1615 | 1616 | 2424
    )
}

/// The element loops of VectorAssign & co after the type fix: `y` is
/// converted to the type of `x` (only widening conversions remain).
fn assign_values(x: &mut Value, idx: &[i64], y: &Value) {
    let mut w = CoerceWarnings::default();
    match (x, y) {
        (Value::Lgl(a), Value::Lgl(b)) | (Value::Int(a), Value::Int(b)) => {
            assign_elts(&mut Rc::make_mut(a).data, idx, &b.data)
        }
        (Value::Real(a), Value::Real(b)) => assign_elts(&mut Rc::make_mut(a).data, idx, &b.data),
        (Value::Cplx(a), Value::Cplx(b)) => assign_elts(&mut Rc::make_mut(a).data, idx, &b.data),
        (Value::Str(a), Value::Str(b)) => assign_elts(&mut Rc::make_mut(a).data, idx, &b.data),
        (Value::Raw(a), Value::Raw(b)) => assign_elts(&mut Rc::make_mut(a).data, idx, &b.data),
        (Value::List(a) | Value::Expr(a), Value::List(b) | Value::Expr(b)) => {
            assign_elts(&mut Rc::make_mut(a).data, idx, &b.data)
        }
        (Value::Int(a), y) => assign_elts(&mut Rc::make_mut(a).data, idx, &to_int(y, &mut w)),
        (Value::Real(a), y) => assign_elts(&mut Rc::make_mut(a).data, idx, &to_real(y, &mut w)),
        (Value::Cplx(a), y) => assign_elts(&mut Rc::make_mut(a).data, idx, &to_cplx(y, &mut w)),
        (Value::Str(a), y) => assign_elts(&mut Rc::make_mut(a).data, idx, &to_str(y)),
        _ => unreachable!("assign_values: unexpected types"),
    }
}

// ----------------------------------------------------------------------
// [<-

fn do_subassign(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    match dispatch_or_eval_sp(interp, call, "[<-", args, env)? {
        Dispatched::Done(v) => Ok(v),
        Dispatched::Args(a) => subassign_dflt(interp, call, a),
    }
}

/// R's `do_subassign_dflt`: the default method of `[<-`.
// R: subassign.c:do_subassign_dflt
pub fn subassign_dflt(interp: &mut Interp, call: &Value, mut args: Vec<Arg>) -> RResult<Value> {
    if args.len() < 2 {
        stash_first(&mut args);
        return Err(Flow::error("SubAssignArgs: invalid number of arguments"));
    }
    let y = args.pop().unwrap().value;
    let mut x = std::mem::replace(&mut args[0].value, Value::Null);
    let subs: Vec<Value> = args.drain(1..).map(|a| a.value).collect();
    match subassign_core(interp, call, &mut x, &subs, y) {
        Ok(()) => Ok(x),
        Err(f) => {
            stash(x);
            Err(f)
        }
    }
}

fn subassign_core(interp: &mut Interp, call: &Value, x: &mut Value, subs: &[Value], y: Value) -> RResult<()> {
    let is_lang = matches!(x, Value::Lang(_));
    let mut work: Option<Value> = None;
    if matches!(x, Value::Pairlist(_) | Value::Lang(_)) {
        work = Some(pair_to_vector_list(x));
    } else if x.is_empty() {
        let same_type = sexptype(x) == sexptype(&y);
        if y.is_empty() && (x.is_null() || same_type || y.is_vector_list()) {
            return Ok(());
        }
        if x.is_null() {
            // coerceVector(NULL, TYPEOF(y))
            work = Some(match (&y, y.vtype()) {
                (_, Some(t)) => Value::empty_of(t),
                (Value::Pairlist(_), _) => return Err(not_subsettable(&Value::Null, Some(&replacement_error_call()))),
                (Value::Lang(_), _) => return Err(Flow::error("unimplemented type 'NULL' in 'coercePairList'\n")),
                _ => {
                    return Err(Flow::error(format!("invalid type/length ({}/0) in vector allocation", y.type_name())));
                }
            });
        }
    }
    {
        let target: &mut Value = match work.as_mut() {
            Some(w) => w,
            None => x,
        };
        if !target.is_vector() {
            return Err(not_subsettable(target, Some(&replacement_error_call())));
        }
        match subs.len() {
            0 => vector_assign(interp, call, target, &Value::missing_arg(), y)?,
            1 => vector_assign(interp, call, target, &subs[0], y)?,
            2 => matrix_assign(interp, call, target, &subs[0], &subs[1], y)?,
            _ => array_assign(interp, call, target, subs, y)?,
        }
    }
    if let Some(w) = work {
        if is_lang {
            if w.is_empty() {
                return Err(Flow::error("result is zero-length and so cannot be a language object"));
            }
            *x = vector_to_lang(&w);
        } else {
            *x = w;
        }
    }
    Ok(())
}

/// R's `VectorAssign`: `x[s] <- y`.
// R: subassign.c:VectorAssign
fn vector_assign(interp: &mut Interp, call: &Value, x: &mut Value, s: &Value, y: Value) -> RResult<()> {
    if x.is_null() && y.is_null() {
        return Ok(());
    }
    // (R reports numeric matrix subscript errors against the context.)
    let mat = matrix_subscript(x, s, Some(call), None)?;
    let s = mat.as_ref().unwrap_or(s);
    let mut stretch: i64 = 1;
    let indx = make_subscript(x, s, &mut stretch, None)?;
    let n = indx.idx.len();
    if y.len() > 1 && indx.idx.contains(&NA_IDX) {
        return Err(Flow::error("NAs are not allowed in subscripted assignments"));
    }
    let fix = type_fix(interp, call, x, y, 1)?;
    if stretch > 0 {
        check_bounds(interp, x.len(), stretch)?;
    }
    let y = fix.y;
    let target_list = matches!(fix.which / 100, 19 | 20);
    if n > 0 && (!target_list || !y.is_null()) {
        let ny = y.len();
        if ny == 0 {
            return Err(Flow::error("replacement has length zero"));
        }
        if n % ny != 0 {
            interp.warning("number of items to replace is not a multiple of replacement length")?;
        }
    }
    // Committed: nothing below can fail.
    if let Some(t) = fix.x_to {
        *x = coerce_lhs(x, t);
    }
    if stretch > 0 {
        enlarge_vector(x, stretch as usize)?;
    }
    if n == 0 {
        return Ok(());
    }
    if target_list && y.is_null() {
        delete_list_elements(x, &indx.idx);
        return Ok(());
    }
    if !target_list && !atomic_which_ok(fix.which) {
        return interp.warningcall(call, "sub assignment (*[*] <- *) not done; __bug?__");
    }
    assign_values(x, &indx.idx, &y);
    if let Some(newnames) = indx.use_names {
        let nx = x.len();
        let mut names: Vec<RStr> = match get_attrib(x, s::NAMES) {
            Value::Str(n) => n.data.clone(),
            _ => vec![Some(Rc::from("")); nx],
        };
        for (i, nn) in newnames.into_iter().enumerate() {
            if let Some(nn) = nn {
                let ii = indx.idx[i];
                if ii == NA_IDX {
                    continue;
                }
                names[(ii - 1) as usize] = nn;
            }
        }
        x.set_attr_raw(s::NAMES, Value::strs(names));
    }
    Ok(())
}

/// R's `DeleteListElements`: `x[i] <- NULL` on a list.
// R: subassign.c:DeleteListElements
fn delete_list_elements(x: &mut Value, which: &[i64]) {
    let len = x.len();
    let mut include = vec![true; len];
    for &ii in which {
        if ii >= 1 && ii as usize <= len {
            include[ii as usize - 1] = false;
        }
    }
    if include.iter().all(|&b| b) {
        return;
    }
    let names = match get_attrib(x, s::NAMES) {
        Value::Str(n) => {
            let mut keep = include.iter();
            Some(Value::strs(n.data.iter().filter(|_| *keep.next().unwrap()).cloned().collect()))
        }
        _ => None,
    };
    let attrs = rebuilt_attrs(x, names);
    if let Value::List(v) | Value::Expr(v) = x {
        let mut keep = include.iter();
        Rc::make_mut(v).data.retain(|_| *keep.next().unwrap());
    }
    x.set_attrs(attrs);
}

/// R's `MatrixAssign`: `x[i, j] <- y`.
// R: subassign.c:MatrixAssign
fn matrix_assign(interp: &mut Interp, call: &Value, x: &mut Value, s0: &Value, s1: &Value, y: Value) -> RResult<()> {
    if !is_matrix(x) {
        return Err(Flow::error("incorrect number of subscripts on matrix"));
    }
    let dim = dim_of(x).unwrap_or_default();
    let nr = dim[0] as i64;
    let ny = y.len();
    let sr = array_subscript(interp, 0, s0, &dim, x, Some(call))?;
    let sc = array_subscript(interp, 1, s1, &dim, x, Some(call))?;
    let any_na = sr.contains(&NA_IDX) || sc.contains(&NA_IDX);
    if ny > 1 && any_na {
        return Err(Flow::error("NAs are not allowed in subscripted assignments"));
    }
    let n = sr.len() * sc.len();
    if n > 0 && ny == 0 {
        // GNU R reports a NULL RHS with the "not a multiple" error here even
        // though subassign.c:MatrixAssign checks ny == 0 first (verified
        // against the 4.5.0 oracle); a zero-length non-NULL RHS gets the
        // length-zero message.
        if y.is_null() {
            return Err(Flow::error("number of items to replace is not a multiple of replacement length"));
        }
        return Err(Flow::error("replacement has length zero"));
    }
    if n > 0 && n % ny != 0 {
        return Err(Flow::error("number of items to replace is not a multiple of replacement length"));
    }
    let fix = type_fix(interp, call, x, y, 1)?;
    if n > 0 && fix.which != 1919 && !atomic_which_ok(fix.which) {
        return Err(Flow::error(format!(
            "incompatible types (from {} to {}) in matrix subset assignment",
            type_char(fix.which / 100),
            type_char(fix.which % 100)
        )));
    }
    if let Some(t) = fix.x_to {
        *x = coerce_lhs(x, t);
    }
    if n == 0 {
        return Ok(());
    }
    let mut flat = Vec::with_capacity(n);
    for &jj in &sc {
        for &ii in &sr {
            flat.push(if ii == NA_IDX || jj == NA_IDX { NA_IDX } else { ii + (jj - 1) * nr });
        }
    }
    assign_values(x, &flat, &fix.y);
    Ok(())
}

/// R's `ArrayAssign`: `x[i, j, k, ...] <- y`.
// R: subassign.c:ArrayAssign
fn array_assign(interp: &mut Interp, call: &Value, x: &mut Value, subs: &[Value], y: Value) -> RResult<()> {
    let dims = match dim_of(x) {
        Some(d) if d.len() == subs.len() => d,
        _ => return Err(Flow::error("incorrect number of subscripts")),
    };
    let k = dims.len();
    let ny = y.len();
    let mut sub_idx = Vec::with_capacity(k);
    for i in 0..k {
        sub_idx.push(array_subscript(interp, i, &subs[i], &dims, x, Some(call))?);
    }
    let bound: Vec<usize> = sub_idx.iter().map(|v| v.len()).collect();
    let n: usize = bound.iter().product();
    if n > 0 && ny == 0 {
        // See matrix_assign: GNU R reports a NULL RHS with the "not a
        // multiple" error.
        if y.is_null() {
            return Err(Flow::error("number of items to replace is not a multiple of replacement length"));
        }
        return Err(Flow::error("replacement has length zero"));
    }
    if n > 0 && n % ny != 0 {
        return Err(Flow::error("number of items to replace is not a multiple of replacement length"));
    }
    if ny > 1 && sub_idx.iter().any(|v| v.contains(&NA_IDX)) {
        return Err(Flow::error("NAs are not allowed in subscripted assignments"));
    }
    let mut offset = vec![1i64; k];
    for i in 1..k {
        offset[i] = offset[i - 1] * dims[i - 1] as i64;
    }
    let fix = type_fix(interp, call, x, y, 1)?;
    if n > 0 && fix.which != 1919 && !atomic_which_ok(fix.which) {
        return Err(Flow::error(format!(
            "incompatible types (from {} to {}) in array subset assignment",
            type_char(fix.which / 100),
            type_char(fix.which % 100)
        )));
    }
    if let Some(t) = fix.x_to {
        *x = coerce_lhs(x, t);
    }
    if n == 0 {
        return Ok(());
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
    assign_values(x, &flat, &fix.y);
    Ok(())
}

// ----------------------------------------------------------------------
// [[<-

fn do_subassign2(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    match dispatch_or_eval_sp(interp, call, "[[<-", args, env)? {
        Dispatched::Done(v) => Ok(v),
        Dispatched::Args(a) => subassign2_dflt(interp, call, a),
    }
}

/// R's `do_subassign2_dflt`: the default method of `[[<-`.
// R: subassign.c:do_subassign2_dflt
pub fn subassign2_dflt(interp: &mut Interp, call: &Value, mut args: Vec<Arg>) -> RResult<Value> {
    if args.len() < 2 {
        stash_first(&mut args);
        return Err(Flow::error("SubAssignArgs: invalid number of arguments"));
    }
    let y = args.pop().unwrap().value;
    let mut x = std::mem::replace(&mut args[0].value, Value::Null);
    let subs: Vec<Value> = args.drain(1..).map(|a| a.value).collect();
    let r = if x.is_null() {
        if y.is_null() {
            Ok(())
        } else {
            // A NULL target becomes a list.
            let mut tmp = Value::list(vec![]);
            let r = subassign2_core(interp, call, &mut tmp, &subs, y);
            if r.is_ok() {
                x = tmp;
            }
            r
        }
    } else {
        subassign2_core(interp, call, &mut x, &subs, y)
    };
    match r {
        Ok(()) => Ok(x),
        Err(f) => {
            stash(x);
            Err(f)
        }
    }
}

fn subassign2_core(interp: &mut Interp, call: &Value, x: &mut Value, subs: &[Value], y: Value) -> RResult<()> {
    if let Value::Env(e) = x {
        let name = match subs {
            [Value::Str(v)] if v.data.len() == 1 => char_of(&v.data[0]).to_string(),
            _ => return Err(Flow::error("wrong args for environment subassignment")),
        };
        let e = e.clone();
        return interp.define(Sym::new(&name), y, &e);
    }
    if subs.len() == 1 && subs[0].len() > 1 {
        return subassign2_recursive(interp, call, x, &subs[0], y);
    }
    let ind = subs.first().map(|s| s.len() as i64 - 1).unwrap_or(-1);
    assign2_level(interp, call, x, subs, y, -1, ind)
}

/// `x[[c(i, j, k)]] <- y`: walk down to the list holding the element and
/// assign into the element.
fn subassign2_recursive(interp: &mut Interp, call: &Value, top: &mut Value, thesub: &Value, y: Value) -> RResult<()> {
    let len = thesub.len();
    // Find (and check) the path without modifying anything.
    let (mut path, xup) = vector_index_path(interp, top, thesub, 0, len - 2, 1, call)?;
    let mut newname = None;
    let off = one_index(&xup, thesub, xup.len() as i64, &mut newname, len as i64 - 2, None)?;
    let (last, _) = vector_index_path(interp, &xup, thesub, len - 2, len - 1, 1, call)?;
    if xup.is_vector_list() && off != last[0] as i64 {
        return Err(Flow::error("robigo: partial matching in recursive [[<- is not implemented yet"));
    }
    path.extend(last);
    let mut cur: &mut Value = top;
    for o in path {
        cur = elt_mut(cur, o);
    }
    assign2_level(interp, call, cur, std::slice::from_ref(thesub), y, len as i64 - 1, len as i64 - 1)
}

/// `LENGTH(y)`: R's macro is undefined for non-vectors.
fn rhs_length(y: &Value) -> RResult<usize> {
    if y.is_null() || y.is_vector() {
        Ok(y.len())
    } else {
        Err(Flow::error("long vectors not supported yet: subassign.c:1854"))
    }
}

/// One level of `[[<-`: `pos` is the position in a recursive subscript
/// (-1 for a plain one) and `ind` the one used for pairlists.
fn assign2_level(
    interp: &mut Interp,
    call: &Value,
    x: &mut Value,
    subs: &[Value],
    y: Value,
    pos: i64,
    ind: i64,
) -> RResult<()> {
    let nsubs = subs.len();
    if x.is_vector() {
        if !x.is_vector_list() {
            let ly = rhs_length(&y)?;
            if ly == 0 {
                return Err(Flow::error("replacement has length zero"));
            }
            if ly > 1 {
                return Err(Flow::error("more elements supplied than there are to replace"));
            }
        }
        if nsubs == 0 || subs[0].is_missing_arg() {
            return Err(missing_subscript(Some(&replacement_error_call())));
        }
        let mut stretch: usize = 0;
        let mut newname: Option<RStr> = None;
        let offset: i64;
        if nsubs == 1 {
            offset = one_index(x, &subs[0], x.len() as i64, &mut newname, pos, None)?;
            if x.is_vector_list() && y.is_null() {
                delete_one_vector_list_item(x, offset);
                return Ok(());
            }
            if offset < 0 {
                return Err(out_of_bounds(Some(&replacement_error_call()), Some("[[ ]]")));
            }
            if offset as usize >= x.len() {
                stretch = offset as usize + 1;
            }
        } else {
            let dims = dim_of(x).unwrap_or_default();
            if dims.len() != nsubs {
                return Err(Flow::error("[[ ]] improper number of subscripts"));
            }
            let dimnames = get_attrib(x, s::DIMNAMES);
            let mut pindx = vec![0i64; nsubs];
            for i in 0..nsubs {
                let names = if dimnames.is_null() { Value::Null } else { generic_elt(&dimnames, i) };
                pindx[i] = get1index(interp, &subs[i], &names, dims[i] as i64, 0, -1, Some(call))?;
                if pindx[i] < 0 || pindx[i] >= dims[i] as i64 {
                    return Err(out_of_bounds(Some(&replacement_error_call()), Some("[[ ]]")));
                }
            }
            let mut off: i64 = 0;
            for i in (1..nsubs).rev() {
                off = (off + pindx[i]) * dims[i - 1] as i64;
            }
            offset = off + pindx[0];
        }
        let fix = type_fix(interp, call, x, y, 2)?;
        // R: subassign.c:do_subassign2_dflt's assignment switch. NULL
        // deletion is handled only for a single subscript above; matrices
        // must reject the remaining list <- NULL combination (1900).
        let list_which_ok = matches!(
            fix.which,
            1019 | 1319
                | 1419
                | 1519
                | 1619
                | 1901
                | 1902
                | 1903
                | 1904
                | 1905
                | 1906
                | 1907
                | 1908
                | 1910
                | 1913
                | 1914
                | 1915
                | 1916
                | 1919
                | 1920
                | 1921
                | 1922
                | 1923
                | 1924
                | 1925
                | 1999
                | 2001
                | 2002
                | 2006
                | 2010
                | 2013
                | 2014
                | 2015
                | 2016
                | 2020
                | 2024
                | 2025
        );
        if !list_which_ok && !atomic_which_ok(fix.which) {
            return Err(Flow::error(format!(
                "incompatible types (from {} to {}) in [[ assignment",
                type_char(fix.which / 100),
                type_char(fix.which % 100)
            )));
        }
        if stretch > 0 {
            check_bounds(interp, x.len(), stretch as i64)?;
        }
        // Committed.
        if let Some(t) = fix.x_to {
            *x = coerce_lhs(x, t);
        }
        if stretch > 0 {
            enlarge_vector(x, stretch)?;
        }
        let o = offset as usize;
        match x {
            Value::List(l) | Value::Expr(l) => Rc::make_mut(l).data[o] = fix.y,
            _ => assign_values(x, &[offset + 1], &fix.y),
        }
        if stretch > 0 {
            if let Some(nn) = newname {
                let mut names: Vec<RStr> = match get_attrib(x, s::NAMES) {
                    Value::Str(n) => n.data.clone(),
                    _ => vec![Some(Rc::from("")); x.len()],
                };
                names[o] = nn;
                x.set_attr_raw(s::NAMES, Value::strs(names));
            }
        }
        Ok(())
    } else if matches!(x, Value::Null | Value::Pairlist(_) | Value::Lang(_)) {
        if nsubs == 1 {
            if y.is_null() { list_remove(x, &subs[0], ind) } else { simple_list_assign(x, &subs[0], y, ind) }
        } else if dim_of(x).map(|d| d.len()).unwrap_or(0) != nsubs {
            Err(Flow::error("[[ ]] improper number of subscripts"))
        } else {
            Err(Flow::error("robigo: matrix indexing of pairlists in [[<- is not implemented yet"))
        }
    } else {
        Err(not_subsettable(x, Some(&replacement_error_call())))
    }
}

/// R's `DeleteOneVectorListItem`.
// R: subassign.c:DeleteOneVectorListItem
fn delete_one_vector_list_item(x: &mut Value, which: i64) {
    let n = x.len() as i64;
    if which < 0 || which >= n {
        return;
    }
    let w = which as usize;
    let names = match get_attrib(x, s::NAMES) {
        Value::Str(nm) => {
            let mut d = nm.data.clone();
            d.remove(w);
            Some(Value::strs(d))
        }
        _ => None,
    };
    let attrs = rebuilt_attrs(x, names);
    if let Value::List(v) | Value::Expr(v) = x {
        Rc::make_mut(v).data.remove(w);
    }
    x.set_attrs(attrs);
}

/// R's `GetOneIndex`: element `ind` of a vector subscript, for pairlists.
// R: subassign.c:GetOneIndex
fn get_one_index(sub: &Value, ind: i64) -> RResult<Value> {
    let len = sub.len() as i64;
    if ind < 0 || ind + 1 > len {
        return Err(Flow::error(format!("internal error: index {} from length {}", ind, len)));
    }
    if len > 1 {
        let i = ind as usize;
        return Ok(match sub {
            Value::Int(v) => Value::int(v.data[i]),
            Value::Real(v) => Value::real(v.data[i]),
            Value::Str(v) => Value::strs(vec![v.data[i].clone()]),
            _ => return Err(Flow::error("invalid subscript in list assign")),
        });
    }
    Ok(sub.clone())
}

/// Rebuild a pairlist-like object from cells, keeping its kind (a call
/// stays a call) and attributes.
fn rebuild_pairlike(old: &Value, items: Vec<Arg>, keep_lang: bool) -> Value {
    if items.is_empty() {
        return Value::Null;
    }
    let attrs = match old {
        Value::Pairlist(p) => p.attrs.clone(),
        Value::Lang(l) => l.attrs.clone(),
        _ => None,
    };
    if keep_lang && matches!(old, Value::Lang(_)) {
        Value::Lang(Rc::new(Lang::from_items(items, attrs)))
    } else {
        Value::Pairlist(Rc::new(Pairlist { items, attrs }))
    }
}

/// R's `SimpleListAssign`: `x[[s]] <- y` for pairlists and calls.
// R: subassign.c:SimpleListAssign
fn simple_list_assign(x: &mut Value, thesub: &Value, y: Value, ind: i64) -> RResult<()> {
    let sub = get_one_index(thesub, ind)?;
    let mut stretch: i64 = 1;
    let indx = make_subscript(x, &sub, &mut stretch, None)?;
    let n = indx.idx.len();
    if n > 1 {
        return Err(Flow::error("invalid subscript in list assign"));
    }
    let mut items = pairlike_items(x);
    let nx = items.len();
    if stretch > 0 {
        let extra = (stretch as usize).saturating_sub(nx);
        let tags: Option<Vec<Option<Sym>>> = match thesub {
            Value::Str(t) if t.data.len() == extra => Some(t.data.iter().map(|n| tag_of(Some(n))).collect()),
            _ => None,
        };
        for k in 0..extra {
            items.push(Arg::new(tags.as_ref().and_then(|t| t[k]), Value::Null));
        }
    }
    let nx = items.len();
    if n == 1 && nx > 0 {
        let ii = indx.idx[0];
        if ii != NA_IDX {
            items[((ii - 1) as usize) % nx].value = y;
        }
    }
    *x = rebuild_pairlike(x, items, true);
    Ok(())
}

/// R's `listRemove`: `x[[s]] <- NULL` for pairlists and calls. Removing a
/// call's function leaves a pairlist.
// R: subassign.c:listRemove
fn list_remove(x: &mut Value, thesub: &Value, ind: i64) -> RResult<()> {
    let sub = get_one_index(thesub, ind)?;
    let mut stretch: i64 = 0;
    let indx = make_subscript(x, &sub, &mut stretch, None)?;
    let items = pairlike_items(x);
    let mut include = vec![true; items.len()];
    for &ii in &indx.idx {
        if ii != NA_IDX && ii >= 1 && (ii as usize) <= include.len() {
            include[ii as usize - 1] = false;
        }
    }
    let keep_lang = include.first().copied().unwrap_or(false);
    let mut keep = include.iter();
    let items: Vec<Arg> = items.into_iter().filter(|_| *keep.next().unwrap()).collect();
    *x = rebuild_pairlike(x, items, keep_lang);
    Ok(())
}

// ----------------------------------------------------------------------
// $<-

fn do_subassign3(interp: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Err(f) = check_arity(&args, 3, "$<-", call) {
        stash_first(&mut args);
        return Err(f);
    }
    let (input, sym) = match fix_subset3_arg(interp, call, &args[1].value, env) {
        Ok(v) => v,
        Err(f) => {
            stash_first(&mut args);
            return Err(f);
        }
    };
    args[1].value = Value::strs(vec![input.clone()]);
    match dispatch_or_eval_sp(interp, call, "$<-", args, env)? {
        Dispatched::Done(v) => Ok(v),
        Dispatched::Args(mut a) => {
            let nlist = sym.unwrap_or_else(|| Sym::new(char_of(&input)));
            let val = a.get_mut(2).map(|v| std::mem::replace(&mut v.value, Value::Null)).unwrap_or(Value::Null);
            let mut x = std::mem::replace(&mut a[0].value, Value::Null);
            match subassign3_core(interp, &mut x, nlist, val) {
                Ok(()) => Ok(x),
                Err(f) => {
                    stash(x);
                    Err(f)
                }
            }
        }
    }
}

/// R's `R_subassign3_dflt`: `x$name <- val`.
// R: subassign.c:R_subassign3_dflt
pub fn subassign3_core(interp: &mut Interp, x: &mut Value, nlist: Sym, val: Value) -> RResult<()> {
    match x {
        Value::Pairlist(_) | Value::Lang(_) => {
            let mut items = pairlike_items(x);
            let mut keep_lang = true;
            if items[0].tag == Some(nlist) {
                if val.is_null() {
                    items.remove(0);
                    keep_lang = false;
                } else {
                    items[0].value = val;
                }
            } else {
                match items.iter().skip(1).position(|a| a.tag == Some(nlist)) {
                    Some(p) => {
                        if val.is_null() {
                            items.remove(p + 1);
                        } else {
                            items[p + 1].value = val;
                        }
                    }
                    None => {
                        if !val.is_null() {
                            items.push(Arg::new(Some(nlist), val));
                        }
                    }
                }
            }
            *x = rebuild_pairlike(x, items, keep_lang);
            Ok(())
        }
        Value::Env(e) => {
            let e = e.clone();
            interp.define(nlist, val, &e)
        }
        Value::Sym(_) | Value::Closure(_) | Value::Builtin(_) => {
            Err(not_subsettable(x, Some(&replacement_error_call())))
        }
        _ => {
            let is_expr = matches!(x, Value::Expr(_));
            let mut coerced: Option<Value> = None;
            if !is_expr && !matches!(x, Value::Null | Value::List(_)) {
                if !x.is_atomic() {
                    return Err(Flow::error(format!(
                        "cannot coerce type '{}' to vector of type 'list'",
                        x.type_name()
                    )));
                }
                interp.warning("Coercing LHS to a list")?;
                coerced = Some(coerce_lhs(x, 19));
            }
            if let Some(c) = coerced {
                *x = c;
            }
            let names: Option<Vec<RStr>> = match get_attrib(x, s::NAMES) {
                Value::Str(n) => Some(n.data.clone()),
                _ => None,
            };
            let nx = x.len();
            let nl = nlist.name();
            let imatch = names
                .as_ref()
                .and_then(|n| n.iter().position(|v| v.as_deref().is_some_and(|v| !v.is_empty() && *v == *nl)));
            if val.is_null() {
                if let Some(im) = imatch {
                    let mut nn = names.unwrap();
                    nn.remove(im);
                    let attrs = rebuilt_attrs(x, Some(Value::strs(nn)));
                    if let Value::List(v) | Value::Expr(v) = x {
                        Rc::make_mut(v).data.remove(im);
                    }
                    x.set_attrs(attrs);
                }
            } else if let Some(im) = imatch {
                *elt_mut(x, im) = val;
            } else {
                let mut data: Vec<Value> = match x {
                    Value::List(v) | Value::Expr(v) => v.data.clone(),
                    _ => vec![],
                };
                data.push(val);
                let mut nn = names.unwrap_or_else(|| vec![Some(Rc::from("")); nx]);
                nn.push(Some(nl));
                let attrs = rebuilt_attrs(x, Some(Value::strs(nn)));
                let mut ans = Value::list(data);
                ans.set_attrs(attrs);
                *x = ans;
            }
            Ok(())
        }
    }
}
