//! Language primitives: control flow, functions, assignment, quoting,
//! `missing`, `on.exit`, `switch`, `UseMethod`, `.Internal` (GNU R's
//! eval.c, builtin.c and parts of envir.c).

use std::rc::Rc;

use super::{check1arg, internal, match_args, prim, special};
use crate::coerce::{as_condition, as_integer};
use crate::env::{BIND_ACTIVE, BIND_MISSING_DEFAULT, Env};
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    special("~", do_tilde, -1),
    special("if", do_if, -1).custom_vis(),
    special("for", do_for, 3).invisible().arity_checked(),
    special("while", do_while, 2).invisible().arity_checked(),
    special("repeat", do_repeat, 1).invisible().arity_checked(),
    special("break", do_break, 0),
    special("next", do_next, 0),
    special("return", do_return, -1),
    special("function", do_function, -1),
    special("<-", do_assign, -1).invisible(),
    special("=", do_assign, -1).invisible(),
    special("<<-", do_super_assign, -1).invisible(),
    special("{", do_begin, -1).custom_vis(),
    prim("(", do_paren, 1).arity_checked(),
    special("quote", do_quote, 1).arg1("expr"),
    special("substitute", do_substitute, -1),
    special("missing", do_missing, 1).arg1("x"),
    special("on.exit", do_onexit, -1).invisible(),
    special("switch", do_switch, -1).custom_vis(),
    special("UseMethod", do_usemethod, -1).custom_vis(),
    special(".Internal", do_internal, 1).custom_vis().arity_checked(),
    prim(".Primitive", do_primitive, 1).arity_checked(),
    prim("nargs", do_nargs, 0).arity_checked(),
    prim("interactive", do_interactive, 0).arity_checked(),
    prim("invisible", do_invisible, -1).invisible(),
    special("forceAndCall", do_force_and_call, -1).custom_vis(),
    prim("...length", do_dots_length, 0).arity_checked(),
    prim("...elt", do_dots_elt, 1).custom_vis().arg1("n"),
    prim("...names", do_dots_names, 0).arity_checked(),
    internal("NextMethod", do_nextmethod, -1).custom_vis(),
    internal("Recall", do_recall, -1).custom_vis(),
];

fn do_if(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let c = interp.eval(&args[0].value, env)?;
    if as_condition(interp, &c, call)? {
        if stepping(interp, env) && !has_braces(&args[1].value) {
            debug_step(interp, &args[1].value, env)?;
        }
        interp.eval(&args[1].value, env)
    } else if args.len() > 2 {
        if stepping(interp, env) && !has_braces(&args[2].value) {
            debug_step(interp, &args[2].value, env)?;
        }
        interp.eval(&args[2].value, env)
    } else {
        interp.visible = false;
        Ok(Value::Null)
    }
}

/// `RDEBUG(rho) && !R_GlobalContext->browserfinish`: does a statement of
/// the frame `env` stop at the browser first?
#[inline(always)]
fn stepping(interp: &Interp, env: &Env) -> bool {
    env.rdebug() && !interp.browser_finish
}

/// R's `CTXT_LOOP` context as far as the browser's `f` is concerned: the
/// loop begins with its parent's `browserfinish` and gives it back when it
/// ends, however it ends.
// R: context.c:begincontext, endcontext
#[inline(always)]
fn loop_context<T>(interp: &mut Interp, f: impl FnOnce(&mut Interp) -> RResult<T>) -> RResult<T> {
    let saved = interp.browser_finish;
    interp.loop_depth += 1;
    let r = f(interp);
    interp.loop_depth -= 1;
    interp.browser_finish = saved;
    r
}

/// R: eval.c:BodyHasBraces.
fn has_braces(body: &Value) -> bool {
    matches!(body, Value::Lang(l) if l.fun.as_sym() == Some(s::BRACE))
}

/// The browser before a statement of a frame being debugged: the
/// statement's srcref and the statement, then `do_browser`.
// R: eval.c:do_begin, do_if, do_for, do_while
#[cold]
#[inline(never)]
fn debug_step(interp: &mut Interp, stmt: &Value, env: &Env) -> RResult<()> {
    debug_prompt(interp);
    let global = interp.global.clone();
    interp.print_value_toplevel(stmt, &global)?;
    crate::builtins::debug::browse(interp, env, Value::str(""), Value::Null, 0)?;
    Ok(())
}

/// `SrcrefPrompt("debug", R_Srcref)`.
fn debug_prompt(interp: &mut Interp) {
    let srcref = match &interp.srcref {
        Some(v) => Some(Value::Int(v.clone())),
        // (the closure's own srcref, or none at top level)
        None => interp.frames.iter().rev().find(|f| !f.builtin).and_then(|f| f.function.get_attr(s::SRCREF)),
    };
    crate::builtins::debug::srcref_prompt(interp, "debug", srcref.as_ref());
}

/// The `(while)` browser after a `while` body of a frame being debugged.
// R: eval.c:do_while
#[cold]
#[inline(never)]
fn debug_while(interp: &mut Interp, cond: &Value, env: &Env) -> RResult<()> {
    debug_prompt(interp);
    interp.write_out("(while) ");
    let global = interp.global.clone();
    interp.print_value_toplevel(cond, &global)?;
    crate::builtins::debug::browse(interp, env, Value::str(""), Value::Null, 0)?;
    Ok(())
}

/// How one pass through a loop's `CTXT_LOOP` context ended.
#[derive(PartialEq)]
enum LoopFlow {
    Done,
    Next,
    Break,
}

/// R's `CTXT_LOOP` context: `break`/`next` aimed at this frame end the pass.
#[inline(always)]
fn loop_flow<T>(r: RResult<T>, env: &Env) -> RResult<LoopFlow> {
    match r {
        Ok(_) => Ok(LoopFlow::Done),
        Err(Flow::Break(e)) if e.ptr_eq(env) => Ok(LoopFlow::Break),
        Err(Flow::Next(e)) if e.ptr_eq(env) => Ok(LoopFlow::Next),
        Err(f) => Err(f),
    }
}

/// Run a loop body inside the loop's context. With `step` (the body has no
/// braces) a frame being debugged first browses at the body; that browser
/// runs inside the context too, so `break`/`next` typed at its prompt
/// leave or advance the loop (R: eval.c:do_for, do_while).
fn loop_body(interp: &mut Interp, body: &Value, env: &Env, step: bool) -> RResult<LoopFlow> {
    // R: eval.c:eval's interrupt check, which every iteration's eval of the
    // body passes
    if crate::autogc::due() {
        crate::autogc::collect(interp);
    }
    if step && stepping(interp, env) {
        return loop_flow(debug_step(interp, body, env).and_then(|()| interp.eval(body, env)), env);
    }
    loop_flow(interp.eval(body, env), env)
}

/// Run a loop through the bytecode VM when enabled and the loop head is
/// the base function.
fn try_compiled_loop(interp: &mut Interp, call: &Value, head: Sym, env: &Env) -> Option<RResult<Value>> {
    // (R: a frame being debugged runs its loops interpreted)
    if !crate::vm::enabled() || env.rdebug() || !interp.resolves_to_base(head, env) {
        return None;
    }
    let code = crate::vm::code_for(call, false)?;
    Some(crate::vm::run(interp, &code, env))
}

