//! Environments and frames: GNU R's envir.c and context.c.

use std::rc::Rc;

use super::{arg, internal, lgl_arg, prim, str_arg};
use crate::coerce::{as_integer, as_logical};
use crate::env::{Env, BIND_ACTIVE};
use crate::error::{Flow, RResult};
use crate::interp::{Context, Interp};
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("globalenv", do_globalenv, 0),
    prim("emptyenv", do_emptyenv, 0),
    prim("baseenv", do_baseenv, 0),
    prim("environment<-", do_set_environment, 2),
    prim("as.environment", do_as_environment, 1),
    internal("environment", do_environment, 1),
    internal("new.env", do_new_env, 3),
    internal("parent.env", do_parent_env, 1),
    internal("parent.env<-", do_set_parent_env, 2),
    internal("environmentName", do_environment_name, 1),
    internal("assign", do_assign, 4).invisible(),
    internal("get", do_get, 4),
    internal("get0", do_get0, 5),
    internal("mget", do_mget, 5),
    internal("exists", do_exists, 4),
    internal("remove", do_remove, 3).invisible(),
    internal("ls", do_ls, 3),
    internal("env2list", do_env2list, 3),
    internal("list2env", do_list2env, 2),
    internal("sys.call", do_sys_call, 1),
    internal("sys.function", do_sys_function, 1),
    internal("sys.frame", do_sys_frame, 1),
    internal("sys.nframe", do_sys_nframe, 0),
    internal("sys.calls", do_sys_calls, 0),
    internal("sys.frames", do_sys_frames, 0),
    internal("sys.parent", do_sys_parent, 1),
    internal("sys.parents", do_sys_parents, 0),
    internal("sys.on.exit", do_sys_on_exit, 0),
    internal("parent.frame", do_parent_frame, 1),
    internal("eval", do_eval, 3).custom_vis(),
    internal("delayedAssign", do_delayed_assign, 4).invisible(),
    internal("makeActiveBinding", do_make_active_binding, 3).invisible(),
    internal("lockEnvironment", do_lock_environment, 2).invisible(),
    internal("environmentIsLocked", do_environment_is_locked, 1),
    internal("lockBinding", do_lock_binding, 2).invisible(),
    internal("unlockBinding", do_unlock_binding, 2).invisible(),
    internal("bindingIsLocked", do_binding_is_locked, 2),
    internal("bindingIsActive", do_binding_is_active, 2),
    internal("topenv", do_topenv, 2),
    internal("isNamespaceEnv", do_is_namespace_env, 1),
    internal("search", do_search, 0),
    super::prim("pos.to.env", do_pos_to_env, 1),
];

fn do_globalenv(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.global.clone()))
}

fn do_emptyenv(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.empty.clone()))
}

fn do_baseenv(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.base.clone()))
}

pub fn as_character_factor(v: &Value) -> Value {
    let levels = match v.get_attr(s::LEVELS) {
        Some(Value::Str(l)) => l.data.clone(),
        _ => vec![],
    };
    let codes = match v {
        Value::Int(x) => x.data.clone(),
        _ => vec![],
    };
    let out: Vec<RStr> = codes
        .iter()
        .map(|&c| if c == NA_INTEGER || c < 1 || c as usize > levels.len() { None } else { levels[c as usize - 1].clone() })
        .collect();
    let mut r = Value::strs(out);
    if let Some(n) = v.get_attr(s::NAMES) {
        r.set_attr_raw(s::NAMES, n);
    }
    r
}

/// Convert something to an environment (R's `simple_as_environment` plus
/// `pos.to.env` for numbers).
pub fn as_env(interp: &mut Interp, v: &Value, call: &Value) -> RResult<Env> {
    match v {
        Value::Env(e) => Ok(e.clone()),
        Value::Real(_) | Value::Int(_) => {
            let n = as_integer(v);
            pos_to_env(interp, n, call)
        }
        Value::Str(sv) if sv.data.len() == 1 => {
            let name = sv.data[0].as_deref().unwrap_or("");
            // R: envir.c:matchEnvir
            match name {
                ".GlobalEnv" => Ok(interp.global.clone()),
                "package:base" => Ok(interp.base.clone()),
                _ => {
                    let mut t = interp.global.parent();
                    while let Some(e) = t {
                        if e.ptr_eq(&interp.empty) {
                            break;
                        }
                        if let Some(Value::Str(n)) = Value::Env(e.clone()).get_attr(Sym::new("name")) {
                            if n.data.first().and_then(|s| s.as_deref()) == Some(name) {
                                return Ok(e);
                            }
                        }
                        t = e.parent();
                    }
                    Err(Flow::error_call(call.clone(), format!("no item called \"{}\" on the search list", name)))
                }
            }
        }
        Value::List(_) => {
            let e = Env::new(Some(interp.empty.clone()));
            list_into_env(v, &e);
            Ok(e)
        }
        Value::Null => Err(Flow::error_call(call.clone(), "using 'as.environment(NULL)' is defunct")),
        _ => Err(Flow::error_call(call.clone(), "invalid object for 'as.environment'")),
    }
}

