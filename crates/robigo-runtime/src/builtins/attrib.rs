//! Attribute builtins (GNU R's attrib.c, plus `class<-` from coerce.c and
//! `unclass`/`inherits` from objects.c).

use super::{arg, check_arity, internal, match_args, prim};
use crate::attrib::*;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("attr", do_attr, -1),
    prim("attr<-", do_attrgets, 3),
    prim("attributes", do_attributes, 1),
    prim("attributes<-", do_attributesgets, 2),
    prim("names", do_names, 1),
    prim("names<-", do_namesgets, 2),
    prim("dim", do_dim, 1),
    prim("dim<-", do_dimgets, 2),
    prim("dimnames", do_dimnames, 1),
    prim("dimnames<-", do_dimnamesgets, 2),
    prim("class", do_class, 1),
    prim("class<-", do_set_class, 2),
    prim("oldClass", do_oldclass, 1),
    prim("oldClass<-", do_oldclassgets, 2),
    prim("levels<-", do_levelsgets, 2),
    prim("unclass", do_unclass, 1),
    internal("inherits", do_inherits, 3),
    internal("comment", do_comment, 1),
    internal("comment<-", do_commentgets, 2),
    internal("shortRowNames", do_short_row_names, 2),
    internal("copyDFattr", do_copy_df_attr, 2),
];

fn take(args: &mut [Arg], i: usize) -> Value {
    args.get_mut(i).map(|a| std::mem::replace(&mut a.value, Value::Null)).unwrap_or(Value::Null)
}

fn do_attr(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let nargs = args.len();
    let m = match_args(args, &["x", "which", "exact"], call)?;
    if !(2..=3).contains(&nargs) {
        return Err(Flow::error_call(call.clone(), "either 2 or 3 arguments are required"));
    }
    let x = m.vals[0].clone().unwrap_or(Value::Null);
    let which = match &m.vals[1] {
        Some(Value::Str(w)) => w.clone(),
        _ => return Err(Flow::error_call(call.clone(), "'which' must be of mode character")),
    };
    if which.data.len() != 1 {
        return Err(Flow::error_call(call.clone(), "exactly one attribute 'which' must be given"));
    }
    let exact = match &m.vals[2] {
        Some(v) => crate::coerce::as_logical(v) == TRUE,
        None => false,
    };
    let Some(want) = which.data[0].clone() else { return Ok(Value::Null) };
    #[derive(PartialEq)]
    enum M {
        None,
        Partial,
        Partial2,
        Full,
    }
    let mut matched = M::None;
    let mut tag: Option<Sym> = None;
    let attrs = x.attrs().cloned().or_else(|| x.ref_attrs()).unwrap_or_default();
    for (k, _) in &attrs.0 {
        let kn = k.name();
        if kn.starts_with(&*want) {
            if kn.len() == want.len() {
                tag = Some(*k);
                matched = M::Full;
                break;
            } else if matched == M::Partial || matched == M::Partial2 {
                matched = M::Partial2;
            } else {
                tag = Some(*k);
                matched = M::Partial;
            }
        }
    }
    if matched == M::Partial2 {
        return Ok(Value::Null);
    }
    if matched != M::Full && "names".starts_with(&*want) {
        if want.len() == 5 {
            tag = Some(s::NAMES);
            matched = M::Full;
        } else if matched == M::None && !exact {
            return Ok(get_attrib(&x, s::NAMES));
        } else if matched == M::Partial && tag != Some(s::NAMES) && !get_attrib(&x, s::NAMES).is_null() {
            return Ok(Value::Null);
        }
    }
    if matched == M::None || (exact && matched != M::Full) {
        return Ok(Value::Null);
    }
    Ok(get_attrib(&x, tag.unwrap()))
}

fn do_attrgets(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut m = match_args(args, &["x", "which", "value"], call)?;
    let x = m.or_null(0);
    let name = match m.get(1) {
        Some(Value::Str(n)) if n.data.len() == 1 && n.data[0].is_some() => n.data[0].clone().unwrap(),
        _ => return Err(Flow::error("'name' must be non-null character string")),
    };
    let val = m.or_null(2);
    set_attrib(x, Sym::new(&name), val)
}

fn do_attributes(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "attributes", call)?;
    Ok(attributes_list(&args[0].value))
}

