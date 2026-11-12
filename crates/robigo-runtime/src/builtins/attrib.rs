//! Attribute builtins (GNU R's attrib.c, plus `class<-` from coerce.c and
//! `unclass`/`inherits` from objects.c).

use super::{arg, internal, match_args, prim};
use crate::attrib::*;
use crate::builtins::subassign::{exposing, replacement};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("attr", do_attr, -1),
    prim("attr<-", do_attrgets, 3).arity_checked().in_place(InPlace::Always),
    prim("attributes", do_attributes, 1).arg1("x"),
    prim("attributes<-", do_attributesgets, 2).arity_checked().in_place(InPlace::Always),
    prim("names", do_names, 1).arg1("x"),
    prim("names<-", do_namesgets, 2).arity_checked().in_place(InPlace::Unclassed),
    prim("dim", do_dim, 1).arg1("x"),
    prim("dim<-", do_dimgets, 2).arity_checked().in_place(InPlace::Unclassed),
    prim("dimnames", do_dimnames, 1).arg1("x"),
    prim("dimnames<-", do_dimnamesgets, 2).arity_checked().in_place(InPlace::Unclassed),
    prim("class", do_class, 1).arg1("x"),
    prim("class<-", do_set_class, 2).arg1("x").in_place(InPlace::Always),
    prim("oldClass", do_oldclass, 1).arg1("x"),
    prim("oldClass<-", do_oldclassgets, 2).arity_checked().in_place(InPlace::Always),
    prim("levels<-", do_levelsgets, 2).arity_checked().in_place(InPlace::Unclassed),
    prim("unclass", do_unclass, 1).arg1("x"),
    internal("inherits", do_inherits, 3).arity_checked(),
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
    let mut m = match_args(args, &["x", "which", "exact"], call)?;
    if !(2..=3).contains(&nargs) {
        return Err(Flow::error_call(call.clone(), "either 2 or 3 arguments are required"));
    }
    let x = m.vals[0].take().unwrap_or(Value::Null);
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

    if exact {
        if let Some(sym) = Sym::lookup(&want) {
            if matches!(sym, s::ROW_NAMES | s::NAMES) {
                return Ok(get_attrib(&x, sym));
            }
            if let Some(val) = x.get_attr(sym) {
                return Ok(val);
            }
        }
    }

    #[derive(PartialEq)]
    enum M {
        None,
        Partial,
        Partial2,
        Full,
    }
    let (mut tag, mut matched) = {
        let env_guard;
        let extptr_guard;
        let attrs: Option<&Attrs> = if let Some(a) = x.attrs() {
            Some(a)
        } else {
            match &x {
                Value::Env(e) => {
                    env_guard = e.0.attrs.borrow();
                    env_guard.as_deref()
                }
                Value::ExtPtr(p) => {
                    extptr_guard = p.attrs.borrow();
                    extptr_guard.as_deref()
                }
                _ => None,
            }
        };
        let mut matched = M::None;
        let mut tag: Option<Sym> = None;
        if let Some(attrs) = attrs {
            for (k, _) in &attrs.0 {
                let hit = k.with_name(|kn| {
                    if kn.starts_with(&*want) {
                        if kn.len() == want.len() { M::Full } else { M::Partial }
                    } else {
                        M::None
                    }
                });
                match hit {
                    M::Full => {
                        tag = Some(*k);
                        matched = M::Full;
                        break;
                    }
                    M::Partial => {
                        if matched == M::Partial || matched == M::Partial2 {
                            matched = M::Partial2;
                        } else {
                            tag = Some(*k);
                            matched = M::Partial;
                        }
                    }
                    M::None | M::Partial2 => {}
                }
            }
        }
        (tag, matched)
    };
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

