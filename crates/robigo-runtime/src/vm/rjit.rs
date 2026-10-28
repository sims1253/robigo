//! When GNU R's JIT would have compiled a closure, and what that binds.
//!
//! GNU R (JIT level 3, the default) byte-compiles a closure on its first or
//! second call (`R_CheckJIT`). The compiler inlines the base "language
//! functions" (`+`, `<`, `if`, `[<-`, ...) that it finds, from the
//! closure's environment, to be base's: compiled code keeps using base's
//! version even if one is rebound later. Other base functions are inlined
//! behind a run-time guard (`BASEGUARD`), and everything else is looked up
//! when called, as robigo's guards always do.
//!
//! robigo compiles eagerly and guards every inlined function, so it would
//! see a later rebinding where GNU R does not. This module tracks, per
//! closure, the moment GNU R would compile it and the set of language
//! functions bound to base then (`JitState`); a failed guard in code
//! running in the frame of such a closure still takes base's function
//! ([`frozen`]), and so does the interpreter ([`frozen_fun`]).
//!
//! R: eval.c:R_CheckJIT, JIT_score, R_cmpfun (and the JIT cache);
//! compiler/R/cmp.R: getInlineInfo, findCenvVar, findLocalsList,
//! mayCallBrowser, languageFuns and the inline handlers.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::env::Env;
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

/// `languageFuns` (compiler/R/cmp.R) with an inline (or setter inline)
/// handler for package base: inlined without a guard when they resolve to
/// base at compile time. (`%%`, `%/%`, `%*%`, `~`, `@` and `[<-`'s getter
/// have no handler: they are called as found.)
const FROZEN_FUNS: &[&str] = &[
    "function", "{", "if", "&&", "||", "<-", "=", "<<-", "break", "next", "repeat", "while", "for", "+", "-", "*", "/",
    "^", "==", "!=", "<", "<=", ">=", ">", "&", "|", "!", "(", "[", "[[", "$", "local", "return", "switch", ":", "::",
    ":::", "[<-", "[[<-", "$<-", "@<-",
];

// R: eval.c (MIN_JIT_SCORE, LOOP_JIT_SCORE)
const MIN_JIT_SCORE: i64 = 50;
const LOOP_JIT_SCORE: i64 = MIN_JIT_SCORE;

const UNSEEN: u64 = 0;
/// `MAYBEJIT`: compiled at the next call.
const MAYBE: u64 = 1;
/// Created by compiled code (so compiled already); the creator's frozen
/// set is in the high bits, its own locals are taken out at the first call.
const INHERITED: u64 = 2;
/// `NOJIT`: never compiled.
const NOJIT: u64 = 3;
/// Compiled; the frozen set is in the high bits.
const COMPILED: u64 = 4;
/// Defined in a namespace: compiled when its package was installed, and
/// lookups from there find base's functions anyway.
const IGNORE: u64 = 5;

/// A closure's JIT state: the state in the low byte, the set of
/// [`FROZEN_FUNS`] (by index) bound to base in the high bits.
#[derive(Clone, Default)]
pub struct JitState(Cell<u64>);

impl JitState {
    /// Calls still have to go through [`jit_check`].
    #[inline(always)]
    pub fn unsettled(&self) -> bool {
        self.0.get() & 0xff < NOJIT
    }

    fn set(&self, state: u64, mask: u64) {
        self.0.set(state | (mask << 8));
    }

    /// The frozen set of a compiled closure.
    fn mask(&self) -> u64 {
        let v = self.0.get();
        if v & 0xff == COMPILED { v >> 8 } else { 0 }
    }
}

thread_local! {
    static SYMS: Vec<Sym> = FROZEN_FUNS.iter().map(|n| Sym::new(n)).collect();
    /// Membership of [`FROZEN_FUNS`] by symbol id.
    static BITS: Vec<u64> = {
        let ids: Vec<usize> = FROZEN_FUNS.iter().map(|n| Sym::new(n).0 as usize).collect();
        let mut b = vec![0u64; ids.iter().max().unwrap() / 64 + 1];
        for i in ids {
            b[i / 64] |= 1 << (i % 64);
        }
        b
    };
    /// Symbols `findLocals` treats specially.
    static K: Names = Names {
        assign: s::ASSIGN,
        eq_assign: s::EQ_ASSIGN,
        r#for: s::FOR,
        assign_fn: Sym::new("assign"),
        delayed_assign: Sym::new("delayedAssign"),
        function: s::FUNCTION,
        tilde: s::TILDE,
        local: Sym::new("local"),
        expression: Sym::new("expression"),
        quote: s::QUOTE,
        browser: Sym::new("browser"),
    };
    /// JIT scores and hashes of closure bodies, by address (kept alive in
    /// the entry; the hash is computed when first needed).
    static SCORES: RefCell<HashMap<usize, (Value, i64, Option<u64>), crate::env::FastHash>> =
        RefCell::new(HashMap::default());
    /// The JIT cache (R: eval.c:JIT_cache), by body hash.
    static CACHE: RefCell<HashMap<u64, Entry, crate::env::FastHash>> = RefCell::new(HashMap::default());
    /// The formals and locals (as a frozen-set mask) of closures created
    /// by compiled code, by body and formals address (kept alive in the
    /// entry).
    static LOCALS: RefCell<HashMap<(usize, usize), (Value, Rc<[Arg]>, u64), crate::env::FastHash>> =
        RefCell::new(HashMap::default());
}

