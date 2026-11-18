//! Dead-slot poisoning (C1-PZ, `ROBIGO_POISON_DEAD=1`, robigo-rt's
//! `poison-dead` feature): a register cleared before its last read is
//! caught where it is read, naming the register and the clearing
//! instruction, in the VM and in the evaluators that run compiled code
//! outside it. The early clear is planted by the compiler
//! (`ROBIGO_POISON_PLANT=1`: each closure's value register emptied just
//! before its Ret).
//!
//! One test, as the two variables are process-wide: the poison's is read
//! once, the plant's at each compile.

#![cfg(feature = "poison-dead")]

mod common;

use common::*;
use robigo_rt::Profile;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn a_planted_early_clear_is_caught() {
    // SAFETY: this test binary's only test, run before any other thread of
    // it reads the environment.
    unsafe { std::env::set_var("ROBIGO_POISON_DEAD", "1") };
    for p in [Profile::Bc, Profile::Eval] {
        // the code as compiled: no dead slot is read
        let mut r = Session::new(p);
        assert_eq!(r.val("f <- function(x) { y <- x * 2; y + 1 }; f(1)"), real(3.0));
        assert_eq!(r.val("s <- 0; for (i in 1:3) s <- s + i; s"), real(6.0));
    }
    // SAFETY: as above.
    unsafe { std::env::set_var("ROBIGO_POISON_PLANT", "1") };
    for p in [Profile::Bc, Profile::Eval] {
        let mut r = Session::new(p);
        let e = catch_unwind(AssertUnwindSafe(|| r.val("g <- function(x) { y <- x * 2; y + 1 }; g(1)")))
            .expect_err("the planted clear was not caught");
        let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            msg.starts_with("robigo: ROBIGO_POISON_DEAD: a read of a dead slot: register ")
                && msg.contains(", cleared by instruction "),
            "{p:?}: {msg}"
        );
        // the isolate was left mid-evaluation by the panic
        std::mem::forget(r);
    }
    // SAFETY: as above.
    unsafe { std::env::remove_var("ROBIGO_POISON_PLANT") };

    // the evaluators that run compiled code outside the VM (the inline
    // body evaluator, `pure` and `early::run` for arguments): clean code runs
    // there under the poison, with their deaths made and their reads checked
    let mut r = Session::jit();
    r.val("f <- function(x) x + 1; g <- function() { s <- 0; for (i in 1:20) s <- s + f(i + 1); s }");
    for _ in 0..3 {
        r.val("g()");
    }
    let n0 = robigo_rt::inline_runs(r.cx());
    assert_eq!(r.val("g()"), real(250.0));
    assert!(robigo_rt::inline_runs(r.cx()) > n0, "the warm loop runs f inline under the poison");
    // and the deaths they make: an operand read for the last time is poison
    // to the next read (a plant cannot reach them first: the JIT runs a
    // closure's first compiled call, and a caller its arguments' code, on
    // the VM)
    let mut regs = [robigo_rtcore::Slot::int(1), robigo_rtcore::Slot::int(2)];
    robigo_rt::poison_deaths_of_last_read(&mut regs, 1, 0);
    regs[1].check_live();
    let e = catch_unwind(|| regs[0].check_live()).expect_err("the dead operand's read was not caught");
    let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("register 0, cleared by instruction 7") && msg.contains("early.rs"), "{msg}");
}
