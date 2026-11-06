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
//! Installed code (`IGNORE`) is known by provenance only: the closures the
//! unserializer read with a BCODESXP body, as GNU R's lazy-load databases
//! (base's `base.rdb`, packages GNU R installed) and saved compiled
//! closures hold them. A closure's environment says nothing: code sourced
//! into a namespace, or made below a namespace copy (testthat's test
//! files), is JIT-decided as in GNU R. So base loaded from its sources
//! (`ROBIGO_BASE=src`, or no installed `base.rdb`) and packages robigo
//! installs itself (it writes plain bodies unless built with
//! `bytecode-fidelity`) are JIT-decided too, where GNU R's would be
//! installed bytecode.
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
/// base at compile time. (`%%`, `%/%`, `%*%`, `~` and `[<-`'s getter
/// have no handler: they are called as found.)
const FROZEN_FUNS: &[&str] = &[
    "function", "{", "if", "&&", "||", "<-", "=", "<<-", "break", "next", "repeat", "while", "for", "+", "-", "*", "/",
    "^", "==", "!=", "<", "<=", ">=", ">", "&", "|", "!", "(", "[", "[[", "$", "local", "return", "switch", ":", "::",
    ":::", "[<-", "[[<-", "$<-", "@<-", "@",
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
/// Installed: unserialized with a bytecode body (base's and packages'
/// lazy-load databases); the frozen set it was compiled with is in the high
/// bits, taken when first needed ([`installed_mask`]): [`MASK_PENDING`]
/// until then.
const IGNORE: u64 = 5;
/// An installed closure's frozen set is not computed yet. (Above any mask
/// bit: frozen sets hold [`FROZEN_FUNS`] indices only.)
const MASK_PENDING: u64 = 1 << 63;
// R: print.c:PrintClosure / serialize.c:ReadItem. A loaded BCODESXP body
// is printed as bytecode without changing the execution/inheritance state.
const LOADED_BYTECODE: u64 = 0x80;
/// debug.c's RTRACE bit (`.primTrace`). It lives here so that the inline
/// paths' [`JitState::inline_blocked`] refuses traced closures for free;
/// `duplicate` does not copy it (duplicate.c:142), nor does `Clone`.
const TRACED: u64 = 0x40;
const STATE_MASK: u64 = 0x3f;

/// A closure's JIT state: execution state and loaded-bytecode flag in the
/// low byte, and the [`FROZEN_FUNS`] bound to base in the high bits.
#[derive(Default)]
pub struct JitState(Cell<u64>);

impl Clone for JitState {
    fn clone(&self) -> Self {
        Self(Cell::new(self.0.get() & !TRACED))
    }
}

impl JitState {
    /// R: serialize.c:ReadItem preserves a BCODESXP closure body.
    pub fn from_bytecode() -> Self {
        Self(Cell::new(LOADED_BYTECODE))
    }

    /// R: print.c:PrintClosure tests isByteCode(BODY(s)), independently
    /// of robigo's eager execution bytecode.
    pub fn is_bytecode(&self) -> bool {
        let v = self.0.get();
        v & LOADED_BYTECODE != 0 || matches!(v & STATE_MASK, INHERITED | COMPILED)
    }

    /// Calls still have to go through [`jit_check`].
    #[inline(always)]
    pub fn unsettled(&self) -> bool {
        self.0.get() & STATE_MASK < NOJIT
    }

    /// [`Self::unsettled`], or traced: calls must not run inline (a traced
    /// call prints). One range check: a set [`TRACED`] bit is above it.
    #[inline(always)]
    pub fn inline_blocked(&self) -> bool {
        !(NOJIT..=IGNORE).contains(&(self.0.get() & (STATE_MASK | TRACED)))
    }

    /// debug.c's RTRACE bit of the closure.
    #[inline(always)]
    pub fn traced(&self) -> bool {
        self.0.get() & TRACED != 0
    }

    pub fn set_traced(&self, on: bool) {
        let v = self.0.get() & !TRACED;
        // (Native code for a call tree holding this closure is revalidated.
        // Only for a change: every guard memo depends on the epoch.)
        if on != self.traced() {
            crate::env::bump_search_epoch();
        }
        self.0.set(if on { v | TRACED } else { v });
    }

    /// Whether GNU R's AST interpreter would run this closure's body: it is
    /// not installed bytecode, and the JIT has not compiled it (`IGNORE`
    /// marks installed closures: their bodies were unserialized as
    /// bytecode).
    /// Only the trace reports differ between the two evaluators
    /// (eval.c:1224 vs 7894).
    pub fn interpreted(&self) -> bool {
        let v = self.0.get();
        !self.is_bytecode() && v & STATE_MASK != IGNORE
    }

    fn set(&self, state: u64, mask: u64) {
        self.0.set(state | (self.0.get() & (LOADED_BYTECODE | TRACED)) | (mask << 8));
    }

    /// The frozen set of a compiled closure.
    fn mask(&self) -> u64 {
        let v = self.0.get();
        if v & STATE_MASK == COMPILED { v >> 8 } else { 0 }
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

pub(super) fn clear_gc_caches() {
    SCORES.with(|c| c.borrow_mut().clear());
    CACHE.with(|c| c.borrow_mut().clear());
    LOCALS.with(|c| c.borrow_mut().clear());
}

// Classes of names (besides the index of a `FROZEN_FUNS` entry). Names are
// classified by text and never interned here: a new symbol would shift the
// ids, and so the frame masks (`env::sym_bit`), of all later symbols.
const C_ASSIGN_FN: u8 = 100;
const C_DELAYED_ASSIGN: u8 = 101;
const C_EXPRESSION: u8 = 102;
const C_QUOTE: u8 = 103;
const C_BROWSER: u8 = 104;
const C_OTHER: u8 = 254;
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
        "@" => 41,
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
    if let Some(c) = known_class(sym) {
        return c;
    }
    let c = sym.with_name(class_of_name);
    CLASS.with_borrow_mut(|t| {
        if t.len() <= i {
            t.resize(i + 1, 0);
        }
        t[i] = c + 1;
    });
    c
}

/// The memoized class of `sym`, if known.
#[inline(always)]
fn known_class(sym: Sym) -> Option<u8> {
    let c = CLASS.with_borrow(|t| t.get(sym.0 as usize).copied()).unwrap_or(0);
    c.checked_sub(1)
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
    match v & STATE_MASK {
        UNSEEN => {
            // Installed code is what the unserializer read with a bytecode
            // body (serialize.c ReadBC: base's and packages' lazy-load
            // databases, a saved compiled closure): compiled with the
            // bindings its environment had then. Nothing else is installed,
            // whatever its environment (a closure interpreted code made
            // below a namespace, testthat's test files): the JIT decides, as
            // for any closure. Its frozen set matters only to the closures
            // its code makes and to its `local()` calls, which most
            // installed closures never reach: it is taken then
            // ([`installed_mask`]), not at every first call.
            if v & LOADED_BYTECODE != 0 {
                clo.jit.set(IGNORE, MASK_PENDING >> 8);
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
            ignore_bytecode: true,
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

/// The frozen set of the installed closure `clo` ([`IGNORE`]): computed at
/// the first need, then kept.
#[inline(never)]
fn installed_mask(interp: &mut Interp, clo: &Closure) -> u64 {
    let v = clo.jit.0.get();
    if v & MASK_PENDING == 0 {
        return v >> 8;
    }
    let mask = compile_mask(interp, clo).unwrap_or(0);
    // (Unless finding `local` ran code that changed the closure's state.)
    if clo.jit.0.get() & (STATE_MASK | MASK_PENDING) == IGNORE | MASK_PENDING {
        clo.jit.set(IGNORE, mask);
    }
    mask
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

/// The call `l` of `clo` in compiled code (the caller checks `Interp::bc`):
/// did GNU R's compiler inline it as `(function() expr)()`? It does for
/// `local` with one argument when `local` was base's as the code was
/// compiled: the frozen set of a closure the JIT compiled, else (installed
/// package code, a top-level loop) base's `local` as found now.
// R: compiler/R/cmp.R: setInlineHandler("local") (length(e) == 2)
#[inline]
pub fn inlines_local(interp: &mut Interp, clo: &Rc<Closure>, l: &Lang, env: &Env) -> bool {
    if l.args.len() != 1 || l.fun.as_sym().is_none_or(|f| f != s::LOCAL) {
        return false;
    }
    inlines_local_slow(interp, clo, env)
}

#[cold]
#[inline(never)]
fn inlines_local_slow(interp: &mut Interp, clo: &Rc<Closure>, env: &Env) -> bool {
    let base = match interp.base.get_local(s::LOCAL) {
        Some(Value::Promise(p)) => p.forced_fun(),
        v => v,
    };
    if !base.is_some_and(|b| matches!(&b, Value::Closure(c) if Rc::ptr_eq(c, clo))) {
        return false;
    }
    let Some(ctx) = interp.frames.iter().rev().find(|ctx| ctx.cloenv.ptr_eq(env)) else {
        // A loop in the global environment (do_for and friends).
        return true;
    };
    let Value::Closure(c) = &ctx.function else { return true };
    // Compiled by the JIT, or installed (and closures their code made):
    // base's `local` when it was compiled, by its frozen set.
    let v = c.jit.0.get();
    let mask = match v & STATE_MASK {
        COMPILED => v >> 8,
        IGNORE => {
            let c = c.clone();
            installed_mask(interp, &c)
        }
        _ => return true,
    };
    index(s::LOCAL).is_some_and(|i| mask >> i & 1 != 0)
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
    match known_class(sym) {
        Some(c) if c as usize >= FROZEN_FUNS.len() => return None,
        Some(_) => {}
        None => {
            index(sym)?;
        }
    }
    // (A lazily loaded base binding is a promise: once forced, its value is
    // the function, as findFun takes PRVALUE. Still unforced, `f` was found
    // below base and base's function has not been needed yet; the call then
    // keeps `f`, since forcing here would need `&mut Interp`.)
    let base = match interp.base.get_local(sym)? {
        Value::Promise(p) => p.forced_fun()?,
        v => v,
    };
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
pub fn jit_inherit(interp: &mut Interp, clo: &Closure, env: &Env) {
    if env.is_search() {
        // R: eval.c:do_for and friends compile a loop the interpreter meets
        // in the global environment (`Interp::bc`); `function` there makes
        // a compiled closure (MAKECLOSURE), with the global environment's
        // bindings as the loop was compiled with them. (Taken now: a frozen
        // function rebound earlier in the same loop would differ.)
        if interp.bc && env.ptr_eq(&interp.global) {
            clo.jit.set(INHERITED, top_mask(interp, env, &frozen_syms()));
        }
        return;
    }
    let Some(ctx) = interp.frames.iter().rev().find(|ctx| ctx.cloenv.ptr_eq(env)) else { return };
    let Value::Closure(c) = &ctx.function else { return };
    // A formal's default is never compiled (formals stay AST even in
    // installed bytecode), so a closure created while one is forced is
    // interpreted.
    if DEFAULT_FORCING.with(|d| d.borrow().last().is_some_and(|&p| p == env.id())) {
        return;
    }
    let v = c.jit.0.get();
    match v & STATE_MASK {
        COMPILED => clo.jit.set(INHERITED, v >> 8),
        // Installed bytecode: its nested closures are compiled with it
        // (MAKECLOSURE), with its frozen set.
        IGNORE => {
            let c = c.clone();
            clo.jit.set(INHERITED, installed_mask(interp, &c));
        }
        _ => {}
    }
}

thread_local! {
    /// Environments whose formal default is being forced, innermost last.
    static DEFAULT_FORCING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Marks `env` as running a formal's default; closures created there take
/// no JIT state.
pub struct DefaultForceGuard;

impl Drop for DefaultForceGuard {
    fn drop(&mut self) {
        DEFAULT_FORCING.with(|d| d.borrow_mut().pop());
    }
}

#[cold]
#[inline(never)]
pub fn begin_default_force(env: &Env) -> DefaultForceGuard {
    DEFAULT_FORCING.with(|d| d.borrow_mut().push(env.id()));
    DefaultForceGuard
}

#[cfg(test)]
mod tests {
    #[test]
    fn classes_match_frozen_funs() {
        for (i, n) in super::FROZEN_FUNS.iter().enumerate() {
            assert_eq!(super::class_of_name(n) as usize, i, "{n}");
        }
        assert_eq!(super::class_of_name("x"), super::C_OTHER);
        // (The memo stores a class plus one in a byte.)
        const { assert!(super::C_OTHER < u8::MAX) };
    }
}