struct Names {
    assign: Sym,
    eq_assign: Sym,
    r#for: Sym,
    assign_fn: Sym,
    delayed_assign: Sym,
    function: Sym,
    tilde: Sym,
    local: Sym,
    expression: Sym,
    quote: Sym,
    browser: Sym,
}

/// A JIT cache entry: the compiled body and its compile environment
/// (R: eval.c:make_cached_cmpenv: the top-level environment and the local
/// variables).
struct Entry {
    body: Value,
    top: Env,
    locals: Vec<Sym>,
    mask: u64,
}

/// Is `sym` one of [`FROZEN_FUNS`]?
#[inline(never)]
fn is_frozen_fun(sym: Sym) -> bool {
    let i = sym.0 as usize;
    BITS.with(|b| b.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 != 0))
}

#[inline(never)]
fn index(sym: Sym) -> Option<usize> {
    SYMS.with(|v| v.iter().position(|s| *s == sym))
}

/// A call of `clo` is about to run its body (R: eval.c:R_execClosure):
/// R_CheckJIT, and R_cmpfun when it says to compile.
#[cold]
#[inline(never)]
pub fn jit_check(interp: &mut Interp, clo: &Closure) {
    let v = clo.jit.0.get();
    match v & 0xff {
        UNSEEN => {
            if in_namespace(interp, &clo.env) {
                clo.jit.set(IGNORE, 0);
                return;
            }
            // R: eval.c:R_CheckJIT, strategy STRATEGY_TOP_SMALL_MAYBE.
            let score = body_score(&clo.body);
            if clo.env.ptr_eq(&interp.global) {
                // Top-level functions are compiled at their first call if
                // the score is high enough, else at their second.
                if score < MIN_JIT_SCORE {
                    clo.jit.set(MAYBE, 0);
                    return;
                }
            } else {
                // Others only if the score is high enough, at their second
                // call.
                clo.jit.set(if score < MIN_JIT_SCORE { NOJIT } else { MAYBE }, 0);
                return;
            }
            compile(interp, clo);
        }
        MAYBE => compile(interp, clo),
        INHERITED => {
            let outer = v >> 8;
            clo.jit.set(COMPILED, outer & !function_locals(interp, clo));
        }
        _ => {}
    }
}

/// Is `env` (or an enclosure of it below the global environment) a
/// namespace or the base environment? Namespaces reach the global
/// environment through the base namespace (their imports' parent), other
/// code directly.
#[inline(never)]
fn in_namespace(interp: &Interp, env: &Env) -> bool {
    let mut e = env.clone();
    loop {
        if e.ptr_eq(&interp.global) {
            return false;
        }
        if e.ptr_eq(&interp.base_ns) || e.ptr_eq(&interp.base) {
            return true;
        }
        match e.parent() {
            Some(p) => e = p,
            None => return false,
        }
    }
}

/// [`jit_score`] of a closure body, memoized by body (closures made by
/// one `function` expression share it).
#[inline(never)]
fn body_score(body: &Value) -> i64 {
    let Value::Lang(l) = body else { return 1 };
    let key = Rc::as_ptr(l) as usize;
    if let Some(n) = SCORES.with(|c| c.borrow().get(&key).map(|e| e.1)) {
        return n;
    }
    let n = jit_score(body);
    SCORES.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 100_000 {
            c.clear();
        }
        c.insert(key, (body.clone(), n, None))
    });
    n
}

/// [`hash_expr`] of a closure body, memoized with its score.
#[inline(never)]
fn body_hash(body: &Value) -> u64 {
    let Value::Lang(l) = body else { return hash_expr(body, 5381) };
    let key = Rc::as_ptr(l) as usize;
    if let Some(h) = SCORES.with(|c| c.borrow().get(&key).and_then(|e| e.2)) {
        return h;
    }
    let h = hash_expr(body, 5381);
    body_score(body);
    SCORES.with(|c| {
        if let Some(e) = c.borrow_mut().get_mut(&key) {
            e.2 = Some(h);
        }
    });
    h
}