fn list_into_env(v: &Value, e: &Env) {
    let items = crate::coerce::to_list(v);
    if let Some(Value::Str(n)) = v.names() {
        for (nm, val) in n.data.iter().zip(items) {
            if let Some(nm) = nm {
                if !nm.is_empty() {
                    e.set_local(Sym::new(nm), val);
                }
            }
        }
    }
}

fn pos_to_env(interp: &mut Interp, n: i32, call: &Value) -> RResult<Env> {
    if n == -1 {
        // R: envir.c:do_pos2env: the calling environment of the innermost
        // function context.
        return match interp.current_frame() {
            Some(i) => Ok(interp.frames[i].sysparent.clone()),
            None => Err(Flow::error_call(call.clone(), "no enclosing environment")),
        };
    }
    if n == NA_INTEGER || n < 1 {
        return Err(Flow::error_call(call.clone(), "invalid 'pos' argument"));
    }
    // Walk the search path: global, ..., base.
    let mut e = Some(interp.global.clone());
    let mut k = 1;
    while let Some(env) = e {
        if k == n {
            return Ok(env);
        }
        if env.ptr_eq(&interp.base) {
            break;
        }
        e = env.parent();
        k += 1;
    }
    Err(Flow::error_call(call.clone(), "invalid 'pos' argument"))
}

fn do_as_environment(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    if v.is_object() {
        if let Some(r) = i.try_dispatch("as.environment", call, &args, env)? {
            return Ok(r);
        }
    }
    Ok(Value::Env(as_env(i, &v, call)?))
}

fn do_set_environment(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = arg(&args, 0);
    let e = arg(&args, 1);
    match (&f, &e) {
        (Value::Closure(c), Value::Env(e)) => {
            let mut c2 = (**c).clone();
            c2.env = e.clone();
            Ok(Value::Closure(Rc::new(c2)))
        }
        (Value::Closure(_), _) => Err(Flow::error_call(call.clone(), "replacement object is not an environment")),
        (_, Value::Env(_)) | (_, Value::Null) => {
            let mut f = f.clone();
            f.set_attr_raw(s::DOT_ENVIRONMENT, e.clone());
            Ok(f)
        }
        _ => Err(Flow::error_call(call.clone(), "replacement object is not an environment")),
    }
}

fn do_environment(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match arg(&args, 0) {
        Value::Null => {
            // The calling environment of `environment()`.
            let Some(idx) = i.current_frame() else { return Ok(Value::Env(i.global.clone())) };
            Ok(Value::Env(i.frames[idx].sysparent.clone()))
        }
        Value::Closure(c) => Ok(Value::Env(c.env.clone())),
        Value::Builtin(_) => Ok(Value::Null),
        v => Ok(v.get_attr(s::DOT_ENVIRONMENT).unwrap_or(Value::Null)),
    }
}

fn do_new_env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let parent = match arg(&args, 1) {
        Value::Env(e) => e,
        Value::Null => return Err(Flow::error_call(call.clone(), "use of NULL environment is defunct")),
        _ => return Err(Flow::error_call(call.clone(), "'enclos' must be an environment")),
    };
    let _ = i;
    Ok(Value::Env(Env::new(Some(parent))))
}

fn do_parent_env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match arg(&args, 0) {
        Value::Env(e) => {
            if e.ptr_eq(&i.empty) {
                return Err(Flow::error_call(call.clone(), "the empty environment has no parent"));
            }
            Ok(e.parent().map(Value::Env).unwrap_or(Value::Null))
        }
        _ => Err(Flow::error_call(call.clone(), "argument is not an environment")),
    }
}

