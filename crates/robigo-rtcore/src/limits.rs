//! GNU R's limits at R entries, with its messages (docs/g3.md §12, last
//! row): the evaluation depth (`options(expressions=)`, `eval.c`), the C
//! stack (`R_CheckStack`, `errors.c`), robigo's value stack (GNU R's byte
//! code node stack, whose message it reports) and the user-interrupt poll
//! (`R_CheckUserInterrupt`). The evaluator (V1) calls them where GNU R
//! does: `eval` increments the depth and checks the C stack for every call
//! it evaluates.
//!
//! GNU R signals these as condition objects of classes
//! `expressionStackOverflowError`, `CStackOverflowError` and
//! `nodeStackOverflowError` (`errors.c:R_InitConditions`); building
//! condition objects is S1's, so the exits recorded here carry the message
//! and the call, as `errorcall` does.

use crate::exit::{Exit, ExitKind, RError, RResult};
use crate::isolate::Cx;

/// `Cstack_info()`'s numbers ([`Cx::c_stack_info`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CStackInfo {
    /// `R_CStackLimit`; None when unlimited (GNU R's NA).
    pub limit: Option<usize>,
    /// The C stack in use, measured at the caller's frame.
    pub usage: isize,
    /// `R_CStackDir`.
    pub dir: isize,
}

/// The evaluation depth an R entry saved (`depthsave`), to restore as it
/// returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalDepth(i32);

/// GNU R's message for `R_Expressions` (errors.c, `EXPR_SO_MSG`).
pub const EXPR_SO_MSG: &str = "evaluation nested too deeply: infinite recursion / options(expressions=)?";
/// GNU R's message for its node stack (errors.c, `NODE_SO_MSG`), which
/// robigo's value stack reports.
pub const NODE_SO_MSG: &str = "node stack overflow";

impl Cx {
    /// `INCREMENT_EVAL_DEPTH()` and `R_CheckStack()`, as `eval` does for
    /// every call: one level deeper, and GNU R's errors past
    /// `R_Expressions` or near the C stack's limit. Restore the depth with
    /// [`Cx::eval_depth_leave`] on return (an exit restores it at its
    /// stop, from the record there).
    pub fn eval_depth_enter(&mut self) -> RResult<EvalDepth> {
        let st = self.ctxs();
        let save = st.eval_depth.get();
        // R: eval.c:INCREMENT_EVAL_DEPTH (saturating: `set_expressions`
        // leaves the range to `options`)
        let depth = save.saturating_add(1);
        st.eval_depth.set(depth);
        if depth > st.expressions.get() {
            // R: eval.c:handle_eval_depth_overflow ("This bump of
            // R_Expressions doesn't really work in many cases ...")
            st.expressions.set(st.expressions_keep.get().saturating_add(500));
            // a NULL call, so the handler does not deparse it
            return Err(self.error_call_sexp(self.iso().nil, EXPR_SO_MSG));
        }
        self.check_c_stack()?;
        Ok(EvalDepth(save))
    }

    /// `R_EvalDepth = depth` for an evaluation `depth` levels deep, with
    /// `INCREMENT_EVAL_DEPTH`'s error past `R_Expressions` and
    /// `R_CheckStack()`: the VM's form of [`Cx::eval_depth_enter`], which
    /// knows the depth of each of its evaluation points statically
    /// (docs/g3.md §9.1, the `Eval` profile).
    #[inline]
    pub fn eval_depth_at(&mut self, depth: i32) -> RResult<()> {
        let st = self.ctxs();
        st.eval_depth.set(depth);
        if depth > st.expressions.get() {
            return Err(self.eval_depth_overflow());
        }
        self.check_c_stack()
    }

    /// `handle_eval_depth_overflow`.
    #[cold]
    #[inline(never)]
    fn eval_depth_overflow(&mut self) -> RError {
        let st = self.ctxs();
        // R: eval.c:handle_eval_depth_overflow ("This bump of
        // R_Expressions doesn't really work in many cases ...")
        st.expressions.set(st.expressions_keep.get() + 500);
        // a NULL call, so the handler does not deparse it
        self.error_call_sexp(self.iso().nil, EXPR_SO_MSG)
    }

    /// `R_EvalDepth = depth`, unchecked: an R entry restoring the depth it
    /// saved, or the VM setting the depth of an evaluation point it already
    /// checked.
    #[inline(always)]
    pub fn set_eval_depth(&self, depth: i32) {
        self.ctxs().eval_depth.set(depth);
    }

    /// Restore the depth an R entry saved (`R_EvalDepth = depthsave`).
    pub fn eval_depth_leave(&mut self, saved: EvalDepth) {
        self.ctxs().eval_depth.set(saved.0);
    }

    /// `R_EvalDepth`.
    pub fn eval_depth(&self) -> i32 {
        self.ctxs().eval_depth.get()
    }

    /// `R_Expressions`, the depth limit in force (`R_Expressions_keep + 500`
    /// while an overflow is handled).
    pub fn expressions(&self) -> i32 {
        self.ctxs().expressions.get()
    }

