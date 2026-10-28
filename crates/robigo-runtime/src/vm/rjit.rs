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
    /// [`class`] by symbol id, plus one (0: not known yet).
    static CLASS: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
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

// Classes of names (besides the index of a `FROZEN_FUNS` entry). Names are
// classified by text and never interned here: a new symbol would shift the
// ids, and so the frame masks (`env::sym_bit`), of all later symbols.
const C_ASSIGN_FN: u8 = 100;
const C_DELAYED_ASSIGN: u8 = 101;
const C_EXPRESSION: u8 = 102;
const C_QUOTE: u8 = 103;
const C_BROWSER: u8 = 104;
const C_OTHER: u8 = 255;
/// Mask bits (above the `FROZEN_FUNS` indices) for local variables named
/// `expression` and `quote`.
const EXPRESSION_BIT: u64 = 1 << 62;
const QUOTE_BIT: u64 = 1 << 63;

fn class_of_name(n: &str) -> u8 {
    // (Indices into `FROZEN_FUNS`; checked by a test.)
    match n {
        "function" => 0,
        "{" => 1,
        "if" => 2,
        "&&" => 3,
        "||" => 4,
        "<-" => 5,
        "=" => 6,
        "<<-" => 7,
        "break" => 8,
        "next" => 9,
        "repeat" => 10,
        "while" => 11,
        "for" => 12,
        "+" => 13,
        "-" => 14,
        "*" => 15,
        "/" => 16,
        "^" => 17,
        "==" => 18,
        "!=" => 19,
        "<" => 20,
        "<=" => 21,
        ">=" => 22,
        ">" => 23,
        "&" => 24,
        "|" => 25,
        "!" => 26,
        "(" => 27,
        "[" => 28,
        "[[" => 29,
        "$" => 30,
        "local" => 31,
        "return" => 32,
        "switch" => 33,
        ":" => 34,
        "::" => 35,
        ":::" => 36,
        "[<-" => 37,
        "[[<-" => 38,
        "$<-" => 39,
        "@<-" => 40,
        "assign" => C_ASSIGN_FN,
        "delayedAssign" => C_DELAYED_ASSIGN,
        "expression" => C_EXPRESSION,
        "quote" => C_QUOTE,
        "browser" => C_BROWSER,
        _ => C_OTHER,
    }
}

/// The class of `sym`'s name, memoized by id.
fn class(sym: Sym) -> u8 {
    let i = sym.0 as usize;
    if let Some(c) = CLASS.with(|c| c.borrow().get(i).copied()).filter(|c| *c != 0) {
        return c - 1;
    }
    let c = sym.with_name(class_of_name);
    CLASS.with(|t| {
        let mut t = t.borrow_mut();
        if t.len() <= i {
            t.resize(i + 1, 0);
        }
        t[i] = c + 1;
    });
    c
}

/// The local-variable mask bit of a name's class.
fn class_bit(c: u8) -> u64 {
    match c {
        c if (c as usize) < FROZEN_FUNS.len() => 1 << c,
        C_EXPRESSION => EXPRESSION_BIT,
        C_QUOTE => QUOTE_BIT,
        _ => 0,
    }
}

