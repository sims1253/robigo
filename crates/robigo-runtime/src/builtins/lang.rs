//! Language primitives: control flow, functions, assignment, quoting,
//! `missing`, `on.exit`, `switch`, `UseMethod`, `.Internal` (GNU R's
//! eval.c, builtin.c and parts of envir.c).

use std::rc::Rc;

use super::{internal, match_args, prim, special};
use crate::coerce::{as_condition, as_integer};
use crate::env::{BIND_ACTIVE, BIND_MISSING_DEFAULT, Env};
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    special("~", do_tilde, -1),
    special("if", do_if, -1).custom_vis(),
    special("for", do_for, 3).invisible(),
    special("while", do_while, 2).invisible(),
    special("repeat", do_repeat, 1).invisible(),
    special("break", do_break, 0),
    special("next", do_next, 0),
    special("return", do_return, -1),
    special("function", do_function, -1),
    special("<-", do_assign, -1).invisible(),
    special("=", do_assign, -1).invisible(),
    special("<<-", do_super_assign, -1).invisible(),
    special("{", do_begin, -1).custom_vis(),
    prim("(", do_paren, 1),
    special("quote", do_quote, 1),
    special("substitute", do_substitute, -1),
    special("missing", do_missing, 1),
    special("on.exit", do_onexit, -1).invisible(),
    special("switch", do_switch, -1).custom_vis(),
    special("UseMethod", do_usemethod, -1).custom_vis(),
    special(".Internal", do_internal, 1).custom_vis(),
    prim(".Primitive", do_primitive, 1),
    prim("nargs", do_nargs, 0),
    prim("interactive", do_interactive, 0),
    prim("invisible", do_invisible, -1).invisible(),
    special("forceAndCall", do_force_and_call, -1).custom_vis(),
    special("...length", do_dots_length, 0),
    special("...elt", do_dots_elt, -1).custom_vis(),
    special("...names", do_dots_names, 0),
    internal("NextMethod", do_nextmethod, -1).custom_vis(),
    internal("Recall", do_recall, -1).custom_vis(),
    special("standardGeneric", do_standard_generic, -1),
];

fn do_if(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let c = interp.eval(&args[0].value, env)?;
    if as_condition(interp, &c, call)? {
        interp.eval(&args[1].value, env)
    } else if args.len() > 2 {
        interp.eval(&args[2].value, env)
    } else {
        interp.visible = false;
        Ok(Value::Null)
    }
}

/// Run a loop body, translating `break`/`next` aimed at this frame.
/// Returns Ok(true) to continue, Ok(false) on break.
fn loop_body(interp: &mut Interp, body: &Value, env: &Env) -> RResult<bool> {
    match interp.eval(body, env) {
        Ok(_) => Ok(true),
        Err(Flow::Break(e)) if e.ptr_eq(env) => Ok(false),
        Err(Flow::Next(e)) if e.ptr_eq(env) => Ok(true),
        Err(f) => Err(f),
    }
}

/// Run a loop through the bytecode VM when enabled and the loop head is
/// the base function.
fn try_compiled_loop(interp: &mut Interp, call: &Value, head: Sym, env: &Env) -> Option<RResult<Value>> {
    if !crate::vm::enabled() || !interp.resolves_to_base(head, env) {
        return None;
    }
    let code = crate::vm::code_for(call, false)?;
    Some(crate::vm::run(interp, &code, env))
}

fn do_for(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = try_compiled_loop(interp, call, s::FOR, env) {
        return r;
    }
    let Some(sym) = args[0].value.as_sym() else {
        return Err(Flow::error_call(call.clone(), "non-symbol loop variable"));
    };
    let mut val = interp.eval(&args[1].value, env)?;
    if val.inherits("factor") {
        val = crate::builtins::envir::as_character_factor(&val);
    }
    let body = args[2].value.clone();
    let n = val.len();
    interp.define(sym, Value::Null, env)?;
    for i in 0..n {
        // Reuse the previous iteration's scalar if nothing else holds it
        // (R's ALLOC_LOOP_VAR).
        if reuse_loop_var(&val, i, sym, env) {
            if !loop_body(interp, &body, env)? {
                break;
            }
            continue;
        }
        let v = match &val {
            Value::List(l) | Value::Expr(l) => l.data[i].clone(),
            Value::Pairlist(p) => p.items[i].value.clone(),
            Value::Lgl(x) => Value::lgl_raw(x.data[i]),
            Value::Int(x) => Value::int(x.data[i]),
            Value::Real(x) => Value::real(x.data[i]),
            Value::Cplx(x) => Value::cplx(x.data[i]),
            Value::Str(x) => Value::strs(vec![x.data[i].clone()]),
            Value::Raw(x) => Value::raws(vec![x.data[i]]),
            Value::Null => break,
            _ => return Err(Flow::error_call(call.clone(), "invalid for() loop sequence")),
        };
        interp.define(sym, v, env)?;
        if !loop_body(interp, &body, env)? {
            break;
        }
    }
    if !matches!(
        val,
        Value::Null
            | Value::List(_)
            | Value::Expr(_)
            | Value::Pairlist(_)
            | Value::Lgl(_)
            | Value::Int(_)
            | Value::Real(_)
            | Value::Cplx(_)
            | Value::Str(_)
            | Value::Raw(_)
    ) {
        return Err(Flow::error_call(call.clone(), "invalid for() loop sequence"));
    }
    interp.visible = false;
    Ok(Value::Null)
}

/// Overwrite the loop variable's scalar in place when it is unshared and
/// of the right type. Returns false if a fresh value is needed.
fn reuse_loop_var(val: &Value, i: usize, sym: Sym, env: &Env) -> bool {
    let mut frame = env.0.frame().borrow_mut();
    let Some(b) = frame.get_mut(sym) else { return false };
    if b.flags != 0 {
        return false;
    }
    match (&mut b.value, val) {
        (Value::Int(cur), Value::Int(src)) => match Rc::get_mut(cur) {
            Some(c) if c.data.len() == 1 && c.attrs.is_none() => {
                c.data[0] = src.data[i];
                true
            }
            _ => false,
        },
        (Value::Real(cur), Value::Real(src)) => match Rc::get_mut(cur) {
            Some(c) if c.data.len() == 1 && c.attrs.is_none() => {
                c.data[0] = src.data[i];
                true
            }
            _ => false,
        },
        _ => false,
    }
}

fn do_while(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = try_compiled_loop(interp, call, s::WHILE, env) {
        return r;
    }
    loop {
        let c = interp.eval(&args[0].value, env)?;
        if !as_condition(interp, &c, call)? {
            break;
        }
        if !loop_body(interp, &args[1].value, env)? {
            break;
        }
    }
    interp.visible = false;
    Ok(Value::Null)
}

