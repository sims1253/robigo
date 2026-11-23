//! The compiler from LANGSXP to the IR (docs/g3.md §9.1-9.2).
//!
//! # The `Eval` profile
//!
//! GNU R's AST interpreter looks a call's function up every time it
//! evaluates the call (`findFun3`), so a user's `if` or `+` replaces base's
//! from then on. The compiler inlines a base primitive (a language special
//! as VM code, an arithmetic builtin as its scalar kernel) behind a
//! [`Op::Guard`] that checks, without running code, that the symbol still
//! names that primitive from `rho`; when it does not, the guard jumps to
//! the generic call of the same expression, which does what `eval` does.
//! Everything else is a generic call.
//!
//! Each instruction that stands for one of GNU R's `eval` calls is an
//! evaluation point (`F_TICK`, `d`): the root of the compiled expression is
//! at depth 1, and an expression evaluated by the code of a node at depth
//! `d` (a special's operand, a builtin's argument) is at `d + 1`, as
//! `eval` nests.
//!
//! The compiler runs under `&Cx`: it allocates nothing but symbols and the
//! final code object, and every object it puts in the pool is reachable
//! from the source expression, immortal, or a primitive rooted by the
//! isolate. A refused allocation is retried after a collection by
//! compiling again ([`compile`]).

#![allow(unsafe_code)]

use crate::cenv::{self, Cenv, Need};
use crate::code::{Built, FIRST_CONST};
use crate::ir::*;
use crate::prims::{self, Inline, Kind, RtState, Special};
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, RResult, Ref, Root};
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;

/// The hasher of the translator's maps ([`PosMap`]), whose keys
/// are fixed-width integers it makes itself (object addresses, positions),
/// never chosen by R code: rustc-hash's multiply-rotate, not the standard
/// library's SipHash, which was about a third of translating a closure at
/// its first call (about 180k instructions each in the rdb boot's profile).
/// Not collision-resistant, so only for keys no program supplies: a byte
/// string reaching [`std::hash::Hasher::write`] here is a debug-assertion
/// failure (strings of compiled-in names go in a [`NameSet`]).
#[derive(Default, Clone, Copy)]
pub(crate) struct PosHasher(u64);

/// The multiply of [`PosHasher`] and [`NameHasher`].
#[inline]
fn mix(h: u64, n: u64) -> u64 {
    (h ^ n).wrapping_mul(0xf135_7aea_2e62_a9c5)
}

/// `bytes` into `h`, eight at a time, then the tail (zero-padded).
#[inline]
fn mix_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        h = mix(h, u64::from_le_bytes(c.try_into().expect("an 8-byte chunk")));
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        let mut b = [0u8; 8];
        b[..tail.len()].copy_from_slice(tail);
        h = mix(h, u64::from_le_bytes(b));
    }
    h
}

impl std::hash::Hasher for PosHasher {
    #[inline]
    fn finish(&self) -> u64 {
        // (the low bits pick the bucket: the rotate brings the multiply's
        // well-mixed high bits down, as rustc-hash 2 does)
        self.0.rotate_left(26)
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        debug_assert!(false, "robigo: a byte-string key in a PosMap (integer keys only; a NameSet for names)");
        self.0 = mix_bytes(self.0, bytes);
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0 = mix(self.0, n);
    }
    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.write_u64(n as u64);
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    #[inline]
    fn write_u16(&mut self, n: u16) {
        self.write_u64(n as u64);
    }
    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }
}

/// A map of the translator's ([`PosHasher`]): fixed-width integer keys only.
pub(crate) type PosMap<K, V> = HashMap<K, V, BuildHasherDefault<PosHasher>>;

/// A compiled-in name (names.c's table, a builtin's `PrimDef::name`): a
/// `&'static str` of the binary, never a string a program supplies, so a
/// fast hash that is not collision-resistant serves it ([`NameSet`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Name(pub &'static str);

impl std::hash::Hash for Name {
    // (the bytes in one write, the length folded in first, not `str`'s
    // write and its 0xff terminator: one multiply fewer per name; equal
    // names hash equal, which is all a set needs)
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        h.write_usize(self.0.len());
        h.write(self.0.as_bytes());
    }
}

/// The hasher of a [`NameSet`]: [`PosHasher`]'s multiply over the name's
/// bytes, eight per multiply.
#[derive(Default, Clone, Copy)]
pub(crate) struct NameHasher(u64);

impl std::hash::Hasher for NameHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0.rotate_left(26)
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0 = mix_bytes(self.0, bytes);
    }
    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.0 = mix(self.0, n as u64);
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0 = mix(self.0, n as u64);
    }
}

/// A set of compiled-in names ([`Name`]) with a flag, hashed fast
/// ([`NameHasher`]): `base::stubs_for`'s defined builtins.
pub(crate) type NameSet = HashSet<(Name, bool), BuildHasherDefault<NameHasher>>;

mod bctrans;
pub(crate) mod crosscheck;
#[doc(hidden)]
pub use crosscheck::Verdict;

/// Run one compile attempt under `&Cx` until it succeeds: a refused
/// allocation collects and compiles again (the same request refused twice
/// is an error), a lazy binding a decision needs is forced first
/// ([`Need::Force`], with `R_Visible` kept as `R_cmpfun1` keeps it), the
/// constant folds a try asked for are evaluated and it compiles again with
/// their values ([`crate::fold`]). None when cmp.R would leave the code
/// uncompiled ([`Need::Fail`]).
fn attempt(cx: &mut Cx, mut f: impl FnMut(&Cx) -> Result<Sexp, Need>) -> RResult<Option<Sexp>> {
    let mut round = crate::fold::Round::default();
    let r = attempt_rounds(cx, &mut round, &mut f);
    round.release(cx);
    r
}

fn attempt_rounds(
    cx: &mut Cx,
    round: &mut crate::fold::Round,
    f: &mut impl FnMut(&Cx) -> Result<Sexp, Need>,
) -> RResult<Option<Sexp>> {
    let mut retried = None;
    loop {
        round.begin(cx);
        let r = f(cx);
        let reqs = crate::fold::Round::end(cx);
        match r {
            // a fold may drop the code that failed (`if (FALSE)`)
            Ok(_) | Err(Need::Fail) if !reqs.is_empty() => {
                round.resolve(cx, reqs)?;
                retried = None;
            }
            Ok(code) => return Ok(Some(code)),
            Err(Need::Fail) => return Ok(None),
            Err(Need::Force(p)) => {
                let vis = cx.visible();
                cx.scope(|cx, s| {
                    // SAFETY: the promise is reachable from a binding (or a
                    // root) and nothing collected since it was found.
                    let p = s.root(cx, unsafe { robigo_rtcore::Ref::from_sexp(p, cx) });
                    let o = s.out(cx);
                    cx.force(p, o)
                })?;
                cx.set_visible(vis);
            }
            Err(Need::Gc(e)) if retried == Some(e) => return Err(cx.refused_again(e)),
            Err(Need::Gc(e)) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
        }
    }
}

/// Compile `expr` for `profile`: a code object, or GNU R's allocation
/// error. Under `Bc` the code is a promise's compiled code made in `rho`
/// (or code `bcEval` runs there): cmp.R's decisions for that environment
/// ([`Cenv::of_promise`]); where cmp.R would not compile it, the `Eval`
/// profile's code. `expr` and `rho` are read through their roots on every
/// attempt (a collection may have moved them).
pub fn compile(cx: &mut Cx, expr: Root<'_>, rho: Root<'_>, profile: Profile) -> RResult<Sexp> {
    if profile == Profile::Bc {
        let r = attempt(cx, |cx| {
            let (e, env) = (expr.bind(cx).sexp(), rho.bind(cx).sexp());
            // SAFETY: rooted objects of `cx`'s isolate; nothing collects in
            // this interval.
            unsafe {
                let cenv = Cenv::of_promise(cx, env)?;
                let st = prims::state(cx);
                let mut c = Comp::new(cx, st, Profile::Bc, Some(cenv));
                let b = c.root(e)?;
                let code = c.build(b, e, None)?;
                // the code runs in `env`'s frames: their shape's slot
                // indices, for it and the arguments compiled with it
                if env.typ() == ENVSXP && env.has_flag(robigo_heap::flags::SLOTS) {
                    let shape = env.sexp_at(robigo_heap::slots::OFF_SHAPE);
                    if shape.typ() == VECSXP {
                        crate::code::set_index_shape(cx, code, shape);
                        for &a in &c.arg_codes {
                            crate::code::set_index_shape(cx, a, shape);
                        }
                    }
                }
                Ok(code)
            }
        })?;
        if let Some(code) = r {
            return Ok(code);
        }
    }
    attempt(cx, |cx| {
        let e = expr.bind(cx).sexp();
        // SAFETY: as above.
        unsafe {
            let st = prims::state(cx);
            let mut c = Comp::new(cx, st, Profile::Eval, None);
            let b = c.root(e)?;
            Ok(c.build(b, e, None)?)
        }
    })
    .map(|c| c.expect("robigo: the Eval profile always compiles"))
}

/// Compile closure `op`'s body for its formals under the `Eval` profile:
/// the code object carries the frame shape (the formals, then the locals
/// the body assigns).
///
/// # Safety
/// `op` roots a live closure of `cx`'s isolate.
pub unsafe fn compile_closure(cx: &mut Cx, op: Root<'_>) -> RResult<Sexp> {
    attempt(cx, |cx| {
        // SAFETY: the caller's contract; nothing collects in this interval.
        unsafe {
            let f = op.bind(cx).sexp();
            let (formals, body) = (f.car(), source_body(cx, f));
            let st = prims::state(cx);
            let mut c = Comp::new(cx, st, Profile::Eval, None);
            let b = c.root(body)?;
            Ok(c.build(b, body, Some(formals))?)
        }
    })
    .map(|c| c.expect("robigo: the Eval profile always compiles"))
}

/// The expression closure `f`'s body runs under the `Eval` profile: its
/// body, or for a BCODESXP body its first constant, the source
/// (`bytecodeExpr`): what bcEval falls back to when it does not run the
/// code (`R_disable_bytecode`, a version it does not know). The `Bc`
/// profile translates the byte code ([`translate_code`]).
///
/// # Safety
/// `f` is a live closure.
// R: eval.c:bcEval (its fallback), bytecodeExpr
unsafe fn source_body(cx: &Cx, f: Sexp) -> Sexp {
    // SAFETY: the caller's contract.
    let b = unsafe { f.cdr() };
    // SAFETY: a live object of `cx`'s heap.
    robigo_rtbuiltins::bytecode::expr(cx, unsafe { Ref::from_sexp(b, cx) }).sexp()
}

/// `cmpfun(f)` for closure `op` (rooted) under the `Bc` profile: its body
/// compiled with cmp.R's decisions for its environment as it is now;
/// `installed`: base's installed byte code (base's shipped definitions).
/// None when cmp.R returns the closure uncompiled (it may call
/// `browser()`, a compiler error).
// R: cmp.R:cmpfun, make.functionContext, funEnv
pub(crate) fn compile_closure_bc(cx: &mut Cx, op: Root<'_>, installed: bool) -> RResult<Option<Sexp>> {
    // (the crosscheck's finding of the last attempt: an attempt a fold
    // retries compared code without the fold's value)
    let mut check = None;
    // A byte-code body's translation is kept on the BCODESXP, and every
    // closure `function` makes from it shares it, as GNU R's MKCLOSURE
    // shares the body. It depends on the byte code, its constant pool and
    // the frame shape of the formals only: the translation runs with no
    // compiler environment (`Cenv::for_translation`), so it never reads the
    // closure's environment, a binding's value, an option or the JIT state.
    // A write into the body's expression (its first constant, R_ClosureExpr
    // of every closure made from it) does not change it, as it does not
    // change GNU R's byte code; a write into the formals makes it stale (the
    // code stamps: `is_closure_code`), and other formals miss. Looked up
    // before `attempt`, kept after it: a
    // fold's retry must not find the translation of an attempt it retries.
    // SAFETY: a rooted closure; its body's 4th word is R_NilValue or a code
    // object.
    unsafe {
        let f = op.bind(cx).sexp();
        if f.cdr().typ() == BCODESXP
            && let Some(c) = crate::code::cached(cx, f.cdr().aux(), Profile::Bc)
            && crate::code::is_closure_code(cx, c, Profile::Bc, f.car())
        {
            return Ok(Some(c));
        }
    }
    // (findLocals' warnings raised, and the compile made again after them)
    let mut prelude = Prelude::default();
    let code = loop {
        let code = attempt(cx, |cx| {
            let f = op.bind(cx).sexp();
            // (a retried attempt gives cmp.R's warnings again)
            prims::state(cx).compile_warnings.clear();
            // SAFETY: a rooted closure (and cache entry); nothing collects in
            // this interval.
            unsafe {
                if f.cdr().typ() == BCODESXP {
                    if crosscheck::enabled() {
                        check = Some(crosscheck_closure(cx, f, installed)?);
                    }
                    // V3: GNU R's byte code, translated
                    return translate_code(cx, f.cdr(), Some(f.car()), None);
                }
                let (formals, body) = (f.car(), source_body(cx, f));
                let mut cenv = Cenv::of_env(cx, f.tag(), installed)?;
                let st = prims::state(cx);
                if !function_context(cx, st, &mut cenv, formals, body, Some(&mut prelude))? {
                    return Err(Need::Fail);
                }
                let mut c = Comp::new(cx, st, Profile::Bc, Some(cenv));
                let mut b = c.root(body)?;
                c.add_unit_locals(&mut b);
                let code = c.build(b, body, Some(formals))?;
                index_codes(cx, code, &c.arg_codes, None);
                Ok(code)
            }
        });
        if matches!(code, Ok(None)) && !prelude.raised && prims::state(cx).compile_warnings.prelude() > 0 {
            cenv::raise_prelude(cx)?;
            prelude.raised = true;
            continue;
        }
        break code;
    };
    if let Some(c) = check {
        c.report();
    }
    // cmp.R's warnings, given in the compile whether it compiled or not
    // (tryCmpfun's error comes after them)
    let code = match code {
        Ok(c) => Ok(raise_compile_warnings(cx, c)?),
        e => e,
    };
    if let Ok(Some(c)) = code {
        let f = op.bind(cx).sexp();
        // SAFETY: a rooted closure and its fresh code object.
        unsafe {
            if f.cdr().typ() == BCODESXP {
                crate::code::cache_on(f.cdr(), c);
            }
        }
    }
    code
}

/// `ROBIGO_BC_CROSSCHECK`: closure `f`'s byte code translated and its
/// source compiled as the from-source compiler compiles it for the
/// closure's environment, the two compared ([`crosscheck`]). The
/// from-source compile keeps its argument code on the source's expressions,
/// the translation its own on the promises' byte code (`MAKEPROM`'s
/// BCODESXPs, translated at their first force), so neither replaces the
/// other's. A compile that would force
/// a lazy binding, or that cmp.R declines, is skipped. `installed`: base's
/// installed byte code (provenance, as for [`compile_closure_bc`]).
///
/// # Safety
/// `f` is a live closure whose body is a BCODESXP; nothing collects.
unsafe fn crosscheck_closure(cx: &Cx, f: Sexp, installed: bool) -> Result<crosscheck::Finding, Need> {
    // (its compile from source is no compile of GNU R's: cmp.R's warnings
    // there are not given)
    let warnings = &prims::state(cx).compile_warnings;
    let mark = warnings.len();
    // SAFETY: the caller's contract.
    let r = unsafe { crosscheck_closure_in(cx, f, installed) };
    warnings.truncate(mark);
    r
}

/// [`crosscheck_closure`]'s comparison.
///
/// # Safety
/// As for [`crosscheck_closure`].
unsafe fn crosscheck_closure_in(cx: &Cx, f: Sexp, installed: bool) -> Result<crosscheck::Finding, Need> {
    use crosscheck::{Finding, Verdict};
    // SAFETY: the caller's contract.
    unsafe {
        let (formals, bc) = (f.car(), f.cdr());
        let body = robigo_rtbuiltins::bytecode::expr(cx, Ref::from_sexp(bc, cx)).sexp();
        let st = prims::state(cx);
        let what = crate::deparse::deparse1line(cx, Ref::from_sexp(body, cx), false, 0)
            .map(|d| {
                let s = String::from_utf8_lossy(&d).split_whitespace().collect::<Vec<_>>().join(" ");
                if s.chars().count() > 70 { format!("{}...", s.chars().take(70).collect::<String>()) } else { s }
            })
            .unwrap_or_else(|_| "<a closure>".to_owned());
        let source = (|| -> Result<Option<Built>, Need> {
            let mut cenv = Cenv::of_env(cx, f.tag(), installed)?;
            if !function_context(cx, st, &mut cenv, formals, body, None)? {
                return Ok(None);
            }
            let mut c = Comp::new(cx, st, Profile::Bc, Some(cenv));
            let mut b = c.root(body)?;
            c.add_unit_locals(&mut b);
            Ok(Some(b))
        })();
        let source = match source {
            Ok(Some(b)) => b,
            Ok(None) | Err(Need::Fail) => {
                let line = format!("skipped: {what}: the from-source compiler leaves it uncompiled");
                return Ok(Finding::new(Verdict::Skipped, line));
            }
            Err(Need::Force(p)) => {
                // (the promise's code, for triage)
                let code = crate::deparse::deparse1line(cx, Ref::from_sexp(p.cdr(), cx), false, 0)
                    .map(|d| String::from_utf8_lossy(&d).chars().take(60).collect::<String>())
                    .unwrap_or_default();
                let line = format!("skipped: {what}: the from-source compile would force a binding ({code})");
                return Ok(Finding::new(Verdict::Skipped, line));
            }
            Err(e) => return Err(e),
        };
        let mut c = Comp::new(cx, st, Profile::Bc, Some(Cenv::for_translation()));
        let r = c.alloc();
        match c.translate(bc, r) {
            Ok(()) => {
                let mut i = Ins::new(Op::Ret);
                i.b = r;
                c.emit(i);
                let t = c.finish();
                Ok(crosscheck::compare(cx, &what, &t, &source))
            }
            Err(Need::Fail) => {
                let why =
                    c.bc_untranslated.take().unwrap_or_else(|| "byte code the translation cannot follow".to_owned());
                Ok(Finding::new(Verdict::Untranslated, format!("untranslated: {what}: {why}")))
            }
            Err(e) => Err(e),
        }
    }
}

/// For the base.rdb translation test: closure `f`'s byte code translated,
/// and the byte code of each function it makes (`MAKECLOSURE`'s), without
/// running any; each translation through the [`crate::code::Verified`]
/// gate. What each one stopped at (an opcode or a primitive robigo-rt does
/// not have, code the translation cannot follow, a refusal of the gate);
/// none when all of them translate. (The crosscheck has its own entry,
/// [`crosscheck_installed`], in a session with base loaded.)
#[doc(hidden)]
pub fn translation_gaps(cx: &mut Cx, f: Root<'_>) -> RResult<Vec<String>> {
    let mut gaps = Vec::new();
    attempt(cx, |cx| {
        gaps.clear();
        // SAFETY: a rooted closure; the byte code it makes is reachable
        // from it; nothing collects in this interval.
        unsafe {
            let f = f.bind(cx).sexp();
            let st = prims::state(cx);
            let mut todo = vec![f.cdr()];
            while let Some(bc) = todo.pop() {
                if bc.typ() != BCODESXP {
                    continue;
                }
                let mut c = Comp::new(cx, st, Profile::Bc, Some(Cenv::for_translation()));
                let r = c.alloc();
                match c.translate(bc, r) {
                    Ok(()) => {
                        let mut i = Ins::new(Op::Ret);
                        i.b = r;
                        c.emit(i);
                        if let Err(e) = crate::code::Verified::new(c.finish()) {
                            gaps.push(format!("the verifier refuses the translation: {e}"));
                        }
                    }
                    Err(Need::Fail) => gaps.push(
                        c.bc_untranslated
                            .take()
                            .unwrap_or_else(|| "byte code the translation cannot follow".to_owned()),
                    ),
                    Err(e) => return Err(e),
                }
                // the byte code it holds: MAKEPROM's, and the functions it
                // makes (MAKECLOSURE's list(formals, body, ...)), whose
                // own byte code a promise's may hold in turn
                let consts = bc.cdr();
                let p = consts.std_data() as *const Sexp;
                for j in 0..consts.std_len() as usize {
                    let k = *p.add(j);
                    if k.typ() == BCODESXP {
                        todo.push(k);
                    } else if k.typ() == VECSXP && k.std_len() >= 2 {
                        todo.push(*(k.std_data() as *const Sexp).add(1));
                    }
                }
            }
            Ok(cx.nil().sexp())
        }
    })?;
    Ok(gaps)
}

