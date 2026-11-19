//! The specials that are functions over their call (GNU R's `do_*` for a
//! SPECIALSXP: the arguments are the call's expressions, evaluated by the
//! special): subsetting and its setters (subset.c, subassign.c: dispatch on
//! an object, then the default method of `robigo-rtbuiltins`), `switch`,
//! `on.exit` (builtin.c) and `substitute` (coerce.c).
//!
//! A compiled call runs one of these behind a guard ([`crate::ir::Op::SpecialFn`]);
//! a call built at run time (complex assignment's) or one reaching the
//! special through a value runs it the same way ([`crate::call`]).

#![allow(unsafe_code)]

use crate::eval::eval;
use crate::links;
use crate::prims;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtbuiltins::{subassign, subset};
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, Scope, Slot, Sym, View};
use std::ptr::NonNull;

/// The number of arguments of `call`.
fn nargs(cx: &Cx, call: Root<'_>) -> usize {
    call.bind(cx).len().unwrap_or(1).saturating_sub(1)
}

/// `checkArity(op, args)` of a primitive special, its arguments the
/// call's (`crate::prims::check_arity`).
fn check_arity(cx: &mut Cx, call: Root<'_>, name: &str, arity: usize) -> RResult<()> {
    let n = nargs(cx, call);
    crate::prims::check_arity(cx, call, n, name, arity, false)
}

/// `eval(e, rho)` into a new root of scope `s`.
fn eval_root<'s>(cx: &mut Cx, s: Scope<'s>, e: Root<'_>, rho: Root<'_>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    eval(cx, e, rho, o)?;
    Ok(r)
}

/// Element `i` of the call's arguments (the `i`-th cell after the head).
fn arg<'g>(cx: &'g Cx, call: Root<'_>, i: usize) -> Option<(Ref<'g>, Option<Sym<'g>>)> {
    let mut c = call.bind(cx).as_cons()?.cdr().as_cons();
    for _ in 0..i {
        c = c?.cdr().as_cons();
    }
    c.map(|c| (c.car(), c.tag().as_sym()))
}

/// A copy of the cells of the pairlist `args` (their tags kept; GNU R's
/// `shallow_duplicate`), the first one's value `first`.
fn tagged_rest<'s>(cx: &mut Cx, s: Scope<'s>, args: Root<'_>, first: Root<'_>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.shallow_duplicate(args, o)?;
    cx.set_car(r, first);
    Ok(r)
}

/// Element `i` of the pairlist `args` (R_NilValue past its end).
fn list_elt<'g>(cx: &'g Cx, args: Root<'_>, i: usize) -> Ref<'g> {
    let mut c = args.bind(cx).as_cons();
    for _ in 0..i {
        c = c.and_then(|c| c.cdr().as_cons());
    }
    c.map_or(cx.nil(), |c| c.car())
}

/// The evaluated arguments: a window of value slots and tag slots of the
/// caller's scope, filled in order (GNU R's `CONS_NR` list).
pub(crate) struct Evaluated<'s> {
    vals: NonNull<Slot>,
    names: NonNull<Slot>,
    n: usize,
    cap: usize,
    _s: std::marker::PhantomData<Scope<'s>>,
}

impl<'s> Evaluated<'s> {
    /// Room for `cap` arguments in scope `s`.
    pub(crate) fn new(cx: &Cx, s: Scope<'s>, cap: usize) -> Evaluated<'s> {
        Evaluated { vals: s.frame(cx, cap), names: s.frame(cx, cap), n: 0, cap, _s: std::marker::PhantomData }
    }

    /// The next value's slot, its tag written.
    pub(crate) fn next(&mut self, tag: Sexp) -> Out<'s> {
        assert!(self.n < self.cap, "robigo: more arguments than counted");
        // SAFETY: slots of the scope's frames, below `cap`; a tag is a
        // symbol or R_NilValue (immortal).
        unsafe {
            self.names.add(self.n).write(Slot::object(tag));
            let o = Out::from_raw(self.vals.add(self.n));
            self.n += 1;
            o
        }
    }

    /// A guard for the links of the values written from position `start`
    /// on (each linked as it is written).
    fn linked_from(&self, start: usize) -> links::Linked {
        assert!(start <= self.cap);
        // SAFETY: the window's slots belong to the caller's scope, which
        // outlives the guard; each slot linked holds the value written.
        unsafe { links::Linked::slots(self.vals.add(start)) }
    }

    /// Value `i`.
    pub(crate) fn len(&self) -> usize {
        self.n
    }

    pub(crate) fn root(&self, i: usize) -> Root<'s> {
        assert!(i < self.n);
        // SAFETY: a filled slot of the window.
        unsafe { Root::from_raw(self.vals.add(i)) }
    }

    /// The window for a builtin's default method.
    pub(crate) fn args(&self, cx: &Cx) -> Args<'s> {
        let any_alt = (0..self.n).any(|i| self.root(i).bind(cx).is_altrep());
        // SAFETY: `n` filled value slots and `n` tag slots of the scope.
        unsafe { Args::from_raw(self.vals, Some(self.names), self.n as u32, any_alt) }
    }
}

/// How many values `evalListKeepMissing` makes of the call's arguments
/// from `first` on: `...` counts its elements (anything else it is bound
/// to counts none: the evaluation raises its error).
fn count_args(cx: &mut Cx, call: Root<'_>, first: usize, rho: Root<'_>) -> RResult<usize> {
    let dots = prims::state(cx).syms.dots;
    let (n, has_dots) = {
        let mut c = call.bind(cx).as_cons().and_then(|c| c.cdr().as_cons());
        let (mut n, mut has) = (0, false);
        let mut i = 0;
        while let Some(cell) = c {
            if i >= first {
                if cell.car().sexp() == dots {
                    has = true;
                } else {
                    n += 1;
                }
            }
            i += 1;
            c = cell.cdr().as_cons();
        }
        (n, has)
    };
    if !has_dots {
        return Ok(n);
    }
    let per = cx.scope(|cx, s| {
        // SAFETY: an immortal symbol.
        let d = s.root(cx, unsafe { Ref::from_sexp(dots, cx) });
        let h = s.out(cx);
        let hr = h.root();
        cx.find_var(d, rho, h)?;
        let v = hr.bind(cx);
        Ok(if v.sexptype() == DOTSXP { v.len().unwrap_or(0) } else { 0 })
    })?;
    let mut total = n;
    let mut c = call.bind(cx).as_cons().and_then(|c| c.cdr().as_cons());
    let mut i = 0;
    while let Some(cell) = c {
        if i >= first && cell.car().sexp() == dots {
            total += per;
        }
        i += 1;
        c = cell.cdr().as_cons();
    }
    Ok(total)
}

