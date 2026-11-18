//! The VM: one loop over a code object's instructions, its registers in the
//! value stack (docs/g3.md §9.2).
//!
//! An activation ([`Act`]) is one run of one code object: the body of a
//! closure call, a promise's code, an expression `eval`uated. Its registers
//! are slots of a root scope it opens ([`robigo_rtcore::Scope::frame`]), so
//! everything it holds is a precise root, and nothing in a Rust local
//! survives a safepoint: after every instruction that may collect, the loop
//! reloads its view of the code object from the code register (the
//! contract of docs/g3.md §3 that lets a moving collector run, and that
//! deopt needs).
//!
//! Closure calls recurse in Rust (`call.rs`): a call's context record lives
//! in that Rust frame (C2's model), and the perf-vm ledger measured a
//! non-recursive VM call as no faster (`docs/ledger/perf-vm.md`, "VM-to-VM
//! calls").

#![allow(unsafe_code)]

use crate::code::View;
use crate::ir::*;
use crate::ops::{self, Num};

use robigo_heap::types::*;
use robigo_heap::{NeedGc, Safepoint, Sexp};
use robigo_rtbuiltins::kernel::relop::{EQOP, NEOP};
use robigo_rtcore::{Cx, Lgl, Out, RResult, Ref, Root, Slot, Tag, ctxt};
use std::ptr::NonNull;

/// An activation of a code object.
///
/// # Safety invariant
/// `base` points to `nregs` (at least [`R_FIRST`]) value-stack slots of this
/// isolate that stay live as long as the activation, and the code it runs
/// names no register at or past `nregs` ([`Act::new`]'s contract). The
/// compiler keeps that today; #738's verifier (C1, V3) is to prove it.
pub struct Act {
    /// The registers.
    base: NonNull<Slot>,
    /// How many there are.
    nregs: u32,
    /// Where the value goes: an empty object slot of the caller.
    out: NonNull<Slot>,
    /// `R_EvalDepth` when the activation began: an evaluation point of depth
    /// `d` runs at `depth + d`.
    depth: i32,
    /// The body of a closure call: `return` leaves it directly.
    body: bool,
    /// The value may go to `out` unboxed: the body of a closure that
    /// compiled code called, whose register takes it (`args::apply`).
    unboxed: bool,
    /// The value went to `out` unboxed ([`exec`]'s result).
    left_unboxed: bool,
    /// `Eval` code: GNU R's evaluator protections at the root's `eval`
    /// minus `depth` (robigo_rtcore's ppacct): an instruction calling out
    /// sets the excess to this plus its offset ([`View::ppk`]).
    e0: i32,
}

/// How a run of instructions ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// [`Op::Ret`]: the value is in the activation's out slot.
    Ret,
    /// [`Op::LoopEnd`]: a loop's region is done.
    RegionEnd,
    /// `return()` left the closure body: the value is in the out slot.
    Return,
}

impl Act {
    /// An activation of code with `nregs` registers at `base`, its value
    /// going to `out`.
    ///
    /// # Safety
    /// `base` points to `nregs` (at least [`R_FIRST`]) value-stack slots of
    /// this isolate, live and initialized for as long as the activation is
    /// used; the code run on it names no register at or past `nregs`; `out`
    /// is an empty value-stack slot of this isolate that stays live.
    #[inline(always)]
    unsafe fn new(
        base: NonNull<Slot>,
        nregs: usize,
        out: NonNull<Slot>,
        depth: i32,
        body: bool,
        unboxed: bool,
        e0: i32,
    ) -> Act {
        debug_assert!(
            nregs >= R_FIRST as usize && nregs <= u32::MAX as usize,
            "robigo: an activation of {nregs} registers"
        );
        Act { base, nregs: nregs as u32, out, depth, body, unboxed, left_unboxed: false, e0 }
    }

    /// The registers' first slot (the links walk's view of the frame).
    #[inline(always)]
    pub(crate) fn base(&self) -> NonNull<Slot> {
        self.base
    }

    /// The slot of register `r`. Its bound is the type's invariant: checked
    /// in debug builds, and in release builds by the code's construction.
    #[inline(always)]
    pub fn slot(&self, r: Reg) -> NonNull<Slot> {
        debug_assert!(u32::from(r) < self.nregs, "robigo: VM register {r} past the activation's {}", self.nregs);
        // SAFETY: `r` is below `nregs` (the invariant), so the slot is in
        // the activation's frame.
        unsafe { self.base.add(r as usize) }
    }

    /// The first slot of the `len` registers from `base`, a call's argument
    /// window. It may be empty: `base` is then at most `nregs`, the slot one
    /// past the last register, which nothing reads or writes.
    #[inline(always)]
    pub(crate) fn window(&self, base: Reg, len: usize) -> NonNull<Slot> {
        debug_assert!(
            u32::from(base) as usize + len <= self.nregs as usize,
            "robigo: VM registers {base}..+{len} past the activation's {}",
            self.nregs
        );
        // SAFETY: `base + len <= nregs` (the invariant), so the window is in
        // the activation's frame or, when empty, one past its end.
        unsafe { self.base.add(base as usize) }
    }

    #[inline(always)]
    pub fn get(&self, r: Reg) -> Slot {
        // SAFETY: a live slot of this activation.
        unsafe { self.slot(r).read() }
    }

    /// Write register `r`.
    ///
    /// # Safety
    /// An object written is a live object of this isolate.
    #[inline(always)]
    pub unsafe fn set(&self, r: Reg, s: Slot) {
        // SAFETY: a live slot of this activation; the caller's contract.
        unsafe { self.slot(r).write(s) }
    }

    #[inline(always)]
    pub fn clear(&self, r: Reg) {
        // SAFETY: EMPTY holds no object.
        unsafe { self.set(r, Slot::EMPTY) }
    }

    /// The object in register `r`. Panics if it holds an unboxed value.
    #[inline(always)]
    pub fn obj(&self, r: Reg) -> Sexp {
        match self.get(r).as_object() {
            Some(x) => x,
            None => not_an_object(r),
        }
    }

    /// Register `r` as a root, for as long as the activation is borrowed.
    #[inline(always)]
    pub fn root(&self, r: Reg) -> Root<'_> {
        // SAFETY: a live slot of this activation, which outlives the borrow.
        unsafe { Root::from_raw(self.slot(r)) }
    }

    /// Register `r`, emptied, as the `Out` of a callee, for as long as the
    /// activation is borrowed.
    #[inline(always)]
    pub fn out(&self, r: Reg) -> Out<'_> {
        self.clear(r);
        // SAFETY: as in `root`; the slot is empty.
        unsafe { Out::from_raw(self.slot(r)) }
    }

    /// The environment.
    #[inline(always)]
    pub fn rho(&self) -> Sexp {
        self.obj(R_ENV)
    }

    /// Put `x` in the scratch register `r` (for a root argument of the
    /// raw layer's callee).
    ///
    /// # Safety
    /// `x` is a live object of this isolate.
    #[inline(always)]
    pub unsafe fn scratch(&self, r: Reg, x: Sexp) -> Root<'_> {
        // SAFETY: the caller's contract.
        unsafe { self.set(r, Slot::object(x)) };
        self.root(r)
    }
}

#[cold]
#[inline(never)]
fn not_an_object(r: Reg) -> ! {
    panic!("robigo: VM register {r} holds an unboxed value where an object is needed")
}

/// Run `code` (rooted) in `rho` (rooted), its value written to the empty
/// slot `out`. `body`: the body of a closure call; `unboxed`: the value may
/// go to `out` unboxed ([`Act::unboxed`]). True when it did.
///
/// # Safety
/// `code` is a live code object and `rho` a live environment, both rooted
/// by the caller for the call; `out` is an empty value-stack slot of this
/// isolate that stays live.
pub unsafe fn exec(cx: &mut Cx, code: Sexp, rho: Sexp, out: NonNull<Slot>, body: bool, unboxed: bool) -> RResult<bool> {
    // (`Ret` leaves a scalar unboxed on `unboxed` alone: only a closure's
    // body may have it)
    debug_assert!(body || !unboxed, "robigo: an unboxed result for code that is not a closure's body");
    // SAFETY: the caller's contract.
    let v = unsafe { View::of(code) };
    let entry = cx.eval_depth();
    cx.check_value_stack(crate::links::FRAME_HEAD + v.nregs)?;
    // an `Eval` activation's protections at its root: the caller's excess,
    // and a closure's `newrho` (applyClosure_core's PROTECT); a promise's
    // PROTECT(e) is forcePromise's (`Cx::eval_promise_code`). Byte code
    // keeps the excess: its `eval` nests a level for that protection.
    let excess = cx.pp_excess();
    let r = cx.scope(|cx, s| {
        // the registers, headed by the marks that put them on the node
        // stack while the activation runs (`crate::links`)
        let frame = s.frame(cx, crate::links::FRAME_HEAD + v.nregs);
        // SAFETY: fresh empty slots of this scope; the registers follow the
        // head.
        let base = unsafe {
            crate::links::mark_frame(frame, v.nregs);
            frame.add(crate::links::FRAME_HEAD)
        };
        // SAFETY: `v.nregs` fresh slots of this scope follow the head, live
        // until the scope ends with the activation; the compiler names only
        // registers below `v.nregs`; `out` as the caller's contract says.
        let mut act = unsafe { Act::new(base, v.nregs, out, entry, body, unboxed, excess + body as i32) };
        // SAFETY: `rho` and `code` are live and rooted by the caller, and
        // nothing collected since they were read (no safepoint between):
        // from here on the registers hold them.
        unsafe {
            act.set(R_ENV, Slot::object(rho));
            act.set(R_CODE, Slot::object(code));
        }
        if v.profile == Profile::Bc {
            // R: eval.c:eval (the BCODESXP case's INCREMENT_EVAL_DEPTH), and
            // bcEval's entry poll (eval.c:7539)
            act.depth = entry + 1;
            cx.eval_depth_at(act.depth)?;
            crate::eval::bc_tick(cx)?;
        } else {
            cx.check_c_stack()?;
        }
        cx.safepoint(Safepoint::Entry)?;
        // R_BCIntActive: set by bcEval, cleared by eval
        let flow = cx.bc_activation(v.profile == Profile::Bc, |cx| run(cx, &mut act, 0))?;
        if flow == Flow::Return && v.profile == Profile::Eval {
            // R: eval.c:do_return, findcontext's longjmp to the function's
            // record (the innermost now) restores R_PPStackTop
            cx.pp_restore_innermost();
        }
        Ok(act.left_unboxed)
    });
    cx.set_eval_depth(entry);
    if v.profile == Profile::Eval {
        cx.set_pp_excess(excess);
    }
    r
}

/// An `Eval` instruction's call out (`pc`, its index): the protect
/// excess where GNU R's AST runs code there (robigo_rtcore's ppacct). Byte
/// code keeps the excess its activation started with.
#[inline(always)]
fn pp_point(cx: &Cx, act: &Act, v: &View, pc: usize) {
    if v.profile == Profile::Eval {
        // SAFETY: `pc` is an instruction of the live `Eval` code.
        cx.set_pp_excess(act.e0 + unsafe { v.ppk(pc) });
    }
}

/// Whether `s` holds an unboxed scalar (a double, integer or logical).
#[inline(always)]
fn is_unboxed(s: Slot) -> bool {
    matches!(s.tag(), Tag::Real | Tag::Int | Tag::Logical)
}

