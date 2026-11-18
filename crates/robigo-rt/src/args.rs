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
//! (docs/g3.md §16). CI runs the rt conformance under it against the
//! production recordings (`scripts/rt-ci.py`).

#![allow(unsafe_code)]

use crate::call::closure_code;
use crate::ir::{NO_SITE, Profile, R_ENV, Reg};
use crate::prims;
use crate::vm::{self, Act};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Safepoint, Sexp};
use robigo_rtcore::{
    ArgSlot, ContextId, Cx, Out, RError, RResult, RawBinding, RawFunctionContext, Ref, Reserved, Root, Scope, Slot,
    SlotInit, bytecode_expr, ctxt,
};
use std::mem::MaybeUninit;
use std::ptr::NonNull;

// ---- small stack vectors -------------------------------------------------------

/// A small vector on the stack (the common case: a call's arguments and a
/// closure's formals), spilling to the heap past [`INLINE`] elements: the
/// argument matcher runs on every call and must not allocate. The inline
/// storage is not initialized ahead of use (a call pays for the elements it
/// writes, not for the capacity). The length alone says where the
/// elements are: inline up to [`INLINE`], all of them on the heap past it,
/// so a push or a read on the common path tests one word.
const INLINE: usize = 8;

pub(crate) struct Buf<T: Copy> {
    inline: [std::mem::MaybeUninit<T>; INLINE],
    len: usize,
    /// Every element while `len > INLINE`; empty otherwise.
    heap: Vec<T>,
}

impl<T: Copy> Buf<T> {
    #[inline(always)]
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

    #[inline(always)]
    pub(crate) fn push(&mut self, v: T) {
        if self.len < INLINE {
            self.inline[self.len].write(v);
            self.len += 1;
        } else {
            self.push_heap(v);
        }
    }

    /// [`Buf::push`] past the inline storage: the first such push moves
    /// the inline elements to the heap.
    #[cold]
    #[inline(never)]
    fn push_heap(&mut self, v: T) {
        if self.len == INLINE {
            let init = self.as_slice().to_vec();
            self.heap = init;
        }
        self.heap.push(v);
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

    #[inline(always)]
    pub(crate) fn as_slice(&self) -> &[T] {
        if self.len <= INLINE {
            // SAFETY: the first `len` inline elements are written (`push`
            // and `filled` write before they count).
            unsafe { std::slice::from_raw_parts(self.inline.as_ptr() as *const T, self.len) }
        } else {
            &self.heap
        }
    }

    #[inline(always)]
    fn as_mut_slice(&mut self) -> &mut [T] {
        if self.len <= INLINE {
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
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        &self.as_slice()[i]
    }
}

impl<T: Copy> std::ops::IndexMut<usize> for Buf<T> {
    #[inline(always)]
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
    /// S3 dispatch's method called with its generic's arguments
    /// ([`Source::Generic`]): the generic's delayed or forced argument slot
    /// of this index, in the frame the actual's `val` is. The method's frame
    /// binds it as an alias of that slot (the promise GNU R's generic
    /// binds, made only when something needs it: robigo-rtcore's
    /// `SlotInit::Alias`); a `...` list gets the promise.
    Alias(u16),
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

/// Why the actuals of a call could not be collected.
enum ActualsError {
    /// `...` bound to something that is not a `...` list: "'...' used in an
    /// incorrect context".
    BadDots,
    /// More arguments than an actual's position encodes (a u16): robigo's
    /// limit, not GNU R's, reported as the matcher's
    /// [`MatchError::TooManyActuals`].
    TooMany,
}

/// The actuals of `call` in `rho`: its arguments in order, each `...`
/// expanded into the elements of the caller's `...` (`promiseArgs`' walk,
/// eval.c:3781). True when a `...` was expanded (the tags then vary per
/// call).
///
/// # Safety
/// `call` a live call and `rho` a live environment of `cx`'s isolate,
/// reachable from roots; nothing collects while the result is used.
unsafe fn call_actuals(cx: &Cx, call: Sexp, rho: Sexp, acts: &mut Buf<Actual>) -> Result<bool, ActualsError> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    acts.clear();
    let mut any_dots = false;
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        let mut el = call.cdr();
        // (counted past u16: a position that does not fit is refused below)
        let mut pos = 0u32;
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
                    _ => return Err(ActualsError::BadDots),
                }
            } else {
                acts.push(Actual { val: a, tag: el.tag(), from: Origin::Expr, missing: a == missing, pos: pos as u16 });
            }
            el = el.cdr();
            pos += 1;
        }
        // A call of more arguments than NO: refused as the matcher refuses
        // as many actuals. An empty `...` adds no actual, so the actuals'
        // count alone does not bound the positions.
        if pos > NO as u32 {
            return Err(ActualsError::TooMany);
        }
    }
    Ok(any_dots)
}

/// The actuals of `call`'s arguments as written: a `...` among them is an
/// argument, not expanded.
///
/// # Safety
/// `call` is a live call.
unsafe fn expr_actuals(cx: &Cx, call: Sexp, acts: &mut Buf<Actual>) {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    acts.clear();
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        let mut el = call.cdr();
        let mut pos = 0u16;
        while el != nil {
            let a = el.car();
            acts.push(Actual { val: a, tag: el.tag(), from: Origin::Expr, missing: a == missing, pos });
            el = el.cdr();
            pos = pos.saturating_add(1);
        }
    }
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

    /// [`Matched::new`]`(nformals)` of a `Matched::new(0)` nothing was
    /// pushed to (the matcher's fill data is made before the formals are
    /// read, [`prepare`]): its formals, none matched yet.
    #[inline(always)]
    fn set_formals(&mut self, nformals: usize) {
        debug_assert!(self.src.is_empty() && self.dots.is_empty() && self.partial.is_empty());
        for _ in 0..nformals {
            self.src.push(NO);
        }
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
    /// More formals than a match index encodes (a u16, with NO for none):
    /// robigo's limit, not GNU R's.
    TooManyFormals,
    /// More actuals than a match index encodes, or more call arguments than
    /// an actual's position does: robigo's limit, not GNU R's.
    TooManyActuals,
}

