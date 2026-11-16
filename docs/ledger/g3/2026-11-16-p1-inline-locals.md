## 2026-11-16: P1 step 7, inlined bodies with locals: a closure that assigns locals runs inline

The seventh P1 family (step 6: `2026-11-16-p1-inline.md`), which the
foreman asked for next as the cheapest step to more of E1. The `named`
row's callee, `weigh <- function(x, y) { z <- x * y; z + 1 }`, assigns a
local, so step 6 rejected it. Step 6's conditions apply to this step
too.

- Base: `g3/p1-inline` at `0b2d3497` (#695 after #693's main merge; its
  head `6c74110a` adds only a ledger note). Change: branch
  `g3/p1-inline-locals` at `2e5776dc` (`robigo-rt`).
- What changes (`crates/robigo-rt/src/inline.rs`):
  - **The verdict.** It takes a frame shape whose first slots are the
    formals, in order (`code::make_shape` puts them first), then locals,
    up to 4 slots, and a `SetVar` into a local's slot. The formal count is
    kept in the header's new `inline_formals` byte.
  - **The frame.** A slot is an argument, a value, or `Arg::Unbound`: a
    local not yet assigned. Its read goes on to the enclosure, as in a
    real frame that does not bind it yet. That holds for the body's
    `GetVar` and for argument code reading through the frame.
  - **`SetVar`.** It stores the register's slot and turns `R_Visible`
    off, as the VM's `set_var` does. A scalar stays unboxed, as `set_var`
    keeps it in a slot frame.
  - **Not a formal.** A body that assigns one of its formals does not
    qualify. The real call binds the formal to a promise, of the caller's
    argument or of the formal's default. The store drops that promise from
    the frame, and the dead promise keeps its references to its expression
    and, once forced, to its value. GNU R's cleanup
    (`eval.c:R_CleanupEnvir`, `clearPromise`) releases them only for
    promises the frame still binds. An inline run would make none of
    those references, so an overwritten default or argument would stay
    unshared where GNU R's is shared (Sol's review, below).
- Also here, from two OpenCodeReview threads on #695 that the foreman
  deferred to this step: `Cx::fun_lookup_from_raw` (step 6's global-cache
  read for `GetFun`) is renamed `Cx::fun_lookup_cached_raw`, and its doc
  states the condition first. A rename only; the code is unchanged.