fn do_attributesgets(_: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "attributes<-", call)?;
    let mut object = take(&mut args, 0);
    let attrs = take(&mut args, 1);
    let (vals, names): (Vec<Value>, Vec<RStr>) = match &attrs {
        Value::Null => (vec![], vec![]),
        Value::List(l) => {
            let names = match attrs.get_attr(s::NAMES) {
                Some(Value::Str(n)) => n.data.clone(),
                _ => {
                    if l.data.is_empty() {
                        vec![]
                    } else {
                        return Err(Flow::error("attributes must be named"));
                    }
                }
            };
            for (i, n) in names.iter().enumerate().skip(1) {
                if n.as_deref().is_none_or(|s| s.is_empty()) {
                    return Err(Flow::error(format!("all attributes must have names [{} does not]", i + 1)));
                }
            }
            (l.data.clone(), names)
        }
        _ => return Err(Flow::error("attributes must be a list or NULL")),
    };
    if object.is_null() {
        if vals.is_empty() {
            return Ok(Value::Null);
        }
        object = Value::list(vec![]);
    }
    if matches!(object, Value::Pairlist(_) | Value::Lang(_)) {
        object = remove_attrib(object, s::NAMES);
    }
    object.clear_attrs();
    let dim_i = names.iter().position(|n| n.as_deref() == Some("dim"));
    if let Some(i) = dim_i {
        object = set_attrib(object, s::DIM, vals[i].clone())?;
    }
    for (i, (n, v)) in names.iter().zip(vals.iter()).enumerate() {
        if Some(i) == dim_i {
            continue;
        }
        let n = n.as_deref().unwrap_or("");
        object = set_attrib(object, Sym::new(n), v.clone())?;
    }
    Ok(object)
}

fn do_names(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "names", call)?;
    if let Some(r) = i.try_dispatch("names", call, &args, env)? {
        return Ok(r);
    }
    let x = &args[0].value;
    Ok(match x {
        Value::Env(e) => {
            let names: Vec<String> = e.symbols().into_iter().map(|s| s.name().to_string()).collect();
            Value::strs_from(names)
        }
        x if x.is_vector() || matches!(x, Value::Pairlist(_) | Value::Lang(_) | Value::Dots(_)) => {
            get_attrib(x, s::NAMES)
        }
        _ => Value::Null,
    })
}

fn do_namesgets(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "names<-", call)?;
    if let Some(r) = i.try_dispatch("names<-", call, &args, env)? {
        return Ok(r);
    }
    let x = take(&mut args, 0);
    let mut names = take(&mut args, 1);
    if names.is_null() && get_attrib(&x, s::NAMES).is_null() {
        return Ok(x);
    }
    if !names.is_null() && !(matches!(names, Value::Str(_)) && !names.has_attrs()) {
        // as.character(names), with dispatch
        let f = i.find_fun(Sym::new("as.character"), &i.base.clone())?;
        names = i.call_value(&f, vec![Arg::pos(names)], env)?;
    }
    if names.is_null() && is_one_dim_array(&x) {
        return set_attrib(x, s::DIMNAMES, Value::Null);
    }
    set_attrib(x, s::NAMES, names)
}

fn do_dim(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "dim", call)?;
    if let Some(r) = i.try_dispatch("dim", call, &args, env)? {
        return Ok(r);
    }
    Ok(get_attrib(&args[0].value, s::DIM))
}

fn do_dimgets(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "dim<-", call)?;
    if let Some(r) = i.try_dispatch("dim<-", call, &args, env)? {
        return Ok(r);
    }
    let x = take(&mut args, 0);
    let d = take(&mut args, 1);
    if d.is_null() && x.get_attr(s::DIM).is_none() && x.get_attr(s::NAMES).is_none() {
        return Ok(x);
    }
    let x = set_attrib(x, s::DIM, d)?;
    Ok(remove_attrib(x, s::NAMES))
}

fn do_dimnames(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "dimnames", call)?;
    if let Some(r) = i.try_dispatch("dimnames", call, &args, env)? {
        return Ok(r);
    }
    Ok(get_attrib(&args[0].value, s::DIMNAMES))
}

fn do_dimnamesgets(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "dimnames<-", call)?;
    if let Some(r) = i.try_dispatch("dimnames<-", call, &args, env)? {
        return Ok(r);
    }
    let x = take(&mut args, 0);
    let v = take(&mut args, 1);
    set_attrib(x, s::DIMNAMES, v)
}

fn do_class(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "class", call)?;
    Ok(class_of(&args[0].value))
}

fn do_oldclass(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "oldClass", call)?;
    Ok(args[0].value.get_attr(s::CLASS).unwrap_or(Value::Null))
}

fn do_oldclassgets(_: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "oldClass<-", call)?;
    let x = take(&mut args, 0);
    let mut v = take(&mut args, 1);
    if v.is_empty() {
        v = Value::Null;
    }
    set_attrib(x, s::CLASS, v)
}