/// `evalListKeepMissing(el, rho)`: each argument of the call's cells from
/// `first` on evaluated in order into the window, `...` expanded; an empty
/// argument, or a symbol that is a missing argument (`R_isMissing`), is
/// `R_MissingArg`. Each value holds a link until all are evaluated, as
/// GNU R's do.
// R: eval.c:evalListKeepMissing
fn eval_list_keep_missing<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    call: Root<'_>,
    first: usize,
    rho: Root<'_>,
    ev: &mut Evaluated<'s>,
    pp: i32,
) -> RResult<()> {
    let start = ev.n;
    // the values' links, dropped once all are evaluated, or on an early exit
    let mut lk = ev.linked_from(start);
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    // the cursor over the call's argument cells
    let cur = {
        let mut c = call.bind(cx).as_cons().unwrap().cdr();
        for _ in 0..first {
            c = c.as_cons().map_or(cx.nil(), |cell| cell.cdr());
        }
        s.root(cx, c)
    };
    while let Some(cell) = cur.bind(cx).as_cons() {
        let (es, tag) = (cell.car().sexp(), cell.tag().sexp());
        if es == dots {
            cx.scope(|cx, s2| {
                // SAFETY: an immortal symbol.
                let d = s2.root(cx, unsafe { Ref::from_sexp(dots, cx) });
                let h = s2.out(cx);
                let hr = h.root();
                cx.find_var(d, rho, h)?;
                let t = hr.bind(cx).sexptype();
                if !(t == DOTSXP || t == NILSXP) {
                    if hr.bind(cx).sexp() != missing {
                        return Err(cx.error("'...' used in an incorrect context"));
                    }
                    return Ok(());
                }
                while let Some(c) = hr.bind(cx).as_cons() {
                    let (car, tag) = (c.car(), c.tag().sexp());
                    // under PROTECT(h), and the head's after the first value
                    // (robigo_rtcore's ppacct; `pp` the caller's)
                    let k = pp + 1 + (ev.n > start) as i32;
                    let o = ev.next(tag);
                    if car.sexp() == missing {
                        o.set(cx, car);
                    } else {
                        let er = s2.root(cx, car);
                        cx.with_pp_excess(k, |cx| eval(cx, er, rho, o))?;
                    }
                    // SAFETY: the slot just written holds the value.
                    unsafe { lk.link_next() };
                    let next = s2.root(cx, hr.bind(cx).as_cons().unwrap().cdr());
                    hr.replace_from(cx, next);
                }
                Ok(())
            })?;
        } else {
            // SAFETY: a symbol of the rooted call, and the rooted environment.
            let is_missing = es == missing
                || (unsafe { es.typ() } == SYMSXP
                    && unsafe { crate::missing::r_is_missing(cx, es, rho.bind(cx).sexp()) }?);
            let k = pp + (ev.n > start) as i32;
            let o = ev.next(tag);
            if is_missing {
                o.set(cx, cx.missing_arg());
            } else {
                let e = s.root(cx, cur.bind(cx).as_cons().unwrap().car());
                cx.with_pp_excess(k, |cx| eval(cx, e, rho, o))?;
            }
            // SAFETY: the slot just written holds the value.
            unsafe { lk.link_next() };
        }
        let next = s.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
        cur.replace_from(cx, next);
    }
    lk.release();
    Ok(())
}

/// `log(x, base)`: its arguments evaluated, missing ones kept, then
/// `do_log_builtin` (robigo_rtbuiltins::math), which matches and
/// dispatches. An argument that is one of robigo's compact sequences is
/// expanded first, as a builtin's are ([`crate::call::run_builtin`]):
/// GNU R's `math1`/`math2` read it through `REAL()`.
// R: arithmetic.c:do_log, memory.c:DATAPTR
pub fn do_log(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let cap = count_args(cx, call, 0, rho)?;
        let mut ev = Evaluated::new(cx, s, cap);
        eval_list_keep_missing(cx, s, call, 0, rho, &mut ev, 0)?;
        let args = ev.args(cx);
        if args.any_altrep() {
            cx.materialize_args(args)?;
        }
        robigo_rtbuiltins::math::do_log_builtin(cx, call, args, rho, out)
    })
}

/// The isolate's `eval_args` hook: `DispatchOrEval(call, op, generic,
/// args, rho, &ans, 0, 0)`'s evaluation of a special's arguments for its
/// default method (a builtin special such as `rep`). The object is the
/// first argument, or `...`'s first element (an empty `...` skipped), or
/// R_NilValue when there is none. The first argument is evaluated and holds
/// a link while `evalListKeepMissing` evaluates the rest (each linked until
/// all are), and the links drop before the special goes on; after `...`'s
/// first element, `evalListKeepMissing` of them all from `...` on; with no
/// object, the window holds R_NilValue alone. (The object case, the method
/// called with the arguments as promises, is the caller's: it dispatches on
/// the values.)
// R: eval.c:DispatchOrEval
pub(crate) fn eval_args_hook<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, rho: Root<'_>) -> RResult<Args<'s>> {
    let dots = prims::state(cx).syms.dots;
    let missing = cx.missing_arg().sexp();
    let nil = cx.nil().sexp();
    let mut k = 0;
    loop {
        let cell = {
            let mut c = call.bind(cx).as_cons().unwrap().cdr();
            for _ in 0..k {
                c = c.as_cons().unwrap().cdr();
            }
            c.as_cons().map(|c| (c.car().sexp(), c.tag().sexp()))
        };
        match cell {
            None => {
                // CONS_NR(R_NilValue, evalListKeepMissing(CDR(R_NilValue)))
                let mut ev = Evaluated::new(cx, s, 1);
                let o = ev.next(nil);
                o.set(cx, cx.nil());
                return Ok(ev.args(cx));
            }
            Some((e, _)) if e == dots => {
                let found = cx.scope(|cx, s2| {
                    // SAFETY: an immortal symbol.
                    let d = s2.root(cx, unsafe { Ref::from_sexp(dots, cx) });
                    let h = s2.out(cx);
                    let hr = h.root();
                    cx.find_var(d, rho, h)?;
                    let hv = hr.bind(cx);
                    if let Some(c) = hv.as_cons().filter(|_| hv.sexptype() == DOTSXP) {
                        // x = eval(CAR(h), rho): the first element forced
                        let first = s2.root(cx, c.car());
                        let o = s2.out(cx);
                        eval(cx, first, rho, o)?;
                        return Ok(true);
                    }
                    if hv.sexp() != nil && hv.sexp() != missing {
                        return Err(cx.error("'...' used in an incorrect context"));
                    }
                    Ok(false)
                })?;
                if found {
                    let cap = count_args(cx, call, k, rho)?;
                    let mut ev = Evaluated::new(cx, s, cap);
                    eval_list_keep_missing(cx, s, call, k, rho, &mut ev, 0)?;
                    return Ok(ev.args(cx));
                }
                k += 1;
            }
            Some((_, tag)) => {
                let cap = 1 + count_args(cx, call, k + 1, rho)?;
                let mut ev = Evaluated::new(cx, s, cap);
                let e = {
                    let mut c = call.bind(cx).as_cons().unwrap().cdr();
                    for _ in 0..k {
                        c = c.as_cons().unwrap().cdr();
                    }
                    s.root(cx, c.as_cons().unwrap().car())
                };
                let mut lx = ev.linked_from(0);
                // SET_TAG(*ans, CreateTag(TAG(args)))
                let o = ev.next(tag);
                eval(cx, e, rho, o)?;
                // x holds a link while the rest is evaluated
                // SAFETY: the slot just written holds x.
                unsafe { lx.link_next() };
                // under PROTECT(x)
                eval_list_keep_missing(cx, s, call, k + 1, rho, &mut ev, 1)?;
                lx.release();
                return Ok(ev.args(cx));
            }
        }
    }
}