fn local_index() -> u8 {
    FROZEN_FUNS.iter().position(|f| *f == "local").unwrap() as u8
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

/// The index of `sym` in [`FROZEN_FUNS`].
#[inline(never)]
fn index(sym: Sym) -> Option<usize> {
    let c = class(sym) as usize;
    (c < FROZEN_FUNS.len()).then_some(c)
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
        if class(f) == C_BROWSER {
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
    let syms = frozen_syms();
    // R: cmp.R:findCenvVar: the first frame with any binding of the name;
    // a frame below the top-level environment makes it local.
    let mut below = 0u64;
    let mut e = clo.env.clone();
    while !e.ptr_eq(&top) {
        for (i, s) in syms.iter().enumerate() {
            if s.is_some_and(|s| e.has_local(s)) {
                below |= 1 << i;
            }
        }
        match e.parent() {
            Some(p) => e = p,
            None => break,
        }
    }
    Some(top_mask(interp, &top, &syms) & !below & !local)
}

/// The symbols of [`FROZEN_FUNS`] that exist (a name never interned is
/// bound nowhere).
fn frozen_syms() -> [Option<Sym>; FROZEN_FUNS.len()] {
    thread_local! {
        static SYMS: Cell<[Option<Sym>; FROZEN_FUNS.len()]> = const { Cell::new([None; FROZEN_FUNS.len()]) };
    }
    let mut v = SYMS.with(|c| c.get());
    if v.iter().any(|s| s.is_none()) {
        for (i, n) in FROZEN_FUNS.iter().enumerate() {
            if v[i].is_none() {
                v[i] = Sym::lookup(n);
            }
        }
        SYMS.with(|c| c.set(v));
    }
    v
}

/// The names whose first binding from the top-level environment `top` on
/// is base's, memoized for the search epoch. (Function bindings on the
/// search path change the epoch; a non-function bound to `+` or `if` there
/// is not noticed.)
fn top_mask(interp: &Interp, top: &Env, syms: &[Option<Sym>]) -> u64 {
    thread_local! {
        static MEMO: Cell<(u64, usize, u64)> = const { Cell::new((u64::MAX, 0, 0)) };
    }
    let key = (crate::env::search_epoch(), top.id());
    let m = MEMO.with(|c| c.get());
    if (m.0, m.1) == key {
        return m.2;
    }
    let mut mask = 0u64;
    for (i, s) in syms.iter().enumerate() {
        let Some(s) = s else { continue };
        let mut e = Some(top.clone());
        while let Some(env) = e {
            if env.has_local(*s) {
                if env.ptr_eq(&interp.base) || env.ptr_eq(&interp.base_ns) {
                    mask |= 1 << i;
                }
                break;
            }
            e = env.parent();
        }
    }
    MEMO.with(|c| c.set((key.0, key.1, mask)));
    mask
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
    let local_bit = 1u64 << local_index();
    let local_is_base = !clo.formals.iter().any(|f| f.tag.is_some_and(|t| class(t) == local_index()))
        && Sym::lookup("local").is_some_and(|l| interp.resolves_to_base(l, &clo.env));
    let all = EXPRESSION_BIT | QUOTE_BIT | local_bit;
    let shadowed = if local_is_base { all & !local_bit } else { all };
    let mut sf = all;
    let vals = loop {
        let vals = find_locals(&elist, sf)?;
        let n = sf.count_ones();
        sf = shadowed | (sf & vals);
        if sf.count_ones() == n {
            break vals;
        }
    };
    Some((vals & !(EXPRESSION_BIT | QUOTE_BIT)) | formals_mask(clo))
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

// R: cmp.R:findLocalsList1 + findLocals1, as a mask of the local
// variables' `class_bit`s (`shadowed`: likewise).
#[inline(never)]
fn find_locals(elist: &[&Value], shadowed: u64) -> Option<u64> {
    let local = local_index();
    let mut vars = 0u64;
    let mut todo: Vec<&Value> = elist.to_vec();
    while let Some(e) = todo.pop() {
        let Value::Lang(l) = e else { continue };
        let head = match &l.fun {
            Value::Sym(f) => Some((*f, class(*f))),
            Value::Str(v) if v.data.len() == 1 => {
                v.data[0].as_ref().map(|n| (Sym::lookup(n).unwrap_or(s::EMPTY), class_of_name(n)))
            }
            _ => None,
        };
        let Some((v, c)) = head else {
            // A call with a computed function: all its parts.
            todo.push(&l.fun);
            todo.extend(l.args.iter().map(|a| &a.value));
            continue;
        };
        let args = l.args.iter().map(|a| &a.value);
        if v == s::ASSIGN || v == s::EQ_ASSIGN {
            vars |= class_bit(assigned_var(l)?);
            todo.extend(args);
        } else if v == s::FOR {
            if let Some(Value::Sym(var)) = l.args.first().map(|a| &a.value) {
                vars |= class_bit(class(*var));
            }
            todo.extend(args.skip(1));
        } else if c == C_ASSIGN_FN || c == C_DELAYED_ASSIGN {
            if let [name, value] = &l.args[..] {
                if let Value::Str(n) = &name.value {
                    if n.data.len() == 1 {
                        if let Some(n) = &n.data[0] {
                            vars |= class_bit(class_of_name(n));
                        }
                        todo.push(&value.value);
                    }
                }
            }
        } else if v == s::FUNCTION || v == s::TILDE {
        } else if c == local && shadowed & class_bit(c) == 0 && l.args.len() == 1 {
        } else if (c == C_EXPRESSION || c == C_QUOTE) && shadowed & class_bit(c) == 0 {
        } else {
            todo.extend(args);
        }
    }
    Some(vars)
}

// R: cmp.R:getAssignedVar: the class of the assigned name (`None`: "bad
// assignment", a compiler error)
#[inline(never)]
fn assigned_var(l: &Lang) -> Option<u8> {
    let mut v = l.args.first()?.value.clone();
    loop {
        match v {
            Value::Sym(s) if !s.is_empty() => return Some(class(s)),
            Value::Str(ref x) if x.data.len() == 1 => return x.data[0].as_ref().map(|n| class_of_name(n)),
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
    index(sym)?;
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
                match v & 0xff {
                    COMPILED => clo.jit.set(INHERITED, v >> 8),
                    // (Namespace code creates namespace code.)
                    IGNORE => clo.jit.set(IGNORE, 0),
                    _ => {}
                }
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn classes_match_frozen_funs() {
        for (i, n) in super::FROZEN_FUNS.iter().enumerate() {
            assert_eq!(super::class_of_name(n) as usize, i, "{n}");
        }
        assert_eq!(super::class_of_name("x"), super::C_OTHER);
    }
}
