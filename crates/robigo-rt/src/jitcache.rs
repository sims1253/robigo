//! GNU R's JIT cache and `R_cmpfun` (eval.c:1612-1909): compiling a
//! closure the JIT decided to compile reuses the code of an identical body
//! compiled for a compatible environment, so closures that share a body
//! (one `function` evaluated again, a file sourced twice) share its code
//! and the decisions cmp.R made for it.
//!
//! The cache is GNU R's: `JIT_CACHE_SIZE` entries, direct mapped by a hash
//! of the body (`hashfun`). An entry holds the body, the compiled code, and
//! a snapshot of its compile environment (`make_cached_cmpenv`: the top
//! environment and the names of the formals and of every binding between
//! the closure's environment and the top). Code is reused for a closure
//! whose top environment is the entry's and whose formals and frame
//! bindings are all among the entry's names (`jit_env_match`), when the
//! bodies are identical (`jit_expr_match`) and, for a body without a
//! srcref, so are the closures' srcrefs. A closure whose environment does
//! not match an entry of its hash is marked `NOJIT`.
//!
//! The hash is GNU R's walk (`hashexpr1`: types, lengths, scalar values by
//! value) except that what GNU R hashes by address (symbols, longer
//! vectors) is hashed by symbol id or not at all: an address is not stable
//! under a moving collection, and GNU R's addresses are its own, so which
//! bodies share a slot (and evict each other) can differ from GNU R's.
//!
//! The cache's objects hang off one VECSXP the isolate roots (GNU R's
//! `R_PreserveObject(JIT_cache)`).

#![allow(unsafe_code)]

use crate::jit;
use crate::prims;
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp};
use robigo_rtcore::{Cx, Global, RResult, Ref, Root};

/// `JIT_CACHE_SIZE`.
pub const JIT_CACHE_SIZE: usize = 1024;

// An entry's elements.
const E_BODY: usize = 0;
const E_CODE: usize = 1;
const E_TOP: usize = 2;
/// A VECSXP of the snapshot's names, or R_NilValue when the snapshot is
/// the top environment itself (`make_cached_cmpenv`'s `cmpenv == top`).
const E_LOCALS: usize = 3;
const E_SRCREF: usize = 4;
const E_LEN: usize = 5;

/// The cache: the entries' vector and their hashes.
#[derive(Default)]
pub struct Cache {
    entries: Option<Global>,
    hashes: Vec<u64>,
}

impl Cache {
    /// The isolate's cache moved out of a state being replaced.
    pub fn take(&mut self) -> Cache {
        std::mem::take(self)
    }
}

// ---- hashing -----------------------------------------------------------------

// R: eval.c:hash (djb2 over the value's bytes)
fn djb2(bytes: &[u8], mut h: u64) -> u64 {
    for &b in bytes {
        h = (h << 5).wrapping_add(h).wrapping_add(b as u64);
    }
    h
}

/// `hashexpr1(e, h)`.
///
/// # Safety
/// `e` is live.
// R: eval.c:hashexpr1
unsafe fn hash_expr(cx: &Cx, e: Sexp, h: u64) -> u64 {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let t = e.typ();
        let len: i32 = match t {
            NILSXP => 0,
            LANGSXP | LISTSXP | DOTSXP => {
                let mut n = 0;
                let mut a = e;
                while a != nil && matches!(a.typ(), LANGSXP | LISTSXP | DOTSXP) {
                    n += 1;
                    a = a.cdr();
                }
                n
            }
            _ if robigo_heap::is_vector_type(t) && !e.is_altrep() => e.std_len() as i32,
            _ => 1,
        };
        let mut h = djb2(&(t as i32).to_le_bytes(), h);
        h = djb2(&len.to_le_bytes(), h);
        match t {
            LANGSXP | LISTSXP => {
                let mut a = e;
                while a != nil && matches!(a.typ(), LANGSXP | LISTSXP | DOTSXP) {
                    h = hash_expr(cx, a.car(), h);
                    a = a.cdr();
                }
                h
            }
            LGLSXP | INTSXP if len == 1 && !e.is_altrep() => djb2(&(*(e.std_data() as *const i32)).to_le_bytes(), h),
            REALSXP if len == 1 && !e.is_altrep() => djb2(&(*(e.std_data() as *const f64)).to_le_bytes(), h),
            STRSXP if len == 1 && !e.is_altrep() => djb2((*(e.std_data() as *const Sexp)).char_bytes(), h),
            // (GNU R: the address; a symbol's id is stable here)
            SYMSXP => djb2(&(e.sym_id() as u64).to_le_bytes(), h),
            _ => h,
        }
    }
}