/// `pmatch(formal, tag, exact)`: `exact` or a prefix match.
///
/// # Safety
/// Both are live symbols.
// R: match.c:pmatch, psmatch
unsafe fn pmatch(formal: Sexp, tag: Sexp, exact: bool) -> bool {
    // SAFETY: the caller's contract.
    let (f, t) = unsafe { (formal.sym_name_bytes(), tag.sym_name_bytes()) };
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
    // (a memoized plan holds at most PLAN_FORMALS formals and PLAN_DOTS
    // actuals: only a match made here can meet these; `call_actuals`
    // refuses a call whose positions would not fit)
    if nf > NO as usize {
        return Err(MatchError::TooManyFormals);
    }
    if acts.len() > NO as usize {
        return Err(MatchError::TooManyActuals);
    }
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
    /// The callee cannot run inline from this site ([`no_inline_at`]).
    no_inline: u8,
    _pad: [u8; 2],
    src: [u8; PLAN_FORMALS],
    dots: [u8; PLAN_DOTS],
    partial: [[u8; 2]; PLAN_PARTIAL],
    /// The formals whose arguments this site no longer computes early
    /// (crate::early): their early values were dropped before the callee
    /// took them.
    no_early: u16,
    _pad2: [u8; 6],
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
            no_inline: 0,
            _pad: [0; 2],
            src: [PLAN_NO; PLAN_FORMALS],
            dots: [0; PLAN_DOTS],
            partial: [[0; 2]; PLAN_PARTIAL],
            no_early: 0,
            _pad2: [0; 6],
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

    /// Fill `m` (made for `nformals` formals) with the match this plan
    /// memoizes: for a call the plan alone does not bind ([`site_plan`]:
    /// partial matches to warn of, actuals for `...`).
    #[inline(always)]
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

/// How many match plans of methods called with their generics' arguments
/// [`MethodPlans`] keeps.
const METHOD_PLANS: usize = 16;

/// One method's match plan ([`MethodPlans`]): the actuals' tags (symbols,
/// immortal, or R_NilValue) and which are R_MissingArg, which the match
/// reads besides the callee's formals.
#[derive(Clone, Copy)]
struct MethodPlan {
    tags: [Sexp; PLAN_DOTS],
    missing: u16,
    plan: Plan,
}

/// The match plans of S3 methods called with their generics' arguments
/// ([`Source::Generic`]): no call site holds them (the method is called by
/// dispatch, not by code), so they are kept by the callee code's stamp and
/// the actuals' tags and missing ones, the most recently made first.
#[derive(Default)]
pub(crate) struct MethodPlans(Vec<MethodPlan>);

/// The actuals' missing ones as a bit mask (at most [`PLAN_DOTS`]).
fn missing_mask(acts: &[Actual]) -> u16 {
    acts.iter().enumerate().fold(0, |m, (i, a)| if a.missing { m | 1 << i } else { m })
}

/// [`MethodPlans`]' plan for the callee stamped `stamp` with `nformals`
/// formals and these actuals, filled into `m`; false when there is none.
/// Out of line: other calls never run it.
#[inline(never)]
fn method_plan_get(cx: &Cx, stamp: u64, nformals: usize, acts: &[Actual], m: &mut Matched) -> bool {
    if acts.len() > PLAN_DOTS {
        return false;
    }
    let missing = missing_mask(acts);
    let plans = prims::state(cx).cond.method_plans.borrow();
    let hit = plans.0.iter().find(|e| {
        e.plan.stamp == stamp
            && e.plan.nformals as usize == nformals
            && e.plan.nactuals as usize == acts.len()
            && e.missing == missing
            && acts.iter().zip(e.tags.iter()).all(|(a, &t)| a.tag == t)
    });
    match hit {
        Some(e) => {
            e.plan.fill(m);
            true
        }
        None => false,
    }
}

/// Keep `m`, the match for the callee stamped `stamp` and these actuals, in
/// [`MethodPlans`], if it fits a plan.
#[inline(never)]
fn method_plan_put(cx: &Cx, stamp: u64, m: &Matched, acts: &[Actual]) {
    let Some(plan) = Plan::of(stamp, m, acts.len()) else { return };
    let mut tags = [cx.nil().sexp(); PLAN_DOTS];
    for (t, a) in tags.iter_mut().zip(acts) {
        *t = a.tag;
    }
    let mut plans = prims::state(cx).cond.method_plans.borrow_mut();
    plans.0.insert(0, MethodPlan { tags, missing: missing_mask(acts), plan });
    plans.0.truncate(METHOD_PLANS);
}

/// Call site `site`'s plan of the callee code stamped `stamp`, when it
/// binds the call alone ([`plan_inits`]): no partial match whose warning is
/// due, no actual for `...`. The pointer is valid until the next safepoint
/// (the plans' RAWSXP may move then); a call keeps a copy (the site's plan
/// may be replaced while it runs).
///
/// # Safety
/// `a` is the caller's activation, its code object live.
#[inline(always)]
unsafe fn site_plan(a: &Act, site: u16, stamp: u64) -> Option<*const Plan> {
    // SAFETY: the caller's contract; plans are plain data in a RAWSXP,
    // 8-aligned at multiples of PLAN_BYTES.
    unsafe {
        let p = crate::code::plan_bytes(a.obj(crate::ir::R_CODE), site)? as *const Plan;
        ((*p).stamp == stamp && (*p).npartial == 0 && (*p).ndots == 0).then_some(p)
    }
}

/// The slot inits of `call` from `op`'s formals, as its site's plan `p`
/// ([`site_plan`]) matched them, pushed to the empty `inits`: what
/// [`apply`]'s matcher path makes, from the call's arguments (none of them
/// `...`: a site's plan is made only for such a call) and the formals'
/// defaults, without the actuals' and the formals' buffers. The arguments'
/// tags go to `tags` and the mask of those that are R_MissingArg is
/// returned: what the call's lazy `promargs` reads later ([`FillData`]).
/// None, with `inits` cleared, when the call's or the formals' count is not
/// the plan's; nothing observable is done before that is known.
///
/// # Safety
/// `call` is a live call and `op` a live closure, both rooted.
#[inline(always)]
unsafe fn plan_inits<'s>(
    cx: &Cx,
    s: Scope<'s>,
    p: &Plan,
    call: Sexp,
    op: Sexp,
    profile: Profile,
    bytecode: bool,
    codes: Sexp,
    inits: &mut Buf<SlotInit<'s>>,
    tags: &mut [MaybeUninit<Sexp>; PLAN_DOTS],
) -> Option<u16> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let mut vals = [nil; PLAN_DOTS];
    let mut absent = 0u16;
    // SAFETY: the caller's contract: the call's and the formals' cells are
    // read, the values rooted in `s`; nothing collects.
    unsafe {
        let mut el = call.cdr();
        for j in 0..(p.nactuals as usize).min(PLAN_DOTS) {
            if el == nil {
                return None;
            }
            let v = el.car();
            vals[j] = v;
            tags[j].write(el.tag());
            if v == missing {
                absent |= 1 << j;
            }
            el = el.cdr();
        }
        if el != nil {
            return None;
        }
        let mut f = op.car();
        for i in 0..p.nformals as usize {
            if f == nil {
                inits.clear();
                return None;
            }
            let a = p.src[i];
            inits.push(if i == p.dots_formal as usize {
                // the `...` formal, gobbling nothing: R_MissingArg, MISSING 0
                SlotInit::Bound { value: s.root(cx, cx.missing_arg()), missing: 0 }
            } else if a == PLAN_NO || absent & (1 << a) != 0 {
                match f.car() {
                    d if d == missing => SlotInit::Missing,
                    d => SlotInit::Default(s.root(cx, Ref::from_sexp(d, cx))),
                }
            } else {
                let v = vals[a as usize];
                if profile == Profile::Bc && is_constant(v) {
                    mark_not_mutable(v);
                    SlotInit::Bound { value: s.root(cx, Ref::from_sexp(v, cx)), missing: 0 }
                } else {
                    // (a translated call's promise holds its own code)
                    let code = arg_code(codes, a as usize);
                    let expr = s.root(cx, Ref::from_sexp(if code.is_null() { v } else { code }, cx));
                    SlotInit::Delayed { expr, value: None, bytecode }
                }
            });
            f = f.cdr();
        }
        if f != nil {
            inits.clear();
            return None;
        }
        Some(absent)
    }
}

/// Whether call site `site` of the caller's code `code` has the memoized
/// plan of the callee code stamped `stamp`, marked as a site whose call of
/// it cannot run inline (`crate::inline`: its arguments cannot be matched
/// there, which the call's fixed expression and the callee's formals
/// decide).
///
/// # Safety
/// `code` is a live code object.
#[inline(always)]
pub(crate) unsafe fn no_inline_at(code: Sexp, site: u16, stamp: u64) -> bool {
    // SAFETY: the caller's contract; plans are plain data in a RAWSXP,
    // 8-aligned at multiples of PLAN_BYTES.
    unsafe {
        crate::code::plan_bytes(code, site).is_some_and(|p| {
            let plan = &*(p as *const Plan);
            plan.stamp == stamp && plan.no_inline != 0
        })
    }
}

