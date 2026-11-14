//! cmp.R's constant folding (`constantFoldCall`): the values of the calls
//! of base functions on constants that the `Bc` compiler compiles as
//! constants, each `tryCatch(checkConst(do.call(ffun, args)), error =
//! function(e) NULL, warning = function(w) NULL)`.
//!
//! The compiler runs under `&Cx` and evaluates nothing ([`crate::cenv`]).
//! A call whose value it does not know yet is a request
//! ([`Folds::request`]) and is compiled as not folded for now; when the try
//! ends, [`crate::compile`]'s retry loop evaluates the requests
//! ([`Round::resolve`]) and compiles again with their values in view
//! ([`Folds::lookup`]). Each round knows more values (a call folds once its
//! arguments have), so a compile ends after as many rounds as its folds
//! nest, and the last round's code is the code cmp.R makes.
//!
//! A fold is evaluated by a closure of base's namespace that establishes
//! exiting handlers for errors and warnings (`.addCondHands`, as
//! `tryCatch` does) and calls `.Internal(do.call(f, args, env))`: only
//! internals and primitives, so that it works while base's own closures
//! (`tryCatch`, `do.call`, `environment`) are lazy bindings not yet
//! forced, being compiled, or absent. It is never JIT-compiled itself
//! (`NOJIT`).

#![allow(unsafe_code)]

use crate::prims;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, Global, RResult, Ref};
use std::cell::{Cell, RefCell};

/// The source of the closure that evaluates one fold, given the handlers'
/// classes `cl` (`c("error", "warning")`) and handlers `h` (unused, as an
/// exiting handler's result is returned): `list(value)`, or (a handler
/// took a condition) `.addCondHands`'s result vector of four. Its frame
/// comes from `.Internal(environment(NULL))` in a closure it calls
/// (base's `environment` may not be bound: an isolate without base's R
/// code). Its body holds no call that folds, so compiling it (the `Bc`
/// profile forced) asks for no fold.
const HELPER: &[u8] = b"function(f, a, cl, h) {
    env <- (function() .Internal(environment(NULL)))()
    .Internal(.addCondHands(cl, h, env, env, FALSE))
    list(.Internal(do.call(f, a, env)))
}";

/// A call the compiler would fold: the base function and the folded
/// arguments with their tags (R_NilValue for none).
#[derive(Clone, Debug)]
pub struct FoldReq {
    pub call: Sexp,
    pub fun: Sexp,
    pub args: Vec<(Sexp, Sexp)>,
}

/// The isolate's folding state ([`prims::RtState::folds`]).
#[derive(Default)]
pub struct Folds {
    /// The values the compile try under way knows: each call's, None
    /// where it does not fold. Set by [`Round::begin`].
    view: RefCell<Vec<(Sexp, Option<Sexp>)>>,
    /// The calls the try met whose values are not known yet.
    pending: RefCell<Vec<FoldReq>>,
    /// The closure that evaluates a fold ([`HELPER`]).
    helper: RefCell<Option<Global>>,
    /// Folds being evaluated (nested compiles).
    busy: Cell<u32>,
}

impl Folds {
    /// The fold of `call` in the try under way: `Some(Some(value))`,
    /// `Some(None)` when it does not fold, None when not known yet.
    pub fn lookup(&self, call: Sexp) -> Option<Option<Sexp>> {
        self.view.borrow().iter().find(|e| e.0 == call).map(|e| e.1)
    }

    /// Whether a compile may ask for folds now. Under a forced profile
    /// (`ROBIGO_RT_PROFILE`, tests) `eval` compiles too, so the call
    /// `do.call` makes while a fold is evaluated is compiled, and folding it
    /// would evaluate it again without end: such compiles fold nothing
    /// (GNU R evaluates that call as the AST).
    pub fn may_request(&self, forced: bool) -> bool {
        !forced || self.busy.get() == 0
    }

    /// How many requests the try under way has made.
    pub fn requests(&self) -> usize {
        self.pending.borrow().len()
    }

