//! `args::bind`: a closure call's arguments bound into its frame
//! (docs/g3.md §6.2-6.4). The one module that decides how arguments are
//! represented, so that X2's D4 control (a PROMSXP per argument, GNU R's
//! `promiseArgs`) swaps it on a branch.
//!
//! # What a call binds
//!
//! GNU R makes a promise per supplied argument (`promiseArgs`), matches
//! them to the formals (`matchArgs`' three passes, match.c:185), binds
//! the matched promises and a default promise per missing formal with a
//! default (`make_applyClosure_env`, eval.c:2240), and clears the
//! promises at the end (`R_CleanupEnvir`, `unpromiseArgs`). Here a
//! supplied argument is a *delayed slot* of the callee's frame (its
//! expression, the caller's environment), not an object; it becomes the
//! PROMSXP GNU R would have only when something observes it as one
//! (rtcore's `env::promise`, §6.3), and its slot's kind is the kind GNU R's
//! binding has at every moment (§6.2):
//!
//! | actual | `Eval` profile | `Bc` profile |
//! |---|---|---|
//! | an expression | delayed in the caller | delayed; a constant is a value (`PUSHCONSTARG`, marked not mutable) |
//! | empty (`f(x = )`) | the formal's default (delayed in the frame, MISSING 2) or R_MissingArg (MISSING 1) | same |
//! | a `...` element `v` | delayed, expression `v` in the caller (GNU R's double promise `mkPROMISE(v, rho)`, no object made) | same |
//! | an element of a promise list (C, `lapply`) | that object, as GNU R binds it | same |
//!
//! The `...` formal gets a DOTSXP of the unmatched actuals: promises made
//! for them as `promiseArgs` makes them (each element its own promise,
//! shared by every alias of it). It is built from the call site directly,
//! with no intermediate argument list to copy.
//!
//! # Match plans
//!
//! A call site whose arguments contain no `...` has static tags, so its
//! match against one callee's formals is always the same: the result is a
//! plan (which actual each formal gets, which go to `...`, which partial
//! matches it made) memoized in the caller's code object per site, keyed by
//! the callee code object's stamp (a counter that never repeats, never an
//! address). A plan replays the partial-match warnings `matchArgs` signals
//! under `options(warnPartialMatchArgs = TRUE)` (match.c:277-279) in the
//! same order, for the option in force at the call. Errors are never
//! memoized: a failing match runs the matcher every time.
//!
//! # `promargs`
//!
//! The function record's `promargs` is GNU R's argument list. It is made
//! only when read ([`fill_promargs`]: `match.call`, `UseMethod`, C that
//! reads the field), from the frame: each argument slot's promise, made
//! once (rtcore's `Cx::arg_promise`).
//!
//! # `ROBIGO_MATERIALIZE=1`
//!
//! Every argument slot is made its PROMSXP at call entry: GNU R's
//! representation, through the same §6.3 transitions, so a run under it
//! compared with a normal run finds a slot kind that misses an observer
//! (docs/g3.md §16; T2 completes the mode).

#![allow(unsafe_code)]

use crate::call::closure_code;
use crate::ir::{NO_SITE, Profile, R_ENV, Reg};
use crate::prims;
use crate::vm::{self, Act};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Safepoint, Sexp};
use robigo_rtcore::{
    ContextId, Cx, Out, RResult, RawBinding, RawFunctionContext, Ref, Root, Scope, Slot, SlotInit, ctxt,
};
use std::ptr::NonNull;

// ---- small stack vectors -------------------------------------------------------

/// A small vector on the stack (the common case: a call's arguments and a
/// closure's formals), spilling to the heap past [`INLINE`] elements: the
/// argument matcher runs on every call and must not allocate. The inline
/// storage is not initialized ahead of use (a call pays for the elements it
/// writes, not for the capacity).
const INLINE: usize = 8;

pub(crate) struct Buf<T: Copy> {
    inline: [std::mem::MaybeUninit<T>; INLINE],
    len: usize,
    heap: Vec<T>,
}

impl<T: Copy> Buf<T> {
    pub(crate) fn new() -> Buf<T> {
        Buf { inline: [const { std::mem::MaybeUninit::uninit() }; INLINE], len: 0, heap: Vec::new() }
    }

    pub(crate) fn filled(v: T, n: usize) -> Buf<T> {
        let mut b = Buf::new();
        if n > INLINE {
            b.heap = vec![v; n];
        } else {
            for x in &mut b.inline[..n] {
                x.write(v);
            }
        }
        b.len = n;
        b
    }

    pub(crate) fn push(&mut self, v: T) {
        if self.len < INLINE && self.heap.is_empty() {
            self.inline[self.len].write(v);
        } else {
            if self.heap.is_empty() {
                let init = self.as_slice().to_vec();
                self.heap = init;
            }
            self.heap.push(v);
        }
        self.len += 1;
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.heap.clear();
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        if self.heap.is_empty() {
            // SAFETY: the first `len` inline elements are written (`push`
            // and `filled` write before they count), and `len <= INLINE`
            // while the heap is empty.
            unsafe { std::slice::from_raw_parts(self.inline.as_ptr() as *const T, self.len) }
        } else {
            &self.heap
        }
    }

    fn as_mut_slice(&mut self) -> &mut [T] {
        if self.heap.is_empty() {
            // SAFETY: as in `as_slice`.
            unsafe { std::slice::from_raw_parts_mut(self.inline.as_mut_ptr() as *mut T, self.len) }
        } else {
            &mut self.heap
        }
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
}

impl<T: Copy> std::ops::Index<usize> for Buf<T> {
    type Output = T;
    fn index(&self, i: usize) -> &T {
        &self.as_slice()[i]
    }
}

impl<T: Copy> std::ops::IndexMut<usize> for Buf<T> {
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.as_mut_slice()[i]
    }
}

// ---- actuals --------------------------------------------------------------------

/// Where a supplied argument came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Origin {
    /// An argument expression of the call (R_MissingArg when empty).
    Expr,
    /// An element of the caller's `...` (a promise, R_MissingArg, or
    /// whatever C put there).
    Dot,
    /// An element of a promise list handed to `applyClosure`.
    Obj,
}

/// One supplied argument (after `...` is expanded), as `matchArgs` sees
/// the element of its `supplied` list.
#[derive(Clone, Copy, Debug)]
struct Actual {
    /// The expression, `...` element or list element.
    val: Sexp,
    /// The tag (a symbol, immortal) or R_NilValue.
    tag: Sexp,
    from: Origin,
    /// The value is R_MissingArg.
    missing: bool,
    /// Its position: among the call's arguments (`Expr`), in the caller's
    /// `...` (`Dot`), in the list (`Obj`).
    pos: u16,
}