/// Mark site `site`'s plan of the callee code stamped `stamp` (if it has
/// one) for [`no_inline_at`]. A plan made again for the site starts
/// unmarked.
///
/// # Safety
/// As for [`no_inline_at`].
pub(crate) unsafe fn mark_no_inline_at(code: Sexp, site: u16, stamp: u64) {
    // SAFETY: as in `no_inline_at`.
    unsafe {
        if let Some(p) = crate::code::plan_bytes(code, site) {
            let plan = &mut *(p as *mut Plan);
            if plan.stamp == stamp {
                plan.no_inline = 1;
            }
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
    unsafe { expr_actuals(cx, call.bind(cx).sexp(), &mut acts) };
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
    /// site `site` of the caller's code (or [`NO_SITE`]), with the pool
    /// index in the caller's code of a translated call's promise codes
    /// ([`arg_code`]; 0 for none). An index, not the list: it is read from
    /// the caller's rooted code where it is used, after anything that may
    /// have collected (and moved it).
    Site(u16, u16),
    /// A promise list (`applyClosure(call, op, arglist, rho, ...)`).
    List,
    /// S3 dispatch's method called with the generic's arguments
    /// (`dispatchMethod`'s `cptr->promargs`, record `g`), read from the
    /// generic's record and frame as they are ([`generic_actuals`]), the
    /// list made only when one cannot be read so.
    Generic(ContextId),
}

/// What the lazy `promargs` filler needs of a call: its actuals (tags,
/// kinds, positions; values are read again from the frame and the call)
/// and the match. The call's record reads it while the callee runs, so it
/// lives beside the record, in the fill area of the record's slot
/// (robigo-rtcore's record stack, X9-1c), not on the C stack: [`apply`]
/// reserves the slot, [`prepare`] fills it, its variants sharing the
/// storage.
enum FillData {
    /// Not made yet: [`prepare`] did not get that far.
    Unset,
    /// As the matcher made them.
    Matched { acts: Buf<Actual>, matched: Matched },
    /// A call bound by its site's plan ([`plan_inits`]): the actuals are
    /// the call's own arguments, matched as the plan says, with their tags
    /// (the first `plan.nactuals` written) and the mask of those that were
    /// R_MissingArg, read at the call: C may rewrite the call while it
    /// runs, and GNU R's list was made at the call.
    Plan { plan: Plan, tags: [MaybeUninit<Sexp>; PLAN_DOTS], missing: u16 },
}

// The fill data fits the fill area beside each record (robigo-rtcore's
// `FILL_SIZE`, `FILL_ALIGN`; `Reserved::fill` checks it again).
const _: () =
    assert!(size_of::<FillData>() <= robigo_rtcore::FILL_SIZE && align_of::<FillData>() <= robigo_rtcore::FILL_ALIGN);

/// Where the match put an actual of a call with a lazy `promargs`.
#[derive(Clone, Copy)]
enum Went {
    /// Nowhere: the actual is R_MissingArg.
    Missing,
    /// The k-th element of the frame's `...` list.
    Dots(u16),
    /// The slot of formal `formal` ([`NO`]: none); `pos`, its position
    /// among the call's arguments.
    Formal { formal: u16, pos: u16 },
}

impl FillData {
    /// Each actual of the call, in order: its tag (a symbol or R_NilValue)
    /// and where the match put it, given to `f` until it returns false
    /// (then false). A call bound by a plan is read from the plan and the
    /// tags, without the matcher's buffers.
    ///
    /// # Safety
    /// `self` is a `FillData` [`prepare`] made (the `FillData` of a live
    /// record).
    #[inline(always)]
    unsafe fn each_actual(&self, mut f: impl FnMut(Sexp, Went) -> bool) -> bool {
        match self {
            FillData::Unset => unreachable!("robigo: the promargs of a call whose arguments were not bound"),
            FillData::Matched { acts, matched: m } => {
                let mut dest: Buf<u16> = Buf::filled(NO, acts.len());
                for (fi, &a) in m.src.iter().enumerate() {
                    if a != NO {
                        dest[a as usize] = fi as u16;
                    }
                }
                for (j, a) in acts.iter().enumerate() {
                    let went = if a.missing {
                        Went::Missing
                    } else if let Some(k) = m.dots.iter().position(|&b| b as usize == j) {
                        Went::Dots(k as u16)
                    } else {
                        Went::Formal { formal: dest[j], pos: a.pos }
                    };
                    if !f(a.tag, went) {
                        return false;
                    }
                }
                true
            }
            FillData::Plan { plan: p, tags, missing } => {
                let mut dest = [NO; PLAN_DOTS];
                for (fi, &a) in p.src[..p.nformals as usize].iter().enumerate() {
                    if a != PLAN_NO {
                        dest[a as usize] = fi as u16;
                    }
                }
                // (none of the call's arguments is `...` or went there)
                for (j, (&formal, tag)) in dest.iter().zip(tags).take(p.nactuals as usize).enumerate() {
                    let went =
                        if missing & (1 << j) != 0 { Went::Missing } else { Went::Formal { formal, pos: j as u16 } };
                    // SAFETY: the first `nactuals` tags are written
                    // ([`plan_inits`]).
                    if !f(unsafe { tag.assume_init() }, went) {
                        return false;
                    }
                }
                true
            }
        }
    }

    /// Make this fill data, which owns nothing (`Unset`, or a `Plan` that
    /// did not bind the call), `new`. The old value is forgotten, not
    /// dropped: `*self = new` would call the drop glue on every call.
    #[inline(always)]
    fn set(&mut self, new: FillData) {
        debug_assert!(!matches!(self, FillData::Matched { .. }), "robigo: fill data replaced after the match");
        std::mem::forget(std::mem::replace(self, new));
    }

    /// How many actuals the call has.
    fn nactuals(&self) -> usize {
        match self {
            FillData::Unset => unreachable!("robigo: the promargs of a call whose arguments were not bound"),
            FillData::Matched { acts, .. } => acts.len(),
            FillData::Plan { plan, .. } => plan.nactuals as usize,
        }
    }
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
    let dots = prims::state(cx).syms.dots;
    let mut acts: Buf<Actual> = Buf::new();
    let mut ftags: Buf<Sexp> = Buf::new();
    // SAFETY: the caller's contract.
    unsafe {
        expr_actuals(cx, call, &mut acts);
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

/// The promise code of call argument `pos` (`MAKEPROM`'s BCODESXP, GNU
/// R's PRCODE) among a translated call's codes `codes`, or null for none:
/// a call without them (null: the from-source compiler's, the AST's),
/// or an argument that is no promise of byte code (a constant, `...`, an
/// empty one, `bquote`'s). The promise holds the code; what reads its
/// expression reads the code's first constant
/// ([`robigo_rtcore::bytecode_expr`]).
///
/// # Safety
/// `codes` is null, R_NilValue or a live VECSXP.
#[inline(always)]
pub(crate) unsafe fn arg_code(codes: Sexp, pos: usize) -> Sexp {
    if codes.is_null() {
        return Sexp::NULL;
    }
    // SAFETY: the caller's contract.
    unsafe {
        if codes.typ() != VECSXP || pos >= codes.std_len() as usize {
            return Sexp::NULL;
        }
        let c = *(codes.std_data() as *const Sexp).add(pos);
        if c.typ() == BCODESXP { c } else { Sexp::NULL }
    }
}

/// The element GNU R's argument list (and so a `...` list) holds for
/// actual `a` of a call from `rho`: `promiseArgs`' promise, R_MissingArg,
/// a `Bc` constant (marked not mutable, `PUSHCONSTARG`), or a list's own
/// element.
///
/// # Safety
/// `a`'s value and `rho` are live and reachable from roots.
unsafe fn list_element(
    cx: &Cx,
    a: &Actual,
    rho: Sexp,
    profile: Profile,
    bytecode: bool,
    codes: Sexp,
) -> Result<Sexp, NeedGc> {
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
                // (a translated call's promise holds its own code)
                let code = arg_code(codes, a.pos as usize);
                let p = mk_promise(cx, if code.is_null() { a.val } else { code }, rho)?;
                if bytecode {
                    // compiled code's promise holds byte code (MAKEPROM)
                    p.set_extra(p.extra() | robigo_heap::extra::promise::BYTECODE);
                }
                Ok(p)
            }
            Origin::Dot => mk_promise(cx, a.val, rho),
            // ([`apply`] makes such an actual its promise first)
            Origin::Alias(_) => unreachable!("robigo: an argument alias in a list"),
        }
    }
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
    codes: Sexp,
) -> Result<Sexp, NeedGc> {
    let mut d = cx.nil().sexp();
    // SAFETY: the caller's contract; nothing collects in this interval, so
    // the list built so far stays valid.
    unsafe {
        for &bi in m.dots.as_slice().iter().rev() {
            let a = &acts[bi as usize];
            let v = list_element(cx, a, rho, profile, bytecode, codes)?;
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
            let name = unsafe { Ref::from_sexp(s, cx) }.as_sym().unwrap().name(cx);
            format!(
                "formal argument \"{}\" matched by multiple actual arguments",
                String::from_utf8_lossy(name.bytes())
            )
        }
        MatchError::ArgTwice(i) => format!("argument {i} matches multiple formal arguments"),
        MatchError::TooManyFormals => {
            format!("robigo: a function of more than {NO} formal arguments is not implemented yet")
        }
        MatchError::TooManyActuals => format!("robigo: a call of more than {NO} arguments is not implemented yet"),
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
            // since the match; PREXPR of a promise is its code's expression
            // (`bytecodeExpr`).
            unsafe {
                let v = match a.from {
                    // the expression of the generic's argument
                    Origin::Alias(i) => match cx.arg_slot_raw(a.val, i as usize) {
                        Some(ArgSlot::Delayed { expr } | ArgSlot::Forced { expr, .. }) => bytecode_expr(expr, nil),
                        Some(ArgSlot::Promise(p)) => bytecode_expr(p.cdr(), nil),
                        None => unreachable!("robigo: an argument alias of no argument"),
                    },
                    _ if a.val.typ() == PROMSXP => bytecode_expr(a.val.cdr(), nil),
                    _ => a.val,
                };
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
    // (deparse forces a promise it meets, as GNU R's does)
    let text = match crate::deparse::deparse1line_forcing(cx, l, false, 0)? {
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
        // SAFETY: the tags are symbols: immortal.
        let tag = unsafe { crate::eval::sym_name(cx, acts[a as usize].tag) };
        // SAFETY: the tags are symbols: immortal.
        let formal = unsafe { crate::eval::sym_name(cx, ftags[f as usize]) };
        let msg = format!("partial argument match of '{tag}' to '{formal}'");
        cx.warning_call(Some(call), &msg)?;
    }
    Ok(())
}

/// A closure call: `applyClosure(call, op, args, rho, suppliedvars,
/// unpromise)` with the arguments from `src` (the call's own, or the
/// promise list `arglist`), the value written to `out`. The frame is bound
/// here (`args::bind`), with S3 dispatch's variables `supplied`
/// (`usemethod`'s) added; the body runs in the function record;
/// at the end the frame is cleaned (`R_CleanupEnvir`), a getter call's
/// shared value duplicated, and, if `unpromise`, the argument list's
/// promises released (`unpromiseArgs`).
// R: eval.c:applyClosure, applyClosure_core, make_applyClosure_env, R_execClosure
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn apply(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    src: Source,
    arglist: Option<Root<'_>>,
    rho: Root<'_>,
    caller: Option<&Act>,
    supplied: Option<&crate::dispatch::S3Vars<'_>>,
    unpromise: bool,
    out: Out<'_>,
) -> RResult<()> {
    // (a method called with its generic's arguments has its own instance:
    // every other call's has none of that code)
    match src {
        Source::Generic(_) => apply_with::<true>(cx, call, op, src, arglist, rho, caller, supplied, unpromise, out),
        _ => apply_with::<false>(cx, call, op, src, arglist, rho, caller, supplied, unpromise, out),
    }
}

/// [`apply`], `GENERIC` when `src` is [`Source::Generic`]. The arguments
/// are bound and the frame made out of line, by [`prepare`]: what this part
/// keeps while the callee's body runs (its frame is one of those every R
/// call level keeps, X9) is a few words. The call's record and the fill
/// data it reads ([`FillData`]) are in a slot of the record stack, reserved
/// before [`prepare`] runs (whose warnings and early values may run R code,
/// whose records take the slots after it).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn apply_with<const GENERIC: bool>(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    src: Source,
    arglist: Option<Root<'_>>,
    rho: Root<'_>,
    caller: Option<&Act>,
    supplied: Option<&crate::dispatch::S3Vars<'_>>,
    unpromise: bool,
    out: Out<'_>,
) -> RResult<()> {
    // R code is about to run: values computed early for an earlier call's
    // arguments are dropped (crate::early)
    cx.note_effect();
    cx.safepoint(Safepoint::Entry)?;
    // (moved into the scope's closure, and from there to [`apply_in`], where
    // [`prepare`] reads them through one reference)
    let a = CallIn { call, op, rho, caller, supplied, src, arglist, unpromise };
    cx.scope(move |cx, s| {
        // (a method's call stays inline in its one caller, `dispatch_method`:
        // out of line it cost `s3` 11 instructions more an iteration, X9-1c)
        if GENERIC { apply_in::<GENERIC>(cx, s, a, out) } else { apply_out_of_line(cx, s, a, out) }
    })
}

/// [`apply_in`] out of line, so that the reservation is in this frame only,
/// not in its callers' (the VM's `run` loop: with the reservation inlined
/// there, LLVM laid `run` out again and `plain` cost 13 instructions more
/// an iteration although it makes no call through here, X9-1c).
#[inline(never)]
fn apply_out_of_line(cx: &mut Cx, s: Scope<'_>, a: CallIn<'_, '_>, out: Out<'_>) -> RResult<()> {
    apply_in::<false>(cx, s, a, out)
}

/// [`apply_with`] in the scope `s`: the record slot reserved, the call
/// prepared and made.
#[inline(always)]
fn apply_in<const GENERIC: bool>(cx: &mut Cx, s: Scope<'_>, a: CallIn<'_, '_>, out: Out<'_>) -> RResult<()> {
    cx.with_reserved(move |cx, rec_slot| apply_reserved::<GENERIC>(cx, s, a, rec_slot, out))
}

/// [`apply_in`] in the record slot `rec_slot` reserved.
#[inline(always)]
fn apply_reserved<'s, const GENERIC: bool>(
    cx: &mut Cx,
    s: Scope<'s>,
    a: CallIn<'_, '_>,
    rec_slot: Reserved<'_, FillData>,
    out: Out<'_>,
) -> RResult<()> {
    let res = out.root();
    let fill = rec_slot.fill();
    // SAFETY: the reserved slot's fill area, sized and aligned for a
    // `FillData` (`Reserved::fill`), uninitialized: written once here,
    // then read and written only through `fill` (by `prepare`, this
    // function and the record's filler) until `_fill` drops it.
    unsafe { fill.as_ptr().write(FillData::Unset) };
    let _fill = FillGuard(fill);
    let mut st = CallState { generic_list: None, early: None, fill };
    let (code, newrho) = prepare::<GENERIC>(cx, s, &a, &mut st)?;
    let (src, arglist) = match st.generic_list {
        Some(l) if GENERIC => (Source::List, Some(l)),
        _ => (a.src, a.arglist),
    };
    // R: eval.c:applyClosure_core, "If we have a generic function we
    // need to use the sysparent of the generic as the sysparent of the
    // method"
    let g = cx.global_context();
    let sysparent = if g.callflag() == ctxt::GENERIC { g.sysparent().sexp() } else { a.rho.bind(cx).sexp() };
    let promargs = match (src, arglist) {
        (Source::List, Some(l)) => Ok(l.bind(cx).sexp()),
        (Source::Generic(gr), _) if GENERIC => Err((
            st.fill_data().nactuals() as i32,
            generic_promargs as robigo_rtcore::PromargsFill,
            gr.addr(),
            FILL_GENERIC,
        )),
        _ => Err((
            st.fill_data().nactuals() as i32,
            fill_promargs as robigo_rtcore::PromargsFill,
            st.fill.as_ptr() as usize,
            FILL_ARGS,
        )),
    };
    let spec = RawFunctionContext {
        callflag: ctxt::RETURN,
        call: a.call.bind(cx).sexp(),
        cloenv: newrho.bind(cx).sexp(),
        sysparent,
        callfun: a.op.bind(cx).sexp(),
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
    // live record); the fill data outlives the record (popped before
    // this returns; `_fill` drops it after).
    let r = unsafe {
        cx.function_context_raw(rec_slot, &spec, out, &mut on_exit, |cx, _id, out| {
            let slot = NonNull::new_unchecked(out.root().as_ptr() as *mut Slot);
            // (a scalar may come back unboxed when compiled code made
            // the call: its register takes it so)
            vm::exec(cx, code.bind(cx).sexp(), newrho.bind(cx).sexp(), slot, true, a.caller.is_some())
        })
    };
    // (the record that pointed at the fill data is popped)
    st.end_early(cx, newrho);
    // (true: an unboxed scalar in `res`, no object: the frame cannot
    // hold it, and no one else references it)
    let unboxed = match r {
        Ok(u) => u,
        Err(e) => {
            // a jump past the call: GNU R's cleanup does not run
            cx.end_call(newrho, false);
            return Err(e);
        }
    };
    // R: eval.c:applyClosure_core (ADJUST_ENVIR_REFCNTS)
    let nil = cx.nil().sexp();
    // SAFETY: the rooted call: CAR and CADR read.
    let is_getter_call = unsafe {
        let c = a.call.bind(cx).sexp();
        let tmp = prims::state(cx).syms.tmpval;
        c.cdr() != nil && c.cdr().car() == tmp && !is_replace_symbol(c.car())
    };
    cx.cleanup_envir(newrho, if unboxed { None } else { Some(res) });
    if is_getter_call && !unboxed && cx.referenced(res.bind(cx)) {
        let v = s.root(cx, res.bind(cx));
        let o = res.reset(cx);
        if let Err(e) = cx.shallow_duplicate(v, o) {
            cx.end_call(newrho, false);
            return Err(e);
        }
    }
    cx.end_call(newrho, true);
    if a.unpromise
        && let (Source::List, Some(l)) = (src, arglist)
    {
        cx.unpromise_args(l);
    }
    // D5: the frame to the pool, if nothing can reach it (env::recycle)
    cx.recycle_frame(newrho, if unboxed { None } else { Some(res) });
    Ok(())
}

/// Drops the [`FillData`] in a reserved slot's fill area on every path,
/// before the slot is released. A plan's fill data owns nothing: only the
/// matcher's is dropped, so that the common call does not call the drop
/// glue.
struct FillGuard(NonNull<FillData>);

impl Drop for FillGuard {
    #[inline(always)]
    fn drop(&mut self) {
        // SAFETY: the `FillData` [`apply_reserved`] wrote in the fill area of its
        // slot, which is still reserved (this guard is dropped first), and
        // which nothing reads any more (its record is popped).
        unsafe {
            if matches!(*self.0.as_ptr(), FillData::Matched { .. }) {
                self.0.as_ptr().drop_in_place();
            }
        }
    }
}

/// [`apply`]'s arguments that [`prepare`] reads (through one reference).
#[derive(Clone, Copy)]
struct CallIn<'r, 'a> {
    call: Root<'r>,
    op: Root<'r>,
    rho: Root<'r>,
    caller: Option<&'a Act>,
    supplied: Option<&'a crate::dispatch::S3Vars<'a>>,
    src: Source,
    arglist: Option<Root<'r>>,
    unpromise: bool,
}

/// A closure call's state in the frame of [`apply_in`] that spans the
/// callee's body: what [`prepare`] leaves for the rest of the call.
struct CallState<'r, 'a> {
    /// The generic's list, when a method called with its generic's
    /// arguments ([`Source::Generic`]) takes it instead ([`generic_source`]):
    /// the call's source is then [`Source::List`].
    generic_list: Option<Root<'r>>,
    /// The end of the call's early values, when it computed some.
    early: Option<EarlyEnd<'a>>,
    /// What the call's lazy `promargs` reads ([`prepare`] makes it), in
    /// the fill area of the call's record slot.
    fill: NonNull<FillData>,
}

/// What the end of a call that computed early values needs
/// ([`end_early_call`]).
#[derive(Clone, Copy)]
struct EarlyEnd<'a> {
    /// An outer call's record of dropped early values, given back.
    outer: (Sexp, u32),
    /// The caller's call site, for a call matched by a site's plan or one
    /// the matcher memoizes one for.
    site: Option<(u16, &'a Act)>,
    /// The callee code's stamp.
    stamp: u64,
}

impl CallState<'_, '_> {
    /// The call's fill data.
    #[inline(always)]
    fn fill_data(&self) -> &FillData {
        // SAFETY: `fill` points at the `FillData` of the call's reserved
        // slot (`apply_in`), live while this state is; nothing writes it
        // while the reference is used.
        unsafe { self.fill.as_ref() }
    }

    /// The end of the call's early values in its frame `newrho`
    /// ([`end_early_call`]), if it computed any.
    #[inline(always)]
    fn end_early(&self, cx: &mut Cx, newrho: Root<'_>) {
        if let Some(e) = self.early {
            end_early_call(cx, newrho.bind(cx).sexp(), e.outer, e.site, e.stamp);
        }
    }
}

/// [`apply_with`] until the callee's body runs: the arguments matched (by
/// the site's plan or the matcher) into slot inits, a method's aliases
/// reserved, the values computed early, the new frame made and S3
/// dispatch's variables bound in it, the call begun (and its arguments
/// materialized under `ROBIGO_MATERIALIZE=1`). The call's code object and
/// new frame are returned (rooted); what the call's lazy `promargs` reads
/// goes to `st.fill` (`Unset` on entry), which outlives the record, and
/// the rest of the call's state to `st`. Out of line, so that the matcher's
/// buffers, the slot inits and the temporaries of this work are not in the
/// frame that spans the callee's body (X9).
#[inline(never)]
fn prepare<'s: 'r, 'r, 'a, const GENERIC: bool>(
    cx: &mut Cx,
    s: Scope<'s>,
    a: &CallIn<'r, 'a>,
    st: &mut CallState<'r, 'a>,
) -> RResult<(Root<'r>, Root<'r>)> {
    let CallIn { call, op, rho, caller, supplied, src, arglist, .. } = *a;
    // SAFETY: the call's `FillData` (`CallState::fill`), which only this
    // function reads or writes until it returns.
    let fill = unsafe { &mut *st.fill.as_ptr() };
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
    // (the state's fields this call reads, in one lookup)
    let (bquote, dots_sym, materialize, forced) = {
        let st = prims::state(cx);
        (st.syms.bquote, st.syms.dots, st.materialize.get(), st.forced)
    };
    // SAFETY: the rooted call.
    let bytecode = profile == Profile::Bc && unsafe { call.bind(cx).sexp().car() } != bquote;
    let nil = cx.nil().sexp();
    // a translated call's promise codes (MAKEPROM's), by call argument: a
    // constant of the caller's code, read where it is used
    let codes_k = match src {
        Source::Site(_, k) => k,
        _ => 0,
    };
    let codes_now = || match caller {
        // SAFETY: the caller's rooted code object (its register, which a
        // collection updates); `codes_k` the pool index its CallCodes holds.
        Some(a) if codes_k != 0 => unsafe { crate::code::View::of(a.obj(crate::ir::R_CODE)).k(codes_k) },
        _ => Sexp::NULL,
    };
    // R: eval.c:make_applyClosure_env's check of rho
    if rho.bind(cx).sexptype() != ENVSXP {
        return Err(bad_rho(cx, call, rho));
    }
    // SAFETY: the rooted code object.
    let stamp = unsafe { crate::code::stamp(code.bind(cx).sexp()) };
    // a call from compiled code whose site's plan for this callee binds
    // it alone: the call's own arguments to the slot inits, as the plan
    // matched them, without the matcher's buffers
    let mut inits: Buf<SlotInit<'_>> = Buf::new();
    let hit = match (src, caller) {
        (Source::Site(site, _), Some(a)) if !GENERIC && site != NO_SITE => {
            // SAFETY: the caller's activation; the rooted call (a site's
            // plan is made only for a call without `...`) and closure;
            // the plan read before anything can collect.
            unsafe {
                site_plan(a, site, stamp).and_then(|p| {
                    // (the plan and the argument tags, read by the lazy
                    // `promargs` filler while the call's record lives)
                    fill.set(FillData::Plan {
                        plan: *p,
                        tags: [const { MaybeUninit::uninit() }; PLAN_DOTS],
                        missing: 0,
                    });
                    let FillData::Plan { tags, missing, .. } = fill else { unreachable!() };
                    let (c, f) = (call.bind(cx).sexp(), op.bind(cx).sexp());
                    plan_inits(cx, s, &*p, c, f, profile, bytecode, codes_now(), &mut inits, tags).map(|absent| {
                        *missing = absent;
                        (p, site, a)
                    })
                })
            }
        }
        _ => None,
    };
    // (a method's aliases among the inits)
    let mut aliases = 0usize;
    let (no_early, site, src, arglist) = match hit {
        // SAFETY: the plan of `site_plan`, nothing having collected since.
        Some((p, site, a)) => unsafe { ((*p).no_early, Some((site, a)), src, arglist) },
        None => {
            // (the matcher's actuals and match, read by the lazy
            // `promargs` filler while the call's record lives)
            fill.set(FillData::Matched { acts: Buf::new(), matched: Matched::new(0) });
            let FillData::Matched { acts, matched: m } = fill else { unreachable!() };
            let mut ftags: Buf<Sexp> = Buf::new();
            let mut fdefs: Buf<Sexp> = Buf::new();
            // a method called with its generic's arguments: read from the
            // generic's record and frame as they are, or, when one cannot be,
            // the generic's `promargs` made and passed as GNU R's
            // dispatchMethod passes it
            let (src, arglist) = match src {
                Source::Generic(g) if GENERIC => generic_source(cx, s, g, acts)?,
                _ => (src, arglist),
            };
            // the actuals and the match: no allocation, nothing collects
            let collected = {
                let (c, r, l) = (call.bind(cx).sexp(), rho.bind(cx).sexp(), arglist.map(|l| l.bind(cx).sexp()));
                // SAFETY: rooted objects of this isolate.
                unsafe {
                    formals(cx, op.bind(cx).sexp(), &mut ftags, &mut fdefs);
                    match (src, l) {
                        (Source::List, Some(l)) => {
                            list_actuals(cx, l, acts);
                            Ok(true)
                        }
                        // (read above)
                        (Source::Generic(_), _) if GENERIC => Ok(true),
                        _ => call_actuals(cx, c, r, acts),
                    }
                }
            };
            let any_dots = match collected {
                Ok(d) => d,
                Err(e) => return Err(actuals_error(cx, call, e)),
            };
            // (a formal's or an actual's index is a u16 with NO, u16::MAX, for
            // none: more than that is robigo's limit, not GNU R's)

            // a memoized plan for this site and callee, or the matcher
            let site = match (src, caller) {
                (Source::Site(site, _), Some(a)) if site != NO_SITE && !any_dots => Some((site, a)),
                _ => None,
            };
            m.set_formals(ftags.len());
            // the formals whose arguments this site no longer computes early
            let mut no_early = 0u16;
            // (a method called with its generic's arguments: the plans by
            // callee)
            let cached = (GENERIC && method_plan_get(cx, stamp, ftags.len(), acts.as_slice(), m))
                || site.is_some_and(|(site, a)| {
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
                            plan.fill(m);
                            no_early = plan.no_early;
                        }
                        hit
                    }
                });
            match cached {
                true => {}
                false => {
                    // SAFETY: live symbols (formals' tags, actuals' tags).
                    let r = unsafe { match_args(ftags.as_slice(), acts.as_slice(), nil, dots_sym, m) };
                    if let Err(e) = r {
                        return Err(match_error(cx, s, call, e, acts, m, &ftags));
                    }
                    if let Some((site, a)) = site
                        && let Some(plan) = Plan::of(stamp, m, acts.len())
                    {
                        // SAFETY: as above.
                        unsafe {
                            if let Some(p) = crate::code::plan_bytes(a.obj(crate::ir::R_CODE), site) {
                                (p as *mut Plan).write(plan);
                            }
                        }
                    } else if GENERIC {
                        method_plan_put(cx, stamp, m, acts.as_slice());
                    }
                }
            }
            // what was read above stays valid until something runs code or
            // collects: a warning's handler, the `...` list's allocation
            let mut stale = false;
            // (a method called with its generic's arguments takes the list,
            // made before the warnings: their handlers may change the generic's
            // frame, and GNU R's list was made before the match)
            let (src, arglist) = match src {
                Source::Generic(g) if GENERIC && !m.partial.is_empty() && cx.warn_partial_match_args() => {
                    generic_list(cx, s, g)?
                }
                _ => (src, arglist),
            };
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
                        (Source::Generic(g), _) if GENERIC => {
                            generic_actuals(cx, g, acts);
                        }
                        _ => {
                            let _ = call_actuals(cx, c, r, acts);
                        }
                    }
                }
            };
            // a method's actuals its `...` gets: their promises, which the
            // list holds (made now: the generic's slots then hold them)
            if GENERIC
                && let (Source::Generic(g), true) = (src, m.dots_formal != NO)
                && m.dots.iter().any(|&bi| matches!(acts[bi as usize].from, Origin::Alias(_)))
            {
                std::hint::cold_path();
                dots_promises(cx, s, g, acts, m)?;
                stale = true;
            }
            if stale {
                reread(cx, acts, &mut ftags, &mut fdefs);
            }
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
                            dots_list(cx, acts.as_slice(), m, r, profile, bytecode, codes_now())
                        } else {
                            reread(cx, &mut a2, &mut t2, &mut d2);
                            dots_list(cx, a2.as_slice(), m, r, profile, bytecode, codes_now())
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
                reread(cx, acts, &mut ftags, &mut fdefs);
            }
            let missing = cx.missing_arg().sexp();
            // (read after the list's allocation: nothing below collects)
            let codes = codes_now();
            for i in 0..ftags.len() {
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
                inits.push(if i as u16 == m.dots_formal {
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
                            Origin::Expr => {
                                // (a translated call's promise holds its own
                                // code)
                                // SAFETY: the codes are R_NilValue or a
                                // constant of the caller's code, as each
                                // code it holds.
                                let code = unsafe { arg_code(codes, a.pos as usize) };
                                let expr = if code.is_null() {
                                    v
                                } else {
                                    // SAFETY: as above: a live code.
                                    s.root(cx, unsafe { Ref::from_sexp(code, cx) })
                                };
                                SlotInit::Delayed { expr, value: None, bytecode }
                            }
                            // (`...`'s double promise: its code is the promise)
                            Origin::Dot => SlotInit::Delayed { expr: v, value: None, bytecode: false },
                            // (`v` is the generic's frame)
                            Origin::Alias(index) => {
                                aliases += 1;
                                SlotInit::Alias { frame: v, index: index as usize }
                            }
                        }
                    }
                });
            }
            (no_early, site, src, arglist)
        }
    };
    let nf = inits.len();
    // a method's aliases: their promises reserved now, while a refusal
    // is an ordinary error, so that its call's end can make them without
    // allocating (robigo-rtcore's `env::alias`); every init is rooted
    if GENERIC && aliases > 0 {
        cx.reserve_alias_promises(aliases)?;
    }
    // the arguments whose values the call computes early (§6.2,
    // crate::early): every init is rooted, so this may collect
    // (not when every argument is materialized, ROBIGO_MATERIALIZE=1, or
    // byte code's promises are evaluated under a forced `Eval` profile)
    let early = if bytecode && !materialize && forced != Some(Profile::Eval) {
        early_args(cx, s, inits.as_mut_slice(), rho, no_early)?
    } else {
        0
    };
    // SAFETY: the code's shape, a rooted part of the code object.
    let shape = s.root(cx, unsafe { Ref::from_sexp(crate::code::shape(code.bind(cx).sexp()), cx) });
    // a method's frame: the shape with slots for dispatch's variables
    let shape = match supplied {
        Some(sv) => {
            std::hint::cold_path();
            crate::dispatch::method_shape(cx, s, shape, nf, sv)?
        }
        None => shape,
    };
    // SAFETY: CLOENV of the rooted closure.
    let enclos = s.root(cx, unsafe { Ref::from_sexp(op.bind(cx).sexp().tag(), cx) });
    let newrho = s.out(cx);
    let newrho_r = newrho.root();
    cx.new_closure_frame(shape, enclos, rho, inits.as_slice(), newrho)?;
    let newrho = newrho_r;
    // (an outer call's record of dropped early values, given back when
    // this call ends)
    let outer_wasted = (early > 0).then(|| cx.take_early_wasted());
    if early > 0 {
        // SAFETY: the new frame, rooted, with `early` early values.
        unsafe { cx.set_early_frame(newrho.bind(cx).sexp(), early) };
    }
    if let Some(outer) = outer_wasted {
        st.early = Some(EarlyEnd { outer, site, stamp });
    }
    if GENERIC && let (Source::List, Some(l)) = (src, arglist) {
        st.generic_list = Some(l);
    }
    // (`new_closure_frame` marks NO_SPECIAL_SYMBOLS: R_envHasNoSpecialSymbols
    // of a frame binding the formals alone)
    drop(inits);
    // R: eval.c:addMissingVarsToNewEnv (usemethod's variables)
    if let Some(sv) = supplied
        && let Err(e) = crate::dispatch::bind_s3_vars(cx, newrho, sv)
    {
        st.end_early(cx, newrho);
        // (the frame's aliases end with it)
        cx.end_call(newrho, false);
        return Err(e);
    }
    cx.begin_call(newrho);
    if materialize && let Err(e) = (0..nf).try_for_each(|i| cx.materialize_arg(newrho, i)) {
        cx.end_call(newrho, false);
        return Err(e);
    }
    Ok((code, newrho))
}

