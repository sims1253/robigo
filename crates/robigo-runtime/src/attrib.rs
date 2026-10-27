//! Attribute access with R's validation rules (GNU R's attrib.c).
//!
//! `get_attrib`/`set_attrib` are the equivalents of `getAttrib` and
//! `setAttrib`: names of calls and pairlists live in their tags, 1-d arrays
//! keep names in their dimnames, compact row names `c(NA, -n)` expand to
//! `1:n`, and setting `names`, `dim`, `dimnames`, `class` and `row.names`
//! validates and coerces the value.

use std::rc::Rc;

use crate::coerce::{coerce_quiet, to_str};
use crate::error::{Flow, RResult};
use crate::sym::{Sym, s};
use crate::value::*;

pub fn is_one_dim_array(v: &Value) -> bool {
    matches!(v.get_attr(s::DIM), Some(d) if d.len() == 1)
}

/// `getAttrib(vec, name)`.
pub fn get_attrib(v: &Value, name: Sym) -> Value {
    if name == s::NAMES {
        if is_one_dim_array(v) {
            if let Some(Value::List(dn)) = v.get_attr(s::DIMNAMES) {
                if let Some(first) = dn.data.first() {
                    if !first.is_null() {
                        return first.clone();
                    }
                }
            }
        }
        if matches!(v, Value::Pairlist(_) | Value::Lang(_) | Value::Dots(_)) {
            return v.names().unwrap_or(Value::Null);
        }
    }
    let a = v.get_attr(name).unwrap_or(Value::Null);
    if name == s::ROW_NAMES {
        if let Value::Int(iv) = &a {
            if iv.data.len() == 2 && iv.data[0] == NA_INTEGER {
                let n = iv.data[1].unsigned_abs() as i32;
                return Value::ints((1..=n).collect());
            }
        }
    }
    a
}

pub fn names_vec(v: &Value) -> Option<Vec<RStr>> {
    match get_attrib(v, s::NAMES) {
        Value::Str(s) => Some(s.data.clone()),
        _ => None,
    }
}

pub fn dims(v: &Value) -> Option<Vec<i32>> {
    match v.get_attr(s::DIM) {
        Some(Value::Int(d)) => Some(d.data.clone()),
        _ => None,
    }
}

/// `setAttrib(vec, name, val)`; returns the modified object.
pub fn set_attrib(mut v: Value, name: Sym, val: Value) -> RResult<Value> {
    if val.is_null() {
        return Ok(remove_attrib(v, name));
    }
    if v.is_null() {
        return Err(Flow::error("attempt to set an attribute on NULL"));
    }
    if name == s::NAMES {
        return names_gets(v, val);
    }
    if name == s::DIM {
        return dim_gets(v, val);
    }
    if name == s::DIMNAMES {
        return dimnames_gets(v, val);
    }
    if name == s::CLASS {
        return class_gets(v, val);
    }
    if name == s::ROW_NAMES {
        return row_names_gets(v, val);
    }
    if name == s::COMMENT && !matches!(val, Value::Str(_)) {
        return Err(Flow::error("attempt to set invalid 'comment' attribute"));
    }
    install_attrib(&mut v, name, val)?;
    Ok(v)
}

pub fn install_attrib(v: &mut Value, name: Sym, val: Value) -> RResult<()> {
    match v {
        Value::Sym(_) => return Err(Flow::error("cannot set attribute on a 'symbol'")),
        Value::Null => return Err(Flow::error("attempt to set an attribute on NULL")),
        _ => {}
    }
    v.set_attr_raw(name, val);
    Ok(())
}

pub fn remove_attrib(mut v: Value, name: Sym) -> Value {
    if name == s::NAMES {
        match &mut v {
            Value::Pairlist(p) => {
                for a in Rc::make_mut(p).items.iter_mut() {
                    a.tag = None;
                }
                return v;
            }
            Value::Lang(l) => {
                for a in Rc::make_mut(l).args.iter_mut() {
                    a.tag = None;
                }
                return v;
            }
            _ => {}
        }
    }
    if name == s::DIM {
        v.set_attr_raw(s::DIMNAMES, Value::Null);
    }
    v.set_attr_raw(name, Value::Null);
    v
}