/// Collecting actuals found `...` bound to something that is not a `...`
/// list: "'...' used in an incorrect context".
struct BadDots;

/// The actuals of `call` in `rho`: its arguments in order, each `...`
/// expanded into the elements of the caller's `...` (`promiseArgs`' walk,
/// eval.c:3781). True when a `...` was expanded (the tags then vary per
/// call).
///
/// # Safety
/// `call` a live call and `rho` a live environment of `cx`'s isolate,
/// reachable from roots; nothing collects while the result is used.
unsafe fn call_actuals(cx: &Cx, call: Sexp, rho: Sexp, acts: &mut Buf<Actual>) -> Result<bool, BadDots> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    acts.clear();
    let mut any_dots = false;
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        let mut el = call.cdr();
        let mut pos = 0u16;
        while el != nil {
            let a = el.car();
            if a == dots {
                any_dots = true;
                // R_findVar(R_DotsSymbol, rho): a DOTSXP or NULL is
                // spliced in, R_MissingArg is nothing, anything else is an
                // error
                match cx.var_lookup_raw(dots, rho) {
                    RawBinding::Value(h) if h.typ() == DOTSXP || h == nil => {
                        let mut h = h;
                        let mut i = 0u16;
                        while h != nil {
                            let v = h.car();
                            acts.push(Actual {
                                val: v,
                                tag: h.tag(),
                                from: Origin::Dot,
                                missing: v == missing,
                                pos: i,
                            });
                            h = h.cdr();
                            i = i.saturating_add(1);
                        }
                    }
                    RawBinding::Value(h) if h == missing => {}
                    _ => return Err(BadDots),
                }
            } else {
                acts.push(Actual { val: a, tag: el.tag(), from: Origin::Expr, missing: a == missing, pos });
            }
            el = el.cdr();
            pos = pos.saturating_add(1);
        }
    }
    Ok(any_dots)
}

/// The actuals of a promise list (`applyClosure`'s `arglist`).
///
/// # Safety
/// As for [`call_actuals`].
unsafe fn list_actuals(cx: &Cx, list: Sexp, acts: &mut Buf<Actual>) {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    acts.clear();
    // SAFETY: the caller's contract.
    unsafe {
        let mut el = list;
        let mut pos = 0u16;
        while el != nil {
            let v = el.car();
            acts.push(Actual { val: v, tag: el.tag(), from: Origin::Obj, missing: v == missing, pos });
            el = el.cdr();
            pos = pos.saturating_add(1);
        }
    }
}

// ---- matching -------------------------------------------------------------------

/// No actual (a formal unmatched), no `...` formal.
const NO: u16 = u16::MAX;

/// What `matchArgs` decided.
pub(crate) struct Matched {
    /// Per formal: the index of the actual bound to it, or [`NO`].
    src: Buf<u16>,
    /// The `...` formal (the first), or [`NO`].
    dots_formal: u16,
    /// The actuals the `...` formal gobbles, in order.
    dots: Buf<u16>,
    /// The partial matches made, (actual, formal), in the order
    /// `matchArgs` signals their warnings.
    partial: Buf<(u16, u16)>,
}

impl Matched {
    fn new(nformals: usize) -> Matched {
        Matched { src: Buf::filled(NO, nformals), dots_formal: NO, dots: Buf::new(), partial: Buf::new() }
    }
}

/// `matchArgs`' errors (reported against the call).
#[derive(Clone, Copy, Debug)]
enum MatchError {
    /// "formal argument \"%s\" matched by multiple actual arguments"
    FormalTwice(Sexp),
    /// "argument %d matches multiple formal arguments"
    ArgTwice(usize),
    /// "unused argument(s) %s": how many
    Unused,
}

/// `pmatch(formal, tag, exact)`: `exact` or a prefix match.
///
/// # Safety
/// Both are live symbols.
// R: match.c:pmatch, psmatch
unsafe fn pmatch(formal: Sexp, tag: Sexp, exact: bool) -> bool {
    // SAFETY: the caller's contract.
    let (f, t) = unsafe { (formal.car().char_bytes(), tag.car().char_bytes()) };
    if exact { f == t } else { f.starts_with(t) }
}

/// `matchArgs_NR`'s three passes over the formals `fs` (their tags) and
/// the actuals. The partial matches are recorded in order whether or not
/// the option warns; an error comes with the partial matches made before
/// it (their warnings precede it).
///
/// # Safety
/// The formals' tags and the actuals' tags are live symbols (or R_NilValue).
// R: match.c:matchArgs_NR
unsafe fn match_args(fs: &[Sexp], acts: &[Actual], nil: Sexp, dots: Sexp, m: &mut Matched) -> Result<(), MatchError> {
    let nf = fs.len();
    // the value of each formal so far is missing (R_MissingArg)
    let mut fmissing: Buf<bool> = Buf::filled(true, nf);
    let mut fargused: Buf<u8> = Buf::filled(0, nf);
    let mut argused: Buf<u8> = Buf::filled(0, acts.len());
    // SAFETY: the caller's contract.
    unsafe {
        // First pass: exact matches by tag
        for (fi, &ftag) in fs.iter().enumerate() {
            if ftag == dots || ftag == nil {
                continue;
            }
            for (bi, b) in acts.iter().enumerate() {
                if b.tag != nil && pmatch(ftag, b.tag, true) {
                    if fargused[fi] == 2 {
                        return Err(MatchError::FormalTwice(ftag));
                    }
                    if argused[bi] == 2 {
                        return Err(MatchError::ArgTwice(bi + 1));
                    }
                    m.src[fi] = bi as u16;
                    fmissing[fi] = b.missing;
                    argused[bi] = 2;
                    fargused[fi] = 2;
                }
            }
        }
        // Second pass: partial matches based on tags; an exact match is
        // required after the first `...`
        let mut seendots = false;
        for (fi, &ftag) in fs.iter().enumerate() {
            if fargused[fi] != 0 {
                continue;
            }
            if ftag == dots && !seendots {
                m.dots_formal = fi as u16;
                seendots = true;
                continue;
            }
            for (bi, b) in acts.iter().enumerate() {
                if argused[bi] != 2 && b.tag != nil && pmatch(ftag, b.tag, seendots) {
                    if argused[bi] != 0 {
                        return Err(MatchError::ArgTwice(bi + 1));
                    }
                    if fargused[fi] == 1 {
                        return Err(MatchError::FormalTwice(ftag));
                    }
                    m.partial.push((bi as u16, fi as u16));
                    m.src[fi] = bi as u16;
                    fmissing[fi] = b.missing;
                    argused[bi] = 1;
                    fargused[fi] = 1;
                }
            }
        }
        // Third pass: matches based on order; at `...` the remaining
        // actuals are left to it
        let (mut fi, mut bi) = (0, 0);
        while fi < nf && bi < acts.len() {
            if fs[fi] == dots {
                break;
            } else if !fmissing[fi] {
                // already matched by tag
                fi += 1;
            } else if argused[bi] != 0 || acts[bi].tag != nil {
                // this value used or tagged: skip to the next value
                bi += 1;
            } else {
                m.src[fi] = bi as u16;
                fmissing[fi] = acts[bi].missing;
                argused[bi] = 1;
                bi += 1;
                fi += 1;
            }
        }
    }
    if m.dots_formal != NO {
        // gobble up all unused actuals
        for (bi, &u) in argused.iter().enumerate() {
            if u == 0 {
                m.dots.push(bi as u16);
            }
        }
    } else if argused.iter().any(|&u| u == 0) {
        return Err(MatchError::Unused);
    }
    Ok(())
}

