//! The protect stack's logical account: GNU R's `R_PPStackTop` as its
//! evaluator would have it, for eval.c's `check_stack_balance` warnings
//! (the foreman's PPACCT decision; docs/g3.md §3, "kept off the hot path"
//! as L7's `gc()` account is).
//!
//! robigo's protect stack holds C's protections only: its evaluator roots
//! in the value stack. GNU R's holds its evaluator's too (`PROTECT(op)` in
//! `eval`, `evalList`'s head, `applyClosure`'s `newrho`, ...), so the depths
//! a C routine's imbalance is reported with are GNU R's evaluator depth plus
//! C's own protections. The account keeps the evaluator's part as an
//! *excess* over `R_EvalDepth`, `E = P - R_EvalDepth`, because byte code
//! never changes it: every transition `bcEval` makes protects one object for
//! each level of `eval` it nests (a closure call's `newrho` and its body's
//! `eval`; a promise's `PROTECT(e)` and its code's `eval`), and calls
//! builtins and specials with neither. Only the AST interpreter's positions
//! (each with a static offset the compiler derives, `robigo-rt`'s `Eval`
//! profile) and the builtins that protect before they evaluate (`lapply`,
//! `do_internal`, a promise's code evaluated by `eval`) move `E`, and each
//! restores it when it returns or an exit leaves it.
//!
//! GNU R's logical top at a point is then `R_EvalDepth + E` plus robigo's
//! real top ([`Cx::pp_top`]), C's protections.

use crate::isolate::Cx;

/// What a check site saved: GNU R's top (`save`) and C's own top then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PpSave {
    /// GNU R's `R_PPStackTop` at the site.
    pub save: i32,
    /// robigo's protect stack's top at the site (C's protections).
    pub cpp: i32,
}

impl Cx {
    /// `E`, GNU R's evaluator protections at the current point minus
    /// `R_EvalDepth`.
    #[inline(always)]
    pub fn pp_excess(&self) -> i32 {
        self.ctxs().pp_excess.get()
    }

    /// Set `E` (the VM at an evaluation point it knows the offset of; an
    /// activation restoring the value it was entered with).
    #[inline(always)]
    pub fn set_pp_excess(&self, e: i32) {
        self.ctxs().pp_excess.set(e);
    }

    /// Run `f` with `delta` more of GNU R's evaluator protections at the
    /// current point (a builtin's `PROTECT`s before it evaluates), `E`
    /// restored after on every path.
    #[inline]
    pub fn with_pp_excess<R>(&mut self, delta: i32, f: impl FnOnce(&mut Cx) -> R) -> R {
        let e = self.pp_excess();
        self.set_pp_excess(e + delta);
        let r = f(self);
        self.set_pp_excess(e);
        r
    }

    /// GNU R's `R_PPStackTop` at the current point with `off` more of its
    /// evaluator's protections, and C's own top: what a check site saves
    /// (`int save = R_PPStackTop`).
    #[inline]
    pub fn pp_save(&self, off: i32) -> PpSave {
        let cpp = self.pp_top();
        PpSave { save: self.eval_depth() + self.pp_excess() + off + cpp, cpp }
    }

    /// eval.c's `check_stack_balance(op, save)` for the primitive named
    /// `name`: C's protections since the site saved `s`, reported with
    /// GNU R's depths (its evaluator's part balanced by then).
    // R: eval.c:check_stack_balance
    #[inline]
    pub fn check_stack_balance(&self, name: &[u8], s: PpSave) {
        let now = self.pp_top();
        if now != s.cpp {
            self.stack_imbalance(name, s.save, s.save + (now - s.cpp));
        }
    }

    #[cold]
    #[inline(never)]
    fn stack_imbalance(&self, name: &[u8], save: i32, now: i32) {
        let mut msg = b"Warning: stack imbalance in '".to_vec();
        msg.extend_from_slice(name);
        msg.extend_from_slice(format!("', {save} then {now}\n").as_bytes());
        self.reprintf(&msg);
    }

    /// The protect stack's top restored to the innermost record's, as the
    /// `longjmp` GNU R's AST interpreter takes to that record restores
    /// `R_PPStackTop` (`R_restore_globals`): `return` leaving a closure's
    /// body, `break` and `next` landing at their loop. robigo's VM leaves
    /// by returning there instead.
    // R: context.c:R_restore_globals
    pub fn pp_restore_innermost(&mut self) {
        let top = self.ctxs().innermost_cstacktop();
        if self.pp_top() != top {
            self.set_pp_top(top);
        }
    }
}