// R: eval.c:JIT_score
#[inline(never)]
fn jit_score(e: &Value) -> i64 {
    let Value::Lang(l) = e else { return 1 };
    match l.fun.as_sym() {
        Some(f) if f == s::IF => {
            // (The condition and the consequent, as R does.)
            let cons = l.args.first().map_or(1, |a| jit_score(&a.value));
            let alt = l.args.get(1).map_or(1, |a| jit_score(&a.value));
            cons.max(alt)
        }
        Some(f) if f == s::FOR || f == s::WHILE || f == s::REPEAT => LOOP_JIT_SCORE,
        _ => l.args.iter().fold(1i64, |n, a| n.saturating_add(jit_score(&a.value))),
    }
}

/// R: eval.c:R_cmpfun: reuse the cached code of an identical body compiled
/// for a compatible environment, else compile.
#[inline(never)]
fn compile(interp: &mut Interp, clo: &Closure) {
    if may_call_browser(&clo.body) {
        // (cmpfun returns the closure uncompiled.)
        clo.jit.set(NOJIT, 0);
        return;
    }
    let hash = body_hash(&clo.body);
    let cached = CACHE.with(|c| {
        let c = c.borrow();
        let e = c.get(&hash)?;
        if !env_match(interp, e, clo) {
            return Some(None);
        }
        let flags = crate::builtins::identical::IdFlags {
            num_eq: true,
            single_na: true,
            attrib_as_set: true,
            ignore_env: true,
            ignore_srcref: false,
        };
        crate::builtins::identical::identical_flags(&e.body, &clo.body, flags).then_some(Some(e.mask))
    });
    match cached {
        Some(Some(mask)) => {
            clo.jit.set(COMPILED, mask);
            return;
        }
        Some(None) => {
            clo.jit.set(NOJIT, 0);
            return;
        }
        None => {}
    }
    let Some(mask) = compile_mask(interp, clo) else {
        // A compiler error (a bad assignment): left uncompiled.
        clo.jit.set(NOJIT, 0);
        return;
    };
    let top = interp.topenv(&clo.env);
    let mut locals: Vec<Sym> = clo.formals.iter().filter_map(|f| f.tag).collect();
    let mut e = clo.env.clone();
    while !e.ptr_eq(&top) {
        locals.extend(e.0.frame().borrow().entries.iter().map(|b| b.sym));
        match e.parent() {
            Some(p) => e = p,
            None => break,
        }
    }
    CACHE.with(|c| c.borrow_mut().insert(hash, Entry { body: clo.body.clone(), top, locals, mask }));
    clo.jit.set(COMPILED, mask);
}

// R: eval.c:jit_env_match
#[inline(never)]
fn env_match(interp: &Interp, e: &Entry, clo: &Closure) -> bool {
    let top = interp.topenv(&clo.env);
    if !top.ptr_eq(&e.top) {
        return false;
    }
    if clo.formals.iter().any(|f| f.tag.is_some_and(|t| !e.locals.contains(&t))) {
        return false;
    }
    let mut env = clo.env.clone();
    while !env.ptr_eq(&top) {
        if env.0.frame().borrow().entries.iter().any(|b| !e.locals.contains(&b.sym)) {
            return false;
        }
        match env.parent() {
            Some(p) => env = p,
            None => break,
        }
    }
    true
}

// R: eval.c:hashexpr1 (symbols and non-scalar constants by identity)
#[inline(never)]
fn hash_expr(e: &Value, h: u64) -> u64 {
    let mix = |h: u64, x: u64| h.wrapping_mul(33) ^ x;
    match e {
        Value::Lang(l) => {
            let mut h = mix(mix(h, 6), l.args.len() as u64 + 1);
            h = hash_expr(&l.fun, h);
            for a in l.args.iter() {
                h = hash_expr(&a.value, h);
            }
            h
        }
        Value::Sym(s) => mix(mix(h, 1), s.0 as u64),
        Value::Real(v) if v.data.len() == 1 => mix(mix(h, 14), v.data[0].to_bits()),
        Value::Int(v) if v.data.len() == 1 => mix(mix(h, 13), v.data[0] as u64),
        Value::Lgl(v) if v.data.len() == 1 => mix(mix(h, 10), v.data[0] as u64),
        Value::Str(v) if v.data.len() == 1 => {
            let mut h = mix(h, 16);
            if let Some(s) = &v.data[0] {
                for b in s.bytes() {
                    h = mix(h, b as u64);
                }
            }
            h
        }
        Value::Null => mix(h, 0),
        _ => mix(h, e.len() as u64),
    }
}