fn names_gets(mut v: Value, val: Value) -> RResult<Value> {
    let n = v.len();
    let mut names: Vec<RStr> = match &val {
        Value::Pairlist(_) | Value::List(_) if !matches!(val, Value::Str(_)) => {
            let items = crate::coerce::to_list(&val);
            items.iter().take(n).map(|x| to_str(x).first().cloned().flatten()).collect()
        }
        _ => {
            if val.inherits("factor") {
                match crate::builtins::envir::as_character_factor(&val) {
                    Value::Str(s) => s.data.clone(),
                    _ => vec![],
                }
            } else {
                to_str(&val)
            }
        }
    };
    if !(v.is_vector() || matches!(v, Value::Pairlist(_) | Value::Lang(_))) {
        return Err(Flow::error("names() applied to a non-vector"));
    }
    if names.len() < n {
        names.resize(n, None);
    }
    if names.len() != n {
        return Err(Flow::error(format!(
            "'names' attribute [{}] must be the same length as the vector [{}]",
            names.len(),
            n
        )));
    }
    if is_one_dim_array(&v) {
        let dn = Value::list(vec![Value::strs(names)]);
        return dimnames_gets(v, dn);
    }
    match &mut v {
        Value::Pairlist(p) => {
            for (a, nm) in Rc::make_mut(p).items.iter_mut().zip(names) {
                a.tag = nm.filter(|s| !s.is_empty()).map(|s| Sym::new(&s));
            }
            Ok(v)
        }
        Value::Lang(l) => {
            let l = Rc::make_mut(l);
            // The first name belongs to the function position; it cannot be
            // stored, so only argument tags are set.
            for (a, nm) in l.args.iter_mut().zip(names.into_iter().skip(1)) {
                a.tag = nm.filter(|s| !s.is_empty()).map(|s| Sym::new(&s));
            }
            Ok(v)
        }
        _ => {
            v.set_attr_raw(s::NAMES, Value::strs(names));
            Ok(v)
        }
    }
}

fn dim_gets(mut v: Value, val: Value) -> RResult<Value> {
    if !(v.is_vector() || matches!(v, Value::Pairlist(_))) {
        return Err(Flow::error("invalid first argument, must be vector (list or atomic)"));
    }
    if !val.is_atomic() {
        return Err(Flow::error("invalid second argument, must be vector or NULL"));
    }
    let d = match coerce_quiet(&val, VType::Int) {
        Value::Int(x) => x.data.clone(),
        _ => vec![],
    };
    if d.is_empty() {
        return Err(Flow::error("length-0 dimension vector is invalid"));
    }
    let mut total: i64 = 1;
    for &x in &d {
        if x == NA_INTEGER {
            return Err(Flow::error("the dims contain missing values"));
        }
        if x < 0 {
            return Err(Flow::error("the dims contain negative values"));
        }
        total *= x as i64;
    }
    if total != v.len() as i64 {
        return Err(Flow::error(format!("dims [product {}] do not match the length of object [{}]", total, v.len())));
    }
    v.set_attr_raw(s::DIMNAMES, Value::Null);
    v.set_attr_raw(s::DIM, Value::ints(d));
    Ok(v)
}

fn as_char_simpl(x: &Value) -> Value {
    if x.is_empty() {
        return Value::Null;
    }
    if x.inherits("factor") {
        return crate::builtins::envir::as_character_factor(x);
    }
    match x {
        Value::Str(_) => x.clone(),
        _ => Value::strs(to_str(x)),
    }
}

fn dimnames_gets(mut v: Value, val: Value) -> RResult<Value> {
    let d = dims(&v);
    let is_pairlist = matches!(v, Value::Pairlist(_));
    if d.is_none() && !is_pairlist {
        return Err(Flow::error("'dimnames' applied to non-array"));
    }
    let mut items: Vec<Value> = match &val {
        Value::List(l) => l.data.clone(),
        Value::Pairlist(p) => p.items.iter().map(|a| a.value.clone()).collect(),
        _ => return Err(Flow::error("'dimnames' must be a list")),
    };
    let dn_names = val.get_attr(s::NAMES);
    let d = d.unwrap_or_default();
    let k = d.len();
    if k < items.len() {
        return Err(Flow::error(format!("length of 'dimnames' [{}] must match that of 'dims' [{}]", items.len(), k)));
    }
    if items.is_empty() {
        v.set_attr_raw(s::DIMNAMES, Value::Null);
        return Ok(v);
    }
    let mut names_out = dn_names.as_ref().and_then(|n| match n {
        Value::Str(s) => Some(s.data.clone()),
        _ => None,
    });
    if items.len() < k {
        items.resize(k, Value::Null);
        if let Some(n) = names_out.as_mut() {
            n.resize(k, Some(Rc::from("")));
        }
    }
    for (i, it) in items.iter_mut().enumerate() {
        if !it.is_null() {
            if !it.is_vector() {
                return Err(Flow::error(format!(
                    "invalid type ({}) for 'dimnames' (must be a vector)",
                    it.type_name()
                )));
            }
            if d[i] as usize != it.len() && !it.is_empty() {
                return Err(Flow::error(format!("length of 'dimnames' [{}] not equal to array extent", i + 1)));
            }
            *it = as_char_simpl(it);
        }
    }
    let mut dn = Value::list(items);
    if let Some(n) = names_out {
        dn.set_attr_raw(s::NAMES, Value::strs(n));
    }
    v.set_attr_raw(s::DIMNAMES, dn);
    Ok(v)
}

