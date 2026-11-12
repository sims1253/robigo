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
//! The same moment decides which calls of the simple `.Internal` wrappers
//! (cmp.R:safeBaseInternals: `nchar`, `identical`, ...) the code inlines,
//! and against which definitions ([`si_decision`]): the compiler takes the
//! definition then (findFunDef) and BASEGUARD only checks, at run time,
//! that the call still finds base's binding. The promises compiled code
//! makes are compiled with it, so they keep its decision, also once its
//! frame is gone ([`si_frame_escaped`]).
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
/// debug.c's RDEBUG bit of a closure (`debug()`), and its RSTEP bit
/// (`debugonce()`). Like [`TRACED`], they keep calls off the inline paths
/// (the call goes through the browser) and are not copied.
const DEBUG: u64 = 0x20;
const STEP: u64 = 0x10;
/// The bits that make a call print or browse.
const HOOKS: u64 = TRACED | DEBUG | STEP;
const STATE_MASK: u64 = 0x0f;

/// A closure's JIT state: execution state and loaded-bytecode flag in the
/// low byte, and the [`FROZEN_FUNS`] bound to base in the high bits; then
/// its simple-wrapper inline decision ([`si_decision`], 0 until made).
#[derive(Default)]
pub struct JitState(Cell<u64>, Cell<u64>);

impl Clone for JitState {
    fn clone(&self) -> Self {
        Self(Cell::new(self.0.get() & !HOOKS), Cell::new(self.1.get()))
    }
}

impl JitState {
    /// R: serialize.c:ReadItem preserves a BCODESXP closure body.
    pub fn from_bytecode() -> Self {
        Self(Cell::new(LOADED_BYTECODE), Cell::new(0))
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

    /// [`Self::unsettled`], or traced or debugged: calls must not run
    /// inline (a traced call prints, a debugged one browses). One range
    /// check: a set [`HOOKS`] bit is above it.
    #[inline(always)]
    pub fn inline_blocked(&self) -> bool {
        !(NOJIT..=IGNORE).contains(&(self.0.get() & (STATE_MASK | HOOKS)))
    }

    /// Traced or debugged (RTRACE, RDEBUG or RSTEP): calls must go through
    /// the closure call path that reports them.
    #[inline(always)]
    pub fn hooked(&self) -> bool {
        self.0.get() & HOOKS != 0
    }

    /// debug.c's RDEBUG bit of the closure.
    pub fn debugged(&self) -> bool {
        self.0.get() & DEBUG != 0
    }

    /// debug.c's RSTEP bit of the closure (`debugonce()`).
    pub fn stepped(&self) -> bool {
        self.0.get() & STEP != 0
    }

    pub fn set_debugged(&self, on: bool) {
        self.set_hook(DEBUG, on);
    }

    pub fn set_stepped(&self, on: bool) {
        self.set_hook(STEP, on);
    }

    fn set_hook(&self, bit: u64, on: bool) {
        let v = self.0.get();
        // (Native code for a call tree holding this closure is revalidated.
        // Only for a change: every guard memo depends on the epoch.)
        if on != (v & bit != 0) {
            crate::env::bump_search_epoch();
        }
        self.0.set(if on { v | bit } else { v & !bit });
    }

    /// debug.c's RTRACE bit of the closure.
    #[inline(always)]
    pub fn traced(&self) -> bool {
        self.0.get() & TRACED != 0
    }

    pub fn set_traced(&self, on: bool) {
        self.set_hook(TRACED, on);
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
        self.0.set(state | (self.0.get() & (LOADED_BYTECODE | HOOKS)) | (mask << 8));
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
    /// The JIT cache (R: eval.c:JIT_cache and JIT_cache_hashes): direct
    /// mapped, an entry per slot with its hash.
    static CACHE: RefCell<Vec<Option<(u64, Entry)>>> = const { RefCell::new(Vec::new()) };
    /// The formals and locals (as a frozen-set mask) of closures created
    /// by compiled code, by body and formals address and the base
    /// variables they were found with (kept alive in the entry).
    static LOCALS: RefCell<HashMap<(usize, usize, u8), (Value, Rc<[Arg]>, u64), crate::env::FastHash>> =
        RefCell::new(HashMap::default());
}

// R: eval.c:JIT_CACHE_SIZE
const JIT_CACHE_SIZE: u64 = 1024;

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
const C_BQUOTE: u8 = 105;
/// The class of [`SPECIALS`]`[SP_OWN + i]` is `C_SPECIAL + i`.
const C_SPECIAL: u8 = 106;
const C_TILDE: u8 = C_SPECIAL;
/// R: cmp.R:constNames (`pi`, `T`, `F`).
const C_CONST_NAME: u8 = 253;
const C_OTHER: u8 = 254;

/// The base specials whose inline handler compiles less than all their
/// arguments (R: cmp.R: cmpSpecial, the default handler of a special
/// without one of its own, and the handlers that punt to it; `if`, which
/// drops the branch a constant test does not take), by their bit in a
/// specials mask ([`sp_bit`]). Inlined, a call of one through cmpSpecial
/// is a constant: CALLSPECIAL, no code for its arguments. The first
/// `SP_OWN` have classes of their own.
const SPECIALS: [&str; 28] = [
    "expression",
    "quote",
    "@",
    "@<-",
    "[<-",
    "[[<-",
    "$<-",
    "$",
    "return",
    "switch",
    "if",
    "[",
    "[[",
    "~",
    "call",
    "declare",
    "Exec",
    "forceAndCall",
    "missing",
    "on.exit",
    "rep",
    "round",
    "signif",
    "substitute",
    "Tailcall",
    "UseMethod",
    "log",
    ".Internal",
];
const SP_OWN: usize = 13;
const SP_IF: u32 = 10;
/// The bits of `[` and `[[`.
const SP_SUBSET: u32 = 3 << 11;
const SP_LOG: u32 = 26;
const SP_DOT_INTERNAL: u32 = 27;
/// Mask bits (above the `FROZEN_FUNS` indices) for local variables named
/// `expression` and `quote`.
const EXPRESSION_BIT: u64 = 1 << 62;
const QUOTE_BIT: u64 = 1 << 63;
/// The class of [`SIMPLE_INTERNALS`]`[i]` is `SI_CLASS + i`: a local mask
/// carries them above the `FROZEN_FUNS` indices.
const SI_CLASS: u8 = FROZEN_FUNS.len() as u8;
const SIMPLE_INTERNALS: [&str; 20] = crate::compile::SIMPLE_INTERNALS;
const SI_ALL: u64 = (1 << SIMPLE_INTERNALS.len()) - 1;
const _: () =
    assert!(SI_CLASS as usize + SIMPLE_INTERNALS.len() <= 62 && (SI_CLASS as usize + 20) < C_ASSIGN_FN as usize);

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
        "local" => C_LOCAL,
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
        "bquote" => C_BQUOTE,
        "pi" | "T" | "F" => C_CONST_NAME,
        _ => match SIMPLE_INTERNALS.iter().position(|m| *m == n) {
            Some(i) => SI_CLASS + i as u8,
            None => match SPECIALS[SP_OWN..].iter().position(|m| *m == n) {
                Some(i) => C_SPECIAL + i as u8,
                None => C_OTHER,
            },
        },
    }
}

/// The [`SPECIALS`] bit of a name's class (0 for others).
#[inline(always)]
fn sp_bit(c: u8) -> u32 {
    SP_BITS[c as usize]
}

const SP_BITS: [u32; 256] = {
    let mut t = [0u32; 256];
    let frozen = [C_EXPRESSION, C_QUOTE, 41, 40, 37, 38, 39, 30, 32, 33, 2, 28, 29];
    let mut i = 0;
    while i < SPECIALS.len() {
        let c = if i < SP_OWN { frozen[i] } else { C_SPECIAL + (i - SP_OWN) as u8 };
        t[c as usize] = 1 << i;
        i += 1;
    }
    t
};