/// `make_applyClosure_env`'s error for a `rho` that is not an environment.
/// Out of line, as [`apply`]'s other errors: its calls never run them.
// R: eval.c:make_applyClosure_env
#[cold]
#[inline(never)]
fn bad_rho(cx: &mut Cx, call: Root<'_>, rho: Root<'_>) -> RError {
    let msg = format!(
        "'rho' must be an environment not {}: detected in C-level applyClosure",
        crate::eval::type_name(rho.bind(cx).sexptype())
    );
    cx.error_call(Some(call), &msg)
}

/// The error of actuals that could not be collected ([`call_actuals`]).
#[cold]
#[inline(never)]
fn actuals_error(cx: &mut Cx, call: Root<'_>, e: ActualsError) -> RError {
    match e {
        ActualsError::BadDots => cx.error("'...' used in an incorrect context"),
        ActualsError::TooMany => {
            let msg = match_message(cx, MatchError::TooManyActuals, &[]);
            cx.error_call(Some(call), &msg)
        }
    }
}

/// `matchArgs`' error `e` for the call's actuals `acts` and formals'
/// tags `ftags`, after the partial-match warnings of `m` (made so far) under
/// the option: the error, or the warning's.
#[cold]
#[inline(never)]
fn match_error(
    cx: &mut Cx,
    s: Scope<'_>,
    call: Root<'_>,
    e: MatchError,
    acts: &Buf<Actual>,
    m: &Matched,
    ftags: &Buf<Sexp>,
) -> RError {
    // the unused actuals, rooted (a warning's handler may run code before
    // the message is made)
    let unused = match e {
        MatchError::Unused => unused_actuals(cx, s, acts.as_slice(), m),
        _ => Vec::new(),
    };
    if cx.warn_partial_match_args()
        && let Err(w) = partial_warnings(cx, call, m.partial.as_slice(), acts.as_slice(), ftags.as_slice())
    {
        return w;
    }
    let msg = match e {
        MatchError::Unused => match unused_message(cx, s, &unused) {
            Ok(msg) => msg,
            Err(w) => return w,
        },
        _ => match_message(cx, e, acts.as_slice()),
    };
    cx.error_call(Some(call), &msg)
}

