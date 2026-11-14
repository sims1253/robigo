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
use robigo_rtcore::{Cx, RResult, Root};
use std::collections::HashMap;

/// Run one compile attempt under `&Cx` until it succeeds: a refused
/// allocation collects and compiles again (the same request refused twice
/// is an error), a lazy binding a decision needs is forced first
/// ([`Need::Force`], with `R_Visible` kept as `R_cmpfun1` keeps it). None
/// when cmp.R would leave the code uncompiled ([`Need::Fail`]).
fn attempt(cx: &mut Cx, mut f: impl FnMut(&Cx) -> Result<Sexp, Need>) -> RResult<Option<Sexp>> {
    let mut retried = None;
    loop {
        match f(cx) {
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
            Err(Need::Gc(e)) if retried == Some(e) => {
                return Err(cx.error_call(None, "robigo: no room to compile an expression"));
            }
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
                Ok(crate::code::build(cx, &b, e, None)?)
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
            Ok(crate::code::build(cx, &b, e, None)?)
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
            let (formals, body) = (f.car(), f.cdr());
            let st = prims::state(cx);
            let mut c = Comp::new(cx, st, Profile::Eval, None);
            let b = c.root(body)?;
            Ok(crate::code::build(cx, &b, body, Some(formals))?)
        }
    })
    .map(|c| c.expect("robigo: the Eval profile always compiles"))
}

/// `cmpfun(f)` for closure `op` (rooted) under the `Bc` profile: its body
/// compiled with cmp.R's decisions for its environment as it is now; None
/// when cmp.R returns the closure uncompiled (it may call `browser()`, a
/// compiler error).
// R: cmp.R:cmpfun, make.functionContext, funEnv
pub fn compile_closure_bc(cx: &mut Cx, op: Root<'_>) -> RResult<Option<Sexp>> {
    attempt(cx, |cx| {
        let f = op.bind(cx).sexp();
        // SAFETY: a rooted closure (and cache entry); nothing collects in
        // this interval.
        unsafe {
            let (formals, body) = (f.car(), f.cdr());
            let mut cenv = Cenv::of_env(cx, f.tag())?;
            let st = prims::state(cx);
            if !function_context(cx, st, &mut cenv, formals, body)? {
                return Err(Need::Fail);
            }
            let mut c = Comp::new(cx, st, Profile::Bc, Some(cenv));
            let mut b = c.root(body)?;
            c.add_unit_locals(&mut b);
            Ok(crate::code::build(cx, &b, body, Some(formals))?)
        }
    })
}

/// `make.functionContext(cntxt, forms, body)` on `cenv`: a frame of the
/// formals, then the locals of their defaults and the body; false when
/// the body may call `browser()` (cmp.R does not compile it). The frame is
/// left on `cenv` (popped by the caller when it is done).
///
/// # Safety
/// `formals` (a pairlist) and `body` are live.
unsafe fn function_context(cx: &Cx, st: &RtState, cenv: &mut Cenv, formals: Sexp, body: Sexp) -> Result<bool, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        let forms = cenv::elements(cx, formals, false);
        cenv.push_unit(forms.iter().map(|f| f.1).collect());
        let mut elist: Vec<Sexp> = forms.iter().map(|f| f.0).collect();
        elist.push(body);
        let locals = cenv.find_locals(cx, st, &elist)?;
        cenv.add_unit_vars(&locals);
        Ok(!cenv::may_call_browser(cx, st, cenv, body))
    }
}

/// `R_compileExpr(call, rho)` for a top-level loop (`compile(e, env)`: the
/// loop's locals added to the global frame): its code, or None when cmp.R
/// fails (`tryCompile` returns the call; it runs uncompiled).
// R: eval.c:R_compileExpr; cmp.R:compile
pub fn compile_top_loop(cx: &mut Cx, call: Root<'_>, rho: Root<'_>) -> RResult<Option<Sexp>> {
    attempt(cx, |cx| {
        let (e, env) = (call.bind(cx).sexp(), rho.bind(cx).sexp());
        // SAFETY: rooted objects; nothing collects in this interval.
        unsafe {
            let mut cenv = Cenv::of_env(cx, env)?;
            let st = prims::state(cx);
            let locals = cenv.find_locals(cx, st, &[e])?;
            cenv.add_top_vars(locals);
            let mut c = Comp::new(cx, st, Profile::Bc, Some(cenv));
            let b = c.root(e)?;
            Ok(crate::code::build(cx, &b, e, None)?)
        }
    })
}