fn do_set_parent_env(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match (arg(&args, 0), arg(&args, 1)) {
        (Value::Env(e), Value::Env(p)) => {
            e.set_parent(Some(p));
            Ok(Value::Env(e))
        }
        (Value::Env(_), _) => Err(Flow::error_call(call.clone(), "'parent' is not an environment")),
        _ => Err(Flow::error_call(call.clone(), "argument is not an environment")),
    }
}

fn do_environment_name(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    let name = match &v {
        Value::Env(e) => {
            if e.ptr_eq(&i.global) {
                "R_GlobalEnv".to_string()
            } else if e.ptr_eq(&i.base) {
                "base".to_string()
            } else if e.ptr_eq(&i.empty) {
                "R_EmptyEnv".to_string()
            } else if let Some(n) = e.package_env_name() {
                n.to_string()
            } else if let Some(n) = e.namespace_name() {
                n.to_string()
            } else if let Some(Value::Str(n)) = v.get_attr(Sym::new("name")) {
                n.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_default()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    Ok(Value::str(&name))
}

fn env_arg(i: &mut Interp, v: &Value, call: &Value, what: &str) -> RResult<Env> {
    match v {
        Value::Env(e) => Ok(e.clone()),
        Value::Null => Err(Flow::error_call(call.clone(), "use of NULL environment is defunct")),
        _ => {
            let e = as_env(i, v, call);
            e.map_err(|_| Flow::error_call(call.clone(), format!("invalid '{}' argument", what)))
        }
    }
}

fn do_assign(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() => match &s.data[0] {
            Some(n) => n.clone(),
            None => return Err(Flow::error_call(call.clone(), "invalid first argument")),
        },
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let value = arg(&args, 1);
    let env = env_arg(i, &arg(&args, 2), call, "envir")?;
    let inherits = lgl_arg(&arg(&args, 3), "inherits")?;
    let sym = Sym::new(&name);
    if inherits {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if e.has_local(sym) {
                i.define(sym, value.clone(), &e)?;
                return Ok(value);
            }
            cur = e.parent();
        }
        let g = i.global.clone();
        i.define(sym, value.clone(), &g)?;
    } else {
        i.define(sym, value.clone(), &env)?;
    }
    Ok(value)
}

fn mode_matches(v: &Value, mode: &str) -> bool {
    match mode {
        "any" => true,
        "function" => v.is_function(),
        "numeric" => matches!(v, Value::Int(_) | Value::Real(_)) && !v.inherits("factor"),
        "character" => matches!(v, Value::Str(_)),
        "logical" => matches!(v, Value::Lgl(_)),
        "integer" => matches!(v, Value::Int(_)),
        "double" => matches!(v, Value::Real(_)),
        "complex" => matches!(v, Value::Cplx(_)),
        "list" => matches!(v, Value::List(_)),
        "environment" => matches!(v, Value::Env(_)),
        "name" | "symbol" => matches!(v, Value::Sym(_)),
        "call" | "language" => matches!(v, Value::Lang(_)),
        "S4" => false,
        other => v.type_name() == other,
    }
}

/// Look up a variable with a mode filter (R's `findVar1mode`).
fn find_var_mode(i: &mut Interp, sym: Sym, env: &Env, mode: &str, inherits: bool) -> RResult<Option<Value>> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let b = e.0.frame().borrow().get(sym).cloned();
        if let Some(b) = b {
            let mut v = b.value;
            if b.flags & BIND_ACTIVE != 0 {
                v = i.call_value(&v, vec![], &e)?;
            }
            if mode != "any" || matches!(v, Value::Promise(_)) {
                v = i.force(v)?;
            }
            if mode_matches(&v, mode) {
                return Ok(Some(v));
            }
        }
        if !inherits {
            break;
        }
        cur = e.parent();
    }
    Ok(None)
}

fn do_get(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let mode = str_arg(&arg(&args, 2), "mode")?;
    let inherits = lgl_arg(&arg(&args, 3), "inherits")?;
    let sym = Sym::new(&name);
    match find_var_mode(i, sym, &env, &mode, inherits)? {
        Some(v) => {
            if v.is_missing_arg() {
                return Err(Flow::error_call(call.clone(), format!("argument \"{}\" is missing, with no default", name)));
            }
            Ok(v)
        }
        None => {
            if &*mode == "any" {
                Err(Flow::error_call(call.clone(), format!("object '{}' not found", name)))
            } else {
                Err(Flow::error_call(call.clone(), format!("object '{}' of mode '{}' was not found", name, mode)))
            }
        }
    }
}