fn do_repeat(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = try_compiled_loop(interp, call, s::REPEAT, env) {
        return r;
    }
    while loop_body(interp, &args[0].value, env)? {}
    interp.visible = false;
    Ok(Value::Null)
}

fn do_break(_: &mut Interp, _: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    Err(Flow::Break(env.clone()))
}

fn do_next(_: &mut Interp, _: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    Err(Flow::Next(env.clone()))
}

fn do_return(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let v = match args.len() {
        0 => Value::Null,
        1 => interp.eval(&args[0].value, env)?,
        _ => return Err(Flow::error_call(call.clone(), "multi-argument returns are not permitted")),
    };
    Err(Flow::Return(v, env.clone()))
}

thread_local! {
    /// The formals of each `function` expression evaluated, by the address
    /// of its formals pairlist (kept alive here): closures created by one
    /// expression share them (and the caches keyed by them).
    static FORMALS: std::cell::RefCell<std::collections::HashMap<usize, (Value, Rc<[Arg]>), crate::env::FastHash>> =
        Default::default();
}

fn do_function(_: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let formals: Rc<[Arg]> = match &args[0].value {
        Value::Pairlist(p) => {
            let key = Rc::as_ptr(p) as *const u8 as usize;
            match FORMALS.with(|c| c.borrow().get(&key).map(|(_, f)| f.clone())) {
                Some(f) => f,
                None => {
                    let f: Rc<[Arg]> = p.items.iter().cloned().collect();
                    FORMALS.with(|c| {
                        let mut c = c.borrow_mut();
                        // Pairlists built at run time are new each time:
                        // bound the cache (clearing it only loses sharing).
                        if c.len() >= 100_000 {
                            c.clear();
                        }
                        c.insert(key, (args[0].value.clone(), f.clone()))
                    });
                    f
                }
            }
        }
        Value::Null => Rc::from(Vec::new()),
        _ => return Err(Flow::error_call(call.clone(), "invalid formal argument list for \"function\"")),
    };
    let body = args.get(1).map(|a| a.value.clone()).unwrap_or(Value::Null);
    Ok(Value::Closure(Rc::new(Closure {
        formals,
        body,
        env: env.clone(),
        attrs: None,
        code: Default::default(),
        inline: Default::default(),
        defaults: Default::default(),
    })))
}

fn do_begin(interp: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let mut v = Value::Null;
    interp.visible = true;
    for a in &args {
        v = interp.eval(&a.value, env)?;
    }
    Ok(v)
}

fn do_paren(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(args.pop().map(|a| a.value).unwrap_or(Value::Null))
}

fn do_quote(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    super::check_arity(&args, 1, "quote", call)?;
    Ok(args[0].value.clone())
}

// ----------------------------------------------------------------------
// assignment

fn assign_target(lhs: &Value) -> Option<Sym> {
    match lhs {
        Value::Sym(s) => Some(*s),
        Value::Str(v) if v.data.len() == 1 => v.data[0].as_ref().map(|s| Sym::new(s)),
        _ => None,
    }
}

fn do_assign(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error_call(call.clone(), "invalid assignment"));
    }
    let lhs = &args[0].value;
    if let Some(sym) = assign_target(lhs) {
        let v = interp.eval(&args[1].value, env)?;
        interp.define(sym, v.clone(), env)?;
        interp.visible = false;
        return Ok(v);
    }
    match lhs {
        Value::Lang(_) => {
            let v = interp.eval(&args[1].value, env)?;
            complex_assign(interp, call, lhs, &args[1].value, v.clone(), env, false)?;
            interp.visible = false;
            Ok(v)
        }
        _ => Err(Flow::error_call(call.clone(), "invalid (do_set) left-hand side to assignment")),
    }
}

fn do_super_assign(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error_call(call.clone(), "invalid assignment"));
    }
    let lhs = &args[0].value;
    if let Some(sym) = assign_target(lhs) {
        let v = interp.eval(&args[1].value, env)?;
        interp.super_assign(sym, v.clone(), env)?;
        interp.visible = false;
        return Ok(v);
    }
    match lhs {
        Value::Lang(_) => {
            let v = interp.eval(&args[1].value, env)?;
            complex_assign(interp, call, lhs, &args[1].value, v.clone(), env, true)?;
            interp.visible = false;
            Ok(v)
        }
        _ => Err(Flow::error_call(call.clone(), "invalid (do_set) left-hand side to assignment")),
    }
}

/// The replacement function for a getter head: `f` -> `f<-`,
/// `pkg::f` -> `pkg::\`f<-\``.
/// `f` -> `f<-`, memoized (complex assignments look it up every time).
fn replacement_sym(f: Sym) -> Sym {
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<Sym, Sym, crate::env::FastHash>> =
            std::cell::RefCell::new(Default::default());
    }
    CACHE.with(|c| *c.borrow_mut().entry(f).or_insert_with(|| Sym::new(&format!("{}<-", f))))
}

fn assign_fun(head: &Value) -> RResult<Value> {
    match head {
        Value::Sym(s) => Ok(Value::Sym(replacement_sym(*s))),
        Value::Str(v) if v.data.len() == 1 => {
            Ok(Value::Sym(Sym::new(&format!("{}<-", v.data[0].as_deref().unwrap_or("")))))
        }
        Value::Lang(l) => {
            let ns = l.fun_sym().map(|s| s.name());
            if matches!(ns.as_deref(), Some("::") | Some(":::")) && l.args.len() == 2 {
                let fname = match &l.args[1].value {
                    Value::Sym(s) => s.name().to_string(),
                    Value::Str(v) => v.data[0].as_deref().unwrap_or("").to_string(),
                    _ => return Err(Flow::error("invalid function in complex assignment")),
                };
                Ok(Value::lang(
                    l.fun.clone(),
                    vec![l.args[0].clone(), Arg::pos(Value::Sym(Sym::new(&format!("{}<-", fname))))],
                ))
            } else {
                Err(Flow::error("invalid function in complex assignment"))
            }
        }
        _ => Err(Flow::error("invalid function in complex assignment")),
    }
}

/// R's `applydefine`: `f(g(x, a), b) <- value`.
pub(crate) fn complex_assign(
    interp: &mut Interp,
    call: &Value,
    lhs: &Value,
    rhs_expr: &Value,
    rhs: Value,
    env: &Env,
    superassign: bool,
) -> RResult<()> {
    complex_assign_pre(interp, call, lhs, rhs_expr, rhs, env, superassign, None)
}

