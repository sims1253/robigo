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
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, Scope, Slot, Sym};
use std::ptr::NonNull;

/// The number of arguments of `call`.
fn nargs(cx: &Cx, call: Root<'_>) -> usize {
    call.bind(cx).len().unwrap_or(1).saturating_sub(1)
}

/// `checkArity(op, args)` of a primitive special: GNU R's message against
/// the call.
// R: errors.c:Rf_checkArityCall
fn check_arity(cx: &mut Cx, call: Root<'_>, name: &str, arity: usize) -> RResult<()> {
    let n = nargs(cx, call);
    if n == arity {
        return Ok(());
    }
    let msg = format!("{n} argument{} passed to '{name}' which requires {arity}", if n == 1 { "" } else { "s" });
    Err(cx.error_call(Some(call), &msg))
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

/// The evaluated arguments: a window of value slots and tag slots of the
/// caller's scope, filled in order (GNU R's `CONS_NR` list).
struct Evaluated<'s> {
    vals: NonNull<Slot>,
    names: NonNull<Slot>,
    n: usize,
    cap: usize,
    _s: std::marker::PhantomData<Scope<'s>>,
}

impl<'s> Evaluated<'s> {
    /// Room for `cap` arguments in scope `s`.
    fn new(cx: &Cx, s: Scope<'s>, cap: usize) -> Evaluated<'s> {
        Evaluated { vals: s.frame(cx, cap), names: s.frame(cx, cap), n: 0, cap, _s: std::marker::PhantomData }
    }

    /// The next value's slot, its tag written.
    fn next(&mut self, tag: Sexp) -> Out<'s> {
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

    /// Value `i`.
    fn root(&self, i: usize) -> Root<'s> {
        assert!(i < self.n);
        // SAFETY: a filled slot of the window.
        unsafe { Root::from_raw(self.vals.add(i)) }
    }