fn do_get0(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let mode = str_arg(&arg(&args, 2), "mode")?;
    let inherits = lgl_arg(&arg(&args, 3), "inherits")?;
    match find_var_mode(i, Sym::new(&name), &env, &mode, inherits)? {
        Some(v) => Ok(v),
        None => Ok(arg(&args, 4)),
    }
}

fn do_mget(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let names = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let mode = str_arg(&arg(&args, 2), "mode")?;
    let ifnotfound = arg(&args, 3);
    let inherits = lgl_arg(&arg(&args, 4), "inherits")?;
    let mut out = Vec::new();
    for (k, n) in names.iter().enumerate() {
        let n = n.clone().unwrap_or_else(|| Rc::from("NA"));
        match find_var_mode(i, Sym::new(&n), &env, &mode, inherits)? {
            Some(v) => out.push(v),
            None => {
                let items = crate::coerce::to_list(&ifnotfound);
                if items.is_empty() {
                    return Err(Flow::error_call(call.clone(), format!("value for '{}' not found", n)));
                }
                let d = items[k % items.len()].clone();
                if d.is_function() {
                    let r = i.call_value(&d, vec![Arg::pos(Value::str(&n))], &env)?;
                    out.push(r);
                } else {
                    out.push(d);
                }
            }
        }
    }
    let mut v = Value::list(out);
    v.set_attr_raw(s::NAMES, Value::strs(names));
    Ok(v)
}

fn do_exists(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() && s.data[0].is_some() => s.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let mode = str_arg(&arg(&args, 2), "mode")?;
    let inherits = lgl_arg(&arg(&args, 3), "inherits")?;
    let sym = Sym::new(&name);
    if &*mode == "any" {
        let mut cur = Some(env);
        while let Some(e) = cur {
            if e.has_local(sym) {
                return Ok(Value::lgl(true));
            }
            if !inherits {
                break;
            }
            cur = e.parent();
        }
        return Ok(Value::lgl(false));
    }
    Ok(Value::lgl(find_var_mode(i, sym, &env, &mode, inherits)?.is_some()))
}

fn do_remove(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let names = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        Value::Null => vec![],
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let inherits = lgl_arg(&arg(&args, 2), "inherits")?;
    for n in names.iter() {
        let n = n.clone().unwrap_or_else(|| Rc::from("NA"));
        let sym = Sym::new(&n);
        let mut cur = Some(env.clone());
        let mut done = false;
        while let Some(e) = cur {
            if e.remove(sym) {
                done = true;
                break;
            }
            if !inherits {
                break;
            }
            cur = e.parent();
        }
        if !done {
            i.warningcall(call, format!("object '{}' not found", n))?;
        }
    }
    Ok(Value::Null)
}

fn env_names(e: &Env, all: bool, sorted: bool) -> Vec<String> {
    let mut names: Vec<String> = e
        .symbols()
        .into_iter()
        .map(|s| s.name().to_string())
        .filter(|n| all || !n.starts_with('.'))
        .collect();
    if sorted {
        names.sort();
    }
    names
}

fn do_ls(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "envir")?;
    let all = lgl_arg(&arg(&args, 1), "all.names")?;
    let sorted = lgl_arg(&arg(&args, 2), "sorted")?;
    Ok(Value::strs_from(env_names(&env, all, sorted)))
}

fn do_env2list(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "x")?;
    let all = lgl_arg(&arg(&args, 1), "all.names")?;
    let sorted = lgl_arg(&arg(&args, 2), "sorted")?;
    let names = env_names(&env, all, sorted);
    let mut vals = Vec::new();
    for n in &names {
        let v = env.get_local(Sym::new(n)).unwrap_or(Value::Null);
        vals.push(i.force(v)?);
    }
    let mut v = Value::list(vals);
    v.set_attr_raw(s::NAMES, Value::strs_from(names));
    Ok(v)
}