/// [`complex_assign`] for a single-level `f(x, i, ...) <- value` whose index
/// arguments `i, ...` were already evaluated (by compiled code) to `pre`:
/// they are not evaluated again, and calls in messages still show their
/// expressions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn complex_assign_pre(
    interp: &mut Interp,
    call: &Value,
    lhs: &Value,
    rhs_expr: &Value,
    rhs: Value,
    env: &Env,
    superassign: bool,
    pre: Option<&[Value]>,
) -> RResult<()> {
    // Unwind the chain of getter calls down to the target symbol.
    let mut levels: Vec<Rc<Lang>> = Vec::new();
    let mut cur = lhs.clone();
    let target = loop {
        match &cur {
            Value::Lang(l) => {
                levels.push(l.clone());
                let inner = l.args.first().map(|a| a.value.clone()).unwrap_or(Value::Null);
                cur = inner;
            }
            Value::Sym(s) => break *s,
            Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => {
                break Sym::new(v.data[0].as_ref().unwrap());
            }
            Value::Null => return Err(Flow::error_call(call.clone(), "invalid (NULL) left side of assignment")),
            _ => return Err(Flow::error_call(call.clone(), "target of assignment expands to non-language object")),
        }
    };
    if env.ptr_eq(&interp.base) {
        return Err(Flow::error_call(call.clone(), "cannot do complex assignments in base environment"));
    }
    interp.ccode_calls.push((interp.frames.len(), call.clone()));
    let r = complex_assign_inner(interp, call, &levels, target, rhs_expr, rhs, env, superassign, pre);
    // Errors raised with `error()` by the replacement functions report the
    // assignment call: signal them while its context is still active.
    let r = match r {
        Err(Flow::Error(e)) if !e.signaled && e.call.is_none() => Err(interp.signal_error(e)),
        r => r,
    };
    interp.ccode_calls.pop();
    r
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn complex_assign_inner(
    interp: &mut Interp,
    _call: &Value,
    levels: &[Rc<Lang>],
    target: Sym,
    rhs_expr: &Value,
    rhs: Value,
    env: &Env,
    superassign: bool,
    pre: Option<&[Value]>,
) -> RResult<()> {
    // Base value (EnsureLocal for `<-`).
    let base_val = if superassign {
        let parent = env.parent().unwrap_or_else(|| interp.global.clone());
        match interp.eval_sym(target, &parent) {
            Ok(v) => v,
            Err(Flow::Error(e)) if e.message.starts_with("object '") => {
                return Err(Flow::error(format!("object '{}' not found", target)));
            }
            Err(f) => return Err(f),
        }
    } else if env.has_local(target) {
        interp.eval_sym(target, env)?
    } else {
        let parent = env.parent().unwrap_or_else(|| interp.global.clone());
        let v = match interp.eval_sym(target, &parent) {
            Ok(v) => v,
            Err(Flow::Error(e)) if e.message.starts_with("object '") => {
                return Err(Flow::error(format!("object '{}' not found", target)));
            }
            Err(f) => return Err(f),
        };
        interp.define(target, v.clone(), env)?;
        v
    };

    // Fast path: `x[i] <- v`, `names(x) <- v`, ... with a builtin
    // replacement function on a local, non-object value: modify in place.
    // (Not for active bindings: their binding holds the function.)
    let active = env.binding_flags(target).is_some_and(|f| f & BIND_ACTIVE != 0);
    if levels.len() == 1 && !superassign && !base_val.is_object() && !active {
        let l = &levels[0];
        let fname = assign_fun(&l.fun)?;
        if let Value::Sym(fsym) = &fname {
            if let Ok(Value::Builtin(b)) = interp.find_fun(*fsym, env) {
                if b.kind == BuiltinKind::Builtin || is_inplace_special(b.name) {
                    drop(base_val);
                    return fast_replace(interp, b, *fsym, l, target, rhs, env, pre);
                }
            }
        }
    }

    // General path, with `*tmp*` bound to intermediate values.
    let mut vals: Vec<Value> = vec![base_val];
    // Evaluate getters for all levels except the outermost.
    for i in (1..levels.len()).rev() {
        let l = &levels[i];
        env.set_local(s::TMP, vals.last().unwrap().clone());
        let mut args = vec![Arg::pos(Value::Sym(s::TMP))];
        args.extend(l.args.iter().skip(1).cloned());
        let getter = Value::lang(l.fun.clone(), args);
        let r = interp.eval(&getter, env);
        let v = match r {
            Ok(v) => v,
            Err(f) => {
                env.remove(s::TMP);
                return Err(f);
            }
        };
        vals.push(v);
    }
    // vals[k] is the value at depth k (0 = target symbol). Level i (0 =
    // outermost) operates on the value at depth levels.len() - 1 - i.
    let rhs_prom = Rc::new(Promise::forced(rhs_expr.clone(), rhs));
    let mut new_value = Value::Promise(rhs_prom);
    let n = levels.len();
    for (i, l) in levels.iter().enumerate() {
        let depth = n - 1 - i;
        let obj = vals[depth].clone();
        env.set_local(s::TMP, obj);
        let fname = assign_fun(&l.fun)?;
        let mut args = vec![Arg::pos(Value::Sym(s::TMP))];
        match pre {
            // Already evaluated indices reach the replacement function as
            // forced promises.
            Some(vals) if i == 0 => {
                args.extend(l.args.iter().skip(1).zip(vals.iter()).map(|(a, v)| {
                    Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced(a.value.clone(), v.clone()))))
                }))
            }
            _ => args.extend(l.args.iter().skip(1).cloned()),
        }
        args.push(Arg::new(Some(s::VALUE), new_value));
        let setter = Value::lang(fname, args);
        let r = interp.eval(&setter, env);
        match r {
            Ok(v) => {
                new_value = Value::Promise(Rc::new(Promise::forced(v.clone(), v)));
            }
            Err(f) => {
                env.remove(s::TMP);
                return Err(f);
            }
        }
    }
    env.remove(s::TMP);
    let final_value = match new_value {
        Value::Promise(p) => p.value().unwrap(),
        v => v,
    };
    if superassign { interp.super_assign(target, final_value, env) } else { interp.define(target, final_value, env) }
}

fn is_inplace_special(name: &str) -> bool {
    matches!(name, "[<-" | "[[<-" | "$<-")
}