pub fn class_gets(mut v: Value, val: Value) -> RResult<Value> {
    match &val {
        Value::Null => {
            v.set_attr_raw(s::CLASS, Value::Null);
            Ok(v)
        }
        Value::Str(c) => {
            if c.data.is_empty() {
                v.set_attr_raw(s::CLASS, Value::Null);
                return Ok(v);
            }
            if v.is_null() {
                return Err(Flow::error("attempt to set an attribute on NULL"));
            }
            if c.data.iter().any(|x| x.as_deref() == Some("factor")) && !matches!(v, Value::Int(_)) {
                return Err(Flow::error("adding class \"factor\" to an invalid object"));
            }
            let mut c2 = val.clone();
            c2.clear_attrs();
            v.set_attr_raw(s::CLASS, c2);
            Ok(v)
        }
        _ => Err(Flow::error("attempt to set invalid 'class' attribute")),
    }
}

fn row_names_gets(mut v: Value, val: Value) -> RResult<Value> {
    if let Value::Real(r) = &val {
        if r.data.len() == 2 && r.data[0].is_nan() {
            let iv = coerce_quiet(&val, VType::Int);
            v.set_attr_raw(s::ROW_NAMES, iv);
            return Ok(v);
        }
    }
    match &val {
        Value::Int(iv) => {
            let n = iv.data.len();
            let mut compact: Option<i32> = None;
            if n == 2 && iv.data[0] == NA_INTEGER {
                compact = Some(iv.data[1]);
            } else if n > 2 && iv.data.iter().enumerate().all(|(i, &x)| x == i as i32 + 1) {
                compact = Some(n as i32);
            }
            match compact {
                Some(k) => v.set_attr_raw(s::ROW_NAMES, Value::ints(vec![NA_INTEGER, k])),
                None => v.set_attr_raw(s::ROW_NAMES, val.clone()),
            }
            Ok(v)
        }
        Value::Str(_) => {
            v.set_attr_raw(s::ROW_NAMES, val.clone());
            Ok(v)
        }
        _ => Err(Flow::error(format!("row names must be 'character' or 'integer', not '{}'", val.type_name()))),
    }
}

/// `copyMostAttrib`: all attributes except names, dim and dimnames.
pub fn copy_most_attrib(from: &Value, to: &mut Value) {
    if let Some(a) = from.attrs() {
        for (k, val) in &a.0 {
            if *k != s::NAMES && *k != s::DIM && *k != s::DIMNAMES {
                to.set_attr_raw(*k, val.clone());
            }
        }
    }
}

/// `copyMostAttribNoTs`: like [`copy_most_attrib`], also leaving out
/// `tsp` and removing "ts" from the class.
pub fn copy_most_attrib_no_ts(from: &Value, to: &mut Value) {
    let Some(a) = from.attrs() else { return };
    let tsp = Sym::new("tsp");
    for (k, val) in &a.0 {
        if *k == s::CLASS {
            if let Value::Str(cl) = val {
                if cl.data.iter().any(|c| c.as_deref() == Some("ts")) {
                    if cl.data.len() > 1 {
                        let rest: Vec<RStr> = cl.data.iter().filter(|c| c.as_deref() != Some("ts")).cloned().collect();
                        to.set_attr_raw(*k, Value::strs(rest));
                    }
                    continue;
                }
            }
            to.set_attr_raw(*k, val.clone());
        } else if *k != s::NAMES && *k != tsp && *k != s::DIM && *k != s::DIMNAMES {
            to.set_attr_raw(*k, val.clone());
        }
    }
}

/// `attributes(x)` as a named list (names first).
pub fn attributes_list(v: &Value) -> Value {
    let mut names: Vec<RStr> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if matches!(v, Value::Pairlist(_) | Value::Lang(_)) {
        if let Some(n) = v.names() {
            names.push(rstr("names"));
            vals.push(n);
        }
    }
    let attrs = v.attrs().cloned().or_else(|| v.ref_attrs());
    if let Some(a) = attrs {
        for (k, _) in &a.0 {
            names.push(Some(k.name()));
            vals.push(get_attrib(v, *k));
        }
    }
    if vals.is_empty() {
        return Value::Null;
    }
    let mut l = Value::list(vals);
    l.set_attr_raw(s::NAMES, Value::strs(names));
    l
}

/// `class(x)` (R_data_class with singleString = FALSE).
pub fn class_of(v: &Value) -> Value {
    Value::strs(crate::dispatch::implicit_class(v).into_iter().map(Some).collect())
}