/// What `R_DispatchOrEvalSP` did.
enum Disp<'s> {
    /// A method ran: its value is in the special's `out`.
    Done,
    /// No method: the arguments, evaluated.
    Default(Evaluated<'s>),
}

/// `R_DispatchOrEvalSP(call, op, generic, args, rho, &ans)`: evaluate the
/// first argument; unless it is an object, the others as
/// `evalListKeepMissing` does (`x` holds a link meanwhile); an object goes
/// to S3 dispatch (S1).
// R: subset.c:R_DispatchOrEvalSP
fn dispatch_or_eval_sp<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    call: Root<'_>,
    generic: &'static str,
    rho: Root<'_>,
    out: &Out<'_>,
) -> RResult<Disp<'s>> {
    let dots = prims::state(cx).syms.dots;
    let first = call.bind(cx).as_cons().unwrap().cdr().as_cons().map(|c| c.car().sexp());
    match first {
        Some(f) if f != dots => {
            let cap = 1 + count_args(cx, call, 1, rho)?;
            let mut ev = Evaluated::new(cx, s, cap);
            let e = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().car());
            let nil = cx.nil().sexp();
            let mut lx = ev.linked_from(0);
            let o = ev.next(nil);
            let x = o.root();
            eval(cx, e, rho, o)?;
            // x holds a link while the rest is evaluated (or dispatched)
            // SAFETY: the slot just written holds x.
            unsafe { lx.link_next() };
            if !x.bind(cx).is_object() {
                // under PROTECT(x)
                eval_list_keep_missing(cx, s, call, 1, rho, &mut ev, 1)?;
                lx.release();
                return Ok(Disp::Default(ev));
            }
            let prom = ev_promise(cx, s, e, x)?;
            let rest = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr());
            let r = crate::dispatch::dispatch_special(cx, generic, call, prom, rest, rho, out);
            lx.release();
            if r? {
                return Ok(Disp::Done);
            }
            // DispatchOrEval's evalArgs, under R_DispatchOrEvalSP's
            // PROTECT(args) and its own PROTECT(x), PROTECT(pargs),
            // PROTECT(rho1)
            eval_list_keep_missing(cx, s, call, 1, rho, &mut ev, 4)?;
            Ok(Disp::Default(ev))
        }
        Some(_) => {
            // `...` first: DispatchOrEval finds the object among its
            // elements; with no object it evaluates the whole list
            let cap = count_args(cx, call, 0, rho)?;
            let mut ev = Evaluated::new(cx, s, cap);
            // DispatchOrEval's evalArgs, under R_DispatchOrEvalSP's
            // PROTECT(args) and its own PROTECT(x)
            eval_list_keep_missing(cx, s, call, 0, rho, &mut ev, 2)?;
            if ev.n > 0 && ev.root(0).bind(cx).is_object() {
                let msg = format!("robigo: S3 dispatch of '{generic}' through '...' is not implemented yet");
                return Err(cx.error(&msg));
            }
            Ok(Disp::Default(ev))
        }
        None => Ok(Disp::Default(Evaluated::new(cx, s, 0))),
    }
}

/// `R_mkEVPROMISE_NR(expr, value)`: an evaluated promise that counts no
/// references.
pub(crate) fn ev_promise<'s>(cx: &mut Cx, s: Scope<'s>, expr: Root<'_>, value: Root<'_>) -> RResult<Root<'s>> {
    let p = cx.retry_alloc(|cx| {
        let unbound = cx.unbound().sexp();
        let nil = cx.nil().sexp();
        // SAFETY: rooted objects; nothing collects in this interval.
        unsafe {
            let p = cx.alloc_node_raw(PROMSXP, unbound, expr.bind(cx).sexp(), nil)?;
            p.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
            p.set_car(value.bind(cx).sexp());
            Ok(p)
        }
    })?;
    // SAFETY: fresh, of this isolate.
    Ok(s.root(cx, unsafe { Ref::from_sexp(p, cx) }))
}

/// `do_subset`: `x[...]`.
// R: subset.c:do_subset
pub fn do_subset(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| match dispatch_or_eval_sp(cx, s, call, "[", rho, &out)? {
        Disp::Done => Ok(()),
        Disp::Default(ev) => {
            let args = ev.args(cx);
            cx.materialize_subset_args(args)?;
            subset::do_subset_dflt(cx, call, args, rho, out)
        }
    })
}

/// `do_subset2`: `x[[...]]`.
// R: subset.c:do_subset2
pub fn do_subset2(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| match dispatch_or_eval_sp(cx, s, call, "[[", rho, &out)? {
        Disp::Done => Ok(()),
        Disp::Default(ev) => {
            let args = ev.args(cx);
            cx.materialize_subset_args(args)?;
            subset::do_subset2_dflt(cx, call, args, rho, out)
        }
    })
}

/// `fixSubset3Args`: `$`'s name as a string (a symbol's name, or a
/// one-element character vector; a promise is forced first), and the
/// symbol when the argument was one.
// R: subset.c:fixSubset3Args
fn subset3_name<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, rho: Root<'_>) -> RResult<(Root<'s>, Option<Root<'s>>)> {
    let mut nl = s.root(cx, arg(cx, call, 1).map_or(cx.nil(), |(e, _)| e));
    if nl.bind(cx).sexptype() == PROMSXP {
        nl = eval_root(cx, s, nl, rho)?;
    }
    let v = nl.bind(cx);
    match v.sexptype() {
        SYMSXP => {
            let name = v.as_sym().unwrap().name(cx);
            Ok((s.root(cx, name.as_ref()), Some(nl)))
        }
        STRSXP => {
            let sv = v.as_vector::<Char<'_>>().unwrap();
            if sv.len() != 1 {
                return Err(cx.error("invalid subscript length"));
            }
            let c = sv.get(0);
            Ok((s.root(cx, c.as_ref()), None))
        }
        _ => {
            let msg = format!("invalid subscript type '{}'", robigo_rtbuiltins::util::type_to_char(v));
            Err(cx.error_call(Some(call), &msg))
        }
    }
}

/// A one-element character vector of the CHARSXP in `c`.
fn scalar_string<'s>(cx: &mut Cx, s: Scope<'s>, c: Root<'_>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, c.bind(cx).as_char().unwrap());
        Ok(v.finish().into())
    })?;
    Ok(r)
}