/// R: eval.c:do_for, do_while, do_repeat — at JIT level 3 a loop the AST
/// interpreter meets in the global environment is compiled and run by
/// bcEval (R_compileAndExecute), so its call sites are bytecode ones.
/// (Everywhere else a loop is part of the code around it.)
fn loop_bc<T>(
    interp: &mut Interp,
    head: Sym,
    call: &Value,
    env: &Env,
    f: impl FnOnce(&mut Interp) -> RResult<T>,
) -> RResult<T> {
    if interp.bc || !env.ptr_eq(&interp.global) || !interp.resolves_to_base(head, env) {
        return f(interp);
    }
    interp.bc = true;
    // A loop run by `eval()` inside a running one is compiled on its own;
    // the outer loop's code keeps the decision it was compiled with. (The
    // run count is not restored: the outer loop's memos, made before the
    // inner run, resolve again against that decision.)
    let outer_si = interp.toplevel_si;
    interp.toplevel_loop_run += 1;
    crate::vm::snapshot_toplevel_si(interp, call);
    let r = f(interp);
    interp.bc = false;
    interp.toplevel_si = outer_si;
    r
}

fn do_for(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    loop_bc(interp, s::FOR, call, env, |interp| for_loop(interp, call, args, env))
}

fn for_loop(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = try_compiled_loop(interp, call, s::FOR, env) {
        return r;
    }
    // R: eval.c:do_for: the frame's debugging state comes back after the loop.
    let dbg = env.rdebug();
    let r = loop_context(interp, |interp| for_loop_ast(interp, call, args, env))?;
    env.set_rdebug(dbg);
    Ok(r)
}

