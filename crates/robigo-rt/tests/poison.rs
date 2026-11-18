//! Dead-slot poisoning (C1-PZ, `ROBIGO_POISON_DEAD=1`, robigo-rt's
//! `poison-dead` feature): a register cleared before its last read is
//! caught where it is read, naming the register and the clearing
//! instruction. The early clear is planted by the compiler
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
}