/// A guard for the link of the object in root `r` (a slot of the caller's
/// scope), raised now.
fn link_root(r: Root<'_>) -> links::Linked {
    let p = NonNull::new(r.as_ptr() as *mut Slot).unwrap();
    // SAFETY: a slot of the caller's scope, which outlives the guard, and
    // it holds the evaluated object.
    unsafe {
        let mut l = links::Linked::slots(p);
        l.link_next();
        l
    }
}

/// `do_subset3`: `x$name`. The name is matched, not evaluated; the first
/// argument is evaluated (and dispatched on), then the name as a string
/// constant, as `R_DispatchOrEvalSP` evaluates the fixed arguments.
// R: subset.c:do_subset3
pub fn do_subset3(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, "$", 2)?;
    cx.scope(|cx, s| {
        let (input, _) = subset3_name(cx, s, call, rho)?;
        let e = s.root(cx, arg(cx, call, 0).unwrap().0);
        if e.bind(cx).sexp() == prims::state(cx).syms.dots {
            return Err(cx.error("robigo: '$' with '...' as its object is not implemented yet"));
        }
        let x = eval_root(cx, s, e, rho)?;
        // x holds a link while the rest is evaluated (or dispatched)
        let lx = link_root(x);
        if x.bind(cx).is_object() {
            let prom = ev_promise(cx, s, e, x)?;
            // fixSubset3Args: the name as a string is the second argument
            let str = scalar_string(cx, s, input)?;
            let o = s.out(cx);
            let rest = o.root();
            cx.build(o, |cx| cx.cons(str.bind(cx), cx.nil()))?;
            let r = crate::dispatch::dispatch_special(cx, "$", call, prom, rest, rho, &out);
            lx.release();
            if r? {
                return Ok(());
            }
        } else {
            // the name, a string constant, is evaluated as an argument
            let str = scalar_string(cx, s, input)?;
            let _ = eval_root(cx, s, str, rho)?;
            lx.release();
        }
        subset::subset3_dflt(cx, x, input, call, out)
    })
}

/// `do_subassign`: `` `[<-`(x, ..., value) ``.
// R: subassign.c:do_subassign
pub fn do_subassign(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| match dispatch_or_eval_sp(cx, s, call, "[<-", rho, &out)? {
        Disp::Done => Ok(()),
        Disp::Default(ev) => {
            let args = ev.args(cx);
            cx.materialize_subset_args(args)?;
            subassign::do_subassign_dflt(cx, call, args, rho, out)
        }
    })
}

/// `do_subassign2`: `` `[[<-`(x, ..., value) ``.
// R: subassign.c:do_subassign2
pub fn do_subassign2(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| match dispatch_or_eval_sp(cx, s, call, "[[<-", rho, &out)? {
        Disp::Done => Ok(()),
        Disp::Default(ev) => {
            let args = ev.args(cx);
            cx.materialize_subset_args(args)?;
            subassign::do_subassign2_dflt(cx, call, args, rho, out)
        }
    })
}

/// `do_subassign3`: `` `$<-`(x, name, value) ``.
// R: subassign.c:do_subassign3
pub fn do_subassign3(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, "$<-", 3)?;
    cx.scope(|cx, s| {
        let (input, sym) = subset3_name(cx, s, call, rho)?;
        let e = s.root(cx, arg(cx, call, 0).unwrap().0);
        if e.bind(cx).sexp() == prims::state(cx).syms.dots {
            return Err(cx.error("robigo: '$<-' with '...' as its object is not implemented yet"));
        }
        let x = eval_root(cx, s, e, rho)?;
        // x holds a link while the rest is evaluated (or dispatched)
        let lx = link_root(x);
        if x.bind(cx).is_object() {
            let prom = ev_promise(cx, s, e, x)?;
            // fixSubset3Args: a shallow copy of the call's cells after the
            // object, the name made a string; the tags (`value =`) kept
            let str = scalar_string(cx, s, input)?;
            let after = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr());
            let o = s.out(cx);
            let rest = o.root();
            cx.shallow_duplicate(after, o)?;
            cx.set_car(rest, str);
            match crate::dispatch::dispatch_special(cx, "$<-", call, prom, rest, rho, &out) {
                // no method: the default below, with the arguments
                // evaluated (x still holds its link)
                Ok(false) => {}
                r => {
                    lx.release();
                    r?;
                    // SAFETY: raised above.
                    unsafe { links::decrement_links(x.bind(cx).sexp()) };
                    return Ok(());
                }
            }
        }
        // the name (a constant) and the value are evaluated as arguments
        let str = scalar_string(cx, s, input)?;
        let _ = eval_root(cx, s, str, rho)?;
        let ve = s.root(cx, arg(cx, call, 2).unwrap().0);
        // evalListKeepMissing: an empty argument, or a missing argument's
        // symbol, is R_MissingArg
        let vs = ve.bind(cx).sexp();
        // SAFETY: a part of the rooted call and the rooted environment.
        let is_missing = vs == cx.missing_arg().sexp()
            || (unsafe { vs.typ() } == SYMSXP && unsafe { crate::missing::r_is_missing(cx, vs, rho.bind(cx).sexp()) }?);
        let val = if is_missing { s.root(cx, cx.missing_arg()) } else { eval_root(cx, s, ve, rho)? };
        lx.release();
        let nlist = match sym {
            Some(sy) => sy,
            None => {
                let bytes = {
                    let c = input.bind(cx).as_char().unwrap();
                    robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                        .map(|b| b.into_owned())
                        .unwrap_or_else(|_| c.bytes().to_vec())
                };
                subset::install_sym(cx, s, &bytes)?
            }
        };
        subassign::subassign3_dflt(cx, call, x, nlist, val, out)
    })
}

// ---- ~, @, @<- --------------------------------------------------------------------