// ---- plans ----------------------------------------------------------------------

/// The bytes of one call site's plan in a code object's plan cache.
pub const PLAN_BYTES: usize = 64;
const PLAN_FORMALS: usize = 16;
const PLAN_DOTS: usize = 16;
const PLAN_PARTIAL: usize = 4;
const PLAN_NO: u8 = u8::MAX;

/// A memoized match: what [`Matched`] holds, for a site with at most 16
/// formals, 16 actuals and 4 partial matches.
#[repr(C)]
#[derive(Clone, Copy)]
struct Plan {
    /// The callee code object's stamp (0: empty).
    stamp: u64,
    nformals: u8,
    nactuals: u8,
    ndots: u8,
    npartial: u8,
    dots_formal: u8,
    _pad: [u8; 3],
    src: [u8; PLAN_FORMALS],
    dots: [u8; PLAN_DOTS],
    partial: [[u8; 2]; PLAN_PARTIAL],
    _pad2: [u8; 8],
}

const _: () = assert!(size_of::<Plan>() == PLAN_BYTES && align_of::<Plan>() <= 16);

impl Plan {
    /// The plan of `m`, if it fits.
    fn of(stamp: u64, m: &Matched, nactuals: usize) -> Option<Plan> {
        if m.src.len() > PLAN_FORMALS
            || nactuals > PLAN_DOTS
            || m.partial.len() > PLAN_PARTIAL
            || nactuals >= PLAN_NO as usize
        {
            return None;
        }
        let mut p = Plan {
            stamp,
            nformals: m.src.len() as u8,
            nactuals: nactuals as u8,
            ndots: m.dots.len() as u8,
            npartial: m.partial.len() as u8,
            dots_formal: if m.dots_formal == NO { PLAN_NO } else { m.dots_formal as u8 },
            _pad: [0; 3],
            src: [PLAN_NO; PLAN_FORMALS],
            dots: [0; PLAN_DOTS],
            partial: [[0; 2]; PLAN_PARTIAL],
            _pad2: [0; 8],
        };
        for (i, &s) in m.src.iter().enumerate() {
            p.src[i] = if s == NO { PLAN_NO } else { s as u8 };
        }
        for (i, &d) in m.dots.iter().enumerate() {
            p.dots[i] = d as u8;
        }
        for (i, &(a, f)) in m.partial.iter().enumerate() {
            p.partial[i] = [a as u8, f as u8];
        }
        Some(p)
    }

    /// The match this plan memoizes.
    /// Fill `m` (made for `nformals` formals) with the match this plan
    /// memoizes.
    fn fill(&self, m: &mut Matched) {
        for i in 0..self.nformals as usize {
            let s = self.src[i];
            m.src[i] = if s == PLAN_NO { NO } else { s as u16 };
        }
        m.dots_formal = if self.dots_formal == PLAN_NO { NO } else { self.dots_formal as u16 };
        for &d in &self.dots[..self.ndots as usize] {
            m.dots.push(d as u16);
        }
        for &[a, f] in &self.partial[..self.npartial as usize] {
            m.partial.push((a as u16, f as u16));
        }
    }
}

/// `matchArgs_NR(formals, CDR(call), call)` for a special's own formals
/// (`on.exit`, `substitute`: no `...` among them): the call's arguments as
/// written (a `...` among them is an argument, not expanded), each formal's
/// expression or `R_MissingArg`, rooted in `s`; GNU R's errors against the
/// call, its partial-match warnings under the option.
// R: match.c:matchArgs_NR
pub(crate) fn match_builtin_args<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    call: Root<'_>,
    formals: &[Sexp],
) -> RResult<Vec<Root<'s>>> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    let mut acts: Buf<Actual> = Buf::new();
    // SAFETY: the rooted call's cells, read in this interval; nothing
    // collects.
    unsafe {
        let mut el = call.bind(cx).sexp().cdr();
        let mut pos = 0u16;
        while el != nil {
            let a = el.car();
            acts.push(Actual { val: a, tag: el.tag(), from: Origin::Expr, missing: a == missing, pos });
            el = el.cdr();
            pos = pos.saturating_add(1);
        }
    }
    let mut m = Matched::new(formals.len());
    // SAFETY: the formals are symbols and the actuals' tags symbols or nil.
    if let Err(e) = unsafe { match_args(formals, acts.as_slice(), nil, dots, &mut m) } {
        let msg = match_message(cx, e, acts.as_slice());
        return Err(cx.error_call(Some(call), &msg));
    }
    // the values rooted before a warning can run code (and collect)
    let vals: Vec<Root<'s>> = (0..formals.len())
        .map(|i| {
            let v = if m.src[i] == NO { missing } else { acts[m.src[i] as usize].val };
            // SAFETY: parts of the rooted call, or R_MissingArg, read in this
            // interval.
            s.root(cx, unsafe { Ref::from_sexp(v, cx) })
        })
        .collect();
    if !m.partial.is_empty() && cx.warn_partial_match_args() {
        // (the actuals' tags are symbols: immortal)
        partial_warnings(cx, call, m.partial.as_slice(), acts.as_slice(), formals)?;
    }
    Ok(vals)
}

// ---- binding ----------------------------------------------------------------------