/// Box register `r` if it holds an unboxed scalar (the box is written back,
/// so later reads see the one object); its object.
pub fn boxed(cx: &mut Cx, act: &Act, r: Reg) -> RResult<Sexp> {
    let s = act.get(r);
    if let Some(x) = s.as_object() {
        return Ok(x);
    }
    let x = cx.retry_alloc(|cx| box_slot(cx, s))?;
    // SAFETY: fresh, of this isolate; nothing collected since.
    unsafe { act.set(r, Slot::object(x)) };
    Ok(x)
}

/// A new length-1 vector holding the unboxed `s` (an object slot's object
/// itself, not read).
pub fn box_slot(cx: &Cx, s: Slot) -> Result<Sexp, NeedGc> {
    Ok(match s.tag() {
        Tag::Real => cx.alloc_with(1, |_| s.as_real().unwrap())?.finish().as_ref().sexp(),
        Tag::Int => cx.alloc_with(1, |_| s.as_int().unwrap())?.finish().as_ref().sexp(),
        Tag::Logical => cx.alloc_with(1, |_| Lgl(s.as_int().unwrap()))?.finish().as_ref().sexp(),
        Tag::Object => s.as_object().unwrap(),
        Tag::Empty => cx.nil().sexp(),
    })
}

/// A register's value as a scalar number, if it is one the kernels take
/// (an unboxed scalar, or a plain length-1 vector without attributes).
///
/// # Safety
/// If `s` holds an object, that object is live while `num` reads its
/// header and payload: it was live when `s` was read, and nothing has
/// collected since. That rules out an object pointer kept unrooted across
/// anything that can collect, since a collection may free or move it. Two
/// kinds of caller meet the contract today:
/// - the VM, reading a register of the running activation (rooted);
/// - the inline runs (`inline::pure`, `body_loop`, `fused`) and early
///   evaluation (`early.rs`), reading their local register arrays during a
///   run, while nothing collects.
///
/// Any other caller has to show the same: the object is live and has not
/// been through a collection since the slot was read.
#[inline(always)]
pub unsafe fn num(cx: &Cx, s: Slot) -> Option<Num> {
    match s.tag() {
        Tag::Real => Some(Num::Real(s.as_real().unwrap())),
        Tag::Int => Some(Num::Int(s.as_int().unwrap())),
        Tag::Logical => Some(Num::Lgl(s.as_int().unwrap())),
        Tag::Object => {
            let x = s.as_object().unwrap();
            // SAFETY: the caller's contract, the object is live while it is
            // read; a vector that is not ALTREP has its length and payload in
            // its own words.
            unsafe {
                // a plain vector, read from its words (`ops::plain_num`'s
                // answer without its view)
                let t = x.typ();
                if matches!(t, REALSXP | INTSXP | LGLSXP) && !x.is_altrep() {
                    if x.std_len() != 1 || x.attrib_raw() != cx.nil().sexp() {
                        return None;
                    }
                    let p = x.std_data();
                    return Some(match t {
                        REALSXP => Num::Real(*(p as *const f64)),
                        INTSXP => Num::Int(*(p as *const i32)),
                        _ => Num::Lgl(*(p as *const i32)),
                    });
                }
                // anything else (not a real, integer or logical vector, or
                // ALTREP: no instance of robigo's classes has length 1, and
                // a package class's is not standard storage) is not a plain
                // scalar, `ops::plain_num`'s None
                None
            }
        }
        Tag::Empty => None,
    }
}

#[inline(always)]
pub(crate) fn num_slot(v: Num) -> Slot {
    match v {
        Num::Real(x) => Slot::real(x),
        Num::Int(x) => Slot::int(x),
        Num::Lgl(x) => Slot::logical(x),
    }
}