/// `do_tilde`: `y ~ x` is its own call, duplicated, of class "formula" with
/// the environment it was evaluated in as `.Environment` (a call that is
/// already an object is only duplicated).
// R: names.c:do_tilde
pub fn do_tilde(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let r = o.root();
        cx.duplicate(call, o)?;
        if !call.bind(cx).is_object() {
            let klass = s.out(cx);
            let k = klass.root();
            cx.build(klass, |cx| cx.mk_string(b"formula", robigo_rtcore::Encoding::Native))?;
            robigo_rtbuiltins::attrib::set_attrib(cx, r, b"class", k)?;
            robigo_rtbuiltins::attrib::set_attrib(cx, r, b".Environment", rho)?;
        }
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// `isMethodsDispatchOn()`: never, in robigo-rt yet (no methods package;
/// [`do_s4on`]).
// R: objects.c:R_isMethodsDispatchOn
fn methods_dispatch_on() -> bool {
    false
}

/// `do_AT`: `object@name`. An S3 object (not S4) dispatches on `@` first,
/// the name made a string as `$`'s is; then, without the methods package,
/// formal classes are an error.
// R: attrib.c:do_AT
pub fn do_at(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, "@", 2)?;
    cx.scope(|cx, s| {
        let e = s.root(cx, arg(cx, call, 0).unwrap().0);
        let x = eval_root(cx, s, e, rho)?;
        let xv = x.bind(cx);
        if xv.is_object() && !xv.is_s4() {
            // fixSubset3Args (its errors), the object as an evaluated
            // promise, then DispatchOrEval
            let (input, _) = subset3_name(cx, s, call, rho)?;
            let lx = link_root(x);
            let prom = ev_promise(cx, s, e, x)?;
            let str = scalar_string(cx, s, input)?;
            // shallow_duplicate(args), the name's cell holding the string
            // (the arguments' tags kept)
            let after = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr());
            let rest = tagged_rest(cx, s, after, str)?;
            let r = crate::dispatch::dispatch_special(cx, "@", call, prom, rest, rho, &out);
            lx.release();
            if r? {
                return Ok(());
            }
            // fall through to handle @.Data or signal an error
        }
        if !methods_dispatch_on() {
            return Err(cx.error("formal classes cannot be used without the 'methods' package"));
        }
        // R_do_slot: only with the methods package
        Err(cx.error("robigo: '@' with the methods package (R_do_slot) is not implemented yet"))
    })
}

/// `do_attrgets` for `@<-` (`PRIMVAL(op)` 1): `` `@<-`(object, name,
/// value) ``, the name a symbol or one string. An object dispatches on
/// `@<-`; otherwise the slot assignment is checked by the methods package's
/// `checkAtAssignment`, which needs `initMethodDispatch()` evaluated in its
/// namespace first (the global environment until methods is loaded).
// R: attrib.c:do_attrgets, check_slot_assign
pub fn do_slot_gets(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    slot_gets(cx, call, None, rho, out)
}

/// [`do_slot_gets`] given its call and its arguments apart, as bcEval's
/// `SETTER_CALL` calls a special (`PRIMFUN(fun)(call, fun, args, rho)`,
/// `args` the call's arguments with evaluated promises of `*tmp*` and of
/// the value): `None` for the call's own (`CDR(call)`). do_attrgets puts the
/// name's string into `args`; when those are the call's own, the call
/// dispatched with holds it too.
// R: attrib.c:do_attrgets, eval.c:bcEval SETTER_CALL
pub(crate) fn slot_gets(
    cx: &mut Cx,
    call: Root<'_>,
    args: Option<Root<'_>>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let own = args.is_none();
        let args = match args {
            Some(a) => a,
            None => s.root(cx, call.bind(cx).as_cons().unwrap().cdr()),
        };
        let n = args.bind(cx).len().unwrap_or(0);
        crate::prims::check_arity(cx, call, n, "@<-", 3, false)?;
        // the slot name as a string
        let input = {
            let nl = list_elt(cx, args, 1);
            let c = match nl.sexptype() {
                SYMSXP => nl.as_sym().unwrap().name(cx),
                STRSXP => {
                    let sv = nl.as_vector::<Char<'_>>().unwrap();
                    if sv.len() != 1 {
                        return Err(cx.error("invalid slot name length"));
                    }
                    sv.get(0)
                }
                _ => {
                    let msg = format!("invalid type '{}' for slot name", robigo_rtbuiltins::util::type_to_char(nl));
                    return Err(cx.error(&msg));
                }
            };
            let c = s.root(cx, c.as_ref());
            scalar_string(cx, s, c)?
        };
        // DispatchOrEval(call, op, "@<-", args, env, &ans, 0, 0), the
        // second argument replaced by the string
        let e = s.root(cx, list_elt(cx, args, 0));
        if e.bind(cx).sexp() == prims::state(cx).syms.dots {
            return Err(cx.error("robigo: '@<-' with '...' as its object is not implemented yet"));
        }
        let x = eval_root(cx, s, e, rho)?;
        let lx = link_root(x);
        if x.bind(cx).is_object() {
            let prom = ev_promise(cx, s, e, x)?;
            // the arguments after the object, their tags kept (`value =`),
            // the name's cell holding the string
            let after = s.root(cx, args.bind(cx).as_cons().unwrap().cdr());
            let rest = tagged_rest(cx, s, after, input)?;
            // SETCADR(args, input): the call's own arguments are the call's
            let dcall = if own {
                let o = s.out(cx);
                let c = o.root();
                cx.shallow_duplicate(call, o)?;
                let second = s.root(cx, c.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr());
                cx.set_car(second, input);
                c
            } else {
                call
            };
            match crate::dispatch::dispatch_special(cx, "@<-", dcall, prom, rest, rho, &out) {
                Ok(false) => {}
                r => {
                    lx.release();
                    r?;
                    // SAFETY: raised above.
                    unsafe { links::decrement_links(x.bind(cx).sexp()) };
                    return Ok(());
                }
            }
        }
        // evalListKeepMissing of the rest: the name (a constant), then the
        // value
        let _ = eval_root(cx, s, input, rho)?;
        let ve = s.root(cx, list_elt(cx, args, 2));
        let vs = ve.bind(cx).sexp();
        // SAFETY: a part of the rooted call and the rooted environment.
        let is_missing = vs == cx.missing_arg().sexp()
            || (unsafe { vs.typ() } == SYMSXP && unsafe { crate::missing::r_is_missing(cx, vs, rho.bind(cx).sexp()) }?);
        let value = if is_missing { s.root(cx, cx.missing_arg()) } else { eval_root(cx, s, ve, rho)? };
        lx.release();
        // check_slot_assign(obj, input, value, env), after the classes of
        // both (R_data_class: nothing to observe)
        if !methods_dispatch_on() {
            // eval(lang1(install("initMethodDispatch")), R_MethodsNamespace),
            // which is R_GlobalEnv without methods
            let f = subset::install_sym(cx, s, b"initMethodDispatch")?;
            let o = s.out(cx);
            let lang = o.root();
            cx.build(o, |cx| cx.lang1(f.bind(cx)))?;
            let g = s.root(cx, cx.global_env());
            let _ = eval_root(cx, s, lang, g)?;
        }
        // findFun(install("checkAtAssignment"), R_MethodsNamespace)
        let f = subset::install_sym(cx, s, b"checkAtAssignment")?;
        let g = s.root(cx, cx.global_env());
        let o = s.out(cx);
        cx.find_fun3(f, g, None, o)?;
        let _ = value;
        Err(cx.error(
            "robigo: '@<-' with the methods package (checkAtAssignment, R_do_slot_assign) is not implemented yet",
        ))
    })
}

// ---- switch -------------------------------------------------------------------