fn do_list2env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let names = x.names();
    let items = crate::coerce::to_list(&x);
    if !items.is_empty() {
        match names {
            Some(Value::Str(n)) => {
                for (nm, v) in n.data.iter().zip(items) {
                    i.define(Sym::new(nm.as_deref().unwrap_or("NA")), v, &env)?;
                }
            }
            _ => return Err(Flow::error_call(call.clone(), "names(x) must be a character vector of the same length as x")),
        }
    }
    Ok(Value::Env(env))
}

// ----- frames ---------------------------------------------------------

/// Function frames (excluding builtin dispatch frames), outermost first.
fn fn_frames(i: &Interp) -> Vec<usize> {
    (0..i.frames.len()).filter(|&k| !i.frames[k].builtin).collect()
}

/// The context for `sys.xxx()`: the function frame whose environment is
/// the caller of the sys.xxx closure (R's getLexicalContext).
fn lexical_context(i: &Interp) -> Option<usize> {
    let cur = i.current_frame()?;
    let t = i.frames[cur].sysparent.clone();
    (0..=cur).rev().find(|&k| !i.frames[k].builtin && i.frames[k].cloenv.ptr_eq(&t))
}

/// framedepth: number of function frames from cptr outwards (inclusive).
fn framedepth(i: &Interp, cptr: Option<usize>) -> i32 {
    match cptr {
        None => 0,
        Some(c) => fn_frames(i).iter().filter(|&&k| k <= c).count() as i32,
    }
}

/// Walk function frames from cptr outwards; return the frame index for
/// R's `n` (after conversion), or None for "top level".
fn nth_frame(i: &Interp, n: i32, cptr: Option<usize>) -> RResult<Option<usize>> {
    let mut n = if n > 0 { framedepth(i, cptr) - n } else { -n };
    if n < 0 {
        return Err(Flow::error("not that many frames on the stack"));
    }
    let frames: Vec<usize> = match cptr {
        Some(c) => fn_frames(i).into_iter().filter(|&k| k <= c).rev().collect(),
        None => vec![],
    };
    for k in frames {
        if n == 0 {
            return Ok(Some(k));
        }
        n -= 1;
    }
    if n == 0 {
        return Ok(None);
    }
    Err(Flow::error("not that many frames on the stack"))
}

fn do_sys_call(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'which' argument"));
    }
    let cptr = lexical_context(i);
    Ok(match nth_frame(i, n, cptr)? {
        Some(k) => i.frames[k].call.clone(),
        None => Value::Null,
    })
}

fn do_sys_function(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'which' value"));
    }
    let cptr = lexical_context(i);
    match nth_frame(i, n, cptr)? {
        Some(k) => Ok(i.frames[k].function.clone()),
        None => Err(Flow::error("not that many frames on the stack")),
    }
}

fn do_sys_frame(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'which' argument"));
    }
    if n == 0 {
        return Ok(Value::Env(i.global.clone()));
    }
    let cptr = lexical_context(i);
    Ok(match nth_frame(i, n, cptr)? {
        Some(k) => Value::Env(i.frames[k].cloenv.clone()),
        None => Value::Env(i.global.clone()),
    })
}

fn do_sys_nframe(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    Ok(Value::int(framedepth(i, cptr)))
}

fn do_sys_calls(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let mut out = Vec::new();
    for k in 1..=nframe {
        if let Some(f) = nth_frame(i, k, cptr)? {
            out.push(Arg::pos(i.frames[f].call.clone()));
        }
    }
    Ok(Value::pairlist(out))
}

fn do_sys_frames(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let mut out = Vec::new();
    for k in 1..=nframe {
        if let Some(f) = nth_frame(i, k, cptr)? {
            out.push(Arg::pos(Value::Env(i.frames[f].cloenv.clone())));
        }
    }
    Ok(Value::pairlist(out))
}

/// R_sysparent
fn sysparent(i: &Interp, mut n: i32, cptr: Option<usize>) -> i32 {
    let frames: Vec<usize> = match cptr {
        Some(c) => fn_frames(i).into_iter().filter(|&k| k <= c).rev().collect(),
        None => return 0,
    };
    let mut pos = 0;
    while pos < frames.len() && n > 1 {
        n -= 1;
        pos += 1;
    }
    if pos >= frames.len() {
        return 0;
    }
    let s = i.frames[frames[pos]].sysparent.clone();
    if s.ptr_eq(&i.global) {
        return 0;
    }
    let mut j = 0;
    let mut nn = n;
    for &k in &frames[pos..] {
        j += 1;
        if i.frames[k].cloenv.ptr_eq(&s) {
            nn = j;
        }
    }
    let r = j - nn + 1;
    r.max(0)
}