/// Run `act`'s code from `pc` until it returns or its region ends.
pub(crate) fn run(cx: &mut Cx, act: &mut Act, mut pc: usize) -> RResult<Flow> {
    // SAFETY: the code register holds the activation's live code object.
    let mut v = unsafe { View::of(act.obj(R_CODE)) };
    // an evaluation point of depth d errs when depth + d > R_Expressions
    let mut slack = cx.expressions() - act.depth;
    macro_rules! reload {
        () => {{
            // SAFETY: as above: the code register is rooted, and its object
            // may have moved.
            v = unsafe { View::of(act.obj(R_CODE)) };
            slack = cx.expressions() - act.depth;
        }};
    }
    loop {
        // SAFETY: the compiler ends every path in Ret, Return or LoopEnd,
        // and its jump targets are instructions of the code.
        let i = unsafe { v.at(pc) };
        pc += 1;
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            if i.fl & F_TICK != 0 && crate::eval::tick(cx)? {
                reload!();
            }
            if i.fl & F_DEPTH != 0 && i.d as i32 > slack {
                // R: eval.c:INCREMENT_EVAL_DEPTH
                cx.eval_depth_at(act.depth + i.d as i32)?;
            }
        }
        match i.op {
            Op::Nop => {}
            Op::LdConst => {
                // SAFETY: a constant of the live code object.
                unsafe { act.set(i.a, Slot::object(v.k(i.e))) };
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
            }
            Op::LdScalar => {
                // SAFETY: the compiler loads plain scalar constants only, with
                // their payloads inline (`compile::plain_scalar_const`).
                let s = unsafe {
                    let k = v.k(i.e);
                    let d = k.inline_data();
                    match k.typ() {
                        REALSXP => Slot::real(*(d as *const f64)),
                        INTSXP => Slot::int(*(d as *const i32)),
                        _ => Slot::logical(*(d as *const i32)),
                    }
                };
                // SAFETY: unboxed values hold no object.
                unsafe { act.set(i.a, s) };
            }
            Op::GetVar => {
                // SAFETY: the symbol is a constant of the code.
                let sym = unsafe { v.k(i.e) };
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                if !unsafe { crate::eval::get_var_fast(cx, act, i.a, sym, i.f, v.shape) } {
                    cx.set_eval_depth(act.depth + i.d as i32);
                    pp_point(cx, act, &v, pc - 1);
                    // a formal's first read: its slot (the index compiled
                    // in) forced in place, as `get_var` would find and force
                    // it, without the lookup
                    let at = i.f as usize;
                    let thunk = at != 0 && {
                        // SAFETY: the activation's rooted environment and the
                        // code's shape, for which the index was compiled.
                        let b = unsafe { cx.frame_slot_raw(act.rho(), v.shape, at - 1) };
                        matches!(b, Some(robigo_rtcore::RawBinding::Thunk(_)))
                    };
                    if thunk {
                        cx.force_arg(act.root(R_ENV), at - 1, act.out(i.a))?;
                    } else {
                        // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                        unsafe { crate::eval::get_var(cx, act, i.a, sym, false, v.profile == Profile::Bc) }?;
                    }
                    reload!();
                }
            }
            Op::GetDdVar => {
                // SAFETY: as above.
                let sym = unsafe { v.k(i.e) };
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
                cx.set_eval_depth(act.depth + i.d as i32);
                pp_point(cx, act, &v, pc - 1);
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                unsafe { crate::eval::get_var(cx, act, i.a, sym, true, v.profile == Profile::Bc) }?;
                reload!();
            }
            Op::SetVar | Op::SetVar2 => {
                // SAFETY: as above.
                let sym = unsafe { v.k(i.e) };
                set_var(cx, act, i.a, sym, i.op == Op::SetVar2, i.f, v.shape)?;
                reload!();
            }
            Op::Jmp => pc = i.target(),
            Op::StartSubset => {
                // SAFETY: a constant of the live code object.
                let call = unsafe { v.k(i.e) };
                let done = start_subset(cx, act, i.a, i.b, call, i.c == 2)?;
                reload!();
                if done {
                    pc = i.target();
                }
            }
            Op::DfltSubset => {
                // SAFETY: as above.
                let call = unsafe { v.k(i.e) };
                dflt_subset(cx, act, i.a, call, i.b, i.c as usize, i.fl & F_NAMES != 0, i.g == 2)?;
                reload!();
            }
            Op::Dollar => {
                // SAFETY: as above.
                let (call, sym) = unsafe { (v.k(i.e), v.k(i.c)) };
                dollar(cx, act, i.a, i.b, call, sym)?;
                reload!();
            }
            Op::GetVarMissOk => {
                // SAFETY: as above.
                let sym = unsafe { v.k(i.e) };
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                unsafe { crate::eval::get_var_missok(cx, act, i.a, sym, v.profile == Profile::Bc) }?;
                reload!();
            }
            Op::Switch => {
                // SAFETY: constants of the live code object.
                let (call, names) = unsafe { (v.k(i.e), v.k(i.c)) };
                let at = switch_entry(cx, act, i.b, call, names, i.f as usize);
                act.clear(i.b);
                reload!();
                pc += at?;
            }
            Op::BrFalse => {
                let s = act.get(i.b);
                // SAFETY: as above.
                let call = unsafe { v.k(i.e) };
                let c = match s.tag() {
                    Tag::Logical if s.as_int() != Some(ops::NA_LOGICAL) => s.as_int().unwrap() != 0,
                    _ => {
                        let c = as_logical_no_na(cx, act, i.b, call)?;
                        reload!();
                        c
                    }
                };
                if i.fl & F_CLR_B != 0 {
                    act.clear(i.b);
                }
                if !c {
                    pc = i.target();
                }
            }
            Op::Guard => {
                // SAFETY: constants of the code; the environment is rooted.
                let ok = unsafe {
                    let sym = v.k(i.e);
                    let want = v.k(i.c);
                    cx.fun_lookup_raw(sym, act.rho()) == Some(want)
                };
                if !ok {
                    pc = i.target();
                } else if i.a != 0 {
                    // a check site's `save` (eval's, after PROTECT(op))
                    // SAFETY: an unboxed value.
                    unsafe { act.set(i.a, Slot::int(cx.pp_top())) };
                }
            }
            Op::GetFun => {
                // SAFETY: as above.
                let (sym, call) = unsafe { (v.k(i.e), v.k(i.c)) };
                // (`f` 1: the code's shape lacks the name, so a frame of that
                // shape binding nothing else is passed over)
                let shape = if i.f != 0 { v.shape } else { cx.nil().sexp() };
                // SAFETY: as above; `shape` (if not R_NilValue) lacks `sym`.
                match unsafe { cx.fun_lookup_skip_raw(sym, act.rho(), shape) } {
                    // SAFETY: a function bound in a live frame.
                    Some(f) => unsafe { act.set(i.a, Slot::object(f)) },
                    None => {
                        cx.set_eval_depth(act.depth + i.d as i32);
                        pp_point(cx, act, &v, pc - 1);
                        // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                        unsafe { crate::call::find_fun(cx, act, i.a, sym, call) }?;
                        reload!();
                    }
                }
            }
            Op::Call => {
                // SAFETY: as above.
                let call = unsafe { v.k(i.e) };
                cx.set_eval_depth(act.depth + i.d as i32);
                pp_point(cx, act, &v, pc - 1);
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                unsafe { crate::call::call(cx, act, i.a, i.b, call, i.c, v.profile == Profile::Bc) }?;
                if i.fl & F_CLR_B != 0 && i.b != i.a {
                    act.clear(i.b);
                }
                reload!();
            }
            Op::CallCodes => {
                // `Call` with the call's promise codes (`consts[f]`), which
                // the promises it makes hold (translated code: `Bc`)
                // SAFETY: a constant of the code.
                let call = unsafe { v.k(i.e) };
                cx.set_eval_depth(act.depth + i.d as i32);
                pp_point(cx, act, &v, pc - 1);
                // (the codes by their pool index `f`: read where used)
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                unsafe { crate::call::call_codes(cx, act, i.a, i.b, call, i.f, i.c) }?;
                if i.fl & F_CLR_B != 0 && i.b != i.a {
                    act.clear(i.b);
                }
                reload!();
            }
            Op::SpecialFn => {
                // SAFETY: constants of the code.
                let (call, prim) = unsafe { (v.k(i.e), v.k(i.c)) };
                cx.set_eval_depth(act.depth + i.d as i32);
                pp_point(cx, act, &v, pc - 1);
                if i.g == 1 {
                    // R: names.c:do_internal (save, PROTECT(args), check);
                    // the name read now (the call may move the primitive)
                    // SAFETY: a primitive constant of the code.
                    let name = unsafe { crate::prims::def(cx, prim) }.name;
                    let save = cx.pp_save(0);
                    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                    cx.with_pp_excess(1, |cx| unsafe { crate::apply::special_fn(cx, act, i.a, prim, call) })?;
                    cx.check_stack_balance(name.as_bytes(), save);
                } else {
                    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                    unsafe { crate::apply::special_fn(cx, act, i.a, prim, call) }?;
                }
                reload!();
            }
            Op::BcAssign => {
                // SAFETY: a constant of the code.
                let plan = unsafe { v.k(i.e) };
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                unsafe { crate::bcassign::run(cx, act, i.a, i.b, plan, i.g == 1) }?;
                reload!();
            }
            Op::BaseGuard => {
                // SAFETY: a constant of the code.
                let call = unsafe { v.k(i.e) };
                if !base_guard(cx, act, i.a, call)? {
                    pc = i.target();
                }
                reload!();
            }
            Op::TopLoop => {
                if crate::jit::compiles_top_loop(cx, act.root(R_ENV).bind(cx)) {
                    // SAFETY: a constant of the code.
                    let call = unsafe { v.k(i.e) };
                    pp_point(cx, act, &v, pc - 1);
                    if top_loop(cx, act, i.a, call, act.depth + i.d as i32)? {
                        pc = i.target();
                    }
                    reload!();
                }
            }
            Op::ComplexAssign => {
                // SAFETY: a constant of the code.
                let call = unsafe { v.k(i.e) };
                cx.set_eval_depth(act.depth + i.d as i32);
                pp_point(cx, act, &v, pc - 1);
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                unsafe { crate::assign::applydefine(cx, act, i.a, i.b, call, i.g) }?;
                reload!();
            }
            Op::CallBuiltin | Op::Internal => {
                // SAFETY: as above.
                let (call, fun) = unsafe { (v.k(i.e), v.k(i.f)) };
                cx.set_eval_depth(act.depth + i.d as i32);
                if v.profile == Profile::Bc {
                    // the expression bcEval is at, for error() and
                    // warning() (R_getBCInterpreterExpression)
                    // SAFETY: a constant of the rooted code object.
                    unsafe { cx.set_bc_expression(if i.g != 0 { v.k(i.g) } else { call }) };
                } else {
                    pp_point(cx, act, &v, pc - 1);
                }
                let r = if i.c == crate::ir::SPECIAL_INTERNAL {
                    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                    unsafe { crate::call::call_special_internal(cx, act, i.a, fun, call) }
                } else if i.c == u16::MAX {
                    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                    unsafe { crate::call::call_builtin_list(cx, act, i.a, fun, call, v.profile == Profile::Bc) }
                } else {
                    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                    unsafe {
                        crate::call::call_builtin_regs(cx, act, i.a, fun, call, i.b, i.c as usize, i.fl & F_NAMES != 0)
                    }
                };
                if v.profile == Profile::Bc {
                    // SAFETY: R_NilValue.
                    unsafe { cx.set_bc_expression(cx.nil().sexp()) };
                }
                r?;
                reload!();
            }
            Op::Arith => {
                // SAFETY: registers of the running activation: their objects are live, rooted.
                let (x, y) = (unsafe { num(cx, act.get(i.b)) }, unsafe { num(cx, act.get(i.c)) });
                let fast = match (x, y) {
                    (Some(x), Some(y)) => match ops::arith2(i.g as u8, x, y, v.profile == Profile::Bc) {
                        (r, false) => Some(r),
                        // an overflow's or a lost modulus's warning is the
                        // builtin's to signal
                        (_, true) => None,
                    },
                    _ => None,
                };
                match fast {
                    Some(r) => {
                        // SAFETY: an unboxed value.
                        unsafe { act.set(i.a, num_slot(r)) };
                        clear_operands(act, &i);
                        if i.fl & F_VIS != 0 {
                            cx.set_visible(true);
                        }
                    }
                    None => {
                        slow_builtin(cx, act, &v, &i, pc - 1, 2)?;
                        reload!();
                    }
                }
            }
            // SAFETY: registers of the running activation: their objects are live, rooted.
            Op::Arith1 => match unsafe { num(cx, act.get(i.b)) } {
                Some(x) => {
                    // SAFETY: an unboxed value.
                    unsafe { act.set(i.a, num_slot(ops::arith1(i.g as u8, x))) };
                    clear_operands(act, &i);
                    if i.fl & F_VIS != 0 {
                        cx.set_visible(true);
                    }
                }
                None => {
                    slow_builtin(cx, act, &v, &i, pc - 1, 1)?;
                    reload!();
                }
            },
            Op::Relop => {
                // SAFETY: registers of the running activation: their objects are live, rooted.
                let (x, y) = (unsafe { num(cx, act.get(i.b)) }, unsafe { num(cx, act.get(i.c)) });
                let fast = match (x, y) {
                    (Some(x), Some(y)) => Some(ops::relop2(i.g as u8, x, y)),
                    _ if matches!(i.g as u8, EQOP | NEOP) => {
                        match (act.get(i.b).as_object(), act.get(i.c).as_object()) {
                            // SAFETY: registers' objects are live.
                            (Some(a), Some(b)) => unsafe {
                                ops::str_eq(cx, Ref::from_sexp(a, cx), Ref::from_sexp(b, cx)).map(|e| match e {
                                    None => ops::NA_LOGICAL,
                                    Some(eq) => (eq == (i.g as u8 == EQOP)) as i32,
                                })
                            },
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match fast {
                    Some(r) => {
                        // SAFETY: an unboxed value.
                        unsafe { act.set(i.a, Slot::logical(r)) };
                        clear_operands(act, &i);
                        if i.fl & F_VIS != 0 {
                            cx.set_visible(true);
                        }
                    }
                    None => {
                        slow_builtin(cx, act, &v, &i, pc - 1, 2)?;
                        reload!();
                    }
                }
            }
            // SAFETY: registers of the running activation: their objects are live, rooted.
            Op::Not => match unsafe { num(cx, act.get(i.b)) } {
                Some(x) => {
                    // SAFETY: an unboxed value.
                    unsafe { act.set(i.a, Slot::logical(ops::not1(x))) };
                    clear_operands(act, &i);
                    if i.fl & F_VIS != 0 {
                        cx.set_visible(true);
                    }
                }
                None => {
                    slow_builtin(cx, act, &v, &i, pc - 1, 1)?;
                    reload!();
                }
            },
            // SAFETY: registers of the running activation: their objects are live, rooted.
            Op::Logic => match (unsafe { num(cx, act.get(i.b)) }, unsafe { num(cx, act.get(i.c)) }) {
                (Some(x), Some(y)) => {
                    let r = ops::logic2(i.g, x, y);
                    // SAFETY: an unboxed value.
                    unsafe { act.set(i.a, Slot::logical(r)) };
                    clear_operands(act, &i);
                    if i.fl & F_VIS != 0 {
                        cx.set_visible(true);
                    }
                }
                _ => {
                    slow_builtin(cx, act, &v, &i, pc - 1, 2)?;
                    reload!();
                }
            },
            Op::Math1 => {
                // (out of line: the arm leaves vm::run's own code as it was)
                math1(cx, act, &v, &i, pc - 1)?;
                reload!();
            }
            Op::AndOr1 | Op::AndOr2 => {
                // SAFETY: as above.
                let call = unsafe { v.k(i.e) };
                let x = and_or_operand(cx, act, i.b, call, i.c, i.op == Op::AndOr1)?;
                let r = if i.op == Op::AndOr1 {
                    let decided = if i.c == 1 { x == 0 } else { x == 1 };
                    if decided {
                        pc = i.target();
                    }
                    x
                } else {
                    let x1 = act.get(i.a).as_int().unwrap();
                    // R: logic.c:do_logic2
                    if i.c == 1 {
                        if x1 == ops::NA_LOGICAL {
                            if x == ops::NA_LOGICAL || x != 0 { ops::NA_LOGICAL } else { x }
                        } else {
                            x
                        }
                    } else if x1 == ops::NA_LOGICAL {
                        if x == ops::NA_LOGICAL || x == 0 { ops::NA_LOGICAL } else { x }
                    } else {
                        x
                    }
                };
                if i.op == Op::AndOr2 && i.fl & F_CLR_B != 0 {
                    act.clear(i.b);
                }
                // SAFETY: an unboxed value.
                unsafe { act.set(i.a, Slot::logical(r)) };
            }
            Op::MkClosure => {
                mk_closure(cx, act, i.a, i.e, i.f, i.c, i.g == 1)?;
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
                reload!();
            }
            Op::Quote => {
                // SAFETY: a constant of the code: MARK_NOT_MUTABLE, then the
                // value.
                unsafe {
                    let x = v.k(i.e);
                    if !x.has_flag(robigo_heap::flags::IMMORTAL) {
                        x.set_refcnt(u16::MAX);
                    }
                    act.set(i.a, Slot::object(x));
                }
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
            }
            Op::Missing => {
                // SAFETY: as above.
                let sym = unsafe { v.k(i.e) };
                // SAFETY: the running code's constants (or rooted values): live, of this isolate.
                let m = unsafe { crate::missing::r_missing(cx, act, sym) }?;
                // SAFETY: an unboxed value.
                unsafe { act.set(i.a, Slot::logical(m as i32)) };
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
                reload!();
            }
            Op::Return => {
                if act.body && act.unboxed && is_unboxed(act.get(i.b)) {
                    // SAFETY: an unboxed value into the empty out slot, whose
                    // reader takes it so (`Act::unboxed`).
                    unsafe { act.out.write(act.get(i.b)) };
                    act.left_unboxed = true;
                    act.clear(i.b);
                    return Ok(Flow::Return);
                }
                let x = boxed(cx, act, i.b)?;
                if act.body {
                    // SAFETY: a live object (rooted in register b) into the
                    // empty out slot.
                    unsafe { act.out.write(Slot::object(x)) };
                    act.clear(i.b);
                    return Ok(Flow::Return);
                }
                // R: eval.c:do_return, findcontext(CTXT_BROWSER | CTXT_FUNCTION, rho, v)
                return Err(cx.find_context(ctxt::BROWSER | ctxt::FUNCTION, act.root(R_ENV), Some(act.root(i.b))));
            }
            Op::Ret => {
                let r = act.get(i.b);
                if act.unboxed && is_unboxed(r) {
                    // SAFETY: as in `Return`.
                    unsafe { act.out.write(r) };
                    act.left_unboxed = true;
                } else {
                    let x = boxed(cx, act, i.b)?;
                    // SAFETY: as above.
                    unsafe { act.out.write(Slot::object(x)) };
                }
                act.clear(i.b);
                match i.g {
                    1 => cx.set_visible(true),
                    2 => cx.set_visible(false),
                    _ => {}
                }
                return Ok(Flow::Ret);
            }
            Op::LoopEnd => return Ok(Flow::RegionEnd),
            Op::Loop => {
                let end = i.target();
                match crate::call::loop_region(cx, act, pc, i.a, v.nregs as Reg, v.profile)? {
                    Flow::Return => return Ok(Flow::Return),
                    _ => pc = end + 1,
                }
                reload!();
            }
            Op::ForPrep => {
                // SAFETY: as above.
                let (sym, call) = unsafe { (v.k(i.e), v.k(i.c)) };
                for_prep(cx, act, i.a, sym, call)?;
                reload!();
            }
            Op::ForStep => {
                // SAFETY: as above.
                let (sym, call) = unsafe { (v.k(i.e), v.k(i.c)) };
                if !for_step(cx, act, i.a, sym, call, v.profile)? {
                    pc = i.target();
                }
                reload!();
            }
            Op::ForDone => {
                // R: eval.c:do_for, DECREMENT_LINKS(val): the sequence's
                // (a compact one's is in `a + 5`, its buffer in `a`)
                let seq = match act.get(i.a + 5).as_object() {
                    // SAFETY: a live object.
                    Some(s) if unsafe { s.typ() } != NILSXP => Some(s),
                    _ => act.get(i.a).as_object(),
                };
                if let Some(x) = seq {
                    // SAFETY: a live object.
                    unsafe { x.decref() };
                }
            }
            Op::BackJmp => {
                // a VM back-edge: a safepoint (docs/g3.md §3 item 2), and
                // bcEval's back-branch poll in the `Bc` profile
                cx.safepoint(Safepoint::Loop)?;
                if v.profile == Profile::Bc {
                    crate::eval::bc_tick(cx)?;
                }
                pc = i.target();
                reload!();
            }
            Op::Resume | Op::LoopResume => {
                // the check inline, the deopt out of line
                // SAFETY: the code register holds the running code object.
                if !cx.code_is_current(unsafe { crate::code::stamp(act.obj(R_CODE)) }, true, false) {
                    pp_point(cx, act, &v, pc - 1);
                    pc = boundary(cx, act, i)?;
                    reload!();
                }
            }
            Op::BreakJump => {
                // R: eval.c:do_break, findcontext(PRIMVAL(op), rho, R_NilValue)
                let mask = if i.g == 2 { ctxt::BREAK } else { ctxt::NEXT };
                return Err(cx.find_context(mask, act.root(R_ENV), None));
            }
            Op::Clear => act.clear(i.a),
            Op::ClearRange => {
                for r in i.a..i.b {
                    act.clear(r);
                }
            }
            Op::Move => {
                // SAFETY: a copy of a register of this activation.
                unsafe { act.set(i.a, act.get(i.b)) };
            }
            Op::SetVis => cx.set_visible(i.g != 0),
            Op::EvalConst => {
                // SAFETY: a constant of the rooted code.
                let e = unsafe { act.scratch(R_SCRATCH, v.k(i.e)) };
                // (byte code's `eval` runs one level below bcEval's)
                let base = if v.profile == Profile::Bc { act.depth } else { act.depth + i.d as i32 - 1 };
                cx.set_eval_depth(base);
                pp_point(cx, act, &v, pc - 1);
                let r = crate::eval::eval(cx, e, act.root(R_ENV), act.out(i.a));
                act.clear(R_SCRATCH);
                r?;
                reload!();
            }
            Op::PpCheck => {
                let saved = act.get(i.b).as_int().unwrap();
                if cx.pp_top() != saved {
                    // SAFETY: a primitive constant of the `Eval` code, and
                    // its instruction's offset.
                    unsafe { pp_imbalance(cx, act, v.k(i.e), v.ppk(pc - 1), saved) };
                }
            }
            Op::PpRestore => cx.pp_restore_innermost(),
            // SAFETY: an unboxed value.
            Op::PpSave => unsafe { act.set(i.a, Slot::int(cx.pp_top())) },
            Op::ErrorCall => {
                // SAFETY: constants of the code.
                let (call, msg) = unsafe { (v.k(i.e), v.k(i.f)) };
                // SAFETY: a CHARSXP of the code.
                let msg = String::from_utf8_lossy(unsafe { msg.char_bytes() }).into_owned();
                return Err(if i.g != 0 {
                    // SAFETY: the call is a live constant.
                    let c = unsafe { act.scratch(R_SCRATCH, call) };
                    cx.error_call(Some(c), &msg)
                } else {
                    cx.error(&msg)
                });
            }
        }
    }
}

/// [`Op::PpCheck`]'s warning: the primitive `prim`'s site saved C's top
/// `cpp`, `off` protections above the activation's root.
// R: eval.c:check_stack_balance
#[cold]
#[inline(never)]
fn pp_imbalance(cx: &Cx, act: &Act, prim: Sexp, off: i32, cpp: i32) {
    // SAFETY: a primitive of this isolate.
    let name = unsafe { crate::prims::def(cx, prim) }.name;
    let save = act.depth + act.e0 + off + cpp;
    cx.check_stack_balance(name.as_bytes(), robigo_rtcore::PpSave { save, cpp });
}

/// Clear the operand registers an instruction reads for the last time.
#[inline(always)]
fn clear_operands(act: &Act, i: &Ins) {
    if i.fl & F_CLR_B != 0 && i.b != i.a {
        act.clear(i.b);
    }
    if i.fl & F_CLR_C != 0 && i.c != i.a {
        act.clear(i.c);
    }
}

/// bcEval's scalar math ([`Op::Math1`]) of kind `kind` on the operands
/// `x` (and `y`), read as bcStackScalar reads them: the value and whether
/// it warns "NaNs produced"; None for the builtin. `SQRT` and `EXP` take a
/// double or a non-NA integer; `MATH1`, `LOG` and `LOGBASE` a double or an
/// integer as a double, NA too (bcStackScalarReal). A NaN of a NaN operand
/// is that operand; of others it warns.
// R: eval.c:FastMath1, DO_MATH1, DO_LOG, DO_LOGBASE, bcStackScalarReal
fn math1_fast(kind: u16, x: Option<Num>, y: Option<Num>) -> Option<(f64, bool)> {
    use robigo_rtbuiltins::kernel::arith::{NA_INTEGER, NA_REAL};
    let real = |n: Option<Num>| match n {
        Some(Num::Real(d)) => Some(d),
        Some(Num::Int(i)) => Some(if i == NA_INTEGER { NA_REAL } else { i as f64 }),
        _ => None,
    };
    let of = |v: f64, a: f64| if v.is_nan() { if a.is_nan() { (a, false) } else { (v, true) } } else { (v, false) };
    match kind {
        MATH_SQRT | MATH_EXP => {
            let f = if kind == MATH_SQRT { f64::sqrt } else { f64::exp };
            match x? {
                Num::Real(a) => Some(of(f(a), a)),
                Num::Int(i) if i != NA_INTEGER => {
                    let v = f(i as f64);
                    Some((v, v.is_nan()))
                }
                _ => None,
            }
        }
        MATH_LOG => {
            let a = real(x)?;
            Some(of(robigo_rtbuiltins::math::bc_log(a), a))
        }
        MATH_LOGBASE => {
            let (a, b) = (real(x)?, real(y)?);
            let v = robigo_rtbuiltins::math::bc_logbase(a, b);
            Some(match v.is_nan() {
                true if a.is_nan() => (a, false),
                true if b.is_nan() => (b, false),
                nan => (v, nan),
            })
        }
        k => {
            let a = real(x)?;
            let f = robigo_rtbuiltins::math::BC_MATH1.get(k as usize)?.1;
            Some(of(f(a), a))
        }
    }
}

/// [`Op::Math1`]: bcEval's scalar math on the operands' registers
/// ([`math1_fast`]), nmath's queued warnings and "NaNs produced" after it
/// ([`math1_warnings`]); else the builtin ([`math1_slow`]). Out of
/// `vm::run`, whose register allocation an inline arm shifted (every
/// closure-call row of perf-rt paid ~8 instructions a call).
// R: eval.c:FastMath1, DO_MATH1, DO_LOG, DO_LOGBASE
#[inline(never)]
fn math1(cx: &mut Cx, act: &mut Act, v: &View, i: &Ins, pc: usize) -> RResult<()> {
    // SAFETY: registers of the running activation: their objects are live, rooted.
    let (x, y) = unsafe { (num(cx, act.get(i.b)), if i.c == 2 { num(cx, act.get(i.b + 1)) } else { None }) };
    // nmath's functions queue their warnings (ML_WARNING): those of this
    // number only
    let nmath = robigo_rtbuiltins::math::BC_MATH1.get(i.g as usize).is_some_and(|e| e.2);
    if nmath {
        robigo_rtbuiltins::math::take_nmath_warnings();
    }
    let Some((val, warn)) = math1_fast(i.g, x, y) else {
        return math1_slow(cx, act, v, i, pc);
    };
    let queued = if nmath { robigo_rtbuiltins::math::take_nmath_warnings() } else { Vec::new() };
    if warn || !queued.is_empty() {
        // SAFETY: a constant of the code.
        let call = unsafe { v.k(i.e) };
        math1_warnings(cx, act, call, &queued, warn)?;
    }
    // SAFETY: an unboxed value.
    unsafe { act.set(i.a, Slot::real(val)) };
    for r in i.b..i.b + i.c {
        act.clear(r);
    }
    cx.set_visible(true);
    Ok(())
}

/// [`Op::Math1`]'s warnings, in bcEval's order: nmath's (`queued`, raised
/// by `MATHLIB_WARNING`'s `warning()` while the function ran, so against
/// the expression bcEval is at, the call), then "NaNs produced" if `nan`,
/// against the call (`warningcall(call, R_MSG_NA)`).
#[inline(never)]
fn math1_warnings(cx: &mut Cx, act: &Act, call: Sexp, queued: &[String], nan: bool) -> RResult<()> {
    // SAFETY: a constant of the rooted code.
    unsafe { act.scratch(R_SCRATCH, call) };
    let r = (|| {
        for w in queued {
            // SAFETY: the rooted call.
            unsafe { cx.set_bc_expression(act.obj(R_SCRATCH)) };
            let r = cx.warning(w.strip_suffix('\n').unwrap_or(w));
            // SAFETY: R_NilValue.
            unsafe { cx.set_bc_expression(cx.nil().sexp()) };
            r?;
        }
        if nan {
            cx.warning_call(Some(act.root(R_SCRATCH)), "NaNs produced")?;
        }
        Ok(())
    })();
    act.clear(R_SCRATCH);
    r
}

/// [`Op::Math1`]'s slow path: the builtin `consts[f]` (or, 0, `log`'s
/// evaluated form, `do_log_builtin`) on the window, for the call
/// `consts[e]`, as bcEval's `Builtin1(do_math1, ...)` and
/// `do_log_builtin(call, ...)` call them.
fn math1_slow(cx: &mut Cx, act: &mut Act, v: &View, i: &Ins, pc: usize) -> RResult<()> {
    // SAFETY: constants of the code.
    let call = unsafe { v.k(i.e) };
    cx.set_eval_depth(act.depth + i.d as i32);
    pp_point(cx, act, v, pc);
    if v.profile == Profile::Bc {
        // the expression bcEval is at, for error() and warning()
        // SAFETY: a constant of the rooted code object.
        unsafe { cx.set_bc_expression(call) };
    }
    let r = if i.f != 0 {
        // SAFETY: a primitive constant of the code.
        let fun = unsafe { v.k(i.f) };
        // SAFETY: the running code's constants: live, of this isolate.
        unsafe { crate::call::call_builtin_regs(cx, act, i.a, fun, call, i.b, i.c as usize, false) }
    } else {
        // SAFETY: as above.
        unsafe {
            crate::call::call_fn_regs(cx, act, i.a, robigo_rtbuiltins::math::do_log_builtin, call, i.b, i.c as usize)
        }
    };
    if v.profile == Profile::Bc {
        // SAFETY: R_NilValue.
        unsafe { cx.set_bc_expression(cx.nil().sexp()) };
    }
    r
}

/// An inlined builtin's slow path: the builtin `consts[f]` called with the
/// `n` operands from register `b` (boxed) for the call `consts[e]`.
fn slow_builtin(cx: &mut Cx, act: &mut Act, v: &View, i: &Ins, pc: usize, n: usize) -> RResult<()> {
    // SAFETY: constants of the code.
    let (call, fun) = unsafe { (v.k(i.e), v.k(i.f)) };
    cx.set_eval_depth(act.depth + i.d as i32);
    pp_point(cx, act, v, pc);
    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
    unsafe { crate::call::call_builtin_regs(cx, act, i.a, fun, call, i.b, n, false) }
}

/// `defineVar(sym, value, rho)` (`<-`) or `setVar(sym, value, ENCLOS(rho))`
/// (`<<-`), with register `r` (boxed) the value; `R_Visible = FALSE`.
// R: eval.c:do_set
fn set_var(cx: &mut Cx, act: &Act, r: Reg, sym: Sexp, super_assign: bool, slot: u16, shape: Sexp) -> RResult<()> {
    if !super_assign {
        let v = act.get(r);
        let scalar = match v.tag() {
            Tag::Real => Some(robigo_rtcore::Scalar::Real(v.as_real().unwrap())),
            Tag::Int => Some(robigo_rtcore::Scalar::Int(v.as_int().unwrap())),
            Tag::Logical => Some(robigo_rtcore::Scalar::Lgl(Lgl(v.as_int().unwrap()))),
            _ => None,
        };
        // a local of the running closure: its slot, directly
        if let Some(x) = scalar
            && slot != 0
        {
            // SAFETY: an immortal symbol, the rooted environment and the
            // code's shape; the slot index was compiled for that shape.
            if unsafe { cx.frame_store_scalar_raw(act.rho(), shape, slot as usize - 1, sym, x) } {
                cx.set_visible(false);
                return Ok(());
            }
        }
        if let Some(x) = scalar {
            // an unboxed value stays unboxed in a slot frame's slot (N1b's
            // `define_scalar`; elsewhere it is boxed and defined)
            // SAFETY: an immortal symbol.
            let s = unsafe { act.scratch(R_SCRATCH, sym) };
            let res = cx.define_scalar(s, x, act.root(R_ENV));
            act.clear(R_SCRATCH);
            res?;
            cx.set_visible(false);
            return Ok(());
        }
    }
    boxed(cx, act, r)?;
    // SAFETY: an immortal symbol.
    let s = unsafe { act.scratch(R_SCRATCH, sym) };
    let res = if super_assign {
        // SAFETY: ENCLOS of the live environment.
        let enclos = unsafe { act.scratch(R_SCRATCH2, robigo_heap::env::enclos(act.rho())) };
        cx.set_var(s, act.root(r), enclos)
    } else {
        cx.define_var(s, act.root(r), act.root(R_ENV))
    };
    act.clear(R_SCRATCH);
    act.clear(R_SCRATCH2);
    res?;
    cx.set_visible(false);
    Ok(())
}

/// `asLogicalNoNA(s, call)` for register `r`: the condition of `if` and
/// `while`.
// R: eval.c:asLogicalNoNA
/// [`Op::Resume`] and [`Op::LoopResume`] when the running code is stale:
/// the rest of the `{` or the iteration as GNU R's AST runs it, and the
/// instruction to go on at.
#[cold]
#[inline(never)]
fn boundary(cx: &mut Cx, act: &Act, i: Ins) -> RResult<usize> {
    // SAFETY: the code register holds the running code object.
    let v = unsafe { View::of(act.obj(R_CODE)) };
    cx.set_eval_depth(act.depth + i.d as i32 - 1);
    if i.op == Op::Resume {
        // SAFETY: a constant of the code.
        let prev = unsafe { v.k(i.e) };
        resume_begin(cx, act, prev, i.b, i.a)?;
        return Ok(i.target());
    }
    // SAFETY: constants of the code.
    let (body, cond) = unsafe { (v.k(i.e), (i.c != NO_CONST).then(|| (v.k(i.b), v.k(i.c)))) };
    Ok(if resume_iteration(cx, act, body, cond, i.a)? { i.target() } else { i.target() + 1 })
}

/// [`Op::Resume`]: the rest of a `{` as GNU R's `do_begin` runs it, from
/// the cell after `prev` (re-read after each statement: the list is live),
/// the value so far in register `val` ([`NO_REG`]: NULL); the last value
/// into `dst`.
// R: eval.c:do_begin
#[cold]
#[inline(never)]
fn resume_begin(cx: &mut Cx, act: &Act, prev: Sexp, val: Reg, dst: Reg) -> RResult<()> {
    if val == NO_REG {
        let nil = cx.nil().sexp();
        // SAFETY: R_NilValue into a register of this activation.
        unsafe { act.set(dst, Slot::object(nil)) };
    } else if val != dst {
        // SAFETY: a copy of a register of this activation.
        unsafe { act.set(dst, act.get(val)) };
        act.clear(val);
    }
    cx.scope(|cx, s| {
        // SAFETY: `prev` is a cell of the source, a constant of the rooted
        // code object.
        let cur = s.root(cx, unsafe { Ref::from_sexp(prev.cdr(), cx) });
        while cur.bind(cx).as_cons().is_some() {
            // (a scope per statement: a statement done is not kept alive)
            cx.scope(|cx, s| {
                let e = s.root(cx, cur.bind(cx).as_cons().unwrap().car());
                crate::eval::eval(cx, e, act.root(R_ENV), act.out(dst))
            })?;
            cur.replace_with(cx, |cx| Ok(cur.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr())))?;
        }
        Ok(())
    })
}

/// [`Op::LoopResume`]: one iteration as GNU R's loop runs it: for `while`
/// (`cond`: its `args` and call) the condition `CAR(args)` evaluated and
/// tested as `asLogicalNoNA` does (false: the loop ends), then `body`
/// evaluated. Register `r` takes each value and is cleared. True: the
/// iteration ran (go to the back-edge).
// R: eval.c:do_for, do_while, do_repeat
#[cold]
#[inline(never)]
fn resume_iteration(cx: &mut Cx, act: &Act, body: Sexp, cond: Option<(Sexp, Sexp)>, r: Reg) -> RResult<bool> {
    cx.scope(|cx, s| {
        // all rooted before anything runs: an evaluation may move them
        // SAFETY: constants of the rooted code object.
        let body = s.root(cx, unsafe { Ref::from_sexp(body, cx) });
        // SAFETY: as above.
        let cond = cond.map(|(a, c)| unsafe { (s.root(cx, Ref::from_sexp(a, cx)), s.root(cx, Ref::from_sexp(c, cx))) });
        if let Some((args, call)) = cond {
            // the condition `CAR(args)`, read now, in a scope of its own: as
            // in do_while, nothing of its evaluation stays protected while
            // the body runs
            let go = cx.scope(|cx, s| {
                let cond = s.root(cx, args.bind(cx).as_cons().unwrap().car());
                crate::eval::eval(cx, cond, act.root(R_ENV), act.out(r))
                    .and_then(|()| as_logical_no_na(cx, act, r, call.bind(cx).sexp()))
            });
            act.clear(r);
            if !go? {
                return Ok(false);
            }
        }
        let res = crate::eval::eval(cx, body, act.root(R_ENV), act.out(r));
        act.clear(r);
        res.map(|()| true)
    })
}

fn as_logical_no_na(cx: &mut Cx, act: &Act, r: Reg, call: Sexp) -> RResult<bool> {
    let s = act.get(r);
    let (cond, len, is_lgl) = match s.tag() {
        Tag::Logical => (s.as_int().unwrap(), 1, true),
        Tag::Int => (s.as_int().unwrap(), 1, false),
        Tag::Real => (ops::as_lgl(Num::Real(s.as_real().unwrap())), 1, false),
        _ => {
            // SAFETY: a register's object is live.
            let x = unsafe { Ref::from_sexp(act.obj(r), cx) };
            let len = match x.len() {
                Some(n) => n,
                None => {
                    return Err(cx.error("robigo: 'if' on an object without a plain length is not implemented yet"));
                }
            };
            if len > 1 {
                // SAFETY: a live constant.
                let c = unsafe { act.scratch(R_SCRATCH, call) };
                return Err(cx.error_call(Some(c), "the condition has length > 1"));
            }
            let cond = if len == 0 { ops::NA_LOGICAL } else { as_logical(cx, x) };
            (cond, len, x.sexptype() == LGLSXP)
        }
    };
    if cond == ops::NA_LOGICAL {
        let msg = if len == 0 {
            "argument is of length zero"
        } else if is_lgl {
            "missing value where TRUE/FALSE needed"
        } else {
            "argument is not interpretable as logical"
        };
        // SAFETY: a live constant.
        let c = unsafe { act.scratch(R_SCRATCH, call) };
        return Err(cx.error_call(Some(c), msg));
    }
    Ok(cond != 0)
}

/// `asLogical(x)` for an object of length at least 1: its first element.
// R: coerce.c:asLogical2 (checking = 0)
fn as_logical(cx: &Cx, x: Ref<'_>) -> i32 {
    use robigo_rtcore::View as V;
    match x.view() {
        V::Lgl(v) => v.get(0).0,
        V::Int(v) => ops::as_lgl(Num::Int(v.get(0))),
        V::Real(v) => ops::as_lgl(Num::Real(v.get(0))),
        V::Cplx(v) => {
            let c = v.get(0);
            if c.r.is_nan() || c.i.is_nan() { ops::NA_LOGICAL } else { (c.r != 0.0 || c.i != 0.0) as i32 }
        }
        V::Str(v) => logical_from_string(cx, v.get(0)),
        V::Raw(v) => (v.get(0) != 0) as i32,
        V::Char(c) => logical_from_string(cx, c),
        _ => ops::NA_LOGICAL,
    }
}

/// `LogicalFromString`: `StringTrue`/`StringFalse`.
// R: coerce.c:LogicalFromString, util.c:StringTrue
fn logical_from_string(cx: &Cx, c: robigo_rtcore::Char<'_>) -> i32 {
    if c == cx.na_string() {
        return ops::NA_LOGICAL;
    }
    match c.bytes() {
        b"T" | b"True" | b"TRUE" | b"true" => 1,
        b"F" | b"False" | b"FALSE" | b"false" => 0,
        _ => ops::NA_LOGICAL,
    }
}

/// An operand of `&&`/`||`: do_logic2's type check and
/// `asLogical2(s, 1, call)`.
// R: logic.c:do_logic2
fn and_or_operand(cx: &mut Cx, act: &Act, r: Reg, call: Sexp, op: u16, first: bool) -> RResult<i32> {
    let s = act.get(r);
    match s.tag() {
        Tag::Logical | Tag::Int => return Ok(ops::as_lgl(Num::Int(s.as_int().unwrap()))),
        Tag::Real => return Ok(ops::as_lgl(Num::Real(s.as_real().unwrap()))),
        _ => {}
    }
    // SAFETY: a register's object is live.
    let x = unsafe { Ref::from_sexp(act.obj(r), cx) };
    let opname = if op == 1 { "&&" } else { "||" };
    // isNumber: integer (not a factor), logical, double, complex
    let number = match x.sexptype() {
        INTSXP => !inherits(cx, x, b"factor"),
        LGLSXP | REALSXP | CPLXSXP => true,
        _ => false,
    };
    if !number {
        let which = if first { 'x' } else { 'y' };
        let msg = format!("invalid '{which}' type in 'x {opname} y'");
        // SAFETY: a live constant.
        let c = unsafe { act.scratch(R_SCRATCH, call) };
        return Err(cx.error_call(Some(c), &msg));
    }
    let len = x.len().unwrap_or(0);
    if len < 1 {
        return Ok(ops::NA_LOGICAL);
    }
    if len > 1 {
        let msg = format!("'length = {len}' in coercion to 'logical(1)'");
        // SAFETY: a live constant.
        let c = unsafe { act.scratch(R_SCRATCH, call) };
        return Err(cx.error_call(Some(c), &msg));
    }
    Ok(as_logical(cx, x))
}

/// Whether `x`'s class attribute contains `what` (`inherits` for an S3
/// class; S4 classes are S1's).
pub fn inherits(cx: &Cx, x: Ref<'_>, what: &[u8]) -> bool {
    if !x.is_object() {
        return false;
    }
    let mut a = match x.attrib() {
        Some(a) => a,
        None => return false,
    };
    while let Some(c) = a.as_cons() {
        if let Some(t) = c.tag().as_sym()
            && t.name_bytes() == b"class"
        {
            if let Some(v) = c.car().as_vector::<robigo_rtcore::Char<'_>>() {
                return v.as_slice().unwrap_or(&[]).iter().any(|s| s.bytes() == what && *s != cx.na_string());
            }
            return false;
        }
        a = c.cdr();
    }
    false
}