/// For the base.rdb crosscheck test: under `ROBIGO_BC_CROSSCHECK`, closure
/// `f`'s byte code (base's installed code) compared with what the
/// from-source compiler makes of its source ([`crosscheck`]), reported,
/// and the verdict. The session has base loaded with every binding
/// forced, as GNU R's build had when it compiled base (makebasedb.R), so
/// the compile decides on base's definitions as cmp.R did. None for a
/// closure without byte code, or without the variable.
#[doc(hidden)]
pub fn crosscheck_installed(cx: &mut Cx, f: Root<'_>) -> RResult<Option<crosscheck::Verdict>> {
    if !crosscheck::enabled() {
        return Ok(None);
    }
    let mut check = None;
    attempt(cx, |cx| {
        // SAFETY: a rooted closure; nothing collects in this interval.
        unsafe {
            let f = f.bind(cx).sexp();
            if f.cdr().typ() == BCODESXP {
                check = Some(crosscheck_closure(cx, f, true)?);
            }
        }
        Ok(cx.nil().sexp())
    })?;
    Ok(check.map(|c| {
        c.report();
        c.verdict
    }))
}

/// The code of the byte code `bc` evaluated as itself (`eval(bc, rho)`,
/// bcEval at the top level, or a promise's code, `MAKEPROM`'s BCODESXP):
/// its translation, kept on it. A promise's code is translated at its
/// first force (its closure's translation leaves it), with the slot
/// indices of the frame `rho` that force runs in; every later force runs
/// that translation, and a frame of another shape has its slots read by
/// name (`Cx::frame_slot_raw` checks the shape, then the slot's symbol).
pub(crate) fn bytecode_code(cx: &mut Cx, bc: Root<'_>, rho: Root<'_>) -> RResult<Sexp> {
    // SAFETY: a rooted BCODESXP: its 4th word is R_NilValue or a code
    // object this module stored.
    if let Some(c) = unsafe { crate::code::cached(cx, bc.bind(cx).sexp().aux(), Profile::Bc) } {
        return Ok(c);
    }
    let code = attempt(cx, |cx| {
        // SAFETY: a rooted BCODESXP and a rooted environment, both read
        // in this attempt: a collection between attempts (Need::Gc) may
        // move the frame's shape, so the shape is never kept across one;
        // nothing collects within the interval.
        unsafe {
            let shape = cx.frame_shape_raw(rho.bind(cx).sexp());
            translate_code(cx, bc.bind(cx).sexp(), None, shape)
        }
    })?
    .expect("robigo: a translation always makes code");
    // SAFETY: as above: a traced word, stored with the barrier.
    unsafe { crate::code::cache_on(bc.bind(cx).sexp(), code) };
    Ok(code)
}

/// The code of the byte code `bc` (a BCODESXP): its translation (V3,
/// [`bctrans`]); with `formals`, a closure body's, with the frame shape of
/// its formals and the variables it assigns. Where the translation stops
/// at what it does not know, the code raises "not implemented yet" when it
/// runs (AGENTS rule 6), never runs the source instead.
///
/// # Safety
/// `bc` is a live BCODESXP of `cx`'s isolate, `formals` a live pairlist,
/// `index` None or a live shape (a VECSXP of symbols) of the frames the
/// code runs in, read in the caller's current interval
/// ([`crate::code::set_index_shape`]'s contract).
unsafe fn translate_code(cx: &Cx, bc: Sexp, formals: Option<Sexp>, index: Option<Sexp>) -> Result<Sexp, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        // the translation cache's, when it has this byte code's (and every
        // check passes): what the translation below makes, made from it
        if let Some((code, entry)) = cached_translation(cx, bc, formals, index)? {
            if !crate::tcache::verify_enabled() {
                return Ok(code);
            }
            // compared, not trusted: a fresh translation too, the same or
            // reported (and then the fresh one runs)
            crate::tcache::verify_begin(cx);
            let fresh = match translate_uncached(cx, bc, formals, index) {
                Ok(c) => c,
                Err(e) => {
                    // a retryable stop (Need::Gc, Need::Force): the attempt
                    // runs again and verifies then; this one compares nothing
                    crate::tcache::verify_abort(cx);
                    return Err(e);
                }
            };
            let same = crate::tcache::verify_end(cx, &entry);
            return Ok(if same { code } else { fresh });
        }
        translate_uncached(cx, bc, formals, index)
    }
}

/// [`translate_code`] with no look at the translation cache.
///
/// # Safety
/// As [`translate_code`]'s.
unsafe fn translate_uncached(cx: &Cx, bc: Sexp, formals: Option<Sexp>, index: Option<Sexp>) -> Result<Sexp, Need> {
    // SAFETY: the caller's contract.
    let r = unsafe { translate_uncached_in(cx, bc, formals, index) };
    if r.is_err() {
        // (an abandoned attempt leaves no Sexp in the writer's capture)
        crate::tcache::writer_abort(cx);
    }
    r
}

/// [`translate_uncached`]'s body.
///
/// # Safety
/// As [`translate_code`]'s.
unsafe fn translate_uncached_in(cx: &Cx, bc: Sexp, formals: Option<Sexp>, index: Option<Sexp>) -> Result<Sexp, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        let writing = crate::tcache::writer_begin(cx, bc);
        let st = prims::state(cx);
        let mut c = Comp::new(cx, st, Profile::Bc, Some(Cenv::for_translation()));
        let r = c.alloc();
        let res = c.translate(bc, r);
        let expr = robigo_rtbuiltins::bytecode::expr(cx, Ref::from_sexp(bc, cx)).sexp();
        // (each code built by the compile that made it: its buffers go back
        // to the scratch with that compile's)
        let code = match res {
            Ok(()) => {
                let mut i = Ins::new(Op::Ret);
                i.b = r;
                c.emit(i);
                let b = c.finish();
                if writing {
                    crate::tcache::writer_end(cx, bc, index, &b);
                }
                c.build(b, expr, formals)?
            }
            Err(Need::Fail) => {
                let what =
                    c.bc_untranslated.take().unwrap_or_else(|| "byte code the translation cannot follow".to_owned());
                let mut f = Comp::new(cx, st, Profile::Bc, Some(Cenv::for_translation()));
                let r = f.alloc();
                let nil = f.nil();
                f.unimplemented(nil, &format!("translating {what}"))?;
                let mut i = Ins::new(Op::Ret);
                i.b = r;
                f.emit(i);
                let b = f.finish();
                if writing {
                    crate::tcache::writer_end(cx, bc, index, &b);
                }
                f.build(b, expr, formals)?
            }
            Err(e) => return Err(e),
        };
        index_codes(cx, code, &c.arg_codes, index);
        Ok(code)
    }
}

/// [`translate_code`]'s result from the translation cache
/// ([`crate::tcache::lookup`]): the code and its argument codes built from
/// the cached translation as the translation builds its own, if every one
/// passes [`crate::code::Verified::new`]; None on any miss.
///
/// # Safety
/// As [`translate_code`]'s.
unsafe fn cached_translation(
    cx: &Cx,
    bc: Sexp,
    formals: Option<Sexp>,
    index: Option<Sexp>,
) -> Result<Option<(Sexp, crate::tcache::format::Entry)>, Need> {
    // SAFETY: the caller's contract.
    let Some(hit) = (unsafe { crate::tcache::lookup(cx, bc, index)? }) else { return Ok(None) };
    // (the entry passed every check of the file: a refusal is the writer's,
    // the round trip's or the verifier's disagreement, reported)
    let refused = |m: &crate::code::Malformed| {
        crate::tcache::refused(m);
        Ok(None)
    };
    // SAFETY: the entry is this build's translator's own, intact: its file's
    // header carries this binary's build ID and this tree's stamp
    // (`tcache::open`), its row's keyed checksum matched its key and data
    // (`format::decode_checked`), and its constants passed the typed check
    // (`tcache::typed_ok`). The trust stops there: the cache is trusted as
    // the tree's base.rdb is, since whoever can write one can write the
    // other, and base.rdb's R code can already run anything (`dyn.load`,
    // `system`).
    let main = match unsafe { crate::code::Verified::from_cached_translation(hit.built) } {
        Ok(v) => v,
        Err(m) => return refused(&m),
    };
    let mut args = Vec::with_capacity(hit.args.len());
    for (call, b) in hit.args {
        // SAFETY: as above, an argument code of the same entry.
        match unsafe { crate::code::Verified::from_cached_translation(b) } {
            Ok(v) => args.push((call, v)),
            Err(m) => return refused(&m),
        }
    }
    // SAFETY: the caller's contract; the calls are the BCODESXP's constants
    // (`tcache::lookup` resolved them, live), nothing collects meanwhile.
    unsafe {
        let expr = robigo_rtbuiltins::bytecode::expr(cx, Ref::from_sexp(bc, cx)).sexp();
        let code = crate::code::build(cx, &main, expr, formals)?;
        let mut arg_codes = Vec::with_capacity(args.len());
        for (call, v) in &args {
            let built = crate::code::build(cx, v, *call, None)?;
            crate::code::cache_on(*call, built);
            arg_codes.push(built);
        }
        index_codes(cx, code, &arg_codes, index);
        crate::tcache::count(crate::tcache::Outcome::Hit);
        Ok(Some((code, hit.entry)))
    }
}

/// The translation cache's writer (`ROBIGO_TRANSLATION_CACHE_WRITE`,
/// scripts/rt-home-db.sh): translate every closure of base's and the
/// compiler's namespaces whose body is their database's byte code, and the
/// function literals that byte code makes (a `MAKECLOSURE` constant: its
/// formals and body), so each translation is kept ([`crate::tcache`]). The
/// bindings were forced by the script that ran before.
pub fn translate_for_cache(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let mut envs = vec![s.root(cx, cx.base_namespace())];
        let o = s.out(cx);
        let ns = o.root();
        if crate::ns::find_namespace(cx, b"compiler", o).is_ok() {
            envs.push(ns);
        }
        for env in envs {
            let names: Vec<Sexp> = match env.bind(cx).view() {
                robigo_rtcore::View::Env(e) => match cx.frame_bindings(e, true) {
                    Some(b) => b.into_iter().map(|(y, _)| y.as_ref().sexp()).collect(),
                    None => {
                        crate::tcache::skip(cx, "a namespace whose frame could not be listed");
                        continue;
                    }
                },
                _ => continue,
            };
            for y in names {
                // SAFETY: a symbol, immortal.
                let y = s.root(cx, unsafe { Ref::from_sexp(y, cx) });
                let o = s.out(cx);
                let v = o.root();
                if cx.eval(y, env, o).is_err() {
                    crate::tcache::skip(cx, "a binding whose value could not be got");
                    continue;
                }
                let f = v.bind(cx).sexp();
                // SAFETY: a rooted object; a closure's body and formals.
                let (body, formals) = unsafe {
                    if f.typ() != CLOSXP || f.cdr().typ() != BCODESXP {
                        continue;
                    }
                    (f.cdr(), f.car())
                };
                // the closure's body, then each function literal its byte
                // code makes, depth first
                // (each queued pair rooted as it is queued: a translation's
                // attempt may collect and move the ones still waiting)
                // SAFETY: reachable from the rooted closure, rooted at once.
                let first = unsafe { (s.root(cx, Ref::from_sexp(body, cx)), s.root(cx, Ref::from_sexp(formals, cx))) };
                let mut todo = vec![first];
                while let Some((bcr, fmr)) = todo.pop() {
                    // SAFETY: a rooted BCODESXP: its 4th word is R_NilValue,
                    // a stub or a code object.
                    if unsafe { crate::code::cached(cx, bcr.bind(cx).sexp().aux(), Profile::Bc) }.is_none() {
                        let code = attempt(cx, |cx| {
                            // SAFETY: rooted, read in this attempt.
                            unsafe { translate_code(cx, bcr.bind(cx).sexp(), Some(fmr.bind(cx).sexp()), None) }
                        })?;
                        if let Some(c) = code {
                            // SAFETY: the rooted BCODESXP and its fresh code.
                            unsafe { crate::code::cache_on(bcr.bind(cx).sexp(), c) };
                        }
                    }
                    // the function literals in its constants, each rooted as
                    // it is found (nothing collects while they are found)
                    // SAFETY: the rooted BCODESXP's constants, a VECSXP;
                    // each element tested before its words are read.
                    unsafe {
                        let pool = bcr.bind(cx).sexp().cdr();
                        if pool.typ() != VECSXP {
                            continue;
                        }
                        for i in 0..pool.std_len() as usize {
                            let k = *(pool.std_data() as *const Sexp).add(i);
                            if k.typ() == VECSXP && k.std_len() >= 2 {
                                let (kf, kb) = (*(k.std_data() as *const Sexp), *(k.std_data() as *const Sexp).add(1));
                                if matches!(kf.typ(), LISTSXP | NILSXP) && kb.typ() == BCODESXP {
                                    todo.push((s.root(cx, Ref::from_sexp(kb, cx)), s.root(cx, Ref::from_sexp(kf, cx))));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    })
}

/// `make.functionContext(cntxt, forms, body)` on `cenv`: a frame of the
/// formals, then the locals of their defaults and the body; false when
/// the body may call `browser()` (cmp.R does not compile it). The frame is
/// left on `cenv` (popped by the caller when it is done). With `prelude`
/// (a closure's own compile), findLocals runs through it
/// ([`Prelude::found`]): when its warnings are still to be raised, the
/// attempt stops ([`Need::Fail`]) before `mayCallBrowser`, which may force
/// a promise.
///
/// # Safety
/// `formals` (a pairlist) and `body` are live.
unsafe fn function_context(
    cx: &Cx,
    st: &RtState,
    cenv: &mut Cenv<'_>,
    formals: Sexp,
    body: Sexp,
    prelude: Option<&mut Prelude>,
) -> Result<bool, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        let forms = cenv::elements(cx, formals, false);
        cenv.push_unit(forms.iter().map(|f| f.1).collect());
        let mut elist: Vec<Sexp> = forms.iter().map(|f| f.0).collect();
        elist.push(body);
        let locals = match prelude {
            Some(p) => match p.found(cx, st, cenv, &elist)? {
                Some(l) => l,
                // (raised before anything else of the compile)
                None => return Err(Need::Fail),
            },
            None => cenv.find_locals(cx, st, &elist)?,
        };
        cenv.add_unit_vars(&locals);
        Ok(!cenv::may_call_browser(cx, st, cenv, body)?)
    }
}

/// `R_compileExpr(call, rho)` for a top-level loop (`compile(e, env)`: the
/// loop's locals added to the global frame): its code, or None when cmp.R
/// fails (`tryCompile` returns the call; it runs uncompiled).
// R: eval.c:R_compileExpr; cmp.R:compile
pub fn compile_top_loop(cx: &mut Cx, call: Root<'_>, rho: Root<'_>) -> RResult<Option<Sexp>> {
    let mut prelude = Prelude::default();
    let code = loop {
        let code = attempt(cx, |cx| {
            let (e, env) = (call.bind(cx).sexp(), rho.bind(cx).sexp());
            // (a retried attempt gives cmp.R's warnings again)
            prims::state(cx).compile_warnings.clear();
            // SAFETY: rooted objects; nothing collects in this interval.
            unsafe {
                let mut cenv = Cenv::of_env(cx, env, false)?;
                let st = prims::state(cx);
                let locals = match prelude.found(cx, st, &cenv, &[e])? {
                    Some(l) => l,
                    // (raised before anything of the loop is compiled)
                    None => return Err(Need::Fail),
                };
                cenv.add_top_vars(locals);
                let mut c = Comp::new(cx, st, Profile::Bc, Some(cenv));
                let b = c.root(e)?;
                Ok(c.build(b, e, None)?)
            }
        });
        if matches!(code, Ok(None)) && !prelude.raised && prims::state(cx).compile_warnings.prelude() > 0 {
            cenv::raise_prelude(cx)?;
            prelude.raised = true;
            continue;
        }
        break code;
    };
    match code {
        Ok(c) => raise_compile_warnings(cx, c),
        e => e,
    }
}

/// A compile's findLocals (`make.functionContext`'s, `compile`'s) across its
/// attempts. Its warnings are the compile's prelude, which GNU R gives
/// before anything else of the compile, so before it forces a promise or
/// decides anything: the first attempt stops at the end of findLocals when
/// there are any ([`Prelude::found`]), the caller raises them
/// ([`cenv::raise_prelude`]: a handler they run may change the
/// environment) and compiles again. That compile keeps findLocals' locals
/// as GNU R's one findLocals found them, before the handlers ran, and
/// gives no warning of its own.
///
/// INTERIM(C1), until V4 step 3 (cmp.R's own code compiling): what cmp.R
/// gives later (cmpAssign's `as.name`) is raised after the compile, so a
/// handler of one does not change the rest of the compile, where in GNU R
/// it can (`tests/rt/jit/translate-warning-asname.R`, not ratcheted).
#[derive(Default)]
struct Prelude {
    /// Whether the warnings were raised (the compile made again).
    raised: bool,
    /// The locals findLocals found (symbols, which are never collected).
    locals: Vec<Sexp>,
}

impl Prelude {
    /// findLocals of `elist` on `cenv`: its locals, or None when the attempt
    /// stops for its warnings to be raised.
    ///
    /// # Safety
    /// The elements of `elist` are live.
    unsafe fn found(
        &mut self,
        cx: &Cx,
        st: &RtState,
        cenv: &Cenv<'_>,
        elist: &[Sexp],
    ) -> Result<Option<Vec<Sexp>>, Need> {
        if self.raised {
            return Ok(Some(self.locals.clone()));
        }
        // SAFETY: the caller's contract.
        let locals = unsafe { cenv.find_locals(cx, st, elist)? };
        let w = &st.compile_warnings;
        w.mark_prelude();
        if w.prelude() > 0 {
            self.locals = locals;
            return Ok(None);
        }
        Ok(Some(locals))
    }
}

/// cmp.R's warnings of the compile just made ([`cenv::raise_compile_warnings`]),
/// with its code `code` rooted across them (a warning runs R code: a
/// handler, `options(warn = 2)`'s error): the code, read again after them.
///
/// INTERIM(C1), until V4 step 3 (cmp.R's own code compiling), or #964's
/// handler for the first:
/// - an error among them (a warning `options(warn = 2)` makes one, a
///   handler's `stop()`) is an error inside cmp.R, which GNU R's
///   `tryCmpfun` takes with its own handler, the innermost: the compile
///   fails, the closure runs uncompiled (NOJIT) and raises its own error.
///   Here the error leaves the compile, reported with cmp.R's call
///   (`tests/rt/jit/translate-warning-warn2.R`, not ratcheted);
/// - no R frame is on the stack while they run, where GNU R's has the
///   called function's and cmp.R's (`tryCmpfun` ... `addVar`, `assign`):
///   a handler's `sys.calls()` and an error's traceback differ
///   (`tests/rt/jit/translate-warning-frames.R`, not ratcheted).
// R: cmp.R:tryCmpfun, tryCompile
fn raise_compile_warnings(cx: &mut Cx, code: Option<Sexp>) -> RResult<Option<Sexp>> {
    if prims::state(cx).compile_warnings.is_empty() {
        return Ok(code);
    }
    cx.scope(|cx, s| {
        // SAFETY: the compile's fresh code object, rooted at once.
        let r = code.map(|c| s.root(cx, unsafe { Ref::from_sexp(c, cx) }));
        cenv::raise_compile_warnings(cx)?;
        Ok(r.map(|r| r.bind(cx).sexp()))
    })
}

/// Compile the call `call` of the special at `PRIMOFFSET` `off`, reached
/// through a value rather than its name (no guard): its operands are
/// evaluated one level below the code object's base, as the special's own
/// `eval` calls nest. Always the `Eval` profile: GNU R calls the special,
/// which evaluates its arguments by `eval` (bcEval's `CALLSPECIAL` too).
/// `call` is read through its root on every attempt.
///
/// With `args` (a pairlist), they stand in place of the call's:
/// `applyMethod`'s `PRIMFUN(op)(call, op, args, rho)` of a special method,
/// `call` for its messages and its dispatch, `args` the method's promises
/// (each forced as `eval` forces it).
// R: objects.c:applyMethod (SPECIALSXP)
pub(crate) fn compile_special_with(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: Option<robigo_rtcore::Root<'_>>,
    off: usize,
) -> RResult<Sexp> {
    attempt(cx, |cx| {
        let call = call.bind(cx).sexp();
        let args = args.map(|a| a.bind(cx).sexp());
        // SAFETY: a rooted call of `cx`'s isolate; nothing collects in this
        // interval.
        unsafe {
            let st = prims::state(cx);
            let mut c = Comp::new(cx, st, Profile::Eval, None);
            let dst = c.alloc();
            match st.defs[off].kind {
                Kind::Special(s) => c.special(s, call, args.unwrap_or_else(|| call.cdr()), dst, 0, Tail::Yes)?,
                Kind::Builtin(..) | Kind::Fn(_) => unreachable!("robigo: only a language special is compiled"),
            }
            let mut i = Ins::new(Op::Ret);
            i.b = dst;
            c.emit(i);
            let b = c.finish();
            Ok(c.build(b, call, None)?)
        }
    })
    .map(|c| c.expect("robigo: the Eval profile always compiles"))
}

/// The registers of a `for` loop's state ([`Op::ForPrep`]): the
/// sequence, the index, the length, the loop value, the variable's cell,
/// and a compact sequence's first element and increment (`vm::for_prep`).
pub const FOR_STATE: usize = 7;

/// Labels of a loop being compiled: `break` jumps to its end, `next` to
/// its start (both inside its region).
struct LoopLabels {
    start: usize,
    breaks: Vec<usize>,
    /// The first register the loop's body allocates: a `break` or `next`
    /// leaves every register from here on dead.
    regs: Reg,
}

/// Where a value is compiled (cmp.R's context): in the tail (`tailcall`,
/// the shapes that RETURN and set visibility), as `return()`'s operand
/// (`make.nonTailCallContext`: non-tail shapes, the value's visibility what
/// bcEval leaves at RETURN), or neither. `Live`: under `Eval`, a `{`'s
/// statement before its last while the `{` is in the tail, compiled as the
/// tail (a write that cuts the list there makes it the last), but when
/// the statement is an assignment its invisibility is left to its `Resume`
/// (`c` 1), which sets it only when that cut happens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tail {
    No,
    Yes,
    Ret,
    Live,
}

impl Tail {
    /// cmp.R's tail shapes (INVISIBLE, a jump to the end, RETURN).
    fn is_tail(self) -> bool {
        matches!(self, Tail::Yes | Tail::Live)
    }

    /// A value op sets visibility (F_VIS): bcEval's loads and operators set
    /// R_Visible, observed in the tail and at `return()`.
    fn vis(self) -> bool {
        self != Tail::No
    }

    /// `return()`'s operand: an LDNULL (a missing `else`, a switch's miss, a
    /// loop's value) sets R_Visible, and an assignment leaves it as its
    /// value's (bcEval's SETVAR, ENDASSIGN).
    fn ret(self) -> bool {
        self == Tail::Ret
    }

    /// An assignment's value: `return()`'s context carries into it, the
    /// tail's does not (cmp.R compiles the value non-tail).
    fn value(self) -> Tail {
        if self == Tail::Ret { Tail::Ret } else { Tail::No }
    }
}

impl From<bool> for Tail {
    fn from(tail: bool) -> Tail {
        if tail { Tail::Yes } else { Tail::No }
    }
}

/// A `SetVis` that falls into a `Ret` no jump lands on, through `Move`s and
/// `Clear`s only (`moves`: under `Eval`, whose instructions keep offsets of
/// their own, none), gives the `Ret` its visibility (`g`; cmp.R's INVISIBLE,
/// then RETURN): the instructions after it move up one, the `Ret` with
/// them, and the old `Ret` is left unreached.
fn fold_vis_into_ret(ins: &mut [Ins], moves: bool) {
    // (by IR position: a `Loop`'s end can be the code's end, and the one
    // after it one past that; only positions before the end are asked
    // about below, so a mark past the table changes nothing)
    let mut targets = vec![false; ins.len() + 2];
    let mut mark = |t: usize| {
        if let Some(m) = targets.get_mut(t) {
            *m = true;
        }
    };
    for i in ins.iter() {
        match i.op {
            Op::Jmp
            | Op::BackJmp
            | Op::BrFalse
            | Op::Guard
            | Op::AndOr1
            | Op::ForStep
            | Op::BaseGuard
            | Op::TopLoop
            | Op::StartSubset
            | Op::Resume => mark(i.target()),
            // (a region's end, and the instruction after it)
            Op::Loop | Op::LoopResume => {
                mark(i.target());
                mark(i.target() + 1);
            }
            _ => {}
        }
    }
    let mut j = 0;
    while j < ins.len() {
        if ins[j].op == Op::SetVis {
            let mut m = j + 1;
            while moves && m < ins.len() && matches!(ins[m].op, Op::Move | Op::Clear | Op::ClearRange) {
                m += 1;
            }
            if m < ins.len() && ins[m].op == Op::Ret && ins[m].g == 0 && !(j + 1..=m).any(|k| targets[k]) {
                let g = if ins[j].g != 0 { 1 } else { 2 };
                ins.copy_within(j + 1..=m, j);
                ins[m - 1].g = g;
                j = m;
                continue;
            }
        }
        j += 1;
    }
}

/// The buffers one compile works in ([`Comp`]'s vectors and pool map).
/// Between compiles they wait in the isolate's [`Scratch`] empty, so a
/// compile does not allocate and grow them afresh.
#[derive(Default)]
pub(crate) struct Bufs {
    ins: Vec<Ins>,
    consts: Vec<Sexp>,
    pool: PosMap<usize, u16>,
    loops: Vec<LoopLabels>,
    locals: Vec<Sexp>,
    ppk: Vec<i32>,
    arg_codes: Vec<Sexp>,
}

/// The isolate's spare [`Bufs`] ([`RtState::scratch`]): one set per
/// compile under way takes one (a compile nests one for an argument's or a
/// promise's code), and gives it back when it ends. A set waits cleared,
/// so it holds no `Sexp` (nothing in it is a root across a collection),
/// and keeps at most [`SCRATCH_CAP`] elements' capacity per buffer: an
/// unusually large compile's buffers are freed, not kept.
///
/// One word in [`RtState`]: the pool is allocated at the isolate's first
/// compile (a long-lived per-isolate struct that grows moves allocations
/// into other size classes).
#[derive(Default)]
pub(crate) struct Scratch(std::cell::OnceCell<Box<std::cell::RefCell<Vec<Bufs>>>>);

/// The spare sets [`Scratch`] keeps: the depth of nested compiles that
/// reuse buffers (a deeper one allocates its own, as before). Compiles
/// nest 4 deep in nearly every conformance case, 5 in a fifth of them and
/// rarely deeper; fewer sets lose the start's gain (2 sets: 2.14M of the
/// rdb-jit start's 2.23M, 1 set: 0.75M).
const SCRATCH_SETS: usize = 4;

/// The capacity, in elements, a spare buffer keeps (256 instructions are
/// 4 KiB). Measured against 512, 1,024 and 2,048: the same instructions on
/// the run-once rows, the jit per-compile rows and the rdb-jit start, for
/// 18,640 bytes kept at the end of that start against 121,680 at 2,048.
const SCRATCH_CAP: usize = 256;

impl Scratch {
    fn sets(&self) -> &std::cell::RefCell<Vec<Bufs>> {
        self.0.get_or_init(Default::default)
    }

    fn take(&self) -> Bufs {
        let b = self.sets().borrow_mut().pop().unwrap_or_default();
        debug_assert!(
            b.ins.is_empty()
                && b.consts.is_empty()
                && b.pool.is_empty()
                && b.loops.is_empty()
                && b.locals.is_empty()
                && b.ppk.is_empty()
                && b.arg_codes.is_empty(),
            "robigo: a compile's spare buffers were given back uncleared"
        );
        b
    }

    fn give(&self, mut b: Bufs) {
        fn keep<T>(v: &mut Vec<T>) {
            v.clear();
            if v.capacity() > SCRATCH_CAP {
                *v = Vec::new();
            }
        }
        keep(&mut b.ins);
        keep(&mut b.consts);
        keep(&mut b.loops);
        keep(&mut b.locals);
        keep(&mut b.ppk);
        keep(&mut b.arg_codes);
        b.pool.clear();
        if b.pool.capacity() > SCRATCH_CAP {
            b.pool = PosMap::default();
        }
        let mut sets = self.sets().borrow_mut();
        if sets.len() < SCRATCH_SETS {
            sets.push(b);
        }
    }
}

struct Comp<'g> {
    cx: &'g Cx,
    st: &'g RtState,
    profile: Profile,
    ins: Vec<Ins>,
    consts: Vec<Sexp>,
    pool: PosMap<usize, u16>,
    next: Reg,
    max: Reg,
    max_depth: u16,
    loops: Vec<LoopLabels>,
    /// Symbols the code binds in its frame (`<-`, `=`, `for`).
    locals: Vec<Sexp>,
    /// Closure call sites so far ([`Op::Call`]'s `c`).
    sites: u16,
    /// The `Bc` profile's compile environment (cmp.R's `cntxt$env`), whose
    /// decisions say what the code inlines; None under `Eval`.
    bc: Option<Cenv<'g>>,
    /// `Eval`: GNU R's evaluator protections at the entry of the `eval` of
    /// the expression being compiled, above the code object's root's
    /// (robigo_rtcore's ppacct): what `eval`'s `PROTECT(op)`, `evalList`'s
    /// head and the specials' own `PROTECT`s add on the way down.
    pp: i32,
    /// `Eval`: each instruction's protect offset ([`crate::code::View::ppk`]).
    ppk: Vec<i32>,
    /// `Eval`: the register of the protect stack's top the guard of the
    /// call being compiled saved (0: none), for a check inside it
    /// (`do_internal`'s).
    site_save: Reg,
    /// The code objects of the arguments compiled with this code (and
    /// theirs): their promises run in this code's frame, so they take its
    /// shape's slot indices once it exists ([`index_codes`]). Raw: an
    /// attempt collects nothing.
    arg_codes: Vec<Sexp>,
    /// What a byte-code translation stopped at ([`bctrans`]): the code
    /// raises "not implemented yet" for it.
    bc_untranslated: Option<String>,
    /// The assignment just compiled as a `{`'s earlier statement
    /// ([`Tail::Live`]): its invisibility is its Resume's. Only one that is
    /// the statement itself (`live_stmt`); one inside it (an `if`'s
    /// branch) sets it as the tail does.
    cut_assign: Option<Sexp>,
    live_stmt: Option<Sexp>,
}

impl Drop for Comp<'_> {
    /// The compile's buffers back to the isolate's [`Scratch`], cleared
    /// (also for an attempt abandoned midway: `Need::Gc`, `Need::Force`).
    fn drop(&mut self) {
        let b = Bufs {
            ins: std::mem::take(&mut self.ins),
            consts: std::mem::take(&mut self.consts),
            pool: std::mem::take(&mut self.pool),
            loops: std::mem::take(&mut self.loops),
            locals: std::mem::take(&mut self.locals),
            ppk: std::mem::take(&mut self.ppk),
            arg_codes: std::mem::take(&mut self.arg_codes),
        };
        self.st.scratch.give(b);
    }
}