/// In-place replacement for a single-level assignment with a builtin.
/// `pre` holds the index arguments when compiled code evaluated them.
#[allow(clippy::too_many_arguments)]
fn fast_replace(
    interp: &mut Interp,
    b: &'static Builtin,
    fsym: Sym,
    l: &Lang,
    target: Sym,
    rhs: Value,
    env: &Env,
    pre: Option<&[Value]>,
) -> RResult<()> {
    crate::builtins::subassign::clear_restore();
    // Evaluate the extra (index) arguments while `target` is still bound.
    let extra: Vec<Arg> = l.args.iter().skip(1).cloned().collect();
    let mut argv = Vec::with_capacity(extra.len() + 2);
    argv.push(Arg::pos(Value::Null));
    if let Some(vals) = pre {
        for (a, v) in extra.iter().zip(vals.iter()) {
            let v = if b.kind == BuiltinKind::Special { quote_if_needed(v.clone()) } else { v.clone() };
            argv.push(Arg::new(a.tag, v));
        }
    } else if b.kind == BuiltinKind::Special {
        if b.name == "$<-" {
            // `$<-` matches its name argument; it is not evaluated.
            argv.extend(extra.iter().cloned());
        } else {
            // `[<-` and friends take unevaluated arguments; evaluate the
            // indices here (keeping missing ones) and pass quoted values.
            for e in crate::builtins::subset::eval_keep_missing(interp, &extra, env)? {
                let v = if e.value.is_missing_arg() { e.value } else { quote_if_needed(e.value) };
                argv.push(Arg::new(e.tag, v));
            }
        }
    } else {
        let ev = interp.eval_args(&extra, env, &Value::Null)?;
        argv.extend(ev);
    }
    // R's call shows the value of the right-hand side promise.
    let rhs_shown = rhs.clone();
    argv.push(Arg::new(Some(s::VALUE), if b.kind == BuiltinKind::Special { quote_if_needed(rhs) } else { rhs }));
    // Now take the value out of its binding so we hold the only reference.
    let obj = match env.take_local(target).unwrap_or(Value::Null) {
        // An argument: its promise was forced when the target was looked up.
        Value::Promise(p) => p.value().unwrap_or(Value::Null),
        v => v,
    };
    argv[0].value = if b.kind == BuiltinKind::Special { quote_if_needed(obj) } else { obj };
    // The call as R would construct it, for error messages.
    let mut cargs = vec![Arg::pos(Value::Sym(s::TMP))];
    cargs.extend(extra);
    cargs.push(Arg::new(Some(s::VALUE), rhs_shown));
    let rcall = Value::lang(Value::Sym(fsym), cargs);
    let r = if b.kind == BuiltinKind::Special {
        // Specials evaluate their arguments; give them a scratch env where
        // values are already quoted.
        interp.invoke_builtin(b, &rcall, argv, env)
    } else {
        interp.invoke_builtin(b, &rcall, argv, env)
    };
    match r {
        Ok(v) => interp.define(target, v, env),
        Err(f) => {
            // The replacement function owned the object; `[<-` and friends
            // hand it back (unchanged) when they fail.
            if let Some(orig) = crate::builtins::subassign::take_restore() {
                env.set_local(target, orig);
            }
            Err(f)
        }
    }
}

/// Wrap language objects in `quote()` so that evaluating them yields the
/// value itself; plain values evaluate to themselves already.
pub fn quote_if_needed(v: Value) -> Value {
    match &v {
        Value::Sym(_) | Value::Lang(_) | Value::Promise(_) => Value::call_sym("quote", vec![v]),
        _ => v,
    }
}

// ----------------------------------------------------------------------
// substitute, missing, on.exit

// R: coerce.c:substitute
pub fn substitute(e: &Value, env: Option<&Env>) -> RResult<Value> {
    Ok(match e {
        Value::Promise(p) => return substitute(&p.expr, env),
        Value::Sym(sym) => {
            if let Some(env) = env {
                if let Some(t) = env.get_local(*sym) {
                    match t {
                        Value::Promise(p) => {
                            let mut ex = p.expr.clone();
                            while let Value::Promise(q) = ex {
                                ex = q.expr.clone();
                            }
                            return Ok(ex);
                        }
                        Value::Dots(_) => return Err(Flow::error("'...' used in an incorrect context")),
                        t => {
                            if !env.special_name().is_some_and(|n| &*n == "R_GlobalEnv") {
                                return Ok(t);
                            }
                        }
                    }
                }
            }
            e.clone()
        }
        Value::Lang(l) => {
            let items = substitute_list(&l.items(), env)?;
            // Expanding dots at the head inserts LISTSXP nodes. It does
            // not turn the first dot argument into the function of a call.
            let dots_head = l.fun.as_sym() == Some(s::DOTS) && env.and_then(|e| e.get_local(s::DOTS)).is_some();
            if items.is_empty() {
                Value::Null
            } else if dots_head {
                Value::pairlist(items)
            } else {
                Value::Lang(Rc::new(Lang::from_items(items, l.attrs.clone())))
            }
        }
        _ => e.clone(),
    })
}

// R: coerce.c:substituteList
fn substitute_list(items: &[Arg], env: Option<&Env>) -> RResult<Vec<Arg>> {
    let mut out = Vec::with_capacity(items.len());
    for a in items {
        if a.value.as_sym() == Some(s::DOTS) {
            match env.and_then(|e| e.get_local(s::DOTS)) {
                Some(Value::Dots(d)) => {
                    for item in d.iter() {
                        out.push(Arg::new(item.tag, substitute(&item.value, None)?));
                    }
                }
                Some(v) if v.is_missing_arg() || v.is_null() => {}
                None => out.push(a.clone()),
                _ => return Err(Flow::error("'...' used in an incorrect context")),
            }
        } else {
            out.push(Arg::new(a.tag, substitute(&a.value, env)?));
        }
    }
    Ok(out)
}

fn do_substitute(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let m = match_args(args, &["expr", "env"], call)?;
    let expr = m.vals[0].clone().unwrap_or(Value::missing_arg());
    let senv = match &m.vals[1] {
        None => Some(env.clone()),
        Some(e) => {
            let ev = interp.eval(e, env)?;
            match ev {
                Value::Env(e) => {
                    if e.ptr_eq(&interp.global) {
                        None
                    } else {
                        Some(e)
                    }
                }
                Value::List(_) | Value::Pairlist(_) => {
                    let ne = Env::new(Some(interp.base.clone()));
                    if let Some(Value::Str(names)) = ev.names() {
                        let vals = crate::coerce::to_list(&ev);
                        for (n, v) in names.data.iter().zip(vals) {
                            if let Some(n) = n {
                                if !n.is_empty() {
                                    ne.set_local(Sym::new(n), v);
                                }
                            }
                        }
                    }
                    Some(ne)
                }
                Value::Null => None,
                _ => return Err(Flow::error_call(call.clone(), "invalid environment specified")),
            }
        }
    };
    let senv = senv.filter(|e| !e.ptr_eq(&interp.global));
    // R: coerce.c:do_substitute wraps expr in a pairlist before expanding
    // dots, then returns the first element (NULL for an empty dots list).
    let items = substitute_list(&[Arg::pos(expr)], senv.as_ref())?;
    Ok(items.into_iter().next().map(|a| a.value).unwrap_or(Value::Null))
}