/// `mkCLOSXP(formals, body, rho)` into register `a`, the formals and body
/// the code's constants `kf` and `kb` (read afresh on each attempt: a
/// refused allocation collects).
// R: dstruct.c:mkCLOSXP
fn mk_closure(cx: &mut Cx, act: &Act, a: Reg, kf: u16, kb: u16, kc: u16, installed: bool) -> RResult<()> {
    // SAFETY: the rooted code object's constant.
    let body = unsafe { View::of(act.obj(R_CODE)).k(kb) };
    // SAFETY: as above.
    if matches!(unsafe { body.typ() }, CLOSXP | BUILTINSXP | SPECIALSXP | DOTSXP | ANYSXP) {
        return Err(cx.error("invalid body argument for 'function'"));
    }
    let x = cx.retry_alloc(|cx| {
        // SAFETY: the formals and body are constants of the rooted code, the
        // environment is rooted; nothing collects in this interval.
        unsafe {
            let v = View::of(act.obj(R_CODE));
            cx.alloc_node_raw(CLOSXP, v.k(kf), v.k(kb), act.rho())
        }
    })?;
    // SAFETY: fresh, of this isolate; the code is a constant of the rooted
    // code object, compiled for the closure's formals and body.
    unsafe {
        if kc != 0 {
            crate::jit::set_compiled(x, View::of(act.obj(R_CODE)).k(kc));
        } else if installed {
            x.set_extra(x.extra() | robigo_heap::extra::closure::BYTECODE);
        }
        act.set(a, Slot::object(x));
    }
    if installed && crate::prims::state(cx).forced != Some(Profile::Eval) {
        // base's installed byte code: compiled as base's build compiled it
        // (its provenance is known here only)
        let f = act.root(a);
        match crate::compile::compile_closure_bc(cx, f, true)? {
            // SAFETY: the rooted closure and its code.
            Some(code) => unsafe { crate::jit::set_compiled(f.bind(cx).sexp(), code) },
            None => {
                // (cmp.R compiled every closure of base: a gap of the port)
                let x = f.bind(cx).sexp();
                // SAFETY: the rooted closure.
                unsafe {
                    crate::jit::decompile(cx, x);
                    x.set_gp(x.gp() | crate::jit::NOJIT_MASK);
                }
            }
        }
    }
    Ok(())
}