/// `hashfun(f)`: the body, then the closure's srcref when the body has none.
///
/// # Safety
/// `f` is a live closure.
// R: eval.c:hashfun, hashsrcref
unsafe fn hashfun(cx: &Cx, f: Sexp) -> u64 {
    // SAFETY: the caller's contract.
    unsafe {
        let body = f.cdr();
        let mut h = hash_expr(cx, body, 5381);
        if srcref(cx, body).is_none()
            && let Some(sr) = srcref(cx, f)
            && sr.typ() == INTSXP
            && sr.std_len() >= 6
        {
            for i in 0..6 {
                h = djb2(&(*(sr.std_data() as *const i32).add(i)).to_le_bytes(), h);
            }
        }
        h
    }
}

/// The `srcref` attribute of `x`, if any.
///
/// # Safety
/// `x` is live.
unsafe fn srcref(cx: &Cx, x: Sexp) -> Option<Sexp> {
    let nil = cx.nil().sexp();
    let sym = prims::state(cx).syms.srcref;
    // SAFETY: the caller's contract: the attribute pairlist is live.
    unsafe {
        let mut a = x.attrib_raw();
        while a != nil && a.typ() == LISTSXP {
            if a.tag() == sym {
                return Some(a.car());
            }
            a = a.cdr();
        }
    }
    None
}

// ---- the compile environment's snapshot ----------------------------------------

/// The names of the bindings of frame `env` below the top environment, or
/// None for a frame GNU R reads through code (a user database).
///
/// # Safety
/// `env` is a live environment.
unsafe fn frame_names(cx: &Cx, env: Sexp) -> Option<Vec<Sexp>> {
    // SAFETY: the caller's contract.
    let e = crate::dispatch::env_ref(unsafe { Ref::from_sexp(env, cx) })?;
    Some(cx.frame_bindings(e, true)?.into_iter().map(|(s, _)| s.as_ref().sexp()).collect())
}

/// Whether `env` is a standard unhashed frame (`IS_STANDARD_UNHASHED_FRAME`).
///
/// # Safety
/// `env` is a live environment.
unsafe fn unhashed(cx: &Cx, env: Sexp) -> bool {
    // SAFETY: the caller's contract.
    match crate::dispatch::env_ref(unsafe { Ref::from_sexp(env, cx) }) {
        Some(e) => cx.hash_profile(e).is_none(),
        None => false,
    }
}

/// `make_cached_cmpenv(fun)`: the top environment and the names (None
/// when the snapshot is the top environment itself).
///
/// # Safety
/// `f` is a live closure.
// R: eval.c:make_cached_cmpenv
unsafe fn cached_cmpenv(cx: &Cx, f: Sexp) -> (Sexp, Option<Vec<Sexp>>) {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let cmpenv = f.tag();
        let top = crate::dispatch::topenv(cx, None, Ref::from_sexp(cmpenv, cx)).sexp();
        let formals = f.car();
        if cmpenv == top && formals == nil {
            return (top, None);
        }
        let mut names: Vec<Sexp> = crate::cenv::elements(cx, formals, false).iter().map(|a| a.1).collect();
        let mut env = cmpenv;
        while env != top && env.typ() == ENVSXP {
            // (hashed frames are listed too; a frame of unsupported type
            // makes the top environment the conservative answer)
            match frame_names(cx, env) {
                Some(v) => names.extend(v),
                None => return (top, None),
            }
            env = robigo_heap::env::enclos(env);
        }
        (top, Some(names))
    }
}

