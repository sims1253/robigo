//! A translated call's promise code (a BCODESXP, `Op::CallCodes`'s
//! arguments) computed early (`early::value`, step 1a): a force that takes
//! the early value makes the counts the ordinary force makes, `eval`'s
//! evaluation count and `bcEval`'s entry poll count, and the early path
//! declines where either would poll or the depth would pass
//! `R_Expressions` (S1's conditions on step 1a).

mod common;

use common::*;
use robigo_heap::types::{BCODESXP, LANGSXP, SYMSXP};
use robigo_rtcore::Cx;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// rtcore's `hooks::EarlyForceFn`.
type EarlyForceFn = fn(&mut Cx, bool, u8) -> bool;

/// The installed `early_force`, which the counting wrapper calls.
static REAL: OnceLock<EarlyForceFn> = OnceLock::new();
/// The forces that took a byte-code object's early value (the hook runs
/// only for a slot holding one).
static BC_EARLY: AtomicU64 = AtomicU64::new(0);

fn counting(cx: &mut Cx, bytecode: bool, kind: u8) -> bool {
    let took = (REAL.get().expect("the real hook"))(cx, bytecode, kind);
    if took && kind == BCODESXP {
        BC_EARLY.fetch_add(1, Ordering::Relaxed);
    }
    took
}

/// The run's result and the counts it made from zero.
fn counts(r: &mut Session, src: &str) -> (Result<(Val, bool), String>, i32, i32) {
    let cx = r.cx();
    cx.evalcount().set(0);
    cx.bc_evalcount().set(0);
    let res = r.run(src);
    let cx = r.cx();
    (res, cx.evalcount().get(), cx.bc_evalcount().get())
}

/// `id`, `f` and `rd` as GNU R 4.6.2's compiler makes them (their bodies
/// BCODESXPs, so their calls are translated `CallCodes` whose arguments are
/// byte-code objects), rebuilt from `compiler::disassemble`'s opcodes with
/// `mkCode` and `bcClose`, as cmp.R's assembler does (the crate's sessions
/// have no compiler namespace). Made from
///
/// ```r
/// id <- compiler::cmpfun(function(a) a)
/// f <- compiler::cmpfun(function(x) { y <- x; id(y + 1) })
/// rd <- compiler::cmpfun(function(x) { y <- x; id(y) })
/// ```
///
/// (the `expressionsIndex` constants unclassed: they only place errors).
/// The arguments read a local `y`, a value: `x` itself is a promise not
/// yet forced, which no early evaluation reads.
const PROGRAM: &str = r#"
id <- .Internal(bcClose(.Internal(formals(function(a) NULL)), .Internal(mkCode(c(12L, 20L, 0L, 1L), list(quote(a), c(NA, 0L, 0L, 0L)))), globalenv()))
f <- .Internal(bcClose(.Internal(formals(function(x) NULL)), .Internal(mkCode(c(12L, 20L, 1L, 22L, 2L, 4L, 23L, 4L, 29L, 6L, 38L, 5L, 1L), list(quote({
    y <- x
    id(y + 1)
}), quote(x), quote(y), quote(y <- x), quote(id), quote(id(y + 1)), .Internal(mkCode(c(12L, 20L, 1L, 16L, 2L, 44L, 0L, 1L), list(quote(y + 1), quote(y), 1, c(NA, 1L, 1L, 2L, 2L, 0L, 0L, 0L)))), c(NA, 1L, 1L, 3L, 3L, 3L, 5L, 5L, 5L, 5L, 5L, 5L, 5L)))), globalenv()))
rd <- .Internal(bcClose(.Internal(formals(function(x) NULL)), .Internal(mkCode(c(12L, 20L, 1L, 22L, 2L, 4L, 23L, 4L, 29L, 6L, 38L, 5L, 1L), list(quote({
    y <- x
    id(y)
}), quote(x), quote(y), quote(y <- x), quote(id), quote(id(y)), .Internal(mkCode(c(12L, 20L, 0L, 1L), list(quote(y), quote(id(y)), c(NA, 1L, 1L, 1L)))), c(NA, 1L, 1L, 3L, 3L, 3L, 5L, 5L, 5L, 5L, 5L, 5L, 5L)))), globalenv()))
g <- function(n) { s <- 0; for (i in 1:n) s <- s + f(i) + rd(i); s }
"#;

#[test]
fn a_byte_code_argument_forced_early_counts_as_the_ordinary_force() {
    let mut r = Session::jit();
    let cx = r.cx();
    let mut h = cx.hooks();
    let _ = REAL.set(h.early_force);
    h.early_force = counting;
    cx.install_hooks(h);
    r.val(PROGRAM);
    // the first forces translate the promise code; the JIT compiles g
    for _ in 0..3 {
        r.val("g(5)");
    }
    BC_EARLY.store(0, Ordering::Relaxed);
    let on = counts(&mut r, "g(300)");
    let taken = BC_EARLY.swap(0, Ordering::Relaxed);
    robigo_rt::set_early_bytecode(r.cx(), false);
    let off = counts(&mut r, "g(300)");
    let taken_off = BC_EARLY.swap(0, Ordering::Relaxed);
    robigo_rt::set_early_bytecode(r.cx(), true);
    assert_eq!(on, off, "the value and eval's and bcEval's counts, early values on and off");
    // both calls' arguments, `x + 1` and `x`, are byte-code objects taken
    // early in every iteration but the counts' own polls
    assert!(taken >= 500, "byte-code arguments taken early: {taken}");
    assert_eq!(taken_off, 0);
}

#[test]
fn the_early_path_declines_where_the_force_would_poll_or_overflow() {
    let mut r = Session::jit();
    let cx = r.cx();
    let force = cx.hooks().early_force;
    let depth = cx.eval_depth();
    let check = |cx: &mut Cx, kind: u8, e: i32, b: i32| -> (bool, i32, i32) {
        cx.evalcount().set(e);
        cx.bc_evalcount().set(b);
        let took = force(cx, true, kind);
        (took, cx.evalcount().get(), cx.bc_evalcount().get())
    };
    // a byte-code object: eval's count (polls past 1000) and bcEval's (past
    // 1023), both or neither
    assert_eq!(check(cx, BCODESXP, 999, 1022), (true, 1000, 1023));
    assert_eq!(check(cx, BCODESXP, 1000, 0), (false, 1000, 0));
    assert_eq!(check(cx, BCODESXP, 0, 1023), (false, 0, 1023));
    // a call: bcEval's count only; a symbol: eval's only (unchanged by 1a)
    assert_eq!(check(cx, LANGSXP, 1000, 1022), (true, 1000, 1023));
    assert_eq!(check(cx, LANGSXP, 0, 1023), (false, 0, 1023));
    assert_eq!(check(cx, SYMSXP, 999, 1023), (true, 1000, 1023));
    assert_eq!(check(cx, SYMSXP, 1000, 0), (false, 1000, 0));
    // the depth: one level below `expressions` takes it, at it declines
    let n = cx.expressions();
    cx.set_eval_depth(n - 1);
    assert_eq!(check(cx, BCODESXP, 0, 0), (true, 1, 1));
    cx.set_eval_depth(n);
    assert_eq!(check(cx, BCODESXP, 0, 0), (false, 0, 0));
    cx.set_eval_depth(depth);
    // not byte code's slot: never
    assert!(!force(cx, false, BCODESXP));
}