/// Give code `code` and the argument codes compiled with it (`arg_codes`)
/// their slot indices ([`crate::code::set_index_shape`]): with `shape`,
/// that frame shape's for all of them (a promise's code, translated for
/// the frame it is forced in); without, the argument codes take closure
/// code `code`'s own frame shape (which its build indexed it with).
///
/// # Safety
/// `code` is a live code object and `arg_codes` live code objects that
/// have not run, of `cx`'s isolate; `shape` is None or a live shape (a
/// VECSXP of symbols) read in the caller's current interval.
unsafe fn index_codes(cx: &Cx, code: Sexp, arg_codes: &[Sexp], shape: Option<Sexp>) {
    // SAFETY: the caller's contract.
    unsafe {
        let shape = match shape {
            Some(s) => {
                crate::code::set_index_shape(cx, code, s);
                s
            }
            None => crate::code::shape(code),
        };
        for &a in arg_codes {
            crate::code::set_index_shape(cx, a, shape);
        }
    }
}

/// What the `Bc` profile compiles for a call of a base function cmp.R
/// inlines (its inline handler).
#[derive(Clone, Copy)]
enum Handler {
    /// A primitive the VM runs inline (cmp.R's handlers for the language
    /// functions, and `cmpSpecial`'s and `cmpBuiltin`'s for these).
    Known(Known),
    /// `cmpSimpleInternal`: the `.Internal` call (`inner`, a new call
    /// built here) of the builtin internal `internal`.
    SimpleInternal { inner: Sexp, internal: Sexp },
    /// `local(expr)` as `(function() expr)()` (the call built here).
    Local(Sexp),
    /// `cmpBuiltin`: the builtin `prim` called with its evaluated
    /// arguments.
    Builtin(Sexp),
}

/// What a call's function is, for the compiler.
#[derive(Clone, Copy)]
enum Known {
    Special(Special, usize),
    Inline(Inline, usize),
    /// A special that is a function over its call.
    Fn(usize),
}