/// Where the arguments come from.
#[derive(Clone, Copy)]
pub(crate) enum Source {
    /// The call's own arguments, in the caller's environment, from call
    /// site `site` of the caller's code (or [`NO_SITE`]).
    Site(u16),
    /// A promise list (`applyClosure(call, op, arglist, rho, ...)`).
    List,
}

/// What the lazy `promargs` filler needs of a call: its actuals (tags,
/// kinds, positions; values are read again from the frame and the call)
/// and the match. Lives on [`apply`]'s stack while the call's record does.
struct FillData {
    acts: *const Buf<Actual>,
    matched: *const Matched,
}

/// `match.call(def, call)` at compile time (cmp.R's `checkCall` and
/// `inlineSimpleInternalCall`) for a call without `...`: per formal of
/// `formals`, the argument expression matched to it (None: not supplied),
/// or None when `matchArgs` would fail (an unused argument, a formal or an
/// argument matched twice).
///
/// # Safety
/// `formals` is a live pairlist and `call` a live call without `...`.
pub(crate) unsafe fn match_static(cx: &Cx, formals: Sexp, call: Sexp) -> Option<Vec<Option<Sexp>>> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    let mut acts: Buf<Actual> = Buf::new();
    let mut ftags: Buf<Sexp> = Buf::new();
    // SAFETY: the caller's contract.
    unsafe {
        let mut el = call.cdr();
        let mut pos = 0u16;
        while el != nil {
            let a = el.car();
            acts.push(Actual { val: a, tag: el.tag(), from: Origin::Expr, missing: a == missing, pos });
            pos = pos.saturating_add(1);
            el = el.cdr();
        }
        let mut f = formals;
        while f != nil {
            ftags.push(f.tag());
            f = f.cdr();
        }
        let mut m = Matched::new(ftags.len());
        match_args(ftags.as_slice(), acts.as_slice(), nil, dots, &mut m).ok()?;
        Some(m.src.as_slice().iter().map(|&i| (i != NO).then(|| acts[i as usize].val)).collect())
    }
}

/// A constant argument under the `Bc` profile: not a symbol, call,
/// promise, byte code or `...` (cmp.R's constant arguments).
///
/// # Safety
/// `a` is live.
unsafe fn is_constant(a: Sexp) -> bool {
    // SAFETY: the caller's contract.
    !matches!(unsafe { a.typ() }, SYMSXP | LANGSXP | PROMSXP | BCODESXP | DOTSXP)
}

/// `mkPROMISE(expr, rho)`: PRVALUE R_UnboundValue, PRSEEN 0; PRCODE and
/// PRENV counted (memory.c:2607; `ENSURE_NAMEDMAX(expr)` does nothing
/// under reference counting).
///
/// # Safety
/// `expr` and `rho` are live objects of `cx`'s isolate, reachable from roots.
// R: memory.c:mkPROMISE
pub unsafe fn mk_promise(cx: &Cx, expr: Sexp, rho: Sexp) -> Result<Sexp, NeedGc> {
    // SAFETY: the caller's contract.
    unsafe { cx.alloc_node_raw(PROMSXP, cx.unbound().sexp(), expr, rho) }
}

/// GNU R's `MARK_NOT_MUTABLE`.
///
/// # Safety
/// `x` is live.
unsafe fn mark_not_mutable(x: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if !x.has_flag(robigo_heap::flags::IMMORTAL) {
            x.set_refcnt(u16::MAX);
        }
    }
}

/// The element GNU R's argument list (and so a `...` list) holds for
/// actual `a` of a call from `rho`: `promiseArgs`' promise, R_MissingArg,
/// a `Bc` constant (marked not mutable, `PUSHCONSTARG`), or a list's own
/// element.
///
/// # Safety
/// `a`'s value and `rho` are live and reachable from roots.
unsafe fn list_element(cx: &Cx, a: &Actual, rho: Sexp, profile: Profile, bytecode: bool) -> Result<Sexp, NeedGc> {
    let missing = cx.missing_arg().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        if a.missing {
            return Ok(missing);
        }
        match a.from {
            Origin::Obj => Ok(a.val),
            Origin::Expr if profile == Profile::Bc && is_constant(a.val) => {
                mark_not_mutable(a.val);
                Ok(a.val)
            }
            Origin::Expr => {
                let p = mk_promise(cx, a.val, rho)?;
                if bytecode {
                    // compiled code's promise holds byte code (MAKEPROM)
                    p.set_extra(p.extra() | robigo_heap::extra::BYTECODE);
                }
                Ok(p)
            }
            Origin::Dot => mk_promise(cx, a.val, rho),
        }
    }
}

/// D4's control: GNU R's argument list for the actuals `acts` of a call
/// from `rho`, one tagged cons per actual holding its [`list_element`].
///
/// # Safety
/// As for [`list_element`].
// R: eval.c:promiseArgs
unsafe fn promise_list(cx: &Cx, acts: &[Actual], rho: Sexp, profile: Profile, bytecode: bool) -> Result<Sexp, NeedGc> {
    let mut l = cx.nil().sexp();
    // SAFETY: the caller's contract; nothing collects in this interval, so
    // the list built so far stays valid.
    unsafe {
        for a in acts.iter().rev() {
            let v = list_element(cx, a, rho, profile, bytecode)?;
            l = cx.alloc_node_raw(LISTSXP, v, l, a.tag)?;
        }
    }
    Ok(l)
}

/// The DOTSXP the `...` formal gets: the gobbled actuals' list elements,
/// with their tags (`matchArgs`' `allocList` + `SET_TYPEOF(a, DOTSXP)`).
///
/// # Safety
/// As for [`list_element`].
unsafe fn dots_list(
    cx: &Cx,
    acts: &[Actual],
    m: &Matched,
    rho: Sexp,
    profile: Profile,
    bytecode: bool,
) -> Result<Sexp, NeedGc> {
    let mut d = cx.nil().sexp();
    // SAFETY: the caller's contract; nothing collects in this interval, so
    // the list built so far stays valid.
    unsafe {
        for &bi in m.dots.as_slice().iter().rev() {
            let a = &acts[bi as usize];
            let v = list_element(cx, a, rho, profile, bytecode)?;
            d = cx.alloc_node_raw(DOTSXP, v, d, a.tag)?;
        }
    }
    Ok(d)
}

/// The formals of closure `op` as a vector of their tags and defaults.
///
/// # Safety
/// `op` is a live closure.
unsafe fn formals(cx: &Cx, op: Sexp, tags: &mut Buf<Sexp>, defaults: &mut Buf<Sexp>) {
    let nil = cx.nil().sexp();
    tags.clear();
    defaults.clear();
    // SAFETY: the caller's contract.
    unsafe {
        let mut f = op.car();
        while f != nil {
            tags.push(f.tag());
            defaults.push(f.car());
            f = f.cdr();
        }
    }
}