    /// `options(expressions = n)`: `R_Expressions = R_Expressions_keep = n`
    /// (the option's own checks are `options`').
    pub fn set_expressions(&mut self, n: i32) {
        let st = self.ctxs();
        st.expressions.set(n);
        st.expressions_keep.set(n);
    }

    /// `R_CheckStack()`: GNU R's error when the C stack in use exceeds
    /// `R_CStackLimit` (95% of the thread's stack). While the error is
    /// handled the limit is divided by 0.95, back to the whole stack
    /// (`R_SignalCStackOverflow`: "We have 5% head room because we reduced
    /// R_CStackLimit to 95% of the initial value"), and an exit's stop
    /// restores it.
    #[inline]
    pub fn check_c_stack(&mut self) -> RResult<()> {
        self.check_c_stack_extra(0)
    }

    /// `R_CheckStack2(extra)`: [`Cx::check_c_stack`] for a caller about to
    /// use `extra` more bytes.
    pub fn check_c_stack_extra(&mut self, extra: usize) -> RResult<()> {
        let st = self.ctxs();
        let limit = st.cstack_limit.get();
        if limit == usize::MAX {
            return Ok(());
        }
        let usage = c_stack_usage(st.cstack_start, st.cstack_dir);
        // R: errors.c:R_CheckStack2 (an `extra` or a sum past isize::MAX is
        // "too much")
        let usage = isize::try_from(extra).ok().and_then(|x| usage.checked_add(x)).unwrap_or(isize::MAX);
        if usage > limit as isize { Err(self.signal_c_stack_overflow(usage)) } else { Ok(()) }
    }

    /// `Cstack_info()`'s numbers.
    pub fn c_stack_info(&self) -> CStackInfo {
        let st = self.ctxs();
        let limit = st.cstack_limit.get();
        CStackInfo {
            limit: (limit != usize::MAX).then_some(limit),
            usage: c_stack_usage(st.cstack_start, st.cstack_dir),
            dir: st.cstack_dir,
        }
    }

    /// `R_SignalCStackOverflow(usage)`.
    #[cold]
    #[inline(never)]
    fn signal_c_stack_overflow(&mut self, usage: isize) -> RError {
        let st = self.ctxs();
        // R: errors.c:R_SignalCStackOverflow
        if st.old_cstack_limit.get() == 0 {
            st.old_cstack_limit.set(st.cstack_limit.get());
            st.cstack_limit.set((st.cstack_limit.get() as f64 / 0.95) as usize);
        }
        // (C_SO_msg_fmt, not translated; a NULL call)
        let msg = format!("C stack usage  {usage} is too close to the limit");
        self.error_call_sexp(self.iso().nil, &msg)
    }

    /// GNU R's node-stack check for robigo's value stack: an R entry about
    /// to use `slots` more value-stack slots raises "node stack overflow"
    /// against the current call instead of overflowing (the last resort is
    /// the stack's panic).
    #[inline]
    pub fn check_value_stack(&mut self, slots: usize) -> RResult<()> {
        if self.stack_headroom() >= slots { Ok(()) } else { Err(self.error(NODE_SO_MSG)) }
    }

    /// `R_interrupts_suspended`.
    pub fn interrupts_suspended(&self) -> bool {
        self.ctxs().intr_suspended.get()
    }

    /// Suspend or resume interrupts (`BEGIN_SUSPEND_INTERRUPTS`); a record
    /// saves the state when it is pushed and an exit's stop restores it.
    pub fn set_interrupts_suspended(&mut self, on: bool) {
        self.ctxs().intr_suspended.set(on);
    }

    /// Mark a user interrupt pending (`R_interrupts_pending = 1`, what the
    /// SIGINT handler does): the next poll takes it.
    pub fn set_interrupt_pending(&mut self) {
        self.ctxs().intr_pending.set(true);
    }

    /// `R_CheckUserInterrupt()`: [`Cx::check_c_stack`], then, unless
    /// interrupts are suspended, take a pending interrupt: an exit of kind
    /// [`ExitKind::Interrupt`] to the top level (`onintr`; signalling the
    /// `interrupt` condition to calling handlers and the console's newline
    /// are S1's, before this exit).
    pub fn check_user_interrupt(&mut self) -> RResult<()> {
        self.check_c_stack()?;
        let st = self.ctxs();
        // R: errors.c:R_CheckUserInterrupt, onintrEx
        if st.intr_suspended.get() || !st.intr_pending.get() {
            return Ok(());
        }
        st.intr_pending.set(false);
        Err(self.exit(Exit::new(ExitKind::Interrupt)))
    }
}

/// `R_CStackDir * (R_CStackStart - (uintptr_t) &dummy)`: the C stack in
/// use, measured at the caller's frame.
#[inline(always)]
fn c_stack_usage(start: usize, dir: isize) -> isize {
    let dummy = 0u8;
    let here = std::hint::black_box(&dummy) as *const u8 as usize;
    dir * (start as isize).wrapping_sub(here as isize)
}
