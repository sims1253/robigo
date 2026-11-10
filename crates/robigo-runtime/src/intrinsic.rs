//! Base R closures answered without running their R code, where doing so
//! is unobservable.
//!
//! Base R's argument checking is R code that runs on every call of the
//! functions using it: `match.arg(method)` alone looks up the caller's
//! formals, evaluates the default `c("a", "b", ...)` and partially matches
//! the argument against it, about forty operations and five closure calls.
//! A call site marked at compile time (`compile::CallSite::intrinsic`)
//! computes the same value here when the callee is base's own closure and
//! nothing on the way could run R code or fail. Anything else (an argument
//! still to be computed, a default that is not a plain `c()` of strings, a
//! value that would not match, a traced function, a rebound `c`) is the
//! ordinary call, which then does exactly what it always did.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::compile::GuardMemo;
use crate::env::{BIND_ACTIVE, Env};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

/// What a marked call site needs at run time.
pub struct Site {
    /// The function's name as called (`match.arg`).
    fun: Sym,
    /// The symbol passed as `arg`.
    arg: Sym,
    /// `c`.
    c: Sym,
    /// `c`, looked up from the caller's frame, is base's.
    c_memo: Cell<GuardMemo>,
    /// The closure last found to be base's (address), and the search epoch
    /// then: rebinding it in base changes the epoch.
    base_memo: Cell<(u64, usize)>,
    /// The choices of the last formals seen (kept alive, so the address
    /// cannot be reused): the value of their default `c(...)`.
    choices: RefCell<Option<(Rc<[Arg]>, Value)>>,
}

/// The intrinsic for the call `l` to the function named `f`, if any.
pub fn site(f: Sym, l: &Lang) -> Option<Box<Site>> {
    if !f.with_name(|n| n == "match.arg") || l.args.len() != 1 {
        return None;
    }
    let a = &l.args[0];
    if a.tag.is_some_and(|t| !t.with_name(|n| n == "arg")) {
        return None;
    }
    let Value::Sym(arg) = a.value else { return None };
    if arg == s::DOTS || arg.with_name(|n| n.starts_with("..")) {
        return None;
    }
    Some(Box::new(Site {
        fun: f,
        arg,
        c: Sym::new("c"),
        c_memo: Cell::new(GuardMemo {
            epoch: u64::MAX,
            env: 0,
            version: 0,
            search: 0,
            ok: false,
            outer: 0,
            outer_version: 0,
        }),
        base_memo: Cell::new((u64::MAX, 0)),
        choices: RefCell::new(None),
    }))
}

/// Whether `clo` is the closure base binds to `sym`.
fn is_base_closure(interp: &Interp, sym: Sym, clo: &Rc<Closure>) -> bool {
    let frame = interp.base.0.frame().borrow();
    let Some(b) = frame.get(sym) else { return false };
    match &b.value {
        Value::Closure(c) => Rc::ptr_eq(c, clo),
        // A lazily loaded base binding is a (forced) promise.
        Value::Promise(p) => matches!(&*p.value.borrow(), PromiseVal::Boxed(Value::Closure(c)) if Rc::ptr_eq(c, clo)),
        _ => false,
    }
}

/// The closures match.arg calls on the way (`formals(sys.function(...))`,
/// `eval(..., envir = sys.frame(...))`, `identical`, `pmatch`): base's
/// plain, untraced closures, so that skipping their calls is unobservable.
/// (`trace()` marks or rebinds them, which changes the search epoch, so
/// this is checked again then.)
fn helpers_plain(interp: &Interp) -> bool {
    const HELPERS: [&str; 7] = ["formals", "sys.function", "sys.parent", "sys.frame", "eval", "identical", "pmatch"];
    let frame = interp.base.0.frame().borrow();
    HELPERS.iter().all(|h| {
        let Some(b) = frame.get(Sym::new(h)) else { return false };
        let f = match &b.value {
            Value::Promise(p) => match &*p.value.borrow() {
                PromiseVal::Boxed(v) => v.clone(),
                _ => return false,
            },
            v => v.clone(),
        };
        matches!(&f, Value::Closure(c) if !c.jit.traced() && c.attrs.is_none())
    })
}

/// The strings of a default `c("a", "b", ...)`: untagged, ASCII, not NA.
fn plain_choices(e: &Value) -> Option<Value> {
    let Value::Lang(l) = e else { return None };
    if !l.fun_sym()?.with_name(|n| n == "c") || l.args.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(l.args.len());
    for a in &l.args {
        let Value::Str(v) = &a.value else { return None };
        if a.tag.is_some() || v.attrs.is_some() || v.data.len() != 1 {
            return None;
        }
        let s = v.data[0].as_ref()?;
        if !s.is_ascii() {
            return None;
        }
        out.push(Some(s.clone()));
    }
    Some(Value::strs(out))
}