/// The match error's message.
fn match_message(cx: &Cx, e: MatchError, acts: &[Actual]) -> String {
    match e {
        MatchError::FormalTwice(s) => {
            // SAFETY: a formal's tag, a live symbol.
            let name = unsafe { Ref::from_sexp(s, cx) }.as_sym().unwrap().name();
            format!(
                "formal argument \"{}\" matched by multiple actual arguments",
                String::from_utf8_lossy(name.bytes())
            )
        }
        MatchError::ArgTwice(i) => format!("argument {i} matches multiple formal arguments"),
        // made by unused_message, which deparses the arguments
        MatchError::Unused => {
            let _ = acts;
            unreachable!("robigo: the unused-arguments message is unused_message's")
        }
    }
}

/// The unused actuals of a failed match, rooted in `s`: (tag, the
/// argument as `matchArgs` shows it: a promise's expression).
fn unused_actuals<'s>(
    cx: &Cx,
    s: robigo_rtcore::Scope<'s>,
    acts: &[Actual],
    m: &Matched,
) -> Vec<(Option<Root<'s>>, Root<'s>)> {
    let nil = cx.nil().sexp();
    let mut used = vec![false; acts.len()];
    for &a in m.src.iter() {
        if a != NO {
            used[a as usize] = true;
        }
    }
    acts.iter()
        .zip(used)
        .filter(|(_, u)| !u)
        .map(|(a, _)| {
            // SAFETY: the actual's value and tag, reachable from the rooted
            // call, list or `...` and read with nothing able to collect
            // since the match; PREXPR of a promise is its code.
            unsafe {
                let v = if a.val.typ() == PROMSXP { a.val.cdr() } else { a.val };
                let tag = (a.tag != nil).then(|| s.root(cx, Ref::from_sexp(a.tag, cx)));
                (tag, s.root(cx, Ref::from_sexp(v, cx)))
            }
        })
        .collect()
}

/// `matchArgs`' "unused argument %s" / "unused arguments %s": the unused
/// actuals as a tagged pairlist, deparsed on one line, from its `(`.
// R: match.c:matchArgs_NR
pub(crate) fn unused_message(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'_>,
    unused: &[(Option<Root<'_>>, Root<'_>)],
) -> RResult<String> {
    let o = s.out(cx);
    let l = o.root();
    cx.build(o, |cx| {
        let mut l = cx.nil();
        for (t, v) in unused.iter().rev() {
            l = match t.and_then(|t| t.bind(cx).as_sym()) {
                Some(t) => cx.cons_tagged(v.bind(cx), l, t)?,
                None => cx.cons(v.bind(cx), l)?,
            };
        }
        Ok(l)
    })?;
    let what = if unused.len() == 1 { "unused argument" } else { "unused arguments" };
    let text = match crate::deparse::deparse1line(cx, l.bind(cx), false, 0) {
        Ok(b) => b,
        Err(m) => return Err(cx.error(&m)),
    };
    let from = text.iter().position(|&b| b == b'(').unwrap_or(0);
    Ok(format!("{what} {}", String::from_utf8_lossy(&text[from..])))
}

/// Signal `matchArgs`' partial-match warnings for `partial` (the option is
/// on): "partial argument match of '%s' to '%s'" against the call, in the
/// order `matchArgs` signals them.
// R: match.c:277-279, errors.c:R_makePartialMatchWarningCondition
fn partial_warnings(
    cx: &mut Cx,
    call: Root<'_>,
    partial: &[(u16, u16)],
    acts: &[Actual],
    ftags: &[Sexp],
) -> RResult<()> {
    for &(a, f) in partial {
        // (tags are symbols: immortal)
        let tag = crate::eval::sym_name(cx, acts[a as usize].tag);
        let formal = crate::eval::sym_name(cx, ftags[f as usize]);
        let msg = format!("partial argument match of '{tag}' to '{formal}'");
        cx.warning_call(Some(call), &msg)?;
    }
    Ok(())
}