    /// The window for a builtin's default method.
    fn args(&self, cx: &Cx) -> Args<'s> {
        let any_alt = (0..self.n).any(|i| self.root(i).bind(cx).is_package_altrep());
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
) -> RResult<()> {
    let start = ev.n;
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
                    let o = ev.next(tag);
                    let r = o.root();
                    if car.sexp() == missing {
                        o.set(cx, car);
                    } else {
                        let er = s2.root(cx, car);
                        eval(cx, er, rho, o)?;
                    }
                    // SAFETY: a rooted value.
                    unsafe { links::increment_links(r.bind(cx).sexp()) };
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
            let o = ev.next(tag);
            let r = o.root();
            if is_missing {
                o.set(cx, cx.missing_arg());
            } else {
                let e = s.root(cx, cur.bind(cx).as_cons().unwrap().car());
                eval(cx, e, rho, o)?;
            }
            // SAFETY: a rooted value.
            unsafe { links::increment_links(r.bind(cx).sexp()) };
        }
        let next = s.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
        cur.replace_from(cx, next);
    }
    for i in start..ev.n {
        // SAFETY: each value's link was raised above.
        unsafe { links::decrement_links(ev.root(i).bind(cx).sexp()) };
    }
    Ok(())
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
            let o = ev.next(nil);
            let x = o.root();
            eval(cx, e, rho, o)?;
            // SAFETY: a rooted value.
            unsafe { links::increment_links(x.bind(cx).sexp()) };
            if !x.bind(cx).is_object() {
                eval_list_keep_missing(cx, s, call, 1, rho, &mut ev)?;
                // SAFETY: raised above.
                unsafe { links::decrement_links(x.bind(cx).sexp()) };
                return Ok(Disp::Default(ev));
            }
            let prom = ev_promise(cx, s, e, x)?;
            let rest = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr());
            let r = crate::dispatch::dispatch_special(cx, generic, call, prom, rest, rho, out);
            // SAFETY: raised above.
            unsafe { links::decrement_links(x.bind(cx).sexp()) };
            if r? {
                return Ok(Disp::Done);
            }
            eval_list_keep_missing(cx, s, call, 1, rho, &mut ev)?;
            Ok(Disp::Default(ev))
        }
        Some(_) => {
            // `...` first: DispatchOrEval finds the object among its
            // elements; with no object it evaluates the whole list
            let cap = count_args(cx, call, 0, rho)?;
            let mut ev = Evaluated::new(cx, s, cap);
            eval_list_keep_missing(cx, s, call, 0, rho, &mut ev)?;
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
fn ev_promise<'s>(cx: &mut Cx, s: Scope<'s>, expr: Root<'_>, value: Root<'_>) -> RResult<Root<'s>> {
    let p = crate::vm::alloc_retry(cx, |cx| {
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
            let name = v.as_sym().unwrap().name();
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
        // SAFETY: a rooted value.
        unsafe { links::increment_links(x.bind(cx).sexp()) };
        if x.bind(cx).is_object() {
            let prom = ev_promise(cx, s, e, x)?;
            // fixSubset3Args: the name as a string is the second argument
            let str = scalar_string(cx, s, input)?;
            let o = s.out(cx);
            let rest = o.root();
            cx.build(o, |cx| cx.cons(str.bind(cx), cx.nil()))?;
            let r = crate::dispatch::dispatch_special(cx, "$", call, prom, rest, rho, &out);
            // SAFETY: raised above.
            unsafe { links::decrement_links(x.bind(cx).sexp()) };
            if r? {
                return Ok(());
            }
        } else {
            // the name, a string constant, is evaluated as an argument
            let str = scalar_string(cx, s, input)?;
            let _ = eval_root(cx, s, str, rho)?;
            // SAFETY: raised above.
            unsafe { links::decrement_links(x.bind(cx).sexp()) };
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
        // SAFETY: a rooted value.
        unsafe { links::increment_links(x.bind(cx).sexp()) };
        if x.bind(cx).is_object() {
            let prom = ev_promise(cx, s, e, x)?;
            // fixSubset3Args: (name as a string, value expression)
            let str = scalar_string(cx, s, input)?;
            let ve = s.root(cx, arg(cx, call, 2).unwrap().0);
            let o = s.out(cx);
            let rest = o.root();
            cx.build(o, |cx| {
                let t = cx.cons(ve.bind(cx), cx.nil())?;
                cx.cons(str.bind(cx), t)
            })?;
            let r = crate::dispatch::dispatch_special(cx, "$<-", call, prom, rest, rho, &out);
            if r? {
                // SAFETY: raised above.
                unsafe { links::decrement_links(x.bind(cx).sexp()) };
                return Ok(());
            }
            // no method: the default, with the arguments evaluated
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
        // SAFETY: raised above.
        unsafe { links::decrement_links(x.bind(cx).sexp()) };
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
fn as_integer(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
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
        let supplied = t.name().bytes();
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
                        let name = unsafe { Ref::from_sexp(tag, cx) }.as_sym().unwrap().name().bytes();
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
                            return eval(cx, w[y].0, rho, out);
                        }
                    } else {
                        set_dflt(cx, i, &mut dflt)?;
                    }
                    i += 1;
                }
                if let Some(d) = dflt {
                    return eval(cx, w[d].0, rho, out);
                }
            } else {
                let argval = as_integer(cx, x)?;
                if argval != i32::MIN && argval >= 1 && argval as usize <= w.len() {
                    let alt = w[argval as usize - 1].0;
                    if alt.bind(cx).sexp() == missing {
                        return Err(cx.error("empty alternative in numeric switch"));
                    }
                    return eval(cx, alt, rho, out);
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
        PROMSXP => substitute(cx, prexpr(lang), found),
        SYMSXP => {
            if let Some(f) = found
                && let Some(&(_, t)) = f.iter().find(|(s, _)| *s == lang.sexp())
                && let Some(t) = t
            {
                if t.sexptype() == PROMSXP {
                    let mut t = t;
                    while t.sexptype() == PROMSXP {
                        t = prexpr(t);
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

/// A promise's expression (`PREXPR`).
fn prexpr<'g>(p: Ref<'g>) -> Ref<'g> {
    match p.view() {
        robigo_rtcore::View::Promise(pr) => pr.code(),
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
fn symbols_of(e: Ref<'_>, acc: &mut Vec<Sexp>) {
    match e.sexptype() {
        SYMSXP => {
            if !acc.contains(&e.sexp()) {
                acc.push(e.sexp());
            }
        }
        PROMSXP => symbols_of(prexpr(e), acc),
        LANGSXP | LISTSXP => {
            let mut c = e.as_cons();
            while let Some(cell) = c {
                symbols_of(cell.car(), acc);
                c = cell.cdr().as_cons();
            }
        }
        _ => {}
    }
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
                VECSXP | LISTSXP => {
                    return Err(cx.error("robigo: 'substitute' in a list (as an environment) is not implemented yet"));
                }
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
            symbols_of(expr.bind(cx), &mut v);
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