- On the way: with 6 slots, the rows that already inlined paid up to 46
  instructions per call for the larger frame array (`fib` +2.2%, `plain`
  +1.3%, `defaults` +1.0%, `env-fn` +0.6%, against step 6's `6e647203`;
  bars at `03782888`, raw JSON `run{1,2}-*` of
  `data/2026-11-16-p1locals-03782888/`, its base's as `base-run*`). With 4
  slots they pay 0 to 0.4%. `03782888` is also the base of the D4, D2 and
  D5 re-runs (`2026-11-16-p1-reruns.md`).
- Admission at `2e5776dc`: `scripts/test-rt-ci.py` and `scripts/rt-ci.py`
  pass every step (conformance 254/259, main's 5 unratcheted failures),
  and so do `cargo +nightly-2026-11-02 miri test -p robigo-rtcore`,
  `scripts/lint.sh` and `cargo fmt --check`.
- Commands as in step 2 (`rt-bars.sh` twice, `s3` three more times,
  `rt-bars-compare.py --a inline ... --b locals ...`).
- Raw JSON: `data/2026-11-16-p1locals-2e5776dc/run{1,2}-*.json`,
  `run{3,4,5}-s3.json`; the base's as `base-run*`.

Instructions and estimated cycles per iteration (medians of two runs;
`s3` of five):

| row | inline instr | locals instr | ÷ | inline est. cycles | locals est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,220.0 | 2,221.0 | 1.0004 | 2,223.1 | 2,224.2 | 1.0005 |
| defaults | 2,255.7 | 2,253.5 | 0.9991 | 2,258.7 | 2,256.6 | 0.9991 |
| named | 4,924.0 | 1,980.9 | 0.4023 | 5,261.3 | 1,983.3 | 0.3770 |
| env-fn | 2,586.2 | 2,586.1 | 1.0000 | 2,588.7 | 2,588.7 | 1.0000 |
| superassign | 8,836.2 | 8,836.2 | 1.0000 | 9,369.0 | 9,396.9 | 1.0030 |
| s3 (median of five) | 32,137.4 | 32,137.5 | 1.0000 | 37,608.9 | 37,676.7 | 1.0018 |
| fib | 17,680,782.9 | 17,749,686.6 | 1.0039 | 17,813,011.4 | 17,882,971.1 | 1.0039 |
| binarytrees | 6,224,508.4 | 6,224,506.7 | 1.0000 | 6,677,255.4 | 6,690,161.7 | 1.0019 |

The geomean (with the two-run `s3`) is 0.8928 in instructions and 0.8862
in estimated cycles. `named` allocates 0.001 objects per iteration (was 1,
its frame). The other rows' allocations are unchanged. On every row the
two runs of each binary agree within 0.004% in instructions. `s3`'s
estimated cycles spread 0.45% and 0.25% over its five runs, under the
protocol's 0.5%. `fib` pays 8 instructions per inline call for the slot
frame (an `Arg` with one more variant, the formal count read from the
header).

E1 after this step: `named` 0.447x GNU R 4.6.2's instructions (bar 0.6x,
met). E1's unmet bars and E2's are in `2026-11-16-p1-gaps.md`.

Checks of step 6's conditions:
- **Condition 1.** The Rust differential test
  (`crates/robigo-rt/tests/inline.rs`) gains local-assigning programs,
  run with inlining on and off: a local, a conditional local that falls
  to a global, a reassigned formal, an error with a local. A test checks
  that a body assigning a local runs inline and one reassigning a formal
  does not, and that both leave what the real calls leave.
- **Condition 2.** `tests/rt/inline/locals.R`, recorded on GNU R 4.6.2:
  - shadowing a global;
  - a local assigned on one branch only;
  - a formal reassigned before it is forced (its argument is never
    evaluated), which robigo now runs as a real call;
  - locals as arguments of nested inlined calls;
  - an error and an integer-overflow warning after an assignment;
  - `sys.call`, `environment()` and `match.call` in closures with locals.

  Each case runs three times in a compiled top-level loop. A temporary
  per-callee count showed the local-assigning callees running inline and
  the erring ones giving up.
- **Condition 3.** As admitted above.

Sol's review of `29121238` asked for changes, with two findings, both
fixed:
1. `code::reshape` copied the header, so code judged for one set of
   formals kept its verdict and `inline_formals` for another. Sol's
   reproducer warms `function(x) { if (x) y <- 1; y }` with a global
   `y <- 100`, then appends a formal `y = 7` through package C: robigo
   returned 100 where GNU R returns 7. The fix is #695's (`35cfb9ee`):
   the copy is judged again, which also recomputes `inline_formals`.
2. A `SetVar` over an unforced constant default lost the reference the
   real call's promise keeps (finding and fix above). Sol's reproducer
   installs a vector `x` as the default of a warmed
   `function(a = NULL) { a <- 1; 0 }`, calls it without arguments, then
   removes the default: with inlining, `x[1] <- 5` then wrote `x` in
   place, where GNU R and the real call copy it.

`tests/rt/capi/inline-reshape.R` (recorded on GNU R 4.6.2) gains both
reproducers, the default forced before it is overwritten, and a forced
argument overwritten. With `SetVar` allowed into formals again, its two
default cases print `TRUE` (written in place) where GNU R prints `FALSE`.

Noticed: closures whose bodies assign locals and then call a function
that cannot run inline (the observer cases) now qualify. Each call makes
a partial inline run that gives up at that call. A give-up memo for such
bodies (a nested call whose target is marked not inlinable) would save
that work; it is not measured on the bars' rows.