/// The end of a call that made early values in frame `f`: early values not
/// taken are dropped; those dropped before the callee took them stop the
/// call site computing them (for this callee, `stamp`), and the outer
/// call's record of dropped values (`outer`) is given back.
#[inline(always)]
fn end_early_call(cx: &mut Cx, f: Sexp, outer: (Sexp, u32), site: Option<(u16, &Act)>, stamp: u64) {
    // SAFETY: (this module's calls) `f` is the ending call's frame, rooted
    // by it, and `outer` the record its start took.
    unsafe { cx.end_early_frame(f) };
    // SAFETY: as above.
    let wasted = unsafe { cx.early_wasted(f) };
    if wasted != 0
        && let Some((site, a)) = site
    {
        // SAFETY: the caller's live code object, rooted in its activation;
        // its plans are plain data, 8-aligned (a RAWSXP's data, at
        // multiples of PLAN_BYTES).
        unsafe {
            if let Some(p) = crate::code::plan_bytes(a.obj(crate::ir::R_CODE), site) {
                let plan = &mut *(p as *mut Plan);
                if plan.stamp == stamp {
                    plan.no_early |= wasted as u16;
                }
            }
        }
    }
    // SAFETY: as above.
    unsafe { cx.restore_early_wasted(outer) };
}

/// The delayed arguments of compiled code (`inits`) whose values the call
/// computes early ([`crate::early::value`]), each bound with its value,
/// except the formals in `skip` (the site's plan: values dropped there
/// before); how many. Inlined in [`prepare`], its one caller, which is
/// out of line.
#[inline(always)]
fn early_args<'s>(cx: &mut Cx, s: Scope<'s>, inits: &mut [SlotInit<'s>], rho: Root<'_>, skip: u16) -> RResult<u32> {
    let mut n = 0;
    for (i, init) in inits.iter_mut().enumerate() {
        if i < 16 && skip & (1 << i) != 0 {
            continue;
        }
        if let SlotInit::Delayed { expr, value: None, bytecode: true } = *init
            && let Some(x) = crate::early::value(cx, s, expr, rho)?
        {
            *init = SlotInit::Delayed { expr, value: Some(x), bytecode: true };
            n += 1;
        }
    }
    Ok(n)
}