/// The [`SPECIALS`] that code compiled with `env` above it does not inline
/// as base's: R: cmp.R:getInlineInfo (guardOK, as tryInline calls it)
/// finds the name first in a frame that is not base's.
fn specials_not_in_base(interp: &Interp, env: &Env) -> u32 {
    let mut m = 0;
    for (i, n) in SPECIALS.iter().enumerate() {
        if Sym::lookup(n).is_none_or(|s| found_in_base(interp, s, env).is_none()) {
            m |= 1 << i;
        }
    }
    m
}

/// The class of `sym`'s name, memoized by id.
fn class(sym: Sym) -> u8 {
    let i = sym.index();
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
    let c = CLASS.with_borrow(|t| t.get(sym.index()).copied()).unwrap_or(0);
    c.checked_sub(1)
}

/// The local-variable mask bit of a name's class.
fn class_bit(c: u8) -> u64 {
    match c {
        c if (c as usize) < FROZEN_FUNS.len() + SIMPLE_INTERNALS.len() => 1 << c,
        C_EXPRESSION => EXPRESSION_BIT,
        C_QUOTE => QUOTE_BIT,
        _ => 0,
    }
}

/// The class of `local` (its [`FROZEN_FUNS`] index; checked by a test).
const C_LOCAL: u8 = 31;

#[inline(always)]
fn local_index() -> u8 {
    C_LOCAL
}

/// A JIT cache entry: the compiled body, its compile environment
/// (R: eval.c:make_cached_cmpenv: the top-level environment and the local
/// variables) and the closure's source reference (the entry's TAG).
struct Entry {
    body: Value,
    top: Env,
    locals: Vec<Sym>,
    srcref: Option<Value>,
    mask: u64,
    /// The simple-wrapper decision the code was compiled with.
    si: u64,
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
            // Compiled with its creator (R: cmp.R:cmpfun compiles the
            // functions code makes, make.functionContext). The creator's
            // compile decided which wrappers its nested code inlines and
            // which of `expression`, `quote` and `local` are base variables
            // there; this closure's locals are found from its syntax and
            // those, a pure function of what the creator's compile saw, so
            // finding them now gives what finding them then would have.
            let outer = v >> 8;
            let si = clo.jit.1.get();
            let (locals, base) = function_locals(interp, clo, si);
            clo.jit.set(COMPILED, outer & !locals);
            // (The creator's decision, less this closure's own locals.)
            if si != 0 {
                clo.jit.1.set((si & !si_bits(locals) & !BV_WORD) | bv_pack(base & !locals));
            }
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
    // R: eval.c:hashfun -- the closure's srcref counts when its body has
    // none (the srcrefs compiled in then come from the closure).
    let body_srcref = clo.body.get_attr(s::SRCREF).is_some();
    let srcref = clo.attrs.as_ref().and_then(|a| a.get(s::SRCREF)).cloned();
    let mut hash = body_hash(&clo.body);
    if !body_srcref {
        hash = hash_srcref(srcref.as_ref(), hash);
    }
    let slot = (hash % JIT_CACHE_SIZE) as usize;
    let cached = CACHE.with(|c| {
        let c = c.borrow();
        let (h, e) = c.get(slot)?.as_ref()?;
        if *h != hash {
            return None;
        }
        if !env_match(interp, e, clo) {
            return Some(None);
        }
        // R: eval.c:jit_expr_match, identical(flags = IDENT_USE_CLOENV);
        // R ignores srcrefs only on closures, so they are compared here.
        let flags = crate::builtins::identical::IdFlags {
            num_eq: true,
            single_na: true,
            attrib_as_set: true,
            ignore_env: false,
            ignore_bytecode: true,
            ignore_srcref: false,
        };
        if !crate::builtins::identical::identical_flags(&e.body, &clo.body, flags) {
            return None;
        }
        // R: eval.c:jit_srcref_match, identical(flags = 0).
        let srcref_match = || match (&e.srcref, &srcref) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                let flags = crate::builtins::identical::IdFlags {
                    num_eq: true,
                    single_na: true,
                    attrib_as_set: true,
                    ignore_env: true,
                    ignore_bytecode: true,
                    ignore_srcref: false,
                };
                crate::builtins::identical::identical_flags(a, b, flags)
            }
            _ => false,
        };
        // (Else the functions differ in source references only: compiled
        // again and cached again.)
        (body_srcref || srcref_match()).then_some(Some((e.mask, e.si)))
    });
    match cached {
        Some(Some((mask, si))) => {
            // (The cached code, with the decisions it was compiled with.)
            clo.jit.set(COMPILED, mask);
            clo.jit.1.set(si);
            return;
        }
        Some(None) => {
            clo.jit.set(NOJIT, 0);
            return;
        }
        None => {}
    }
    let Some((mask, compiled)) = compile_masks(interp, clo) else {
        // A compiler error (a bad assignment): left uncompiled.
        clo.jit.set(NOJIT, 0);
        return;
    };
    let si = if interp.si_ready {
        let generation = si_generation(interp);
        let d = si_decide(interp, &clo.env, &compiled, generation);
        match si_resolve(interp, d) {
            Some(d) => d,
            None => {
                // (A compiler error: tryCmpfun returns it uncompiled.)
                clo.jit.set(NOJIT, 0);
                return;
            }
        }
    } else {
        0
    };
    let top = interp.topenv(&clo.env);
    let mut locals: Vec<Sym> = clo.formals.iter().filter_map(|f| f.tag).collect();
    let mut e = clo.env.clone();
    while !e.ptr_eq(&top) {
        locals.extend(e.0.frame().borrow().bindings().map(|b| b.sym));
        match e.parent() {
            Some(p) => e = p,
            None => break,
        }
    }
    let entry = Entry { body: clo.body.clone(), top, locals, srcref, mask, si };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_empty() {
            c.resize_with(JIT_CACHE_SIZE as usize, || None);
        }
        c[slot] = Some((hash, entry));
    });
    clo.jit.set(COMPILED, mask);
    clo.jit.1.set(si);
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
        let frame = env.0.frame();
        let frame = frame.borrow();
        // (Only standard unhashed frames match.)
        if frame.hashed || frame.bindings().any(|b| !e.locals.contains(&b.sym)) {
            return false;
        }
        drop(frame);
        match env.parent() {
            Some(p) => env = p,
            None => break,
        }
    }
    true
}

// R: eval.c:hash -- djb2 over the bytes of a value as it lies in memory
// (`HASH(x, h)`: little-endian, `sizeof(x)` bytes).
#[inline(always)]
fn djb2(bytes: &[u8], mut h: u64) -> u64 {
    for &b in bytes {
        h = (h << 5).wrapping_add(h).wrapping_add(b as u64);
    }
    h
}

/// The identity GNU R hashes for an object it does not hash by content:
/// its address (`HASH(e, h)`). Symbols are unique, so their index stands
/// for theirs.
fn sexp_identity(e: &Value) -> u64 {
    match e {
        Value::Null => 0,
        Value::Sym(s) => (s.index() as u64 + 1) << 6,
        Value::Builtin(b) => *b as *const _ as usize as u64,
        Value::Promise(r) => Rc::as_ptr(r) as *const u8 as usize as u64,
        Value::Dots(r) => Rc::as_ptr(r) as *const u8 as usize as u64,
        Value::Bytecode(r) => Rc::as_ptr(r) as *const u8 as usize as u64,
        _ => crate::deparse::value_addr(e).unwrap_or(0) as u64,
    }
}

// R: eval.c:hashsrcref
fn hash_srcref(srcref: Option<&Value>, mut h: u64) -> u64 {
    let Some(Value::Int(v)) = srcref else { return h };
    if v.data.len() < 6 {
        return h;
    }
    for &i in &v.data[..6] {
        h = djb2(&i.to_le_bytes(), h);
    }
    // (By identity: R_NilValue's when there is none.)
    let srcfile = Sym::lookup("srcfile").and_then(|s| srcref?.get_attr(s)).unwrap_or(Value::Null);
    djb2(&sexp_identity(&srcfile).to_le_bytes(), h)
}

