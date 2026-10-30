//! `identical()` (GNU R's identical.c).

use super::{arg, internal};
use crate::env::Env;
use crate::error::RResult;
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("identical", do_identical, 8)];

#[derive(Clone, Copy)]
pub struct IdFlags {
    pub num_eq: bool,
    pub single_na: bool,
    pub attrib_as_set: bool,
    pub ignore_env: bool,
    pub ignore_srcref: bool,
}

impl Default for IdFlags {
    fn default() -> Self {
        IdFlags { num_eq: true, single_na: true, attrib_as_set: true, ignore_env: false, ignore_srcref: true }
    }
}

pub fn identical(a: &Value, b: &Value) -> bool {
    identical_flags(a, b, IdFlags::default())
}

fn real_eq(x: f64, y: f64, f: IdFlags) -> bool {
    if x.is_nan() || y.is_nan() {
        if !x.is_nan() || !y.is_nan() {
            return false;
        }
        if f.single_na {
            return is_na_real(x) == is_na_real(y);
        }
        return x.to_bits() == y.to_bits();
    }
    if f.num_eq { x == y } else { x.to_bits() == y.to_bits() }
}

fn attrs_identical(a: &Value, b: &Value, f: IdFlags) -> bool {
    let empty = Attrs::default();
    let aa = a.attrs().cloned().or_else(|| a.ref_attrs()).unwrap_or_default();
    let ba = b.attrs().cloned().or_else(|| b.ref_attrs()).unwrap_or_default();
    let filt = |x: &Attrs| -> Vec<(crate::sym::Sym, Value)> {
        x.0.iter().filter(|(k, _)| !(f.ignore_srcref && *k == s::SRCREF)).cloned().collect()
    };
    let (av, bv) = (filt(&aa), filt(&ba));
    let _ = empty;
    if av.len() != bv.len() {
        return false;
    }
    // R: identical.c:R_compute_identical — row.names are compared through
    // getAttrib (both ATTR_AS_SET paths), which expands the compact
    // c(NA, ±n) form to 1:n, so stored signs don't matter.
    let val = |o: &Value, k: crate::sym::Sym, v: &Value| -> Value {
        if k == s::ROW_NAMES { crate::attrib::get_attrib(o, s::ROW_NAMES) } else { v.clone() }
    };
    if f.attrib_as_set {
        av.iter()
            .all(|(k, v)| bv.iter().any(|(k2, v2)| k == k2 && identical_flags(&val(a, *k, v), &val(b, *k2, v2), f)))
    } else {
        av.iter()
            .zip(bv.iter())
            .all(|((k, v), (k2, v2))| k == k2 && identical_flags(&val(a, *k, v), &val(b, *k2, v2), f))
    }
}

pub fn identical_flags(a: &Value, b: &Value, f: IdFlags) -> bool {
    if a.ptr_eq(b) && !matches!(a, Value::Real(_)) {
        return true;
    }
    // R: identical.c:R_compute_identical (TYPEOF, IS_S4_OBJECT)
    if a.type_name() != b.type_name() || a.is_s4() != b.is_s4() {
        return false;
    }
    if !matches!(a, Value::Env(_)) && !attrs_identical(a, b, f) {
        return false;
    }
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Sym(x), Value::Sym(y)) => x == y,
        (Value::Lgl(x), Value::Lgl(y)) => x.data == y.data,
        (Value::Int(x), Value::Int(y)) => x.data == y.data,
        (Value::Real(x), Value::Real(y)) => {
            x.data.len() == y.data.len() && x.data.iter().zip(y.data.iter()).all(|(p, q)| real_eq(*p, *q, f))
        }
        (Value::Cplx(x), Value::Cplx(y)) => {
            x.data.len() == y.data.len()
                && x.data.iter().zip(y.data.iter()).all(|(p, q)| real_eq(p.re, q.re, f) && real_eq(p.im, q.im, f))
        }
        (Value::Str(x), Value::Str(y)) => x.data == y.data,
        (Value::Raw(x), Value::Raw(y)) => x.data == y.data,
        (Value::List(x), Value::List(y)) | (Value::Expr(x), Value::Expr(y)) => {
            x.data.len() == y.data.len() && x.data.iter().zip(y.data.iter()).all(|(p, q)| identical_flags(p, q, f))
        }
        (Value::Pairlist(x), Value::Pairlist(y)) => {
            x.items.len() == y.items.len()
                && x.items
                    .iter()
                    .zip(y.items.iter())
                    .all(|(p, q)| p.tag == q.tag && identical_flags(&p.value, &q.value, f))
        }
        (Value::Lang(x), Value::Lang(y)) => {
            identical_flags(&x.fun, &y.fun, f)
                && x.args.len() == y.args.len()
                && x.args
                    .iter()
                    .zip(y.args.iter())
                    .all(|(p, q)| p.tag == q.tag && identical_flags(&p.value, &q.value, f))
        }
        (Value::Closure(x), Value::Closure(y)) => {
            x.formals.len() == y.formals.len()
                && x.formals
                    .iter()
                    .zip(y.formals.iter())
                    .all(|(p, q)| p.tag == q.tag && identical_flags(&p.value, &q.value, f))
                && identical_flags(&x.body, &y.body, f)
                && (f.ignore_env || x.env.ptr_eq(&y.env))
        }
        // R: identical.c:R_compute_identical, BCODESXP. The expression tag
        // is NULL for loaded bytecode; the source is constant zero.
        (Value::Bytecode(x), Value::Bytecode(y)) => {
            identical_flags(&x.code, &y.code, f)
                && x.consts.len() == y.consts.len()
                && x.consts.iter().zip(&y.consts).all(|(x, y)| identical_flags(x, y, f))
        }
        (Value::Builtin(x), Value::Builtin(y)) => std::ptr::eq(*x, *y),
        (Value::Env(x), Value::Env(y)) => x.ptr_eq(y),
        (Value::Promise(x), Value::Promise(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::ExtPtr(x), Value::ExtPtr(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Dots(x), Value::Dots(y)) => std::rc::Rc::ptr_eq(x, y),
        // OBJSXP: the attributes (slots) were compared above.
        (Value::Obj(_), Value::Obj(_)) => true,
        _ => false,
    }
}

fn do_identical(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let b = |i: usize, d: bool| -> bool {
        let v = arg(&args, i);
        if v.is_null() {
            return d;
        }
        let l = crate::coerce::as_logical(&v);
        if l == NA_LOGICAL { d } else { l != 0 }
    };
    let f = IdFlags {
        num_eq: b(2, true),
        single_na: b(3, true),
        attrib_as_set: b(4, true),
        ignore_env: b(6, false),
        ignore_srcref: b(7, true),
    };
    Ok(Value::lgl(identical_flags(&arg(&args, 0), &arg(&args, 1), f)))
}