/// `R_isReplaceSymbol(fun)`: a symbol whose name ends in `<-`.
///
/// # Safety
/// `f` is live.
// R: eval.c:R_isReplaceSymbol
unsafe fn is_replace_symbol(f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { f.typ() == SYMSXP && f.sym_name_bytes().ends_with(b"<-") }
}

/// The kinds of this module's lazy `promargs` fillers (the tag a record
/// keeps with its filler, [`robigo_rtcore::PromargsFill`]): [`fill_promargs`]
/// over a [`FillData`], and [`generic_promargs`] over a generic's record.
const FILL_ARGS: u32 = 1;
const FILL_GENERIC: u32 = 2;

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
    cx.scope(|cx, s| {
        let rec = cx.context(id).expect("robigo: promargs of a record that is gone");
        let frame = s.root(cx, rec.cloenv());
        let call = s.root(cx, rec.call());
        // each actual's tag and where it went, read before anything collects
        let mut acts: Buf<(Sexp, Went)> = Buf::new();
        // SAFETY: the record's `FillData` and call; nothing collects.
        unsafe {
            d.each_actual(|tag, went| {
                acts.push((tag, went));
                true
            })
        };
        let n = acts.len();
        let vals = s.frame(cx, n);
        let missing = cx.missing_arg().sexp();
        let dots_sym = prims::state(cx).syms.dots;
        for j in 0..n {
            // SAFETY: slot `j` of this scope's frame.
            let slot = unsafe { vals.add(j) };
            let v = match acts[j].1 {
                Went::Missing => missing,
                Went::Dots(k) => {
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
                            _ => {
                                return Err(
                                    cx.error("robigo: 'promargs' after '...' was rebound is not implemented yet")
                                );
                            }
                        }
                    }
                }
                Went::Formal { formal, pos } => {
                    debug_assert!(formal != NO, "robigo: an actual neither matched nor in '...'");
                    // SAFETY: slot `j` is this scope's, empty.
                    let o = unsafe { Out::from_raw(slot) };
                    if cx.arg_promise(frame, formal as usize, o)? {
                        continue;
                    }
                    // a value bound directly: a `Bc` constant, the call's own
                    // argument expression
                    // SAFETY: the rooted call's arguments.
                    unsafe {
                        let mut el = call.bind(cx).sexp().cdr();
                        for _ in 0..pos {
                            el = el.cdr();
                        }
                        el.car()
                    }
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
                    let tag = acts[j].0;
                    let c = cx.alloc_node_raw(LISTSXP, v, l, tag)?;
                    // CONS_NR: the construction's counts undone
                    v.decref();
                    if l != nil {
                        l.decref();
                    }
                    if tag != nil {
                        tag.decref();
                    }
                    c.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
                    l = c;
                }
                Ok(Ref::from_sexp(l, cx))
            }
        })
    })
}