// R: eval.c:hashexpr1 -- the type and length, then calls and pairlists by
// their elements (not their tags), scalar logicals, integers, doubles and
// strings by value, and anything else (symbols, longer vectors) by
// identity.
#[inline(never)]
fn hash_expr(e: &Value, mut h: u64) -> u64 {
    let len = e.len() as i32;
    h = djb2(&(crate::builtins::summary::sexptype(e) as i32).to_le_bytes(), h);
    h = djb2(&len.to_le_bytes(), h);
    match e {
        Value::Lang(l) => {
            h = hash_expr(&l.fun, h);
            for a in l.args.iter() {
                h = hash_expr(&a.value, h);
            }
            return h;
        }
        Value::Pairlist(p) => {
            for a in p.items.iter() {
                h = hash_expr(&a.value, h);
            }
            return h;
        }
        Value::Lgl(v) if len == 1 => return djb2(&v.data[0].to_le_bytes(), h),
        Value::Int(v) if len == 1 => return djb2(&v.data[0].to_le_bytes(), h),
        Value::Real(v) if len == 1 => return djb2(&v.data[0].to_bits().to_le_bytes(), h),
        Value::Str(v) if len == 1 => {
            // (CHAR(NA_STRING) is "NA".)
            return match &v.data[0] {
                Some(s) => djb2(&s.raw_bytes(), h),
                None => djb2(b"NA", h),
            };
        }
        _ => {}
    }
    djb2(&sexp_identity(e).to_le_bytes(), h)
}

// R: cmp.R:mayCallBrowser
#[inline(never)]
pub(super) fn may_call_browser(e: &Value) -> bool {
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
/// body the compiler rejects. With it, what the compile pass found
/// ([`compile_pass`]).
#[inline(never)]
fn compile_masks(interp: &mut Interp, clo: &Closure) -> Option<(u64, Compiled)> {
    let compiled = compile_pass(interp, &clo.formals, &clo.body, &clo.env)?;
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
    Some((top_mask(interp, &top, &syms) & !below & !compiled.vars, compiled))
}

/// The frozen set of the installed closure `clo` ([`IGNORE`]): computed at
/// the first need, then kept. Its simple-wrapper decision comes from the
/// same walk (installed code was compiled with the shipped definitions,
/// generation 0).
#[inline(never)]
fn installed_mask(interp: &mut Interp, clo: &Closure) -> u64 {
    let v = clo.jit.0.get();
    if v & MASK_PENDING == 0 {
        return v >> 8;
    }
    let (mask, si) = match compile_masks(interp, clo) {
        Some((mask, compiled)) => (mask, si_decide(interp, &clo.env, &compiled, 0)),
        None => (0, si_pack(0, 0)),
    };
    // (Unless finding `local` ran code that changed the closure's state.)
    if clo.jit.0.get() & (STATE_MASK | MASK_PENDING) == IGNORE | MASK_PENDING {
        clo.jit.set(IGNORE, mask);
        if interp.si_ready && clo.jit.1.get() == 0 {
            clo.jit.1.set(si);
        }
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

/// What compiling code decides (R: cmp.R:cmpfun, and compile for a
/// top-level loop): one pass over it that, as the compiler does, goes into
/// the code of the arguments it passes (promises), the functions it makes
/// and the inlined `local()` bodies, all compiled with it.
struct Compiled {
    /// The code's own locals with its formals (R: cmp.R:funEnv), as a
    /// local mask ([`class_bit`]s).
    vars: u64,
    /// The [`SIMPLE_INTERNALS`] the compiled code calls (anywhere in it,
    /// nested functions included) where no enclosing code of it has them
    /// as locals: the calls tryInline tries to inline.
    inlined: u64,
    /// Which of `expression`, `quote` and `local` are base variables
    /// (cmp.R:isBaseVar) for the functions the code makes, as a local mask.
    base: u64,
}

/// The mask bits of `expression`, `quote` and `local`.
fn bv_all() -> u64 {
    EXPRESSION_BIT | QUOTE_BIT | 1 << local_index()
}

/// [`Compiled`] for code `body` with `formals` compiled with `env` above
/// it; `None` for a compiler error (a bad assignment), anywhere in it.
#[inline(never)]
fn compile_pass(interp: &mut Interp, formals: &[Arg], body: &Value, env: &Env) -> Option<Compiled> {
    let all = bv_all();
    // The base variables of the code itself, found when first needed (only
    // code that calls `expression`, `quote` or `local` needs them).
    let mut top_base = None;
    fn base_at(top: &mut Option<u64>, interp: &Interp, formals: &[Arg], env: &Env, cleared: u64) -> u64 {
        *top.get_or_insert_with(|| base_vars(interp, formals, env)) & !cleared
    }
    // Likewise the specials it does not inline (only code that calls one
    // needs them).
    let mut top_sp = None;
    fn sp_at(top: &mut Option<u32>, interp: &Interp, env: &Env, locals: u32) -> u32 {
        locals | *top.get_or_insert_with(|| specials_not_in_base(interp, env))
    }
    let mut top = unit_locals(formals, body, &mut || base_at(&mut top_base, interp, formals, env, 0), &mut |l| {
        sp_at(&mut top_sp, interp, env, l)
    })?;
    // (The units walked, for the `if` tests that fold.)
    let mut units: Vec<Unit> = vec![(formals, body, top.sf, None)];
    if !top.folds.is_empty() {
        top = fold_ifs(interp, &units, 0, env, top)?;
    }
    let mut inlined = top.calls & !si_bits(top.vars);
    // (Units still to walk: formals, body, the base variables enclosing
    // code cleared, the wrappers and the specials it has as locals, the
    // enclosing unit.)
    let mut todo: Vec<(&[Arg], &Value, u64, u64, u32, usize)> =
        top.nested.iter().map(|&(f, b)| (f, b, top.vars & all, si_bits(top.vars), top.svars, 0)).collect();
    while let Some((f, b, cleared, chain, schain, parent)) = todo.pop() {
        // R: cmp.R:make.functionContext: the unit's formals, then its
        // locals, sit on top of the enclosing code's.
        let cleared = cleared | (formals_mask(f) & all);
        let mut u = unit_locals(f, b, &mut || base_at(&mut top_base, interp, formals, env, cleared), &mut |l| {
            sp_at(&mut top_sp, interp, env, l | schain)
        })?;
        let at = units.len();
        units.push((f, b, u.sf, Some(parent)));
        if !u.folds.is_empty() {
            u = fold_ifs(interp, &units, at, env, u)?;
        }
        let chain = chain | si_bits(u.vars);
        inlined |= u.calls & !chain;
        let cleared = cleared | (u.vars & all);
        let schain = schain | u.svars;
        todo.extend(u.nested.iter().map(|&(f, b)| (f, b, cleared, chain, schain, at)));
    }
    // (When no code needed them, the base variables matter to no function
    // the code makes either.)
    let base = top_base.map_or(all, |b| b & !top.vars);
    Some(Compiled { vars: top.vars, inlined, base })
}

/// R: cmp.R:findLocalsList over a unit of code (the formals' defaults and
/// the body; [`find_locals`]), with `base()` the base variables among
/// `expression`, `quote` and `local` there. Its local mask includes the
/// formals, and the bits of locals named `expression` and `quote`.
/// `not_inlined(locals)`: the [`SPECIALS`] the unit does not inline, given
/// those among its own formals and locals.
fn unit_locals<'a>(
    formals: &'a [Arg],
    body: &'a Value,
    base: &mut dyn FnMut() -> u64,
    not_inlined: &mut dyn FnMut(u32) -> u32,
) -> Option<Found<'a>> {
    // Of `expression`, `local` and `quote`, those that are not base
    // variables in the compiler's sense (isBaseVar) are shadowed: their
    // arguments are searched too. Code that calls none of them finds the
    // same locals whichever are.
    let all = bv_all();
    let mut found = find_locals(formals, body, all, 0, None)?;
    let mut sf = all;
    if found.quoting {
        let shadowed = all & !base();
        loop {
            let n = sf.count_ones();
            sf = shadowed | (sf & found.vars);
            if sf.count_ones() == n {
                break;
            }
            found = find_locals(formals, body, sf, 0, None)?;
        }
    }
    found.vars |= formals_mask(formals);
    found.svars |= formals_smask(formals);
    // What is compiled depends on which specials are inlined (the walks
    // so far took all to be); the locals do not.
    if found.special {
        let sp = not_inlined(found.svars);
        if sp != 0 {
            let (vars, svars) = (found.vars, found.svars);
            found = find_locals(formals, body, sf, sp, None)?;
            (found.vars, found.svars) = (vars, svars);
        }
    }
    Some(found)
}

/// Which of `expression`, `quote` and `local` are base variables for code
/// with `formals` compiled with `env` above it ([`is_base_var`]).
fn base_vars(interp: &Interp, formals: &[Arg], env: &Env) -> u64 {
    let mut base = 0;
    for (bit, name) in [(EXPRESSION_BIT, "expression"), (QUOTE_BIT, "quote"), (1 << local_index(), "local")] {
        if is_base_var(interp, name, formals, env) {
            base |= bit;
        }
    }
    base
}

/// R: cmp.R:isBaseVar for code with `formals` compiled with `env` above
/// it: getInlineInfo finds `name` in base. Found in the base namespace
/// (ftype "namespace"), any name is; on the search path (ftype "global",
/// optimize level 2) only the language functions (`local`, not `quote` or
/// `expression`).
fn is_base_var(interp: &Interp, name: &str, formals: &[Arg], env: &Env) -> bool {
    // (A name never interned is bound nowhere.)
    let Some(sym) = Sym::lookup(name) else { return false };
    if formals.iter().any(|f| f.tag == Some(sym)) {
        return false;
    }
    match found_in_base(interp, sym, env) {
        Some(ns) => ns || (class(sym) as usize) < FROZEN_FUNS.len(),
        None => false,
    }
}

/// R: cmp.R:findCenvVar from `env`: is the first frame that binds `sym`
/// (a value of any type: `exists(inherits = FALSE)`) base's? `Some(true)`
/// for the base namespace (code below a namespace finds it there),
/// `Some(false)` for the base environment, `None` for another frame or
/// none.
fn found_in_base(interp: &Interp, sym: Sym, env: &Env) -> Option<bool> {
    let mut e = env.clone();
    loop {
        if e.ptr_eq(&interp.base_ns) || e.ptr_eq(&interp.base) {
            return e.has_local(sym).then(|| e.ptr_eq(&interp.base_ns));
        }
        if e.has_local(sym) {
            return None;
        }
        e = e.parent()?;
    }
}

/// The local mask of a closure created by compiled code (its own formals
/// and locals), cached by body, and the base variables it was found with.
/// `decision`: its creator's, which carries them (else they are found
/// now, for code compiled before base had loaded).
#[inline(never)]
fn function_locals(interp: &mut Interp, clo: &Closure, decision: u64) -> (u64, u64) {
    // (The formals are part of the key, so the base variables less the
    // formals are too; the locals include the formals, so the caller
    // takes them out with the locals.)
    let decided = decision & SI_DECIDED != 0;
    let base = if decided { bv_unpack(decision) } else { 0 };
    let base_key = if decided { (decision >> BV_SHIFT & 7) as u8 } else { 8 };
    let find = |interp: &Interp| {
        let mut base_now =
            || if decided { base & !formals_mask(&clo.formals) } else { base_vars(interp, &clo.formals, &clo.env) };
        unit_locals(&clo.formals, &clo.body, &mut base_now, &mut |_| 0).map_or(!0, |u| u.vars)
    };
    let key = match &clo.body {
        Value::Lang(l) => (Rc::as_ptr(l) as usize, Rc::as_ptr(&clo.formals) as *const Arg as usize, base_key),
        _ => return (find(interp), base),
    };
    if let Some(m) = LOCALS.with(|c| c.borrow().get(&key).map(|e| e.2)) {
        return (m, base);
    }
    let m = find(interp);
    LOCALS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 100_000 {
            c.clear();
        }
        c.insert(key, (clo.body.clone(), clo.formals.clone(), m))
    });
    (m, base)
}