/// A closure call: `applyClosure(call, op, args, rho, suppliedvars,
/// unpromise)` with the arguments from `src` (the call's own, or the
/// promise list `arglist`), the value written to `out`. The frame is bound
/// here (`args::bind`), with S3 dispatch's variables `supplied` (a tagged
/// pairlist, `usemethod`'s) added; the body runs in the function record;
/// at the end the frame is cleaned (`R_CleanupEnvir`), a getter call's
/// shared value duplicated, and, if `unpromise`, the argument list's
/// promises released (`unpromiseArgs`).
// R: eval.c:applyClosure, applyClosure_core, make_applyClosure_env, R_execClosure
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    src: Source,
    arglist: Option<Root<'_>>,
    rho: Root<'_>,
    caller: Option<&Act>,
    supplied: Option<Root<'_>>,
    unpromise: bool,
    out: Out<'_>,
) -> RResult<()> {
    cx.safepoint(Safepoint::Entry)?;
    let res = out.root();
    cx.scope(|cx, s| {
        let code = closure_code(cx, op)?;
        // SAFETY: a code object of this isolate.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        // the profile of the code making the call: a constant argument of
        // compiled code is a value (PUSHCONSTARG), the promises it makes
        // are byte code; a call made at run time (complex assignment's) is
        // the AST's, unless a profile is forced
        let profile = match caller {
            // SAFETY: the caller's rooted code object.
            Some(a) => unsafe { crate::code::View::of(a.obj(crate::ir::R_CODE)).profile },
            None => prims::state(cx).forced.unwrap_or(Profile::Eval),
        };
        // the promises compiled code makes hold byte code, but a call of
        // `bquote` keeps its arguments' expressions (cmp.R's
        // maybeNSESymbols)
        // SAFETY: the rooted call.
        let bytecode = profile == Profile::Bc && unsafe { call.bind(cx).sexp().car() } != prims::state(cx).syms.bquote;
        let nil = cx.nil().sexp();
        let dots_sym = prims::state(cx).syms.dots;
        let mut acts: Buf<Actual> = Buf::new();
        let mut ftags: Buf<Sexp> = Buf::new();
        let mut fdefs: Buf<Sexp> = Buf::new();
        // R: eval.c:make_applyClosure_env's check of rho
        if rho.bind(cx).sexptype() != ENVSXP {
            let msg = format!(
                "'rho' must be an environment not {}: detected in C-level applyClosure",
                crate::eval::type_name(rho.bind(cx).sexptype())
            );
            return Err(cx.error_call(Some(call), &msg));
        }
        // the actuals and the match: no allocation, nothing collects
        let collected = {
            let (c, r, l) = (call.bind(cx).sexp(), rho.bind(cx).sexp(), arglist.map(|l| l.bind(cx).sexp()));
            // SAFETY: rooted objects of this isolate.
            unsafe {
                formals(cx, op.bind(cx).sexp(), &mut ftags, &mut fdefs);
                match (src, l) {
                    (Source::List, Some(l)) => {
                        list_actuals(cx, l, &mut acts);
                        Ok(true)
                    }
                    _ => call_actuals(cx, c, r, &mut acts),
                }
            }
        };
        let Ok(any_dots) = collected else {
            return Err(cx.error("'...' used in an incorrect context"));
        };
        // a memoized plan for this site and callee, or the matcher
        // SAFETY: the rooted code object.
        let stamp = unsafe { crate::code::stamp(code.bind(cx).sexp()) };
        let site = match (src, caller) {
            (Source::Site(site), Some(a)) if site != NO_SITE && !any_dots => Some((site, a)),
            _ => None,
        };
        let mut m = Matched::new(ftags.len());
        let cached = site.is_some_and(|(site, a)| {
            // SAFETY: the caller's live code object; its plans are plain
            // data, 8-aligned (a RAWSXP's data, at multiples of
            // PLAN_BYTES), read in this interval.
            unsafe {
                let Some(p) = crate::code::plan_bytes(a.obj(crate::ir::R_CODE), site) else { return false };
                let plan = &*(p as *const Plan);
                let hit = plan.stamp == stamp
                    && plan.nformals as usize == ftags.len()
                    && plan.nactuals as usize == acts.len();
                if hit {
                    plan.fill(&mut m);
                }
                hit
            }
        });
        match cached {
            true => {}
            false => {
                // SAFETY: live symbols (formals' tags, actuals' tags).
                let r = unsafe { match_args(ftags.as_slice(), acts.as_slice(), nil, dots_sym, &mut m) };
                if let Err(e) = r {
                    // the unused actuals, rooted (a warning's handler may
                    // run code before the message is made)
                    let unused = match e {
                        MatchError::Unused => unused_actuals(cx, s, acts.as_slice(), &m),
                        _ => Vec::new(),
                    };
                    if cx.warn_partial_match_args() {
                        partial_warnings(cx, call, m.partial.as_slice(), acts.as_slice(), ftags.as_slice())?;
                    }
                    let msg = match e {
                        MatchError::Unused => unused_message(cx, s, &unused)?,
                        _ => match_message(cx, e, acts.as_slice()),
                    };
                    return Err(cx.error_call(Some(call), &msg));
                }
                if let Some((site, a)) = site
                    && let Some(plan) = Plan::of(stamp, &m, acts.len())
                {
                    // SAFETY: as above.
                    unsafe {
                        if let Some(p) = crate::code::plan_bytes(a.obj(crate::ir::R_CODE), site) {
                            (p as *mut Plan).write(plan);
                        }
                    }
                }
            }
        }
        // what was read above stays valid until something runs code or
        // collects: a warning's handler, the `...` list's allocation
        let mut stale = false;
        if !m.partial.is_empty() && cx.warn_partial_match_args() {
            partial_warnings(cx, call, m.partial.as_slice(), acts.as_slice(), ftags.as_slice())?;
            stale = true;
        }
        let reread = |cx: &Cx, acts: &mut Buf<Actual>, ftags: &mut Buf<Sexp>, fdefs: &mut Buf<Sexp>| {
            let (c, r, l) = (call.bind(cx).sexp(), rho.bind(cx).sexp(), arglist.map(|l| l.bind(cx).sexp()));
            // SAFETY: rooted objects of this isolate; the same actuals as
            // matched (nothing rebinds the caller's `...` in between but a
            // warning handler, whose change GNU R would not see either:
            // its list was built)
            unsafe {
                formals(cx, op.bind(cx).sexp(), ftags, fdefs);
                match (src, l) {
                    (Source::List, Some(l)) => list_actuals(cx, l, acts),
                    _ => {
                        let _ = call_actuals(cx, c, r, acts);
                    }
                }
            }
        };
        if stale {
            reread(cx, &mut acts, &mut ftags, &mut fdefs);
        }
        // D4's control (branch g3/x2-d4-promsxp; a control, never merged):
        // GNU R's argument list instead of delayed slots. The call's own
        // arguments become `promiseArgs`' list (bcEval's MAKEPROM,
        // PUSHCONSTARG and DODOTS make the same one): a promise and a cons
        // per supplied argument, bound into the frame as objects and
        // cleared by `unpromiseArgs` at the call's end. The match above, and
        // its memoized plan, are unchanged: only the binding differs.
        // R: eval.c:promiseArgs
        let (src, arglist, unpromise) = match src {
            Source::List => (src, arglist, unpromise),
            Source::Site(_) => {
                let mut attempt = 0;
                let list = cx.retry_alloc(|cx| {
                    attempt += 1;
                    if attempt > 1 {
                        // a collection followed a refused attempt
                        reread(cx, &mut acts, &mut ftags, &mut fdefs);
                    }
                    let r = rho.bind(cx).sexp();
                    // SAFETY: rooted objects; the actuals were read in this
                    // interval.
                    unsafe { promise_list(cx, acts.as_slice(), r, profile, bytecode) }
                })?;
                // SAFETY: fresh, of this isolate.
                let l = s.root(cx, unsafe { Ref::from_sexp(list, cx) });
                // the list's elements in the actuals' order: the match's
                // indices still hold
                // SAFETY: the rooted list.
                unsafe { list_actuals(cx, l.bind(cx).sexp(), &mut acts) };
                (Source::List, Some(l), true)
            }
        };
        let reread = |cx: &Cx, acts: &mut Buf<Actual>, ftags: &mut Buf<Sexp>, fdefs: &mut Buf<Sexp>| {
            let (c, r, l) = (call.bind(cx).sexp(), rho.bind(cx).sexp(), arglist.map(|l| l.bind(cx).sexp()));
            // SAFETY: as for the `reread` above.
            unsafe {
                formals(cx, op.bind(cx).sexp(), ftags, fdefs);
                match (src, l) {
                    (Source::List, Some(l)) => list_actuals(cx, l, acts),
                    _ => {
                        let _ = call_actuals(cx, c, r, acts);
                    }
                }
            }
        };
        // the `...` list (allocates), then everything the frame binds rooted
        let dots_root = if m.dots_formal != NO && !m.dots.is_empty() {
            let mut attempt = 0;
            let mut a2: Buf<Actual> = Buf::new();
            let (mut t2, mut d2) = (Buf::new(), Buf::new());
            let d = cx.retry_alloc(|cx| {
                attempt += 1;
                let r = rho.bind(cx).sexp();
                // SAFETY: rooted objects; after a refused attempt (a
                // collection followed) the actuals are read again
                unsafe {
                    if attempt == 1 {
                        dots_list(cx, acts.as_slice(), &m, r, profile, bytecode)
                    } else {
                        reread(cx, &mut a2, &mut t2, &mut d2);
                        dots_list(cx, a2.as_slice(), &m, r, profile, bytecode)
                    }
                }
            })?;
            stale |= attempt > 1;
            // SAFETY: fresh, of this isolate.
            Some(s.root(cx, unsafe { Ref::from_sexp(d, cx) }))
        } else {
            None
        };
        if stale {
            reread(cx, &mut acts, &mut ftags, &mut fdefs);
        }
        let missing = cx.missing_arg().sexp();
        let nf = ftags.len();
        let mut inits: Buf<SlotInit<'_>> = Buf::filled(SlotInit::Missing, nf);
        for i in 0..nf {
            let default = fdefs[i];
            // the default for a formal with no value (match MISSING 1 → 2)
            let no_value = |cx: &Cx| {
                if default != missing {
                    // SAFETY: the formal's default, part of the rooted closure.
                    SlotInit::Default(s.root(cx, unsafe { Ref::from_sexp(default, cx) }))
                } else {
                    SlotInit::Missing
                }
            };
            inits[i] = if i as u16 == m.dots_formal {
                // R_MissingArg (nothing gobbled) or the list, MISSING 0
                SlotInit::Bound { value: dots_root.unwrap_or_else(|| s.root(cx, cx.missing_arg())), missing: 0 }
            } else if m.src[i] == NO {
                no_value(cx)
            } else {
                let a = acts[m.src[i] as usize];
                if a.missing {
                    no_value(cx)
                } else {
                    // SAFETY: the actual's value, reachable from the rooted
                    // call, the caller's `...` or the list.
                    let v = s.root(cx, unsafe { Ref::from_sexp(a.val, cx) });
                    match a.from {
                        Origin::Obj => SlotInit::Bound { value: v, missing: 0 },
                        // SAFETY: as above.
                        Origin::Expr if profile == Profile::Bc && unsafe { is_constant(a.val) } => {
                            // SAFETY: as above.
                            unsafe { mark_not_mutable(a.val) };
                            SlotInit::Bound { value: v, missing: 0 }
                        }
                        Origin::Expr => SlotInit::Delayed { expr: v, value: None, bytecode },
                        // (`...`'s double promise: its code is the promise)
                        Origin::Dot => SlotInit::Delayed { expr: v, value: None, bytecode: false },
                    }
                }
            };
        }
        // SAFETY: the code's shape, a rooted part of the code object.
        let shape = s.root(cx, unsafe { Ref::from_sexp(crate::code::shape(code.bind(cx).sexp()), cx) });
        // SAFETY: CLOENV of the rooted closure.
        let enclos = s.root(cx, unsafe { Ref::from_sexp(op.bind(cx).sexp().tag(), cx) });
        let newrho = s.out(cx);
        let newrho_r = newrho.root();
        cx.new_closure_frame(shape, enclos, rho, inits.as_slice(), newrho)?;
        let newrho = newrho_r;
        if cx.env_has_no_special_symbols(newrho.bind(cx)) {
            cx.set_no_special_symbols(newrho);
        }
        drop(inits);
        // R: eval.c:addMissingVarsToNewEnv (usemethod's variables)
        if let Some(sv) = supplied {
            add_missing_vars(cx, newrho, sv)?;
        }
        cx.begin_call(newrho);
        // D2's pairlist control: the frame made each default's promise
        // (`make_applyClosure_env`); a supplied argument is already its own
        // R: eval.c:applyClosure_core, "If we have a generic function we
        // need to use the sysparent of the generic as the sysparent of the
        // method"
        let g = cx.global_context();
        let sysparent = if g.callflag() == ctxt::GENERIC { g.sysparent().sexp() } else { rho.bind(cx).sexp() };
        let fill = FillData { acts: &acts, matched: &m };
        let promargs = match (src, arglist) {
            (Source::List, Some(l)) => Ok(l.bind(cx).sexp()),
            _ => Err((
                acts.len() as i32,
                fill_promargs as robigo_rtcore::PromargsFill,
                &fill as *const FillData as usize,
            )),
        };
        let spec = RawFunctionContext {
            callflag: ctxt::RETURN,
            call: call.bind(cx).sexp(),
            cloenv: newrho.bind(cx).sexp(),
            sysparent,
            callfun: op.bind(cx).sexp(),
            promargs,
        };
        let mut on_exit = |cx: &mut Cx, expr: Root<'_>, env: Root<'_>| -> RResult<()> {
            cx.scope(|cx, s| {
                let o = s.out(cx);
                crate::eval::eval(cx, expr, env, o)
            })
        };
        // SAFETY: every object of `spec` is rooted in this scope or by the
        // caller (`sysparent`: the caller's environment, or a field of a
        // live record); `fill` outlives the record (popped before this
        // returns).
        let r = unsafe {
            cx.function_context_raw(&spec, out, &mut on_exit, |cx, _id, out| {
                let slot = NonNull::new_unchecked(out.root().as_ptr() as *mut Slot);
                vm::exec(cx, code.bind(cx).sexp(), newrho.bind(cx).sexp(), slot, true)
            })
        };
        // (`fill` lives to here: the record that pointed at it is popped)
        let _ = &fill;
        if let Err(e) = r {
            // a jump past the call: GNU R's cleanup does not run
            cx.end_call(newrho, false);
            return Err(e);
        }
        // R: eval.c:applyClosure_core (ADJUST_ENVIR_REFCNTS)
        // SAFETY: the rooted call: CAR and CADR read.
        let is_getter_call = unsafe {
            let c = call.bind(cx).sexp();
            let tmp = prims::state(cx).syms.tmpval;
            c.cdr() != nil && c.cdr().car() == tmp && !is_replace_symbol(c.car())
        };
        cx.cleanup_envir(newrho, Some(res));
        if is_getter_call && res.bind(cx).maybe_referenced() {
            let v = s.root(cx, res.bind(cx));
            let o = res.reset(cx);
            if let Err(e) = cx.shallow_duplicate(v, o) {
                cx.end_call(newrho, false);
                return Err(e);
            }
        }
        cx.end_call(newrho, true);
        if unpromise && let (Source::List, Some(l)) = (src, arglist) {
            cx.unpromise_args(l);
        }
        Ok(())
    })
}