/// What `promargs` would hold for one actual of a generic's call, read
/// from the generic's record and frame ([`generic_args`]).
#[derive(Clone, Copy)]
pub(crate) enum GenericArg {
    /// R_MissingArg, an element of the generic's `...`, or the argument's
    /// promise once made.
    Obj(Sexp),
    /// The argument slot `index` of the generic's frame `frame`, still the
    /// argument's (delayed or forced): the promise the list would hold.
    Slot { frame: Sexp, index: usize },
}

/// The actuals of record `g`'s call as its `promargs` would hold them, in
/// order with their tags, read from the record and its frame without
/// making the list (S3 dispatch's `GetObject` and method call). False, with
/// `out` cleared, when `g`'s `promargs` is not this module's lazy one, or
/// an actual is bound to a slot the generic's body overwrote before its
/// promise was made: then the list is made.
///
/// # Safety
/// Nothing collects while the result is used.
#[inline(never)]
pub(crate) unsafe fn generic_args(cx: &Cx, g: ContextId, out: &mut Buf<(Sexp, GenericArg)>) -> bool {
    out.clear();
    let Some(rec) = cx.context(g) else { return false };
    // (its kind, not the filler's address, says the data is a `FillData`)
    let Some((_, data, FILL_ARGS)) = rec.promargs_filler() else { return false };
    // SAFETY: `data` is the `FillData` of the generic's `apply` frame, live
    // while its record is (`fill_promargs`' contract).
    let d = unsafe { &*(data as *const FillData) };
    let frame = rec.cloenv().sexp();
    let missing = cx.missing_arg().sexp();
    // the generic's `...` list, read when an actual went there
    let mut dots = None;
    let visit = |tag, went| {
        let arg = match went {
            Went::Missing => GenericArg::Obj(missing),
            Went::Dots(k) => {
                let h = *dots.get_or_insert_with(|| {
                    // SAFETY: the record's frame, read without running code.
                    match unsafe { cx.var_lookup_raw(prims::state(cx).syms.dots, frame) } {
                        // SAFETY: a live binding value.
                        RawBinding::Value(h) if unsafe { h.typ() } == DOTSXP => Some(h),
                        _ => None,
                    }
                });
                let Some(mut h) = h else { return false };
                // SAFETY: the k-th cell of the frame's `...` list
                // (`fill_promargs`' walk).
                unsafe {
                    for _ in 0..k {
                        h = h.cdr();
                    }
                    GenericArg::Obj(h.car())
                }
            }
            Went::Formal { formal, .. } => {
                // SAFETY: the record's frame.
                match (formal != NO).then(|| unsafe { cx.arg_slot_raw(frame, formal as usize) }).flatten() {
                    // the promise the list holds (the slot may hold another
                    // value since)
                    Some(ArgSlot::Promise(p)) => GenericArg::Obj(p),
                    Some(_) => GenericArg::Slot { frame, index: formal as usize },
                    None => return false,
                }
            }
        };
        out.push((tag, arg));
        true
    };
    // SAFETY: the record's `FillData` and call; nothing collects.
    let read = unsafe { d.each_actual(visit) };
    if !read {
        out.clear();
    }
    read
}