// R: cmp.R:mayCallBrowser
#[inline(never)]
fn may_call_browser(e: &Value) -> bool {
    let Value::Lang(l) = e else { return false };
    if let Some(f) = l.fun.as_sym() {
        if f == K.with(|k| k.browser) {
            return true;
        }
        if f == s::FUNCTION {
            return false;
        }
    } else if may_call_browser(&l.fun) {
        return true;
    }
    l.args.iter().any(|a| may_call_browser(&a.value))
}

/// The frozen set for `clo` compiled now: the [`FROZEN_FUNS`] that are
/// not formals or local variables of its body and that `findCenvVar`
/// finds in base (R: cmp.R:getInlineInfo, ftype "global"). `None` for a
/// body the compiler rejects.
#[inline(never)]
fn compile_mask(interp: &mut Interp, clo: &Closure) -> Option<u64> {
    let local = locals_mask(interp, clo)?;
    let top = interp.topenv(&clo.env);
    let mut mask = 0u64;
    SYMS.with(|syms| {
        for (i, sym) in syms.iter().enumerate() {
            if local >> i & 1 != 0 {
                continue;
            }
            // R: cmp.R:findCenvVar: the first frame with any binding of it;
            // frames below the top-level environment are local.
            let mut e = Some(clo.env.clone());
            let mut is_local = true;
            while let Some(env) = e {
                if env.ptr_eq(&top) {
                    is_local = false;
                }
                if env.has_local(*sym) {
                    if !is_local && (env.ptr_eq(&interp.base) || env.ptr_eq(&interp.base_ns)) {
                        mask |= 1 << i;
                    }
                    break;
                }
                e = env.parent();
            }
        }
    });
    Some(mask)
}

/// The [`FROZEN_FUNS`] among the formals and local variables of `clo`'s
/// body (R: cmp.R:funEnv).
#[inline(never)]
fn locals_mask(interp: &mut Interp, clo: &Closure) -> Option<u64> {
    let mut elist: Vec<&Value> = clo.formals.iter().map(|f| &f.value).collect();
    elist.push(&clo.body);
    // R: cmp.R:findLocalsList. `quote` and `expression` are never base
    // variables in the compiler's sense (not language functions); `local`
    // is unless it is a formal or not base's.
    let k = K.with(|k| [k.expression, k.local, k.quote]);
    let local_is_base = !clo.formals.iter().any(|f| f.tag == Some(k[1])) && interp.resolves_to_base(k[1], &clo.env);
    let shadowed: Vec<Sym> = k.iter().copied().filter(|n| *n != k[1] || !local_is_base).collect();
    let mut sf: Vec<Sym> = k.to_vec();
    let vals = loop {
        let vals = find_locals(&elist, &sf)?;
        let n = sf.len();
        let mut next = shadowed.clone();
        for f in &sf {
            if vals.contains(f) && !next.contains(f) {
                next.push(*f);
            }
        }
        sf = next;
        if sf.len() == n {
            break vals;
        }
    };
    Some(vals.into_iter().filter_map(index).fold(formals_mask(clo), |m, i| m | 1 << i))
}

/// The local mask of a closure created by compiled code (its own formals
/// and locals), cached by body.
#[inline(never)]
fn function_locals(interp: &mut Interp, clo: &Closure) -> u64 {
    let key = match &clo.body {
        Value::Lang(l) => (Rc::as_ptr(l) as usize, Rc::as_ptr(&clo.formals) as *const Arg as usize),
        _ => return locals_mask(interp, clo).unwrap_or(!0),
    };
    if let Some(m) = LOCALS.with(|c| c.borrow().get(&key).map(|e| e.2)) {
        return m;
    }
    let m = locals_mask(interp, clo).unwrap_or(!0);
    LOCALS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 100_000 {
            c.clear();
        }
        c.insert(key, (clo.body.clone(), clo.formals.clone(), m))
    });
    m
}

#[inline(never)]
fn formals_mask(clo: &Closure) -> u64 {
    clo.formals.iter().filter_map(|f| f.tag.and_then(index)).fold(0, |m, i| m | 1 << i)
}