/// `jit_env_match(cmpenv, fun)`: can code compiled for the entry's
/// snapshot run as `f`'s?
///
/// # Safety
/// `f` is a live closure, `entry` a live cache entry.
// R: eval.c:jit_env_match, cmpenv_exists_local
unsafe fn env_match(cx: &Cx, entry: Sexp, f: Sexp) -> bool {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let el = |i: usize| *(entry.std_data() as *const Sexp).add(i);
        let locals = el(E_LOCALS);
        let has = |s: Sexp| {
            locals != nil && (0..locals.std_len() as usize).any(|i| *(locals.std_data() as *const Sexp).add(i) == s)
        };
        let mut env = f.tag();
        let top = crate::dispatch::topenv(cx, None, Ref::from_sexp(env, cx)).sexp();
        if top != el(E_TOP) {
            return false;
        }
        if crate::cenv::elements(cx, f.car(), false).iter().any(|a| !has(a.1)) {
            return false;
        }
        while env != top && env.typ() == ENVSXP {
            if !unhashed(cx, env) {
                return false;
            }
            match frame_names(cx, env) {
                Some(v) if v.iter().all(|&s| has(s)) => {}
                _ => return false,
            }
            env = robigo_heap::env::enclos(env);
        }
        true
    }
}

// ---- R_cmpfun --------------------------------------------------------------------

/// `R_cmpfun(fun)`: closure `op` (rooted) is compiled, in place: its body
/// becomes byte code (the cached code of a matching entry, or compiled
/// now by the `Bc` profile, then cached); a closure the compiler leaves
/// uncompiled, or whose environment does not match its hash's entry, is
/// marked `NOJIT`.
// R: eval.c:R_cmpfun
pub fn cmpfun(cx: &mut Cx, op: Root<'_>, strategy: i32) -> RResult<()> {
    let cached = strategy != jit::STRATEGY_NO_CACHE;
    // SAFETY: a rooted closure.
    let hash = cached.then(|| unsafe { hashfun(cx, op.bind(cx).sexp()) });
    if let Some(h) = hash {
        match lookup(cx, op, h) {
            Hit::Code(code) => {
                // SAFETY: a rooted closure and a cache entry's code object.
                unsafe { jit::set_compiled(op.bind(cx).sexp(), code) };
                return Ok(());
            }
            Hit::Snapshot(entry) => {
                // the cached code's decisions for this closure's formals
                let code = cx.scope(|cx, s| {
                    // SAFETY: an entry of the rooted cache.
                    let e = s.root(cx, unsafe { Ref::from_sexp(entry, cx) });
                    crate::compile::compile_closure_bc(cx, op, Some(e))
                })?;
                if let Some(code) = code {
                    // SAFETY: as above.
                    unsafe { jit::set_compiled(op.bind(cx).sexp(), code) };
                } else {
                    set_nojit(cx, op);
                }
                return Ok(());
            }
            Hit::EnvMismatch => {
                set_nojit(cx, op);
                return Ok(());
            }
            Hit::None => {}
        }
    }
    let Some(code) = crate::compile::compile_closure_bc(cx, op, None)? else {
        set_nojit(cx, op);
        return Ok(());
    };
    // SAFETY: a rooted closure; a fresh code object, kept by the closure.
    unsafe { jit::set_compiled(op.bind(cx).sexp(), code) };
    if let Some(h) = hash {
        enter(cx, op, h)?;
    }
    Ok(())
}

fn set_nojit(cx: &Cx, op: Root<'_>) {
    let f = op.bind(cx).sexp();
    // SAFETY: a rooted closure.
    unsafe { f.set_gp(f.gp() | jit::NOJIT_MASK) };
}

/// What the cache has for a closure.
enum Hit {
    None,
    /// The entry's code object, compiled for the same formals.
    Code(Sexp),
    /// A matching entry whose code was compiled for other formals: compile
    /// for this closure's against the entry's snapshot.
    Snapshot(Sexp),
    /// An entry of this hash whose environment does not match.
    EnvMismatch,
}