impl<'g> Comp<'g> {
    fn new(cx: &'g Cx, st: &'g RtState, profile: Profile, bc: Option<Cenv<'g>>) -> Comp<'g> {
        debug_assert_eq!(bc.is_some(), profile == Profile::Bc);
        let b = st.scratch.take();
        Comp {
            cx,
            st,
            profile,
            ins: b.ins,
            consts: b.consts,
            pool: b.pool,
            next: R_FIRST,
            max: R_FIRST,
            max_depth: 0,
            loops: b.loops,
            locals: b.locals,
            sites: 0,
            bc,
            pp: 0,
            ppk: b.ppk,
            site_save: 0,
            arg_codes: b.arg_codes,
            bc_untranslated: None,
            cut_assign: None,
            live_stmt: None,
        }
    }

    /// A `Bc` closure's frame holds cmp.R's locals of its body too (its
    /// compile frame), so that code compiled later for a promise it made
    /// sees them as cmp.R did ([`Cenv::of_promise`]).
    fn add_unit_locals(&self, b: &mut Built) {
        if let Some(c) = &self.bc {
            for &v in c.unit() {
                if !b.locals.contains(&v) {
                    b.locals.push(v);
                }
            }
        }
    }

    fn nil(&self) -> Sexp {
        self.cx.nil().sexp()
    }

    /// The code object of `b` (this compile's [`Comp::root`]):
    /// [`crate::code::build`] of it verified, its buffers then kept for the
    /// isolate's next compile ([`Scratch`]).
    ///
    /// # Safety
    /// As [`crate::code::build`]'s: `source` and `formals` are live.
    unsafe fn build(&mut self, b: Built, source: Sexp, formals: Option<Sexp>) -> Result<Sexp, robigo_heap::NeedGc> {
        let v = verified(b);
        // SAFETY: the caller's contract.
        let r = unsafe { crate::code::build(self.cx, &v, source, formals) };
        // (finish took these out of this compile: they go back cleared, to
        // be given to the scratch when it ends)
        let b = v.into_built();
        fn back<T>(slot: &mut Vec<T>, mut v: Vec<T>) {
            if v.capacity() > slot.capacity() {
                v.clear();
                *slot = v;
            }
        }
        back(&mut self.ins, b.ins);
        back(&mut self.consts, b.consts);
        back(&mut self.locals, b.locals);
        back(&mut self.ppk, b.ppk);
        r
    }

    /// Compile the whole expression: its value is the code object's result.
    unsafe fn root(&mut self, expr: Sexp) -> Result<Built, Need> {
        let r = self.alloc();
        // SAFETY: the caller's contract.
        unsafe { self.expr(expr, r, 1, Tail::Yes)? };
        let mut i = Ins::new(Op::Ret);
        i.b = r;
        self.emit(i);
        Ok(self.finish())
    }

    /// The compiled code. Under the `Bc` profile no instruction is an
    /// evaluation point: bcEval counts and nests once per code object.
    fn finish(&mut self) -> Built {
        fold_vis_into_ret(&mut self.ins, self.profile == Profile::Bc);
        if self.profile == Profile::Bc {
            for i in &mut self.ins {
                i.fl &= !(F_TICK | F_DEPTH);
                i.d = 0;
            }
            self.max_depth = 0;
        }
        Built {
            ins: std::mem::take(&mut self.ins),
            consts: std::mem::take(&mut self.consts),
            nregs: self.max,
            max_depth: self.max_depth,
            profile: self.profile,
            locals: std::mem::take(&mut self.locals),
            nsites: std::mem::take(&mut self.sites),
            ppk: if self.profile == Profile::Eval { std::mem::take(&mut self.ppk) } else { Vec::new() },
        }
    }

    // ---- registers, constants, instructions --------------------------------

    fn alloc(&mut self) -> Reg {
        let r = self.next;
        self.next = self.next.checked_add(1).expect("robigo: an expression too deep to compile");
        self.max = self.max.max(self.next);
        r
    }

    /// `n` consecutive registers.
    fn alloc_n(&mut self, n: usize) -> Reg {
        let r = self.next;
        for _ in 0..n {
            self.alloc();
        }
        r
    }

    fn free_to(&mut self, r: Reg) {
        debug_assert!(r <= self.next);
        self.next = r;
    }

    /// The pool index of `x` (one entry per object).
    fn k(&mut self, x: Sexp) -> u16 {
        if let Some(&i) = self.pool.get(&(x.as_ptr() as usize)) {
            return i;
        }
        let i = u16::try_from(FIRST_CONST + self.consts.len()).expect("robigo: too many constants in one code object");
        self.consts.push(x);
        self.pool.insert(x.as_ptr() as usize, i);
        i
    }

    /// The pool index of the constant `x` cmp.R `putconst`s (a value, a
    /// call): under `Bc`, the first constant before it that is `x` or
    /// `identical()` to it, whichever path put that one in (do_putconst
    /// compares with every constant, `x == y || R_compute_identical(x, y,
    /// 16)`: 0 and -0 are one); else a new one.
    // R: cmp.R:make.codeBuf putconst; eval.c:do_putconst
    fn kv(&mut self, x: Sexp) -> u16 {
        if self.profile != Profile::Bc {
            return self.k(x);
        }
        let cx = self.cx;
        // SAFETY: live constants: the source's, or folds rooted by the
        // compile's round.
        let xr = unsafe { robigo_rtcore::Ref::from_sexp(x, cx) };
        for (j, &y) in self.consts.iter().enumerate() {
            let i = u16::try_from(FIRST_CONST + j).expect("robigo: too many constants in one code object");
            if y == x {
                return i;
            }
            // SAFETY: as above.
            let yr = unsafe { robigo_rtcore::Ref::from_sexp(y, cx) };
            if yr.sexptype() == xr.sexptype()
                && matches!(
                    robigo_rtbuiltins::identical::compute_identical(
                        cx,
                        xr,
                        yr,
                        robigo_rtbuiltins::identical::IDENT_USE_CLOENV
                    ),
                    Ok(true)
                )
            {
                return i;
            }
        }
        self.k(x)
    }

    fn emit(&mut self, i: Ins) -> usize {
        // ROBIGO_POISON_PLANT=1: a planted early clear, the value register
        // emptied one instruction before the Ret that reads it
        #[cfg(feature = "poison-dead")]
        if i.op == Op::Ret
            && i.b >= R_FIRST
            && robigo_rtcore::poison_dead()
            && std::env::var_os("ROBIGO_POISON_PLANT").is_some_and(|v| v == "1")
        {
            let mut c = Ins::new(Op::Clear);
            c.a = i.b;
            self.emit(c);
        }
        if self.profile == Profile::Eval {
            // where the instruction calls out, GNU R's evaluator protections
            // minus R_EvalDepth there, less the root's: its node's offset at
            // its `eval`, less its depth, plus what the AST adds before it
            // runs code (robigo_rtcore's ppacct)
            let kind = match i.op {
                // `eval`'s PROTECT(op), then evalList's result (a builtin)
                // or PROTECT(e) (a special): the primitive's entry, or a
                // closure's arguments promised (`newrho` is the body's)
                Op::Call
                | Op::CallCodes
                | Op::CallBuiltin
                | Op::SpecialFn
                | Op::ComplexAssign
                | Op::Arith
                | Op::Arith1
                | Op::Relop
                | Op::Logic
                | Op::Not => 2,
                // `.Internal`'s PROTECT(op) and PROTECT(e), then do_internal's
                // PROTECT(args) of a builtin internal's evaluated arguments
                // (a special internal's is call_special_internal's)
                Op::Internal if i.c != SPECIAL_INTERNAL => 3,
                Op::Internal => 2,
                // do_for's PROTECT(op), PROTECT(e), then R_compileAndExecute's
                // PROTECT(call), PROTECT(rho), PROTECT(code)
                Op::TopLoop => 5,
                // `eval` of a constant one level up (the VM sets the depth
                // below the node's)
                Op::EvalConst => 1,
                // a resumed statement's `eval` (the VM sets the depth below
                // its own), under `do_begin`'s four
                Op::Resume => 5,
                // a resumed iteration's (`while`'s and `repeat`'s two;
                // `for` adds do_for's other five)
                Op::LoopResume => 3,
                _ => 0,
            };
            self.ppk.push(self.pp - i.d as i32 + kind);
        }
        self.ins.push(i);
        self.ins.len() - 1
    }

    fn pc(&self) -> usize {
        self.ins.len()
    }

    fn patch(&mut self, at: usize, target: usize) {
        self.ins[at].set_target(target);
    }

    /// An evaluation point of depth `d` on `i`.
    fn at_depth(&mut self, mut i: Ins, d: u16, constant: bool) -> Ins {
        if self.profile == Profile::Bc {
            // bcEval neither counts nor nests `eval` per operation: its code
            // object's entry does (`vm::exec`)
            return i;
        }
        i.fl |= F_TICK;
        if !constant {
            i.fl |= F_DEPTH;
        }
        i.d = d;
        self.max_depth = self.max_depth.max(d);
        i
    }

    /// `check_stack_balance` of the primitive `prim` against the top its
    /// guard saved in register `r`, GNU R's top there `p` protections
    /// above the root's.
    fn pp_check(&mut self, r: Reg, p: i32, prim: Sexp) {
        let mut i = Ins::new(Op::PpCheck);
        i.b = r;
        i.e = self.k(prim);
        let at = self.emit(i);
        // (its offset is the site's, not a call out's)
        self.ppk[at] = p;
    }

    fn set_vis(&mut self, on: bool) {
        let mut i = Ins::new(Op::SetVis);
        i.g = on as u16;
        self.emit(i);
    }

    fn clear(&mut self, r: Reg) {
        let mut i = Ins::new(Op::Clear);
        i.a = r;
        self.emit(i);
    }

    /// The expression bcEval is at for the `.Internal` call just emitted
    /// (its wrapper's call, constant `k`): an [`Op::Internal`]'s `g`, or
    /// the `c` of the [`Op::ErrorCall`] of a count its checkArity refuses.
    pub(crate) fn set_internal_site(&mut self, k: u16) {
        let at = self.ins.len() - 1;
        match self.ins[at].op {
            Op::ErrorCall => self.ins[at].c = k,
            op => {
                debug_assert_eq!(op, Op::Internal);
                self.ins[at].g = k;
            }
        }
    }

    /// An error GNU R raises when the special `call` runs (`errorcall` if
    /// `with_call`, `error` otherwise): known at compile time, raised in
    /// order at run time.
    fn error_op(&mut self, call: Sexp, msg: &str, with_call: bool) -> Result<(), Need> {
        let m = self.cx.mk_char(msg.as_bytes(), robigo_rtcore::Encoding::Utf8)?;
        let k = self.k(m.as_ref().sexp());
        let mut i = Ins::new(Op::ErrorCall);
        i.e = self.k(call);
        i.f = k;
        i.g = with_call as u16;
        self.emit(i);
        Ok(())
    }

    // ---- expressions --------------------------------------------------------

    /// Compile `e`, evaluated at depth `d`, its value into `dst`; `tail`:
    /// its value is the code object's (visibility).
    ///
    /// # Safety
    /// `e` is a live object reachable from the source.
    unsafe fn expr(&mut self, e: Sexp, dst: Reg, d: u16, tail: Tail) -> Result<(), Need> {
        // SAFETY: the caller's contract.
        let t = unsafe { e.typ() };
        if self.bc.is_some()
            && matches!(t, SYMSXP | LANGSXP)
            // SAFETY: as above.
            && let Some(v) = unsafe { self.constant_fold(e)? }
        {
            // R: cmp.R:cmp, `cmpConst(ce$value)` of `constantFold(e)`
            // SAFETY: a constant of the source or a fold.
            unsafe { self.const_value(v, dst, d, tail) };
            return Ok(());
        }
        match t {
            SYMSXP => {
                // SAFETY: a live symbol.
                let dd = unsafe { e.gp() } & robigo_rtcore::DDVAL != 0;
                let mut i = Ins::new(if dd { Op::GetDdVar } else { Op::GetVar });
                i.a = dst;
                i.e = self.k(e);
                let mut i = self.at_depth(i, d, false);
                if tail.vis() {
                    i.fl |= F_VIS;
                }
                self.emit(i);
            }
            LANGSXP => {
                // SAFETY: as above.
                unsafe { self.call(e, dst, d, tail)? };
            }
            PROMSXP | BCODESXP if self.bc.is_some() => {
                // R: cmp.R:cmp, "cannot compile promise literals in code"
                // (and byte code literals): a compiler error
                return Err(Need::Fail);
            }
            PROMSXP | DOTSXP | BCODESXP => {
                // what `eval` does with these (force, error, run): `eval`
                // itself counts and raises the depth
                let mut i = Ins::new(Op::EvalConst);
                i.a = dst;
                i.e = self.k(e);
                i.d = d;
                self.max_depth = self.max_depth.max(d);
                self.emit(i);
            }
            _ => {
                // SAFETY: the caller's contract.
                unsafe { self.const_value(e, dst, d, tail) };
            }
        }
        Ok(())
    }

    /// [`Comp::expr`] of an expression whose `eval` starts `p` protections
    /// above the root's (the node it belongs to adds them).
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    unsafe fn expr_at(&mut self, e: Sexp, dst: Reg, d: u16, p: i32, tail: Tail) -> Result<(), Need> {
        let save = std::mem::replace(&mut self.pp, p);
        // SAFETY: the caller's contract.
        let r = unsafe { self.expr(e, dst, d, tail) };
        self.pp = save;
        r
    }

    /// A self-evaluating object `v` (`cmpConst`, `eval` of a constant):
    /// `eval`'s ENSURE_NAMEDMAX(e) does nothing under reference counting
    /// (Defn.h:337-340); bcEval's LDCONST marks a constant that is not a
    /// simple scalar not mutable (eval.c:7853), marked now (the mark is
    /// permanent).
    ///
    /// # Safety
    /// `v` is a live constant of the source or a fold.
    unsafe fn const_value(&mut self, v: Sexp, dst: Reg, d: u16, tail: Tail) {
        let k = self.kv(v);
        let v = self.consts[usize::from(k) - FIRST_CONST];
        // SAFETY: the caller's contract.
        if self.profile == Profile::Bc && unsafe { plain_scalar_const(v) }.is_none() {
            // SAFETY: a live object; REFCNT is a header field.
            unsafe { mark_not_mutable(v) };
        }
        let mut i = Ins::new(Op::LdConst);
        i.a = dst;
        i.e = k;
        let mut i = self.at_depth(i, d, true);
        if tail.vis() {
            i.fl |= F_VIS;
        }
        self.emit(i);
    }

    /// cmp.R's `constantFold(e, cntxt)` under `Bc`: the constant `e` folds
    /// to, or None. A call of a base fold function (`foldFuns`) on
    /// arguments that fold is evaluated between tries ([`crate::fold`]):
    /// until its value is known it does not fold (a request for the next
    /// try). A promise or byte code literal is a compiler error.
    ///
    /// # Safety
    /// `e` is live.
    // R: cmp.R:constantFold, constantFoldCall, constantFoldSym, getFoldFun
    unsafe fn constant_fold(&self, e: Sexp) -> Result<Option<Sexp>, Need> {
        let (cx, st) = (self.cx, self.st);
        let Some(cenv) = &self.bc else { return Ok(None) };
        // SAFETY: the caller's contract; symbols of `cx`'s isolate.
        unsafe {
            match e.typ() {
                SYMSXP => {
                    if !cenv::is_const_name(cx, e) || !cenv.is_base_var(cx, st, e)? {
                        return Ok(None);
                    }
                    Ok(cenv::base_value(cx, e)?.filter(|&v| cenv::check_const(cx, v)))
                }
                LANGSXP => {
                    let fun = e.car();
                    if fun.typ() != SYMSXP || !cenv::is_fold_fun(cx, fun) || !cenv.is_base_var(cx, st, fun)? {
                        return Ok(None);
                    }
                    let Some(ffun) = cenv::base_value(cx, fun)? else { return Ok(None) };
                    if !matches!(ffun.typ(), CLOSXP | BUILTINSXP | SPECIALSXP) {
                        return Ok(None);
                    }
                    let missing = cx.missing_arg().sexp();
                    let mut args = Vec::new();
                    for (a, tag) in cenv::elements(cx, e, true) {
                        if a == missing {
                            return Ok(None);
                        }
                        let Some(v) = self.constant_fold(a)? else { return Ok(None) };
                        args.push((v, tag));
                    }
                    // (every argument passed checkConst: their modes are
                    // constModes)
                    match st.folds.lookup(e) {
                        Some(v) => Ok(v),
                        None => {
                            if st.folds.may_request(st.forced.is_some()) {
                                st.folds.request(crate::fold::FoldReq { call: e, fun: ffun, args });
                            }
                            Ok(None)
                        }
                    }
                }
                PROMSXP | BCODESXP => Err(Need::Fail),
                _ => Ok(cenv::check_const(cx, e).then_some(e)),
            }
        }
    }

    /// The primitive a call's function symbol names in base, if the VM
    /// inlines it.
    ///
    /// # Safety
    /// `f` is live.
    unsafe fn known(&self, f: Sexp) -> Option<Known> {
        // SAFETY: the caller's contract.
        if unsafe { f.typ() } != SYMSXP {
            return None;
        }
        // SAFETY: a live symbol (checked above).
        let off = unsafe { self.st.by_sym.get(f) }?;
        match self.st.defs[off].kind {
            Kind::Special(s) => Some(Known::Special(s, off)),
            // (bcEval's scalar math is the `Bc` profile's alone: `eval` calls
            // the builtin)
            Kind::Builtin(_, Inline::None | Inline::Math1(_)) => None,
            Kind::Builtin(_, inl) => Some(Known::Inline(inl, off)),
            Kind::Fn(_) => Some(Known::Fn(off)),
        }
    }

    /// A call. `e` is a LANGSXP.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    unsafe fn call(&mut self, e: Sexp, dst: Reg, d: u16, tail: Tail) -> Result<(), Need> {
        if self.bc.is_some() {
            // SAFETY: the caller's contract.
            return unsafe { self.bc_call(e, dst, d, tail) };
        }
        // SAFETY: the caller's contract: a live call.
        let (f, args) = unsafe { (e.car(), e.cdr()) };
        // SAFETY: as above.
        let Some(known) = (unsafe { self.known(f) }) else {
            // SAFETY: as above.
            return unsafe { self.generic_call(e, dst, d, true) };
        };
        // the guard is the call's evaluation point
        let off = match known {
            Known::Special(_, o) | Known::Inline(_, o) | Known::Fn(o) => o,
        };
        let prim = self.st.objs[off].bind(self.cx).sexp();
        // eval's check_stack_balance for the builtin or special: its top
        // saved by the guard, compared when its code is done (not for one
        // that evaluates nothing, or never returns)
        let checked = !matches!(
            known,
            Known::Special(
                Special::Function
                    | Special::Quote
                    | Special::Missing
                    | Special::Break
                    | Special::Next
                    | Special::Return,
                _
            )
        );
        let save = checked.then(|| self.alloc());
        let mut g = Ins::new(Op::Guard);
        g.e = self.k(f);
        g.c = self.k(prim);
        g.a = save.unwrap_or(0);
        let g = self.at_depth(g, d, false);
        let guard = self.emit(g);
        let mark = self.next;
        let p = self.pp;
        self.site_save = save.unwrap_or(0);
        // SAFETY: as above.
        unsafe {
            match known {
                Known::Special(s, _) => self.special(s, e, args, dst, d, tail)?,
                Known::Inline(inl, _) => self.inline_builtin(inl, prim, e, args, dst, d, tail)?,
                Known::Fn(_) => {
                    // eval's SPECIALSXP case: the special sets R_Visible
                    let mut i = Ins::new(Op::SpecialFn);
                    i.a = dst;
                    i.e = self.k(e);
                    i.c = self.k(prim);
                    i.fl |= F_DEPTH;
                    i.d = d;
                    self.emit(i);
                }
            }
        }
        self.free_to(mark);
        if let Some(r) = save {
            // `save` is taken after eval's PROTECT(op)
            self.pp_check(r, p + 1, prim);
            self.free_to(r);
        }
        let j = self.emit(Ins::new(Op::Jmp));
        let generic = self.pc();
        self.patch(guard, generic);
        // SAFETY: as above.
        unsafe { self.generic_call(e, dst, d, false)? };
        let end = self.pc();
        self.patch(j, end);
        Ok(())
    }

    /// A call under the `Bc` profile: cmp.R's `cmpCall`. A base function
    /// that `getInlineInfo` inlines is compiled by its handler, frozen or
    /// behind a `BASEGUARD` ([`Op::BaseGuard`]) whose failure evaluates the
    /// call as the AST does; anything else is the generic call.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    // R: cmp.R:cmpCall, tryInline
    unsafe fn bc_call(&mut self, e: Sexp, dst: Reg, d: u16, tail: Tail) -> Result<(), Need> {
        // SAFETY: the caller's contract: a live call.
        let (f, args) = unsafe { (e.car(), e.cdr()) };
        // SAFETY: as above; the compile environment's frames are live.
        let info = unsafe {
            if f.typ() == SYMSXP {
                self.bc.as_ref().unwrap().inline_info(self.cx, self.st, f, true, cenv::Table::Inline)?
            } else {
                None
            }
        };
        let Some(info) = info else {
            // SAFETY: as above.
            return unsafe { self.generic_call(e, dst, d, true) };
        };
        // SAFETY: as above.
        let Some(h) = (unsafe { self.handler(f, e)? }) else {
            // SAFETY: as above.
            return unsafe { self.generic_call(e, dst, d, true) };
        };
        let guard = info.guard.then(|| {
            let mut g = Ins::new(Op::BaseGuard);
            g.e = self.k(e);
            // (a register for the function the guard finds)
            g.a = self.alloc();
            self.free_to(g.a);
            self.emit(g)
        });
        let mark = self.next;
        // SAFETY: as above.
        unsafe {
            match h {
                Handler::Known(Known::Special(s, _)) => self.special(s, e, args, dst, d, tail)?,
                Handler::Known(Known::Inline(inl, off)) => {
                    let prim = self.st.objs[off].bind(self.cx).sexp();
                    self.inline_builtin(inl, prim, e, args, dst, d, tail)?
                }
                Handler::Known(Known::Fn(_)) if f == self.st.syms.switch && self.bc_switch(e, dst, d, tail)? => {}
                Handler::Known(Known::Fn(_))
                    if (f == self.st.syms.subset || f == self.st.syms.subset2) && self.bc_subset(f, e, dst, d)? => {}
                Handler::Known(Known::Fn(_)) if f == self.st.syms.dollar && self.bc_dollar(e, dst, d)? => {}
                Handler::Known(Known::Fn(off)) if self.st.defs[off].name == "log" && self.bc_log(e, dst, d)? => {}
                Handler::Known(Known::Fn(off)) => {
                    // (`cmpSpecial`: `ci <- cb$putconst(call)`, shared as
                    // for `Op::Call` below)
                    let prim = self.st.objs[off].bind(self.cx).sexp();
                    let mut i = Ins::new(Op::SpecialFn);
                    i.a = dst;
                    i.e = self.kv(e);
                    i.c = self.k(prim);
                    i.fl |= F_DEPTH;
                    i.d = d;
                    self.emit(i);
                }
                Handler::SimpleInternal { inner, internal } => {
                    self.builtin_args_call(Op::Internal, internal, inner, inner.cdr(), dst, d, tail)?;
                    // the expression bcEval is at: the wrapper's call
                    // (cmpSimpleInternal sets no location of its own)
                    let ke = self.k(e);
                    self.set_internal_site(ke);
                }
                Handler::Local(ee) => self.call(ee, dst, d, tail)?,
                Handler::Builtin(prim) => self.builtin_args_call(Op::CallBuiltin, prim, e, args, dst, d, tail)?,
            }
        }
        if let Some(g) = guard {
            self.free_to(mark);
            let j = self.emit(Ins::new(Op::Jmp));
            let generic = self.pc();
            self.patch(g, generic);
            // the guard failed: `eval(expr, rho)`, the AST's call (its
            // arguments the AST's promises)
            let mut ev = Ins::new(Op::EvalConst);
            ev.a = dst;
            ev.e = self.k(e);
            self.emit(ev);
            let end = self.pc();
            self.patch(j, end);
        }
        Ok(())
    }

    /// cmp.R's inline handler for the base function `f` of call `e`
    /// (`getInlineHandler(name, "base")`), when it compiles the call: None
    /// when there is none or it declines (the call is compiled as a call).
    ///
    /// # Safety
    /// As for [`Comp::expr`]; `f` is a symbol.
    unsafe fn handler(&mut self, f: Sexp, e: Sexp) -> Result<Option<Handler>, Need> {
        let st = self.st;
        let missing = self.cx.missing_arg().sexp();
        let dots = st.syms.dots;
        // `::` and `:::` (closures of GNU R's base, robigo-rt's specials):
        // cmpMultiColon calls the function with the two names as strings
        // (cmpCallSymFun: GETFUN, the constant arguments, CALL of the call),
        // or declines and the call is the generic one; either way the IR's
        // generic call of the call (whose arguments it reads from the call)
        // R: cmp.R:cmpMultiColon, cmpCallSymFun
        if f == st.syms.double_colon || f == st.syms.triple_colon {
            return Ok(None);
        }
        // `.Internal` of a builtin internal: cmpDotInternalCall's cmpBuiltin,
        // which declines for `...` or an empty argument of the inner call
        // (then the call is the generic one)
        // R: cmp.R:cmpDotInternalCall, cmpBuiltin
        if f == st.syms.dot_internal {
            // SAFETY: the caller's contract: a live call.
            let inner = unsafe { cenv::elements(self.cx, e, true) };
            // (a builtin internal by names.c, whatever robigo-rt's object:
            // `cbind`'s is a special there, eval 10)
            if let [(ic, _)] = inner[..]
                // SAFETY: a live part of the call; a symbol's internal is
                // R_NilValue or a primitive of this isolate.
                && unsafe {
                    ic.typ() == LANGSXP
                        && ic.car().typ() == SYMSXP
                        && matches!(self.cx.internal_raw(ic.car()).typ(), BUILTINSXP | SPECIALSXP)
                        && st.defs[crate::prims::offset(self.cx.internal_raw(ic.car()))].eval % 10 == 1
                }
                // SAFETY: as above.
                && unsafe { cenv::elements(self.cx, ic, true) }.iter().any(|&(a, _)| a == dots || a == missing)
            {
                return Ok(None);
            }
        }
        // the primitives the VM inlines: cmp.R's handlers of the language
        // functions, `cmpSpecial`'s and `cmpBuiltin`'s for the others
        // SAFETY: `f` is a live object of the source.
        let off = unsafe { st.by_sym.get(f) }.filter(|&o| !st.defs[o].internal());
        if let Some(off) = off {
            match st.defs[off].kind {
                Kind::Special(s) => return Ok(Some(Handler::Known(Known::Special(s, off)))),
                Kind::Fn(_) => return Ok(Some(Handler::Known(Known::Fn(off)))),
                Kind::Builtin(_, Inline::None) => {}
                Kind::Builtin(_, inl) => return Ok(Some(Handler::Known(Known::Inline(inl, off)))),
            }
        }
        // SAFETY: the caller's contract.
        let args = unsafe { cenv::elements(self.cx, e, true) };
        if f == st.syms.local {
            // R: cmp.R:setInlineHandler("local"): `(function() e[[2]])()`
            if args.len() != 1 {
                return Ok(None);
            }
            let nil = self.nil();
            // SAFETY: live objects of the source and fresh nodes; nothing
            // collects in this interval.
            let ee = unsafe {
                let l = self.cx.alloc_node_raw(LISTSXP, args[0].0, nil, nil)?;
                let l = self.cx.alloc_node_raw(LISTSXP, nil, l, nil)?;
                let fun = self.cx.alloc_node_raw(LANGSXP, st.syms.function, l, nil)?;
                self.cx.alloc_node_raw(LANGSXP, fun, nil, nil)?
            };
            return Ok(Some(Handler::Local(ee)));
        }
        // SAFETY: a symbol.
        if unsafe { cenv::safe_internal(self.cx, f) }.is_some() {
            // SAFETY: as above.
            return unsafe { self.simple_internal(f, e, &args) };
        }
        // cmp.R's default handlers, set for every name of base by the type
        // of its binding in GNU R's base when the compiler was built:
        // `cmpSpecial` for a special (a row of the registry's table that
        // names.c makes one: `rep`), `cmpBuiltin` for a builtin (declining
        // for `...` or an empty argument). A name bound to a primitive of
        // another name is that primitive's ([`cenv::PRIMITIVE_ALIASES`]:
        // `as.numeric`, base's `as.double`).
        // R: cmp.R:local() (setInlineHandler(s, cmpSpecial), setInlineHandler(b, cmpBuiltin)), cmpBuiltin
        // SAFETY: a symbol.
        let alias = unsafe { cenv::alias_of(self.cx, f) }.and_then(|t| st.defs.iter().position(|d| d.name == t));
        // (an alias is its primitive's, before a row of its own name:
        // robigo-rt's table has an `is.name`, GNU R's base binds
        // `is.symbol` to the name)
        if let Some(o) = alias.or(off) {
            let prim = st.objs[o].bind(self.cx).sexp();
            // SAFETY: a live primitive of this isolate.
            let t = unsafe { prim.typ() };
            if t == SPECIALSXP && matches!(st.defs[o].kind, Kind::Builtin(..)) {
                return Ok(Some(Handler::Known(Known::Fn(o))));
            }
            // (a name of its own: base's binding is the builtin itself)
            let own = || {
                // SAFETY: base's frame and a symbol.
                let b = unsafe { self.base_binding(f) };
                b == Some(prim)
            };
            if t == BUILTINSXP && st.defs[o].eval % 10 == 1 && (alias.is_some() || own()) {
                if args.iter().any(|&(a, _)| a == dots || a == missing) {
                    return Ok(None);
                }
                return Ok(Some(Handler::Builtin(prim)));
            }
        }
        Ok(None)
    }