fn for_loop_ast(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let Some(sym) = args[0].value.as_sym() else {
        return Err(Flow::error_call(call.clone(), "non-symbol loop variable"));
    };
    let val = interp.eval(&args[1].value, env)?;
    let mut val = crate::altrep::forced(interp, val)?;
    if val.inherits("factor") {
        val = crate::builtins::envir::as_character_factor(&val)?;
    }
    let body = args[2].value.clone();
    let bgn = has_braces(&body);
    let n = val.len();
    interp.define(sym, Value::Null, env)?;
    // R: eval.c:do_for reads a package ALTREP sequence through its class
    // (INTEGER_ELT / REAL_ELT / LOGICAL_ELT): checked once per loop
    if let Some(h) = crate::altrep::lookup(&val) {
        for i in 0..n {
            let v = crate::altrep::elt(interp, h, i)?;
            interp.define(sym, v, env)?;
            if loop_body(interp, &body, env, !bgn)? == LoopFlow::Break {
                break;
            }
        }
        interp.visible = false;
        return Ok(Value::Null);
    }
    for i in 0..n {
        // Reuse the previous iteration's scalar if nothing else holds it
        // (R's ALLOC_LOOP_VAR).
        if reuse_loop_var(&val, i, sym, env) {
            if loop_body(interp, &body, env, !bgn)? == LoopFlow::Break {
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
        if loop_body(interp, &body, env, !bgn)? == LoopFlow::Break {
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
    loop_bc(interp, s::WHILE, call, env, |interp| while_loop(interp, call, args, env))
}

fn while_loop(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = try_compiled_loop(interp, call, s::WHILE, env) {
        return r;
    }
    let dbg = env.rdebug();
    let bgn = has_braces(&args[1].value);
    loop_context(interp, |interp| {
        loop {
            let c = interp.eval(&args[0].value, env)?;
            if !as_condition(interp, &c, call)? {
                break;
            }
            // (R: a `next` jumps back to the condition, past the "(while)"
            // prompt)
            match loop_body(interp, &args[1].value, env, !bgn)? {
                LoopFlow::Break => break,
                LoopFlow::Next => continue,
                LoopFlow::Done => {}
            }
            if stepping(interp, env) && loop_flow(debug_while(interp, &args[0].value, env), env)? == LoopFlow::Break {
                break;
            }
        }
        Ok(())
    })?;
    env.set_rdebug(dbg);
    interp.visible = false;
    Ok(Value::Null)
}

fn do_repeat(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    loop_bc(interp, s::REPEAT, call, env, |interp| repeat_loop(interp, call, args, env))
}

fn repeat_loop(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = try_compiled_loop(interp, call, s::REPEAT, env) {
        return r;
    }
    let dbg = env.rdebug();
    loop_context(interp, |interp| {
        while loop_body(interp, &args[0].value, env, false)? != LoopFlow::Break {}
        Ok(())
    })?;
    env.set_rdebug(dbg);
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
    // R: context.c:findcontext — the catchable error is raised here, at
    // the jump site, so enclosing handlers see it before unwinding.
    if !interp.has_return_target(env) {
        return Err(Flow::error("no function to return from, jumping to top level"));
    }
    Err(Flow::Return(v, env.clone()))
}

thread_local! {
    /// The formals of each `function` expression evaluated, by the address
    /// of its formals pairlist (kept alive here): closures created by one
    /// expression share them (and the caches keyed by them).
    static FORMALS: std::cell::RefCell<std::collections::HashMap<usize, (Value, Rc<[Arg]>), crate::env::FastHash>> =
        Default::default();
}

/// R: eval.c:CheckFormals — reported with `error()`, against the context call.
fn bad_formals() -> Flow {
    Flow::error("invalid formal argument list for \"function\"")
}

fn do_function(interp: &mut Interp, _call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let formals: Rc<[Arg]> = match &args[0].value {
        Value::Pairlist(p) => {
            let key = Rc::as_ptr(p) as *const u8 as usize;
            match FORMALS.with(|c| c.borrow().get(&key).map(|(_, f)| f.clone())) {
                Some(f) => f,
                None => {
                    if p.items.iter().any(|a| a.tag.is_none()) {
                        return Err(bad_formals());
                    }
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
        _ => return Err(bad_formals()),
    };
    let body = args.get(1).map(|a| a.value.clone()).unwrap_or(Value::Null);
    let srcref = args.get(2).map(|a| &a.value).filter(|v| !v.is_null());
    Ok(make_closure(interp, formals, body, srcref, env))
}

/// The closure `do_function` makes (also compiled code's `MakeClosure`).
pub(crate) fn make_closure(
    interp: &mut Interp,
    formals: Rc<[Arg]>,
    body: Value,
    srcref: Option<&Value>,
    env: &Env,
) -> Value {
    let clo = Closure {
        #[cfg(feature = "bytecode-fidelity")]
        gnu_bytecode: None,
        formals,
        body,
        env: env.clone(),
        // R: eval.c:do_function -- CADDR(args) belongs to the closure,
        // independently of the source references attached to its body.
        attrs: srcref.map(|v| Box::new(Attrs(vec![(s::SRCREF, v.clone())], false, false))),
        code: Default::default(),
        inline: Default::default(),
        defaults: Default::default(),
        jit: Default::default(),
    };
    crate::vm::jit_inherit(interp, &clo, env);
    Value::Closure(Rc::new(clo))
}

// R: eval.c:do_begin — each value is a C local the collector does not see
// (only the last one is returned and protected), so a statement's value
// is dead once the next statement starts: drop it, else an object only
// it holds outlives a gc() in the next statement (R6's finalizer tests:
// `B <- BC$new()` then `expect_output({ rm(B); gc() })`).
fn do_begin(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    interp.visible = true;
    let Some((last, init)) = args.split_last() else { return Ok(Value::Null) };
    // R: eval.c:do_begin -- with source references, each statement runs
    // under its own srcref (the block's list starts with the block's), and
    // the srcref before the block comes back after it (eval.c:eval). Code
    // without them leaves R_Srcref alone.
    if let Some(refs) = call.get_attr(s::SRCREF).filter(|r| matches!(r, Value::List(_))) {
        let saved = interp.srcref.clone();
        let r = (|| {
            for (j, a) in init.iter().enumerate() {
                interp.srcref = crate::interp::statement_srcref(&refs, j + 1);
                if stepping(interp, env) {
                    debug_step(interp, &a.value, env)?;
                }
                interp.eval(&a.value, env)?;
            }
            interp.srcref = crate::interp::statement_srcref(&refs, args.len());
            if stepping(interp, env) {
                debug_step(interp, &last.value, env)?;
            }
            interp.eval(&last.value, env)
        })();
        interp.srcref = saved;
        return r;
    }
    for a in init {
        if stepping(interp, env) {
            debug_step(interp, &a.value, env)?;
        }
        interp.eval(&a.value, env)?;
    }
    if stepping(interp, env) {
        debug_step(interp, &last.value, env)?;
    }
    interp.eval(&last.value, env)
}

fn do_paren(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(args.pop().map(|a| a.value).unwrap_or(Value::Null))
}

fn do_quote(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(args[0].value.clone())
}

// ----------------------------------------------------------------------
// assignment

fn assign_target(lhs: &Value) -> RResult<Option<Sym>> {
    match lhs {
        Value::Sym(s) => Ok(Some(*s)),
        // R: eval.c:do_set — a string lhs is installed with installTrChar,
        // which rejects the empty name (plain error(), so the message is
        // reported against the context call).
        Value::Str(v) if v.data.len() == 1 => match v.data[0].as_deref() {
            Some(name) => Sym::install_tr(name).map(Some).map_err(Flow::error),
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

fn do_assign(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error_call(call.clone(), "invalid assignment"));
    }
    let lhs = &args[0].value;
    if let Some(sym) = assign_target(lhs)? {
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
    if let Some(sym) = assign_target(lhs)? {
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
    complex_assign_pre(interp, call, lhs, rhs_expr, rhs, env, superassign, None, None)
}

/// [`complex_assign`] for a single-level `f(x, i, ...) <- value` whose index
/// arguments `i, ...` were already evaluated (by compiled code) to `pre`:
/// they are not evaluated again, and calls in messages still show their
/// expressions. `rhs_call` supplies the source RHS for compiled builtin
/// diagnostics; interpreted assignments show the evaluated promise value.
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
    rhs_call: Option<&Value>,
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
    let r = complex_assign_inner(interp, call, &levels, target, rhs_expr, rhs, env, superassign, pre, rhs_call);
    // Errors raised with `error()` by the replacement functions report the
    // assignment call: signal them while its context is still active.
    let r = match r {
        Err(Flow::Error(e)) if !e.signaled => {
            // R: errors.c:R_vmakeErrorCondition resolves R_CurrentExpression
            // for handlers while R_signalErrorConditionEx retains the marker
            // for default output. Signal while the assignment is active.
            if e.call.as_ref().and_then(Value::as_sym) == Some(Sym::new("<current-expression>")) {
                Err(interp.signal_error(e))
            } else if e.call.is_none() {
                Err(interp.signal_error(e))
            } else {
                Err(Flow::Error(e))
            }
        }
        r => r,
    };
    interp.ccode_calls.pop();
    r
}

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
    rhs_call: Option<&Value>,
) -> RResult<()> {
    // In-place path for nested assignments over list containers
    // (`a$x[[1]] <- v`): R mutates the element without copying. It runs
    // before any value is acquired (it takes the binding itself and
    // restores it on every fallback), so no other path pays for it.
    if levels.len() >= 2 && !superassign && !env.binding_flags(target).is_some_and(|f| f & BIND_ACTIVE != 0) {
        if let Some(r) = complex_assign_inplace(interp, levels, target, rhs.clone(), env, pre) {
            return r;
        }
    }

    // R: eval.c:EnsureLocal evaluates the symbol in the enclosing
    // environment. Preserve the error signaled there, including its call.
    let base_val = if superassign {
        let parent = env.parent().unwrap_or_else(|| interp.global.clone());
        interp.eval(&Value::Sym(target), &parent)?
    } else if env.has_local(target) {
        interp.eval(&Value::Sym(target), env)?
    } else {
        let parent = env.parent().unwrap_or_else(|| interp.global.clone());
        let v = interp.eval(&Value::Sym(target), &parent)?;
        interp.define(target, v.clone(), env)?;
        v
    };

    // Fast path: `x[i] <- v`, `names(x) <- v`, ... with a builtin
    // replacement function that takes the object (Builtin::in_place) on a
    // local value: modify in place. (Not for active bindings: their
    // binding holds the function.)
    let active = env.binding_flags(target).is_some_and(|f| f & BIND_ACTIVE != 0);
    if levels.len() == 1 && !superassign && !active {
        let l = &levels[0];
        let fname = assign_fun(&l.fun)?;
        if let Value::Sym(fsym) = &fname {
            if let Ok(Value::Builtin(b)) = interp.find_fun(*fsym, env) {
                if b.takes_in_place(&base_val) {
                    return fast_replace(interp, b, *fsym, l, target, base_val, rhs, env, pre, rhs_call);
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

/// R: eval.c:applydefine — GNU R hands a replacement the element of a
/// uniquely-held container without copying (the replacement path's
/// `lazy_duplicate` only ENSURE_NAMEDMAXes; duplicate.c:171), so
/// `a$x[[1]] <- v` mutates `a$x` in place. The general path below keeps the
/// container's reference alive, which forces `Rc::make_mut` to duplicate
/// the element. This path takes the target's binding out and walks the
/// getter chain (`$`/`[[` builtins over list containers, slot found by
/// allocation identity) taking each element out of its uniquely-held
/// container; the innermost replacement then runs in place and the chain is
/// healed back into the binding. Returns `None` when a shape is not
/// supported, leaving the binding untouched for the general path.
#[allow(clippy::too_many_arguments)]
fn complex_assign_inplace(
    interp: &mut Interp,
    levels: &[Rc<Lang>],
    target: Sym,
    rhs: Value,
    env: &Env,
    pre: Option<&[Value]>,
) -> Option<RResult<()>> {
    let n = levels.len();
    if n < 2 || !env.has_local(target) {
        return None;
    }
    // Every getter level must be a plain base `$`/`[[` call; the innermost
    // setter a builtin replacement (evaluated arguments).
    for k in 1..n {
        let l = &levels[n - k];
        let Value::Sym(f) = &l.fun else { return None };
        if *f != s::DOLLAR && *f != s::BRACKET2 {
            return None;
        }
    }
    // The innermost level's head is the *getter* (`[[`, `[`, `$`, ...);
    // its replacement function is what runs on the element.
    let setter_sym = match assign_fun(&levels[0].fun) {
        Ok(Value::Sym(f)) => f,
        _ => return None,
    };
    let setter: &'static Builtin = match interp.find_fun(setter_sym, env) {
        Ok(Value::Builtin(bb)) if bb.in_place != InPlace::No => bb,
        _ => return None,
    };
    // Take the binding: the sole robigo reference to the container (other
    // variables aliasing it share the allocation; the spine copy that
    // `Rc::make_mut` then makes matches GNU R's `$<-` duplicate). Restore
    // it on every exit.
    let mut root = match env.take_local(target) {
        Some(Value::Promise(p)) => match p.value() {
            Some(v) => v,
            // An unforced promise (a function argument not yet touched):
            // put the binding back exactly as it was and let the general
            // path force it. (Taking it out and returning here left NULL
            // bound, so the general path then assigned into NULL and the
            // object's attributes vanished.)
            None => {
                // force_set keeps the binding's flags (a default
                // argument's BIND_MISSING_DEFAULT, which missing()
                // reads): set_local would reset them to 0
                env.force_set(target, Value::Promise(p));
                return None;
            }
        },
        Some(v) => v,
        None => return None,
    };
    let restore = |env: &Env, v: Value| {
        env.force_set(target, v);
    };
    if root.is_object() || !matches!(root, Value::List(_) | Value::Expr(_)) {
        restore(env, root);
        return None;
    }
    // Walk the getters, taking every element out of its (now uniquely
    // held) container. `slots[k]` is containers[k+1]'s slot in
    // containers[k]. Every exit after a take-out heals the chain first:
    // a fallback that left holes would drop data.
    let mut containers: Vec<Value> = vec![std::mem::replace(&mut root, Value::Null)];
    let mut slots: Vec<usize> = Vec::new();
    /// Put every taken element back into its slot and the binding back;
    /// `err` distinguishes a fallback (None) from a propagated error.
    fn leave(
        env: &Env,
        target: Sym,
        containers: &mut Vec<Value>,
        slots: &[usize],
        err: Option<Flow>,
    ) -> Option<RResult<()>> {
        for j in (1..containers.len()).rev() {
            let v = containers[j].clone();
            match &mut containers[j - 1] {
                Value::List(rc) | Value::Expr(rc) => Rc::make_mut(rc).data[slots[j - 1]] = v,
                _ => unreachable!("complex_assign_inplace containers are lists"),
            }
        }
        env.set_local(target, containers.remove(0));
        err.map(Err)
    }
    /// [`leave`] for an error, unless R code rebound or removed the target.
    fn leave_after(
        env: &Env,
        target: Sym,
        containers: &mut Vec<Value>,
        slots: &[usize],
        err: Flow,
        rebound: bool,
    ) -> Option<RResult<()>> {
        if rebound {
            return Some(Err(err));
        }
        leave(env, target, containers, slots, Some(err))
    }
    // R code in an argument rebound or removed the target: on an error,
    // that binding stays (applydefine assigns only on success).
    let mut rebound = false;
    for k in 1..n {
        let l = levels[n - k].clone();
        let is_dollar = matches!(&l.fun, Value::Sym(f) if *f == s::DOLLAR);
        let extra: Vec<Arg> = l.args.iter().skip(1).cloned().collect();
        let mut argv = vec![Arg::pos(containers.last().unwrap().clone())];
        if is_dollar {
            // `$` is a special: its name argument stays unevaluated.
            argv.extend(extra.iter().cloned());
        } else {
            // R: eval.c:evalseq evaluates the getter's arguments while the
            // target is bound (its setter's evaluation follows the
            // replacement, below).
            let (r, kept) = bound_while(interp, env, target, rebound, &extra, &mut containers, &slots, |i| {
                i.eval_args(&extra, env, &Value::Null)
            });
            rebound |= !kept;
            match r {
                Ok(mut ev) => argv.append(&mut ev),
                Err(f) => return leave_after(env, target, &mut containers, &slots, f, rebound),
            }
        }
        let Value::Builtin(b) = interp.find_fun(if is_dollar { s::DOLLAR } else { s::BRACKET2 }, env).ok()? else {
            unreachable!("checked above");
        };
        let gcall = Value::lang(l.fun.clone(), argv.clone());
        let got = match interp.invoke_builtin(b, &gcall, argv, env) {
            Ok(v) => v,
            Err(f) => return leave_after(env, target, &mut containers, &slots, f, rebound),
        };
        // The getter's call holds the container too: let it go, or the
        // take-out below would copy the container's spine and leave the
        // element shared with the old one.
        drop(gcall);
        // An object would dispatch the next getter, or the setter unless
        // it never dispatches. The setter's other arguments are evaluated
        // after the take-out, where R code would not see the element:
        // only a setter without them takes an object here.
        if got.is_object() && !(k == n - 1 && levels[0].args.len() == 1 && setter.takes_in_place(&got)) {
            return leave(env, target, &mut containers, &slots, None);
        }
        // Find the slot by allocation identity and take the element out of
        // the container (whose only other holders are aliases outside our
        // control; a spine copy here is GNU R's own `$<-` duplicate).
        let container = containers.last_mut().unwrap();
        let slot = match container {
            Value::List(rc) | Value::Expr(rc) => {
                let vec = Rc::make_mut(rc);
                vec.data.iter().position(|e| e.same_object(&got))
            }
            _ => None,
        };
        let Some(i) = slot else {
            return leave(env, target, &mut containers, &slots, None);
        };
        let element = match containers.last_mut().unwrap() {
            Value::List(rc) | Value::Expr(rc) => std::mem::replace(&mut Rc::make_mut(rc).data[i], Value::Null),
            _ => unreachable!(),
        };
        slots.push(i);
        containers.push(element);
    }
    // Only a uniquely-held innermost element can be modified in place;
    // otherwise the general path's copy is GNU R's, too. An element
    // without an allocation (NULL, a symbol) is never taken: see
    // Builtin::takes_in_place.
    let last = containers.last().unwrap();
    if !sole_allocation(last) || last.alloc_addr() == 0 {
        return leave(env, target, &mut containers, &slots, None);
    }
    // The setter's argument conventions, as in fast_replace: builtin
    // replacements take evaluated arguments; specials take them
    // unevaluated (`$<-`'s name as-is, others evaluated here and quoted,
    // the object and the value quoted). R code they run sees the target.
    let l0 = levels[0].clone();
    let extra: Vec<Arg> = l0.args.iter().skip(1).cloned().collect();
    let mut argv = Vec::with_capacity(extra.len() + 2);
    argv.push(Arg::pos(Value::Null));
    let r = if let Some(vals) = pre {
        for (a, v) in extra.iter().zip(vals.iter()) {
            let v = if setter.kind == BuiltinKind::Special { quote_if_needed(v.clone()) } else { v.clone() };
            argv.push(Arg::new(a.tag, v));
        }
        Ok(())
    } else if setter.kind == BuiltinKind::Special && setter.name == "$<-" {
        argv.extend(extra.iter().cloned());
        Ok(())
    } else {
        let (r, kept) = bound_while(interp, env, target, rebound, &extra, &mut containers, &slots, |i| {
            if setter.kind == BuiltinKind::Special {
                crate::builtins::subset::eval_keep_missing(i, &extra, env)
            } else {
                i.eval_args(&extra, env, &Value::Null)
            }
        });
        rebound |= !kept;
        r.map(|ev| {
            for e in ev {
                let v = match e.value {
                    v if setter.kind != BuiltinKind::Special || v.is_missing_arg() => v,
                    v => quote_if_needed(v),
                };
                argv.push(Arg::new(e.tag, v));
            }
        })
    };
    if let Err(f) = r {
        return leave_after(env, target, &mut containers, &slots, f, rebound);
    }
    // The innermost replacement runs on the (uniquely held) element.
    let element = containers.pop().unwrap();
    let element_addr = element.alloc_addr();
    argv[0].value = element;
    if setter.kind == BuiltinKind::Special {
        argv[0].value = quote_if_needed(argv[0].value.clone());
        argv.push(Arg::new(Some(s::VALUE), quote_if_needed(rhs)));
    } else {
        argv.push(Arg::new(Some(s::VALUE), rhs));
    }
    // The call, for error messages, names the object `*tmp*` as R's does:
    // holding the element itself would make it shared, and the
    // replacement would copy it.
    let mut cargs = vec![Arg::pos(Value::Sym(s::TMP))];
    cargs.extend(argv[1..].iter().cloned());
    let scall = Value::lang(Value::Sym(setter_sym), cargs);
    // As in fast_replace: the setter hands the element back on error.
    // The window shows R code the setter runs the element in its place.
    use crate::builtins::subassign::{Chain, Taken, in_window};
    let chain = Rc::new(std::cell::RefCell::new(Chain {
        containers: std::mem::take(&mut containers),
        slots: std::mem::take(&mut slots),
    }));
    let taken = crate::builtins::subassign::watched(interp, &argv).then(|| Taken {
        env: env.clone(),
        sym: target,
        addr: element_addr,
        hidden: !rebound,
        chain: Some(chain.clone()),
    });
    let (r, window_rebound) = in_window(interp, taken, |i| i.invoke_builtin(setter, &scall, argv, env));
    let Ok(chain) = Rc::try_unwrap(chain) else { unreachable!("the closed window released the chain") };
    Chain { containers, slots } = chain.into_inner();
    let new_element = match r {
        Ok(v) => v,
        // R code the setter ran rebound or removed the target: R's
        // applydefine assigns only on success, so that binding stays.
        Err(f) if rebound || window_rebound => {
            drop(crate::builtins::subassign::take_restore().expect(HANDED_BACK));
            return Some(Err(f));
        }
        Err(f) => {
            // The replacement owned the element; subassign hands it back
            // unchanged on error, so put it back where it came from.
            let back = crate::builtins::subassign::take_restore().expect(HANDED_BACK);
            return_into_slot(&mut containers, &slots, back);
            return leave(env, target, &mut containers, &slots, Some(f));
        }
    };
    return_into_slot(&mut containers, &slots, new_element);
    rebound |= window_rebound;
    // R: eval.c:applydefine then calls each enclosing level's setter, from
    // the innermost out, evaluating the getter's arguments again: R code
    // in them sees the target as the replacement left it. (A replacement
    // that allocated a new object is seen too, where GNU R's binding still
    // holds the old one: keeping that alive would cost every in-place
    // replacement a copy.) The setters themselves are the slots filled
    // above; the assignment follows them, whatever R code bound.
    let held = containers.len() - 1;
    for l in &levels[1..] {
        let extra = &l.args[1..];
        if matches!(&l.fun, Value::Sym(f) if *f == s::DOLLAR) {
            continue;
        }
        let (r, kept) = bound_while(interp, env, target, rebound, extra, &mut containers, &slots[..held], |i| {
            i.eval_args(extra, env, &Value::Null)
        });
        rebound |= !kept;
        if let Err(f) = r {
            return leave_after(env, target, &mut containers, &slots, f, rebound);
        }
    }
    leave(env, target, &mut containers, &slots, None);
    Some(Ok(()))
}

/// Every replacement a complex assignment hands its object to
/// (Builtin::in_place) hands it back on every error; losing it would bind
/// NULL in its place.
const HANDED_BACK: &str = "an in-place replacement hands its object back when it fails";

/// Run `f`, which evaluates `args` of a level of a complex assignment,
/// with the elements taken out so far back in their slots and the target
/// bound to its container, as GNU R's binding holds it then
/// (eval.c:applydefine evaluates them while the target is bound): R code
/// in them reads the target. Then take everything out again. Returns
/// whether the target's binding is still the one bound here; if R code
/// rebound or removed it, that binding stays, as it does once it is
/// `rebound` already. Arguments that run no R code (constants, local
/// variables already evaluated) skip all of it.
#[allow(clippy::too_many_arguments)]
fn bound_while<T>(
    interp: &mut Interp,
    env: &Env,
    target: Sym,
    rebound: bool,
    args: &[Arg],
    containers: &mut [Value],
    slots: &[usize],
    f: impl FnOnce(&mut Interp) -> T,
) -> (T, bool) {
    if rebound {
        return (f(interp), false);
    }
    if args.iter().all(|a| runs_no_code(&a.value, target, env)) {
        return (f(interp), true);
    }
    for j in (1..containers.len()).rev() {
        let v = containers[j].clone();
        set_slot(&mut containers[j - 1], slots[j - 1], v);
    }
    let addr = containers[0].alloc_addr();
    env.force_set(target, containers[0].clone());
    let r = f(interp);
    let kept = match env.take_local(target) {
        Some(v) if v.alloc_addr() == addr => true,
        Some(v) => {
            env.force_set(target, v);
            false
        }
        None => false,
    };
    for (c, &k) in containers.iter_mut().zip(slots) {
        set_slot(c, k, Value::Null);
    }
    (r, kept)
}

/// Whether evaluating `v` (an argument of a complex assignment's level)
/// certainly runs no R code and reads no `target`: a constant, an empty
/// argument, or a local variable other than the target holding a value
/// (no promise to force, no active binding, no `...`).
fn runs_no_code(v: &Value, target: Sym, env: &Env) -> bool {
    match v {
        Value::Lang(_) | Value::Promise(_) | Value::Dots(_) => false,
        v if v.is_missing_arg() => true,
        Value::Sym(s) => {
            *s != target
                && env.binding_value(*s).is_some_and(|(v, flags)| {
                    flags & crate::env::BIND_ACTIVE == 0
                        && !v.is_missing_arg()
                        && !matches!(&v, Value::Dots(_))
                        && !matches!(&v, Value::Promise(p) if p.value().is_none())
                })
        }
        _ => true,
    }
}

fn set_slot(container: &mut Value, k: usize, v: Value) {
    match container {
        Value::List(rc) | Value::Expr(rc) => Rc::make_mut(rc).data[k] = v,
        _ => unreachable!("complex_assign_inplace containers are lists"),
    }
}

/// The element the innermost replacement received (by `pop`, its sole
/// owner) is back as its result: fill the slot it was taken from, in the
/// last remaining container.
fn return_into_slot(containers: &mut [Value], slots: &[usize], v: Value) {
    let slot = *slots.last().expect("a take-out happened");
    match containers.last_mut().expect("the root container") {
        Value::List(rc) | Value::Expr(rc) => Rc::make_mut(rc).data[slot] = v,
        _ => unreachable!("complex_assign_inplace containers are lists"),
    }
}

/// Whether `v` is the only handle to its allocation (R's `MAYBE_SHARED`
/// being false: the replacement may modify it in place).
fn sole_allocation(v: &Value) -> bool {
    let count = match v {
        Value::Lgl(x) => Rc::strong_count(x),
        Value::Int(x) => Rc::strong_count(x),
        Value::Real(x) => Rc::strong_count(x),
        Value::Cplx(x) => Rc::strong_count(x),
        Value::Str(x) => Rc::strong_count(x),
        Value::Raw(x) => Rc::strong_count(x),
        Value::List(x) => Rc::strong_count(x),
        Value::Expr(x) => Rc::strong_count(x),
        Value::Pairlist(x) => Rc::strong_count(x),
        Value::Lang(x) => Rc::strong_count(x),
        Value::Closure(x) => Rc::strong_count(x),
        Value::Promise(x) => Rc::strong_count(x),
        Value::Dots(x) => Rc::strong_count(x),
        Value::ExtPtr(x) => Rc::strong_count(x),
        Value::Obj(x) => Rc::strong_count(x),
        _ => return true,
    };
    count == 1
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
    obj: Value,
    rhs: Value,
    env: &Env,
    pre: Option<&[Value]>,
    rhs_call: Option<&Value>,
) -> RResult<()> {
    // The call as R would construct it, for error messages:
    // `f(*tmp*, <extra>, value = <rhs>)`. Its middle holds the extra (index)
    // arguments, which are evaluated while `target` is still bound.
    let mut cargs = Vec::with_capacity(l.args.len() + 1);
    cargs.push(Arg::pos(Value::Sym(s::TMP)));
    cargs.extend(l.args.iter().skip(1).cloned());
    let extra = &cargs[1..];
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
            for e in crate::builtins::subset::eval_keep_missing(interp, extra, env)? {
                let v = if e.value.is_missing_arg() { e.value } else { quote_if_needed(e.value) };
                argv.push(Arg::new(e.tag, v));
            }
        }
    } else {
        let ev = interp.eval_args(extra, env, &Value::Null)?;
        argv.extend(ev);
    }
    // R: eval.c:SETTER_CALL uses the compiler's source call, whereas
    // applydefine/replaceCall deparses the forced RHS promise's value.
    let rhs_shown = rhs_call.unwrap_or(&rhs).clone();
    argv.push(Arg::new(Some(s::VALUE), if b.kind == BuiltinKind::Special { quote_if_needed(rhs) } else { rhs }));
    // R: eval.c:evalseq saves the LHS before evaluating setter indices.
    // Release the binding's reference after index evaluation, retaining that
    // saved object even when an index assigned a different value to target.
    // An unchanged binding goes before Rc::make_mut: retaining it would copy
    // the entire vector on every replacement.
    let hidden = match env.take_local(target) {
        Some(v)
            if match &v {
                Value::Promise(p) => p.value().is_some_and(|v| v.same_object(&obj)),
                v => v.same_object(&obj),
            } =>
        {
            true
        }
        // If evaluation of an index changed or removed the binding, R code
        // sees that binding, which stays on error: applydefine assigns
        // only on success.
        Some(v) => {
            env.force_set(target, v);
            false
        }
        None => false,
    };
    // R: eval.c:EnsureLocal — a value referenced elsewhere too (another
    // variable, `.Last.value`) is duplicated before the setter runs, so
    // that the setter modifies, and R code it runs sees, the target's own
    // copy. The setter's first Rc::make_mut would make the same copy.
    let mut obj = obj;
    let addr = crate::builtins::debug::make_unique_reported(interp, &mut obj);
    argv[0].value = if b.kind == BuiltinKind::Special { quote_if_needed(obj) } else { obj };
    let taken = crate::builtins::subassign::watched(interp, &argv).then(|| crate::builtins::subassign::Taken {
        env: env.clone(),
        sym: target,
        addr,
        hidden,
        chain: None,
    });
    cargs.push(Arg::new(Some(s::VALUE), rhs_shown));
    let rcall = Value::lang(Value::Sym(fsym), cargs);
    // The replacement owns the binding's object; if it fails, it hands
    // the object back through the restore channel, which the window
    // empties first (the index arguments ran user code) and keeps open
    // for the call alone. (Specials evaluate their arguments: theirs are
    // quoted.)
    let (r, rebound) = crate::builtins::subassign::in_window(interp, taken, |i| i.invoke_builtin(b, &rcall, argv, env));
    match r {
        Ok(v) => interp.define(target, v, env),
        Err(f) => {
            // The replacement function owned the object and handed it back
            // when it failed (Builtin::in_place). A binding R code left
            // (Taken::hidden) stays.
            let orig = crate::builtins::subassign::take_restore().expect(HANDED_BACK);
            if hidden && !rebound {
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
        Value::Sym(_) | Value::Lang(_) | Value::Promise(_) | Value::Bytecode(_) => Value::call_sym("quote", vec![v]),
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
            let (items, lang_cells) = substitute_list(&l.items(), env)?;
            // Expanding dots at the head inserts LISTSXP nodes. It does
            // not turn the first dot argument into the function of a call.
            let dots_head = l.fun.as_sym() == Some(s::DOTS) && env.and_then(|e| e.get_local(s::DOTS)).is_some();
            if items.is_empty() {
                Value::Null
            } else if dots_head {
                Value::pairlist(items)
            } else {
                let l = Rc::new(Lang::from_items(items, l.attrs.clone()));
                if !lang_cells.is_empty() {
                    crate::serialize::set_lang_cells(
                        &l,
                        lang_cells.into_iter().filter_map(|k| k.checked_sub(1)).collect(),
                    );
                }
                Value::Lang(l)
            }
        }
        _ => e.clone(),
    })
}

/// The substituted items, and indices of exceptional LANGSXP cells: an
/// unbound `...` becomes `LCONS(R_DotsSymbol, R_NilValue)` (no tag).
// R: coerce.c:substituteList
fn substitute_list(items: &[Arg], env: Option<&Env>) -> RResult<(Vec<Arg>, Vec<usize>)> {
    let mut out = Vec::with_capacity(items.len());
    let mut lang_cells = Vec::new();
    for a in items {
        if a.value.as_sym() == Some(s::DOTS) {
            match env.and_then(|e| e.get_local(s::DOTS)) {
                Some(Value::Dots(d)) => {
                    for item in d.iter() {
                        out.push(Arg::new(item.tag, substitute(&item.value, None)?));
                    }
                }
                Some(v) if v.is_missing_arg() || v.is_null() => {}
                None => {
                    lang_cells.push(out.len());
                    out.push(Arg::new(None, a.value.clone()));
                }
                _ => return Err(Flow::error("'...' used in an incorrect context")),
            }
        } else {
            out.push(Arg::new(a.tag, substitute(&a.value, env)?));
        }
    }
    Ok((out, lang_cells))
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
    let (items, _) = substitute_list(&[Arg::pos(expr)], senv.as_ref())?;
    Ok(items.into_iter().next().map(|a| a.value).unwrap_or(Value::Null))
}

pub fn is_missing_in(_interp: &Interp, sym: Sym, env: &Env, _depth: usize) -> bool {
    // R: envir.c:R_isMissing follows unforced symbol promises and uses
    // PRSEEN to recognize cycles. Track their identities without mutating them.
    let mut sym = sym;
    let mut env = env.clone();
    let mut seen = Vec::new();
    loop {
        if sym.is_empty() {
            return true;
        }
        let (lookup, ddv) = match sym.ddval() {
            Some(i) => (s::DOTS, Some(i)),
            None => (sym, None),
        };
        let Some(mut v) = env.get_local(lookup) else { return false };
        if let Some(i) = ddv {
            match &v {
                Value::Dots(d) if d.len() >= i => v = d[i - 1].value.clone(),
                _ => return true,
            }
        }
        // MISSING == 2 (a default argument) is not missing here.
        if v.is_missing_arg() {
            return true;
        }
        let Value::Promise(mut p) = v else { return false };
        while let Value::Promise(q) = &p.expr {
            p = q.clone();
        }
        if p.value().is_some() {
            return false;
        }
        let Value::Sym(next) = p.expr else { return false };
        let Some(penv) = p.env.borrow().clone() else { return false };
        let id = Rc::as_ptr(&p);
        if seen.contains(&id) {
            return true;
        }
        seen.push(id);
        sym = next;
        env = penv;
    }
}

fn do_missing(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
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
            // R: builtin.c:do_onexit — `on.exit(NULL)` clears, as no code does.
            None => ctx.on_exit.clear(),
            Some(Value::Null) if !add => ctx.on_exit.clear(),
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
    check1arg(&args, call, "EXPR")?;
    let x = interp.eval(&args[0].value, env)?;
    let x = crate::altrep::forced(interp, x)?;
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
            let key: Option<crate::value::RString> = Some(sv.data[0].clone().unwrap_or_else(|| "NA".into()));
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
    let generic_val = interp.eval(&gexpr, env)?;
    let generic = match &generic_val {
        Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => v.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "'generic' argument must be a character string")),
    };
    // `.Generic` is `mkString(generic)`: the argument itself when it is
    // exactly that string.
    let generic_val = (!generic_val.has_attrs()).then_some(generic_val);
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
    match interp.usemethod_ex(&generic, generic_val.as_ref(), &obj, &gcall, promargs, env, &callenv, &defenv)? {
        Some(ans) => Err(Flow::Return(ans, env.clone())),
        None => {
            // R: objects.c:do_usemethod — R_data_class2 (S4 superclasses).
            let klass = interp.data_class2(&obj)?;
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
fn do_nextmethod(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
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
    let generic: crate::value::RString = match get(&method_env, s::DOT_GENERIC) {
        Some(v) => match crate::coerce::as_string(&interp.force(v)?) {
            Some(g) => g,
            None => return Err(Flow::error("generic function not specified")),
        },
        None => match crate::coerce::as_string(&generic_arg) {
            Some(g) => g,
            None => return Err(Flow::error("generic function not specified")),
        },
    };
    let generic: crate::value::RString =
        if !generic_arg.is_null() { crate::coerce::as_string(&generic_arg).unwrap_or(generic) } else { generic };
    // .Class
    // R: objects.c:do_nextmethod. Kept as the value itself: the next
    // method's .Class is a suffix of it with it as attribute "previous".
    let klass_val: Value = match get(&method_env, s::DOT_CLASS) {
        Some(v) => interp.force(v)?,
        None => {
            let obj = get_object(interp, cptr)?;
            crate::dispatch::dispatch_class_value(&obj)
        }
    };
    let klass: &[crate::value::RStr] = match &klass_val {
        Value::Str(c) => &c.data,
        _ => &[],
    };
    let group: crate::value::RString = match get(&method_env, s::DOT_GROUP) {
        Some(v) => crate::coerce::as_string(&interp.force(v)?).unwrap_or_else(|| crate::value::RString::from("")),
        None => crate::value::RString::from(""),
    };
    let basename: crate::value::RString = if group.is_empty() { generic.clone() } else { group.clone() };
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
    let is_sig = |base: &str, c: &str| {
        cur_method.len() == base.len() + 1 + c.len()
            && cur_method.starts_with(base)
            && cur_method.as_bytes()[base.len()] == b'.'
            && cur_method.ends_with(c)
    };
    for (i, c) in klass.iter().enumerate() {
        let c = crate::dispatch::class_elt(c);
        if is_sig(&basename, c) || is_sig(&generic, c) {
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
    let mut scope = crate::dispatch::LookupScope::new();
    for (i, c) in klass.iter().enumerate().skip(start) {
        let c = crate::dispatch::class_elt(c);
        let m1 = Sym::dotted(&generic, c);
        if let Some(f) = interp.lookup_method_in(m1, &callenv, &defenv, &mut scope)? {
            found = Some((f, m1, i));
            break;
        }
        if !group.is_empty() {
            let m2 = Sym::dotted(&group, c);
            if let Some(f) = interp.lookup_method_in(m2, &callenv, &defenv, &mut scope)? {
                found = Some((f, m2, i));
                break;
            }
        }
    }
    if found.is_none() {
        let m1 = Sym::dotted(&generic, "default");
        if let Some(f) = interp.lookup_method_in(m1, &callenv, &defenv, &mut scope)? {
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
                    match interp.base_binding(Sym::new(&gname)) {
                        Some(f @ Value::Closure(_)) => (f, Sym::new(&gname), klass.len()),
                        _ => return Err(Flow::error(format!("no more methods for '{}'", generic))),
                    }
                }
            }
        }
    };
    // R: match.c:patchArgsByActuals / patchArgument. Patch the supplied
    // slots in place, retaining their order, tags, and explicit missing args.
    let mut newargs = mpromargs.clone();
    let mut formal_state = vec![0u8; mclo.formals.len()];
    let mut used = vec![false; newargs.len()];
    let patch = |arg: &mut Arg, tag: Sym| {
        let missing = arg.value.is_missing_arg();
        if missing && method_env.get_local(tag).is_some_and(|v| v.is_missing_arg()) {
            return 2;
        }
        arg.value = Value::Promise(Rc::new(Promise::new(Value::Sym(tag), method_env.clone())));
        if missing { 3 } else { 1 }
    };
    for (fi, f) in mclo.formals.iter().enumerate() {
        let tag = f.tag.unwrap();
        if tag == s::DOTS {
            continue;
        }
        if let Some(bi) = newargs.iter().position(|a| a.tag == Some(tag)) {
            formal_state[fi] = patch(&mut newargs[bi], tag);
            used[bi] = true;
        }
    }
    let mut seen_dots = false;
    for (fi, f) in mclo.formals.iter().enumerate() {
        if formal_state[fi] != 0 {
            continue;
        }
        let tag = f.tag.unwrap();
        if tag == s::DOTS && !seen_dots {
            seen_dots = true;
            continue;
        }
        if let Some(bi) = newargs.iter().enumerate().position(|(bi, a)| {
            !used[bi]
                && a.tag.is_some_and(|t| {
                    if seen_dots { t == tag } else { !t.name().is_empty() && tag.name().starts_with(&*t.name()) }
                })
        }) {
            formal_state[fi] = patch(&mut newargs[bi], tag);
            used[bi] = true;
        }
    }
    let (mut fi, mut bi) = (0, 0);
    while fi < mclo.formals.len() && bi < newargs.len() {
        let tag = mclo.formals[fi].tag.unwrap();
        if tag == s::DOTS {
            break;
        }
        if formal_state[fi] == 1 {
            fi += 1;
        } else if used[bi] || newargs[bi].tag.is_some() {
            bi += 1;
        } else {
            if formal_state[fi] == 3 {
                newargs[bi].value = Value::missing_arg();
            } else {
                patch(&mut newargs[bi], tag);
            }
            used[bi] = true;
            fi += 1;
            bi += 1;
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
    // R: stringSuffix(klass, i) with setAttrib(s, R_PreviousSymbol, klass).
    let mut dot_class = Value::strs(klass[next_i..].to_vec());
    if matches!(klass_val, Value::Str(_)) {
        dot_class.set_attr_raw(s::PREVIOUS, klass_val.clone());
    }
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
            // R: objects.c:do_nextmethod applies it with rho = env
            interp.step_into(c, env);
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

fn do_recall(interp: &mut Interp, _call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // The function that called Recall's closure.
    let Some(i) = interp.current_frame() else { return Err(Flow::error("'Recall' called from outside a closure")) };
    // R: eval.c:do_recall takes `cptr->promargs` for the enclosing call, so
    // the arguments of `Recall(...)` stay promises of the original call's
    // expressions (e.g. a missing argument in a pairlist is never forced).
    let promargs = match env.lookup(s::DOTS) {
        Some((Value::Dots(d), _)) => d.as_ref().clone(),
        _ => args,
    };
    let parent_env = interp.frames[i].sysparent.clone();
    let Some(j) = interp.frame_of_env(&parent_env) else {
        return Err(Flow::error("'Recall' called from outside a closure"));
    };
    let f = interp.frames[j].function.clone();
    let call = interp.frames[j].call.clone();
    interp.apply_function(&f, promargs, &parent_env, &call)
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
    // R: names.c:do_internal -- arguments are evaluated by evalList with the
    // *outer* `.Internal(...)` call (so `argument N is empty` reports that
    // call), then the primitive runs with the inner call `s`.
    let argv = match b.kind {
        BuiltinKind::Special => l.args.to_vec(),
        BuiltinKind::Builtin => interp.eval_args(&l.args, env, call)?,
    };
    interp.invoke_builtin(b, inner, argv, env)
}

fn do_primitive(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match args.first().map(|a| &a.value) {
        Some(Value::Str(v)) if v.data.len() == 1 && v.data[0].is_some() => v.data[0].clone().unwrap(),
        _ => return Err(Flow::error_call(call.clone(), "string argument required")),
    };
    match super::lookup_primitive(&name) {
        Some(b) => Ok(Value::Builtin(b)),
        None => Err(Flow::error_call(call.clone(), "no such primitive function".to_string())),
    }
}

fn do_nargs(interp: &mut Interp, _: &Value, _: Vec<Arg>, env: &Env) -> RResult<Value> {
    let n = interp.frame_of_env(env).map(|i| interp.frames[i].promargs.len() as i32).unwrap_or(i32::MIN);
    Ok(Value::int(n))
}

fn do_interactive(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(i.interactive))
}

fn do_invisible(_: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: print.c:do_invisible — no arguments give NULL, one is checked
    // against `x`, more fail checkArity (names.c's arity is 1).
    match args.len() {
        0 => Ok(Value::Null),
        1 => {
            check1arg(&args, call, "x")?;
            Ok(args.swap_remove(0).value)
        }
        n => Err(Flow::error_call(call.clone(), format!("{n} arguments passed to 'invisible' which requires 1"))),
    }
}

fn do_force_and_call(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // R: eval.c:do_forceAndCall — CAR/CDR of a short list are NULL, so a
    // missing function is the NULL call's "attempt to apply non-function".
    let n = match args.first() {
        Some(a) => as_integer(&interp.eval(&a.value, env)?),
        None => NA_INTEGER,
    };
    let fexpr = args.get(1).map_or(Value::Null, |a| a.value.clone());
    let rest: Vec<Arg> = args.get(2..).unwrap_or_default().to_vec();
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
    Ok(Value::strs(
        d.iter().map(|a| Some(a.tag.map(|t| t.name()).unwrap_or_else(|| crate::value::RString::from("")))).collect(),
    ))
}

fn do_dots_elt(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // R: envir.c:do_dotsElt and ddfind (checkArity and check1arg: `.arg1("n")`).
    let index = &args[0].value;
    if !index.is_numeric_type() || (matches!(index, Value::Int(_)) && index.inherits("factor")) || index.len() != 1 {
        return Err(Flow::error_call(call.clone(), "indexing '...' with an invalid index"));
    }
    let n = as_integer(index);
    if n <= 0 {
        return Err(Flow::error(format!("indexing '...' with non-positive index {}", n)));
    }
    let d = match env.lookup(s::DOTS) {
        Some((Value::Dots(d), _)) => (*d).clone(),
        Some(_) => vec![],
        None => return Err(Flow::error(format!("..{} used in an incorrect context, no ... to look in", n))),
    };
    if n as usize > d.len() {
        return Err(Flow::error(format!(
            "the ... list contains fewer than {} element{}",
            n,
            if n == 1 { "" } else { "s" }
        )));
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

#[cfg(test)]
mod assignment_ownership_tests {
    use super::*;

    #[test]
    fn unchanged_replacement_targets_reuse_their_buffers() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        let env = interp.global.clone();
        let target = Sym::new("x");
        let cases = [
            (Value::Cplx(Rc::new(Vector::new(vec![Complex::new(0.0, 0.0); 256]))), "x[2] <- 1i"),
            (Value::Lgl(Rc::new(Vector::new(vec![0; 256]))), "x[2] <- TRUE"),
            (Value::Raw(Rc::new(Vector::new(vec![0; 256]))), "x[2] <- as.raw(1)"),
            (Value::Expr(Rc::new(Vector::new(vec![Value::Null; 256]))), "x[2] <- expression(a)"),
        ];
        for (value, source) in cases {
            let address = |v: &Value| match v {
                Value::Cplx(v) => Rc::as_ptr(v) as usize,
                Value::Lgl(v) => Rc::as_ptr(v) as usize,
                Value::Raw(v) => Rc::as_ptr(v) as usize,
                Value::Expr(v) => Rc::as_ptr(v) as usize,
                _ => panic!("unexpected replacement type"),
            };
            let original = address(&value);
            env.set_local(target, value);
            let expr = crate::lang::parse_to_values(source).unwrap().remove(0);
            for _ in 0..3 {
                assert!(interp.eval(&expr, &env).is_ok(), "{source}");
                assert_eq!(address(&env.get_local(target).unwrap()), original, "{source}");
            }
        }
    }
}
