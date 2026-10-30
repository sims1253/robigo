//! `c()`, `unlist()`, `list()`, `length()` and friends (GNU R's bind.c and
//! parts of builtin.c).

use super::{arg, check_arity, internal, prim};
use crate::attrib::{get_attrib, set_attrib};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("c", do_c, -1),
    internal("unlist", do_unlist, 3),
    prim("list", do_list, -1),
    prim("length", do_length, 1),
    prim("length<-", do_lengthgets, 2),
    internal("lengths", do_lengths, 2),
    internal("vector", do_vector, 2),
];

// ----------------------------------------------------------------------
// AnswerType and the *Answer functions

const F_RAW: u32 = 1;
const F_LGL: u32 = 2;
const F_INT: u32 = 16;
const F_REAL: u32 = 32;
const F_CPLX: u32 = 64;
const F_STR: u32 = 128;
const F_LIST: u32 = 256;
const F_EXPR: u32 = 512;

struct BindData {
    flags: u32,
    length: usize,
    nnames: bool,
}

fn has_names(x: &Value) -> bool {
    match x {
        Value::Pairlist(p) => p.items.iter().any(|a| a.tag.is_some()),
        x if x.is_vector() => !get_attrib(x, s::NAMES).is_null(),
        _ => false,
    }
}

fn answer_type(x: &Value, recurse: bool, usenames: bool, d: &mut BindData) {
    match x {
        Value::Null => {}
        Value::Raw(v) => {
            d.flags |= F_RAW;
            d.length += v.data.len();
        }
        Value::Lgl(v) => {
            d.flags |= F_LGL;
            d.length += v.data.len();
        }
        Value::Int(v) => {
            d.flags |= F_INT;
            d.length += v.data.len();
        }
        Value::Real(v) => {
            d.flags |= F_REAL;
            d.length += v.data.len();
        }
        Value::Cplx(v) => {
            d.flags |= F_CPLX;
            d.length += v.data.len();
        }
        Value::Str(v) => {
            d.flags |= F_STR;
            d.length += v.data.len();
        }
        Value::List(v) | Value::Expr(v) => {
            if recurse {
                if usenames && !d.nnames && !get_attrib(x, s::NAMES).is_null() {
                    d.nnames = true;
                }
                for e in &v.data {
                    if usenames && !d.nnames {
                        d.nnames = has_names(e);
                    }
                    answer_type(e, recurse, usenames, d);
                }
            } else {
                d.flags |= if matches!(x, Value::Expr(_)) { F_EXPR } else { F_LIST };
                d.length += v.data.len();
            }
        }
        Value::Pairlist(p) => {
            if recurse {
                for a in &p.items {
                    if usenames && !d.nnames {
                        d.nnames = a.tag.is_some() || has_names(&a.value);
                    }
                    answer_type(&a.value, recurse, usenames, d);
                }
            } else {
                d.flags |= F_LIST;
                d.length += p.items.len();
            }
        }
        _ => {
            d.flags |= F_LIST;
            d.length += 1;
        }
    }
}

fn list_answer(x: &Value, recurse: bool, out: &mut Vec<Value>) {
    match x {
        Value::Null => {}
        Value::Lgl(v) => out.extend(v.data.iter().map(|&e| Value::lgl_raw(e))),
        Value::Int(v) => out.extend(v.data.iter().map(|&e| Value::int(e))),
        Value::Real(v) => out.extend(v.data.iter().map(|&e| Value::real(e))),
        Value::Cplx(v) => out.extend(v.data.iter().map(|&e| Value::cplx(e))),
        Value::Str(v) => out.extend(v.data.iter().map(|e| Value::strs(vec![e.clone()]))),
        Value::Raw(v) => out.extend(v.data.iter().map(|&e| Value::raws(vec![e]))),
        Value::List(v) | Value::Expr(v) => {
            if recurse {
                for e in &v.data {
                    list_answer(e, recurse, out);
                }
            } else {
                out.extend(v.data.iter().cloned());
            }
        }
        Value::Pairlist(p) => {
            if recurse {
                for a in &p.items {
                    list_answer(&a.value, recurse, out);
                }
            } else {
                out.extend(p.items.iter().map(|a| a.value.clone()));
            }
        }
        other => out.push(other.clone()),
    }
}