    /// cmp.R's inline handler for `switch`: `EXPR` (the first argument,
    /// whatever its tag) compiled, then [`Op::Switch`] and its jump tables,
    /// the code of the alternatives after them: an empty one of a numeric
    /// switch is `stop("empty alternative in numeric switch")`, the
    /// default NULL (invisible in tail position). False when cmp.R calls
    /// the special instead (`cmpSpecial`: no `EXPR`, `...` among the
    /// arguments, more than one unnamed alternative).
    ///
    /// # Safety
    /// As for [`Comp::expr`]; `e` is a call of `switch`.
    // R: cmp.R:setInlineHandler("switch"), findActionIndex
    unsafe fn bc_switch(&mut self, e: Sexp, dst: Reg, d: u16, tail: Tail) -> Result<bool, Need> {
        let (cx, st) = (self.cx, self.st);
        let (nil, missing) = (self.nil(), cx.missing_arg().sexp());
        // SAFETY: the caller's contract.
        let args = unsafe { cenv::elements(cx, e, true) };
        if args.is_empty() || args.iter().any(|&(a, _)| a == st.syms.dots) {
            return Ok(false);
        }
        let cases = &args[1..];
        let miss: Vec<bool> = cases.iter().map(|&(a, _)| a == missing).collect();
        // names(cases), "" for an untagged one; `switch(x, 1)`'s is ""
        let tag_name = |t: Sexp| {
            if t == nil {
                return cx.blank_string().as_ref().sexp();
            }
            // SAFETY: a tag is a symbol: its PRINTNAME, the isolate's CHARSXP.
            unsafe { cx.printname_raw(t) }
        };
        let mut nm: Option<Vec<Sexp>> =
            cases.iter().any(|&(_, t)| t != nil).then(|| cases.iter().map(|&(_, t)| tag_name(t)).collect());
        if nm.is_none() && cases.len() == 1 {
            nm = Some(vec![cx.blank_string().as_ref().sexp()]);
        }
        let blank = |c: Sexp| {
            // SAFETY: CHARSXPs.
            unsafe { robigo_rtcore::Ref::from_sexp(c, cx) }.as_char().is_some_and(|c| c.bytes().is_empty())
        };
        let have_dflt = match &nm {
            Some(nm) => {
                let ndflt = nm.iter().filter(|&&c| blank(c)).count();
                if ndflt > 1 {
                    // "punt back to interpreted version for now to get
                    // runtime error message for multiple defaults"
                    return Ok(false);
                }
                ndflt > 0
            }
            None => false,
        };
        // the labels: each alternative's (the empty ones share one), then
        // the default's
        const MISS: usize = usize::MAX;
        let n = cases.len();
        let dflt = n;
        let labels: Vec<usize> = (0..=n).map(|i| if i < n && miss[i] { MISS } else { i }).collect();
        // the character table: each name's label (`findActionIndex`: the
        // first alternative from its first occurrence on that is not
        // empty), the default "" last
        let mut table: Vec<(Sexp, usize)> = Vec::new();
        if let Some(nm) = &nm {
            let same = |a: Sexp, b: Sexp| {
                // SAFETY: CHARSXPs.
                let (a, b) = unsafe { (robigo_rtcore::Ref::from_sexp(a, cx), robigo_rtcore::Ref::from_sexp(b, cx)) };
                a.as_char().map(|c| c.bytes()) == b.as_char().map(|c| c.bytes())
            };
            let action = |name: Sexp| {
                let start = nm.iter().position(|&c| same(c, name)).unwrap();
                (start..n).find(|&i| !miss[i]).unwrap_or(n)
            };
            for &c in nm {
                if !blank(c) && !table.iter().any(|&(u, _)| same(u, c)) {
                    table.push((c, labels[action(c)]));
                }
            }
            let blank_s = cx.blank_string().as_ref().sexp();
            table.push((blank_s, if have_dflt { labels[action(blank_s)] } else { dflt }));
        }
        // EXPR
        let r = self.alloc();
        // SAFETY: the caller's contract.
        unsafe { self.expr(args[0].0, r, d + 1, Tail::No)? };
        let names = if table.is_empty() {
            nil
        } else {
            let v = cx.alloc_vector_raw(STRSXP, table.len())?;
            for (i, &(c, _)) in table.iter().enumerate() {
                // SAFETY: a fresh vector; CHARSXPs of the source or the
                // blank string.
                unsafe { v.set_elt(i, c) };
            }
            v
        };
        let mut sw = Ins::new(Op::Switch);
        sw.b = r;
        sw.e = self.k(e);
        sw.c = self.k(names);
        sw.f = u16::try_from(labels.len()).expect("robigo: a 'switch' with too many alternatives");
        self.emit(sw);
        self.free_to(r);
        let tabs = self.pc();
        for _ in 0..table.len() + labels.len() {
            self.emit(Ins::new(Op::Jmp));
        }
        // the code of each label, and the jumps to the end
        let mut at = vec![0usize; n + 1];
        let mut miss_at = 0;
        let mut ends = Vec::new();
        if miss.iter().any(|&m| m) {
            miss_at = self.pc();
            let stop = cx.install(b"stop")?.as_ref().sexp();
            let m = cx.mk_char(b"empty alternative in numeric switch", robigo_rtcore::Encoding::Native)?;
            let msg = cx.alloc_vector_raw(STRSXP, 1)?;
            // SAFETY: fresh objects; nothing collects in this interval.
            let call = unsafe {
                msg.set_elt(0, m.as_ref().sexp());
                let a = cx.alloc_node_raw(LISTSXP, msg, nil, nil)?;
                cx.alloc_node_raw(LANGSXP, stop, a, nil)?
            };
            // (rooted by the pool: the call's code keeps it)
            self.k(call);
            // SAFETY: a fresh call.
            unsafe { self.expr(call, dst, d + 1, tail)? };
            // (it does not return: the default follows, as in cmp.R; in
            // tail position cmp.R's call RETURNs, a jump to the end)
            if tail.is_tail() {
                ends.push(self.emit(Ins::new(Op::Jmp)));
            }
        }
        at[dflt] = self.pc();
        self.ld_nil(dst, tail.ret());
        if tail.is_tail() {
            self.set_vis(false);
        }
        ends.push(self.emit(Ins::new(Op::Jmp)));
        for (i, &(a, _)) in cases.iter().enumerate() {
            if !miss[i] {
                at[i] = self.pc();
                // SAFETY: the caller's contract.
                unsafe { self.expr(a, dst, d + 1, tail)? };
                ends.push(self.emit(Ins::new(Op::Jmp)));
            }
        }
        let end = self.pc();
        for j in ends {
            self.patch(j, end);
        }
        let target = |l: usize| if l == MISS { miss_at } else { at[l] };
        for (j, &(_, l)) in table.iter().enumerate() {
            self.patch(tabs + j, target(l));
        }
        for (j, &l) in labels.iter().enumerate() {
            self.patch(tabs + table.len() + j, target(l));
        }
        Ok(true)
    }

    /// cmp.R's inline handlers for `[` and `[[` (`cmpSubsetDispatch`, or
    /// for `...`, an empty argument, a tag or no index `cmpDispatch`): the
    /// object compiled, then [`Op::StartSubset`] (dispatch), the indices
    /// compiled (`cmpIndices`, `cmpBuiltinArgs(missingOK = TRUE)`: an empty
    /// one R_MissingArg, a symbol read by `GETVAR_MISSOK`), then
    /// [`Op::DfltSubset`]. False when cmp.R calls the special instead
    /// (`cmpSpecial`: `...`, no object).
    ///
    /// # Safety
    /// As for [`Comp::expr`]; `e` is a call of `f`, `[` or `[[`.
    // R: cmp.R:setInlineHandler("["), setInlineHandler("[["), cmpSubsetDispatch, cmpDispatch, cmpIndices
    unsafe fn bc_subset(&mut self, f: Sexp, e: Sexp, dst: Reg, d: u16) -> Result<bool, Need> {
        let (cx, st) = (self.cx, self.st);
        let (nil, missing) = (self.nil(), cx.missing_arg().sexp());
        // SAFETY: the caller's contract.
        let args = unsafe { cenv::elements(cx, e, true) };
        let named = args.iter().any(|&(_, t)| t != nil);
        if args.iter().any(|&(a, _)| a == st.syms.dots) || args.is_empty() || args[0].0 == missing {
            return Ok(false);
        }
        let n = args.len();
        let base = self.alloc_n(if named { 2 * n } else { n });
        // (cmp(oe, cb, ncntxt))
        // SAFETY: the caller's contract.
        unsafe { self.expr(args[0].0, base, d + 1, Tail::No)? };
        let mut start = Ins::new(Op::StartSubset);
        start.a = dst;
        start.b = base;
        start.e = self.k(e);
        start.c = 1 + (f == st.syms.subset2) as u16;
        let start = self.emit(start);
        for (j, &(a, _)) in args.iter().enumerate().skip(1) {
            // SAFETY: as above.
            unsafe { self.missok_arg(a, base + j as Reg, d + 1)? };
        }
        if named {
            // (STARTSUBSET's SETCALLARG_TAG of the object, then
            // cmpBuiltinArgs' tags)
            for (j, &(_, t)) in args.iter().enumerate() {
                let mut l = Ins::new(Op::LdConst);
                l.a = base + (n + j) as Reg;
                l.e = self.k(t);
                self.emit(l);
            }
        }
        let mut df = Ins::new(Op::DfltSubset);
        df.a = dst;
        df.b = base;
        df.c = n as u16;
        df.e = self.k(e);
        df.g = 1 + (f == st.syms.subset2) as u16;
        if named {
            df.fl |= F_NAMES;
        }
        self.emit(df);
        let end = self.pc();
        self.patch(start, end);
        self.free_to(base);
        Ok(true)
    }

    /// An argument compiled with `missingOK` (`cmpIndices`'
    /// `cmp(indices[[i]], cb, cntxt, TRUE)`, `cmpBuiltinArgs(missingOK =
    /// TRUE)`): an empty argument is R_MissingArg (`DOMISSING`), a symbol
    /// that does not fold is read by `GETVAR_MISSOK`.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    // R: cmp.R:cmpSym, cmpBuiltinArgs
    unsafe fn missok_arg(&mut self, a: Sexp, r: Reg, d: u16) -> Result<(), Need> {
        let missing = self.cx.missing_arg().sexp();
        if a == missing {
            let mut l = Ins::new(Op::LdConst);
            l.a = r;
            l.e = self.k(missing);
            self.emit(l);
            return Ok(());
        }
        // SAFETY: the caller's contract.
        unsafe {
            if a.typ() == SYMSXP && self.constant_fold(a)?.is_none() {
                let mut i = Ins::new(Op::GetVarMissOk);
                i.a = r;
                i.e = self.k(a);
                let i = self.at_depth(i, d, false);
                self.emit(i);
                return Ok(());
            }
            self.expr(a, r, d, Tail::No)
        }
    }

    /// cmp.R's inline handler for `log`: the argument (and the base)
    /// compiled, then [`Op::Math1`]'s `LOG` (`LOGBASE`), `log`'s evaluated
    /// form its slow path. False when cmp.R calls the special instead
    /// (`...` or an empty argument, a name, not one or two arguments).
    ///
    /// # Safety
    /// As for [`Comp::expr`]; `e` is a call of `log`.
    // R: cmp.R:setInlineHandler("log")
    unsafe fn bc_log(&mut self, e: Sexp, dst: Reg, d: u16) -> Result<bool, Need> {
        let (cx, st) = (self.cx, self.st);
        let (nil, missing) = (self.nil(), cx.missing_arg().sexp());
        // SAFETY: the caller's contract.
        let args = unsafe { cenv::elements(cx, e, true) };
        if args.is_empty() || args.len() > 2 || args.iter().any(|&(a, t)| a == st.syms.dots || a == missing || t != nil)
        {
            return Ok(false);
        }
        let n = args.len();
        let r = self.alloc_n(n);
        for (j, &(a, _)) in args.iter().enumerate() {
            // SAFETY: the caller's contract.
            unsafe { self.expr(a, r + j as Reg, d + 1, Tail::No)? };
        }
        let kind = if n == 1 { crate::ir::MATH_LOG } else { crate::ir::MATH_LOGBASE };
        self.math1_op(e, None, dst, r, n as u16, kind, d);
        self.free_to(r);
        Ok(true)
    }

    /// [`Op::Math1`] of kind `kind` on the `n` operands from `x` for the
    /// call `e`, the builtin `prim` its slow path (None: `log`'s), the
    /// value into `dst`.
    #[allow(clippy::too_many_arguments)]
    fn math1_op(&mut self, e: Sexp, prim: Option<Sexp>, dst: Reg, x: Reg, n: u16, kind: u16, d: u16) {
        let mut i = Ins::new(Op::Math1);
        i.a = dst;
        i.b = x;
        i.c = n;
        i.e = self.k(e);
        i.f = prim.map_or(0, |p| self.k(p));
        i.g = kind;
        i.fl |= F_DEPTH;
        i.d = d;
        self.emit(i);
    }

