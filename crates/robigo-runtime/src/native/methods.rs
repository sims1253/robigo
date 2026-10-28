//! Native routines of the methods package: method dispatch, slots and
//! the package's metadata helpers (port of GNU R's
//! src/library/methods/src/{methods_list_dispatch,slot,utils,
//! class_support,do_substitute_direct,tests}.c). The evaluator's side of
//! S4 (objects.c, attrib.c, eval.c) is in `crate::s4`.

use std::rc::Rc;

use super::NativeFn;
use crate::builtins::arg;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::s4::{StdGeneric, binding_missing, data_class, deferred_default, find_var, find_var_in_frame, syms};
use crate::sym::{Sym, s};
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[
    ("R_initMethodDispatch", |i, _, a, _| init_method_dispatch(i, arg(&a, 0))),
    ("R_set_method_dispatch", |i, _, a, _| set_method_dispatch(i, &arg(&a, 0))),
    ("R_M_setPrimitiveMethods", |i, _, a, _| {
        i.r_set_prim_method(arg(&a, 0), &arg(&a, 1), &arg(&a, 2), arg(&a, 3), arg(&a, 4))
    }),
    ("R_clear_method_selection", |_, _, _, _| Ok(Value::Null)),
    ("R_dummy_extern_place", |_, _, _, _| {
        Err(Flow::error("calling the C routine used as an initializer for 'externalptr' objects"))
    }),
    ("R_externalptr_prototype_object", |_, _, _, _| {
        Ok(Value::ExtPtr(Rc::new(ExtPtr { tag: Value::Null, attrs: Default::default(), payload: None })))
    }),
    ("R_el_named", |_, _, a, _| Ok(element_named(&arg(&a, 0), &as_char(&arg(&a, 1))))),
    ("R_set_el_named", |i, _, a, _| {
        let mut obj = arg(&a, 0);
        let what = Sym::new(&as_char(&arg(&a, 1)));
        crate::builtins::subassign::subassign3_core(i, &mut obj, what, arg(&a, 2))?;
        Ok(obj)
    }),
    ("R_getClassFromCache", |i, _, a, _| get_class_from_cache(i, &arg(&a, 0), &arg(&a, 1))),
    ("R_getGeneric", |i, _, a, _| get_generic_r(i, &arg(&a, 0), &arg(&a, 1), &arg(&a, 2), &arg(&a, 3))),
    ("R_get_slot", |i, _, a, _| {
        let name = i.slot_name(&arg(&a, 1))?;
        i.do_slot(&arg(&a, 0), name)
    }),
    ("R_set_slot", |i, _, mut a, env| {
        let value = take(&mut a, 2);
        let obj = take(&mut a, 0);
        let orig = obj.clone();
        let new = i.do_slot_assign(obj, &arg(&a, 1), value)?;
        set_slot_in_place(i, env, &orig, &new);
        Ok(new)
    }),
    ("R_hasSlot", |i, _, a, _| {
        let name = i.slot_name(&arg(&a, 1))?;
        Ok(Value::lgl(i.has_slot(&arg(&a, 0), name)))
    }),
    ("R_identC", |_, _, a, _| Ok(Value::lgl(ident_c(&arg(&a, 0), &arg(&a, 1))))),
    ("R_methodsPackageMetaName", |_, _, a, _| meta_name(&arg(&a, 0), &arg(&a, 1), &arg(&a, 2))),
    ("R_methods_test_MAKE_CLASS", |i, _, a, _| make_class(i, &as_char(&arg(&a, 0)))),
    ("R_methods_test_NEW", |i, _, a, _| {
        let cl = make_class(i, &as_char(&arg(&a, 0)))?;
        i.do_new_object(&cl)
    }),
    ("R_missingArg", |i, _, a, _| missing_arg(i, &arg(&a, 0), &arg(&a, 1))),
    ("R_nextMethodCall", |i, _, a, _| next_method_call(i, &arg(&a, 0), &arg(&a, 1))),
    ("R_quick_method_check", |i, _, a, _| {
        let args = list_args(&arg(&a, 0));
        quick_method_check(i, &args, &arg(&a, 1))
    }),
    ("R_selectMethod", |i, _, a, _| {
        let ev = env_arg(&arg(&a, 1));
        let eval_args = crate::coerce::as_logical(&arg(&a, 3));
        do_dispatch(i, &arg(&a, 0), ev, &arg(&a, 2), true, eval_args != 0)
    }),
    ("R_standardGeneric", |i, _, a, _| {
        let Value::Env(ev) = arg(&a, 1) else {
            return Err(Flow::error("robigo: R_standardGeneric needs an environment"));
        };
        standard_generic_old(i, &arg(&a, 0), &ev, &arg(&a, 2))
    }),
    ("do_substitute_direct", |i, _, a, _| substitute_direct(i, &arg(&a, 0), &arg(&a, 1))),
    ("Rf_allocS4Object", |_, _, _, _| {
        let mut v = Value::Obj(Rc::new(Obj::new(vec![])));
        v.set_s4(true);
        Ok(v)
    }),
    ("R_get_primname", |_, _, a, _| match arg(&a, 0) {
        Value::Builtin(b) => Ok(Value::str(b.name)),
        _ => Err(Flow::error("'R_get_primname' called on a non-primitive")),
    }),
    ("new_object", |i, _, a, _| i.do_new_object(&arg(&a, 0))),
];