pub fn is_missing_in(interp: &Interp, sym: Sym, env: &Env, depth: usize) -> bool {
    if sym.is_empty() {
        return true;
    }
    if depth > 100 {
        return false;
    }
    let (sym, ddv) = match sym.ddval() {
        Some(i) => (s::DOTS, Some(i)),
        None => (sym, None),
    };
    let frame = env.0.frame().borrow();
    let Some(b) = frame.get(sym) else { return false };
    let mut v = b.value.clone();
    let flags = b.flags;
    drop(frame);
    if let Some(i) = ddv {
        match &v {
            Value::Dots(d) if d.len() >= i => v = d[i - 1].value.clone(),
            _ => return true,
        }
    } else if flags & BIND_MISSING_DEFAULT != 0 || v.is_missing_arg() {
        return true;
    }
    if v.is_missing_arg() {
        return true;
    }
    if let Value::Promise(p) = &v {
        if p.value().is_none() {
            if let Value::Sym(s2) = &p.expr {
                if let Some(penv) = p.env.borrow().clone() {
                    return is_missing_in(interp, *s2, &penv, depth + 1);
                }
            }
        }
    }
    false
}

fn do_missing(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    super::check_arity(&args, 1, "missing", call)?;
    let sym = match &args[0].value {
        Value::Sym(s) => *s,
        Value::Str(v) if v.data.len() == 1 => Sym::new(v.data[0].as_deref().unwrap_or("")),
        _ => return Err(Flow::error_call(call.clone(), "invalid use of 'missing'")),
    };
    Ok(Value::lgl(missing_sym(interp, call, sym, env)?))
}

/// `missing(sym)` in `env`.
// R: envir.c:do_missing
pub fn missing_sym(interp: &mut Interp, call: &Value, sym: Sym, env: &Env) -> RResult<bool> {
    let (lookup, ddv) = match sym.ddval() {
        Some(i) => (s::DOTS, Some(i)),
        None => (sym, None),
    };
    let binding = env.0.frame().borrow().get(lookup).cloned();
    let Some(b) = binding else {
        // (For `..N`, R reports the `...` it looked up.)
        let _ = call;
        return Err(Flow::error(format!("'missing({})' did not find an argument", lookup)));
    };
    let mut v = b.value;
    if let Some(i) = ddv {
        match &v {
            Value::Dots(d) if d.len() >= i => v = d[i - 1].value.clone(),
            _ => return Ok(true),
        }
    } else if b.flags & BIND_MISSING_DEFAULT != 0 || v.is_missing_arg() {
        return Ok(true);
    }
    if v.is_missing_arg() {
        return Ok(true);
    }
    let Value::Promise(mut p) = v else { return Ok(false) };
    // findRootPromise
    while let Value::Promise(q) = &p.expr {
        p = q.clone();
    }
    let r = match &p.expr {
        Value::Sym(s2) => match p.env.borrow().clone() {
            Some(penv) => is_missing_in(interp, *s2, &penv, 0),
            None => false,
        },
        _ => false,
    };
    Ok(r)
}

fn do_onexit(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let m = match_args(args, &["expr", "add", "after"], call)?;
    let expr = m.vals[0].clone();
    let add = match &m.vals[1] {
        Some(e) => {
            let v = interp.eval(e, env)?;
            let l = crate::coerce::as_logical(&v);
            if l == NA_LOGICAL {
                return Err(Flow::error_call(call.clone(), "invalid 'add' argument"));
            }
            l != 0
        }
        None => false,
    };
    let after = match &m.vals[2] {
        Some(e) => {
            let v = interp.eval(e, env)?;
            let l = crate::coerce::as_logical(&v);
            if l == NA_LOGICAL {
                return Err(Flow::error_call(call.clone(), "invalid 'after' argument"));
            }
            l != 0
        }
        None => true,
    };
    if let Some(i) = interp.frame_of_env(env) {
        let ctx = &mut interp.frames[i];
        match expr {
            None => ctx.on_exit.clear(),
            Some(e) => {
                if !add {
                    ctx.on_exit.clear();
                    ctx.on_exit.push(e);
                } else if after {
                    ctx.on_exit.push(e);
                } else {
                    ctx.on_exit.insert(0, e);
                }
            }
        }
    }
    Ok(Value::Null)
}

fn do_switch(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.is_empty() {
        return Err(Flow::error_call(call.clone(), "'EXPR' is missing"));
    }
    let x = interp.eval(&args[0].value, env)?;
    if !x.is_vector() || x.len() != 1 {
        return Err(Flow::error_call(call.clone(), "EXPR must be a length 1 vector"));
    }
    if x.inherits("factor") {
        interp.warningcall(
            call,
            "EXPR is a \"factor\", treated as integer.\n Consider using 'switch(as.character( * ), ...)' instead.",
        )?;
    }
    interp.visible = true;
    if args.len() > 1 {
        // expand ... in the alternatives
        let mut w: Vec<Arg> = Vec::new();
        for a in &args[1..] {
            if a.value.as_sym() == Some(s::DOTS) {
                if let Some((Value::Dots(d), _)) = env.lookup(s::DOTS) {
                    w.extend(d.iter().cloned());
                }
            } else {
                w.push(a.clone());
            }
        }
        if let Value::Str(sv) = &x {
            // pmatch() compares CHAR(): NA_STRING is "NA".
            let key: Option<std::rc::Rc<str>> = Some(sv.data[0].clone().unwrap_or_else(|| "NA".into()));
            let mut dflt: Option<Value> = None;
            let set_dflt = |d: &mut Option<Value>, v: &Value| -> RResult<()> {
                if let Some(old) = d {
                    let a = crate::deparse::deparse_one_line(old);
                    let b = crate::deparse::deparse_one_line(v);
                    let a = if a.len() > 20 { format!("{}...", &a[..20]) } else { a };
                    let b = if b.len() > 20 { format!("{}...", &b[..20]) } else { b };
                    return Err(Flow::error(format!("duplicate 'switch' defaults: '{}' and '{}'", a, b)));
                }
                *d = Some(v.clone());
                Ok(())
            };
            let mut i = 0;
            while i < w.len() {
                let y = &w[i];
                if let Some(tag) = y.tag {
                    if key.as_deref().is_some_and(|k| *k == *tag.name()) {
                        let mut j = i;
                        while w[j].value.is_missing_arg() {
                            j += 1;
                            if j >= w.len() {
                                interp.visible = false;
                                return Ok(Value::Null);
                            }
                            if w[j].tag.is_none() {
                                set_dflt(&mut dflt, &w[j].value)?;
                            }
                        }
                        for z in &w[j + 1..] {
                            if z.tag.is_none() {
                                set_dflt(&mut dflt, &z.value)?;
                            }
                        }
                        let e = w[j].value.clone();
                        return interp.eval(&e, env);
                    }
                } else {
                    set_dflt(&mut dflt, &y.value)?;
                }
                i += 1;
            }
            if let Some(d) = dflt {
                return interp.eval(&d, env);
            }
        } else {
            let argval = as_integer(&x);
            if argval != NA_INTEGER && argval >= 1 && (argval as usize) <= w.len() {
                let alt = w[argval as usize - 1].value.clone();
                if alt.is_missing_arg() {
                    return Err(Flow::error("empty alternative in numeric switch"));
                }
                return interp.eval(&alt, env);
            }
        }
    } else {
        interp.warningcall(call, "'switch' with no alternatives")?;
    }
    interp.visible = false;
    Ok(Value::Null)
}