/// Compile the call `call` of the special at `PRIMOFFSET` `off`, reached
/// through a value rather than its name (no guard): its operands are
/// evaluated one level below the code object's base, as the special's own
/// `eval` calls nest. Always the `Eval` profile: GNU R calls the special,
/// which evaluates its arguments by `eval` (bcEval's `CALLSPECIAL` too).
///
/// `call` is read through its root on every attempt.
pub fn compile_special(cx: &mut Cx, call: robigo_rtcore::Root<'_>, off: usize) -> RResult<Sexp> {
    attempt(cx, |cx| {
        let call = call.bind(cx).sexp();
        // SAFETY: a rooted call of `cx`'s isolate; nothing collects in this
        // interval.
        unsafe {
            let st = prims::state(cx);
            let mut c = Comp::new(cx, st, Profile::Eval, None);
            let dst = c.alloc();
            match st.defs[off].kind {
                Kind::Special(s) => c.special(s, call, call.cdr(), dst, 0, true)?,
                Kind::Builtin(..) | Kind::Fn(_) => unreachable!("robigo: only a language special is compiled"),
            }
            let mut i = Ins::new(Op::Ret);
            i.b = dst;
            c.emit(i);
            let b = c.finish();
            Ok(crate::code::build(cx, &b, call, None)?)
        }
    })
    .map(|c| c.expect("robigo: the Eval profile always compiles"))
}

/// The registers of a `for` loop's state ([`Op::ForPrep`]).
pub const FOR_STATE: usize = 5;

/// Labels of a loop being compiled: `break` jumps to its end, `next` to
/// its start (both inside its region).
struct LoopLabels {
    start: usize,
    breaks: Vec<usize>,
    /// The first register the loop's body allocates: a `break` or `next`
    /// leaves every register from here on dead.
    regs: Reg,
}