/// bcEval's `BASEGUARD` for the call `call`: whether `findFun(CAR(call),
/// rho)` is base's binding of the symbol (`SymbolValue`). Register `r` is
/// scratch.
// R: eval.c:DO_BASEGUARD, SymbolValue
fn base_guard(cx: &mut Cx, act: &Act, r: Reg, call: Sexp) -> RResult<bool> {
    // SAFETY: a constant call of the rooted code: its head is a symbol.
    let sym = unsafe { call.car() };
    // both found without running code: one compare
    // SAFETY: an immortal symbol and the rooted environment; base is
    // immortal.
    unsafe {
        let base = cx.base_env().sexp();
        if let (Some(f), robigo_rtcore::RawBinding::Value(b)) =
            (cx.fun_lookup_raw(sym, act.rho()), cx.var_lookup_raw(sym, base))
        {
            let b = if b.typ() == PROMSXP && b.promise_tag() == 0 { b.car() } else { b };
            if b != cx.unbound().sexp() {
                return Ok(f == b);
            }
        }
    }
    // findFun (it may force a promise), then base's value (forced)
    // SAFETY: the running code's constants (or rooted values): live, of this isolate.
    unsafe { crate::call::find_fun(cx, act, r, sym, call) }?;
    let same = cx.scope(|cx, s| {
        let o = s.out(cx);
        let v = o.root();
        let base = s.root(cx, cx.base_env());
        // SAFETY: the running code's constants (or rooted values): live, of this isolate.
        unsafe { crate::eval::symbol_value(cx, sym, base, o) }?;
        Ok(v.bind(cx).sexp() == act.obj(r))
    });
    act.clear(r);
    same
}