/// GNU R's `R_set_slot` modifies its argument in place, and methods'
/// `initialize()` relies on that (`"slot<-"(.Object, slotName, check =
/// FALSE, slotVal)` without assigning the result). robigo's values cannot
/// change under other references, so the binding the object came from is
/// updated instead: `object` in the frame of `slot<-` (the `.Call`'s
/// environment) is a promise of a variable of the caller; if that variable
/// still holds the very object that was modified, it now holds the result.
fn set_slot_in_place(i: &mut Interp, env: &Env, orig: &Value, new: &Value) {
    let Some(Value::Promise(p)) = env.get_local(Sym::new("object")) else { return };
    let Some(v) = p.value() else { return };
    if !crate::s4::same_object(&v, orig) {
        return;
    }
    let Value::Sym(var) = &p.expr else { return };
    let Some(k) = i.frame_of_env(env) else { return };
    let caller = i.frames[k].sysparent.clone();
    if caller.get_local(*var).is_some_and(|b| crate::s4::same_object(&b, orig)) {
        caller.set_local(*var, new.clone());
    }
}

fn take(args: &mut [Arg], i: usize) -> Value {
    args.get_mut(i).map(|a| std::mem::replace(&mut a.value, Value::Null)).unwrap_or(Value::Null)
}

fn env_arg(v: &Value) -> Option<Env> {
    match v {
        Value::Env(e) => Some(e.clone()),
        _ => None,
    }
}

/// The elements of a list or pairlist argument.
fn list_args(v: &Value) -> Vec<Arg> {
    match v {
        Value::Pairlist(p) => p.items.clone(),
        Value::List(l) => l.data.iter().map(|x| Arg::pos(x.clone())).collect(),
        _ => vec![],
    }
}

/// `CHAR(asChar(x))`.
fn as_char(v: &Value) -> String {
    match v {
        Value::Sym(s) => s.name().to_string(),
        _ => {
            let c = crate::coerce::to_str(v);
            c.first().map(|x| x.as_deref().unwrap_or("NA").to_string()).unwrap_or_else(|| "NA".to_string())
        }
    }
}

/// `class_string(obj)`.
fn class_string(obj: &Value) -> String {
    as_char(&data_class(obj, true))
}

/// `check_single_string(obj, nonEmpty, what)`.
fn check_single_string(obj: &Value, non_empty: bool, what: &str) -> RResult<String> {
    match obj {
        Value::Str(v) => {
            if v.data.len() != 1 {
                return Err(Flow::error(format!(
                    "'{}' must be a single string (got a character vector of length {})",
                    what,
                    v.data.len()
                )));
            }
            let s = v.data[0].as_deref().unwrap_or("NA").to_string();
            if non_empty && s.is_empty() {
                return Err(Flow::error(format!("'{}' must be a non-empty string; got an empty string", what)));
            }
            Ok(s)
        }
        _ => Err(Flow::error(format!(
            "'{}' must be a single string (got an object of class \"{}\")",
            what,
            class_string(obj)
        ))),
    }
}

/// `R_initMethodDispatch(envir)`.
// R: methods_list_dispatch.c:R_initMethodDispatch
fn init_method_dispatch(i: &mut Interp, envir: Value) -> RResult<Value> {
    if let Value::Env(e) = &envir {
        i.s4.methods_ns = Some(e.clone());
    }
    let ns = i.methods_namespace();
    let table = i.s4.quick != StdGeneric::Old;
    i.set_standard_generic(if table { StdGeneric::Table } else { StdGeneric::Old }, Some(ns.clone()));
    i.s4.quick = if table { StdGeneric::Table } else { StdGeneric::Old };
    // The skeleton calls are only needed by the old dispatch; GNU R checks
    // that they exist.
    for name in [".ShortPrimitiveSkeletons", ".EmptyPrimitiveSkeletons"] {
        if find_var(i, &ns, Sym::new(name))?.is_none() {
            return Err(Flow::error(
                "could not find the skeleton calls for 'methods' (package detached?): expect very bad things to happen",
            ));
        }
    }
    Ok(envir)
}