#[inline(never)]
fn formals_mask(formals: &[Arg]) -> u64 {
    formals.iter().filter_map(|f| f.tag).fold(0, |m, t| m | class_bit(class(t)))
}

/// The [`SPECIALS`] among `formals` ([`sp_bit`]s).
fn formals_smask(formals: &[Arg]) -> u32 {
    formals.iter().filter_map(|f| f.tag).fold(0, |m, t| m | sp_bit(class(t)))
}

/// What [`find_locals`] finds in a unit of code.
struct Found<'a> {
    /// The local variables, as a mask of their [`class_bit`]s.
    vars: u64,
    /// The [`SIMPLE_INTERNALS`] the unit's compiled code calls by name,
    /// without `...` (R: cmp.R:cmpSimpleInternal's anyDots).
    calls: u64,
    /// Does it call `expression`, `local` or `quote`? (When not,
    /// `shadowed` changes nothing.)
    quoting: bool,
    /// The [`SPECIALS`] among the local variables ([`sp_bit`]s).
    svars: u32,
    /// Does code that one of the [`SPECIALS`] keeps as constants hold
    /// wrapper calls or functions? (When not, which are inlined changes
    /// nothing.)
    special: bool,
    /// The code compiled with it as functions of its own: the formals and
    /// body of each `function` it makes, the body of each `local()` call
    /// the compiler inlines as `(function() body)()`.
    nested: Vec<(&'a [Arg], &'a Value)>,
    /// The tests of inlined `if` calls that may fold to a constant
    /// (cmp.R:constantFold; [`fold_candidate`]) and drop code that
    /// [`has_compiled_parts`]; the walk took them not to fold.
    folds: Vec<&'a Value>,
    /// The `shadowed` and `not_inlined` of the walk.
    sf: u64,
    sp: u32,
}

/// The tests of [`Found::folds`] by address, with the branch each takes.
type Folds = [(*const Value, Option<bool>)];

// R: cmp.R:findLocalsList1 + findLocals1 over `formals`' defaults and
// `body`, as a mask of the local variables' `class_bit`s (`shadowed`:
// likewise), with what compiling the body visits: the wrapper calls and
// the nested units. The two walks differ. findLocals1 does not search
// what `function`, `~`, `expression` and `quote` quote, nor the arguments
// of an `assign()` or `delayedAssign()` call that is not `(name, value)`.
// The compiler compiles every argument of a call (cmpCallArgs, as promise
// code), but not those of a special it inlines through cmpSpecial
// ([`SPECIALS`] not in `not_inlined`, [`special_args`]), `bquote`'s
// (maybeNSESymbols), a function's defaults, nor the branch an `if` with a
// literal logical test drops (constantFold). Each part is walked with
// whether it is searched and whether it is compiled.
#[inline(never)]
fn find_locals<'a>(
    formals: &'a [Arg],
    body: &'a Value,
    shadowed: u64,
    not_inlined: u32,
    folds: Option<&Folds>,
) -> Option<Found<'a>> {
    let local = local_index();
    let mut found = Found {
        vars: 0,
        calls: 0,
        quoting: false,
        svars: 0,
        special: false,
        nested: Vec::new(),
        folds: Vec::new(),
        sf: shadowed,
        sp: not_inlined,
    };
    let mut todo: Vec<(&Value, bool, bool)> = formals.iter().map(|f| (&f.value, true, false)).collect();
    todo.push((body, true, true));
    while let Some((e, search, compiled)) = todo.pop() {
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
            todo.push((&l.fun, search, compiled));
            todo.extend(l.args.iter().map(|a| (&a.value, search, compiled)));
            continue;
        };
        let si = c.wrapping_sub(SI_CLASS);
        if compiled
            && (si as usize) < SIMPLE_INTERNALS.len()
            && !l.args.iter().any(|a| a.value.as_sym() == Some(s::DOTS))
        {
            found.calls |= 1 << si;
        }
        // (Whether these are shadowed decides both walks.)
        found.quoting |= c == local || c == C_EXPRESSION || c == C_QUOTE;
        let sp = sp_bit(c);
        // R: cmp.R:cmpCall: only a call by symbol tries an inline handler.
        let sym_head = matches!(l.fun, Value::Sym(_));
        let args = l.args.iter().map(|a| (&a.value, search, compiled));
        // The arguments compiled but not searched.
        let compiled_only = |todo: &mut Vec<_>| {
            if compiled {
                todo.extend(l.args.iter().map(|a| (&a.value, false, true)));
            }
        };
        if v == s::ASSIGN || v == s::EQ_ASSIGN {
            if search {
                let a = assigned_var(l)?;
                found.vars |= class_bit(a);
                found.svars |= sp_bit(a);
            }
            todo.extend(args);
        } else if v == s::FOR {
            if let (true, Some(Value::Sym(var))) = (search, l.args.first().map(|a| &a.value)) {
                let c = class(*var);
                found.vars |= class_bit(c);
                found.svars |= sp_bit(c);
            }
            todo.extend(args.skip(1));
        } else if c == C_ASSIGN_FN || c == C_DELAYED_ASSIGN {
            match &l.args[..] {
                [name, value] if search && matches!(&name.value, Value::Str(n) if n.data.len() == 1) => {
                    if let Value::Str(n) = &name.value {
                        if let Some(n) = &n.data[0] {
                            let c = class_of_name(n);
                            found.vars |= class_bit(c);
                            found.svars |= sp_bit(c);
                        }
                    }
                    todo.push((&value.value, true, compiled));
                }
                _ => compiled_only(&mut todo),
            }
        } else if v == s::FUNCTION {
            if compiled {
                let formals: &[Arg] = match l.args.first().map(|a| &a.value) {
                    Some(Value::Pairlist(p)) => &p.items,
                    _ => &[],
                };
                found.nested.extend(l.args.get(1).map(|b| (formals, &b.value)));
            }
        } else if c == local && shadowed & class_bit(c) == 0 && l.args.len() == 1 {
            // R: cmp.R:setInlineHandler("local"): `(function() e)()`.
            if compiled {
                found.nested.push((&[], &l.args[0].value));
            }
        } else if sp & SP_SUBSET != 0
            && l.args.first().is_some_and(|a| !a.value.is_missing_arg())
            && !l.args.iter().any(|a| a.value.as_sym() == Some(s::DOTS))
        {
            // (The common `[` and `[[` calls: all compiled either way.)
            todo.extend(args);
        } else if v == s::IF
            && l.args.first().is_none_or(|a| !matches!(a.value, Value::Lgl(_)) && !fold_candidate(&a.value))
        {
            // (Likewise an `if` whose test cannot fold.)
            todo.extend(args);
        } else if sp != 0 {
            walk_special(l, c, sp, (search, compiled, sym_head), (shadowed, not_inlined, folds), &mut todo, &mut found);
        } else if c == C_BQUOTE && sym_head {
            if search {
                todo.extend(l.args.iter().map(|a| (&a.value, true, false)));
            }
        } else {
            todo.extend(args);
        }
    }
    Some(found)
}