    /// Ask for the value of `r.call` before the next try.
    pub fn request(&self, r: FoldReq) {
        let mut p = self.pending.borrow_mut();
        if !p.iter().any(|q| q.call == r.call) {
            p.push(r);
        }
    }

    /// The helper closure, copied into a new state ([`crate::lib`]'s growth).
    pub fn copy(&self, cx: &Cx) -> Folds {
        let helper = self.helper.borrow().as_ref().map(|g| cx.global(g.bind(cx)));
        Folds { helper: RefCell::new(helper), ..Default::default() }
    }
}

/// The folds of one compile (its retry loop): the values evaluated so
/// far, rooted until [`Round::release`].
#[derive(Default)]
pub struct Round {
    known: Vec<(Global, Option<Global>)>,
}

impl Round {
    /// Put the values known into view for a try; no request yet.
    pub fn begin(&self, cx: &Cx) {
        let f = &prims::state(cx).folds;
        *f.view.borrow_mut() =
            self.known.iter().map(|(c, v)| (c.bind(cx).sexp(), v.as_ref().map(|v| v.bind(cx).sexp()))).collect();
        f.pending.borrow_mut().clear();
    }

    /// The end of a try: the view gone (a nested compile sets its own), the
    /// requests it made. Their objects are live until the next allocation
    /// that may collect.
    pub fn end(cx: &Cx) -> Vec<FoldReq> {
        let f = &prims::state(cx).folds;
        f.view.borrow_mut().clear();
        std::mem::take(&mut *f.pending.borrow_mut())
    }

    /// Evaluate `reqs`, made by the try that just ended (nothing allocated
    /// since): each call's value from now on, or that it does not fold.
    /// `R_Visible` is kept. An exit no handler of the fold takes (a
    /// restart, an interrupt) propagates.
    pub fn resolve(&mut self, cx: &mut Cx, reqs: Vec<FoldReq>) -> RResult<()> {
        // SAFETY: the requests' objects are live: the try that found them
        // (in the source, a cached value) just ended and nothing has
        // allocated since. Rooting them allocates nothing.
        let rooted: Vec<(Global, Global, Vec<(Global, Global)>)> = reqs
            .iter()
            .map(|r| unsafe {
                let g = |x: Sexp| cx.global(Ref::from_sexp(x, cx));
                (g(r.call), g(r.fun), r.args.iter().map(|&(v, t)| (g(v), g(t))).collect())
            })
            .collect();
        let vis = cx.visible();
        let busy = &prims::state(cx).folds.busy;
        busy.set(busy.get() + 1);
        let mut res = Ok(());
        let mut it = rooted.into_iter();
        for (call, fun, args) in it.by_ref() {
            let v = fold1(cx, &fun, &args);
            cx.release_global(fun);
            for (v, t) in args {
                cx.release_global(v);
                cx.release_global(t);
            }
            match v {
                Ok(v) => self.known.push((call, v)),
                Err(e) => {
                    cx.release_global(call);
                    res = Err(e);
                    break;
                }
            }
        }
        for (call, fun, args) in it {
            cx.release_global(call);
            cx.release_global(fun);
            for (v, t) in args {
                cx.release_global(v);
                cx.release_global(t);
            }
        }
        let busy = &prims::state(cx).folds.busy;
        busy.set(busy.get() - 1);
        cx.set_visible(vis);
        res
    }

    /// Unroot the values (the compile is over: its code holds the ones it
    /// uses).
    pub fn release(self, cx: &mut Cx) {
        for (c, v) in self.known {
            cx.release_global(c);
            if let Some(v) = v {
                cx.release_global(v);
            }
        }
    }
}