fn do_sys_parent(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'n' argument"));
    }
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let mut k = nframe;
    while n > 0 {
        k = sysparent(i, nframe - k + 1, cptr);
        n -= 1;
    }
    Ok(Value::int(k))
}

fn do_sys_parents(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let out: Vec<i32> = (0..nframe).map(|k| sysparent(i, nframe - k, cptr)).collect();
    Ok(Value::ints(out))
}

fn do_sys_on_exit(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Some(c) = lexical_context(i) else { return Ok(Value::Null) };
    let oe = &i.frames[c].on_exit;
    Ok(match oe.len() {
        0 => Value::Null,
        1 => oe[0].clone(),
        _ => Value::lang(Value::Sym(s::BRACE), oe.iter().map(|e| Arg::pos(e.clone())).collect()),
    })
}

fn do_parent_frame(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER || n < 1 {
        return Err(Flow::error("invalid 'n' value"));
    }
    // R_findParentContext(R_GlobalContext, n), where R_GlobalContext is
    // the frame of the parent.frame closure itself.
    let Some(mut cptr) = i.current_frame() else { return Ok(Value::Env(i.global.clone())) };
    let mut n = n;
    loop {
        let target = i.frames[cptr].sysparent.clone();
        let found = (0..cptr).rev().find(|&k| !i.frames[k].builtin && i.frames[k].cloenv.ptr_eq(&target));
        match found {
            Some(k) => {
                if n == 1 {
                    return Ok(Value::Env(i.frames[k].sysparent.clone()));
                }
                n -= 1;
                cptr = k;
            }
            None => return Ok(Value::Env(i.global.clone())),
        }
    }
}

fn do_eval(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let envir = arg(&args, 1);
    let enclos = arg(&args, 2);
    let enclos_env = match &enclos {
        Value::Env(e) => Some(e.clone()),
        Value::Null => None,
        _ => return Err(Flow::error_call(call.clone(), "invalid 'enclos' argument of type 'list'")),
    };
    let env = match &envir {
        Value::Env(e) => e.clone(),
        Value::Null => enclos_env.clone().unwrap_or_else(|| i.base.clone()),
        Value::List(_) => {
            let e = Env::new(Some(enclos_env.clone().unwrap_or_else(|| i.base.clone())));
            list_into_env(&envir, &e);
            e
        }
        Value::Pairlist(p) => {
            let e = Env::new(Some(enclos_env.clone().unwrap_or_else(|| i.base.clone())));
            for a in &p.items {
                if let Some(t) = a.tag {
                    e.set_local(t, a.value.clone());
                }
            }
            e
        }
        Value::Int(_) | Value::Real(_) => {
            let n = as_integer(&envir);
            // sys.frame(n) relative to the caller of eval
            if n == 0 {
                i.global.clone()
            } else {
                let cptr = lexical_context(i);
                match nth_frame(i, n, cptr)? {
                    Some(k) => i.frames[k].cloenv.clone(),
                    None => i.global.clone(),
                }
            }
        }
        _ => {
            return Err(Flow::error_call(
                call.clone(),
                format!("invalid 'envir' argument of type '{}'", envir.type_name()),
            ))
        }
    };
    // eval establishes a function-like context so that return() and
    // sys.function() behave as in GNU R.
    let ecall = i.caller_call();
    let sysparent = match i.current_frame() {
        Some(k) => i.frames[k].cloenv.clone(),
        None => i.global.clone(),
    };
    i.frames.push(Context {
        call: ecall,
        function: Value::Builtin(super::lookup_internal("eval").unwrap()),
        cloenv: env.clone(),
        sysparent,
        promargs: Rc::new(vec![]),
        on_exit: vec![],
        handlers: i.handlers.clone(),
        restarts: i.restarts.clone(),
        generic: false,
        builtin: false,
    });
    let r = (|| -> RResult<Value> {
        match &expr {
            Value::Expr(ev) => {
                let mut v = Value::Null;
                for e in ev.data.iter() {
                    v = i.eval(e, &env)?;
                }
                Ok(v)
            }
            Value::Sym(_) | Value::Lang(_) | Value::Promise(_) => i.eval(&expr, &env),
            v => {
                i.visible = true;
                Ok(v.clone())
            }
        }
    })();
    let r = match r {
        Err(Flow::Return(v, t)) if t.ptr_eq(&env) => Ok(v),
        r => r,
    };
    let vis = i.visible;
    let ctx = i.frames.pop().unwrap();
    i.handlers = ctx.handlers;
    i.restarts = ctx.restarts;
    i.visible = vis;
    r
}