/// bcEval's `STARTSUBSET_N` (`STARTSUBSET`): an object in register `r`
/// dispatches on `[` or `[[` (`tryDispatch`); true when a method ran, its
/// value in `dst` (register `r` cleared).
// R: eval.c:DO_STARTDISPATCH_N, DO_STARTDISPATCH, tryDispatch
fn start_subset(cx: &mut Cx, act: &Act, dst: Reg, r: Reg, call: Sexp, sub2: bool) -> RResult<bool> {
    // SAFETY: a constant of the rooted code, kept where a collection
    // updates it.
    unsafe { act.scratch(R_SCRATCH, call) };
    let x = boxed(cx, act, r)?;
    // SAFETY: a register's object is live.
    if !unsafe { Ref::from_sexp(x, cx) }.is_object() {
        act.clear(R_SCRATCH);
        return Ok(false);
    }
    let generic = if sub2 { "[[" } else { "[" };
    let done = cx.scope(|cx, s| {
        let out = act.out(dst);
        crate::bcassign::dispatch(cx, s, generic, act.root(R_SCRATCH), act.root(r), act.root(R_ENV), &out)
    });
    act.clear(R_SCRATCH);
    if matches!(done, Ok(true)) {
        act.clear(r);
    }
    done
}

/// GNU R's `NA_INTEGER`.
const NA_INTEGER: i32 = i32::MIN;

/// `bcStackIndex(si)`: register `r`'s scalar index, -1 for none (NA, a
/// logical, not a scalar number, not above 0 for a double).
// R: eval.c:bcStackIndex
fn stack_index(act: &Act, r: Reg) -> i64 {
    let s = act.get(r);
    match s.tag() {
        Tag::Int => s.as_int().map_or(-1, |i| if i != NA_INTEGER { i as i64 } else { -1 }),
        Tag::Real => s.as_real().map_or(-1, real_index),
        Tag::Object => {
            let x = s.as_object().unwrap();
            // SAFETY: a register's object is live; its length and data are
            // read only once it is an integer or double vector in standard
            // storage.
            unsafe {
                // IS_SCALAR(idx, INTSXP | REALSXP): the type first, then
                // the length; an ALTREP index is never IS_SCALAR (only
                // allocVector3 sets the scalar bit)
                let t = x.typ();
                if !matches!(t, INTSXP | REALSXP) || x.is_altrep() || x.std_len() != 1 {
                    return -1;
                }
                if t == INTSXP {
                    let i = *(x.std_data() as *const i32);
                    if i != NA_INTEGER { i as i64 } else { -1 }
                } else {
                    real_index(*(x.std_data() as *const f64))
                }
            }
        }
        _ => -1,
    }
}

/// bcStackIndex's double: `(R_xlen_t) val` when not NaN, at most
/// `R_XLEN_T_MAX` and above 0; else -1.
fn real_index(d: f64) -> i64 {
    const R_XLEN_T_MAX: f64 = 4503599627370496.0;
    if !d.is_nan() && d <= R_XLEN_T_MAX && d > 0.0 { d as i64 } else { -1 }
}

/// bcEval's `DO_VECSUBSET` and `VECSUBSET_PTR`'s `DO_FAST_VECELT`: `x[i]`
/// or `x[[i]]` (`sub2`) with the vector `x` in register `base` and one
/// scalar index in `base + 1`, without the default. It applies to a vector
/// of type REAL, INT, LGL, CPLX, RAW or VECSXP; for `[`, the vector must
/// have no attributes, or only `dim` (`FAST_VECELT_OK`). The index must be
/// in range. A REAL, INT or LGL element goes into `dst` unboxed. A CPLX or
/// RAW element is a new scalar. A list element is the element itself for
/// `[[`, a new list of one for `[`. True when it did that; false leaves
/// the call to the default.
///
/// An ALTREP vector takes the default too. GNU R's fast path reads one
/// through its class's `Elt` method; here no ALTREP method runs from the
/// byte-code loop, and the default gives the same values.
// R: eval.c:DO_VECSUBSET, VECSUBSET_PTR, DO_FAST_VECELT, FAST_VECELT_OK
#[inline(never)]
fn vec_subset_fast(cx: &mut Cx, act: &Act, dst: Reg, base: Reg, sub2: bool) -> RResult<bool> {
    use robigo_rtcore::View as V;
    let Some(x) = act.get(base).as_object() else {
        return Ok(false);
    };
    // SAFETY: a register's object is live.
    if unsafe { x.is_altrep() } {
        return Ok(false);
    }
    let i = stack_index(act, base + 1) - 1;
    // SAFETY: a register's object is live, of this isolate.
    let xr = unsafe { Ref::from_sexp(x, cx) };
    if !sub2 {
        // FAST_VECELT_OK(vec): ATTRIB(vec) == R_NilValue, or a single `dim`
        let ok = match xr.attrib().filter(|a| a.sexptype() != NILSXP) {
            None => true,
            Some(a) => {
                a.as_cons().is_some_and(|c| c.tag() == cx.sym(robigo_rtcore::RSym::Dim).into() && c.cdr() == cx.nil())
            }
        };
        if !ok {
            return Ok(false);
        }
    }
    let in_range = |n: usize| i >= 0 && (i as usize) < n;
    let k = i as usize;
    let slot = match xr.view() {
        V::Real(v) if in_range(v.len()) => Slot::real(v.get(k)),
        V::Int(v) if in_range(v.len()) => Slot::int(v.get(k)),
        V::Lgl(v) if in_range(v.len()) => Slot::logical(v.get(k).0),
        // VECTOR_ELT(vec, i) for `[[`: the element itself (RAISE_NAMED
        // changes no count)
        // SAFETY: a live element of the list a register keeps.
        V::List(v) if sub2 && in_range(v.len()) => unsafe { Slot::object(v.get(k).sexp()) },
        // ScalarComplex, ScalarRaw, and mkVector1(elt) for `[`: the
        // operands stay rooted in their registers while it allocates
        V::Cplx(_) | V::Raw(_) | V::List(_) if in_range(xr.len().unwrap_or(0)) => {
            cx.build(act.out(dst), |cx| {
                // SAFETY: the register's live object.
                let xr = unsafe { Ref::from_sexp(act.obj(base), cx) };
                Ok(match xr.view() {
                    V::Cplx(v) => {
                        let c = v.get(k);
                        cx.alloc_with(1, |_| c)?.finish().as_ref()
                    }
                    V::Raw(v) => {
                        let b = v.get(k);
                        cx.alloc_with(1, |_| b)?.finish().as_ref()
                    }
                    V::List(v) => {
                        let e = v.get(k);
                        let mut l = cx.alloc_vector::<Ref<'_>>(1)?;
                        l.set(0, e);
                        l.finish().as_ref()
                    }
                    _ => unreachable!(),
                })
            })?;
            clear_subset_operands(cx, act, base);
            return Ok(true);
        }
        _ => return Ok(false),
    };
    // SAFETY: an unboxed value holds no object; a list element is live.
    unsafe { act.set(dst, slot) };
    clear_subset_operands(cx, act, base);
    Ok(true)
}