/// `addMissingVarsToNewEnv(newrho, addVars)`: the variables of the tagged
/// pairlist `vars` bound in the new frame before its arguments in listing
/// order (GNU R prepends the list to FRAME), a variable the frame already
/// binds (a formal) keeping the frame's binding (a later one in the list
/// has precedence over a sooner one).
// R: eval.c:addMissingVarsToNewEnv
fn add_missing_vars(cx: &mut Cx, newrho: Root<'_>, vars: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        // the list's (symbol, value) pairs, in order
        let pairs: Vec<(Root<'_>, Root<'_>)> = {
            let mut v = Vec::new();
            let mut l = vars.bind(cx);
            while let Some(c) = l.as_cons() {
                v.push((c.tag(), c.car()));
                l = c.cdr();
            }
            v.into_iter().map(|(t, x)| (s.root(cx, t), s.root(cx, x))).collect()
        };
        // defined last first: the first has the newest stamp, and lists
        // first; a name listed twice keeps its later value
        for (i, &(sym, val)) in pairs.iter().enumerate().rev() {
            let sy = sym.bind(cx).sexp();
            if pairs[i + 1..].iter().any(|(t, _)| t.bind(cx).sexp() == sy) {
                continue;
            }
            let bound = {
                let robigo_rtcore::View::Env(env) = newrho.bind(cx).view() else {
                    unreachable!("robigo: a closure frame that is not an environment")
                };
                let sy = sym.bind(cx).as_sym().expect("robigo: an S3 variable's tag");
                !matches!(cx.frame_binding(env, sy), robigo_rtcore::Lookup::Unbound)
            };
            if !bound {
                cx.define_var(sym, val, newrho)?;
            }
        }
        Ok(())
    })
}