/// Flatten atomic leaves (all args are atomic, NULL or lists of atomics).
fn atomic_leaves<'a>(x: &'a Value, out: &mut Vec<&'a Value>) {
    match x {
        Value::List(v) | Value::Expr(v) => {
            for e in &v.data {
                atomic_leaves(e, out);
            }
        }
        Value::Pairlist(p) => {
            for a in &p.items {
                atomic_leaves(&a.value, out);
            }
        }
        Value::Null => {}
        other => out.push(other),
    }
}

fn combine_atomic(leaves: &[&Value], mode: VType) -> Value {
    match mode {
        VType::Raw => {
            let mut out = Vec::new();
            for l in leaves {
                if let Value::Raw(v) = l {
                    out.extend_from_slice(&v.data);
                }
            }
            Value::raws(out)
        }
        VType::Lgl => {
            let mut out = Vec::new();
            for l in leaves {
                match l {
                    Value::Lgl(v) => out.extend_from_slice(&v.data),
                    Value::Raw(v) => out.extend(v.data.iter().map(|&b| (b != 0) as Lgl)),
                    _ => {}
                }
            }
            Value::lgls(out)
        }
        VType::Int => {
            let mut out = Vec::new();
            for l in leaves {
                match l {
                    Value::Lgl(v) | Value::Int(v) => out.extend_from_slice(&v.data),
                    Value::Raw(v) => out.extend(v.data.iter().map(|&b| b as i32)),
                    _ => {}
                }
            }
            Value::ints(out)
        }
        VType::Real => {
            let mut out = Vec::new();
            for l in leaves {
                match l {
                    Value::Lgl(v) | Value::Int(v) => {
                        out.extend(v.data.iter().map(|&i| if i == NA_INTEGER { NA_REAL } else { i as f64 }))
                    }
                    Value::Real(v) => out.extend_from_slice(&v.data),
                    Value::Raw(v) => out.extend(v.data.iter().map(|&b| b as f64)),
                    _ => {}
                }
            }
            Value::reals(out)
        }
        VType::Cplx => {
            let mut out = Vec::new();
            for l in leaves {
                match l {
                    Value::Lgl(v) | Value::Int(v) => out.extend(v.data.iter().map(|&i| {
                        if i == NA_INTEGER { Complex::new(NA_REAL, 0.0) } else { Complex::new(i as f64, 0.0) }
                    })),
                    Value::Real(v) => out.extend(v.data.iter().map(|&x| Complex::new(x, 0.0))),
                    Value::Cplx(v) => out.extend_from_slice(&v.data),
                    Value::Raw(v) => out.extend(v.data.iter().map(|&b| Complex::new(b as f64, 0.0))),
                    _ => {}
                }
            }
            Value::cplxs(out)
        }
        _ => {
            let mut out = Vec::new();
            for l in leaves {
                match l {
                    Value::Str(v) => out.extend_from_slice(&v.data),
                    other if other.is_atomic() => out.extend(crate::coerce::to_str(other)),
                    other => out.extend(crate::coerce::to_str(other)),
                }
            }
            Value::strs(out)
        }
    }
}

// ----------------------------------------------------------------------
// names

thread_local! {
    /// The name NA_STRING: spelled "NA" (its CHAR) when names are built
    /// from it, itself (NA) when passed through; told apart by identity.
    static NA_NAME: crate::value::RString = crate::value::RString::from("NA");
}

fn na_name() -> crate::value::RString {
    NA_NAME.with(|n| n.clone())
}

/// A name as a CHARSXP: the NA name is NA.
fn name_rstr(s: &crate::value::RString) -> RStr {
    if NA_NAME.with(|n| crate::value::RString::ptr_eq(n, s)) { None } else { Some(s.clone()) }
}

