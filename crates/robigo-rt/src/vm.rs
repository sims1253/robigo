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
pub struct Act {
    /// The registers.
    pub base: NonNull<Slot>,
    /// Where the value goes: an empty object slot of the caller.
    pub out: NonNull<Slot>,
    /// `R_EvalDepth` when the activation began: an evaluation point of depth
    /// `d` runs at `depth + d`.
    pub depth: i32,
    /// The body of a closure call: `return` leaves it directly.
    pub body: bool,
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
    /// The slot of register `r`.
    #[inline(always)]
    pub fn slot(&self, r: Reg) -> NonNull<Slot> {
        // SAFETY: registers are below the frame's size (the compiler's
        // contract), all inside the activation's frame.
        unsafe { self.base.add(r as usize) }
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

    /// Register `r` as a root.
    #[inline(always)]
    pub fn root<'s>(&self, r: Reg) -> Root<'s> {
        // SAFETY: a live slot of this activation, which outlives every use
        // the VM makes of the root (within one instruction).
        unsafe { Root::from_raw(self.slot(r)) }
    }

    /// Register `r`, emptied, as the `Out` of a callee.
    #[inline(always)]
    pub fn out<'s>(&self, r: Reg) -> Out<'s> {
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
/// slot `out`. `body`: the body of a closure call.
///
/// # Safety
/// `code` is a live code object and `rho` a live environment, both rooted
/// by the caller for the call; `out` is an empty value-stack slot of this
/// isolate that stays live.
pub unsafe fn exec(cx: &mut Cx, code: Sexp, rho: Sexp, out: NonNull<Slot>, body: bool) -> RResult<()> {
    // SAFETY: the caller's contract.
    let v = unsafe { View::of(code) };
    let entry = cx.eval_depth();
    cx.check_value_stack(v.nregs)?;
    let r = cx.scope(|cx, s| {
        let base = s.frame(cx, v.nregs);
        let mut act = Act { base, out, depth: entry, body };
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
        run(cx, &mut act, 0)
    });
    cx.set_eval_depth(entry);
    r.map(|_| ())
}

/// Box register `r` if it holds an unboxed scalar (the box is written back,
/// so later reads see the one object); its object.
pub fn boxed(cx: &mut Cx, act: &Act, r: Reg) -> RResult<Sexp> {
    let s = act.get(r);
    if let Some(x) = s.as_object() {
        return Ok(x);
    }
    let x = alloc_retry(cx, |cx| box_slot(cx, s))?;
    // SAFETY: fresh, of this isolate; nothing collected since.
    unsafe { act.set(r, Slot::object(x)) };
    Ok(x)
}

/// A new length-1 vector holding the unboxed `s`.
pub fn box_slot(cx: &Cx, s: Slot) -> Result<Sexp, NeedGc> {
    Ok(match s.tag() {
        Tag::Real => cx.alloc_with(1, |_| s.as_real().unwrap())?.finish().as_ref().sexp(),
        Tag::Int => cx.alloc_with(1, |_| s.as_int().unwrap())?.finish().as_ref().sexp(),
        Tag::Logical => cx.alloc_with(1, |_| Lgl(s.as_int().unwrap()))?.finish().as_ref().sexp(),
        Tag::Object => s.as_object().unwrap(),
        Tag::Empty => cx.nil().sexp(),
    })
}

/// Run `f` (a `&Cx` allocation) until it succeeds, collecting after a
/// refusal; the same request refused twice is GNU R's error.
pub fn alloc_retry(cx: &mut Cx, mut f: impl FnMut(&Cx) -> Result<Sexp, NeedGc>) -> RResult<Sexp> {
    let mut retried = None;
    loop {
        match f(cx) {
            Ok(x) => return Ok(x),
            Err(e) if retried == Some(e) => {
                let vecrecs = e.length().saturating_mul(robigo_heap::elt_size(e.sexptype())).div_ceil(8);
                return Err(cx.error_call(None, &robigo_heap::AllocError::CannotAllocate { vecrecs }.message()));
            }
            Err(e) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
        }
    }
}

/// A register's value as a scalar number, if it is one the kernels take
/// (an unboxed scalar, or a plain length-1 vector without attributes).
#[inline(always)]
pub fn num(cx: &Cx, s: Slot) -> Option<Num> {
    match s.tag() {
        Tag::Real => Some(Num::Real(s.as_real().unwrap())),
        Tag::Int => Some(Num::Int(s.as_int().unwrap())),
        Tag::Logical => Some(Num::Lgl(s.as_int().unwrap())),
        Tag::Object => {
            // SAFETY: a register's object is live and rooted.
            let r = unsafe { Ref::from_sexp(s.as_object().unwrap(), cx) };
            ops::plain_num(r)
        }
        Tag::Empty => None,
    }
}

#[inline(always)]
fn num_slot(v: Num) -> Slot {
    match v {
        Num::Real(x) => Slot::real(x),
        Num::Int(x) => Slot::int(x),
        Num::Lgl(x) => Slot::logical(x),
    }
}

/// Run `act`'s code from `pc` until it returns or its region ends.
pub fn run(cx: &mut Cx, act: &mut Act, mut pc: usize) -> RResult<Flow> {
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
                // SAFETY: the compiler loads plain scalar constants only.
                let s = unsafe {
                    let k = v.k(i.e);
                    match k.typ() {
                        REALSXP => Slot::real(*(k.std_data() as *const f64)),
                        INTSXP => Slot::int(*(k.std_data() as *const i32)),
                        _ => Slot::logical(*(k.std_data() as *const i32)),
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
                if !crate::eval::get_var_fast(cx, act, i.a, sym) {
                    cx.set_eval_depth(act.depth + i.d as i32);
                    crate::eval::get_var(cx, act, i.a, sym, false)?;
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
                crate::eval::get_var(cx, act, i.a, sym, true)?;
                reload!();
            }
            Op::SetVar | Op::SetVar2 => {
                // SAFETY: as above.
                let sym = unsafe { v.k(i.e) };
                set_var(cx, act, i.a, sym, i.op == Op::SetVar2)?;
                reload!();
            }
            Op::Jmp => pc = i.target(),
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
                }
            }
            Op::GetFun => {
                // SAFETY: as above.
                let (sym, call) = unsafe { (v.k(i.e), v.k(i.c)) };
                // SAFETY: as above.
                match unsafe { cx.fun_lookup_raw(sym, act.rho()) } {
                    // SAFETY: a function bound in a live frame.
                    Some(f) => unsafe { act.set(i.a, Slot::object(f)) },
                    None => {
                        cx.set_eval_depth(act.depth + i.d as i32);
                        crate::call::find_fun(cx, act, i.a, sym, call)?;
                        reload!();
                    }
                }
            }
            Op::Call => {
                // SAFETY: as above.
                let call = unsafe { v.k(i.e) };
                cx.set_eval_depth(act.depth + i.d as i32);
                crate::call::call(cx, act, i.a, i.b, call, i.c)?;
                if i.fl & F_CLR_B != 0 && i.b != i.a {
                    act.clear(i.b);
                }
                reload!();
            }
            Op::CallBuiltin | Op::Internal => {
                // SAFETY: as above.
                let (call, fun) = unsafe { (v.k(i.e), v.k(i.f)) };
                cx.set_eval_depth(act.depth + i.d as i32);
                if i.c == u16::MAX {
                    crate::call::call_builtin_list(cx, act, i.a, fun, call)?;
                } else {
                    crate::call::call_builtin_regs(cx, act, i.a, fun, call, i.b, i.c as usize, i.fl & F_NAMES != 0)?;
                }
                reload!();
            }
            Op::Arith => {
                let (x, y) = (num(cx, act.get(i.b)), num(cx, act.get(i.c)));
                let fast = match (x, y) {
                    (Some(x), Some(y)) => match ops::arith2(i.g as u8, x, y) {
                        (r, false) => Some(r),
                        // the overflow's warning is the builtin's to signal
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
                        slow_builtin(cx, act, &v, &i, 2)?;
                        reload!();
                    }
                }
            }
            Op::Arith1 => match num(cx, act.get(i.b)) {
                Some(x) => {
                    // SAFETY: an unboxed value.
                    unsafe { act.set(i.a, num_slot(ops::arith1(i.g as u8, x))) };
                    clear_operands(act, &i);
                    if i.fl & F_VIS != 0 {
                        cx.set_visible(true);
                    }
                }
                None => {
                    slow_builtin(cx, act, &v, &i, 1)?;
                    reload!();
                }
            },
            Op::Relop => {
                let (x, y) = (num(cx, act.get(i.b)), num(cx, act.get(i.c)));
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
                        slow_builtin(cx, act, &v, &i, 2)?;
                        reload!();
                    }
                }
            }
            Op::Not => match num(cx, act.get(i.b)) {
                Some(x) => {
                    // SAFETY: an unboxed value.
                    unsafe { act.set(i.a, Slot::logical(ops::not1(x))) };
                    clear_operands(act, &i);
                    if i.fl & F_VIS != 0 {
                        cx.set_visible(true);
                    }
                }
                None => {
                    slow_builtin(cx, act, &v, &i, 1)?;
                    reload!();
                }
            },
            Op::Logic => match (num(cx, act.get(i.b)), num(cx, act.get(i.c))) {
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
                    slow_builtin(cx, act, &v, &i, 2)?;
                    reload!();
                }
            },
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
                mk_closure(cx, act, i.a, i.e, i.f)?;
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
                let m = crate::missing::r_missing(cx, act, sym)?;
                // SAFETY: an unboxed value.
                unsafe { act.set(i.a, Slot::logical(m as i32)) };
                if i.fl & F_VIS != 0 {
                    cx.set_visible(true);
                }
                reload!();
            }
            Op::Return => {
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
                let x = boxed(cx, act, i.b)?;
                // SAFETY: as above.
                unsafe { act.out.write(Slot::object(x)) };
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
                // R: eval.c:do_for, DECREMENT_LINKS(val)
                if let Some(x) = act.get(i.a).as_object() {
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
                cx.set_eval_depth(act.depth + i.d as i32 - 1);
                let r = crate::eval::eval(cx, e, act.root(R_ENV), act.out(i.a));
                act.clear(R_SCRATCH);
                r?;
                reload!();
            }
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

/// An inlined builtin's slow path: the builtin `consts[f]` called with the
/// `n` operands from register `b` (boxed) for the call `consts[e]`.
fn slow_builtin(cx: &mut Cx, act: &mut Act, v: &View, i: &Ins, n: usize) -> RResult<()> {
    // SAFETY: constants of the code.
    let (call, fun) = unsafe { (v.k(i.e), v.k(i.f)) };
    cx.set_eval_depth(act.depth + i.d as i32);
    crate::call::call_builtin_regs(cx, act, i.a, fun, call, i.b, n, false)
}

/// `defineVar(sym, value, rho)` (`<-`) or `setVar(sym, value, ENCLOS(rho))`
/// (`<<-`), with register `r` (boxed) the value; `R_Visible = FALSE`.
// R: eval.c:do_set
fn set_var(cx: &mut Cx, act: &Act, r: Reg, sym: Sexp, super_assign: bool) -> RResult<()> {
    if !super_assign {
        let v = act.get(r);
        let scalar = match v.tag() {
            Tag::Real => Some(robigo_rtcore::Scalar::Real(v.as_real().unwrap())),
            Tag::Int => Some(robigo_rtcore::Scalar::Int(v.as_int().unwrap())),
            Tag::Logical => Some(robigo_rtcore::Scalar::Lgl(Lgl(v.as_int().unwrap()))),
            _ => None,
        };
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
            && t.name().bytes() == b"class"
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
fn mk_closure(cx: &mut Cx, act: &Act, a: Reg, kf: u16, kb: u16) -> RResult<()> {
    // SAFETY: the rooted code object's constant.
    let body = unsafe { View::of(act.obj(R_CODE)).k(kb) };
    // SAFETY: as above.
    if matches!(unsafe { body.typ() }, CLOSXP | BUILTINSXP | SPECIALSXP | DOTSXP | ANYSXP) {
        return Err(cx.error("invalid body argument for 'function'"));
    }
    let x = alloc_retry(cx, |cx| {
        // SAFETY: the formals and body are constants of the rooted code, the
        // environment is rooted; nothing collects in this interval.
        unsafe {
            let v = View::of(act.obj(R_CODE));
            cx.alloc_node_raw(CLOSXP, v.k(kf), v.k(kb), act.rho())
        }
    })?;
    // SAFETY: fresh, of this isolate.
    unsafe { act.set(a, Slot::object(x)) };
    Ok(())
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
    }
    Ok(())
}

/// `do_for`'s iteration: bind the next element; false when done.
// R: eval.c:do_for
fn for_step(cx: &mut Cx, act: &Act, a: Reg, sym: Sexp, call: Sexp, profile: Profile) -> RResult<bool> {
    let i = act.get(a + 1).as_real().unwrap();
    let n = act.get(a + 2).as_real().unwrap();
    if i >= n {
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
            let reuse =
                unsafe { vv != nil && vv.refcnt() <= 1 && vv.attrib_raw() == nil && cell != nil && vv == cell.car() };
            let v = if reuse {
                vv
            } else {
                let nv = alloc_retry(cx, |cx| cx.alloc_vector_raw(t, 1))?;
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