/// The cache's answer for closure `op` with hash `h`
/// (`get_jit_cache_entry`, then the matches).
fn lookup(cx: &mut Cx, op: Root<'_>, h: u64) -> Hit {
    let st = prims::state(cx);
    let cache = st.jit_cache.borrow();
    let slot = (h % JIT_CACHE_SIZE as u64) as usize;
    let Some(entries) = &cache.entries else { return Hit::None };
    if cache.hashes[slot] != h {
        return Hit::None;
    }
    let entries = entries.bind(cx).sexp();
    let f = op.bind(cx).sexp();
    // SAFETY: the rooted cache vector and its live entries; a rooted
    // closure.
    unsafe {
        let entry = *(entries.std_data() as *const Sexp).add(slot);
        if entry.typ() != VECSXP {
            return Hit::None;
        }
        if !env_match(cx, entry, f) {
            return Hit::EnvMismatch;
        }
        let el = |i: usize| *(entry.std_data() as *const Sexp).add(i);
        let body = el(E_BODY);
        let same = robigo_rtbuiltins::identical::compute_identical(
            cx,
            Ref::from_sexp(body, cx),
            Ref::from_sexp(f.cdr(), cx),
            robigo_rtbuiltins::identical::USE_CLOENV,
        )
        .unwrap_or(false);
        if !same {
            return Hit::None;
        }
        // "if function body has a srcref, all srcrefs compiled in that
        // function only depend on the body srcref"
        if srcref(cx, f.cdr()).is_none() {
            let nil = cx.nil().sexp();
            let a = el(E_SRCREF);
            let b = srcref(cx, f).unwrap_or(nil);
            let same =
                robigo_rtbuiltins::identical::compute_identical(cx, Ref::from_sexp(a, cx), Ref::from_sexp(b, cx), 0)
                    .unwrap_or(false);
            if !same {
                // (re-compiled and re-cached)
                return Hit::None;
            }
        }
        let code = el(E_CODE);
        if crate::code::is_closure_code(code, crate::ir::Profile::Bc, f.car()) {
            Hit::Code(code)
        } else {
            Hit::Snapshot(entry)
        }
    }
}

/// `set_jit_cache_entry(hash, fun)` for the compiled closure `op`.
// R: eval.c:set_jit_cache_entry
fn enter(cx: &mut Cx, op: Root<'_>, h: u64) -> RResult<()> {
    if prims::state(cx).jit_cache.borrow().entries.is_none() {
        let v = crate::vm::alloc_retry(cx, |cx| cx.alloc_vector_raw(VECSXP, JIT_CACHE_SIZE))?;
        // SAFETY: fresh.
        let g = cx.global(unsafe { Ref::from_sexp(v, cx) });
        let st = prims::state(cx);
        let mut c = st.jit_cache.borrow_mut();
        c.entries = Some(g);
        c.hashes = vec![0; JIT_CACHE_SIZE];
    }
    let entry = crate::vm::alloc_retry(cx, |cx| {
        // SAFETY: the rooted closure and its compiled code; the snapshot's
        // names are symbols; nothing collects in this interval.
        unsafe { make_entry(cx, op.bind(cx).sexp()) }
    })?;
    let st = prims::state(cx);
    let mut c = st.jit_cache.borrow_mut();
    let slot = (h % JIT_CACHE_SIZE as u64) as usize;
    let v = c.entries.as_ref().unwrap().bind(cx).sexp();
    // SAFETY: the rooted cache vector and a fresh entry: the store counts
    // and takes the barrier.
    unsafe { v.set_elt(slot, entry) };
    c.hashes[slot] = h;
    Ok(())
}

/// A cache entry for the compiled closure `f`.
///
/// # Safety
/// `f` is a live compiled closure, reachable from a root.
unsafe fn make_entry(cx: &Cx, f: Sexp) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let (top, names) = cached_cmpenv(cx, f);
        let locals = match names {
            Some(n) => {
                let v = cx.alloc_vector_raw(VECSXP, n.len())?;
                for (i, s) in n.into_iter().enumerate() {
                    v.set_elt(i, s);
                }
                v
            }
            None => nil,
        };
        let e = cx.alloc_vector_raw(VECSXP, E_LEN)?;
        e.set_elt(E_BODY, f.cdr());
        e.set_elt(E_CODE, f.aux());
        e.set_elt(E_TOP, top);
        e.set_elt(E_LOCALS, locals);
        e.set_elt(E_SRCREF, srcref(cx, f).unwrap_or(nil));
        Ok(e)
    }
}

/// An entry's snapshot as a compile environment's names: the top
/// environment and the names (empty when the snapshot is the top).
///
/// # Safety
/// `entry` is a live cache entry.
pub unsafe fn entry_snapshot(cx: &Cx, entry: Sexp) -> (Sexp, Vec<Sexp>) {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let el = |i: usize| *(entry.std_data() as *const Sexp).add(i);
        let l = el(E_LOCALS);
        let names = if l == nil {
            Vec::new()
        } else {
            (0..l.std_len() as usize).map(|i| *(l.std_data() as *const Sexp).add(i)).collect()
        };
        (el(E_TOP), names)
    }
}