fn one(s: &RString) -> Value {
    Value::strs(vec![Some(s.clone())])
}

/// `match.arg(arg)` (choices missing, several.ok FALSE) called from `env`.
// R: base/R/match.R:match.arg
pub(crate) fn match_arg(interp: &mut Interp, site: &Site, clo: &Rc<Closure>, env: &Env) -> Option<Value> {
    if clo.jit.traced() || crate::builtins::debug::any_prim_traced() {
        return None;
    }
    let key = (crate::env::search_epoch(), Rc::as_ptr(clo) as usize);
    if site.base_memo.get() != key {
        if !is_base_closure(interp, site.fun, clo) || !helpers_plain(interp) {
            return None;
        }
        // (The memo holds no reference: an address seen at this epoch is
        // still base's binding, which keeps the closure alive.)
        site.base_memo.set(key);
    }
    // The real call nests a few closure calls; near the limit it is the one
    // to report "evaluation nested too deeply".
    if interp.depth + 8 >= interp.expressions_limit() || env.ptr_eq(&interp.global) {
        return None;
    }
    // `formals(sys.function(sysP <- sys.parent()))`: the function of the
    // outermost function frame of `env` (R: R_sysparent, R_sysfunction).
    let k = interp.frames.iter().position(|c| !c.builtin && c.cloenv.ptr_eq(env))?;
    let Value::Closure(g) = &interp.frames[k].function else { return None };
    let g = g.clone();
    // `formal.args[[as.character(substitute(arg))]]`, then `eval()` of it in
    // `env`: a `c()` of strings with base's `c`.
    let formal = g.formals.iter().find(|f| f.tag == Some(site.arg))?;
    let choices = {
        let mut memo = site.choices.borrow_mut();
        match &*memo {
            Some((formals, v)) if Rc::ptr_eq(formals, &g.formals) => v.clone(),
            _ => {
                let v = plain_choices(&formal.value)?;
                *memo = Some((g.formals.clone(), v.clone()));
                v
            }
        }
    };
    if !crate::vm::guard_ok(interp, site.c, env, &site.c_memo) {
        return None;
    }
    // `arg`: the caller's binding, read without running code, or its
    // default `c(...)` forced (which runs none either).
    let bound = {
        let frame = env.0.frame().borrow();
        let b = frame.get(site.arg)?;
        if b.flags & BIND_ACTIVE != 0 {
            return None;
        }
        b.value.clone()
    };
    let arg = match bound {
        Value::Promise(p) => {
            let forced = match &*p.value.borrow() {
                PromiseVal::Boxed(v) => Some(v.clone()),
                PromiseVal::Unforced => None,
                _ => return None,
            };
            match forced {
                Some(v) => v,
                None => {
                    let own_default =
                        p.expr.ptr_eq(&formal.value) && p.env.borrow().as_ref().is_some_and(|e| e.ptr_eq(env));
                    if !own_default || p.forcing.get() || p.interrupted.get() {
                        return None;
                    }
                    // Forced as `force_promise` would: its value is the
                    // `c()` of the strings just read (base's `c`, untraced).
                    p.set_value(choices.clone());
                    *p.env.borrow_mut() = None;
                    p.note_c_forced();
                    crate::env::bump_r_epoch();
                    choices.clone()
                }
            }
        }
        v if v.is_missing_arg() => return None,
        v => v,
    };
    let Value::Str(ch) = &choices else { return None };
    // `if (is.null(arg)) return(choices[1L])`
    if arg.is_null() {
        return Some(one(ch.data[0].as_ref()?));
    }
    let Value::Str(a) = &arg else { return None };
    if a.attrs.is_some() {
        return None;
    }
    // `if (identical(arg, choices)) return(arg[1L])`
    if a.data.len() == ch.data.len() && a.data.iter().zip(&ch.data).all(|(x, y)| x == y) {
        return Some(one(a.data[0].as_ref()?));
    }
    if a.data.len() != 1 {
        return None;
    }
    // `pmatch(arg, choices, nomatch = 0L, duplicates.ok = TRUE)`: an exact
    // match, else the only partial one; no match is the call's error.
    let x = a.data[0].as_ref()?;
    if x.is_empty() || !x.is_ascii() {
        return None;
    }
    let names = || ch.data.iter().map(|c| c.as_deref().unwrap_or(""));
    let i = match names().position(|c| c == &**x) {
        Some(i) => i,
        None => {
            let mut partial = names().enumerate().filter(|(_, c)| c.starts_with(&**x));
            let (i, _) = partial.next()?;
            if partial.next().is_some() {
                return None;
            }
            i
        }
    };
    Some(one(ch.data[i].as_ref()?))
}