/// `class<-` (R_set_class).
fn do_set_class(i: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "class<-", call)?;
    let obj = take(&mut args, 0);
    let mut value = take(&mut args, 1);
    if value.is_empty() {
        return set_attrib(obj, s::CLASS, Value::Null);
    }
    if !matches!(value, Value::Str(_)) {
        value = i.coerce(&value, VType::Str)?;
        if value.is_empty() {
            return Err(Flow::error("invalid replacement object to be a class string"));
        }
    }
    let classes: Vec<RStr> = value.as_str_vec().unwrap().data.clone();
    let first = classes[0].as_deref().unwrap_or("NA").to_string();
    if first == "matrix" {
        let nd = obj.get_attr(s::DIM).map(|d| d.len()).unwrap_or(0);
        if nd != 2 {
            return Err(Flow::error(format!(
                "invalid to set the class to matrix unless the dimension attribute is of length 2 (was {})",
                nd
            )));
        }
        if classes.len() == 1 || (classes.len() == 2 && classes[1].as_deref() == Some("array")) {
            return set_attrib(obj, s::CLASS, Value::Null);
        }
    }
    if classes.len() > 1 {
        return set_attrib(obj, s::CLASS, value);
    }
    let basic = match first.as_str() {
        "logical" => Some(VType::Lgl),
        "integer" => Some(VType::Int),
        "double" => Some(VType::Real),
        "complex" => Some(VType::Cplx),
        "character" => Some(VType::Str),
        "raw" => Some(VType::Raw),
        "list" => Some(VType::List),
        "expression" => Some(VType::Expr),
        _ => None,
    };
    if let Some(t) = basic {
        let obj = set_attrib(obj, s::CLASS, Value::Null)?;
        return i.coerce(&obj, t);
    }
    match first.as_str() {
        "numeric" => {
            let obj = set_attrib(obj, s::CLASS, Value::Null)?;
            match obj {
                Value::Int(_) | Value::Real(_) => Ok(obj),
                _ => i.coerce(&obj, VType::Real),
            }
        }
        "array" => {
            if obj.get_attr(s::DIM).is_none_or(|d| d.is_empty()) {
                return Err(Flow::error("cannot set class to \"array\" unless the dimension attribute has length > 0"));
            }
            set_attrib(obj, s::CLASS, Value::Null)
        }
        "function" | "name" | "environment" | "NULL" | "call" | "language" | "symbol" => {
            let ok = match first.as_str() {
                "function" => obj.is_function(),
                "name" | "symbol" => matches!(obj, Value::Sym(_)),
                "environment" => matches!(obj, Value::Env(_)),
                "NULL" => obj.is_null(),
                _ => matches!(obj, Value::Lang(_)),
            };
            if !ok {
                return Err(Flow::error(format!(
                    "\"{}\" can only be set as the class if the object has this type; found \"{}\"",
                    first,
                    obj.type_name()
                )));
            }
            set_attrib(obj, s::CLASS, Value::Null)
        }
        _ => set_attrib(obj, s::CLASS, value),
    }
}

fn do_levelsgets(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "levels<-", call)?;
    if let Some(r) = i.try_dispatch("levels<-", call, &args, env)? {
        return Ok(r);
    }
    let x = take(&mut args, 0);
    let v = take(&mut args, 1);
    if let Value::Str(lv) = &v {
        let mut seen = std::collections::HashSet::new();
        for (k, l) in lv.data.iter().enumerate() {
            if !seen.insert(l.clone()) {
                return Err(Flow::error_call(call.clone(), format!("factor level [{}] is duplicated", k + 1)));
            }
        }
    }
    set_attrib(x, s::LEVELS, v)
}

fn do_unclass(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "unclass", call)?;
    let _ = (i, env);
    let x = take(&mut args, 0);
    match &x {
        Value::Env(_) => Err(Flow::error_call(call.clone(), "cannot unclass an environment")),
        Value::ExtPtr(_) => Err(Flow::error_call(call.clone(), "cannot unclass an external pointer")),
        _ => {
            if x.get_attr(s::CLASS).is_some() {
                Ok(remove_attrib(x, s::CLASS))
            } else {
                Ok(x)
            }
        }
    }
}

fn do_inherits(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let what = match arg(&args, 1) {
        Value::Str(w) => w.data.clone(),
        _ => return Err(Flow::error_call(call.clone(), "'what' must be a character vector or an object with a nameOfClass() method")),
    };
    let which = crate::coerce::as_logical(&arg(&args, 2));
    if which == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "'which' must be a length 1 logical vector"));
    }
    let klass = crate::dispatch::dispatch_class(&x);
    if which != 0 {
        let out: Vec<i32> = what
            .iter()
            .map(|w| klass.iter().position(|k| Some(k) == w.as_ref()).map(|p| p as i32 + 1).unwrap_or(0))
            .collect();
        Ok(Value::ints(out))
    } else {
        Ok(Value::lgl(what.iter().any(|w| klass.iter().any(|k| Some(k) == w.as_ref()))))
    }
}

fn do_comment(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(arg(&args, 0).get_attr(s::COMMENT).unwrap_or(Value::Null))
}

fn do_commentgets(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = take(&mut args, 0);
    let v = take(&mut args, 1);
    set_attrib(x, s::COMMENT, v)
}

fn do_short_row_names(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let t = crate::coerce::as_integer(&arg(&args, 1));
    if !(0..=2).contains(&t) {
        return Err(Flow::error("invalid 'type' argument"));
    }
    let s0 = x.get_attr(s::ROW_NAMES).unwrap_or(Value::Null);
    if t == 0 {
        return Ok(s0);
    }
    let n = match &s0 {
        Value::Int(iv) if iv.data.len() == 2 && iv.data[0] == NA_INTEGER => iv.data[1],
        Value::Null => 0,
        v => v.len() as i32,
    };
    Ok(Value::int(if t == 1 { n } else { n.abs() }))
}

fn do_copy_df_attr(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let from = take(&mut args, 0);
    let mut out = take(&mut args, 1);
    out.set_attrs(from.attrs().cloned().map(Box::new));
    Ok(out)
}