// ----------------------------------------------------------------------
// S3

fn get_object(interp: &mut Interp, idx: usize) -> RResult<Value> {
    let ctx = &interp.frames[idx];
    let Value::Closure(clo) = &ctx.function else {
        return Err(Flow::error("generic 'function' is not a function"));
    };
    let promargs = ctx.promargs.clone();
    let tag = clo.formals.first().and_then(|f| f.tag);
    let mut s_val: Option<Value> = None;
    if let Some(tag) = tag.filter(|t| *t != s::DOTS) {
        let tname = tag.name();
        for a in promargs.iter() {
            if a.tag == Some(tag) {
                if s_val.is_some() {
                    return Err(Flow::error(format!(
                        "formal argument \"{}\" matched by multiple actual arguments",
                        tname
                    )));
                }
                s_val = Some(a.value.clone());
            }
        }
        if s_val.is_none() {
            for a in promargs.iter() {
                if let Some(t) = a.tag {
                    if tname.starts_with(&*t.name()) && !t.name().is_empty() {
                        if s_val.is_some() {
                            return Err(Flow::error(format!(
                                "formal argument \"{}\" matched by multiple actual arguments",
                                tname
                            )));
                        }
                        s_val = Some(a.value.clone());
                    }
                }
            }
        }
        if s_val.is_none() {
            s_val = promargs.iter().find(|a| a.tag.is_none()).map(|a| a.value.clone());
        }
    }
    let v = s_val.or_else(|| promargs.first().map(|a| a.value.clone())).unwrap_or(Value::Null);
    let base = interp.base.clone();
    match v {
        Value::Promise(p) => interp.force_promise(&p),
        v if v.is_missing_arg() => Ok(Value::Null),
        v => interp.eval(&v, &base),
    }
}

fn do_usemethod(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let m = match_args(args, &["generic", "object"], call)?;
    let Some(gexpr) = m.vals[0].clone() else {
        return Err(Flow::error_call(call.clone(), "there must be a 'generic' argument"));
    };
    let generic = interp.eval(&gexpr, env)?;
    let generic = match &generic {
        Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => v.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "'generic' argument must be a character string")),
    };
    let Some(idx) = interp.frame_of_env(env) else {
        return Err(Flow::error_call(call.clone(), "'UseMethod' used in an inappropriate fashion"));
    };
    if interp.current_frame() != Some(idx) {
        return Err(Flow::error_call(call.clone(), "'UseMethod' used in an inappropriate fashion"));
    }
    let callenv = interp.frames[idx].sysparent.clone();
    let defenv = interp.topenv(env);
    let obj = match &m.vals[1] {
        Some(o) => interp.eval(o, env)?,
        None => get_object(interp, idx)?,
    };
    let gcall = interp.frames[idx].call.clone();
    let promargs = interp.frames[idx].promargs.clone();
    match interp.usemethod(&generic, &obj, &gcall, promargs, env, &callenv, &defenv)? {
        Some(ans) => Err(Flow::Return(ans, env.clone())),
        None => {
            let klass = crate::dispatch::dispatch_class(&obj);
            let cl = if klass.len() == 1 {
                klass[0].to_string()
            } else {
                format!("c('{}')", klass.iter().map(|k| k.to_string()).collect::<Vec<_>>().join("', '"))
            };
            Err(Flow::error_call(
                call.clone(),
                format!("no applicable method for '{}' applied to an object of class \"{}\"", generic, cl),
            ))
        }
    }
}