/// [`apply`]'s source and list for a method called with record `g`'s
/// arguments ([`Source::Generic`]). The method shares the generic's
/// promises, as GNU R's `dispatchMethod` passes `cptr->promargs`: the
/// actuals are read into `acts` from the generic's record and frame
/// ([`generic_actuals`]), with no list made, and each argument slot of the
/// generic the method gets is bound in the method's frame as an alias of
/// that slot ([`Origin::Alias`]): the promise the list would hold, made
/// only if something needs it (a fresh value has the one reference the
/// generic's slot counts, so the method's write to it is made in place,
/// and the generic sees it). When they cannot be read so, the generic's
/// `promargs` is made and passed. Out of line: other calls never run it.
#[inline(never)]
fn generic_source<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    g: ContextId,
    acts: &mut Buf<Actual>,
) -> RResult<(Source, Option<Root<'s>>)> {
    // SAFETY: the actuals are used before anything can collect, or read
    // again after (`apply`'s `reread`).
    if unsafe { generic_actuals(cx, g, acts) } {
        return Ok((Source::Generic(g), None));
    }
    generic_list(cx, s, g)
}

/// [`apply`]'s source and list for a method called with record `g`'s
/// arguments as a list: the generic's `promargs`, made now.
#[inline(never)]
fn generic_list<'s>(cx: &mut Cx, s: Scope<'s>, g: ContextId) -> RResult<(Source, Option<Root<'s>>)> {
    cx.materialize_promargs(g)?;
    let l = cx.context(g).and_then(|r| r.promargs()).expect("robigo: a generic's promargs");
    Ok((Source::List, Some(s.root(cx, l))))
}

/// The actuals of S3 dispatch's method called with record `g`'s arguments
/// ([`Source::Generic`]), from [`generic_args`]: an argument slot still
/// the argument's (delayed or forced) as an [`Origin::Alias`] of it, one
/// whose promise was made as that promise. False, with `acts` cleared,
/// when `generic_args` is, or a slot was overwritten before its promise
/// was made.
///
/// # Safety
/// Nothing collects while the actuals are used.
#[inline(never)]
unsafe fn generic_actuals(cx: &Cx, g: ContextId, acts: &mut Buf<Actual>) -> bool {
    acts.clear();
    let missing = cx.missing_arg().sexp();
    let mut args: Buf<(Sexp, GenericArg)> = Buf::new();
    // SAFETY: the caller's contract.
    if !unsafe { generic_args(cx, g, &mut args) } {
        return false;
    }
    for (j, &(tag, arg)) in args.as_slice().iter().enumerate() {
        let pos = j as u16;
        acts.push(match arg {
            GenericArg::Obj(v) => Actual { val: v, tag, from: Origin::Obj, missing: v == missing, pos },
            // SAFETY: the record's frame.
            GenericArg::Slot { frame, index } => match unsafe { cx.arg_slot_raw(frame, index) } {
                // the generic's argument, shared by an alias
                Some(ArgSlot::Delayed { .. } | ArgSlot::Forced { .. }) => {
                    Actual { val: frame, tag, from: Origin::Alias(index as u16), missing: false, pos }
                }
                Some(ArgSlot::Promise(p)) => Actual { val: p, tag, from: Origin::Obj, missing: false, pos },
                None => {
                    acts.clear();
                    return false;
                }
            },
        });
    }
    true
}

/// The promises of the actuals the `...` formal of a method called with
/// record `g`'s arguments gets that are its generic's argument slots
/// ([`Origin::Alias`]): made now ([`Cx::arg_promise`]), the slots then
/// holding them, so that the list holds the promise the generic binds.
/// `acts` is stale after this (it may collect).
#[cold]
#[inline(never)]
fn dots_promises(cx: &mut Cx, s: Scope<'_>, g: ContextId, acts: &Buf<Actual>, m: &Matched) -> RResult<()> {
    let frame = s.root(cx, cx.context(g).expect("robigo: the generic's record").cloenv());
    let slots: Buf<u16> = {
        let mut b = Buf::new();
        for &bi in m.dots.iter() {
            if let Origin::Alias(i) = acts[bi as usize].from {
                b.push(i);
            }
        }
        b
    };
    for &i in slots.as_slice() {
        let o = s.out(cx);
        cx.arg_promise(frame, i as usize, o)?;
    }
    Ok(())
}

/// The `promargs` of a method called with its generic's arguments
/// ([`Source::Generic`]), made when first read: the generic's own list
/// (made now if it is still lazy), the object GNU R's `dispatchMethod`
/// hands `applyMethod` (`cptr->promargs`). `data` is the generic's record.
// R: objects.c:dispatchMethod
fn generic_promargs(cx: &mut Cx, _id: ContextId, data: usize, out: Out<'_>) -> RResult<()> {
    let g = ContextId::from_ptr(data as *mut robigo_rtcore::Context).expect("robigo: a method's generic record");
    cx.materialize_promargs(g)?;
    let l = cx.context(g).and_then(|r| r.promargs()).expect("robigo: a method whose generic's record is gone");
    out.set(cx, l);
    Ok(())
}

// ---- entry points ----------------------------------------------------------------

/// The VM's closure call: the closure in register `rf` called by `call`
/// (a constant of the running code) from the activation's environment,
/// with call site `site`'s plan; the value into register `dst`.
///
/// # Safety
/// `call` is a live call of `cx`'s isolate (a constant of `act`'s code, or
/// rooted).
pub unsafe fn call_closure(cx: &mut Cx, act: &Act, dst: Reg, rf: Reg, call: Sexp, site: u16) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code.
        let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        let src = Source::Site(site, 0);
        apply(cx, callr, act.root(rf), src, None, act.root(R_ENV), Some(act), None, true, act.out(dst))
    })
}

/// [`call_closure`] of a translated call whose promises hold their own
/// code: the list at pool index `codes_k` of the running code
/// ([`arg_code`]). Its own function, so that [`call_closure`], which the
/// VM's `Call` inlines, stays as it is.
///
/// # Safety
/// As for [`call_closure`]; `codes_k` is a pool index of `act`'s code.
#[inline(never)]
pub unsafe fn call_closure_codes(
    cx: &mut Cx,
    act: &Act,
    dst: Reg,
    rf: Reg,
    call: Sexp,
    codes_k: u16,
    site: u16,
) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code.
        let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        let src = Source::Site(site, codes_k);
        apply(cx, callr, act.root(rf), src, None, act.root(R_ENV), Some(act), None, true, act.out(dst))
    })
}