/// The two operand registers of a subset cleared, `R_Visible` on: what
/// `DFVE_NEXT` leaves (`R_BCNodeStackTop--`, `R_Visible = TRUE`).
fn clear_subset_operands(cx: &mut Cx, act: &Act, base: Reg) {
    act.clear(base);
    act.clear(base + 1);
    cx.set_visible(true);
}

/// bcEval's default subset (`VECSUBSET`, `MATSUBSET`, `SUBSET_N`, their
/// `[[` forms, `DFLTSUBSET`): `do_subset_dflt` or `do_subset2_dflt` for
/// the call `call` with the `n` arguments in registers `base..` (their
/// names after them if `names`), the value in `dst`; `R_Visible` on.
// R: eval.c:VECSUBSET_PTR, DO_SUBSET_N, DO_DFLTDISPATCH
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn dflt_subset(
    cx: &mut Cx,
    act: &Act,
    dst: Reg,
    call: Sexp,
    base: Reg,
    n: usize,
    names: bool,
    sub2: bool,
) -> RResult<()> {
    // VECSUBSET's and VECSUBSET2's fast path: one index, no names (cmp.R
    // compiles any other form to MATSUBSET, SUBSET_N or DFLTSUBSET)
    if n == 2 && !names && vec_subset_fast(cx, act, dst, base, sub2)? {
        return Ok(());
    }
    // SAFETY: as in `start_subset`.
    unsafe { act.scratch(R_SCRATCH, call) };
    let mut any_alt = false;
    for r in base..base + n as Reg {
        let x = boxed(cx, act, r)?;
        // SAFETY: a register's object is live.
        any_alt |= unsafe { x.is_altrep() };
    }
    // SAFETY: `n` object slots from `base` (and `n` name slots after them),
    // rooted by the activation for the call.
    let args = unsafe {
        let p = act.window(base, if names { 2 * n } else { n });
        robigo_rtcore::Args::from_raw(p, names.then(|| p.add(n)), n as u32, any_alt)
    };
    // the expression bcEval is at, for error() and warning()
    // SAFETY: the rooted call.
    unsafe { cx.set_bc_expression(act.obj(R_SCRATCH)) };
    let (call, rho, out) = (act.root(R_SCRATCH), act.root(R_ENV), act.out(dst));
    // (the operands stay on the node stack while the default runs, as
    // GNU R's byte code keeps the object below the call frame,
    // DO_STARTDISPATCH, and BUILTIN_CALL_FRAME_ARGS drops only the
    // argument list's links: tests/rt/assign/link-walk-subset.R)
    let r = cx.materialize_subset_args(args).and_then(|()| {
        if sub2 {
            robigo_rtbuiltins::subset::do_subset2_dflt(cx, call, args, rho, out)
        } else {
            robigo_rtbuiltins::subset::do_subset_dflt(cx, call, args, rho, out)
        }
    });
    // SAFETY: R_NilValue.
    unsafe { cx.set_bc_expression(cx.nil().sexp()) };
    for r in base..base + if names { 2 * n } else { n } as Reg {
        act.clear(r);
    }
    act.clear(R_SCRATCH);
    cx.set_visible(true);
    r
}

/// bcEval's `DOLLAR`: `dst <- x$member` for the call `call`, the object in
/// register `r` (cleared), the member the symbol `sym`.
// R: eval.c:bcEval DOLLAR
fn dollar(cx: &mut Cx, act: &Act, dst: Reg, r: Reg, call: Sexp, sym: Sexp) -> RResult<()> {
    // SAFETY: constants of the rooted code, kept where a collection
    // updates them.
    unsafe {
        act.scratch(R_SCRATCH, call);
        act.scratch(R_SCRATCH2, sym);
    }
    let res = boxed(cx, act, r).and_then(|_| {
        cx.scope(|cx, s| {
            let (call, sym, x, rho) = (act.root(R_SCRATCH), act.root(R_SCRATCH2), act.root(r), act.root(R_ENV));
            crate::bcassign::dollar(cx, s, call, sym, x, rho, act.out(dst))
        })
    });
    act.clear(R_SCRATCH);
    act.clear(R_SCRATCH2);
    act.clear(r);
    res
}

/// bcEval's `SWITCH` for the call `call`: the entry of the jump tables the
/// value in register `r` selects, given the character table's names
/// (`names`, R_NilValue for none) and the numeric table's length `ni`. A
/// string matches a name exactly, as `pmatch(STRING_ELT(value, 0), name,
/// TRUE)` compares their bytes (so NA matches "NA", unlike `do_switch`).
// R: eval.c:bcEval SWITCH
fn switch_entry(cx: &mut Cx, act: &Act, r: Reg, call: Sexp, names: Sexp, ni: usize) -> RResult<usize> {
    cx.scope(|cx, s| {
        // SAFETY: constants of the rooted code; nothing collected since
        // they were read.
        let (call, names) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(names, cx))) };
        boxed(cx, act, r)?;
        let value = act.root(r);
        let x = value.bind(cx);
        if !robigo_rtbuiltins::attrib::is_vector(x) || x.len() != Some(1) {
            return Err(cx.error_call(Some(call), "EXPR must be a length 1 vector"));
        }
        if x.sexptype() == INTSXP && robigo_rtbuiltins::attrib::inherits(x, b"factor") {
            cx.warning_call(
                Some(call),
                "EXPR is a \"factor\", treated as integer.\n Consider using 'switch(as.character( * ), ...)' instead.",
            )?;
        }
        let nc = names.bind(cx).len().unwrap_or(0);
        if value.bind(cx).sexptype() == STRSXP {
            if nc == 0 {
                if ni == 1 {
                    cx.warning_call(Some(call), "'switch' with no alternatives")?;
                    return Ok(0);
                }
                return Err(cx.error_call(Some(call), "numeric EXPR required for 'switch' without named alternatives"));
            }
            let (x, nm) = (value.bind(cx), names.bind(cx));
            let key = x.as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(0).bytes();
            let nm = nm.as_vector::<robigo_rtcore::Char<'_>>().expect("robigo: switch names are a character vector");
            return Ok((0..nc - 1).find(|&i| nm.get(i).bytes() == key).unwrap_or(nc - 1));
        }
        let w = crate::specials::as_integer(cx, value)?;
        let which = if w == i32::MIN || w < 1 || w as usize > ni { ni - 1 } else { w as usize - 1 };
        if ni == 1 {
            cx.warning_call(Some(call), "'switch' with no alternatives")?;
        }
        Ok(nc + which)
    })
}

/// `R_compileAndExecute(call, rho)` for a loop the `Eval` profile meets in
/// the global environment: compiled (`Bc`) and run there at the loop's
/// evaluation depth `depth`, its value (NULL) in register `dst`; false
/// when the compiler leaves it uncompiled (it runs as it is).
// R: eval.c:R_compileAndExecute
fn top_loop(cx: &mut Cx, act: &Act, dst: Reg, call: Sexp, depth: i32) -> RResult<bool> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code.
        let c = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        let Some(code) = crate::compile::compile_top_loop(cx, c, act.root(R_ENV))? else { return Ok(false) };
        // SAFETY: fresh.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        // bcEval runs at the loop's own depth (no `eval` of the code)
        cx.set_eval_depth(depth - 1);
        let out = act.out(dst);
        let slot = NonNull::new(out.root().as_ptr() as *mut Slot).unwrap();
        // SAFETY: rooted code and environment; an empty slot of the
        // activation.
        unsafe { exec(cx, code.bind(cx).sexp(), act.rho(), slot, false, false)? };
        Ok(true)
    })
}

/// `do_for`'s set-up: the sequence is in register `a`; registers `a..a+5`
/// become (sequence or pairlist node, index, length, loop value, binding
/// cell).
// R: eval.c:do_for
fn for_prep(cx: &mut Cx, act: &Act, a: Reg, sym: Sexp, call: Sexp) -> RResult<()> {
    let val = boxed(cx, act, a)?;
    // SAFETY: a register's object is live.
    let x = unsafe { Ref::from_sexp(val, cx) };
    if inherits(cx, x, b"factor") {
        return Err(cx.error("robigo: 'for' over a factor (asCharacterFactor) is not implemented yet"));
    }
    let n = match x.sexptype() {
        NILSXP | LISTSXP | LANGSXP | DOTSXP => x.len().unwrap(),
        _ if robigo_heap::is_vector_type(x.sexptype()) => match x.len() {
            Some(n) => n,
            None => return Err(cx.error("robigo: 'for' over an ALTREP sequence is not implemented yet")),
        },
        _ => {
            // XLENGTH of a non-vector: GNU R's error comes at the first
            // step ("invalid for() loop sequence"), with n = its length
            1
        }
    };
    let _ = call;
    // defineVar(sym, R_NilValue, rho)
    let nil = cx.nil().sexp();
    // SAFETY: an immortal symbol and object.
    unsafe {
        let s = act.scratch(R_SCRATCH, sym);
        let v = act.scratch(R_SCRATCH2, nil);
        let r = cx.define_var(s, v, act.root(R_ENV));
        act.clear(R_SCRATCH);
        act.clear(R_SCRATCH2);
        r?;
    }
    // cell = GET_BINDING_CELL(sym, rho): R_findVarLocInFrame (a slot
    // binding becomes cell-backed, as N1b's varloc does), R_NilValue for
    // base, a user database or an active binding
    // SAFETY: an immortal symbol.
    let found = unsafe {
        let s = act.scratch(R_SCRATCH, sym);
        let r = cx.find_var_loc_in_frame(act.root(R_ENV), s, act.out(a + 4));
        act.clear(R_SCRATCH);
        r?
    };
    let cell = match act.get(a + 4).as_object() {
        // SAFETY: a live cell or symbol of the frame.
        Some(c) if found && unsafe { c.typ() == LISTSXP && c.gp() & (1 << 15) == 0 } => c,
        _ => nil,
    };
    // INCREMENT_LINKS(val)
    // SAFETY: a live object.
    unsafe { act.obj(a).incref() };
    // SAFETY: unboxed values, and a live cell of the frame (or nil).
    unsafe {
        act.set(a + 1, Slot::real(0.0));
        act.set(a + 2, Slot::real(n as f64));
        act.set(a + 3, Slot::object(nil));
        act.set(a + 4, Slot::object(cell));
        act.set(a + 5, Slot::object(nil));
        act.set(a + 6, Slot::real(0.0));
    }
    // one of robigo's compact sequences: read by region
    // SAFETY: a live object.
    if unsafe { act.obj(a).is_altrep() } {
        compact_prep(cx, act, a)?;
    }
    Ok(())
}