/// `expandDots(el, rho)`: the call's arguments from the second on, `...`
/// replaced by its elements (promises), with their tags.
// R: builtin.c:expandDots
fn expand_dots<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, rho: Root<'_>) -> RResult<Vec<(Root<'s>, Sexp)>> {
    let dots = prims::state(cx).syms.dots;
    let missing = cx.missing_arg().sexp();
    let n = nargs(cx, call);
    let mut w = Vec::with_capacity(n);
    for i in 1..n {
        let (e, tag) = {
            let (e, t) = arg(cx, call, i).unwrap();
            (s.root(cx, e), t.map_or(cx.nil().sexp(), |t| t.as_ref().sexp()))
        };
        if e.bind(cx).sexp() != dots {
            w.push((e, tag));
            continue;
        }
        let o = s.out(cx);
        let h = o.root();
        // SAFETY: an immortal symbol.
        let d = s.root(cx, unsafe { Ref::from_sexp(dots, cx) });
        cx.find_var(d, rho, o)?;
        let t = h.bind(cx).sexptype();
        if t == DOTSXP || t == NILSXP {
            let mut c = h.bind(cx).as_cons();
            let mut items = Vec::new();
            while let Some(cell) = c {
                items.push((cell.car(), cell.tag().sexp()));
                c = cell.cdr().as_cons();
            }
            for (car, tag) in items {
                w.push((s.root(cx, car), tag));
            }
        } else if h.bind(cx).sexp() != missing {
            return Err(cx.error("'...' used in an incorrect context"));
        }
    }
    Ok(w)
}

/// `setDflt(arg, dflt)`: a second default alternative is GNU R's error,
/// which deparses both (B3's deparse is not ported yet).
// R: builtin.c:setDflt
fn set_dflt(cx: &mut Cx, i: usize, dflt: &mut Option<usize>) -> RResult<()> {
    if dflt.is_some() {
        return Err(
            cx.error("robigo: the error for duplicate 'switch' defaults (it deparses them) is not implemented yet")
        );
    }
    *dflt = Some(i);
    Ok(())
}

/// `asInteger(x)` of a length-1 atomic vector, with GNU R's coercion
/// warnings.
// R: coerce.c:asInteger
pub(crate) fn as_integer(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    const NA: i32 = i32::MIN;
    let v = x.bind(cx);
    let (r, warn) = match v.view() {
        robigo_rtcore::View::Lgl(l) => (l.get(0).0, None),
        robigo_rtcore::View::Int(l) => (l.get(0), None),
        robigo_rtcore::View::Real(l) => {
            let d = l.get(0);
            if d.is_nan() {
                (NA, None)
            } else if d >= i32::MAX as f64 + 1.0 || d <= i32::MIN as f64 {
                (NA, Some("NAs introduced by coercion to integer range"))
            } else {
                (d as i32, None)
            }
        }
        robigo_rtcore::View::Cplx(_) | robigo_rtcore::View::Str(_) => {
            return Err(cx.error("robigo: 'switch' on a complex or character code (asInteger) is not implemented yet"));
        }
        robigo_rtcore::View::Raw(_) => return Err(cx.error("unimplemented type 'raw' in 'asInteger'\n")),
        _ => (NA, None),
    };
    if let Some(m) = warn {
        cx.warning(m)?;
    }
    Ok(r)
}

/// `do_switch`: `switch(EXPR, ...)`. The chosen alternative is evaluated
/// (its value visible as it leaves it); no match is NULL, invisible.
// R: builtin.c:do_switch
pub fn do_switch(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = nargs(cx, call);
    if n < 1 {
        return Err(cx.error_call(Some(call), "'EXPR' is missing"));
    }
    if let Some((_, Some(t))) = arg(cx, call, 0) {
        let supplied = t.name_bytes();
        if supplied.len() > 4 || !b"EXPR".starts_with(supplied) {
            let msg = format!("supplied argument name '{}' does not match 'EXPR'", String::from_utf8_lossy(supplied));
            return Err(cx.error_call(Some(call), &msg));
        }
    }
    cx.scope(|cx, s| {
        let e = s.root(cx, arg(cx, call, 0).unwrap().0);
        let x = eval_root(cx, s, e, rho)?;
        let xv = x.bind(cx);
        if !robigo_rtbuiltins::attrib::is_vector(xv) || xv.len() != Some(1) {
            return Err(cx.error_call(Some(call), "EXPR must be a length 1 vector"));
        }
        if robigo_rtbuiltins::attrib::inherits(xv, b"factor") {
            cx.warning_call(
                Some(call),
                "EXPR is a \"factor\", treated as integer.\n Consider using 'switch(as.character( * ), ...)' instead.",
            )?;
        }
        if n > 1 {
            let w = expand_dots(cx, s, call, rho)?;
            let missing = cx.missing_arg().sexp();
            let nil = cx.nil().sexp();
            let is_str = x.bind(cx).sexptype() == STRSXP;
            if is_str {
                // pmatch(STRING_ELT(x, 0), TAG(y), exact): the CHARSXP's
                // bytes (no translation) against the tag's name
                let key = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
                let mut dflt = None;
                let mut i = 0;
                while i < w.len() {
                    let tag = w[i].1;
                    if tag != nil {
                        // SAFETY: a tag is an immortal symbol.
                        let name = unsafe { Ref::from_sexp(tag, cx) }.as_sym().unwrap().name_bytes();
                        if name == key.as_slice() {
                            // the next non-missing alternative
                            let mut y = i;
                            while w[y].0.bind(cx).sexp() == missing {
                                y += 1;
                                if y == w.len() {
                                    break;
                                }
                                if w[y].1 == nil {
                                    set_dflt(cx, y, &mut dflt)?;
                                }
                            }
                            if y == w.len() {
                                cx.set_visible(false);
                                out.set(cx, cx.nil());
                                return Ok(());
                            }
                            for z in y + 1..w.len() {
                                if w[z].1 == nil {
                                    set_dflt(cx, z, &mut dflt)?;
                                }
                            }
                            // (under PROTECT(x), PROTECT(w): robigo_rtcore's ppacct)
                            return cx.with_pp_excess(2, |cx| eval(cx, w[y].0, rho, out));
                        }
                    } else {
                        set_dflt(cx, i, &mut dflt)?;
                    }
                    i += 1;
                }
                if let Some(d) = dflt {
                    return cx.with_pp_excess(2, |cx| eval(cx, w[d].0, rho, out));
                }
            } else {
                let argval = as_integer(cx, x)?;
                if argval != i32::MIN && argval >= 1 && argval as usize <= w.len() {
                    let alt = w[argval as usize - 1].0;
                    if alt.bind(cx).sexp() == missing {
                        return Err(cx.error("empty alternative in numeric switch"));
                    }
                    return cx.with_pp_excess(2, |cx| eval(cx, alt, rho, out));
                }
            }
        } else {
            cx.warning_call(Some(call), "'switch' with no alternatives")?;
        }
        cx.set_visible(false);
        out.set(cx, cx.nil());
        Ok(())
    })
}

// ---- on.exit, substitute: matched arguments ---------------------------------