struct Comp<'g> {
    cx: &'g Cx,
    st: &'g RtState,
    profile: Profile,
    ins: Vec<Ins>,
    consts: Vec<Sexp>,
    pool: HashMap<usize, u16>,
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
    bc: Option<Cenv>,
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
    fn new(cx: &'g Cx, st: &'g RtState, profile: Profile, bc: Option<Cenv>) -> Comp<'g> {
        debug_assert_eq!(bc.is_some(), profile == Profile::Bc);
        Comp {
            cx,
            st,
            profile,
            ins: Vec::new(),
            consts: Vec::new(),
            pool: HashMap::new(),
            next: R_FIRST,
            max: R_FIRST,
            max_depth: 0,
            loops: Vec::new(),
            locals: Vec::new(),
            sites: 0,
            bc,
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

    /// Compile the whole expression: its value is the code object's result.
    unsafe fn root(&mut self, expr: Sexp) -> Result<Built, Need> {
        let r = self.alloc();
        // SAFETY: the caller's contract.
        unsafe { self.expr(expr, r, 1, true)? };
        let mut i = Ins::new(Op::Ret);
        i.b = r;
        self.emit(i);
        Ok(self.finish())
    }

    /// The compiled code. Under the `Bc` profile no instruction is an
    /// evaluation point: bcEval counts and nests once per code object.
    fn finish(&mut self) -> Built {
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
        if let Some(&i) = self.pool.get(&(x.0 as usize)) {
            return i;
        }
        let i = u16::try_from(FIRST_CONST + self.consts.len()).expect("robigo: too many constants in one code object");
        self.consts.push(x);
        self.pool.insert(x.0 as usize, i);
        i
    }

    fn emit(&mut self, i: Ins) -> usize {
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
    unsafe fn expr(&mut self, e: Sexp, dst: Reg, d: u16, tail: bool) -> Result<(), Need> {
        // SAFETY: the caller's contract.
        let t = unsafe { e.typ() };
        match t {
            SYMSXP => {
                // SAFETY: a live symbol.
                let dd = unsafe { e.gp() } & robigo_rtcore::DDVAL != 0;
                let mut i = Ins::new(if dd { Op::GetDdVar } else { Op::GetVar });
                i.a = dst;
                i.e = self.k(e);
                let mut i = self.at_depth(i, d, false);
                if tail {
                    i.fl |= F_VIS;
                }
                self.emit(i);
            }
            LANGSXP => {
                // SAFETY: as above.
                unsafe { self.call(e, dst, d, tail)? };
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
                // a self-evaluating object: `eval`'s ENSURE_NAMEDMAX(e) does
                // nothing under reference counting (Defn.h:337-340);
                // bcEval's LDCONST marks a constant that is not a simple
                // scalar not mutable (eval.c:7853), marked now (the mark is
                // permanent)
                // SAFETY: a live object.
                if self.profile == Profile::Bc && unsafe { plain_scalar_const(e) }.is_none() {
                    // SAFETY: a live object; REFCNT is a header field.
                    unsafe { mark_not_mutable(e) };
                }
                let mut i = Ins::new(Op::LdConst);
                i.a = dst;
                i.e = self.k(e);
                let mut i = self.at_depth(i, d, true);
                if tail {
                    i.fl |= F_VIS;
                }
                self.emit(i);
            }
        }
        Ok(())
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
        let off = *self.st.by_sym.get(&(f.0 as usize))?;
        match self.st.defs[off].kind {
            Kind::Special(s) => Some(Known::Special(s, off)),
            Kind::Builtin(_, Inline::None) => None,
            Kind::Builtin(_, inl) => Some(Known::Inline(inl, off)),
            Kind::Fn(_) => Some(Known::Fn(off)),
        }
    }

    /// A call. `e` is a LANGSXP.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    unsafe fn call(&mut self, e: Sexp, dst: Reg, d: u16, tail: bool) -> Result<(), Need> {
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
        let mut g = Ins::new(Op::Guard);
        g.e = self.k(f);
        g.c = self.k(prim);
        let g = self.at_depth(g, d, false);
        let guard = self.emit(g);
        let mark = self.next;
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
    /// behind a `BASEGUARD` ([`Op::BaseGuard`]) whose failure runs the
    /// generic call; anything else is the generic call.
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    // R: cmp.R:cmpCall, tryInline
    unsafe fn bc_call(&mut self, e: Sexp, dst: Reg, d: u16, tail: bool) -> Result<(), Need> {
        // SAFETY: the caller's contract: a live call.
        let (f, args) = unsafe { (e.car(), e.cdr()) };
        // SAFETY: as above; the compile environment's frames are live.
        let info = unsafe {
            if f.typ() == SYMSXP { self.bc.as_ref().unwrap().inline_info(self.cx, self.st, f, true) } else { None }
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
                Handler::Known(Known::Fn(off)) => {
                    let prim = self.st.objs[off].bind(self.cx).sexp();
                    let mut i = Ins::new(Op::SpecialFn);
                    i.a = dst;
                    i.e = self.k(e);
                    i.c = self.k(prim);
                    i.fl |= F_DEPTH;
                    i.d = d;
                    self.emit(i);
                }
                Handler::SimpleInternal { inner, internal } => {
                    self.builtin_args_call(Op::Internal, internal, inner, inner.cdr(), dst, d, tail)?;
                    // the expression bcEval is at: the wrapper's call
                    // (cmpSimpleInternal sets no location of its own)
                    let at = self.ins.len() - 1;
                    debug_assert_eq!(self.ins[at].op, Op::Internal);
                    self.ins[at].g = self.k(e);
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
            // SAFETY: as above.
            unsafe { self.generic_call(e, dst, d, false)? };
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
        // the primitives the VM inlines: cmp.R's handlers of the language
        // functions, `cmpSpecial`'s and `cmpBuiltin`'s for the others
        let off = st.by_sym.get(&(f.0 as usize)).copied().filter(|&o| !st.defs[o].internal());
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
        // `cmpBuiltin`: base's binding is a builtin (it declines for `...`
        // or an empty argument)
        if let Some(off) = off
            && st.defs[off].eval % 10 == 1
        {
            let prim = st.objs[off].bind(self.cx).sexp();
            // SAFETY: base's frame and a symbol.
            if unsafe { self.base_binding(f) } == Some(prim) {
                if args.iter().any(|&(a, _)| a == dots || a == missing) {
                    return Ok(None);
                }
                return Ok(Some(Handler::Builtin(prim)));
            }
        }
        Ok(None)
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
            Ok(Some(Handler::SimpleInternal { inner, internal: ifun.tag() }))
        }
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
        if !unsafe { function_context(self.cx, self.st, &mut cenv, formals, body)? } {
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
        Ok(Some(unsafe { crate::code::build(self.cx, &b, body, Some(formals))? }))
    }

    /// The generic call (`eval`'s LANGSXP case): the function found at run
    /// time, then promises, evaluated arguments or a compiled special.
    /// `tick`: this is the call's evaluation point (false behind a guard,
    /// which ticked).
    ///
    /// # Safety
    /// As for [`Comp::expr`].
    unsafe fn generic_call(&mut self, e: Sexp, dst: Reg, d: u16, tick: bool) -> Result<(), Need> {
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
            unsafe { self.expr(f, rf, d + 1, false)? };
        }
        let mut i = Ins::new(Op::Call);
        i.a = dst;
        i.b = rf;
        i.e = self.k(e);
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
    unsafe fn special(&mut self, s: Special, e: Sexp, args: Sexp, dst: Reg, d: u16, tail: bool) -> Result<(), Need> {
        let nil = self.nil();
        // SAFETY: the caller's contract.
        let a = unsafe { self.arglist(args) };
        let n = a.len();
        // SAFETY: the operands are live parts of the source.
        unsafe {
            match s {
                Special::Brace => {
                    // R: eval.c:do_begin
                    if n == 0 {
                        self.ld_nil(dst, tail);
                    }
                    for (i, &(st, _)) in a.iter().enumerate() {
                        if i + 1 == n {
                            self.expr(st, dst, d + 1, tail)?;
                        } else {
                            let r = self.alloc();
                            self.expr(st, r, d + 1, false)?;
                            self.clear(r);
                            self.free_to(r);
                        }
                    }
                }
                Special::If => {
                    // R: eval.c:do_if
                    if !(n == 2 || n == 3) {
                        return self.unimplemented(e, "an 'if' call that does not have 2 or 3 arguments");
                    }
                    let r = self.alloc();
                    self.expr(a[0].0, r, d + 1, false)?;
                    let mut br = Ins::new(Op::BrFalse);
                    br.b = r;
                    br.fl |= F_CLR_B;
                    br.e = self.k(e);
                    let br = self.emit(br);
                    self.free_to(r);
                    self.expr(a[1].0, dst, d + 1, tail)?;
                    let j = self.emit(Ins::new(Op::Jmp));
                    let els = self.pc();
                    self.patch(br, els);
                    if n == 3 {
                        self.expr(a[2].0, dst, d + 1, tail)?;
                    } else {
                        self.ld_nil(dst, false);
                        if tail {
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
                    self.expr(a[1].0, state, d + 1, false)?;
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
                    self.loop_body(a[2].0, d)?;
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
                    self.expr(a[0].0, r, d + 1, false)?;
                    let mut br = Ins::new(Op::BrFalse);
                    br.b = r;
                    br.fl |= F_CLR_B;
                    br.e = self.k(e);
                    let br = self.emit(br);
                    self.free_to(r);
                    self.loops.last_mut().unwrap().breaks.push(br);
                    self.loop_body(a[1].0, d)?;
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
                    self.loop_body(a[0].0, d)?;
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
                    // `eval` of the `return` call sets R_Visible, then
                    // `eval` of its operand does
                    let r = self.alloc();
                    if n == 1 {
                        self.expr(a[0].0, r, d + 1, true)?;
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
                    if n > 2 && a[2].0 != nil {
                        return self.unimplemented(e, "a 'function' with a srcref (keep.source = TRUE)");
                    }
                    let mut i = Ins::new(Op::MkClosure);
                    i.a = dst;
                    i.e = self.k(formals);
                    i.f = self.k(a[1].0);
                    if self.bc.is_some() {
                        // MAKECLOSURE: a closure of compiled code
                        if let Some(code) = self.nested_function(formals, a[1].0)? {
                            i.c = self.k(code);
                        }
                    } else if crate::jit::installed_literal(e) {
                        // a closure of base's installed byte code
                        i.g = 1;
                    }
                    if tail {
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
                            // installTrChar(STRING_ELT(lhs, 0)): a UTF-8
                            // session's translation is the bytes
                            let c = *(lhs.std_data() as *const Sexp);
                            self.cx.install(c.char_bytes())?.as_ref().sexp()
                        }
                        LANGSXP => {
                            // applydefine: the right-hand side first, then
                            // the assignment through `*tmp*`
                            self.expr(a[1].0, dst, d + 1, false)?;
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
                    self.expr(a[1].0, dst, d + 1, false)?;
                    let mut i = Ins::new(if s == Special::Assign2 { Op::SetVar2 } else { Op::SetVar });
                    i.a = dst;
                    i.e = self.k(sym);
                    if s != Special::Assign2 && !self.locals.contains(&sym) {
                        self.locals.push(sym);
                    }
                    self.emit(i);
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
                    if tail {
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
                        STRSXP if x.std_len() == 1 => {
                            let c = *(x.std_data() as *const Sexp);
                            self.cx.install(c.char_bytes())?.as_ref().sexp()
                        }
                        _ => return self.error_op(e, "invalid use of 'missing'", true),
                    };
                    let mut i = Ins::new(Op::Missing);
                    i.a = dst;
                    i.e = self.k(sym);
                    i.f = self.k(e);
                    if tail {
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
                    self.expr(a[0].0, dst, d + 1, false)?;
                    let mut i1 = Ins::new(Op::AndOr1);
                    i1.a = dst;
                    i1.b = dst;
                    i1.c = op;
                    i1.e = self.k(e);
                    let i1 = self.emit(i1);
                    let r = self.alloc();
                    self.expr(a[1].0, r, d + 1, false)?;
                    let mut i2 = Ins::new(Op::AndOr2);
                    i2.a = dst;
                    i2.b = r;
                    i2.c = op;
                    i2.e = self.k(e);
                    i2.fl |= F_CLR_B;
                    self.emit(i2);
                    self.free_to(r);
                    let end = self.pc();
                    self.patch(i1, end);
                    if tail {
                        self.set_vis(true);
                    }
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
                    let internal = fun.tag();
                    if internal == nil {
                        let name = String::from_utf8_lossy(fun.car().char_bytes()).into_owned();
                        let msg = format!("there is no .Internal function '{name}'");
                        return self.error_op(e, &msg, true);
                    }
                    if internal.typ() != BUILTINSXP {
                        // a special .Internal (`NextMethod`): do_internal
                        // calls it on the inner call, its arguments
                        // unevaluated, R_Visible on before
                        let off = crate::prims::offset(internal);
                        if !matches!(self.st.defs[off].kind, Kind::Fn(_)) {
                            return self.unimplemented(e, "a special .Internal");
                        }
                        let mut i = Ins::new(Op::SpecialFn);
                        i.a = dst;
                        i.e = self.k(inner);
                        i.c = self.k(internal);
                        i.fl |= F_DEPTH;
                        i.d = d;
                        self.emit(i);
                        return Ok(());
                    }
                    // the internal's arguments are evaluated as a builtin's
                    // (evalList), for the inner call
                    self.builtin_args_call(Op::Internal, internal, inner, inner.cdr(), dst, d, tail)?;
                    // the expression bcEval is at: the `.Internal` call (a
                    // closure wrapper's: error() reports the function's call)
                    let at = self.ins.len() - 1;
                    self.ins[at].g = self.k(e);
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

    /// A loop's body (its value discarded) and its back-edge.
    unsafe fn loop_body(&mut self, body: Sexp, d: u16) -> Result<(), Need> {
        let r = self.alloc();
        // SAFETY: the caller's contract.
        unsafe { self.expr(body, r, d + 1, false)? };
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
    fn loop_value(&mut self, dst: Reg, tail: bool) {
        self.ld_nil(dst, false);
        if tail {
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
        tail: bool,
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
            Inline::None => unreachable!(),
        };
        if !(plain && n_ok) {
            // evalList and the builtin itself (its arity errors)
            // SAFETY: as above.
            return unsafe { self.builtin_args_call(Op::CallBuiltin, prim, e, args, dst, d, tail) };
        }
        if inl == Inline::Paren {
            // SAFETY: as above.
            unsafe { self.expr(a[0].0, dst, d + 1, false)? };
            if tail {
                self.set_vis(true);
            }
            return Ok(());
        }
        let r = self.alloc_n(a.len());
        for (i, &(v, _)) in a.iter().enumerate() {
            // SAFETY: as above.
            unsafe { self.operand(v, r + i as Reg, d + 1)? };
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
        if tail {
            i.fl |= F_VIS;
        }
        self.emit(i);
        self.free_to(r);
        Ok(())
    }

    /// An operand of an inlined kernel: a plain numeric scalar constant is
    /// loaded unboxed (its identity is not observable there).
    unsafe fn operand(&mut self, v: Sexp, r: Reg, d: u16) -> Result<(), Need> {
        // SAFETY: the caller's contract.
        if let Some(_n) = unsafe { plain_scalar_const(v) } {
            // (an unboxed constant is no object: nothing is marked, as
            // neither `eval` nor LDCONST marks one)
            let mut i = Ins::new(Op::LdScalar);
            i.a = r;
            i.e = self.k(v);
            let i = self.at_depth(i, d, true);
            self.emit(i);
            return Ok(());
        }
        // SAFETY: as above.
        unsafe { self.expr(v, r, d, false) }
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
        tail: bool,
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
        if a.iter().any(|&(v, _)| v == dots || v == missing) {
            // evaluated at run time (`...` expanded, an empty argument's
            // error in order)
            i.c = u16::MAX;
            self.emit(i);
            return Ok(());
        }
        let named = a.iter().any(|&(_, t)| t != nil);
        let r = self.alloc_n(if named { 2 * n } else { n });
        for (j, &(v, _)) in a.iter().enumerate() {
            // SAFETY: as above.
            unsafe { self.expr(v, r + j as Reg, d + 1, false)? };
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
    // SAFETY: the caller's contract.
    unsafe {
        let t = x.typ();
        (matches!(t, REALSXP | INTSXP | LGLSXP) && !x.is_altrep() && x.std_len() == 1 && x.attrib_raw().typ() == NILSXP)
            .then_some(())
    }
}

/// `checkArity`'s message.
// R: errors.c:checkArityCall (via names.c's arity)
fn arity_msg(n: usize, name: &str, arity: usize) -> String {
    if n == 1 {
        format!("{n} argument passed to '{name}' which requires {arity}")
    } else {
        format!("{n} arguments passed to '{name}' which requires {arity}")
    }
}

/// `check1arg(args, call, formal)`: a tagged argument must partially match
/// `formal`.
// R: match.c:check1arg
unsafe fn check1arg(tag: Sexp, nil: Sexp, formal: &str) -> Option<String> {
    if tag == nil {
        return None;
    }
    // SAFETY: the caller's contract: a tag is a symbol.
    let supplied = unsafe { tag.car().char_bytes() };
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