fn do_attrgets(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    replacement(args, |args| {
        // R: attrib.c:do_attrgets sets the attribute on CAR(args) and takes
        // only `which` and `value` from the matched list.
        let m = match_args(args.clone(), &["x", "which", "value"], call)?;
        let name = match m.get(1) {
            Some(Value::Str(n)) if n.data.len() == 1 && n.data[0].is_some() => n.data[0].clone().unwrap(),
            _ => return Err(Flow::error("'name' must be non-null character string")),
        };
        let val = m.get(2).cloned().unwrap_or(Value::Null);
        drop(m);
        // R: attrib.c:do_attrgets — will become an error in R 4.6.0.
        if !val.is_null() && matches!(args[0].value, Value::Builtin(_)) {
            exposing(i, &args[0].value, |i| {
                i.warning("Setting attributes on primitive functions is deprecated and will be disabled")
            })?;
        }
        set_attrib_in(&mut args[0].value, Sym::new(&name), val)?;
        Ok(None)
    })
}

fn do_attributes(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(attributes_list(&args[0].value))
}

fn do_attributesgets(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    replacement(args, |args| {
        let attrs = take(args, 1);
        // R: attrib.c:do_attributesgets — check before duplication; will
        // become an error in R 4.6.0.
        let nattrs = match &attrs {
            Value::List(l) => l.data.len(),
            _ => 0,
        };
        if nattrs > 0 && matches!(args[0].value, Value::Builtin(_)) {
            exposing(i, &args[0].value, |i| {
                i.warning("Setting attributes on primitive functions is deprecated and will be disabled")
            })?;
        }
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
                    // R: attrib.c:do_attributesgets checks `CHAR(..)[0] ==
                    // '\0'`; an NA name passes (its CHAR is "NA").
                    if n.as_deref() == Some("") {
                        return Err(Flow::error(format!("all attributes must have names [{} does not]", i + 1)));
                    }
                }
                (l.data.clone(), names)
            }
            _ => return Err(Flow::error("attributes must be a list or NULL")),
        };
        // R: attrib.c:do_attributesgets installs on a shallow duplicate of
        // a referenced object when it sets any attributes, "as an error
        // later on would leave 'obj' changed": the original stays in
        // args[0], which a failure below hands back. Removing them all
        // cannot fail and modifies in place.
        if vals.is_empty() {
            let object = &mut args[0].value;
            // R: attrib.c:do_attributesgets -- only a pairlist (isList)
            // loses its tags; a call keeps them (they are not among its
            // attributes).
            if matches!(object, Value::Pairlist(_)) {
                remove_attrib_in(object, s::NAMES);
            }
            object.clear_attrs();
            return Ok(None);
        }
        let mut object = match &args[0].value {
            Value::Null => Value::list(vec![]),
            x => x.clone(),
        };
        if matches!(object, Value::Pairlist(_)) {
            remove_attrib_in(&mut object, s::NAMES);
        }
        // R keeps the S4 bit unless the new attribute list is empty.
        let s4 = object.is_s4();
        object.clear_attrs();
        let dim_i = names.iter().position(|n| n.as_deref() == Some("dim"));
        if let Some(i) = dim_i {
            set_attrib_in(&mut object, s::DIM, vals[i].clone())?;
        }
        for (i, (n, v)) in names.iter().zip(vals.iter()).enumerate() {
            if Some(i) == dim_i {
                continue;
            }
            let n = match n.as_deref() {
                // R: installTrChar(NA_STRING) installs the symbol "NA".
                None => "NA",
                // R: install("") errors.
                Some("") => return Err(Flow::error("attempt to use zero-length variable name")),
                Some(n) => n,
            };
            set_attrib_in(&mut object, Sym::new(n), v.clone())?;
        }
        if s4 {
            object.set_s4(true);
        }
        Ok(Some(object))
    })
}