/// `R_isReplaceSymbol(fun)`: a symbol whose name ends in `<-`.
///
/// # Safety
/// `f` is live.
// R: eval.c:R_isReplaceSymbol
unsafe fn is_replace_symbol(f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { f.typ() == SYMSXP && f.car().char_bytes().ends_with(b"<-") }
}

/// The record's `promargs`, made when first read (C2's lazy `promargs`):
/// GNU R's argument list in actual order, each element what
/// `promiseArgs` put there: the promise of the argument slot it is bound
/// to (made once, shared with the frame), the `...` list's element, a
/// `Bc` constant, or R_MissingArg. The list's cells count nothing (the
/// promises' list reference is kept by rtcore's residues).
fn fill_promargs(cx: &mut Cx, id: ContextId, data: usize, out: Out<'_>) -> RResult<()> {
    // SAFETY: `data` is the `FillData` of the record's `apply` frame, live
    // while the record is (the filler runs only on a live record).
    let d = unsafe { &*(data as *const FillData) };
    // SAFETY: as above.
    let (acts, m) = unsafe { (&*d.acts, &*d.matched) };
    let n = acts.len();
    cx.scope(|cx, s| {
        let rec = cx.context(id).expect("robigo: promargs of a record that is gone");
        let frame = s.root(cx, rec.cloenv());
        let call = s.root(cx, rec.call());
        let vals = s.frame(cx, n);
        let missing = cx.missing_arg().sexp();
        let dots_sym = prims::state(cx).syms.dots;
        // where each actual went
        let mut dest: Buf<u16> = Buf::filled(NO, n);
        for (fi, &a) in m.src.iter().enumerate() {
            if a != NO {
                dest[a as usize] = fi as u16;
            }
        }
        for j in 0..n {
            let a = acts[j];
            // SAFETY: slot `j` of this scope's frame.
            let slot = unsafe { vals.add(j) };
            let v = if a.missing {
                missing
            } else if let Some(k) = m.dots.iter().position(|&b| b as usize == j) {
                // the k-th element of the frame's `...` list
                // SAFETY: the rooted frame, read without running code.
                unsafe {
                    match cx.var_lookup_raw(dots_sym, frame.bind(cx).sexp()) {
                        RawBinding::Value(mut h) if h.typ() == DOTSXP => {
                            for _ in 0..k {
                                h = h.cdr();
                            }
                            h.car()
                        }
                        _ => return Err(cx.error("robigo: 'promargs' after '...' was rebound is not implemented yet")),
                    }
                }
            } else {
                let fi = dest[j];
                debug_assert!(fi != NO, "robigo: an actual neither matched nor in '...'");
                // SAFETY: slot `j` is this scope's, empty.
                let o = unsafe { Out::from_raw(slot) };
                if cx.arg_promise(frame, fi as usize, o)? {
                    continue;
                }
                // a value bound directly: a `Bc` constant, the call's own
                // argument expression
                // SAFETY: the rooted call's arguments.
                unsafe {
                    let mut el = call.bind(cx).sexp().cdr();
                    for _ in 0..a.pos {
                        el = el.cdr();
                    }
                    el.car()
                }
            };
            // SAFETY: a live object reachable from the frame or call.
            unsafe { slot.write(Slot::object(v)) };
        }
        cx.build(out, |cx| {
            let nil = cx.nil().sexp();
            let mut l = nil;
            // SAFETY: the values are rooted in this scope's frame; the cells
            // are fresh; nothing collects in this interval.
            unsafe {
                for j in (0..n).rev() {
                    let v = vals.add(j).read().as_object().unwrap();
                    let c = cx.alloc_node_raw(LISTSXP, v, l, acts[j].tag)?;
                    // CONS_NR: the construction's counts undone
                    v.decref();
                    if l != nil {
                        l.decref();
                    }
                    if acts[j].tag != nil {
                        acts[j].tag.decref();
                    }
                    c.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
                    l = c;
                }
                Ok(Ref::from_sexp(l, cx))
            }
        })
    })
}

// ---- entry points ----------------------------------------------------------------

/// The VM's closure call: the closure in register `rf` called by `call`
/// (a constant of the running code) from the activation's environment,
/// with call site `site`'s plan; the value into register `dst`.
pub fn call_closure(cx: &mut Cx, act: &Act, dst: Reg, rf: Reg, call: Sexp, site: u16) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code.
        let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        apply(cx, callr, act.root(rf), Source::Site(site), None, act.root(R_ENV), Some(act), None, true, act.out(dst))
    })
}