fn item_name(names: &Option<Vec<RStr>>, i: usize) -> Option<crate::value::RString> {
    // R's `translateChar(NA_STRING)` is "NA": a missing name still names the
    // element (c(list(b = <named>) ) yields "b.NA").
    names.as_ref().and_then(|n| n.get(i).map(|s| s.clone().unwrap_or_else(na_name))).filter(|s| !s.is_empty())
}

// R: bind.c:NewBase
fn new_base(
    base: &Option<crate::value::RString>,
    tag: &Option<crate::value::RString>,
) -> Option<crate::value::RString> {
    let b = base.clone().filter(|s| !s.is_empty());
    let t = tag.clone().filter(|s| !s.is_empty());
    match (b, t) {
        (Some(b), Some(t)) => Some(crate::value::RString::from(format!("{}.{}", b, t))),
        (None, Some(t)) => Some(t),
        (Some(b), None) => Some(b),
        _ => None,
    }
}

// R: bind.c:NewName — NA_STRING's CHAR is "NA": it counts as a name, is
// spelled "NA" inside new names and is returned as itself
fn new_name(
    base: &Option<crate::value::RString>,
    tag: &Option<crate::value::RString>,
    seqno: usize,
    count: usize,
) -> RStr {
    let b = base.as_ref().filter(|s| !s.is_empty());
    let t = tag.as_ref().filter(|s| !s.is_empty());
    match (b, t) {
        (Some(b), Some(t)) => Some(crate::value::RString::from(format!("{}.{}", b, t))),
        (Some(b), None) if count == 1 => name_rstr(b),
        (Some(b), None) => Some(crate::value::RString::from(format!("{}{}", b, seqno))),
        (None, Some(t)) => name_rstr(t),
        (None, None) => Some(crate::value::RString::from("")),
    }
}

fn names_of(v: &Value) -> Option<Vec<RStr>> {
    match get_attrib(v, s::NAMES) {
        Value::Str(n) => Some(n.data.clone()),
        _ => None,
    }
}

fn names_count(v: &Value, recurse: bool, count: &mut usize) {
    let names = names_of(v);
    match v {
        Value::Null => {}
        Value::List(l) | Value::Expr(l) if recurse => {
            for (i, e) in l.data.iter().enumerate() {
                if *count > 1 {
                    break;
                }
                if item_name(&names, i).is_none() {
                    names_count(e, recurse, count);
                }
            }
        }
        Value::Pairlist(p) if recurse => {
            for a in &p.items {
                if *count > 1 {
                    break;
                }
                if a.tag.is_none() {
                    names_count(&a.value, recurse, count);
                }
            }
        }
        v if v.is_vector() || matches!(v, Value::Pairlist(_)) => {
            let n = v.len();
            *count += n.min(2);
        }
        _ => *count += 1,
    }
}

struct NameData {
    count: usize,
    seqno: usize,
}

fn extract_names(
    v: &Value,
    base: &Option<crate::value::RString>,
    tag: &Option<crate::value::RString>,
    recurse: bool,
    out: &mut Vec<RStr>,
    nd: &mut NameData,
) {
    let mut base = base.clone();
    let (saveseqno, savecount);
    if tag.is_some() {
        base = new_base(&base, tag);
        saveseqno = nd.seqno;
        savecount = nd.count;
        nd.count = 0;
        names_count(v, recurse, &mut nd.count);
        nd.seqno = 0;
    } else {
        saveseqno = 0;
        savecount = 0;
    }
    let names = names_of(v);
    match v {
        Value::Null => {}
        Value::Pairlist(p) => {
            for a in &p.items {
                let namei = a.tag.map(|t| t.name().into());
                if recurse {
                    extract_names(&a.value, &base, &namei, recurse, out, nd);
                } else {
                    nd.seqno += 1;
                    out.push(new_name(&base, &namei, nd.seqno, nd.count));
                }
            }
        }
        Value::List(l) | Value::Expr(l) => {
            for (i, e) in l.data.iter().enumerate() {
                let namei = item_name(&names, i);
                if recurse {
                    extract_names(e, &base, &namei, recurse, out, nd);
                } else {
                    nd.seqno += 1;
                    out.push(new_name(&base, &namei, nd.seqno, nd.count));
                }
            }
        }
        v if v.is_atomic() => {
            for i in 0..v.len() {
                let namei = item_name(&names, i);
                nd.seqno += 1;
                out.push(new_name(&base, &namei, nd.seqno, nd.count));
            }
        }
        _ => {
            nd.seqno += 1;
            out.push(new_name(&base, &None, nd.seqno, nd.count));
        }
    }
    if tag.is_some() {
        nd.count = savecount;
    }
    nd.seqno += saveseqno;
}