fn do_names(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
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

fn do_namesgets(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    replacement(args, |args| {
        if let Some(r) = i.try_dispatch("names<-", call, args, env)? {
            return Ok(Some(r));
        }
        let mut names = take(args, 1);
        if names.is_null() && get_attrib(&args[0].value, s::NAMES).is_null() {
            return Ok(None);
        }
        if !names.is_null() && !(matches!(names, Value::Str(_)) && !names.has_attrs()) {
            // as.character(names), with dispatch
            let f = i.find_fun(Sym::new("as.character"), &i.base.clone())?;
            names = exposing(i, &args[0].value, |i| i.call_value(&f, vec![Arg::pos(names)], env))?;
        }
        let x = &mut args[0].value;
        if names.is_null() && is_one_dim_array(x) {
            set_attrib_in(x, s::DIMNAMES, Value::Null)?;
        } else {
            set_attrib_in(x, s::NAMES, names)?;
        }
        Ok(None)
    })
}

fn do_dim(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = i.try_dispatch("dim", call, &args, env)? {
        return Ok(r);
    }
    Ok(get_attrib(&args[0].value, s::DIM))
}

fn do_dimgets(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    replacement(args, |args| {
        if let Some(r) = i.try_dispatch("dim<-", call, args, env)? {
            return Ok(Some(r));
        }
        let d = take(args, 1);
        let x = &args[0].value;
        if d.is_null() && x.get_attr(s::DIM).is_none() && x.get_attr(s::NAMES).is_none() {
            return Ok(None);
        }
        // R: attrib.c:dimgets — coerceVector(val, INTSXP) issues its
        // coercion warnings ("imaginary parts discarded in coercion", "NAs
        // introduced by coercion to integer range") against the dim<- call;
        // dim_gets below re-checks atomicness before this no-op for Int.
        let d = if matches!(d, Value::Int(_) | Value::Lgl(_)) || !d.is_atomic() {
            d
        } else {
            exposing(i, x, |i| i.coerce(&d, VType::Int))?
        };
        let x = &mut args[0].value;
        set_attrib_in(x, s::DIM, d)?;
        remove_attrib_in(x, s::NAMES);
        Ok(None)
    })
}

fn do_dimnames(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = i.try_dispatch("dimnames", call, &args, env)? {
        return Ok(r);
    }
    Ok(get_attrib(&args[0].value, s::DIMNAMES))
}

fn do_dimnamesgets(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    replacement(args, |args| {
        if let Some(r) = i.try_dispatch("dimnames<-", call, args, env)? {
            return Ok(Some(r));
        }
        let v = take(args, 1);
        set_attrib_in(&mut args[0].value, s::DIMNAMES, v)?;
        Ok(None)
    })
}

fn do_class(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(class_of(&args[0].value))
}

// R: attrib.c:do_class — GNU's names.c maps oldClass to do_class itself;
// its `.arg1("x")` DEFS entry gives it the same checkArity + check1arg the
// class getter gets (run by the shared check at builtin entry).
fn do_oldclass(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = &args[0].value;
    if x.is_s4() {
        if let Some(s3class) = x.get_attr(crate::s4::syms::s3class()) {
            return Ok(s3class);
        }
    }
    Ok(x.get_attr(s::CLASS).unwrap_or(Value::Null))
}

// R: attrib.c:do_classgets
fn do_oldclassgets(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    replacement(args, |args| {
        let mut v = take(args, 1);
        let x = &mut args[0].value;
        x.set_s4(false);
        if v.is_empty() {
            v = Value::Null;
        }
        set_attrib_in(x, s::CLASS, v)?;
        Ok(None)
    })
}

/// `class<-` (R_set_class). R strips the class (and the S4 bit) in place
/// before it coerces, so a failure from there on hands back the object
/// without them, as R's binding is left.
fn do_set_class(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    replacement(args, |args| {
        // R: coerce.c:R_do_set_class
        let mut value = take(args, 1);
        if value.is_empty() {
            strip_class(i, &mut args[0].value, &value)?;
            return Ok(None);
        }
        if !matches!(value, Value::Str(_)) {
            value = exposing(i, &args[0].value, |i| i.coerce(&value, VType::Str))?;
            if value.is_empty() {
                return Err(Flow::error("invalid replacement object to be a class string"));
            }
        }
        let classes: Vec<RStr> = value.as_str_vec().unwrap().data.clone();
        let first = classes[0].as_deref().unwrap_or("NA").to_string();
        if first == "matrix" {
            let nd = args[0].value.get_attr(s::DIM).map(|d| d.len()).unwrap_or(0);
            if nd != 2 {
                return Err(Flow::error(format!(
                    "invalid to set the class to matrix unless the dimension attribute is of length 2 (was {})",
                    nd
                )));
            }
            if classes.len() == 1 || (classes.len() == 2 && classes[1].as_deref() == Some("array")) {
                strip_class(i, &mut args[0].value, &value)?;
                return Ok(None);
            }
        }
        if classes.len() > 1 {
            set_attrib_in(&mut args[0].value, s::CLASS, value.clone())?;
            unset_s4(i, &mut args[0].value, &value)?;
            return Ok(None);
        }
        // R: coerce.c:classTable -- the basic classes; assigning one
        // deletes the class attribute, then coerces (or, where the type
        // cannot change, checks it).
        let basic: Option<(&str, Option<VType>)> = match first.as_str() {
            "logical" => Some(("logical", Some(VType::Lgl))),
            "integer" => Some(("integer", Some(VType::Int))),
            "double" => Some(("double", Some(VType::Real))),
            "raw" => Some(("raw", Some(VType::Raw))),
            "complex" => Some(("complex", Some(VType::Cplx))),
            "character" => Some(("character", Some(VType::Str))),
            "expression" => Some(("expression", Some(VType::Expr))),
            "list" => Some(("list", Some(VType::List))),
            "char" => Some(("char", None)),
            "environment" => Some(("environment", None)),
            "externalptr" => Some(("externalptr", None)),
            "weakref" => Some(("weakref", None)),
            "name" => Some(("symbol", None)),
            _ => None,
        };
        if let Some((type_name, coerce_to)) = basic {
            strip_class(i, &mut args[0].value, &value)?;
            let obj = &args[0].value;
            return match coerce_to {
                Some(t) => exposing(i, obj, |i| ascommon(i, call, obj, t)).map(Some),
                // A CHARSXP is not an R value: coerceVector refuses atomic
                // vectors; other types reach R's UNIMPLEMENTED_TYPE paths.
                None if first == "char" => Err(if obj.is_atomic() {
                    Flow::error(format!("cannot coerce type '{}' to vector of type 'char'", obj.type_name()))
                } else {
                    Flow::error(format!("robigo: coercing type '{}' to 'char' is not implemented yet", obj.type_name()))
                }),
                None if obj.type_name() != type_name => Err(Flow::error(format!(
                    "\"{}\" can only be set as the class if the object has this type; found \"{}\"",
                    first,
                    obj.type_name()
                ))),
                None => Ok(None),
            };
        }
        match first.as_str() {
            "numeric" => {
                strip_class(i, &mut args[0].value, &value)?;
                match &args[0].value {
                    Value::Int(_) | Value::Real(_) => Ok(None),
                    obj => exposing(i, obj, |i| i.coerce(obj, VType::Real)).map(Some),
                }
            }
            "array" => {
                if args[0].value.get_attr(s::DIM).is_none_or(|d| d.is_empty()) {
                    return Err(Flow::error(
                        "cannot set class to \"array\" unless the dimension attribute has length > 0",
                    ));
                }
                let obj = &mut args[0].value;
                set_attrib_in(obj, s::CLASS, Value::Null)?;
                obj.set_s4(false);
                Ok(None)
            }
            // Set the class without coercing; an as() method does that.
            _ => {
                set_attrib_in(&mut args[0].value, s::CLASS, value)?;
                Ok(None)
            }
        }
    })
}

/// Remove `obj`'s class attribute and then its S4 bit, as R_set_class
/// does before it coerces to a basic class.
fn strip_class(i: &mut Interp, obj: &mut Value, new_class: &Value) -> RResult<()> {
    set_attrib_in(obj, s::CLASS, Value::Null)?;
    unset_s4(i, obj, new_class)
}

/// `ascommon(call, u, type)` for the vector types `class<-` coerces to.
// R: coerce.c:ascommon
fn ascommon(i: &mut Interp, call: &Value, u: &Value, t: VType) -> RResult<Value> {
    let vectorish = u.is_vector()
        || matches!(u, Value::Null | Value::Pairlist(_) | Value::Lang(_))
        || (matches!(u, Value::Sym(_)) && t == VType::Expr);
    if vectorish {
        return if u.vtype() == Some(t) { Ok(u.clone()) } else { i.coerce(u, t) };
    }
    match (u, t) {
        (Value::Sym(name), VType::Str) => Ok(Value::strs(vec![Some(name.name())])),
        (Value::Sym(_), VType::List) => Ok(Value::list(vec![u.clone()])),
        _ => Err(Flow::error_call(
            call.clone(),
            format!("cannot coerce type '{}' to vector of type '{}'", u.type_name(), t.name()),
        )),
    }
}

/// `do_unsetS4(obj, newClass)` when `obj` is an S4 object.
// R: coerce.c:do_unsetS4 unsets the bit first, "as warning()s may be
// treated as errors": such an error hands back the object without it.
fn unset_s4(i: &mut Interp, obj: &mut Value, new_class: &Value) -> RResult<()> {
    if !obj.is_s4() {
        return Ok(());
    }
    obj.set_s4(false);
    let cls = crate::coerce::to_str(new_class);
    let c = |k: usize| cls.get(k).cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "NA".into());
    let msg = if new_class.is_null() {
        "Setting class(x) to NULL;   result will no longer be an S4 object".to_string()
    } else if cls.len() > 1 {
        format!(
            "Setting class(x) to multiple strings (\"{}\", \"{}\", ...); result will no longer be an S4 object",
            c(0),
            c(1)
        )
    } else {
        format!("Setting class(x) to \"{}\" sets attribute to NULL; result will no longer be an S4 object", c(0))
    };
    exposing(i, obj, |i| i.warning(msg))
}