/// `NextMethod(generic, object, ...)`, called through
/// `.Internal(NextMethod(generic, object, ...))`.
fn do_nextmethod(interp: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    // The context of the method that called NextMethod: the frame whose
    // environment is the sysparent of NextMethod's closure frame.
    let Some(nm_frame) = interp.current_frame() else {
        return Err(Flow::error("NextMethod called from outside a method dispatch"));
    };
    let method_env = interp.frames[nm_frame].sysparent.clone();
    let Some(cptr) = interp.frame_of_env(&method_env) else {
        return Err(Flow::error("NextMethod called from outside a method dispatch"));
    };
    let mcall = interp.frames[cptr].call.clone();
    let mfun = interp.frames[cptr].function.clone();
    let mpromargs = interp.frames[cptr].promargs.clone();
    let method_sysparent = interp.frames[cptr].sysparent.clone();
    let Value::Closure(mclo) = &mfun else {
        return Err(Flow::error("function is not a closure"));
    };
    let get = |e: &Env, sym: Sym| e.get_local(sym);
    // .Generic
    let generic_arg = args.first().map(|a| a.value.clone()).unwrap_or(Value::Null);
    let generic: Rc<str> = match get(&method_env, s::DOT_GENERIC) {
        Some(v) => match crate::coerce::as_string(&interp.force(v)?) {
            Some(g) => g,
            None => return Err(Flow::error("generic function not specified")),
        },
        None => match crate::coerce::as_string(&generic_arg) {
            Some(g) => g,
            None => return Err(Flow::error("generic function not specified")),
        },
    };
    let generic: Rc<str> =
        if !generic_arg.is_null() { crate::coerce::as_string(&generic_arg).unwrap_or(generic) } else { generic };
    // .Class
    let klass: Vec<Rc<str>> = match get(&method_env, s::DOT_CLASS) {
        Some(v) => match interp.force(v)? {
            Value::Str(c) => c.data.iter().flatten().cloned().collect(),
            Value::Null => vec![],
            _ => vec![],
        },
        None => {
            let obj = get_object(interp, cptr)?;
            crate::dispatch::dispatch_class(&obj)
        }
    };
    let group: Rc<str> = match get(&method_env, s::DOT_GROUP) {
        Some(v) => crate::coerce::as_string(&interp.force(v)?).unwrap_or_else(|| Rc::from("")),
        None => Rc::from(""),
    };
    let basename: Rc<str> = if group.is_empty() { generic.clone() } else { group.clone() };
    // The current method name: .Method, or the call's function name.
    let method = get(&method_env, s::DOT_METHOD).map(|v| interp.force(v)).transpose()?;
    let cur_method: String = match &method {
        Some(Value::Str(m)) => {
            m.data.iter().flatten().find(|x| !x.is_empty()).map(|x| x.to_string()).unwrap_or_default()
        }
        Some(_) => return Err(Flow::error("wrong value for .Method")),
        None => match &mcall {
            Value::Lang(l) => l.fun_sym().map(|s| s.name().to_string()).unwrap_or_default(),
            _ => String::new(),
        },
    };
    // Find where we are in the class list.
    let mut j = klass.len();
    for (i, c) in klass.iter().enumerate() {
        if cur_method == format!("{}.{}", basename, c) || cur_method == format!("{}.{}", generic, c) {
            j = i;
            break;
        }
    }
    let callenv = match get(&method_env, s::DOT_GENERIC_CALL_ENV) {
        Some(Value::Env(e)) => e,
        _ => method_sysparent.clone(),
    };
    let defenv = match get(&method_env, s::DOT_GENERIC_DEF_ENV) {
        Some(Value::Env(e)) => e,
        _ => interp.base.clone(),
    };
    // Look for the next method.
    let mut found: Option<(Value, Sym, usize)> = None;
    let start = if j < klass.len() { j + 1 } else { 0 };
    for (i, c) in klass.iter().enumerate().skip(start) {
        let m1 = Sym::dotted(&generic, c);
        if let Some(f) = interp.lookup_method(m1, &callenv, &defenv)? {
            found = Some((f, m1, i));
            break;
        }
        if !group.is_empty() {
            let m2 = Sym::new(&format!("{}.{}", group, c));
            if let Some(f) = interp.lookup_method(m2, &callenv, &defenv)? {
                found = Some((f, m2, i));
                break;
            }
        }
    }
    if found.is_none() {
        let m1 = Sym::dotted(&generic, "default");
        if let Some(f) = interp.lookup_method(m1, &callenv, &defenv)? {
            found = Some((f, m1, klass.len()));
        }
    }
    let (nextfun, method_sym, next_i) = match found {
        Some(x) => x,
        None => {
            // Internal default: the primitive itself.
            let gname = generic.to_string();
            match crate::builtins::lookup_primitive(&gname) {
                Some(b) => (Value::Builtin(b), Sym::new(&gname), klass.len()),
                None => {
                    // A closure generic in base (e.g. print) has no default.
                    match interp.base.get_local(Sym::new(&gname)) {
                        Some(f @ Value::Closure(_)) => (f, Sym::new(&gname), klass.len()),
                        _ => return Err(Flow::error(format!("no more methods for '{}'", generic))),
                    }
                }
            }
        }
    };
    // Arguments: the method's promargs, with formals rebound to promises
    // for their current values in the method frame.
    let mut newargs: Vec<Arg> = Vec::new();
    let actuals = interp.match_args(&mclo.formals, &mpromargs, &mcall)?;
    let mut used_dots = false;
    for (f, a) in mclo.formals.iter().zip(actuals) {
        let tag = f.tag.unwrap();
        if tag == s::DOTS {
            if let Some(Value::Env(_)) = None::<Value> {}
            match method_env.get_local(s::DOTS) {
                Some(Value::Dots(d)) => newargs.extend(d.iter().cloned()),
                _ => {}
            }
            used_dots = true;
            continue;
        }
        match a {
            Some(v) if !v.is_missing_arg() => {
                // Was the formal supplied by name in the original call?
                let named = mpromargs.iter().any(|p| p.tag == Some(tag));
                let p = Promise::new(Value::Sym(tag), method_env.clone());
                let _ = v;
                newargs.push(Arg::new(
                    if named || newargs.len() > 0 && used_dots { Some(tag) } else { None },
                    Value::Promise(Rc::new(p)),
                ));
            }
            _ => {}
        }
    }
    // Extra arguments to NextMethod (after generic, object) override.
    for a in args.iter().skip(2) {
        if let Some(t) = a.tag {
            if let Some(pos) = newargs.iter().position(|x| x.tag == Some(t)) {
                newargs[pos] = a.clone();
                continue;
            }
        }
        newargs.push(a.clone());
    }
    let dot_class = if next_i < klass.len() {
        let mut v = Value::strs(klass[next_i..].iter().map(|c| Some(c.clone())).collect());
        v.set_attr_raw(s::PREVIOUS, Value::strs(klass.iter().map(|c| Some(c.clone())).collect()));
        v
    } else {
        Value::strs(vec![])
    };
    let newcall = match &mcall {
        Value::Lang(l) => {
            let mut nl = (**l).clone();
            nl.fun = Value::Sym(method_sym);
            Value::Lang(Rc::new(nl))
        }
        other => other.clone(),
    };
    // R: objects.c:do_nextmethod. Ops methods retain one entry per
    // operand, updating only entries that named the previous method.
    let next_method = match method {
        Some(Value::Str(mut m)) => {
            for name in &mut Rc::make_mut(&mut m).data {
                if name.as_deref() != Some("") {
                    *name = Some(method_sym.name());
                }
            }
            Value::Str(m)
        }
        _ => Value::str(&method_sym.name()),
    };
    let _ = call;
    let vars = vec![
        (s::DOT_GENERIC, Value::str(&generic)),
        (s::DOT_CLASS, dot_class),
        (s::DOT_METHOD, next_method),
        (s::DOT_GENERIC_CALL_ENV, Value::Env(callenv.clone())),
        (s::DOT_GENERIC_DEF_ENV, Value::Env(defenv.clone())),
        (s::DOT_GROUP, Value::str(&group)),
    ];
    match &nextfun {
        Value::Closure(c) => {
            // The next method is called from the method's caller.
            let saved = interp.frames[cptr].generic;
            interp.frames[cptr].generic = true;
            let last = interp.frames.len() - 1;
            let saved_last = interp.frames[last].generic;
            interp.frames[last].generic = false;
            let r = interp.apply_closure_with_sysparent(&newcall, c, newargs, &method_sysparent, Some(vars));
            interp.frames[cptr].generic = saved;
            interp.frames[last].generic = saved_last;
            r
        }
        Value::Builtin(_) => {
            // Strip the class we are dispatching on? R calls the primitive
            // with the object unchanged; the primitive must not re-dispatch.
            let mut forced = Vec::new();
            for a in newargs {
                let v = interp.force(a.value)?;
                forced.push(Arg::new(a.tag, v));
            }
            let dcall = match &newcall {
                Value::Lang(l) => {
                    let mut nl = (**l).clone();
                    nl.fun = Value::sym(&format!("{}.default", generic));
                    Value::Lang(Rc::new(nl))
                }
                other => other.clone(),
            };
            interp.apply_function(&nextfun, forced, &method_sysparent, &dcall)
        }
        _ => Err(Flow::error("invalid next method")),
    }
}