fn do_delayed_assign(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = str_arg(&arg(&args, 0), "x")?;
    let value = arg(&args, 1);
    let eval_env = env_arg(i, &arg(&args, 2), call, "eval.env")?;
    let assign_env = env_arg(i, &arg(&args, 3), call, "assign.env")?;
    let p = Value::Promise(Rc::new(Promise::new(value, eval_env)));
    i.define(Sym::new(&name), p, &assign_env)?;
    Ok(Value::Null)
}

fn do_make_active_binding(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = match arg(&args, 0) {
        Value::Sym(s) => s,
        Value::Str(v) if v.data.len() == 1 => Sym::new(v.data[0].as_deref().unwrap_or("")),
        _ => return Err(Flow::error_call(call.clone(), "not a symbol")),
    };
    let f = arg(&args, 1);
    if !f.is_function() {
        return Err(Flow::error_call(call.clone(), "not a function"));
    }
    let env = env_arg(i, &arg(&args, 2), call, "env")?;
    env.set_local_flags(sym, f, BIND_ACTIVE);
    Ok(Value::Null)
}

fn do_lock_environment(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "env")?;
    let bindings = lgl_arg(&arg(&args, 1), "bindings")?;
    env.lock(bindings);
    Ok(Value::Null)
}

fn do_environment_is_locked(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "env")?;
    Ok(Value::lgl(env.0.locked.get()))
}

fn sym_arg(v: &Value, call: &Value) -> RResult<Sym> {
    match v {
        Value::Sym(s) => Ok(*s),
        Value::Str(x) if x.data.len() == 1 => Ok(Sym::new(x.data[0].as_deref().unwrap_or(""))),
        _ => Err(Flow::error_call(call.clone(), "not a symbol")),
    }
}

fn do_lock_binding(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    if !env.set_binding_locked(sym, true) {
        return Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym)));
    }
    Ok(Value::Null)
}

fn do_unlock_binding(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    if !env.set_binding_locked(sym, false) {
        return Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym)));
    }
    Ok(Value::Null)
}

fn do_binding_is_locked(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    match env.binding_flags(sym) {
        Some(f) => Ok(Value::lgl(f & crate::env::BIND_LOCKED != 0)),
        None => Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym))),
    }
}

fn do_binding_is_active(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    match env.binding_flags(sym) {
        Some(f) => Ok(Value::lgl(f & BIND_ACTIVE != 0)),
        None => Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym))),
    }
}

fn do_topenv(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let e = match arg(&args, 0) {
        Value::Env(e) => e,
        _ => env.clone(),
    };
    Ok(Value::Env(i.topenv(&e)))
}

fn do_is_namespace_env(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(match arg(&args, 0) {
        Value::Env(e) => e.ptr_eq(&i.base_ns) || crate::dispatch::is_namespace(&e),
        _ => false,
    }))
}

fn do_search(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut out = vec![".GlobalEnv".to_string()];
    let mut e = i.global.parent();
    while let Some(env) = e {
        if env.ptr_eq(&i.base) {
            out.push("package:base".into());
            break;
        }
        let name = match env.get_attr_name() {
            Some(n) => n,
            None => "<unknown>".into(),
        };
        out.push(name);
        e = env.parent();
    }
    Ok(Value::strs_from(out))
}

fn do_pos_to_env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    Ok(Value::Env(pos_to_env(i, n, call)?))
}

impl Env {
    pub fn get_attr_name(&self) -> Option<String> {
        let a = self.0.attrs.borrow();
        match a.as_ref().and_then(|a| a.get(Sym::new("name"))) {
            Some(Value::Str(v)) => v.data.first().cloned().flatten().map(|s| s.to_string()),
            _ => None,
        }
    }
}

#[allow(dead_code)]
fn unused(_: bool) -> Lgl {
    as_logical(&Value::Null)
}