/// `R_set_method_dispatch(onOff)`.
// R: methods_list_dispatch.c:R_set_method_dispatch
fn set_method_dispatch(i: &mut Interp, on_off: &Value) -> RResult<Value> {
    let prev = i.s4.quick != StdGeneric::Old;
    let l = crate::coerce::as_logical(on_off);
    let value = if l == NA_LOGICAL { prev } else { l != 0 };
    if value != prev {
        let ns = i.methods_namespace();
        i.set_standard_generic(if value { StdGeneric::Table } else { StdGeneric::Old }, Some(ns));
        i.s4.quick = if value { StdGeneric::Table } else { StdGeneric::Old };
    } else if i.s4.quick == StdGeneric::Unset {
        i.s4.quick = StdGeneric::Table;
    }
    Ok(Value::lgl(prev))
}

/// `R_element_named(obj, what)`: `obj[[what]]` without partial matching.
fn element_named(obj: &Value, what: &str) -> Value {
    let Some(Value::Str(names)) = obj.names() else { return Value::Null };
    let Some(k) = names.data.iter().position(|n| n.as_deref() == Some(what)) else { return Value::Null };
    match obj {
        Value::List(l) | Value::Expr(l) => l.data.get(k).cloned().unwrap_or(Value::Null),
        Value::Pairlist(p) => p.items.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn ident_c(e1: &Value, e2: &Value) -> bool {
    match (e1, e2) {
        (Value::Str(a), Value::Str(b)) => a.data.len() == 1 && b.data.len() == 1 && a.data[0] == b.data[0],
        _ => false,
    }
}

/// `R_methodsPackageMetaName(prefix, name, pkg)`.
fn meta_name(prefix: &Value, name: &Value, pkg: &Value) -> RResult<Value> {
    let p = check_single_string(prefix, true, "The internal prefix (e.g., \"C\") for a meta-data object")?;
    let n = check_single_string(
        name,
        false,
        "The name of the object (e.g,. a class or generic function) to find in the meta-data",
    )?;
    let k = check_single_string(pkg, false, "The name of the package for a meta-data object")?;
    Ok(Value::str(&if k.is_empty() { format!(".__{}__{}", p, n) } else { format!(".__{}__{}:{}", p, n, k) }))
}

/// `R_do_MAKE_CLASS(what)`: `getClass(what)` in the methods namespace.
fn make_class(i: &mut Interp, what: &str) -> RResult<Value> {
    let ns = i.methods_namespace();
    let call = Value::lang(Value::sym("getClass"), vec![Arg::pos(Value::str(what))]);
    i.eval(&call, &ns)
}

fn package_slot(v: &Value) -> Option<Value> {
    v.get_attr(syms::package())
}

/// `R_getClassFromCache(class, table)`.
// R: methods_list_dispatch.c:R_getClassFromCache
fn get_class_from_cache(i: &mut Interp, class: &Value, table: &Value) -> RResult<Value> {
    match class {
        Value::Str(c) => {
            let Some(first) = c.data.first() else { return Ok(Value::Null) };
            let package = package_slot(class);
            let Value::Env(table) = table else { return Err(Flow::error("use of NULL environment is defunct")) };
            let value = find_var_in_frame(i, table, Sym::new(first.as_deref().unwrap_or("NA")))?;
            let Some(value) = value else { return Ok(Value::Null) };
            if let Some(Value::Str(p)) = package {
                if let Some(Value::Str(d)) = package_slot(&value) {
                    if d.data.len() == 1 && d.data.first() != p.data.first() {
                        return Ok(Value::Null);
                    }
                }
            }
            Ok(value)
        }
        Value::Obj(_) => Ok(class.clone()),
        _ => Err(Flow::error("class should be either a character-string name or a class definition")),
    }
}

fn is_generic(v: &Value) -> bool {
    matches!(v, Value::Closure(_)) && v.get_attr(syms::generic()).is_some()
}

/// `get_generic(symbol, rho, package)`.
// R: methods_list_dispatch.c:get_generic
fn get_generic(i: &mut Interp, symbol: Sym, rho: Option<Env>, pkg: &str) -> RResult<Option<Value>> {
    let mut rho = rho;
    while let Some(e) = rho {
        if let Some(mut vl) = find_var_in_frame(i, &e, symbol)? {
            if let Value::Promise(p) = &vl {
                vl = i.force_promise(p)?;
            }
            let mut ok = false;
            if is_generic(&vl) {
                if !pkg.is_empty() {
                    let gpackage = package_slot(&vl).unwrap_or(Value::Null);
                    let g = check_single_string(&gpackage, false, "The \"package\" slot in generic function object")?;
                    ok = g == pkg;
                } else {
                    ok = true;
                }
            }
            if ok {
                return Ok(Some(vl));
            }
        }
        rho = e.parent();
    }
    let base = i.base.clone();
    let vl = match base.get_local(symbol) {
        Some(Value::Promise(p)) => Some(i.force_promise(&p)?),
        v => v,
    };
    if let Some(vl) = vl {
        if is_generic(&vl) {
            if !pkg.is_empty() {
                let gpackage = package_slot(&vl).unwrap_or(Value::Null);
                let g = check_single_string(&gpackage, false, "The \"package\" slot in generic function object")?;
                if g != pkg {
                    return Ok(None);
                }
            }
            return Ok(Some(vl));
        }
    }
    Ok(None)
}

/// `R_getGeneric(name, mustFind, env, package)`.
fn get_generic_r(i: &mut Interp, name: &Value, must_find: &Value, env: &Value, package: &Value) -> RResult<Value> {
    if !matches!(name, Value::Sym(_)) {
        check_single_string(name, true, "The argument \"f\" to getGeneric")?;
    }
    let pkg = check_single_string(package, false, "The argument \"package\" to getGeneric")?;
    let symbol = Sym::new(&as_char(name));
    let rho = env_arg(env);
    match get_generic(i, symbol, rho.clone(), &pkg)? {
        Some(v) => Ok(v),
        None => {
            if crate::coerce::as_logical(must_find) == 1 {
                let global = rho.is_some_and(|r| r.ptr_eq(&i.global));
                return Err(Flow::error(if global {
                    format!("no generic function definition found for '{}'", as_char(name))
                } else {
                    format!("no generic function definition found for '{}' in the supplied environment", as_char(name))
                }));
            }
            Ok(Value::Null)
        }
    }
}

/// `is_missing_arg(symbol, ev)`.
fn is_missing_arg(symbol: Sym, ev: &Env) -> RResult<bool> {
    match binding_missing(ev, symbol) {
        Some(m) => Ok(m),
        None => Err(Flow::error(format!("could not find symbol '{}' in frame of call", symbol.name()))),
    }
}

/// `R_missingArg(symbol, ev)`.
fn missing_arg(_: &mut Interp, symbol: &Value, ev: &Value) -> RResult<Value> {
    let Value::Sym(sym) = symbol else {
        return Err(Flow::error(format!(
            "invalid symbol in checking for missing argument in method dispatch: expected a name, got an object of class \"{}\"",
            class_string(symbol)
        )));
    };
    let ev = match ev {
        Value::Null => return Err(Flow::error("use of NULL environment is defunct")),
        Value::Env(e) => e,
        _ => {
            return Err(Flow::error(format!(
                "invalid environment in checking for missing argument, '{}', in methods dispatch: got an object of class \"{}\"",
                sym.name(),
                class_string(ev)
            )));
        }
    };
    Ok(Value::lgl(is_missing_arg(*sym, ev)?))
}

/// `R_conditionMessage(cond)`.
fn condition_message(i: &mut Interp, cond: &Value) -> RResult<String> {
    let call = Value::lang(Value::sym("conditionMessage"), vec![Arg::pos(crate::s4::quote_value(cond.clone()))]);
    let base = i.base.clone();
    let out = i.eval(&call, &base)?;
    match &out {
        Value::Str(v) if v.data.len() == 1 => Ok(v.data[0].as_deref().unwrap_or("NA").to_string()),
        Value::Str(_) => Err(Flow::error("condition message must be length 1")),
        _ => Err(Flow::error(format!("unexpected type '{}' for condition message", out.type_name()))),
    }
}

/// The call GNU R reports for an error raised by a C-level calling
/// handler: the handler's own call from `.handleSimpleError`.
fn handler_call() -> Value {
    Value::call_sym("h", vec![Value::call_sym("simpleError", vec![Value::sym("msg"), Value::sym("call")])])
}

/// `R_evalHandleError(e, env, argEvalCleanup, {fname, arg_sym})`.
fn eval_arg_handle_error(i: &mut Interp, e: &Value, ev: &Env, fname: &Value, arg_sym: Sym) -> RResult<Value> {
    let fname = as_char(fname);
    let h: crate::conditions::RustHandler = Rc::new(move |i: &mut Interp, cond: &Value| {
        let msg = condition_message(i, cond)?;
        Err(Flow::error_call(
            handler_call(),
            format!(
                "error in evaluating the argument '{}' in selecting a method for function '{}': {}",
                arg_sym.name(),
                fname,
                msg
            ),
        ))
    });
    i.with_calling_error_handler(h, |i| i.eval(e, ev))
}

/// `R_quick_dispatch(args, genericEnv, fdef)`: look up the method for
/// `args` in the generic's `.AllMTable` without inheritance.
// R: methods_list_dispatch.c:R_quick_dispatch
pub fn quick_dispatch(i: &mut Interp, args: &[Arg], generic_env: &Value) -> RResult<Value> {
    let Value::Env(genv) = generic_env else { return Ok(Value::Null) };
    let mtable = match find_var_in_frame(i, genv, Sym::new(".AllMTable"))? {
        Some(Value::Env(m)) => m,
        _ => return Ok(Value::Null),
    };
    let nsig = match find_var_in_frame(i, genv, Sym::new(".SigLength"))? {
        Some(Value::Real(v)) if !v.data.is_empty() => v.data[0] as i32,
        Some(Value::Int(v)) if !v.data.is_empty() => v.data[0],
        _ => return Ok(Value::Null),
    };
    const NBUF: usize = 200;
    let mut buf = String::new();
    let mut nargs = 0;
    let ns = i.methods_namespace();
    for a in args {
        if nargs >= nsig {
            break;
        }
        let object = match &a.value {
            Value::Promise(p) => i.force_promise(p)?,
            v => {
                let _ = &ns;
                v.clone()
            }
        };
        let class = if object.is_missing_arg() { "missing".to_string() } else { class_string(&object) };
        if buf.len() + class.len() + 2 > NBUF {
            return Ok(Value::Null);
        }
        if !buf.is_empty() {
            buf.push('#');
        }
        buf.push_str(&class);
        nargs += 1;
    }
    while nargs < nsig {
        if buf.len() + "missing".len() + 2 > NBUF {
            return Ok(Value::Null);
        }
        buf.push_str("#missing");
        nargs += 1;
    }
    Ok(find_var_in_frame(i, &mtable, Sym::new(&buf))?.unwrap_or(Value::Null))
}

/// `R_find_method(mlist, class, fname)`.
fn find_method(i: &mut Interp, mlist: &Value, class: &str, fname: &Value) -> RResult<Value> {
    let methods = i.do_slot(mlist, Sym::new("allMethods"))?;
    if methods.is_null() {
        return Err(Flow::error(format!(
            "no \"allMethods\" slot found in object of class \"{}\" used as methods list for function '{}'",
            class_string(mlist),
            as_char(fname)
        )));
    }
    Ok(element_named(&methods, class))
}

/// `R_quick_method_check(args, mlist, fdef)` (the old MethodsList
/// dispatch).
pub fn quick_method_check(i: &mut Interp, args: &[Arg], mlist: &Value) -> RResult<Value> {
    if mlist.is_null() {
        return Ok(Value::Null);
    }
    let mut methods = i.do_slot(mlist, Sym::new("allMethods"))?;
    if methods.is_null() {
        return Ok(Value::Null);
    }
    for a in args {
        if methods.is_null() {
            break;
        }
        let object = match &a.value {
            Value::Promise(p) => i.force_promise(p)?,
            v => v.clone(),
        };
        let class = class_string(&object);
        let value = element_named(&methods, &class);
        if value.is_null() || value.is_function() {
            return Ok(value);
        }
        methods = i.do_slot(&value, Sym::new("allMethods"))?;
    }
    Ok(Value::Null)
}

/// `do_dispatch(fname, ev, mlist, firstTry, evalArgs)` (old dispatch).
// R: methods_list_dispatch.c:do_dispatch
fn do_dispatch(
    i: &mut Interp,
    fname: &Value,
    ev: Option<Env>,
    mlist: &Value,
    first_try: bool,
    eval_args: bool,
) -> RResult<Value> {
    if mlist.is_function() {
        return Ok(mlist.clone());
    }
    let arg_slot = i.do_slot(mlist, Sym::new("argument"))?;
    if arg_slot.is_null() {
        return Err(Flow::error(format!(
            "object of class \"{}\" used as methods list for function '{}' ( no 'argument' slot)",
            class_string(mlist),
            as_char(fname)
        )));
    }
    let arg_sym = match &arg_slot {
        Value::Sym(s) => *s,
        v => Sym::new(&as_char(v)),
    };
    if arg_sym == s::DOTS || arg_sym.ddval().is_some() {
        return Err(Flow::error(format!(
            "(in selecting a method for function '{}') '...' and related variables cannot be used for methods dispatch",
            as_char(fname)
        )));
    }
    let Some(ev) = ev else {
        return Err(Flow::error(format!(
            "(in selecting a method for function '{}') the 'environment' argument for dispatch must be an R environment; got an object of class \"{}\"",
            as_char(fname),
            "NULL"
        )));
    };
    let class = if eval_args {
        if is_missing_arg(arg_sym, &ev)? {
            "missing".to_string()
        } else {
            let a = eval_arg_handle_error(i, &Value::Sym(arg_sym), &ev, fname, arg_sym)?;
            class_string(&a)
        }
    } else {
        let a = eval_arg_handle_error(i, &Value::Sym(arg_sym), &ev, fname, arg_sym)?;
        as_char(&a)
    };
    let method = find_method(i, mlist, &class, fname)?;
    if method.is_null() {
        if !first_try {
            return Err(Flow::error(format!(
                "no matching method for function '{}' (argument '{}', with class \"{}\")",
                as_char(fname),
                arg_sym.name(),
                class
            )));
        }
        return Ok(Value::Null);
    }
    if !method.is_function() {
        return do_dispatch(i, &Value::Null, Some(ev), &method, first_try, eval_args);
    }
    Ok(method)
}

/// `R_standardGeneric(fname, ev, fdef)`: the old MethodsList dispatch.
// R: methods_list_dispatch.c:R_standardGeneric
pub fn standard_generic_old(i: &mut Interp, fname: &Value, ev: &Env, fdef: &Value) -> RResult<Value> {
    if !matches!(fname, Value::Sym(_)) {
        check_single_string(fname, true, "The function name in the call to standardGeneric")?;
    }
    let (f_env, mlist) = match fdef {
        Value::Closure(c) => {
            let m = find_var(i, &c.env, syms::dot_methods())?.unwrap_or(Value::Null);
            (c.env.clone(), m)
        }
        Value::Builtin(b) => (i.base.clone(), i.primitive_methods(b)),
        _ => {
            return Err(Flow::error(format!(
                "invalid generic function object for method selection for function '{}': expected a function or a primitive, got an object of class \"{}\"",
                as_char(fname),
                class_string(fdef)
            )));
        }
    };
    let mut mlist = match mlist {
        Value::Promise(p) => i.force_promise(&p)?,
        m => m,
    };
    let mut f = match &mlist {
        Value::Null | Value::Closure(_) | Value::Builtin(_) => mlist.clone(),
        _ => do_dispatch(i, fname, Some(ev.clone()), &mlist, true, true)?,
    };
    if f.is_null() {
        let mut args = vec![
            Arg::pos(fname.clone()),
            Arg::pos(Value::Env(ev.clone())),
            Arg::pos(crate::s4::quote_value(mlist.clone())),
        ];
        if !f_env.ptr_eq(&i.base) || !matches!(fdef, Value::Builtin(_)) {
            args.push(Arg::pos(Value::Env(f_env.clone())));
        }
        let e = Value::lang(Value::sym("MethodsListSelect"), args);
        let ns = i.methods_namespace();
        let fname2 = fname.clone();
        let h: crate::conditions::RustHandler = Rc::new(move |_: &mut Interp, _: &Value| {
            Err(Flow::error_call(
                handler_call(),
                format!(
                    "S language method selection did not return normally when called from internal dispatch for function '{}'",
                    as_char(&fname2)
                ),
            ))
        });
        let value = i.with_calling_error_handler(h, |i| i.eval(&e, &ns))?;
        if value.is_null() {
            return Err(Flow::error(format!(
                "no direct or inherited method for function '{}' for this call",
                as_char(fname)
            )));
        }
        mlist = value;
        f = do_dispatch(i, fname, Some(ev.clone()), &mlist, false, true)?;
    }
    if f.is_object() {
        f = load_method(i, f, fname, ev)?;
    }
    match &f {
        Value::Closure(c) => {
            if f.inherits("internalDispatchMethod") {
                Ok(deferred_default())
            } else {
                i.exec_method(c, ev)
            }
        }
        Value::Builtin(_) => Ok(deferred_default()),
        _ => Err(Flow::error("invalid object (non-function) used as method")),
    }
}

/// `R_loadMethod(def, fname, ev)`: assign `.target`, `.defined`,
/// `.nextMethod` and `.Method` in the frame; other attributes go through
/// `methods:::loadMethod`.
// R: methods_list_dispatch.c:R_loadMethod
fn load_method(i: &mut Interp, def: Value, fname: &Value, ev: &Env) -> RResult<Value> {
    let (target, defined, next_method, srcref, generic) =
        (Sym::new("target"), Sym::new("defined"), Sym::new("nextMethod"), s::SRCREF, syms::generic());
    let mut found = 1;
    let attrib: Vec<(Sym, Value)> = def.attrs().map(|a| a.0.clone()).unwrap_or_default();
    for (t, v) in &attrib {
        if *t == target {
            i.define(syms::dot_target(), v.clone(), ev)?;
            found += 1;
        } else if *t == defined {
            i.define(syms::dot_defined(), v.clone(), ev)?;
            found += 1;
        } else if *t == next_method {
            i.define(syms::dot_next_method(), v.clone(), ev)?;
            found += 1;
        } else if *t == srcref || *t == generic {
            found += 1;
        }
    }
    i.define(syms::dot_method(), def.clone(), ev)?;
    if found < attrib.len() {
        if as_char(fname) == "loadMethod" {
            return Ok(def);
        }
        let fun = Value::call_sym(":::", vec![Value::sym("methods"), Value::sym("loadMethod")]);
        let e = Value::lang(
            fun,
            vec![Arg::pos(crate::s4::quote_value(def)), Arg::pos(fname.clone()), Arg::pos(Value::Env(ev.clone()))],
        );
        return i.eval(&e, ev);
    }
    Ok(def)
}

/// `R_selectByPackage(table, classes, nargs)`.
fn select_by_package(i: &mut Interp, table: &Env, classes: &[Value]) -> RResult<Option<Value>> {
    let label: Vec<String> = classes
        .iter()
        .map(|c| match package_slot(c) {
            Some(p) => as_char(&p),
            None => "base".to_string(),
        })
        .collect();
    find_var_in_frame(i, table, Sym::new(&label.join("#")))
}

/// `R_dispatchGeneric(fname, ev, fdef)`: table dispatch on the classes of
/// the signature arguments in the generic's frame `ev`.
// R: methods_list_dispatch.c:R_dispatchGeneric
pub fn dispatch_generic(i: &mut Interp, fname: &Value, ev: &Env, fdef: &Value) -> RResult<Value> {
    let (fdef, f_env) = match fdef {
        Value::Closure(c) => (fdef.clone(), c.env.clone()),
        Value::Builtin(b) => {
            let g = i.primitive_generic(b);
            match &g {
                Value::Closure(c) => {
                    let env = c.env.clone();
                    (g, env)
                }
                _ => {
                    return Err(Flow::error(format!(
                        "failed to get the generic for the primitive \"{}\"",
                        as_char(fname)
                    )));
                }
            }
        }
        _ => {
            return Err(Flow::error(format!(
                "expected a generic function or a primitive for dispatch, got an object of class \"{}\"",
                class_string(fdef)
            )));
        }
    };
    let allmtable = Sym::new(".AllMTable");
    let mut mtable = find_var_in_frame(i, &f_env, allmtable)?;
    if mtable.is_none() {
        // do_mtable: initializes the generic.
        let ns = i.methods_namespace();
        let f = i.find_fun(Sym::new(".getMethodsTable"), &ns)?;
        let e = Value::lang(f, vec![Arg::pos(fdef.clone())]);
        i.eval(&e, ev)?;
        mtable = find_var_in_frame(i, &f_env, allmtable)?;
    }
    let sigargs = find_var_in_frame(i, &f_env, Sym::new(".SigArgs"))?;
    let siglength = find_var_in_frame(i, &f_env, Sym::new(".SigLength"))?;
    let (Some(sigargs), Some(siglength), Some(mtable)) = (sigargs, siglength, mtable) else {
        return Err(Flow::error(
            "generic seems not to have been initialized for table dispatch---need to have '.SigArgs' and '.AllMtable' assigned in its environment",
        ));
    };
    let nargs = crate::coerce::as_integer(&siglength);
    let nargs = if nargs == NA_INTEGER { 0 } else { nargs.max(0) as usize };
    let sigargs: Vec<Value> = match &sigargs {
        Value::List(l) => l.data.clone(),
        v => list_args(v).into_iter().map(|a| a.value).collect(),
    };
    if nargs > sigargs.len() {
        return Err(Flow::error("'.SigArgs' is shorter than '.SigLength' says it should be"));
    }
    let mut classes: Vec<Value> = Vec::with_capacity(nargs);
    for arg_sym in sigargs.iter().take(nargs) {
        let sym = match arg_sym {
            Value::Sym(s) => *s,
            v => Sym::new(&as_char(v)),
        };
        let this_class = if is_missing_arg(sym, ev)? {
            let mut m = Value::str("missing");
            m.set_attr_raw(syms::package(), Value::str("methods"));
            m
        } else if sym == s::DOTS {
            let ns = i.methods_namespace();
            let f = i.find_fun(Sym::new(".dotsClass"), &ns)?;
            let call = Value::lang(f, vec![Arg::pos(Value::Sym(s::DOTS))]);
            eval_arg_handle_error(i, &call, ev, fname, sym)?
        } else {
            let a = eval_arg_handle_error(i, &Value::Sym(sym), ev, fname, sym)?;
            data_class(&a, true)
        };
        classes.push(this_class);
    }
    let label: Vec<String> = classes.iter().map(as_char).collect();
    let Value::Env(mtable_env) = &mtable else {
        return Err(Flow::error("robigo: '.AllMTable' is not an environment"));
    };
    let mut method = find_var_in_frame(i, mtable_env, Sym::new(&label.join("#")))?;
    if let Some(Value::Env(t)) = &method {
        let t = t.clone();
        method = select_by_package(i, &t, &classes)?;
    }
    let method = match method {
        Some(m) => m,
        None => {
            let ns = i.methods_namespace();
            let f = i.find_fun(Sym::new(".InheritForDispatch"), &ns)?;
            let e = Value::lang(
                f,
                vec![Arg::pos(Value::list(classes)), Arg::pos(fdef.clone()), Arg::pos(Value::Env(mtable_env.clone()))],
            );
            i.eval(&e, ev)?
        }
    };
    match &method {
        Value::Closure(_) => {
            if method.inherits("internalDispatchMethod") {
                return Ok(deferred_default());
            }
            let f = if method.is_object() { load_method(i, method.clone(), fname, ev)? } else { method.clone() };
            match &f {
                Value::Closure(c) => i.exec_method(c, ev),
                Value::Builtin(_) => Ok(deferred_default()),
                _ => Err(Flow::error("invalid object (non-function) used as method")),
            }
        }
        Value::Builtin(_) => Ok(deferred_default()),
        _ => Err(Flow::error("invalid object (non-function) used as method")),
    }
}

/// `R_nextMethodCall(matched_call, ev)`.
// R: methods_list_dispatch.c:R_nextMethodCall
fn next_method_call(i: &mut Interp, matched_call: &Value, ev: &Value) -> RResult<Value> {
    let Value::Env(ev) = ev else { return Err(Flow::error("robigo: R_nextMethodCall needs an environment")) };
    let Some(mut op) = find_var_in_frame(i, ev, syms::dot_next_method())? else {
        return Err(Flow::error(
            "internal error in 'callNextMethod': '.nextMethod' was not assigned in the frame of the method call",
        ));
    };
    if let Value::Promise(p) = &op {
        op = i.force_promise(p)?;
    }
    let Value::Lang(l) = matched_call else {
        return Err(Flow::error("robigo: R_nextMethodCall needs a call"));
    };
    let mut e = (**l).clone();
    let mut prim: Option<&'static Builtin> = match &op {
        Value::Builtin(b) => Some(b),
        _ => None,
    };
    if prim.is_none() && op.inherits("internalDispatchMethod") {
        let Some(generic) = find_var_in_frame(i, ev, syms::dot_generic())? else {
            return Err(Flow::error(
                "internal error in 'callNextMethod': '.Generic' was not assigned in the frame of the method call",
            ));
        };
        match crate::builtins::lookup_internal(&as_char(&generic)) {
            Some(b) => prim = Some(b),
            None => return Err(Flow::error("invalid object: must be a primitive function")),
        }
    }
    if let Some(op) = prim {
        i.set_prim_method(op, "suppress", Value::Null, Value::Null)?;
    } else {
        e.fun = Value::Sym(syms::dot_next_method());
    }
    for a in e.args.iter_mut() {
        if let Some(t) = a.tag {
            if !a.value.is_missing_arg() {
                a.value = Value::Sym(t);
            }
        }
    }
    let e = Value::Lang(Rc::new(e));
    match prim {
        Some(op) => {
            let h: crate::conditions::RustHandler = Rc::new(|i: &mut Interp, cond: &Value| {
                let msg = condition_message(i, cond)?;
                Err(Flow::error_call(handler_call(), format!("error in evaluating a 'primitive' next method: {}", msg)))
            });
            let ev = ev.clone();
            let r = i.with_calling_error_handler(h, |i| i.eval(&e, &ev));
            i.set_prim_method(op, "set", Value::Null, Value::Null)?;
            r
        }
        None => i.eval(&e, ev),
    }
}

/// `do_substitute_direct(f, env)`.
fn substitute_direct(_: &mut Interp, f: &Value, env: &Value) -> RResult<Value> {
    let env = match env {
        Value::List(l) => {
            let e = Env::new(None);
            let names = env.names();
            for (k, v) in l.data.iter().enumerate() {
                let n = match &names {
                    Some(Value::Str(n)) => n.data.get(k).cloned().flatten(),
                    _ => None,
                };
                if let Some(n) = n.filter(|n| !n.is_empty()) {
                    e.set_local(Sym::new(&n), v.clone());
                }
            }
            e
        }
        Value::Pairlist(p) => {
            let e = Env::new(None);
            for a in &p.items {
                if let Some(t) = a.tag {
                    e.set_local(t, a.value.clone());
                }
            }
            e
        }
        Value::Env(e) => e.clone(),
        _ => return Err(Flow::error("invalid list for substitution")),
    };
    crate::builtins::lang::substitute(f, Some(&env))
}