fn mode_of(flags: u32) -> VType {
    if flags & F_EXPR != 0 {
        VType::Expr
    } else if flags & F_LIST != 0 {
        VType::List
    } else if flags & F_STR != 0 {
        VType::Str
    } else if flags & F_CPLX != 0 {
        VType::Cplx
    } else if flags & F_REAL != 0 {
        VType::Real
    } else if flags & F_INT != 0 {
        VType::Int
    } else if flags & F_LGL != 0 {
        VType::Lgl
    } else if flags & F_RAW != 0 {
        VType::Raw
    } else {
        VType::Null
    }
}

/// The default method of `c()` on (tag, value) arguments.
pub fn c_default(args: &[Arg], recurse: bool, usenames: bool) -> Value {
    let mut d = BindData { flags: 0, length: 0, nnames: false };
    for a in args {
        if usenames && !d.nnames {
            d.nnames = a.tag.is_some() || has_names(&a.value);
        }
        answer_type(&a.value, recurse, usenames, &mut d);
    }
    let mode = mode_of(d.flags);
    let mut ans = match mode {
        VType::Null => return Value::Null,
        VType::List | VType::Expr => {
            let mut out = Vec::with_capacity(d.length);
            for a in args {
                list_answer(&a.value, recurse, &mut out);
            }
            if mode == VType::Expr { Value::expr_vec(out) } else { Value::list(out) }
        }
        _ => {
            let mut leaves = Vec::new();
            for a in args {
                atomic_leaves(&a.value, &mut leaves);
            }
            combine_atomic(&leaves, mode)
        }
    };
    if d.nnames && ans.len() > 0 {
        let mut names = Vec::with_capacity(ans.len());
        for a in args {
            let mut nd = NameData { count: 0, seqno: 0 };
            let tag = a.tag.map(|t| t.name().into());
            extract_names(&a.value, &None, &tag, recurse, &mut names, &mut nd);
        }
        names.resize(ans.len(), Some(crate::value::RString::from("")));
        ans.set_attr_raw(s::NAMES, Value::strs(names));
    }
    ans
}

fn do_c(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // Drop NULL arguments (except a leading one) before dispatch.
    let mut args: Vec<Arg> =
        args.into_iter().enumerate().filter(|(k, a)| *k == 0 || !a.value.is_null()).map(|(_, a)| a).collect();
    if args.first().is_some_and(|a| a.value.is_object()) {
        if let Some(r) = i.try_dispatch("c", call, &args, env)? {
            return Ok(r);
        }
    }
    let mut recurse = false;
    let mut usenames = true;
    args.retain(|a| {
        if let Some(t) = a.tag {
            let n = t.name();
            if &*n == "recursive" {
                let v = crate::coerce::as_logical(&a.value);
                if v != NA_LOGICAL {
                    recurse = v != 0;
                }
                return false;
            }
            if &*n == "use.names" {
                let v = crate::coerce::as_logical(&a.value);
                if v != NA_LOGICAL {
                    usenames = v != 0;
                }
                return false;
            }
        }
        true
    });
    Ok(c_default(&args, recurse, usenames))
}