    /// cmp.R's inline handler for `$`: the object compiled, then
    /// [`Op::Dollar`] with the member as a symbol (a string member one of
    /// its name). False when cmp.R calls the special instead (`...`, not
    /// two arguments, a member that is no name).
    ///
    /// # Safety
    /// As for [`Comp::expr`]; `e` is a call of `$`.
    // R: cmp.R:setInlineHandler("$")
    unsafe fn bc_dollar(&mut self, e: Sexp, dst: Reg, d: u16) -> Result<bool, Need> {
        let cx = self.cx;
        // SAFETY: the caller's contract.
        let args = unsafe { cenv::elements(cx, e, true) };
        if args.iter().any(|&(a, _)| a == self.st.syms.dots) || args.len() != 2 {
            return Ok(false);
        }
        let m = args[1].0;
        // SAFETY: a live object of the source.
        let mr = unsafe { robigo_rtcore::Ref::from_sexp(m, cx) };
        let sym = match mr.as_vector::<robigo_rtcore::Char<'_>>() {
            Some(v) if mr.sexptype() == STRSXP && v.len() == 1 => {
                let c = v.get(0);
                if c.as_ref().sexp() == cx.na_string().as_ref().sexp() {
                    // `e[[3]] != ""` is NA: `if (NA)`, a compiler error
                    return Err(Need::Fail);
                }
                if c.bytes().is_empty() { m } else { cx.install(c.bytes())?.as_ref().sexp() }
            }
            _ => m,
        };
        // SAFETY: a live object.
        if unsafe { sym.typ() } != SYMSXP {
            return Ok(false);
        }
        let r = self.alloc();
        // SAFETY: the caller's contract.
        unsafe { self.expr(args[0].0, r, d + 1, Tail::No)? };
        let mut i = Ins::new(Op::Dollar);
        i.a = dst;
        i.b = r;
        i.e = self.k(e);
        i.c = self.k(sym);
        self.emit(i);
        self.free_to(r);
        Ok(true)
    }

    /// Base's binding of `sym`, raw.
    ///
    /// # Safety
    /// `sym` is a symbol.
    unsafe fn base_binding(&self, sym: Sexp) -> Option<Sexp> {
        let cx = self.cx;
        // SAFETY: the caller's contract; base is immortal.
        let s = unsafe { robigo_rtcore::Ref::from_sexp(sym, cx) }.as_sym()?;
        let e = crate::dispatch::env_ref(cx.base_env())?;
        match cx.frame_binding(e, s) {
            robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
                robigo_rtcore::BindingValue::Value(v) => Some(v.sexp()),
                _ => None,
            },
            _ => None,
        }
    }

    /// `cmpSimpleInternal(e, cb, cntxt)` for the call `e` of the wrapper
    /// `f` (its arguments `args`): the call replaced by the `.Internal`
    /// call the wrapper's body makes, its formals substituted by the
    /// call's arguments (matched as `match.call` matches them) or their
    /// defaults (`inlineSimpleInternalCall`), when the definition is a
    /// simple internal and the call matches it (`checkCall`).
    ///
    /// # Safety
    /// As for [`Comp::handler`].
    // R: cmp.R:cmpSimpleInternal, inlineSimpleInternalCall, checkCall, cmpDotInternalCall, cmpBuiltin
    unsafe fn simple_internal(&mut self, f: Sexp, e: Sexp, args: &[(Sexp, Sexp)]) -> Result<Option<Handler>, Need> {
        let st = self.st;
        let cx = self.cx;
        let nil = self.nil();
        let missing = cx.missing_arg().sexp();
        if args.iter().any(|&(a, _)| a == st.syms.dots) {
            return Ok(None);
        }
        // SAFETY: the caller's contract.
        let def = unsafe { self.bc.as_ref().unwrap().fun_def(cx, st, f)? };
        // checkCall's NA for a binding that is no function: `if (NA)`, a
        // compiler error
        let Some(def) = def else { return Err(Need::Fail) };
        // SAFETY: a live function.
        unsafe {
            if def.typ() != CLOSXP {
                // (a primitive's `args()` matches or not; it is no simple
                // internal either way)
                return Ok(None);
            }
            // checkCall: match.call(def, e)
            let Some(matched) = crate::args::match_static(cx, def.car(), e) else { return Ok(None) };
            // inlineSimpleInternalCall: no empty argument
            if args.iter().any(|&(a, _)| a == missing) {
                return Ok(None);
            }
            let Some(icall) = cenv::simple_internal(cx, st, def) else { return Ok(None) };
            let forms = cenv::elements(cx, def.car(), false);
            let iargs = cenv::elements(cx, icall, true);
            let mut sub = Vec::with_capacity(iargs.len());
            for &(a, tag) in &iargs {
                let v = if a.typ() == SYMSXP {
                    // cenv[[name]]: the matched actual, else the default
                    match forms.iter().position(|&(_, t)| t == a) {
                        Some(i) => matched[i].unwrap_or(forms[i].0),
                        None => a,
                    }
                } else {
                    a
                };
                // cmpBuiltin declines an empty argument
                if v == missing || v == st.syms.dots {
                    return Ok(None);
                }
                sub.push((v, tag));
            }
            let mut l = nil;
            for &(v, tag) in sub.iter().rev() {
                l = cx.alloc_node_raw(LISTSXP, v, l, tag)?;
            }
            let ifun = icall.car();
            let inner = cx.alloc_node_raw(LANGSXP, ifun, l, nil)?;
            Ok(Some(Handler::SimpleInternal { inner, internal: cx.internal_raw(ifun) }))
        }
    }

    /// The plan of the complex assignment `lhs <- value` (or `<<-`) that
    /// cmp.R's `cmpComplexAssign` compiles
    /// ([`crate::bcassign`]): the target symbol, the getter calls of the
    /// target's nested places from the inside out, the setter calls from
    /// the outside in, each by its inline handler (`$`, `[`, `[[` and their
    /// setters, frozen when base's) or as a call. The argument expressions
    /// the calls evaluate are compiled now, with this code
    /// ([`Comp::promise_codes`]'s rule).
    ///
    /// # Safety
    /// `lhs` (a call `check_assign` accepts) and `value` are live parts of
    /// the source.
    // R: cmp.R:cmpComplexAssign, flattenPlace, cmpGetterCall, cmpSetterCall, getAssignFun
    unsafe fn bc_assign_plan(&mut self, lhs: Sexp, value: Sexp) -> Result<Sexp, Need> {
        use crate::bcassign as ba;
        let cx = self.cx;
        let st = self.st;
        let nil = self.nil();
        let tmp = st.syms.tmpval;
        // SAFETY: the caller's contract; every object made here is reached
        // from the plan, which the code's pool keeps, and nothing collects.
        unsafe {
            // getAssignedVar: the target
            let call = cx.alloc_node_raw(LANGSXP, st.syms.assign, nil, nil)?;
            let call = {
                let l = cx.alloc_node_raw(LISTSXP, lhs, nil, nil)?;
                call.set_cdr(l);
                call
            };
            let sym = cenv::assigned_var(cx, call)?;
            // flattenPlace: each place with `*tmp*` for its object
            let mut places = Vec::new();
            let mut orig = Vec::new();
            let mut p = lhs;
            while p.typ() == LANGSXP {
                let second = p.cdr();
                if second == nil {
                    return Err(Need::Fail);
                }
                orig.push(p);
                let cell = cx.alloc_node_raw(LISTSXP, tmp, second.cdr(), second.tag())?;
                places.push(cx.alloc_node_raw(LANGSXP, p.car(), cell, nil)?);
                p = second.car();
            }
            if p.typ() != SYMSXP {
                return Err(Need::Fail);
            }
            let n = places.len();
            let getters = cx.alloc_vector_raw(VECSXP, n - 1)?;
            for (j, i) in (1..n).rev().enumerate() {
                let step = self.bc_getter_step(places[i], orig[i])?;
                getters.set_elt(j, step);
            }
            let setters = cx.alloc_vector_raw(VECSXP, n)?;
            for i in 0..n {
                let vexpr = if i == 0 { value } else { st.syms.vtmp };
                let step = self.bc_setter_step(places[i], orig[i], vexpr)?;
                setters.set_elt(i, step);
            }
            let plan = cx.alloc_vector_raw(VECSXP, ba::PLAN_LEN)?;
            plan.set_elt(ba::P_SYM, sym);
            plan.set_elt(ba::P_GETTERS, getters);
            plan.set_elt(ba::P_SETTERS, setters);
            Ok(plan)
        }
    }

    /// A getter step of [`Comp::bc_assign_plan`] (`cmpGetterCall`).
    ///
    /// # Safety
    /// As for [`Comp::bc_assign_plan`]; `place` is the place with `*tmp*`.
    // R: cmp.R:cmpGetterCall, tryGetterInline, the getter handlers of `$`, `[`, `[[`
    unsafe fn bc_getter_step(&mut self, place: Sexp, origplace: Sexp) -> Result<Sexp, Need> {
        use crate::bcassign as ba;
        // SAFETY: the caller's contract.
        unsafe {
            let fun = place.car();
            let mut kind = ba::K_GENERIC;
            let mut what = fun;
            if fun.typ() == SYMSXP
                && self.bc.as_ref().unwrap().inline_info(self.cx, self.st, fun, false, cenv::Table::Getter)?.is_some()
                && let Some((k, w)) = self.subset_handler(fun, place, false)?
            {
                kind = k;
                what = w;
            }
            self.precompile_args(place.cdr().cdr())?;
            ba::make_step(self.cx, kind, place, what, None, origplace, false, None)
        }
    }

    /// A setter step of [`Comp::bc_assign_plan`] (`cmpSetterCall`), the
    /// value `vexpr` (the right-hand side's expression, or `*vtmp*`).
    ///
    /// # Safety
    /// As for [`Comp::bc_getter_step`].
    // R: cmp.R:cmpSetterCall, getAssignFun, trySetterInline, the setter handlers of `$<-`, `[<-`, `[[<-`
    unsafe fn bc_setter_step(&mut self, place: Sexp, origplace: Sexp, vexpr: Sexp) -> Result<Sexp, Need> {
        use crate::bcassign as ba;
        let cx = self.cx;
        let st = self.st;
        let nil = self.nil();
        // SAFETY: the caller's contract.
        unsafe {
            // getAssignFun: `f<-`, or `pkg::f<-` (anything else: "invalid
            // function in complex assignment", a compiler error)
            let fun = place.car();
            let afun = match fun.typ() {
                SYMSXP => crate::assign::assign_fun_symbol_raw(cx, fun)?,
                LANGSXP => {
                    let el = cenv::elements(cx, fun, false);
                    let colon = el.len() == 3
                        && (el[0].0 == st.syms.double_colon || el[0].0 == st.syms.triple_colon)
                        && el[1].0.typ() == SYMSXP
                        && el[2].0.typ() == SYMSXP;
                    if !colon {
                        return Err(Need::Fail);
                    }
                    let f = crate::assign::assign_fun_symbol_raw(cx, el[2].0)?;
                    let c3 = cx.alloc_node_raw(LISTSXP, f, nil, nil)?;
                    let c2 = cx.alloc_node_raw(LISTSXP, el[1].0, c3, nil)?;
                    cx.alloc_node_raw(LANGSXP, el[0].0, c2, nil)?
                }
                _ => return Err(Need::Fail),
            };
            // acall: afun(`*tmp*`, args, value = vexpr); cexpr: the same on
            // the original object, the expression bcEval is at
            let mut args = cenv::elements(cx, place.cdr().cdr(), false);
            // the setter handler of `@<-` (base's): a slot name that is a
            // symbol becomes a string in the setter's call, not in cexpr
            // R: cmp.R:setSetterInlineHandler("@<-")
            if afun == cx.install(b"@<-")?.as_ref().sexp()
                && self.bc.as_ref().unwrap().inline_info(cx, st, afun, false, cenv::Table::Setter)?.is_some()
                && args.len() == 1
                && place.cdr().car() != st.syms.dots
                && place.cdr().car() != cx.missing_arg().sexp()
                && args[0].0.typ() == SYMSXP
                && args[0].0 != st.syms.dots
            {
                // PRINTNAME(sym): the isolate's CHARSXP (a symbol's CAR is
                // the arena's private copy, which no R value may hold)
                let name = cx.printname_raw(args[0].0);
                let v = cx.alloc_vector_raw(STRSXP, 1)?;
                v.set_elt(0, name);
                args[0].0 = v;
            }
            let with_value = |first: Sexp, second_tag: Sexp| -> Result<Sexp, Need> {
                let mut l = cx.alloc_node_raw(LISTSXP, vexpr, nil, st.syms.value)?;
                for &(a, t) in args.iter().rev() {
                    l = cx.alloc_node_raw(LISTSXP, a, l, t)?;
                }
                let l = cx.alloc_node_raw(LISTSXP, first, l, second_tag)?;
                Ok(cx.alloc_node_raw(LANGSXP, afun, l, nil)?)
            };
            let second_tag = place.cdr().tag();
            let acall = with_value(st.syms.tmpval, second_tag)?;
            let cexpr = with_value(origplace.cdr().car(), origplace.cdr().tag())?;
            // R_getBCInterpreterExpression's inflation of a setter call:
            // `f(x, args) <- vexpr`, what error() reports
            let (report, fin) =
                if afun.typ() == SYMSXP { (ba::inflate_assignment_call(cx, cexpr)?, true) } else { (cexpr, false) };
            let mut kind = ba::K_GENERIC;
            let mut what = afun;
            if afun.typ() == SYMSXP
                && self.bc.as_ref().unwrap().inline_info(cx, st, afun, false, cenv::Table::Setter)?.is_some()
                && let Some((k, w)) = self.subset_handler(afun, place, true)?
            {
                kind = k;
                what = w;
            }
            self.precompile_args(place.cdr().cdr())?;
            ba::make_step(cx, kind, acall, what, Some(vexpr), report, fin, None)
        }
    }

    /// The inline handler cmp.R has for the getter (`[`, `[[`, `$`) or
    /// setter (`[<-`, `[[<-`, `$<-`) `f` of `place`, when it compiles the
    /// call: the step kind and what it needs (the member name of `$`).
    /// None: a call (`anyDots`, an unnamed `$` member).
    ///
    /// # Safety
    /// `f` is a symbol and `place` a live place.
    // R: cmp.R: setGetterInlineHandler/setSetterInlineHandler of `$`, `[`, `[[`; cmpGetterDispatch, cmpSetterDispatch, cmpSubsetGetterDispatch, cmpSubassignDispatch
    unsafe fn subset_handler(&mut self, f: Sexp, place: Sexp, setter: bool) -> Result<Option<(u8, Sexp)>, Need> {
        use crate::bcassign as ba;
        let st = self.st;
        let cx = self.cx;
        let missing = cx.missing_arg().sexp();
        // SAFETY: the caller's contract.
        unsafe {
            let (dollar, sub, sub2) = if setter {
                (st.syms.dollar_gets, st.syms.subassign, st.syms.subassign2)
            } else {
                (st.syms.dollar, st.syms.subset, st.syms.subset2)
            };
            let el = cenv::elements(cx, place, true);
            let any_dots = el.iter().any(|&(a, _)| a == st.syms.dots);
            let dots_or_missing = any_dots || el.iter().any(|&(a, _)| a == missing);
            let named = el.iter().any(|&(_, t)| t != self.nil());
            if f == dollar {
                if any_dots || el.len() != 2 {
                    return Ok(None);
                }
                let m = el[1].0;
                let m = match m.typ() {
                    STRSXP if m.std_len() == 1 => {
                        cx.install((*(m.std_data() as *const Sexp)).char_bytes())?.as_ref().sexp()
                    }
                    SYMSXP => m,
                    _ => return Ok(None),
                };
                return Ok(Some((ba::K_DOLLAR, m)));
            }
            if f == sub || f == sub2 {
                let simple = !(dots_or_missing || named || el.len() < 2);
                if simple {
                    return Ok(Some((if f == sub { ba::K_SUB } else { ba::K_SUB2 }, f)));
                }
                if any_dots {
                    return Ok(None);
                }
                return Ok(Some((if f == sub { ba::K_SUB_PUNT } else { ba::K_SUB2_PUNT }, f)));
            }
            Ok(None)
        }
    }

    /// The code of each argument expression of the list `args` that is a
    /// call, compiled now with this code's compile environment and kept on
    /// it (cmpCallArgs' and cmpBuiltinArgs' `cmp` of each); a promise or
    /// byte-code literal is a compiler error.
    ///
    /// # Safety
    /// `args` is a live pairlist of the source.
    unsafe fn precompile_args(&mut self, args: Sexp) -> Result<(), Need> {
        // SAFETY: the caller's contract.
        unsafe {
            for (a, _) in cenv::elements(self.cx, args, false) {
                match a.typ() {
                    PROMSXP | BCODESXP => return Err(Need::Fail),
                    LANGSXP => self.arg_code(a)?,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Compile the call `a` (an argument) with this code's compile
    /// environment and keep the code on it.
    ///
    /// # Safety
    /// `a` is a live call of the source.
    unsafe fn arg_code(&mut self, a: Sexp) -> Result<(), Need> {
        let cenv = self.bc.take().expect("robigo: argument code outside the Bc profile");
        let mut sub = Comp::new(self.cx, self.st, Profile::Bc, Some(cenv));
        // SAFETY: the caller's contract.
        let r = unsafe { sub.root(a) };
        self.bc = sub.bc.take();
        let b = r?;
        // SAFETY: as above.
        unsafe {
            let code = sub.build(b, a, None)?;
            crate::code::cache_on(a, code);
            // (its promises, and those of the arguments inside it, run in
            // this code's frame)
            self.arg_codes.push(code);
            self.arg_codes.append(&mut sub.arg_codes);
        }
        Ok(())
    }

    /// cmp.R's `cmpCallArgs` for the generic call `e` under `Bc`: the code
    /// of each argument that becomes a promise (a call) is compiled now,
    /// with this code's compile environment (`genCode(a, pcntxt)` before
    /// `MAKEPROM`), and kept on the argument beside any AST code
    /// ([`crate::code::cache_on`]), so forcing the promise runs code with
    /// the decisions of this compile. A call of `bquote` keeps its
    /// arguments' expressions (`maybeNSESymbols`); a promise or byte-code
    /// literal is a compiler error.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    // R: cmp.R:cmpCallArgs, cmpCallSymFun
    unsafe fn promise_codes(&mut self, e: Sexp) -> Result<(), Need> {
        let st = self.st;
        // SAFETY: the caller's contract.
        unsafe {
            let nse = e.car() == st.syms.bquote;
            for (a, _) in cenv::elements(self.cx, e, true) {
                match a.typ() {
                    PROMSXP | BCODESXP => return Err(Need::Fail),
                    LANGSXP if !nse => self.arg_code(a)?,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The code of a function the `Bc` code makes (cmp.R's `function`
    /// handler: its body compiled with the code's compile environment and
    /// a frame of its own, `make.functionContext`), or None when cmp.R
    /// leaves it to `do_function` (it may call `browser()`): then the
    /// closure is made uncompiled.
    ///
    /// # Safety
    /// `formals` (a pairlist) and `body` are live parts of the source.
    // R: cmp.R:setInlineHandler("function")
    unsafe fn nested_function(&mut self, formals: Sexp, body: Sexp) -> Result<Option<Sexp>, Need> {
        let mut cenv = self.bc.take().expect("robigo: a nested function outside the Bc profile");
        // SAFETY: the caller's contract.
        if !unsafe { function_context(self.cx, self.st, &mut cenv, formals, body, None)? } {
            cenv.pop_unit();
            self.bc = Some(cenv);
            return Ok(None);
        }
        let mut sub = Comp::new(self.cx, self.st, Profile::Bc, Some(cenv));
        // SAFETY: as above.
        let mut b = unsafe { sub.root(body)? };
        sub.add_unit_locals(&mut b);
        let mut cenv = sub.bc.take().unwrap();
        cenv.pop_unit();
        self.bc = Some(cenv);
        // SAFETY: as above; the constants are the source's, immortal or
        // made in this interval.
        unsafe {
            let code = sub.build(b, body, Some(formals))?;
            index_codes(self.cx, code, &sub.arg_codes, None);
            Ok(Some(code))
        }
    }

    /// The generic call (`eval`'s LANGSXP case): the function found at run
    /// time, then promises, evaluated arguments or a compiled special.
    /// `tick`: this is the call's evaluation point (false behind a guard,
    /// which ticked).
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    unsafe fn generic_call(&mut self, e: Sexp, dst: Reg, d: u16, tick: bool) -> Result<(), Need> {
        if self.bc.is_some() {
            // SAFETY: the caller's contract.
            unsafe { self.promise_codes(e)? };
        }
        // SAFETY: the caller's contract.
        let f = unsafe { e.car() };
        let rf = self.alloc();
        // SAFETY: as above.
        if unsafe { f.typ() } == SYMSXP {
            let mut i = Ins::new(Op::GetFun);
            i.a = rf;
            i.e = self.k(f);
            i.c = self.k(e);
            let i = if tick {
                self.at_depth(i, d, false)
            } else {
                i.fl |= F_DEPTH;
                i.d = d;
                i
            };
            self.emit(i);
        } else {
            if tick {
                let i = self.at_depth(Ins::new(Op::Nop), d, false);
                self.emit(i);
            }
            // R: eval.c:eval, `op = eval(CAR(e), rho)`
            // SAFETY: as above.
            unsafe { self.expr(f, rf, d + 1, Tail::No)? };
        }
        let mut i = Ins::new(Op::Call);
        i.a = dst;
        i.b = rf;
        // cmp.R's `cb$putconst(call)`: under `Bc` the first constant
        // `identical()` to the call (do_putconst). A special that compares
        // its call with the closure's body by identity sees which cell it
        // got: `Exec`/`Tailcall`'s checkTailPosition (tailcall.rs), however
        // the function is named at the call.
        i.e = self.kv(e);
        // a match-plan cache entry per site (none past the 16-bit range)
        i.c = if self.sites < NO_SITE {
            self.sites += 1;
            self.sites - 1
        } else {
            NO_SITE
        };
        i.fl |= F_DEPTH | F_CLR_B;
        i.d = d;
        self.emit(i);
        self.free_to(rf);
        Ok(())
    }

    // ---- the specials ---------------------------------------------------------

    /// The arguments of a call as a vector (values and tags).
    ///
    /// # Safety
    /// `args` is a live pairlist.
    unsafe fn arglist(&self, mut args: Sexp) -> Vec<(Sexp, Sexp)> {
        let nil = self.nil();
        let mut v = Vec::new();
        // SAFETY: the caller's contract.
        unsafe {
            while args != nil && matches!(args.typ(), LISTSXP | LANGSXP | DOTSXP) {
                v.push((args.car(), args.tag()));
                args = args.cdr();
            }
        }
        v
    }

    /// Compile special `s` (its guard emitted) for the call `e`.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    #[allow(clippy::too_many_arguments)]
    unsafe fn special(&mut self, s: Special, e: Sexp, args: Sexp, dst: Reg, d: u16, tail: Tail) -> Result<(), Need> {
        let nil = self.nil();
        // (the call's own: compiling its operands replaces the field's)
        let mut site_save = std::mem::replace(&mut self.site_save, 0);
        // SAFETY: the caller's contract.
        let a = unsafe { self.arglist(args) };
        let n = a.len();
        // SAFETY: the operands are live parts of the source.
        unsafe {
            match s {
                Special::Brace => {
                    // R: eval.c:do_begin. Under `Eval`, a statement boundary
                    // at the start and after each statement, its value still
                    // in its register: where a write into the source made
                    // the code stale, the rest is `do_begin`'s on the live
                    // list (Op::Resume)
                    let live = self.profile == Profile::Eval;
                    let mut resumes = Vec::new();
                    // (at the start too: the guard's evaluation tick before
                    // it may run R code, a finalizer)
                    if live {
                        resumes.push(self.resume(e, NO_REG, dst, d + 1));
                    }
                    if n == 0 {
                        self.ld_nil(dst, tail.vis());
                    }
                    // each statement under PROTECT(op), PROTECT(e),
                    // PROTECT(srcrefs), PROTECT(R_Srcref)
                    let p = self.pp + 4;
                    let mut cell = args;
                    for (i, &(st, _)) in a.iter().enumerate() {
                        if i + 1 == n {
                            self.expr_at(st, dst, d + 1, p, tail)?;
                            if live {
                                resumes.push(self.resume(cell, dst, dst, d + 1));
                            }
                        } else {
                            let r = self.alloc();
                            // under `Eval`, a write that cuts the list after
                            // this statement makes it the last, so its
                            // visibility is kept as the last one's is (an
                            // assignment's by its Resume)
                            let ctx = match tail {
                                Tail::Yes if live => Tail::Live,
                                _ if live => tail,
                                _ => Tail::No,
                            };
                            self.cut_assign = None;
                            self.live_stmt = Some(st);
                            self.expr_at(st, r, d + 1, p, ctx)?;
                            if live {
                                let at = self.resume(cell, r, dst, d + 1);
                                if self.cut_assign == Some(st) {
                                    self.ins[at].c = 1;
                                }
                                resumes.push(at);
                            }
                            self.clear(r);
                            self.free_to(r);
                        }
                        cell = cell.cdr();
                    }
                    let end = self.pc();
                    for at in resumes {
                        self.patch(at, end);
                    }
                }
                Special::If => {
                    // R: eval.c:do_if
                    if !(n == 2 || n == 3) {
                        return self.unimplemented(e, "an 'if' call that does not have 2 or 3 arguments");
                    }
                    // R: cmp.R:setInlineHandler("if"): a test that folds to
                    // TRUE or FALSE compiles the branch it takes alone. A
                    // test whose fold is not known yet compiles neither
                    // branch in this try (it is redone with the fold's
                    // value, [`crate::fold`]): a fold in the branch the test
                    // drops is never evaluated.
                    let asked = self.st.folds.requests();
                    let t = self.constant_fold(a[0].0)?;
                    if t.is_none() && self.st.folds.requests() > asked {
                        return Ok(());
                    }
                    if let Some(t) = t
                        && let t = robigo_rtcore::Ref::from_sexp(t, self.cx)
                        && t.sexptype() == LGLSXP
                        && t.len() == Some(1)
                        && let v = robigo_rtbuiltins::util::as_logical(self.cx, t)
                        && v != i32::MIN
                    {
                        if v != 0 {
                            self.expr(a[1].0, dst, d + 1, tail)?;
                        } else if n == 3 {
                            self.expr(a[2].0, dst, d + 1, tail)?;
                        } else {
                            self.ld_nil(dst, tail.ret());
                            if tail.is_tail() {
                                self.set_vis(false);
                            }
                        }
                        return Ok(());
                    }
                    // the test and the branch under PROTECT(op), PROTECT(e)
                    let p = self.pp + 2;
                    let r = self.alloc();
                    self.expr_at(a[0].0, r, d + 1, p, Tail::No)?;
                    let mut br = Ins::new(Op::BrFalse);
                    br.b = r;
                    br.fl |= F_CLR_B;
                    br.e = self.k(e);
                    let br = self.emit(br);
                    self.free_to(r);
                    self.expr_at(a[1].0, dst, d + 1, p, tail)?;
                    let j = self.emit(Ins::new(Op::Jmp));
                    let els = self.pc();
                    self.patch(br, els);
                    if n == 3 {
                        self.expr_at(a[2].0, dst, d + 1, p, tail)?;
                    } else {
                        self.ld_nil(dst, tail.ret());
                        if tail.is_tail() {
                            // "case of no 'else' so return invisible NULL"
                            self.set_vis(false);
                        }
                    }
                    let end = self.pc();
                    self.patch(j, end);
                }
                Special::For => {
                    // R: eval.c:do_for
                    if n != 3 {
                        let msg = arity_msg(n, "for", 3);
                        return self.error_op(e, &msg, true);
                    }
                    let sym = a[0].0;
                    if sym.typ() != SYMSXP {
                        return self.error_op(e, "non-symbol loop variable", true);
                    }
                    let top = self.top_loop(e, dst, d);
                    if !self.locals.contains(&sym) {
                        self.locals.push(sym);
                    }
                    let state = self.alloc_n(FOR_STATE);
                    // the sequence under PROTECT(op), PROTECT(e), then
                    // do_for's PROTECT(args), PROTECT(rho)
                    self.expr_at(a[1].0, state, d + 1, self.pp + 4, Tail::No)?;
                    let mut prep = Ins::new(Op::ForPrep);
                    prep.a = state;
                    prep.e = self.k(sym);
                    prep.c = self.k(e);
                    self.emit(prep);
                    let lp = self.loop_begin(e);
                    let mut step = Ins::new(Op::ForStep);
                    step.a = state;
                    step.e = self.k(sym);
                    step.c = self.k(e);
                    let step = self.emit(step);
                    self.loops.last_mut().unwrap().breaks.push(step);
                    let resume = self.loop_resume(a[2].0, None, d);
                    if let Some(at) = resume {
                        // a resumed body under do_for's seven, not the two
                        // of `while` and `repeat`
                        self.ppk[at] += 5;
                    }
                    // and PROTECT(val), PROTECT(cell), PROTECT(v)
                    self.loop_body(a[2].0, d, self.pp + 7)?;
                    self.patch_loop_resume(resume);
                    self.loop_end(lp);
                    let mut done = Ins::new(Op::ForDone);
                    done.a = state;
                    self.emit(done);
                    for r in state..state + FOR_STATE as Reg {
                        self.clear(r);
                    }
                    self.free_to(state);
                    self.top_loop_end(top);
                    self.loop_value(dst, tail);
                }
                Special::While => {
                    // R: eval.c:do_while
                    if n != 2 {
                        let msg = arity_msg(n, "while", 2);
                        return self.error_op(e, &msg, true);
                    }
                    let top = self.top_loop(e, dst, d);
                    let lp = self.loop_begin(e);
                    let r = self.alloc();
                    let resume = self.loop_resume(a[1].0, Some((e, r)), d);
                    self.expr_at(a[0].0, r, d + 1, self.pp + 2, Tail::No)?;
                    let mut br = Ins::new(Op::BrFalse);
                    br.b = r;
                    br.fl |= F_CLR_B;
                    br.e = self.k(e);
                    let br = self.emit(br);
                    self.free_to(r);
                    self.loops.last_mut().unwrap().breaks.push(br);
                    self.loop_body(a[1].0, d, self.pp + 2)?;
                    self.patch_loop_resume(resume);
                    self.loop_end(lp);
                    self.top_loop_end(top);
                    self.loop_value(dst, tail);
                }
                Special::Repeat => {
                    // R: eval.c:do_repeat
                    if n != 1 {
                        let msg = arity_msg(n, "repeat", 1);
                        return self.error_op(e, &msg, true);
                    }
                    let top = self.top_loop(e, dst, d);
                    let lp = self.loop_begin(e);
                    let resume = self.loop_resume(a[0].0, None, d);
                    self.loop_body(a[0].0, d, self.pp + 2)?;
                    self.patch_loop_resume(resume);
                    self.loop_end(lp);
                    self.top_loop_end(top);
                    self.loop_value(dst, tail);
                }
                Special::Break | Special::Next => {
                    // R: eval.c:do_break
                    let name = if s == Special::Break { "break" } else { "next" };
                    if n != 0 {
                        let msg = arity_msg(n, name, 0);
                        return self.error_op(e, &msg, true);
                    }
                    match self.loops.last() {
                        Some(l) => {
                            // the temporaries of the expressions the jump
                            // abandons are dead (D9)
                            let (regs, start) = (l.regs, l.start);
                            if self.bc.is_none() {
                                // do_break's longjmp to the loop's context
                                // restores the protect stack's top
                                self.emit(Ins::new(Op::PpRestore));
                            }
                            if self.next > regs {
                                let mut c = Ins::new(Op::ClearRange);
                                c.a = regs;
                                c.b = self.next;
                                self.emit(c);
                            }
                            if s == Special::Break {
                                let at = self.emit(Ins::new(Op::Jmp));
                                self.loops.last_mut().unwrap().breaks.push(at);
                            } else {
                                // `next` is a back-edge: its safepoint and,
                                // in the `Bc` profile, bcEval's GOTO poll
                                let mut j = Ins::new(Op::BackJmp);
                                j.set_target(start);
                                self.emit(j);
                            }
                        }
                        None => {
                            let mut j = Ins::new(Op::BreakJump);
                            j.g = if s == Special::Break { 2 } else { 1 };
                            self.emit(j);
                        }
                    }
                    // (never reached: the result register stays as it is)
                    self.ld_nil(dst, false);
                }
                Special::Return => {
                    // R: eval.c:do_return
                    if n > 1 {
                        return self.error_op(e, "multi-argument returns are not permitted", true);
                    }
                    // the returned value's visibility is the function's:
                    // under `Eval`, `eval` of the `return` call sets
                    // R_Visible, then `eval` of its operand does; under `Bc`,
                    // cmp.R compiles the operand in a non-tail context
                    // (make.nonTailCallContext), so it is visible as bcEval
                    // leaves it at RETURN
                    let r = self.alloc();
                    let ctx = if self.profile == Profile::Bc { Tail::Ret } else { Tail::Yes };
                    if n == 1 {
                        self.expr_at(a[0].0, r, d + 1, self.pp + 2, ctx)?;
                    } else {
                        self.ld_nil(r, true);
                    }
                    let mut i = Ins::new(Op::Return);
                    i.b = r;
                    self.emit(i);
                    self.free_to(r);
                    self.ld_nil(dst, false);
                }
                Special::Function => {
                    // R: eval.c:do_function
                    if n < 2 {
                        return self.error_op(e, "incorrect number of arguments to \"function\"", false);
                    }
                    let formals = a[0].0;
                    if !check_formals(formals, nil) {
                        return self.error_op(e, "invalid formal argument list for \"function\"", false);
                    }
                    let mut i = Ins::new(Op::MkClosure);
                    i.a = dst;
                    i.e = self.k(formals);
                    i.f = self.k(a[1].0);
                    // its srcref (the call's 4th element, keep.source = TRUE):
                    // the closure's "srcref" attribute, as do_function and
                    // MAKECLOSURE set it
                    // R: eval.c:do_function, MAKECLOSURE; cmp.R's "function" handler
                    if n > 2 && a[2].0 != nil {
                        i.b = self.k(a[2].0);
                    }
                    if self.bc.is_some() {
                        // MAKECLOSURE: a closure of compiled code
                        if let Some(code) = self.nested_function(formals, a[1].0)? {
                            i.c = self.k(code);
                        }
                    } else if crate::jit::installed_literal(e) {
                        // a closure of base's installed byte code
                        i.g = 1;
                    }
                    if tail.vis() {
                        i.fl |= F_VIS;
                    }
                    self.emit(i);
                }
                Special::Assign | Special::AssignEq | Special::Assign2 => {
                    // R: eval.c:do_set
                    let name = match s {
                        Special::Assign => "<-",
                        Special::AssignEq => "=",
                        _ => "<<-",
                    };
                    if n != 2 {
                        let msg = format!("incorrect number of arguments to \"{name}\"");
                        return self.error_op(e, &msg, false);
                    }
                    let lhs = a[0].0;
                    let sym = match lhs.typ() {
                        SYMSXP => lhs,
                        STRSXP if lhs.std_len() >= 1 => {
                            // installTrChar(STRING_ELT(lhs, 0))
                            // (under `Bc` a target of one string is cmpAssign's,
                            // which names it at the compile; checkAssign leaves
                            // any other to cmpSpecial, the special at run time)
                            let compiled = self.bc.is_some() && lhs.std_len() == 1;
                            match cenv::install_tr(self.cx, *(lhs.std_data() as *const Sexp))? {
                                Ok((sym, warning)) => {
                                    if let Some(w) = warning {
                                        if compiled {
                                            // cmpAssign's as.name(getAssignedVar(e,
                                            // cntxt)): at the compile, the code
                                            // silent
                                            let at = cenv::CompileWarning::AsName;
                                            self.st.compile_warnings.push(at, w);
                                        } else {
                                            // do_set's, before the value
                                            self.translate_warning(e, lhs, d)?;
                                        }
                                    }
                                    sym
                                }
                                Err((msg, warning)) => {
                                    // (install's error after the warning)
                                    if compiled {
                                        // cmpAssign's as.name fails: an error
                                        // inside cmp.R, so no compile (run
                                        // uncompiled, it raises both again)
                                        if let Some(w) = warning {
                                            self.st.compile_warnings.push(cenv::CompileWarning::AsName, w);
                                        }
                                        return Err(Need::Fail);
                                    }
                                    if warning.is_some() {
                                        self.translate_warning(e, lhs, d)?;
                                    }
                                    return self.error_op(e, &msg, false);
                                }
                            }
                        }
                        LANGSXP if self.bc.is_some() && crate::bcassign::check_assign(self.cx, lhs) => {
                            // cmp.R's cmpComplexAssign: the right-hand side,
                            // then the getter and setter calls of its plan
                            let plan = self.bc_assign_plan(lhs, a[1].0)?;
                            // (in `return()`'s operand the value's
                            // visibility stands where no step sets it:
                            // `x$a <- 2`)
                            self.expr(a[1].0, dst, d + 1, tail.value())?;
                            let mut i = Ins::new(Op::BcAssign);
                            i.a = dst;
                            i.b = dst;
                            i.e = self.k(plan);
                            i.g = (s == Special::Assign2) as u16;
                            self.emit(i);
                            // cmp.R's INVISIBLE after a tail assignment
                            // (bcEval's ENDASSIGN leaves R_Visible)
                            if tail.is_tail() {
                                self.set_vis(false);
                            }
                            return Ok(());
                        }
                        LANGSXP => {
                            // applydefine: the right-hand side first, then
                            // the assignment through `*tmp*` (a target
                            // cmp.R's checkAssign refuses runs the special,
                            // do_set, in compiled code too)
                            self.expr_at(a[1].0, dst, d + 1, self.pp + 2, Tail::No)?;
                            let mut i = Ins::new(Op::ComplexAssign);
                            i.a = dst;
                            i.b = dst;
                            i.e = self.k(e);
                            i.g = match s {
                                Special::Assign => 1,
                                Special::Assign2 => 2,
                                _ => 3,
                            };
                            i.fl |= F_DEPTH;
                            i.d = d;
                            self.emit(i);
                            // the frame binds `*tmp*` and, for `<-` and `=`,
                            // the target (EnsureLocal)
                            let tmp = self.st.syms.tmpval;
                            if !self.locals.contains(&tmp) {
                                self.locals.push(tmp);
                            }
                            let mut t = lhs;
                            while t.typ() == LANGSXP && t.cdr() != nil {
                                t = t.cdr().car();
                            }
                            if t.typ() == SYMSXP && s != Special::Assign2 && !self.locals.contains(&t) {
                                self.locals.push(t);
                            }
                            return Ok(());
                        }
                        _ => return self.error_op(e, "invalid (do_set) left-hand side to assignment", true),
                    };
                    // do_set's `eval(CADR(args), rho)` under PROTECT(op),
                    // PROTECT(e)
                    self.expr_at(a[1].0, dst, d + 1, self.pp + 2, tail.value())?;
                    let mut i = Ins::new(if s == Special::Assign2 { Op::SetVar2 } else { Op::SetVar });
                    i.a = dst;
                    i.e = self.k(sym);
                    if s != Special::Assign2 && !self.locals.contains(&sym) {
                        self.locals.push(sym);
                    }
                    self.emit(i);
                    // do_set's R_Visible = FALSE, and cmp.R's INVISIBLE after
                    // a tail assignment: observed only in the tail (bcEval's
                    // SETVAR leaves R_Visible, so does `return()`'s operand);
                    // a `{`'s earlier statement, on its Resume
                    match tail {
                        Tail::Live if self.live_stmt == Some(e) => self.cut_assign = Some(e),
                        Tail::Yes | Tail::Live => self.set_vis(false),
                        _ => {}
                    }
                }
                Special::Quote => {
                    // R: eval.c:do_quote
                    if n != 1 {
                        let msg = arity_msg(n, "quote", 1);
                        return self.error_op(e, &msg, true);
                    }
                    if let Some(msg) = check1arg(a[0].1, nil, "expr") {
                        return self.error_op(e, &msg, true);
                    }
                    let mut i = Ins::new(Op::Quote);
                    i.a = dst;
                    i.e = self.k(a[0].0);
                    if tail.vis() {
                        i.fl |= F_VIS;
                    }
                    self.emit(i);
                }
                Special::Missing => {
                    // R: envir.c:do_missing
                    if n != 1 {
                        let msg = arity_msg(n, "missing", 1);
                        return self.error_op(e, &msg, true);
                    }
                    if let Some(msg) = check1arg(a[0].1, nil, "x") {
                        return self.error_op(e, &msg, true);
                    }
                    let x = a[0].0;
                    let sym = match x.typ() {
                        SYMSXP => x,
                        // installTrChar(STRING_ELT(x, 0))
                        STRSXP if x.std_len() == 1 => {
                            match cenv::install_tr(self.cx, *(x.std_data() as *const Sexp))? {
                                Ok((sym, warning)) => {
                                    if warning.is_some() {
                                        // do_missing's, at each evaluation in
                                        // both profiles (cmp.R leaves `missing`
                                        // to it)
                                        self.translate_warning(e, x, d)?;
                                    }
                                    sym
                                }
                                Err((msg, warning)) => {
                                    // (install's error after the warning)
                                    if warning.is_some() {
                                        self.translate_warning(e, x, d)?;
                                    }
                                    return self.error_op(e, &msg, false);
                                }
                            }
                        }
                        _ => return self.error_op(e, "invalid use of 'missing'", true),
                    };
                    let mut i = Ins::new(Op::Missing);
                    i.a = dst;
                    i.e = self.k(sym);
                    i.f = self.k(e);
                    if tail.vis() {
                        i.fl |= F_VIS;
                    }
                    self.emit(i);
                }
                Special::AndAnd | Special::OrOr => {
                    // R: logic.c:do_logic2
                    let op = if s == Special::AndAnd { 1 } else { 2 };
                    if n != 2 {
                        let msg = format!("'{}' operator requires 2 arguments", if op == 1 { "&&" } else { "||" });
                        return self.error_op(e, &msg, false);
                    }
                    // both operands under PROTECT(op), PROTECT(e)
                    self.expr_at(a[0].0, dst, d + 1, self.pp + 2, Tail::No)?;
                    // in the tail, the value visible on both paths (bcEval's
                    // AND1ST and AND2ND set R_Visible), the second operand
                    // reached or not (`a && return(x)`)
                    let vis = if tail.vis() { F_VIS } else { 0 };
                    let mut i1 = Ins::new(Op::AndOr1);
                    i1.a = dst;
                    i1.b = dst;
                    i1.c = op;
                    i1.e = self.k(e);
                    i1.fl |= vis;
                    let i1 = self.emit(i1);
                    let r = self.alloc();
                    self.expr_at(a[1].0, r, d + 1, self.pp + 2, Tail::No)?;
                    let mut i2 = Ins::new(Op::AndOr2);
                    i2.a = dst;
                    i2.b = r;
                    i2.c = op;
                    i2.e = self.k(e);
                    i2.fl |= F_CLR_B | vis;
                    self.emit(i2);
                    self.free_to(r);
                    let end = self.pc();
                    self.patch(i1, end);
                }
                Special::Internal => {
                    // R: names.c:do_internal
                    if n != 1 {
                        let msg = arity_msg(n, ".Internal", 1);
                        return self.error_op(e, &msg, true);
                    }
                    let inner = a[0].0;
                    if !matches!(inner.typ(), NILSXP | LISTSXP | LANGSXP | DOTSXP) {
                        return self.error_op(e, "invalid .Internal() argument", true);
                    }
                    let fun = if inner == nil { nil } else { inner.car() };
                    if fun.typ() != SYMSXP {
                        return self.error_op(e, "invalid .Internal() argument", true);
                    }
                    let internal = self.cx.internal_raw(fun);
                    if internal == nil {
                        let name = String::from_utf8_lossy(fun.sym_name_bytes()).into_owned();
                        let msg = format!("there is no .Internal function '{name}'");
                        return self.error_op(e, &msg, true);
                    }
                    let fn_special = internal.typ() == SPECIALSXP
                        && matches!(self.st.defs[crate::prims::offset(internal)].kind, Kind::Fn(_));
                    if fn_special {
                        // a special .Internal over its call (`NextMethod`):
                        // do_internal calls it on the inner call, its
                        // arguments unevaluated, R_Visible on before
                        let mut i = Ins::new(Op::SpecialFn);
                        i.a = dst;
                        i.e = self.k(inner);
                        i.c = self.k(internal);
                        i.fl |= F_DEPTH;
                        i.d = d;
                        // do_internal around it (its check, PROTECT(args))
                        i.g = 1;
                        self.emit(i);
                    } else if internal.typ() == SPECIALSXP {
                        // a special .Internal of the registry (lapply,
                        // vapply): the inner call's argument expressions,
                        // unevaluated
                        let mut i = Ins::new(Op::Internal);
                        i.a = dst;
                        i.e = self.k(inner);
                        i.f = self.k(internal);
                        i.fl |= F_DEPTH;
                        i.d = d;
                        i.c = SPECIAL_INTERNAL;
                        self.emit(i);
                    } else if internal.typ() != BUILTINSXP {
                        return self.unimplemented(e, "an .Internal that is not a builtin or special");
                    } else {
                        if site_save == 0 && self.bc.is_none() {
                            // do_internal's `save`, with no guard to take it
                            // (`.Internal` reached through a value)
                            let r = self.alloc();
                            let mut i = Ins::new(Op::PpSave);
                            i.a = r;
                            self.emit(i);
                            site_save = r;
                        }
                        // the internal's arguments are evaluated as a
                        // builtin's (evalList), for the inner call
                        self.builtin_args_call(Op::Internal, internal, inner, inner.cdr(), dst, d, tail)?;
                        let at = self.ins.len() - 1;
                        if self.ins[at].op == Op::Internal && self.ins[at].c == u16::MAX {
                            // evaluated at run time (`...` or an empty
                            // argument): through call_special_internal, over
                            // the `.Internal` call itself, which evalList's
                            // errors name (do_internal's `evalList(args, env,
                            // call, 0)`); the builtin `f` tells it from a
                            // special. (Its protect weight was taken when it
                            // was emitted, as a builtin internal's.)
                            self.ins[at].c = SPECIAL_INTERNAL;
                            self.ins[at].e = self.k(e);
                        }
                    }
                    if !fn_special {
                        // the expression bcEval is at: the `.Internal` call
                        // (a closure wrapper's: error() reports the
                        // function's call)
                        let ke = self.k(e);
                        self.set_internal_site(ke);
                    }
                    if site_save != 0 && internal.typ() == BUILTINSXP {
                        // do_internal's own check_stack_balance, its save
                        // taken under eval's PROTECT(op), PROTECT(e) (a
                        // special internal's is the VM's, in both profiles)
                        self.pp_check(site_save, self.pp + 2, internal);
                    }
                }
            }
        }
        Ok(())
    }

    fn ld_nil(&mut self, dst: Reg, vis: bool) {
        let nil = self.nil();
        let mut i = Ins::new(Op::LdConst);
        i.a = dst;
        i.e = self.k(nil);
        if vis {
            i.fl |= F_VIS;
        }
        self.emit(i);
    }

    /// A shape of a special this package does not compile yet: an error
    /// that says so when it runs (AGENTS rule 6).
    fn unimplemented(&mut self, e: Sexp, what: &str) -> Result<(), Need> {
        let msg = format!("robigo: {what} is not implemented yet");
        self.error_op(e, &msg, false)
    }

    /// translateToNative's warning for a name given as a string (`"x" <-
    /// v`, `missing("x")`) that does not translate, raised when the code
    /// runs, as GNU R's special raises it at each evaluation: the internal
    /// `as.vector(name, "symbol")` of `names`' first element alone, whose
    /// installTrChar gives that warning (`warning()`: the context's call),
    /// its value dropped. A builtin call of the existing kind ([`Op::Internal`]),
    /// so no instruction of its own; the symbol the code uses is the one
    /// made now, with the same escapes. The expression bcEval is at is the
    /// special's call `site`, so under `Bc` the warning names the
    /// function's call, as GNU R's names it inside the special.
    ///
    /// # Safety
    /// `site` and `names` are live parts of the source, `names` a character
    /// vector.
    // R: envir.c:do_set, do_missing; sysutils.c:installTrChar
    unsafe fn translate_warning(&mut self, site: Sexp, names: Sexp, d: u16) -> Result<(), Need> {
        let nil = self.nil();
        let cx = self.cx;
        let asvec = cx.install(b"as.vector")?.as_ref().sexp();
        // SAFETY: fresh nodes holding live objects; nothing collects in
        // this interval, and the call is kept in the code's pool.
        unsafe {
            let internal = cx.internal_raw(asvec);
            if internal == nil {
                return Err(Need::Fail);
            }
            let mode = cx.mk_string(b"symbol", robigo_rtcore::Encoding::Utf8)?.sexp();
            // the first name alone, with no attributes: do_set and
            // do_missing read STRING_ELT(x, 0) and never dispatch, where
            // as.vector would on a classed `x`
            let name = cx.alloc_vector_raw(STRSXP, 1)?;
            name.set_elt(0, *(names.std_data() as *const Sexp));
            let l = cx.alloc_node_raw(LISTSXP, mode, nil, nil)?;
            let args = cx.alloc_node_raw(LISTSXP, name, l, nil)?;
            let inner = cx.alloc_node_raw(LANGSXP, asvec, args, nil)?;
            let mark = self.next;
            let r = self.alloc();
            self.builtin_args_call(Op::Internal, internal, inner, args, r, d, Tail::No)?;
            let k = self.k(site);
            self.set_internal_site(k);
            self.free_to(mark);
        }
        Ok(())
    }

    /// Under `Eval`, a loop evaluated in the global environment is
    /// compiled as it starts and run as byte code (`do_for`'s
    /// `R_compileAndExecute`): [`Op::TopLoop`], which jumps past the loop's
    /// own code when it ran it. None under `Bc` (the loop is compiled).
    fn top_loop(&mut self, e: Sexp, dst: Reg, d: u16) -> Option<usize> {
        if self.bc.is_some() {
            return None;
        }
        let mut i = Ins::new(Op::TopLoop);
        i.a = dst;
        i.e = self.k(e);
        i.d = d;
        Some(self.emit(i))
    }

    /// The end of a loop's own code: where [`Op::TopLoop`] lands.
    fn top_loop_end(&mut self, top: Option<usize>) {
        if let Some(t) = top {
            let here = self.pc();
            self.patch(t, here);
        }
    }

    /// Under `Eval`, a statement boundary of a `{` ([`Op::Resume`]): the
    /// rest from the cell after `prev`, the value so far in `val`, the
    /// `{`'s value into `dst`. Its target is patched to the `{`'s end.
    fn resume(&mut self, prev: Sexp, val: Reg, dst: Reg, d: u16) -> usize {
        let mut i = Ins::new(Op::Resume);
        i.a = dst;
        i.b = val;
        i.e = self.k(prev);
        i.d = d;
        self.emit(i)
    }

    /// Under `Eval`, a loop iteration boundary ([`Op::LoopResume`]) for the
    /// body `body` (and, for `while`, its call and the register its
    /// condition goes to). None under `Bc`: byte code keeps running.
    fn loop_resume(&mut self, body: Sexp, cond: Option<(Sexp, Reg)>, d: u16) -> Option<usize> {
        if self.profile != Profile::Eval {
            return None;
        }
        let mut i = Ins::new(Op::LoopResume);
        i.e = self.k(body);
        i.d = d + 1;
        match cond {
            Some((call, r)) => {
                // `do_while`'s `args`, read when the loop starts: its CAR is
                // the condition each iteration evaluates
                // SAFETY: a live call of the source.
                i.b = self.k(unsafe { call.cdr() });
                i.c = self.k(call);
                i.a = r;
            }
            None => {
                // a register for the body's value: a temporary of the
                // loop's own
                let r = self.alloc();
                i.c = NO_CONST;
                i.a = r;
                self.free_to(r);
            }
        }
        Some(self.emit(i))
    }

    /// [`Comp::loop_resume`]'s target: the loop's back-edge, the
    /// instruction [`Comp::loop_body`] emitted last.
    fn patch_loop_resume(&mut self, at: Option<usize>) {
        if let Some(at) = at {
            let back = self.pc() - 1;
            self.patch(at, back);
        }
    }

    /// The start of a loop: its context and region.
    fn loop_begin(&mut self, e: Sexp) -> usize {
        let mut i = Ins::new(Op::Loop);
        i.e = self.k(e);
        // registers from here on are the body's temporaries, dead when a
        // `break` or `next` jump lands at the loop's context
        i.a = self.next;
        let lp = self.emit(i);
        self.loops.push(LoopLabels { start: lp + 1, breaks: Vec::new(), regs: self.next });
        lp
    }

    /// A loop's body (its value discarded, its `eval` `p` protections
    /// above the root's) and its back-edge.
    unsafe fn loop_body(&mut self, body: Sexp, d: u16, p: i32) -> Result<(), Need> {
        let r = self.alloc();
        // SAFETY: the caller's contract.
        unsafe { self.expr_at(body, r, d + 1, p, Tail::No)? };
        self.clear(r);
        self.free_to(r);
        let start = self.loops.last().unwrap().start;
        let mut j = Ins::new(Op::BackJmp);
        j.set_target(start);
        self.emit(j);
        Ok(())
    }

    fn loop_end(&mut self, lp: usize) {
        let l = self.loops.pop().unwrap();
        let end = self.emit(Ins::new(Op::LoopEnd));
        for b in l.breaks {
            self.patch(b, end);
        }
        self.patch(lp, end);
    }

    /// A loop's value: invisible NULL (`for`, `while`, `repeat` are
    /// `PRIMPRINT` 1).
    fn loop_value(&mut self, dst: Reg, tail: Tail) {
        // (`return()`'s operand: cmp.R's non-tail LDNULL sets R_Visible)
        self.ld_nil(dst, tail.ret());
        if tail.is_tail() {
            self.set_vis(false);
        }
    }

    // ---- builtins -------------------------------------------------------------

    /// An inlined builtin (its guard emitted).
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    #[allow(clippy::too_many_arguments)]
    unsafe fn inline_builtin(
        &mut self,
        inl: Inline,
        prim: Sexp,
        e: Sexp,
        args: Sexp,
        dst: Reg,
        d: u16,
        tail: Tail,
    ) -> Result<(), Need> {
        // SAFETY: the caller's contract.
        let a = unsafe { self.arglist(args) };
        let dots = self.st.syms.dots;
        let plain = a.iter().all(|&(v, _)| v != dots && v != robigo_rtcore_missing(self.cx));
        let (op, n_ok) = match inl {
            Inline::Arith(op) => (
                op,
                a.len() == 2
                    || (a.len() == 1
                        && matches!(
                            op,
                            robigo_rtbuiltins::kernel::arith::PLUSOP | robigo_rtbuiltins::kernel::arith::MINUSOP
                        )),
            ),
            Inline::Relop(op) => (op, a.len() == 2),
            Inline::Logic(op) => (op, a.len() == 2),
            Inline::Not => (0, a.len() == 1),
            Inline::Paren => (0, a.len() == 1),
            // (bcEval's: the AST's `do_math1` wraps the C functions)
            Inline::Math1(_) => (0, a.len() == 1 && self.bc.is_some()),
            Inline::None => unreachable!(),
        };
        if !(plain && n_ok) {
            // evalList and the builtin itself (its arity errors)
            // SAFETY: as above.
            return unsafe { self.builtin_args_call(Op::CallBuiltin, prim, e, args, dst, d, tail) };
        }
        if let Inline::Math1(kind) = inl {
            // cmp.R's cmpMath1 / cmpPrim1: the operand, then the op
            let r = self.alloc();
            // SAFETY: as above.
            unsafe { self.expr(a[0].0, r, d + 1, Tail::No)? };
            self.math1_op(e, Some(prim), dst, r, 1, kind, d);
            self.free_to(r);
            return Ok(());
        }
        if inl == Inline::Paren {
            // evalList's first argument, under eval's PROTECT(op)
            // SAFETY: as above.
            unsafe { self.expr_at(a[0].0, dst, d + 1, self.pp + 1, Tail::No)? };
            if tail.vis() {
                self.set_vis(true);
            }
            return Ok(());
        }
        let r = self.alloc_n(a.len());
        let p = self.pp;
        for (i, &(v, _)) in a.iter().enumerate() {
            // evalList: under eval's PROTECT(op), and its head's after the
            // first value
            self.pp = p + 1 + (i > 0) as i32;
            // SAFETY: as above.
            let r = unsafe { self.operand(v, r + i as Reg, d + 1) };
            self.pp = p;
            r?;
        }
        let mut i = Ins::new(match inl {
            Inline::Arith(_) if a.len() == 1 => Op::Arith1,
            Inline::Arith(_) => Op::Arith,
            Inline::Relop(_) => Op::Relop,
            Inline::Logic(_) => Op::Logic,
            _ => Op::Not,
        });
        i.a = dst;
        i.b = r;
        i.c = r + 1;
        i.g = op as u16;
        i.e = self.k(e);
        i.f = self.k(prim);
        i.fl |= F_DEPTH | F_CLR_B | if a.len() == 2 { F_CLR_C } else { 0 };
        i.d = d;
        if tail.vis() {
            i.fl |= F_VIS;
        }
        self.emit(i);
        self.free_to(r);
        Ok(())
    }

    /// An operand of an inlined kernel: a plain numeric scalar constant is
    /// loaded unboxed (its identity is not observable there).
    unsafe fn operand(&mut self, v: Sexp, r: Reg, d: u16) -> Result<(), Need> {
        // a fold first (`cmp`'s), then the constant's own
        // SAFETY: the caller's contract.
        let c = if self.bc.is_some() { unsafe { self.constant_fold(v)? } } else { None };
        // SAFETY: as above.
        let lit = unsafe { plain_scalar_const(v) }.map(|_| v);
        let c = c.or(lit).filter(|&c| {
            // SAFETY: a live constant of the source or a fold.
            unsafe { plain_scalar_const(c) }.is_some()
        });
        if let Some(c) = c {
            // the pool's entry, which may be an earlier identical constant
            // (`kv`): loaded unboxed only if it is a plain scalar as well
            // (an EXTDATA one is loaded as an object; `inline::scalar`
            // reads `LdScalar`'s operand inline)
            let k = self.kv(c);
            // SAFETY: a live constant of the pool.
            if unsafe { plain_scalar_const(self.consts[usize::from(k) - FIRST_CONST]) }.is_none() {
                // SAFETY: as above.
                unsafe { self.const_value(c, r, d, Tail::No) };
                return Ok(());
            }
            // (an unboxed constant is no object: nothing is marked, as
            // neither `eval` nor LDCONST marks one)
            let mut i = Ins::new(Op::LdScalar);
            i.a = r;
            i.e = k;
            let i = self.at_depth(i, d, true);
            self.emit(i);
            return Ok(());
        }
        // SAFETY: as above.
        unsafe { self.expr(v, r, d, Tail::No) }
    }

    /// A builtin (or a builtin `.Internal`, `op`) called with its arguments
    /// evaluated into registers, in order, as `evalList` does; with `...`
    /// among them the evaluation happens at run time (`CallBuiltin` with
    /// no window: [`Op::Call`]'s builtin path).
    #[allow(clippy::too_many_arguments)]
    unsafe fn builtin_args_call(
        &mut self,
        op: Op,
        fun: Sexp,
        call: Sexp,
        args: Sexp,
        dst: Reg,
        d: u16,
        tail: Tail,
    ) -> Result<(), Need> {
        let nil = self.nil();
        // SAFETY: the caller's contract.
        let a = unsafe { self.arglist(args) };
        let dots = self.st.syms.dots;
        let missing = robigo_rtcore_missing(self.cx);
        let n = a.len();
        let mut i = Ins::new(op);
        i.a = dst;
        i.e = self.k(call);
        i.f = self.k(fun);
        i.fl |= F_DEPTH;
        i.d = d;
        let _ = tail;
        // an .Internal of a count its checkArity refuses: the arguments
        // evaluated, then the error (the VM's register call does not check)
        let refused = (op == Op::Internal)
            .then(|| {
                // SAFETY: `fun` is a live primitive of this isolate (the
                // caller's contract).
                let def = &self.st.defs[unsafe { crate::prims::offset(fun) }];
                (!crate::call::internal_arity_ok(def, n))
                    .then(|| crate::prims::arity_message(n, def.name, def.arity as usize, true))
            })
            .flatten();
        if a.iter().any(|&(v, _)| v == dots || v == missing) {
            // evaluated at run time (`...` expanded, an empty argument's
            // error in order); under `Bc` as the generic call's (cmp.R's
            // `cmpBuiltin` declines): each call by code compiled now
            // (`cmpCallArgs`)
            if self.bc.is_some() {
                // SAFETY: the caller's contract.
                unsafe { self.precompile_args(args)? };
            }
            i.c = u16::MAX;
            self.emit(i);
            return Ok(());
        }
        let named = a.iter().any(|&(_, t)| t != nil);
        let r = self.alloc_n(if named { 2 * n } else { n });
        // evalList under eval's PROTECT(op) (and, for `.Internal`'s inner
        // call, do_internal's under the special's PROTECT(e)), its head's
        // after the first value
        let base = self.pp + if op == Op::Internal { 2 } else { 1 };
        for (j, &(v, _)) in a.iter().enumerate() {
            // SAFETY: as above.
            unsafe { self.expr_at(v, r + j as Reg, d + 1, base + (j > 0) as i32, Tail::No)? };
        }
        if let Some(msg) = refused {
            // against the context, as for every .Internal
            // R: util.c:Rf_checkArityCall
            self.error_op(call, &msg, false)?;
            self.free_to(r);
            return Ok(());
        }
        if named {
            for (j, &(_, t)) in a.iter().enumerate() {
                let mut l = Ins::new(Op::LdConst);
                l.a = r + (n + j) as Reg;
                l.e = self.k(t);
                self.emit(l);
            }
            i.fl |= F_NAMES;
        }
        i.b = r;
        i.c = n as u16;
        self.emit(i);
        self.free_to(r);
        Ok(())
    }
}

/// `R_MissingArg` of `cx`'s isolate.
fn robigo_rtcore_missing(cx: &Cx) -> Sexp {
    cx.missing_arg().sexp()
}

/// GNU R's `MARK_NOT_MUTABLE`: the count saturated.
///
/// # Safety
/// `x` is live.
unsafe fn mark_not_mutable(x: Sexp) {
    // SAFETY: the caller's contract; REFCNT is a header field.
    unsafe {
        if !x.has_flag(robigo_heap::flags::IMMORTAL) {
            x.set_refcnt(u16::MAX);
        }
    }
}

/// A plain length-1 double, integer or logical with no attributes.
///
/// # Safety
/// `x` is live.
unsafe fn plain_scalar_const(x: Sexp) -> Option<()> {
    // (its payload inline: C-owned EXTDATA vectors are loaded as constants,
    // so `inline::scalar` reads `LdScalar`'s operand without the test)
    // SAFETY: the caller's contract.
    unsafe { crate::code::plain_scalar(x) }.then_some(())
}

/// `b` through [`crate::code::Verified`]'s gate, the one every code object
/// passes (#738): a check that fails is a compiler bug.
fn verified(b: Built) -> crate::code::Verified {
    crate::code::Verified::new(b).unwrap_or_else(|e| panic!("{e}"))
}

/// `checkArity`'s message for a special (`crate::prims::arity_message`).
fn arity_msg(n: usize, name: &str, arity: usize) -> String {
    crate::prims::arity_message(n, name, arity, false)
}

/// `check1arg(args, call, formal)`: a tagged argument must partially match
/// `formal`.
// R: match.c:check1arg
unsafe fn check1arg(tag: Sexp, nil: Sexp, formal: &str) -> Option<String> {
    if tag == nil {
        return None;
    }
    // SAFETY: the caller's contract: a tag is a symbol.
    let supplied = unsafe { tag.sym_name_bytes() };
    if formal.as_bytes().starts_with(supplied) {
        return None;
    }
    let s = String::from_utf8_lossy(supplied);
    Some(format!("supplied argument name '{s}' does not match '{formal}'"))
}

/// `CheckFormals`: every tag a symbol.
// R: eval.c:CheckFormals
unsafe fn check_formals(mut ls: Sexp, nil: Sexp) -> bool {
    // SAFETY: the caller's contract: a live object.
    unsafe {
        if !matches!(ls.typ(), NILSXP | LISTSXP) {
            return false;
        }
        while ls != nil {
            if ls.tag().typ() != SYMSXP {
                return false;
            }
            ls = ls.cdr();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::fold_vis_into_ret;
    use crate::ir::{Ins, Op};

    /// The pool is one word of the isolate's state (allocated on first use).
    #[test]
    fn the_spare_buffers_are_one_word_of_state() {
        assert_eq!(size_of::<super::Scratch>(), size_of::<usize>());
    }

    /// A compile's buffers go back to the isolate's spares cleared (no
    /// entry survives into the next compile, or across a collection), a
    /// buffer past `SCRATCH_CAP` is freed rather than kept, and at most
    /// `SCRATCH_SETS` sets wait.
    #[test]
    fn spare_buffers_come_back_cleared_and_capped() {
        use super::{Bufs, SCRATCH_CAP, SCRATCH_SETS, Scratch};
        let s = Scratch::default();
        let mut b = s.take();
        b.ins.resize(10, Ins::new(Op::Ret));
        b.ppk.resize(SCRATCH_CAP + 1, 0);
        b.pool.insert(1, 2);
        let kept = b.ins.capacity();
        s.give(b);
        // (take's debug assertion: the set is empty)
        let b = s.take();
        assert!(b.ins.is_empty() && b.ins.capacity() >= kept, "a small buffer is kept, empty");
        assert!(b.pool.is_empty());
        assert_eq!(b.ppk.capacity(), 0, "a buffer past the cap is not kept");
        for _ in 0..SCRATCH_SETS + 2 {
            s.give(Bufs::default());
        }
        assert_eq!(s.sets().borrow().len(), SCRATCH_SETS);
    }

    /// A jump whose target is the code's end, and a loop's region ending
    /// there (whose "instruction after" is one past it), mark positions
    /// past the last instruction: they are kept off the table or in its
    /// two spare entries, never out of bounds, and the fold of a `SetVis`
    /// into the `Ret` after it goes on as with the hashed set (whose
    /// lookups never asked about those positions).
    #[test]
    fn targets_at_and_past_the_end_leave_the_fold_as_before() {
        let jmp = |op: Op, t: usize| {
            let mut i = Ins::new(op);
            i.set_target(t);
            i
        };
        let mut vis = Ins::new(Op::SetVis);
        vis.g = 1;
        let mut ins = vec![vis, Ins::new(Op::Ret), jmp(Op::Jmp, 3), jmp(Op::Loop, 3), jmp(Op::Jmp, 9)];
        let n = ins.len();
        // targets 3 (an instruction), 4 (one past it), 5 (the end), 9 (past)
        ins[3].set_target(n);
        fold_vis_into_ret(&mut ins, false);
        assert_eq!(ins[0].op, Op::Ret, "the SetVis folded into the Ret");
        assert_eq!(ins[0].g, 1);
        // a target on the Ret itself keeps the SetVis
        let mut ins = vec![vis, Ins::new(Op::Ret), jmp(Op::Jmp, 1)];
        fold_vis_into_ret(&mut ins, false);
        assert_eq!(ins[0].op, Op::SetVis);
    }
}