type Todo<'a> = Vec<(&'a Value, bool, bool)>;

/// [`find_locals`] of a call of one of the [`SPECIALS`] (`sp`, class `c`),
/// searched, compiled and called by symbol as `how`.
#[inline(never)]
fn walk_special<'a>(
    l: &'a Lang,
    c: u8,
    sp: u32,
    (search, compiled, sym_head): (bool, bool, bool),
    (shadowed, not_inlined, folds): (u64, u32, Option<&Folds>),
    todo: &mut Todo<'a>,
    found: &mut Found<'a>,
) {
    let inlined = compiled && sym_head && not_inlined & sp == 0;
    if sp == 1 << SP_IF {
        return walk_if(l, (search, compiled, inlined), folds, todo, found);
    }
    // R: cmp.R:findLocals1 searches all but what `~` and an unshadowed
    // `expression` or `quote` quote.
    let search = search
        && match c {
            C_TILDE => false,
            C_EXPRESSION | C_QUOTE => shadowed & class_bit(c) != 0,
            _ => true,
        };
    let how = if inlined { special_args(l, sp) } else { SpArgs::All };
    if let (SpArgs::Inner, Some(Value::Lang(inner))) = (how, l.args.first().map(|a| &a.value)) {
        // R: cmp.R:cmpDotInternalCall: cmpBuiltin compiles the internal's
        // arguments, not the call.
        todo.extend(inner.args.iter().map(|a| (&a.value, false, true)));
        found.special |= compiles_to_more(inner.fun.as_sym());
    }
    for (i, a) in l.args.iter().enumerate() {
        let compiled = compiled
            && match how {
                SpArgs::All => true,
                SpArgs::First => i == 0,
                SpArgs::Constant | SpArgs::Inner => false,
            };
        // (Code dropped here is compiled if the special is not inlined
        // after all; it matters when it has wrapper calls or functions.)
        if !compiled && !found.special && matches!(how, SpArgs::Constant | SpArgs::First) {
            found.special = has_compiled_parts(&a.value);
        }
        if search || compiled {
            todo.push((&a.value, search, compiled));
        }
    }
}

/// [`find_locals`] of an `if` call, searched, compiled and inlined as
/// given.
// R: cmp.R:setInlineHandler("if"): a test that folds to TRUE or FALSE
// (constantFold) compiles only the branch it takes, not the test.
#[inline(never)]
fn walk_if<'a>(
    l: &'a Lang,
    (search, compiled, inlined): (bool, bool, bool),
    folds: Option<&Folds>,
    todo: &mut Todo<'a>,
    found: &mut Found<'a>,
) {
    if let (true, [test, branches @ ..]) = (inlined, &l.args[..]) {
        let t = &test.value;
        let taken = match t {
            Value::Lgl(x) => (x.data.len() == 1 && x.data[0] != NA_LOGICAL).then(|| x.data[0] != 0),
            Value::Lang(_) | Value::Sym(_) if fold_candidate(t) => match folds {
                Some(folds) => folds.iter().find(|f| std::ptr::eq(f.0, t)).and_then(|f| f.1),
                // (Found first; which branch, if any, is decided once the
                // unit's locals are known: fold_ifs.)
                None => {
                    if branches.iter().any(|a| has_compiled_parts(&a.value)) {
                        found.folds.push(t);
                    }
                    None
                }
            },
            _ => None,
        };
        if let Some(taken) = taken {
            if search {
                todo.push((t, true, false));
            }
            for (i, a) in branches.iter().enumerate() {
                let kept = (i == 0) == taken;
                // (Code dropped here is compiled if `if` is not inlined
                // after all.)
                if !kept && !found.special {
                    found.special = has_compiled_parts(&a.value);
                }
                if search || kept {
                    todo.push((&a.value, search, kept));
                }
            }
            return;
        }
    }
    todo.extend(l.args.iter().map(|a| (&a.value, search, compiled)));
}

/// Might `e` fold to a constant (R: cmp.R:constantFold)? Calls of the
/// fold functions by name, `T`, `F` and `pi`, constants; whether the names
/// are base's is for [`constant_fold`] to find.
fn fold_candidate(e: &Value) -> bool {
    match e {
        Value::Lang(l) => {
            l.args.iter().all(|a| !a.value.is_missing_arg() && fold_candidate(&a.value))
                && l.fun.as_sym().is_some_and(|f| f.with_name(|n| FOLD_FUNS.contains(&n)))
        }
        Value::Sym(s) => class(*s) == C_CONST_NAME,
        v => check_const(v),
    }
}

// R: cmp.R:foldFuns
const FOLD_FUNS: [&str; 69] = [
    "+",
    "-",
    "*",
    "/",
    "^",
    "(",
    ">",
    ">=",
    "==",
    "!=",
    "<",
    "<=",
    "||",
    "&&",
    "!",
    "|",
    "&",
    "%%",
    "c",
    "rep",
    ":",
    "abs",
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "ceiling",
    "choose",
    "cos",
    "cosh",
    "exp",
    "expm1",
    "floor",
    "gamma",
    "lbeta",
    "lchoose",
    "lgamma",
    "log",
    "log10",
    "log1p",
    "log2",
    "max",
    "min",
    "prod",
    "range",
    "round",
    "seq_along",
    "seq.int",
    "seq_len",
    "sign",
    "signif",
    "sin",
    "sinh",
    "sqrt",
    "sum",
    "tan",
    "tanh",
    "trunc",
    "baseenv",
    "emptyenv",
    "globalenv",
    "Arg",
    "Conj",
    "Im",
    "Mod",
    "Re",
];

/// R: cmp.R:checkConst: a value of a constant mode, at most
/// maxConstSize (10) long.
fn check_const(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_))
        && v.len() <= 10
}