fn do_unlist(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = i.try_dispatch("unlist", call, &args, env)? {
        return Ok(r);
    }
    let x = arg(&args, 0);
    let recurse = crate::coerce::as_logical(&arg(&args, 1)) != 0;
    let usenames = crate::coerce::as_logical(&arg(&args, 2)) != 0;
    let items: Vec<Arg> = match &x {
        Value::List(l) => {
            let names = names_of(&x);
            l.data
                .iter()
                .enumerate()
                .map(|(k, v)| Arg::new(item_name(&names, k).map(|n| crate::sym::Sym::new(&n)), v.clone()))
                .collect()
        }
        Value::Pairlist(p) => p.items.clone(),
        _ => return Ok(x),
    };
    let mut d = BindData { flags: 0, length: 0, nnames: false };
    if usenames && names_of(&x).is_some() {
        d.nnames = true;
    }
    for a in &items {
        if usenames && !d.nnames {
            d.nnames = a.tag.is_some() || has_names(&a.value);
        }
        answer_type(&a.value, recurse, usenames, &mut d);
    }
    let mode = mode_of(d.flags);
    let mut ans = match mode {
        VType::Null => return Ok(Value::Null),
        VType::List | VType::Expr => {
            let mut out = Vec::new();
            for a in &items {
                list_answer(&a.value, recurse, &mut out);
            }
            if mode == VType::Expr { Value::expr_vec(out) } else { Value::list(out) }
        }
        _ => {
            let mut leaves = Vec::new();
            for a in &items {
                atomic_leaves(&a.value, &mut leaves);
            }
            combine_atomic(&leaves, mode)
        }
    };
    if d.nnames && ans.len() > 0 {
        let mut names = Vec::new();
        let mut nd = NameData { count: 0, seqno: 0 };
        if !recurse {
            for a in &items {
                let tag = a.tag.map(|t| t.name().into());
                extract_names(&a.value, &None, &tag, recurse, &mut names, &mut nd);
            }
        } else {
            extract_names(&x, &None, &None, recurse, &mut names, &mut nd);
        }
        names.resize(ans.len(), Some(crate::value::RString::from("")));
        ans.set_attr_raw(s::NAMES, Value::strs(names));
    }
    Ok(ans)
}

fn do_list(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if !args.iter().any(|a| a.tag.is_some()) {
        return Ok(Value::list(args.into_iter().map(|a| a.value).collect()));
    }
    let names: Vec<RStr> = args
        .iter()
        .map(|a| Some(a.tag.map(|t| t.name().into()).unwrap_or_else(|| crate::value::RString::from(""))))
        .collect();
    let mut v = Value::list(args.into_iter().map(|a| a.value).collect());
    v.set_attr_raw(s::NAMES, Value::strs(names));
    Ok(v)
}

fn do_length(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "length", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("length", call, &args, env)? {
            return Ok(r);
        }
    }
    let n = args[0].value.len();
    if n > i32::MAX as usize {
        return Ok(Value::real(n as f64));
    }
    Ok(Value::int(n as i32))
}

/// `xlengthgets`: truncate or pad with NA (NULL for lists).
pub fn length_gets(x: &Value, n: usize) -> RResult<Value> {
    if !x.is_vector() && !x.is_null() && !matches!(x, Value::Pairlist(_)) {
        return Err(Flow::error("invalid argument"));
    }
    if x.is_null() && n > 0 {
        return Ok(Value::lgls(vec![NA_LOGICAL; n]));
    }
    let names = names_of(x);
    let mut out = match x {
        Value::Null => Value::Null,
        Value::Lgl(v) => Value::lgls(resize(&v.data, n, NA_LOGICAL)),
        Value::Int(v) => Value::ints(resize(&v.data, n, NA_INTEGER)),
        Value::Real(v) => Value::reals(resize(&v.data, n, NA_REAL)),
        Value::Cplx(v) => Value::cplxs(resize(&v.data, n, Complex::NA)),
        Value::Str(v) => Value::strs(resize(&v.data, n, None)),
        Value::Raw(v) => Value::raws(resize(&v.data, n, 0)),
        Value::List(v) => Value::list(resize(&v.data, n, Value::Null)),
        Value::Expr(v) => Value::expr_vec(resize(&v.data, n, Value::Null)),
        Value::Pairlist(p) => {
            let mut items = p.items.clone();
            items.resize(n, Arg::pos(Value::Null));
            Value::pairlist(items)
        }
        _ => unreachable!(),
    };
    if let Some(nm) = names {
        out.set_attr_raw(s::NAMES, Value::strs(resize(&nm, n, Some(crate::value::RString::from("")))));
    }
    Ok(out)
}