// R: cmp.R:findLocalsList1 + findLocals1
#[inline(never)]
fn find_locals(elist: &[&Value], shadowed: &[Sym]) -> Option<Vec<Sym>> {
    K.with(|k| {
        let mut vars: Vec<Sym> = Vec::new();
        let mut todo: Vec<&Value> = elist.to_vec();
        while let Some(e) = todo.pop() {
            let Value::Lang(l) = e else { continue };
            let head = match &l.fun {
                Value::Sym(f) => Some(*f),
                Value::Str(v) if v.data.len() == 1 => v.data[0].as_ref().map(|s| Sym::new(s)),
                _ => None,
            };
            let Some(v) = head else {
                // A call with a computed function: all its parts.
                todo.push(&l.fun);
                todo.extend(l.args.iter().map(|a| &a.value));
                continue;
            };
            let args = l.args.iter().map(|a| &a.value);
            if v == k.assign || v == k.eq_assign {
                add(&mut vars, assigned_var(l)?);
                todo.extend(args);
            } else if v == k.r#for {
                if let Some(Value::Sym(var)) = l.args.first().map(|a| &a.value) {
                    add(&mut vars, *var);
                }
                todo.extend(args.skip(1));
            } else if v == k.assign_fn || v == k.delayed_assign {
                if let [name, value] = &l.args[..] {
                    if let Value::Str(n) = &name.value {
                        if n.data.len() == 1 {
                            if let Some(n) = &n.data[0] {
                                add(&mut vars, Sym::new(n));
                            }
                            todo.push(&value.value);
                        }
                    }
                }
            } else if v == k.function || v == k.tilde {
            } else if v == k.local && !shadowed.contains(&v) && l.args.len() == 1 {
            } else if (v == k.expression || v == k.quote) && !shadowed.contains(&v) {
            } else {
                todo.extend(args);
            }
        }
        Some(vars)
    })
}

#[inline(never)]
fn add(vars: &mut Vec<Sym>, v: Sym) {
    if !vars.contains(&v) {
        vars.push(v);
    }
}

// R: cmp.R:getAssignedVar (`None`: "bad assignment", a compiler error)
#[inline(never)]
fn assigned_var(l: &Lang) -> Option<Sym> {
    let mut v = l.args.first()?.value.clone();
    loop {
        match v {
            Value::Sym(s) if !s.is_empty() => return Some(s),
            Value::Str(ref x) if x.data.len() == 1 => return x.data[0].as_ref().map(|n| Sym::new(n)),
            Value::Lang(l) => v = l.args.first()?.value.clone(),
            _ => return None,
        }
    }
}

/// The frozen set of the compiled closure whose call frame is `env`, if
/// any.
#[inline(never)]
fn frame_mask(interp: &Interp, env: &Env) -> u64 {
    for ctx in interp.frames.iter().rev() {
        if ctx.cloenv.ptr_eq(env) {
            return match &ctx.function {
                Value::Closure(c) => c.jit.mask(),
                _ => 0,
            };
        }
    }
    0
}

/// A guard on `sym` failed in code running in `env`: does GNU R's compiled
/// code there still use base's `sym`?
#[cold]
#[inline(never)]
pub fn frozen(interp: &Interp, sym: Sym, env: &Env) -> bool {
    match index(sym) {
        Some(i) => frame_mask(interp, env) >> i & 1 != 0,
        None => false,
    }
}

/// The interpreter found `f` for the call `l` of the language function
/// `sym` in `env`: base's function instead when GNU R's compiled code
/// there would use it (and `f` is not base's own).
#[inline(never)]
pub fn frozen_fun(interp: &Interp, f: &Value, l: &Lang, env: &Env) -> Option<Value> {
    let Value::Sym(sym) = l.fun else { return None };
    if !is_frozen_fun(sym) {
        return None;
    }
    let base = interp.base.get_local(sym)?;
    if base.ptr_eq(f) {
        return None;
    }
    // Calls with `...` or empty arguments are compiled as calls (cmp.R:
    // cmpPrim2/cmpBuiltin), not inlined.
    if l.args.iter().any(|a| a.value.is_missing_arg() || a.value.as_sym() == Some(s::DOTS)) {
        return None;
    }
    frozen(interp, sym, env).then_some(base)
}

/// `function` evaluated in `env`: when that is the frame of a compiled
/// closure, the new closure is compiled code too, with the creator's
/// bindings (R: cmp.R:cmpfun compiles nested function bodies).
#[inline(never)]
pub fn jit_inherit(interp: &Interp, clo: &Closure, env: &Env) {
    if env.is_search() {
        return;
    }
    for ctx in interp.frames.iter().rev() {
        if ctx.cloenv.ptr_eq(env) {
            if let Value::Closure(c) = &ctx.function {
                let v = c.jit.0.get();
                if v & 0xff == COMPILED {
                    clo.jit.set(INHERITED, v >> 8);
                }
            }
            return;
        }
    }
}