/// The elements a `for` loop over one of robigo's compact sequences reads
/// at a time: GNU R's `ITERATE_BY_REGION` reads the class's `Get_region`
/// into a buffer as this does. A multiple of `BC_LOOP_COUNT_MASK + 1`
/// (1024), so that a chunk-relative index ticks the loop
/// (`BC_CHECK_SIGINT_LOOP`) exactly when the sequence's index does.
const FOR_CHUNK: usize = 1024;

/// [`for_prep`] of one of robigo's compact sequences: the sequence moves to
/// the loop's register `a + 5` (where [`for_refill`] reads it, and the loop's
/// count of it is dropped), and the loop reads a buffer of its elements
/// instead, one chunk at a time, through [`for_step`]'s standard path (no
/// expansion and nothing of the sequence's length allocated). One already
/// expanded is read in place: INTEGER_ELT reads the expansion.
// R: eval.c:bcEval (STARTFOR, STEPFOR), R_ext/Itermacros.h:ITERATE_BY_REGION
#[cold]
#[inline(never)]
fn compact_prep(cx: &mut Cx, act: &Act, a: Reg) -> RResult<()> {
    let seq = act.obj(a);
    // SAFETY: a live compact sequence (`for_prep` admits no other ALTREP
    // vector) of this isolate, rooted in the loop's registers.
    unsafe {
        act.set(a + 5, Slot::object(seq));
        if let Some(e) = robigo_rtcore::altrep::own::expanded(seq) {
            act.set(a, Slot::object(e));
            return Ok(());
        }
        let len = robigo_rtcore::altrep::own::own_len(seq).min(FOR_CHUNK);
        let t = seq.typ();
        let buf = cx.retry_alloc(|cx| cx.alloc_vector_raw(t, len))?;
        act.set(a, Slot::object(buf));
        fill_chunk(act, a, 0, len);
        act.set(a + 2, Slot::real(len as f64));
    }
    Ok(())
}

/// The next chunk of a compact sequence's loop, when [`for_step`] has read
/// the last one: true with the loop's registers on it, false when the
/// sequence is done (or the loop's sequence is not a compact one: the
/// standard loop's end).
#[cold]
#[inline(never)]
fn for_refill(act: &Act, a: Reg) -> bool {
    let Some(seq) = act.get(a + 5).as_object() else { return false };
    // SAFETY: the sequence `compact_prep` put in the register, live and
    // rooted, or R_NilValue for another loop.
    unsafe {
        if seq.typ() == NILSXP {
            return false;
        }
        let base = (act.get(a + 6).as_real().unwrap() + act.get(a + 2).as_real().unwrap()) as usize;
        let total = robigo_rtcore::altrep::own::own_len(seq);
        if base >= total {
            return false;
        }
        if let Some(e) = robigo_rtcore::altrep::own::expanded(seq) {
            // expanded since (C's DATAPTR): the rest read in place, by the
            // sequence's own index
            act.set(a, Slot::object(e));
            act.set(a + 1, Slot::real(base as f64));
            act.set(a + 2, Slot::real(total as f64));
            act.set(a + 6, Slot::real(0.0));
            return true;
        }
        let len = (total - base).min(FOR_CHUNK);
        fill_chunk(act, a, base, len);
        act.set(a + 1, Slot::real(0.0));
        act.set(a + 2, Slot::real(len as f64));
        act.set(a + 6, Slot::real(base as f64));
    }
    true
}

/// Elements `base..base + len` of the compact sequence in register `a + 5`
/// into the buffer in register `a` (`Get_region`).
///
/// # Safety
/// Register `a + 5` holds a live compact sequence of at least `base + len`
/// elements and register `a` a standard vector of its type, at least `len`
/// long and payload inline (`alloc_vector_raw`'s).
unsafe fn fill_chunk(act: &Act, a: Reg, base: usize, len: usize) {
    let (seq, buf) = (act.obj(a + 5), act.obj(a));
    // SAFETY: the caller's contract; only INTSXP and REALSXP have classes,
    // and the sequence is not expanded (`compact_prep` and `for_refill`
    // read an expanded one in place): its info is its elements' formula.
    unsafe {
        let (_, n1, inc) = robigo_rtcore::altrep::own::compact_info(seq);
        if seq.typ() == INTSXP {
            // an integer sequence's first element and increment (1 or -1)
            // are ints, and so is each of its elements: this one in int
            // arithmetic, which vectorizes
            let mut v = (n1 as i64 + inc as i64 * base as i64) as i32;
            let inc = inc as i32;
            let d = std::slice::from_raw_parts_mut(buf.inline_data() as *mut i32, len);
            for x in d {
                *x = v;
                // (past the last element only: an int wraps there)
                v = v.wrapping_add(inc);
            }
        } else {
            let d = buf.inline_data() as *mut f64;
            for k in 0..len {
                *d.add(k) = n1 + inc * (base + k) as f64;
            }
        }
    }
}

/// `do_for`'s iteration: bind the next element; false when done.
// R: eval.c:do_for
fn for_step(cx: &mut Cx, act: &Act, a: Reg, sym: Sexp, call: Sexp, profile: Profile) -> RResult<bool> {
    for_step_in::<true>(cx, act, a, sym, call, profile)
}

/// [`for_step`] at the end of the loop's vector: the end, or a compact
/// sequence's next chunk ([`for_refill`]), whose first element this binds
/// (the step's code is then the standard loop's, unchanged).
#[cold]
#[inline(never)]
fn for_next_chunk(cx: &mut Cx, act: &Act, a: Reg, sym: Sexp, call: Sexp, profile: Profile) -> RResult<bool> {
    if !for_refill(act, a) {
        return Ok(false);
    }
    for_step_in::<false>(cx, act, a, sym, call, profile)
}

#[inline(always)]
fn for_step_in<const FIRST: bool>(
    cx: &mut Cx,
    act: &Act,
    a: Reg,
    sym: Sexp,
    call: Sexp,
    profile: Profile,
) -> RResult<bool> {
    let i = act.get(a + 1).as_real().unwrap();
    let n = act.get(a + 2).as_real().unwrap();
    if i >= n {
        if FIRST {
            return for_next_chunk(cx, act, a, sym, call, profile);
        }
        return Ok(false);
    }
    let idx = i as usize;
    // SAFETY: an unboxed value.
    unsafe { act.set(a + 1, Slot::real(i + 1.0)) };
    if profile == Profile::Bc {
        // R: eval.c:BC_CHECK_SIGINT_LOOP(i)
        crate::eval::bc_tick_loop(cx, idx)?;
    }
    let val = act.obj(a);
    // SAFETY: the sequence is live and rooted.
    let t = unsafe { val.typ() };
    match t {
        EXPRSXP | VECSXP => {
            // SAFETY: an element of a live list.
            let e = unsafe { *(val.std_data() as *const Sexp).add(idx) };
            // SAFETY: as above: ENSURE_NAMEDMAX
            unsafe {
                if !e.has_flag(robigo_heap::flags::IMMORTAL) {
                    e.set_refcnt(u16::MAX);
                }
            }
            define_loop_var(cx, act, sym, e)?;
        }
        LISTSXP => {
            // SAFETY: a live pairlist node.
            let (e, next) = unsafe { (val.car(), val.cdr()) };
            // SAFETY: as above.
            unsafe {
                if !e.has_flag(robigo_heap::flags::IMMORTAL) {
                    e.set_refcnt(u16::MAX);
                }
            }
            define_loop_var(cx, act, sym, e)?;
            // `val = CDR(val)` (its link count stays on the head, as GNU R
            // leaves it)
            // SAFETY: the rest of a live pairlist.
            unsafe { act.set(a, Slot::object(next)) };
        }
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
            // ALLOC_LOOP_VAR(v, val_type, vpi)
            let vv = act.obj(a + 3);
            let cell = act.obj(a + 4);
            let nil = cx.nil().sexp();
            // SAFETY: live objects.
            // (never under ROBIGO_NO_INPLACE: a new value each step)
            let reuse = !cx.no_inplace()
                && unsafe {
                    vv != nil && vv.refcnt() <= 1 && vv.attrib_raw() == nil && cell != nil && vv == cell.car()
                };
            let v = if reuse {
                vv
            } else {
                let nv = cx.retry_alloc(|cx| cx.alloc_vector_raw(t, 1))?;
                // SAFETY: fresh, of this isolate.
                unsafe { act.set(a + 3, Slot::object(nv)) };
                nv
            };
            let val = act.obj(a);
            // SAFETY: `v` is a length-1 vector of the sequence's type, `val`
            // a live vector longer than `idx`.
            unsafe {
                match t {
                    LGLSXP | INTSXP => *(v.std_data() as *mut i32) = *(val.std_data() as *const i32).add(idx),
                    REALSXP => *(v.std_data() as *mut f64) = *(val.std_data() as *const f64).add(idx),
                    CPLXSXP => {
                        *(v.std_data() as *mut robigo_heap::Rcomplex) =
                            *(val.std_data() as *const robigo_heap::Rcomplex).add(idx)
                    }
                    RAWSXP => *v.std_data() = *val.std_data().add(idx),
                    _ => v.set_elt(0, *(val.std_data() as *const Sexp).add(idx)),
                }
            }
            let cell = act.obj(a + 4);
            // SAFETY: live objects of this isolate.
            let stored = unsafe {
                cell != nil && cell.car() != cx.unbound().sexp() && cx.set_cell_value_raw(act.rho(), cell, v)
            };
            if !stored {
                define_loop_var(cx, act, sym, v)?;
            }
        }
        _ => {
            // SAFETY: a live constant.
            let c = unsafe { act.scratch(R_SCRATCH, call) };
            return Err(cx.error_call(Some(c), "invalid for() loop sequence"));
        }
    }
    Ok(true)
}

/// `defineVar(sym, value, rho)` for the loop variable.
fn define_loop_var(cx: &mut Cx, act: &Act, sym: Sexp, value: Sexp) -> RResult<()> {
    // SAFETY: an immortal symbol, and an object reachable from the rooted
    // sequence (or rooted in a register).
    unsafe {
        let s = act.scratch(R_SCRATCH, sym);
        let v = act.scratch(R_SCRATCH2, value);
        let r = cx.define_var(s, v, act.root(R_ENV));
        act.clear(R_SCRATCH);
        act.clear(R_SCRATCH2);
        r
    }
}