fn resize<T: Clone>(v: &[T], n: usize, fill: T) -> Vec<T> {
    let mut out: Vec<T> = v.iter().take(n).cloned().collect();
    out.resize(n, fill);
    out
}

fn do_lengthgets(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "length<-", call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("length<-", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    let v = &args[1].value;
    if v.len() != 1 {
        return Err(Flow::error_call(call.clone(), "wrong length for 'value' argument"));
    }
    let n = crate::coerce::as_real(v);
    if n.is_nan() || n < 0.0 {
        return Err(Flow::error_call(call.clone(), "invalid value"));
    }
    if !x.is_vector() && !x.is_null() {
        return Err(Flow::error_call(call.clone(), "invalid argument"));
    }
    length_gets(x, n as usize)
}

fn do_lengths(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let use_names = crate::coerce::as_logical(&arg(&args, 1)) != 0;
    let lens: Vec<i32> = match &x {
        Value::List(l) | Value::Expr(l) => {
            let mut out = Vec::with_capacity(l.data.len());
            for e in &l.data {
                if e.is_object() {
                    let f = i.find_fun(crate::sym::Sym::new("length"), &i.base.clone())?;
                    let r = i.call_value(&f, vec![Arg::pos(e.clone())], env)?;
                    out.push(crate::coerce::as_integer(&r));
                } else {
                    out.push(e.len() as i32);
                }
            }
            out
        }
        Value::Null => vec![],
        x if x.is_atomic() => vec![1; x.len()],
        _ => return Err(Flow::error_call(call.clone(), "'x' must be a list or atomic vector")),
    };
    let mut v = Value::ints(lens);
    if use_names {
        if let Some(n) = x.get_attr(s::NAMES) {
            v.set_attr_raw(s::NAMES, n);
        }
        if let Some(d) = x.get_attr(s::DIM) {
            v = set_attrib(v, s::DIM, d)?;
            if let Some(dn) = x.get_attr(s::DIMNAMES) {
                v = set_attrib(v, s::DIMNAMES, dn)?;
            }
        }
    }
    Ok(v)
}

fn do_vector(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mode = match arg(&args, 0) {
        Value::Str(m) if m.data.len() == 1 => m.data[0].clone().unwrap_or_else(|| crate::value::RString::from("NA")),
        _ => return Err(Flow::error_call(call.clone(), "vector: cannot make a vector of mode 'NA'.")),
    };
    let n = crate::coerce::as_real(&arg(&args, 1));
    if n.is_nan() || n < 0.0 {
        return Err(Flow::error_call(call.clone(), "vector: cannot make a vector of negative length"));
    }
    let n = n as usize;
    Ok(match &*mode {
        "logical" => Value::lgls(vec![FALSE; n]),
        "integer" => Value::ints(vec![0; n]),
        "numeric" | "double" => Value::reals(vec![0.0; n]),
        "complex" => Value::cplxs(vec![Complex::default(); n]),
        "character" => Value::strs(vec![Some(crate::value::RString::from("")); n]),
        "raw" => Value::raws(vec![0; n]),
        "list" => Value::list(vec![Value::Null; n]),
        "expression" => Value::expr_vec(vec![Value::Null; n]),
        "pairlist" => Value::pairlist(vec![Arg::pos(Value::Null); n]),
        "any" => Value::lgls(vec![FALSE; n]),
        other => {
            return Err(Flow::error_call(call.clone(), format!("vector: cannot make a vector of mode '{}'.", other)));
        }
    })
}