fn do_levelsgets(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    replacement(args, |args| {
        if let Some(r) = i.try_dispatch("levels<-", call, args, env)? {
            return Ok(Some(r));
        }
        let v = take(args, 1);
        // R: attrib.c:do_levelsgets checks any type (any_duplicated errors
        // on non-vectors), not just character levels.
        if !v.is_null() {
            let k = crate::builtins::unique::any_duplicated(&v, false)?;
            if k != 0 {
                return Err(Flow::error_call(call.clone(), format!("factor level [{}] is duplicated", k)));
            }
        }
        set_attrib_in(&mut args[0].value, s::LEVELS, v)?;
        Ok(None)
    })
}

fn do_unclass(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
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

// R: objects.c:do_inherits
fn do_inherits(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let (x, what, which) = (&args[0].value, &args[1].value, &args[2].value);
    if what.is_object() && !matches!(what, Value::Str(_)) {
        let name = name_of_class(i, what, env)?;
        if !name.is_null() {
            return inherits3(i, x, &name, which);
        }
    }
    inherits3(i, x, what, which)
}

/// `base::nameOfClass(X)` evaluated with `X` bound to `what` in a new
/// child of the calling environment.
// R: objects.c:nameOfClass
#[cold]
#[inline(never)]
fn name_of_class(i: &mut Interp, what: &Value, env: &Env) -> RResult<Value> {
    let fun = Value::lang(Value::sym("::"), vec![Arg::pos(Value::sym("base")), Arg::pos(Value::sym("nameOfClass"))]);
    let xsym = Sym::new("X");
    let call = Value::lang(fun, vec![Arg::pos(Value::Sym(xsym))]);
    let callenv = Env::new(Some(env.clone()));
    callenv.set_local(xsym, what.clone());
    i.eval(&call, &callenv)
}

// R: objects.c:inherits3
fn inherits3(i: &mut Interp, x: &Value, what: &Value, which: &Value) -> RResult<Value> {
    let klass: Vec<crate::value::RString> = if x.is_s4() {
        i.data_class2(x)?
    } else {
        crate::coerce::to_str(&crate::s4::data_class(x, false))
            .into_iter()
            .map(|c| c.unwrap_or_else(|| crate::value::RString::from("NA")))
            .collect()
    };
    let what = match what {
        Value::Str(w) => &w.data,
        _ => {
            return Err(Flow::error("'what' must be a character vector or an object with a nameOfClass() method"));
        }
    };
    let which = match which {
        Value::Lgl(w) if w.data.len() == 1 => match w.data[0] {
            NA_LOGICAL => return Err(Flow::error("NA in coercion to boolean")),
            val => val != 0,
        },
        _ => return Err(Flow::error("'which' must be a length 1 logical vector")),
    };
    if which {
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

fn do_commentgets(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    replacement(args, |args| {
        let v = take(args, 1);
        set_attrib_in(&mut args[0].value, s::COMMENT, v)?;
        Ok(None)
    })
}

fn do_short_row_names(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let t = crate::coerce::as_integer(&arg(&args, 1));
    if !(0..=2).contains(&t) {
        return Err(Flow::error("invalid 'type' argument"));
    }
    let s0 = x.get_attr(s::ROW_NAMES).unwrap_or(Value::Null);
    // type 0 hands back the attribute unread; the others read its length
    // (LENGTH: a deferred package ALTREP value is read, crate::altrep)
    if t == 0 {
        return Ok(s0);
    }
    let s0 = crate::altrep::forced(i, s0)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_exact_match_full_returns_attribute() {
        let mut i = Interp::new();
        let env = Env::new(None);
        let call = Value::Null;
        let x = crate::attrib::set_attrib(Value::real(1.0), Sym::new("foobar"), Value::real(2.0)).ok().unwrap();
        let args = vec![Arg::pos(x), Arg::pos(Value::str("foobar")), Arg::pos(Value::lgl(true))];
        let res = do_attr(&mut i, &call, args, &env).ok().unwrap();
        assert!(matches!(res, Value::Real(r) if r.data.as_slice() == [2.0]));
    }

    #[test]
    fn attr_exact_match_expands_compact_row_names() {
        let mut i = Interp::new();
        let env = Env::new(None);
        let call = Value::Null;
        let mut x = Value::reals(vec![1.0, 2.0, 3.0]);
        x.set_attr_raw(Sym::new("row.names"), Value::ints(vec![NA_INTEGER, -3]));
        let args = vec![Arg::pos(x), Arg::pos(Value::str("row.names")), Arg::pos(Value::lgl(true))];
        let res = do_attr(&mut i, &call, args, &env).ok().unwrap();
        assert!(matches!(&res, Value::Int(v) if v.data.as_slice() == [1, 2, 3]));
    }

    #[test]
    fn attr_exact_match_rejects_partial() {
        let mut i = Interp::new();
        let env = Env::new(None);
        let x = Value::real(1.0);
        let x = crate::attrib::set_attrib(x, Sym::new("foobar"), Value::real(2.0)).ok().unwrap();

        // exact = TRUE with partial prefix "foo" returns NULL
        let call = Value::Null;
        let args = vec![Arg::pos(x.clone()), Arg::pos(Value::str("foo")), Arg::pos(Value::lgl(true))];
        let res = do_attr(&mut i, &call, args, &env).ok().unwrap();
        assert!(res.is_null());

        // exact = FALSE with partial prefix "foo" returns the attribute
        let args_partial = vec![Arg::pos(x.clone()), Arg::pos(Value::str("foo")), Arg::pos(Value::lgl(false))];
        let res_partial = do_attr(&mut i, &call, args_partial, &env).ok().unwrap();
        assert!(matches!(res_partial, Value::Real(r) if r.data.as_slice() == [2.0]));
    }
}