/// The `if` tests of a unit (`units[at]`, its enclosing units by parent
/// index) that fold, folded now with the unit's locals known, and the
/// unit walked again with them.
#[cold]
#[inline(never)]
fn fold_ifs<'a>(interp: &mut Interp, units: &[Unit<'a>], at: usize, env: &Env, u: Found<'a>) -> Option<Found<'a>> {
    // R: cmp.R:isBaseVar: not a formal or local of the unit or of code
    // enclosing it.
    let local = |s: Sym| {
        let mut i = Some(at);
        while let Some(j) = i {
            let (f, b, sf, parent) = units[j];
            if unit_binds(f, b, sf, s) {
                return true;
            }
            i = parent;
        }
        false
    };
    let mut folds = Vec::with_capacity(u.folds.len());
    for &t in &u.folds {
        let taken = match constant_fold(interp, t, &local, env) {
            Some(Value::Lgl(x)) if x.data.len() == 1 && x.data[0] != NA_LOGICAL => Some(x.data[0] != 0),
            _ => None,
        };
        folds.push((t as *const Value, taken));
    }
    let (f, b, ..) = units[at];
    let mut r = find_locals(f, b, u.sf, u.sp, Some(&folds))?;
    (r.vars, r.svars) = (u.vars, u.svars);
    Some(r)
}

/// A unit of code [`compile_pass`] walked: formals, body, the `shadowed`
/// of its walk and the unit enclosing it.
type Unit<'a> = (&'a [Arg], &'a Value, u64, Option<usize>);

/// R: cmp.R:constantFold of `e` in code where `local(name)` says a name
/// is a local variable, compiled with `env` above it: the value, if it is
/// a constant.
fn constant_fold(interp: &mut Interp, e: &Value, local: &dyn Fn(Sym) -> bool, env: &Env) -> Option<Value> {
    // R: cmp.R:getFoldFun, constantFoldSym: base's (isBaseVar), from the
    // base namespace.
    let base_value = |interp: &mut Interp, s: Sym| {
        if local(s) || !found_in_base(interp, s, env).is_some_and(|ns| ns || is_language_fun(s)) {
            return None;
        }
        let (v, _) = interp.base_ns.clone().lookup(s)?;
        interp.force(v).ok()
    };
    match e {
        Value::Lang(l) => {
            // R: cmp.R:constantFoldCall
            let f = l.fun.as_sym().filter(|f| f.with_name(|n| FOLD_FUNS.contains(&n)))?;
            let fun = base_value(interp, f).filter(Value::is_function)?;
            let mut args = Vec::with_capacity(l.args.len());
            for a in &l.args {
                if a.value.is_missing_arg() {
                    return None;
                }
                args.push(Arg::new(a.tag, constant_fold(interp, &a.value, local, env)?));
            }
            fold_call(interp, fun, args).filter(check_const)
        }
        Value::Sym(s) if class(*s) == C_CONST_NAME => base_value(interp, *s).filter(check_const),
        // (A literal promise or bytecode is a compiler error.)
        Value::Sym(_) | Value::Promise(_) | Value::Bytecode(_) => None,
        v => check_const(v).then(|| v.clone()),
    }
}

/// R: cmp.R:constantFoldCall: `tryCatch(do.call(fun, args), error =,
/// warning = )`, `None` for an error or a warning.
fn fold_call(interp: &mut Interp, fun: Value, args: Vec<Arg>) -> Option<Value> {
    const SRC: &str = "function(f, a) tryCatch(do.call(f, a), error = function(e) NULL, warning = function(w) NULL)";
    let base = interp.base_ns.clone();
    let code = crate::builtins::parsing::try_parse_text(SRC, "<constantFold>").ok()?;
    let (vis, bc) = (interp.visible, interp.bc);
    let r = interp.eval(&code[0], &base).and_then(|f| {
        let names: Vec<String> = args.iter().map(|a| a.tag.map_or(String::new(), |t| t.name().to_string())).collect();
        let mut list = Vector::new(args.into_iter().map(|a| a.value).collect());
        if names.iter().any(|n| !n.is_empty()) {
            list.attrs = Some(Box::new(Attrs(
                vec![(s::NAMES, Value::strs_from(names.iter().map(|n| n.as_str())))],
                false,
                false,
            )));
        }
        interp.call_value(&f, vec![Arg::pos(fun), Arg::pos(Value::List(Rc::new(list)))], &base)
    });
    (interp.visible, interp.bc) = (vis, bc);
    r.ok()
}

/// cmp.R:languageFuns: the `FROZEN_FUNS` and `%%`, `%/%`, `%*%`, `~`.
fn is_language_fun(s: Sym) -> bool {
    (class(s) as usize) < FROZEN_FUNS.len() || s.with_name(|n| matches!(n, "%%" | "%/%" | "%*%" | "~"))
}

/// R: cmp.R:findLocalsList over a unit with `shadowed` as [`find_locals`]
/// searched it: does it assign `sym` (or have it as a formal)?
#[cold]
#[inline(never)]
fn unit_binds(formals: &[Arg], body: &Value, shadowed: u64, sym: Sym) -> bool {
    let is = |v: &Value| match v {
        Value::Sym(s) => *s == sym,
        Value::Str(x) => x.data.len() == 1 && x.data[0].as_deref().is_some_and(|n| sym.with_name(|m| m == n)),
        _ => false,
    };
    if formals.iter().any(|f| f.tag == Some(sym)) {
        return true;
    }
    let mut todo: Vec<&Value> = formals.iter().map(|f| &f.value).collect();
    todo.push(body);
    while let Some(e) = todo.pop() {
        let Value::Lang(l) = e else { continue };
        // (As find_locals takes the head.)
        let head = match &l.fun {
            Value::Sym(f) => Some((*f, class(*f))),
            Value::Str(v) if v.data.len() == 1 => {
                v.data[0].as_ref().map(|n| (Sym::lookup(n).unwrap_or(s::EMPTY), class_of_name(n)))
            }
            _ => None,
        };
        let Some((v, c)) = head else {
            todo.push(&l.fun);
            todo.extend(l.args.iter().map(|a| &a.value));
            continue;
        };
        if v == s::ASSIGN || v == s::EQ_ASSIGN {
            if assigned_name(l).is_some_and(is) {
                return true;
            }
        } else if v == s::FOR {
            if l.args.first().is_some_and(|a| is(&a.value)) {
                return true;
            }
            todo.extend(l.args.iter().skip(1).map(|a| &a.value));
            continue;
        } else if c == C_ASSIGN_FN || c == C_DELAYED_ASSIGN {
            if let [name, value] = &l.args[..] {
                if let Value::Str(n) = &name.value {
                    if n.data.len() == 1 {
                        if is(&name.value) {
                            return true;
                        }
                        todo.push(&value.value);
                    }
                }
            }
            continue;
        } else if v == s::FUNCTION
            || c == C_TILDE
            || (c == local_index() && shadowed & class_bit(c) == 0 && l.args.len() == 1)
            || ((c == C_EXPRESSION || c == C_QUOTE) && shadowed & class_bit(c) == 0)
        {
            continue;
        }
        todo.extend(l.args.iter().map(|a| &a.value));
    }
    false
}

/// Does code compiled with this head add what [`find_locals`] collects (a
/// wrapper call, a nested unit)? (A cheap overestimate.)
fn compiles_to_more(head: Option<Sym>) -> bool {
    head.is_some_and(|f| {
        let c = class(f);
        f == s::FUNCTION || c == local_index() || (c.wrapping_sub(SI_CLASS) as usize) < SIMPLE_INTERNALS.len()
    })
}

/// Does `e` call a wrapper or make a function or `local()` unit anywhere?
fn has_compiled_parts(e: &Value) -> bool {
    let Value::Lang(l) = e else { return false };
    compiles_to_more(l.fun.as_sym())
        || !matches!(l.fun, Value::Sym(_)) && has_compiled_parts(&l.fun)
        || l.args.iter().any(|a| has_compiled_parts(&a.value))
}