/// `tryCatch(checkConst(do.call(fun, args)), error =, warning =)`: the
/// value, rooted, or None.
// R: cmp.R:constantFoldCall, checkConst
fn fold1(cx: &mut Cx, fun: &Global, args: &[(Global, Global)]) -> RResult<Option<Global>> {
    make_helper(cx)?;
    cx.scope(|cx, s| {
        let helper = {
            let h = prims::state(cx).folds.helper.borrow();
            s.root(cx, h.as_ref().expect("robigo: the fold helper").bind(cx))
        };
        let o = s.out(cx);
        let call = o.root();
        cx.build(o, |cx| {
            let nil = cx.nil().sexp();
            let n = args.len();
            // as.list(e[-1]): the tags are the names, if there is one
            let l = cx.alloc_vector_raw(VECSXP, n)?;
            let names = if args.iter().any(|a| a.1.bind(cx).sexp() != nil) {
                let nm = cx.alloc_vector_raw(STRSXP, n)?;
                let at = cx.cons_tagged(
                    // SAFETY: a fresh vector.
                    unsafe { Ref::from_sexp(nm, cx) },
                    cx.nil(),
                    cx.install(b"names")?,
                )?;
                Some((nm, at.sexp()))
            } else {
                None
            };
            let cl = cx.alloc_vector_raw(STRSXP, 2)?;
            let err = cx.mk_char(b"error", robigo_rtcore::Encoding::Native)?.as_ref().sexp();
            // SAFETY: a fresh vector of two; a live CHARSXP.
            unsafe { cl.set_elt(0, err) };
            let warn = cx.mk_char(b"warning", robigo_rtcore::Encoding::Native)?.as_ref().sexp();
            // SAFETY: as above.
            unsafe { cl.set_elt(1, warn) };
            let h = cx.alloc_vector_raw(VECSXP, 2)?;
            // SAFETY: fresh vectors.
            let tail = unsafe {
                let t = cx.cons(Ref::from_sexp(h, cx), cx.nil())?;
                let t = cx.cons(Ref::from_sexp(cl, cx), t)?;
                cx.cons(Ref::from_sexp(l, cx), t)?
            };
            let tail = cx.cons(fun.bind(cx), tail)?;
            let c = cx.lcons(helper.bind(cx), tail)?;
            // SAFETY: fresh vectors of length `n`; the values are rooted
            // objects, the tags symbols (their print names CHARSXPs).
            unsafe {
                for (i, (v, t)) in args.iter().enumerate() {
                    l.set_elt(i, v.bind(cx).sexp());
                    if let Some((nm, _)) = names {
                        let t = t.bind(cx).sexp();
                        if t != nil {
                            nm.set_elt(i, t.car());
                        }
                    }
                }
                if let Some((_, at)) = names {
                    l.set_attrib(at);
                }
            }
            Ok(c)
        })?;
        let env = s.root(cx, cx.base_namespace());
        let o = s.out(cx);
        let r = o.root();
        crate::eval(cx, call, env, o)?;
        let r = r.bind(cx);
        let Some(v) = r.as_vector::<Ref<'_>>().filter(|v| v.len() == 1) else {
            // an exiting handler's result: a condition
            return Ok(None);
        };
        let v = v.get(0);
        // SAFETY: a live value.
        Ok(unsafe { crate::cenv::check_const(cx, v.sexp()) }.then(|| cx.global(v)))
    })
}

/// Make the helper closure on first use: [`HELPER`] evaluated in base's
/// namespace, marked `NOJIT`.
fn make_helper(cx: &mut Cx) -> RResult<()> {
    if prims::state(cx).folds.helper.borrow().is_some() {
        return Ok(());
    }
    let g = cx.scope(|cx, s| -> RResult<Global> {
        let o = s.out(cx);
        let exprs = o.root();
        let (status, _) = crate::reader::parse_bytes(cx, HELPER, 1, o)?;
        assert!(matches!(status, crate::reader::ParseStatus::Ok), "robigo: the fold helper does not parse");
        let e = s.root(cx, exprs.bind(cx).as_vector::<Ref<'_>>().expect("robigo: parse's expressions").get(0));
        let env = s.root(cx, cx.base_namespace());
        let o = s.out(cx);
        let f = o.root();
        crate::eval(cx, e, env, o)?;
        let f = f.bind(cx);
        // SAFETY: a fresh closure; gp is a header field.
        unsafe { f.sexp().set_gp(f.sexp().gp() | crate::jit::NOJIT_MASK) };
        Ok(cx.global(f))
    })?;
    *prims::state(cx).folds.helper.borrow_mut() = Some(g);
    Ok(())
}