/// `matchArgs_NR(formals, args, call)` for a special's formals (no `...`):
/// each formal's expression, `R_MissingArg` when not supplied.
fn match_formals<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, formals: &[&[u8]]) -> RResult<Vec<Root<'s>>> {
    let syms: Vec<Root<'s>> = formals.iter().map(|f| subset::install_sym(cx, s, f)).collect::<RResult<_>>()?;
    // (symbols are immortal)
    let fs: Vec<Sexp> = syms.iter().map(|r| r.bind(cx).sexp()).collect();
    crate::args::match_builtin_args(cx, s, call, &fs)
}

/// `do_onexit`: `on.exit(expr, add, after)` attached to the function
/// record of `rho` (C2's records).
// R: builtin.c:do_onexit
pub fn do_onexit(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let a = match_formals(cx, s, call, &[b"expr", b"add", b"after"])?;
        let missing = cx.missing_arg();
        let code = if a[0].bind(cx) == missing { s.root(cx, cx.nil()) } else { a[0] };
        let flag = |cx: &mut Cx, r: Root<'_>| -> RResult<Option<i32>> {
            if r.bind(cx) == cx.missing_arg() {
                return Ok(None);
            }
            let v = eval_root(cx, s, r, rho)?;
            Ok(Some(robigo_rtbuiltins::util::as_logical(cx, v.bind(cx))))
        };
        let add = flag(cx, a[1])?;
        let after = flag(cx, a[2])?;
        cx.on_exit(call, rho, code, add, after)?;
        out.set(cx, cx.nil());
        Ok(())
    })
}

/// `substitute(lang, rho)` (`rho` None: R_NilValue, no substitution): the
/// value of the walk over the symbols looked up beforehand in `found`.
// R: coerce.c:substitute
fn substitute<'g>(cx: &'g Cx, lang: Ref<'g>, found: &Found<'g>) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    match lang.sexptype() {
        PROMSXP => substitute(cx, prexpr(cx, lang), found),
        SYMSXP => {
            if let Some(f) = found
                && let Some(&(_, t)) = f.iter().find(|(s, _)| *s == lang.sexp())
                && let Some(t) = t
            {
                if t.sexptype() == PROMSXP {
                    let mut t = t;
                    while t.sexptype() == PROMSXP {
                        t = prexpr(cx, t);
                    }
                    return Ok(t);
                }
                // (a DOTSXP was refused while looking up; GlobalEnv is no
                // environment here)
                return Ok(t);
            }
            Ok(lang)
        }
        LANGSXP => substitute_list(cx, lang, found),
        _ => Ok(lang),
    }
}

/// `substitute(lang, R_NilValue)`: the expression behind `lang`'s
/// promises, no symbol replaced (`do_bind`'s naming of its arguments).
// R: coerce.c:substitute
pub(crate) fn substitute_nil<'g>(cx: &'g Cx, lang: Ref<'g>) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    substitute(cx, lang, &None)
}

/// A promise's expression (`PREXPR`, `R_PromiseExpr`): its code, or for
/// byte code (a promise compiled code made) the code's first constant.
// R: eval.c:R_PromiseExpr
fn prexpr<'g>(cx: &'g Cx, p: Ref<'g>) -> Ref<'g> {
    match p.view() {
        robigo_rtcore::View::Promise(pr) => robigo_rtbuiltins::bytecode::expr(cx, pr.code()),
        _ => unreachable!(),
    }
}

/// The symbol lookups of a substitution: each symbol of the expression and
/// what `R_findVarInFrame` gave (None: unbound), or None for no
/// environment.
type Found<'g> = Option<Vec<(Sexp, Option<Ref<'g>>)>>;

/// `substituteList(el, rho)`.
// R: coerce.c:substituteList
fn substitute_list<'g>(cx: &'g Cx, el: Ref<'g>, found: &Found<'g>) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    let dots = prims::state(cx).syms.dots;
    let mut items: Vec<(Ref<'g>, Ref<'g>, bool)> = Vec::new(); // (value, tag, lang cell)
    let mut c = el.as_cons();
    let is_lang = el.sexptype() == LANGSXP;
    let mut first = true;
    while let Some(cell) = c {
        let lang_cell = is_lang && first;
        first = false;
        if cell.car().sexp() == dots {
            let h = found.as_ref().and_then(|f| f.iter().find(|(s, _)| *s == dots).map(|&(_, v)| v));
            match h {
                None | Some(None) => items.push((cell.car(), cx.nil(), lang_cell)),
                Some(Some(h)) if h.sexptype() == NILSXP || h == cx.missing_arg() => {}
                Some(Some(h)) => {
                    // a DOTSXP: its elements substituted with no environment
                    let mut d = h.as_cons();
                    let mut lc = lang_cell;
                    while let Some(dc) = d {
                        items.push((substitute(cx, dc.car(), &None)?, dc.tag(), lc));
                        lc = false;
                        d = dc.cdr().as_cons();
                    }
                }
            }
        } else {
            items.push((substitute(cx, cell.car(), found)?, cell.tag(), lang_cell));
        }
        c = cell.cdr().as_cons();
    }
    let mut tail = cx.nil();
    for &(v, tag, lang) in items.iter().rev() {
        tail = match (lang, tag.as_sym()) {
            (true, Some(t)) => cx.lcons_tagged(v, tail, t)?,
            (true, None) => cx.lcons(v, tail)?,
            (false, Some(t)) => cx.cons_tagged(v, tail, t)?,
            (false, None) => cx.cons(v, tail)?,
        };
    }
    Ok(tail)
}

/// The symbols of `e` (and `...`), each once, for the lookups.
fn symbols_of<'g>(cx: &'g Cx, e: Ref<'g>, acc: &mut Vec<Sexp>) {
    match e.sexptype() {
        SYMSXP => {
            if !acc.contains(&e.sexp()) {
                acc.push(e.sexp());
            }
        }
        PROMSXP => symbols_of(cx, prexpr(cx, e), acc),
        LANGSXP | LISTSXP => {
            let mut c = e.as_cons();
            while let Some(cell) = c {
                symbols_of(cx, cell.car(), acc);
                c = cell.cdr().as_cons();
            }
        }
        _ => {}
    }
}