/// What compiling a call of one of the [`SPECIALS`] that is inlined
/// compiles of its arguments.
#[derive(Clone, Copy)]
enum SpArgs {
    /// None: R: cmp.R:cmpSpecial.
    Constant,
    All,
    /// The first (`$`'s object).
    First,
    /// The arguments of the internal's call (`.Internal` of a builtin).
    Inner,
}

/// R: cmp.R: the inline handler of the special with [`sp_bit`] `sp`, by
/// when it punts to cmpSpecial.
fn special_args(l: &Lang, sp: u32) -> SpArgs {
    let dots = |a: &Arg| a.value.as_sym() == Some(s::DOTS);
    let dots_or_missing = || l.args.iter().any(|a| dots(a) || a.value.is_missing_arg());
    let punt_unless = |ok: bool| if ok { SpArgs::All } else { SpArgs::Constant };
    match sp.trailing_zeros() {
        // `$`: a name or a string after the object.
        7 => match &l.args[..] {
            [_, n]
                if !l.args.iter().any(dots)
                    && match &n.value {
                        Value::Sym(_) => true,
                        Value::Str(x) => x.data.len() == 1 && x.data[0].as_deref().is_some_and(|s| !s.is_empty()),
                        _ => false,
                    } =>
            {
                SpArgs::First
            }
            _ => SpArgs::Constant,
        },
        // `return`
        8 => punt_unless(!dots_or_missing() && l.args.len() <= 1),
        // `switch`: at most one default when the cases are named.
        9 => {
            let cases = l.args.get(1..).unwrap_or(&[]);
            let named = cases.iter().any(|a| a.tag.is_some_and(|t| !t.is_empty()));
            let defaults = cases.iter().filter(|a| a.tag.is_none_or(|t| t.is_empty())).count();
            punt_unless(!l.args.is_empty() && !l.args.iter().any(dots) && !(named && defaults > 1))
        }
        // `[`, `[[`: cmpDispatch punts with dots, without arguments or
        // without the object; else it and cmpSubsetDispatch compile them.
        11 | 12 => punt_unless(!l.args.iter().any(dots) && l.args.first().is_some_and(|a| !a.value.is_missing_arg())),
        SP_LOG => punt_unless(
            !dots_or_missing() && !l.args.iter().any(|a| a.tag.is_some()) && (1..=2).contains(&l.args.len()),
        ),
        // `.Internal` of a builtin: cmpBuiltin, or (dots or missing
        // arguments) the generic call, which compiles its argument.
        SP_DOT_INTERNAL => match l.args.first().map(|a| &a.value) {
            Some(Value::Lang(inner)) => match inner.fun.as_sym() {
                Some(f)
                    if crate::builtins::lookup_internal(&f.name()).is_some_and(|b| b.kind == BuiltinKind::Builtin) =>
                {
                    if inner.args.iter().any(|a| dots(a) || a.value.is_missing_arg()) {
                        SpArgs::All
                    } else {
                        SpArgs::Inner
                    }
                }
                Some(_) => SpArgs::Constant,
                None => SpArgs::All,
            },
            _ => SpArgs::All,
        },
        _ => SpArgs::Constant,
    }
}

// R: cmp.R:getAssignedVar: the class of the assigned name (`None`: "bad
// assignment", a compiler error)
#[inline(never)]
fn assigned_var(l: &Lang) -> Option<u8> {
    match assigned_name(l)? {
        Value::Sym(s) if !s.is_empty() => Some(class(*s)),
        Value::Str(x) if x.data.len() == 1 => x.data[0].as_ref().map(|n| class_of_name(n)),
        _ => None,
    }
}

/// The name an assignment call `l` assigns, as written (R:
/// cmp.R:getAssignedVar before its checks).
fn assigned_name(l: &Lang) -> Option<&Value> {
    let mut v = &l.args.first()?.value;
    while let Value::Lang(l) = v {
        v = &l.args.first()?.value;
    }
    Some(v)
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
    inlines_base(interp, s::LOCAL, env)
}

/// Code GNU R runs as bytecode in `env` (the caller checks `Interp::bc`):
/// did its compiler inline base's language function `sym` there? By the
/// frozen set of the closure the JIT compiled, or of installed code (and
/// the closures its code made); a loop in the global environment is
/// compiled as it runs, against the bindings found then.
// R: compiler/R/cmp.R:getInlineInfo (findCenvVar)
#[cold]
#[inline(never)]
pub fn inlines_base(interp: &mut Interp, sym: Sym, env: &Env) -> bool {
    let Some(i) = index(sym) else { return false };
    let Some(ctx) = interp.frames.iter().rev().find(|ctx| ctx.cloenv.ptr_eq(env)) else {
        // A loop in the global environment (do_for and friends).
        return first_bound_in_base(interp, sym, env);
    };
    let Value::Closure(c) = &ctx.function else { return true };
    let v = c.jit.0.get();
    let mask = match v & STATE_MASK {
        COMPILED => v >> 8,
        IGNORE => {
            let c = c.clone();
            installed_mask(interp, &c)
        }
        _ => return true,
    };
    mask >> i & 1 != 0
}

/// Whether the first binding of `sym` from `env` is base's (R: findCenvVar
/// finds a binding of any type; only base's inlines).
fn first_bound_in_base(interp: &Interp, sym: Sym, env: &Env) -> bool {
    let mut cur = env.clone();
    loop {
        if cur.ptr_eq(&interp.base) || cur.ptr_eq(&interp.base_ns) {
            return true;
        }
        if cur.has_local(sym) {
            return false;
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return false,
        }
    }
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
            clo.jit.1.set(interp.toplevel_si);
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
        COMPILED => {
            clo.jit.set(INHERITED, v >> 8);
            clo.jit.1.set(c.jit.1.get());
        }
        // Installed bytecode: its nested closures are compiled with it
        // (MAKECLOSURE), with its frozen set.
        IGNORE => {
            let c = c.clone();
            clo.jit.set(INHERITED, installed_mask(interp, &c));
            clo.jit.1.set(si_decision(interp, &c));
        }
        _ => {}
    }
}

// The simple-wrapper inline decisions. A decision is [`SI_DECIDED`], the
// generation of base's wrapper bindings it was made with
// (`Interp::si_gens`) from bit 32, and a bit per [`SIMPLE_INTERNALS`]
// entry that the code inlines; 0 is "not made yet".
const SI_DECIDED: u64 = 1 << 63;

fn si_pack(mask: u64, generation: u32) -> u64 {
    SI_DECIDED | (generation as u64) << 32 | mask
}

/// The [`SIMPLE_INTERNALS`] bits of a local mask.
fn si_bits(locals: u64) -> u64 {
    locals >> SI_CLASS & SI_ALL
}

/// Decision bits (from bit `BV_SHIFT`: `local`, `expression`, `quote`) for
/// the base variables of the functions the code makes ([`Compiled::base`]).
const BV_SHIFT: u32 = 20;
const BV_WORD: u64 = 7 << BV_SHIFT;
const _: () = assert!(SIMPLE_INTERNALS.len() as u32 <= BV_SHIFT);

fn bv_pack(base: u64) -> u64 {
    ((base >> local_index() & 1) | (base >> 62 & 1) << 1 | (base >> 63) << 2) << BV_SHIFT
}

fn bv_unpack(decision: u64) -> u64 {
    let w = decision >> BV_SHIFT;
    (w & 1) << local_index() | (w >> 1 & 1) << 62 | (w >> 2 & 1) << 63
}

/// R: cmp.R:tryInline for each simple wrapper the compiled code calls
/// ([`Compiled::inlined`]), compiled with `env` above it: getInlineInfo
/// (guardOK = TRUE) finds the name in base (cmpSimpleInternal then inlines
/// base's definition, generation `generation`; a guard checks it at run
/// time unless the code is a namespace's). With the base variables of the
/// functions the code makes.
fn si_decide(interp: &Interp, env: &Env, compiled: &Compiled, generation: u32) -> u64 {
    let mut mask = 0;
    let mut todo = compiled.inlined;
    while todo != 0 {
        let i = todo.trailing_zeros() as usize;
        todo &= todo - 1;
        if Sym::lookup(SIMPLE_INTERNALS[i]).is_some_and(|sym| found_in_base(interp, sym, env).is_some()) {
            mask |= 1 << i;
        }
    }
    si_pack(mask, generation) | bv_pack(compiled.base)
}