impl Interp {
    pub fn apply_closure_with_sysparent(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        sysparent: &Env,
        vars: Option<Vec<(Sym, Value)>>,
    ) -> RResult<Value> {
        let actuals = self.match_args(&clo.formals, &promargs, call)?;
        let newenv = Env::with_capacity(Some(clo.env.clone()), clo.formals.len());
        for (k, (f, a)) in clo.formals.iter().zip(actuals).enumerate() {
            let tag = f.tag.unwrap();
            match a {
                Some(v) if !v.is_missing_arg() => newenv.bind_arg(tag, v, 0),
                _ => {
                    if !f.value.is_missing_arg() && tag != s::DOTS {
                        newenv.bind_arg(tag, crate::interp::default_arg(clo, k, &newenv), BIND_MISSING_DEFAULT);
                    } else {
                        newenv.bind_arg(tag, Value::missing_arg(), 0);
                    }
                }
            }
        }
        if let Some(vars) = vars {
            for (k, v) in vars {
                newenv.set_local(k, v);
            }
        }
        self.exec_closure(call, clo, promargs, newenv, sysparent.clone())
    }
}

fn do_standard_generic(_: &mut Interp, call: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Err(Flow::error_call(call.clone(), "S4 dispatch (standardGeneric) is not supported yet"))
}

fn do_recall(interp: &mut Interp, _call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // The function that called Recall's closure.
    let Some(i) = interp.current_frame() else { return Err(Flow::error("'Recall' called from outside a closure")) };
    let parent_env = interp.frames[i].sysparent.clone();
    let Some(j) = interp.frame_of_env(&parent_env) else {
        return Err(Flow::error("'Recall' called from outside a closure"));
    };
    let f = interp.frames[j].function.clone();
    let call = interp.frames[j].call.clone();
    let _ = env;
    interp.apply_function(&f, args, &parent_env, &call)
}

// ----------------------------------------------------------------------
// .Internal, .Primitive, misc

fn do_internal(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let inner = &args[0].value;
    let Value::Lang(l) = inner else {
        return Err(Flow::error_call(call.clone(), "invalid .Internal() argument"));
    };
    let Some(fsym) = l.fun_sym() else {
        return Err(Flow::error_call(call.clone(), "invalid .Internal() argument"));
    };
    let name = fsym.name();
    let Some(b) = super::lookup_internal(&name) else {
        return Err(Flow::error_call(call.clone(), format!("there is no .Internal function '{}'", name)));
    };
    interp.visible = true;
    interp.call_builtin(b, inner, &l.args, env)
}

fn do_primitive(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match args.first().map(|a| &a.value) {
        Some(Value::Str(v)) if v.data.len() == 1 && v.data[0].is_some() => v.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "string argument required")),
    };
    match super::lookup_primitive(&name) {
        Some(b) => Ok(Value::Builtin(b)),
        None => Err(Flow::error_call(call.clone(), format!("no such primitive function"))),
    }
}

fn do_nargs(interp: &mut Interp, _: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    let n = interp.frame_of_env(env).map(|i| interp.frames[i].promargs.len()).unwrap_or(0);
    Ok(Value::int(n as i32))
}

fn do_interactive(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(false))
}

fn do_invisible(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(if args.is_empty() { Value::Null } else { args.swap_remove(0).value })
}

fn do_force_and_call(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let n = as_integer(&interp.eval(&args[0].value, env)?);
    let fexpr = args[1].value.clone();
    let rest: Vec<Arg> = args[2..].to_vec();
    let f = match &fexpr {
        Value::Sym(sym) => interp.find_fun(*sym, env)?,
        other => interp.eval(other, env)?,
    };
    let newcall = Value::lang(fexpr, rest.clone());
    match &f {
        Value::Closure(c) => {
            let pargs = interp.promise_args(&rest, env)?;
            for a in pargs.iter().take(n.max(0) as usize) {
                if let Value::Promise(p) = &a.value {
                    interp.force_promise(p)?;
                }
            }
            interp.apply_closure(&newcall, c, pargs, env, None)
        }
        Value::Builtin(b) => interp.call_builtin(b, &newcall, &rest, env),
        _ => Err(Flow::error_call(call.clone(), "attempt to apply non-function")),
    }
}

fn dots_of(env: &Env) -> RResult<Vec<Arg>> {
    match env.lookup(s::DOTS) {
        Some((Value::Dots(d), _)) => Ok((*d).clone()),
        Some((v, _)) if v.is_missing_arg() => Ok(vec![]),
        _ => Err(Flow::error("incorrect context: the current call has no '...' to look in")),
    }
}

fn do_dots_length(_: &mut Interp, _: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    Ok(Value::int(dots_of(env)?.len() as i32))
}

fn do_dots_names(_: &mut Interp, _: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    let d = dots_of(env)?;
    if d.iter().all(|a| a.tag.is_none()) {
        return Ok(Value::Null);
    }
    Ok(Value::strs(d.iter().map(|a| Some(a.tag.map(|t| t.name()).unwrap_or_else(|| Rc::from("")))).collect()))
}

fn do_dots_elt(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let n = as_integer(&interp.eval(&args[0].value, env)?);
    let d = dots_of(env)?;
    if n == NA_INTEGER || n < 1 {
        return Err(Flow::error_call(call.clone(), "indexing '...' with non-positive index"));
    }
    if n as usize > d.len() {
        return Err(Flow::error_call(
            call.clone(),
            format!("the ... list contains fewer than {} element{}", n, if n == 1 { "" } else { "s" }),
        ));
    }
    interp.visible = true;
    let v = d[n as usize - 1].value.clone();
    interp.eval(&v, env)
}

/// `~`: the call itself, as a formula remembering its environment.
// R: names.c:do_tilde
fn do_tilde(_: &mut Interp, call: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    let mut f = call.clone();
    if !call.is_object() {
        f.set_attr_raw(s::CLASS, Value::str("formula"));
        f.set_attr_raw(Sym::new(".Environment"), Value::Env(env.clone()));
    }
    Ok(f)
}