/// `substitute`'s environment of a list: a new environment whose frame
/// binds each named element (`VectorToPairList` keeps the names as tags; an
/// unnamed element binds nothing a symbol could find), enclosed by base.
// R: coerce.c:do_substitute, VectorToPairList
fn list_env<'s>(cx: &mut Cx, s: Scope<'s>, env: Root<'_>) -> RResult<Root<'s>> {
    let base = s.root(cx, cx.base_env());
    let e = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(base.bind(cx), false, 0)))?;
    let loc = cx.locale();
    let mut bad = None;
    let pairs: Vec<(Option<Vec<u8>>, usize)> = {
        let v = env.bind(cx);
        match v.view() {
            View::List(l) => {
                let names = robigo_rtbuiltins::attrib::names(v).and_then(|n| n.as_vector::<Char<'_>>());
                let mut out = Vec::with_capacity(l.len());
                for i in 0..l.len() {
                    // installTrChar: the name in the native encoding
                    let name = match names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty()) {
                        Some(c) => match robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), loc) {
                            Ok(b) => Some(b.into_owned()),
                            Err(e) => {
                                bad = Some(e.to_string());
                                break;
                            }
                        },
                        None => None,
                    };
                    out.push((name, i));
                }
                out
            }
            _ => {
                let mut out = Vec::new();
                let mut c = v.as_cons();
                let mut i = 0;
                while let Some(cell) = c {
                    out.push((cell.tag().as_sym().map(|t| t.name_bytes().to_vec()), i));
                    c = cell.cdr().as_cons();
                    i += 1;
                }
                out
            }
        }
    };
    if let Some(msg) = bad {
        return Err(cx.error(&msg));
    }
    // a frame lookup finds a name's first cell: defined in reverse, the
    // first element of a repeated name is the binding that stays
    for (name, i) in pairs.into_iter().rev() {
        let Some(name) = name.filter(|n| !n.is_empty()) else { continue };
        cx.scope(|cx, t| {
            let val = {
                let v = env.bind(cx);
                let x = match v.view() {
                    View::List(l) => l.get(i),
                    _ => {
                        let mut c = v.as_cons().unwrap();
                        for _ in 0..i {
                            c = c.cdr().as_cons().unwrap();
                        }
                        c.car()
                    }
                };
                t.root(cx, x)
            };
            let sym = cx.install_rooted(t, &name)?;
            cx.define_var(sym, val, e)
        })?;
    }
    Ok(e)
}

/// `do_substitute`: `substitute(expr, env)`.
// R: coerce.c:do_substitute
pub fn do_substitute(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let a = match_formals(cx, s, call, &[b"expr", b"env"])?;
        let env = if a[1].bind(cx) == cx.missing_arg() { rho } else { eval_root(cx, s, a[1], rho)? };
        let ev = env.bind(cx);
        let env = if ev == cx.global_env() {
            None
        } else {
            match ev.sexptype() {
                ENVSXP => Some(env),
                NILSXP => None,
                // NewEnvironment(R_NilValue, VectorToPairList(env), R_BaseEnv), or
                // of duplicate(env) for a pairlist
                VECSXP | LISTSXP => Some(list_env(cx, s, env)?),
                _ => return Err(cx.error_call(Some(call), "invalid environment specified")),
            }
        };
        // t = CONS(duplicate(CAR(argList)), R_NilValue)
        let o = s.out(cx);
        let expr = o.root();
        cx.duplicate(a[0], o)?;
        // the lookups, materializing a frame's arguments as GNU R's
        // bindings (promises)
        let syms = {
            let mut v = Vec::new();
            symbols_of(cx, expr.bind(cx), &mut v);
            v
        };
        let dots = prims::state(cx).syms.dots;
        let mut found: Vec<(Sexp, Option<Root<'_>>)> = Vec::new();
        if let Some(env) = env {
            for sym in syms {
                // SAFETY: an immortal symbol.
                let sr = s.root(cx, unsafe { Ref::from_sexp(sym, cx) });
                let o = s.out(cx);
                let r = o.root();
                cx.find_var_in_frame3(env, sr, true, o)?;
                let v = r.bind(cx);
                if v == cx.unbound() {
                    found.push((sym, None));
                    continue;
                }
                if v.sexptype() == DOTSXP && sym != dots {
                    return Err(cx.error("'...' used in an incorrect context"));
                }
                if sym == dots && !matches!(v.sexptype(), DOTSXP | NILSXP) && v != cx.missing_arg() {
                    return Err(cx.error("'...' used in an incorrect context"));
                }
                found.push((sym, Some(r)));
            }
        }
        let has_env = env.is_some();
        cx.build(out, |cx| {
            let f: Found<'_> = has_env.then(|| found.iter().map(|&(s, r)| (s, r.map(|r| r.bind(cx)))).collect());
            let lst = cx.cons(expr.bind(cx), cx.nil())?;
            let r = substitute_list(cx, lst, &f)?;
            Ok(r.as_cons().map_or(cx.nil(), |c| c.car()))
        })
    })
}

/// `expression(...)`: the arguments, unevaluated, as an expression vector
/// (each duplicated if referenced: they are the call's), named if any
/// argument is.
// R: builtin.c:do_expression
pub fn do_expression(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let items: Vec<(Root<'_>, Option<Root<'_>>)> = {
            let mut v = Vec::new();
            let mut a = call.bind(cx).as_cons().map(|c| c.cdr()).and_then(|c| c.as_cons());
            while let Some(c) = a {
                v.push((c.car(), (c.tag() != cx.nil()).then(|| c.tag())));
                a = c.cdr().as_cons();
            }
            v.into_iter().map(|(x, t)| (s.root(cx, x), t.map(|t| s.root(cx, t)))).collect()
        };
        let named = items.iter().any(|(_, t)| t.is_some());
        let mut vals = Vec::with_capacity(items.len());
        for (x, _) in &items {
            if cx.referenced(x.bind(cx)) {
                let o = s.out(cx);
                let r = o.root();
                cx.duplicate(*x, o)?;
                vals.push(r);
            } else {
                vals.push(*x);
            }
        }
        cx.build(out, |cx| {
            let mut ans = cx.alloc_expression(vals.len())?;
            for (i, v) in vals.iter().enumerate() {
                ans.set(i, v.bind(cx));
            }
            if named {
                let mut nms = cx.alloc_vector::<Char<'_>>(items.len())?;
                for (i, (_, t)) in items.iter().enumerate() {
                    nms.set(i, t.map_or(cx.blank_string(), |t| t.bind(cx).as_sym().unwrap().name(cx)));
                }
                let mut a = robigo_rtbuiltins::attrib::Attrs::new();
                a.install(robigo_rtbuiltins::attrib::Attr {
                    tag: cx.sym(robigo_rtcore::RSym::Names),
                    value: nms.finish().into(),
                });
                a.apply(cx, &mut ans)?;
            }
            Ok(ans.finish().into())
        })
    })
}

/// `.isMethodsDispatchOn(onOff)`: whether S4 dispatch is on, which it
/// never is in robigo-rt yet (no methods package); `FALSE` turns it off.
// R: objects.c:do_S4on, R_isMethodsDispatchOn
pub fn do_s4on(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if !args.is_empty() && args.get(cx, 0).len().unwrap_or(0) > 0 {
        match robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 0)) {
            i32::MIN => return Err(cx.error("'onOff' must be TRUE or FALSE")),
            0 => {}
            _ => {
                cx.warning("R_isMethodsDispatchOn(TRUE) called -- may not work correctly")?;
                return Err(cx.error("robigo: the methods package (initMethodDispatch) is not implemented yet"));
            }
        }
    }
    cx.build_scalar(out, robigo_rtcore::Lgl(0))
}