/// R: cmp.R:cmpSimpleInternal's findFunDef, as the code is compiled: each
/// definition `decision` inlines is got from base, which forces a lazy
/// binding then. A binding that is a builtin is not inlined
/// (inlineSimpleInternalCall); one that is no function, or whose forcing
/// fails, is a compiler error (checkCall's NA in `if`): `None`.
fn si_resolve(interp: &mut Interp, decision: u64) -> Option<u64> {
    let generation = (decision >> 32 & 0x7fff_ffff) as usize;
    let mut d = decision;
    let mut todo = decision & SI_ALL;
    while todo != 0 {
        let i = todo.trailing_zeros() as usize;
        todo &= todo - 1;
        let Some(raw) = interp.si_gens.get(generation).and_then(|g| g[i].clone()) else {
            d &= !(1 << i);
            continue;
        };
        // Base's shipped binding (generation 0: a lazy-load promise of the
        // wrapper's closure) is forced when first needed instead: no R
        // code sees when that happens, and forcing every wrapper the
        // base code calls as it compiles at startup is not free.
        if interp.si_gens[0][i].as_ref().is_some_and(|b| b.ptr_eq(&raw)) {
            continue;
        }
        let v = match raw {
            Value::Promise(_) => interp.force(raw).ok()?,
            v => v,
        };
        match v {
            Value::Closure(_) => {}
            Value::Builtin(_) => d &= !(1 << i),
            _ => return None,
        }
    }
    Some(d)
}

/// The generation of base's wrapper bindings now (R: the definitions
/// findFunDef would find): a new one when one of them has been rebound
/// since the last.
fn si_generation(interp: &mut Interp) -> u32 {
    let key = (interp.base.fn_version(), interp.base_ns.fn_version());
    let last = interp.si_gens.len() - 1;
    if key == interp.si_gen_key {
        return last as u32;
    }
    interp.si_gen_key = key;
    let now = si_bindings(interp);
    let same = now.iter().zip(&interp.si_gens[last]).all(|(a, b)| match (a, b) {
        (Some(a), Some(b)) => a.ptr_eq(b),
        (None, None) => true,
        _ => false,
    });
    if !same {
        interp.si_gens.push(now);
    }
    (interp.si_gens.len() - 1) as u32
}

/// Base's bindings of the [`SIMPLE_INTERNALS`], raw: lazy-load promises
/// stay unforced (forcing them at startup would perturb the heap for the
/// whole run).
fn si_bindings(interp: &Interp) -> [Option<Value>; 20] {
    std::array::from_fn(|i| Sym::lookup(SIMPLE_INTERNALS[i]).and_then(|sym| interp.base.get_local(sym)))
}

/// Base has loaded: the shipped wrapper definitions are generation 0, the
/// ones installed code was compiled with.
pub fn si_snapshot(interp: &mut Interp) {
    interp.si_gens = vec![si_bindings(interp)];
    interp.si_gen_key = (interp.base.fn_version(), interp.base_ns.fn_version());
    interp.si_ready = true;
}

/// The simple-wrapper decision of the bytecode closure `clo`: made when
/// it was compiled (or inherited from its creator, or the JIT cache), or
/// now for installed code (compiled with the shipped definitions) and
/// code compiled before base had loaded.
#[inline(never)]
pub fn si_decision(interp: &mut Interp, clo: &Closure) -> u64 {
    let v = clo.jit.1.get();
    if v != 0 || !interp.si_ready {
        return v;
    }
    let st = clo.jit.0.get();
    if st & (STATE_MASK | MASK_PENDING) == IGNORE | MASK_PENDING {
        // (Made with its frozen set.)
        installed_mask(interp, clo);
        let v = clo.jit.1.get();
        if v != 0 {
            return v;
        }
    }
    let installed = st & STATE_MASK == IGNORE || st & LOADED_BYTECODE != 0;
    let generation = if installed { 0 } else { si_generation(interp) };
    let d = match compile_pass(interp, &clo.formals, &clo.body, &clo.env) {
        Some(compiled) => si_decide(interp, &clo.env, &compiled, generation),
        None => si_pack(0, generation),
    };
    clo.jit.1.set(d);
    d
}

/// A top-level loop starts (R: eval.c:do_for and friends compile it then,
/// R_compileAndExecute): its decision, with its locals added to the global
/// frame (cmp.R:compile), and the definitions it inlines found. (A
/// compiler error inlines nothing: GNU R runs the loop interpreted.)
pub fn toplevel_si_decision(interp: &mut Interp, call: &Value) -> u64 {
    let global = interp.global.clone();
    let generation = si_generation(interp);
    let none = si_pack(0, generation);
    match compile_pass(interp, &[], call, &global) {
        Some(compiled) => {
            let d = si_decide(interp, &global, &compiled, generation);
            si_resolve(interp, d).unwrap_or(none)
        }
        None => none,
    }
}

/// The definition `decision` inlines for [`SIMPLE_INTERNALS`]`[si]`, if
/// any: base's binding when it was made, forced now (where the generic
/// call would force it; a failure stays generic and reproduces it).
pub fn si_definition(interp: &mut Interp, decision: u64, si: usize) -> Option<Rc<Closure>> {
    if decision >> si & 1 == 0 {
        return None;
    }
    let generation = (decision >> 32 & 0x7fff_ffff) as usize;
    let raw = interp.si_gens.get(generation)?[si].clone()?;
    match interp.force(raw).ok()? {
        Value::Closure(d) => Some(d),
        _ => None,
    }
}

/// The frame `env` of a call of `clo` ended, and something still holds
/// it: a promise its code made may be forced later, as the bytecode it
/// was compiled as, with `clo`'s decision ([`detached_si_decision`]).
#[cold]
#[inline(never)]
pub fn si_frame_escaped(interp: &mut Interp, clo: &Closure, env: &Env) {
    if !interp.si_ready || clo.jit.interpreted() {
        return;
    }
    let d = si_decision(interp, clo);
    if d & SI_ALL == 0 {
        return;
    }
    if interp.si_detached.len() >= interp.si_detached_cap {
        si_sweep_detached(interp);
    }
    interp.si_detached.insert(env.id(), (Rc::downgrade(&env.0), d));
}

/// Forget the decisions of frames no longer alive.
pub fn si_sweep_detached(interp: &mut Interp) {
    interp.si_detached.retain(|_, (w, _)| w.strong_count() > 0);
    interp.si_detached_cap = (2 * interp.si_detached.len()).max(256);
}

/// The decision of the code that made a promise running in `env`, a frame
/// whose call has ended (0: none recorded, so nothing inlines).
pub fn detached_si_decision(interp: &Interp, env: &Env) -> u64 {
    interp.si_detached.get(&env.id()).map_or(0, |e| e.1)
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
        for (i, n) in super::SIMPLE_INTERNALS.iter().enumerate() {
            assert_eq!(super::class_of_name(n), super::SI_CLASS + i as u8, "{n}");
            assert_eq!(super::class_bit(super::class_of_name(n)) >> super::SI_CLASS, 1 << i, "{n}");
        }
        for (i, n) in super::SPECIALS.iter().enumerate() {
            assert_eq!(super::sp_bit(super::class_of_name(n)), 1 << i, "{n}");
        }
        assert_eq!(super::SPECIALS[super::SP_IF as usize], "if");
        assert_eq!(
            super::SP_SUBSET,
            super::sp_bit(super::class_of_name("[")) | super::sp_bit(super::class_of_name("[["))
        );
        assert_eq!(super::class_of_name("T"), super::C_CONST_NAME);
        assert_eq!(super::SPECIALS[super::SP_LOG as usize], "log");
        assert_eq!(super::SPECIALS[super::SP_DOT_INTERNAL as usize], ".Internal");
        assert_eq!(super::class_of_name("~"), super::C_TILDE);
        assert_eq!(super::class_of_name("x"), super::C_OTHER);
        assert_eq!(super::FROZEN_FUNS[super::C_LOCAL as usize], "local");
        // (The memo stores a class plus one in a byte.)
        const { assert!(super::C_OTHER < u8::MAX) };
    }
}
