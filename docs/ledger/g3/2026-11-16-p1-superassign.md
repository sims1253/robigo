## 2026-11-16: P1 step 8, `superassign`: closures GNU R never compiles run inline, with `<<-` as a buffered write

The foreman's first item after D5: run `superassign`'s callee inline, with
`<<-` as a buffered write and a whitelist of pure builtins starting with
`invisible`. The gap entry's plan is `2026-11-16-p1-gaps.md`. Step 6's
conditions apply (`2026-11-16-p1-inline.md`).

The gap entry missed one cause. The callee,
`dep <- function(x) { bal <<- bal + x; invisible(bal) }`, is small and
made inside `acct()`. GNU R's `R_CheckJIT` therefore marks it NOJIT (not
top level, score under `MIN_JIT_SCORE`) and runs its AST every time.
robigo-rt does the same and runs it as `Eval` code. The inliner took byte
code only, so this step also runs `Eval` code inline.

- Base: main at `324cbf91` (the code of `4e4c5b03`, whose runs
  `2026-11-16-d5-recycle.md` archives as its base). Change: branch
  `g3/p1-superassign` (`robigo-rt`, `robigo-rtcore`), measured at
  `2591f576`. The head adds only a test change.
- What changes (`crates/robigo-rt/src/inline.rs`):
  - **`Eval` code inline.** A closure's `Eval` code runs inline when the
    JIT will never compile it (NOJIT, or the JIT off). Skipping the real
    call's `R_CheckJIT` then changes nothing; a closure still Unseen or
    MAYBEJIT makes real calls. `Eval` code counts each evaluation, and the
    inline run makes those counts from the same instruction flags as the
    VM. Its entry has no bcEval tick, and its depth starts at the call's.
    - A statement boundary (`Resume`) gives up when the code is stale (its
      source was written).
    - A guard that saves the protect top stores it, and `PpCheck` gives up
      if the top moved.
    - It calls builtins `pure_builtin` knows, as `eval` does: `evalList`,
      `R_Visible` from `PRIMPRINT`, and the safepoint on return, which
      gives up if one is pending. That is `invisible` with one unnamed
      argument. Closures are never called from it inline, so the verdict
      admits a generic call only of a known builtin, or of a guarded
      function (the guard's fallback).
    - The call hook now reaches closures without GNU R's byte-code bit.
  - **`<<-` as a buffered write.** At `SetVar2`:
    1. The value is boxed as the VM's `set_var` boxes it, without
       collecting. A box refused for a due collection gives up.
    2. `Cx::set_var_target_raw` (rtcore) finds the binding `setVar` would
       write. None, a locked binding or an active one gives up.
    3. The write is logged.

    The run's later reads of the name from the same enclosure return the
    logged value. Any other read of the name (another enclosure, the
    caller's frame) or a lookup of it as a function gives up. Reads never
    take a caller's early values. A run that ends commits the log in order
    (`Cx::set_var_at`: the effect note, the global environment's dirty
    mark, the store). A give-up drops it, so the real call writes once.
    The value returned is the bound box, as the real call's is.
  - **Byte code too.** A compiled function's inner closures are compiled
    with it, and their `invisible(x)` is `BaseGuard` + `CallBuiltin`.
    Both run inline. The guard's fallback (`EvalConst`) gives up.
  - **What stays as fast.** `body` is inlined into `enter` again, the new
    arms are cold helpers, and frames carry the isolate's state pointer,
    fetched once per call site.
- Admission at `38461a89`: `scripts/test-rt-ci.py` and `scripts/rt-ci.py`
  pass every step (conformance 256/261, main's 5 unratcheted failures).
  `cargo +nightly-2026-11-02 miri test -p robigo-rtcore`, `scripts/lint.sh`
  and `cargo fmt --check` pass too. Under gctorture every `<<-` run gives
  up, because its box is refused, so the tests assert their run counts
  only without it.
- Commands as in step 2 (`rt-bars.sh` twice, `s3` three more times,
  `rt-bars-compare.py --a main ... --b superassign ...`).
- Raw JSON: `data/2026-11-16-p1superassign-2591f576/run{1,2}-*.json` and
  `run{3,4,5}-s3.json`, with the base's runs as `base-run*`.

Instructions and estimated cycles per iteration (medians of two runs;
`s3` of five):

| row | main instr | superassign instr | ÷ | main est. cycles | superassign est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,221.1 | 2,235.1 | 1.0063 | 2,224.2 | 2,238.2 | 1.0063 |
| defaults | 2,253.6 | 2,279.6 | 1.0115 | 2,256.8 | 2,282.6 | 1.0114 |
| named | 1,980.8 | 1,980.7 | 1.0000 | 1,983.2 | 1,983.2 | 1.0000 |
| env-fn | 2,586.2 | 2,592.1 | 1.0023 | 2,588.8 | 2,595.6 | 1.0026 |
| superassign | 8,836.2 | 5,155.3 | 0.5834 | 9,396.9 | 5,260.1 | 0.5598 |
| s3 (median of five) | 32,137.5 | 32,141.3 | 1.0001 | 37,696.7 | 37,677.1 | 0.9995 |
| fib | 17,749,678.3 | 17,899,577.7 | 1.0084 | 17,882,969.0 | 18,038,099.7 | 1.0087 |
| binarytrees | 6,224,506.3 | 6,221,448.6 | 0.9995 | 6,690,162.2 | 6,632,683.3 | 0.9914 |

The geomean is 0.9382 in instructions and 0.9324 in estimated cycles.
`superassign` allocates 1.027 objects per iteration, down from 4: the
committed box, plus a real call's frame and argument list for the 0.7% of
iterations where a collection is due (the box was refused, or the builtin's
safepoint had work). The other rows' allocations are unchanged. No row
spreads more than 0.005% in instructions or 0.36% in estimated cycles over
its runs.

E1 after this step: `superassign` 0.69x GNU R 4.6.2's 7,480 instructions
(bar 1.0x, met).

The rows that already ran inline pay 0 to 1.2%: `fib` 18 instructions per
inline call, `defaults` 26 per iteration. That is the write log's checks
on reads, guards and function lookups (one load and compare each while the
log is empty), the frame's two more words, and the hook reaching closures
without the byte-code bit. On the way there, the larger interpreter loop
had stopped being inlined into `enter` (+8.1% on `fib`), which forcing it
inline, cold helpers for the new instructions, and one fetch of the
isolate's state per call site brought back.

Checks of step 6's conditions:
- **Condition 1.** The Rust differential test (`crates/robigo-rt/tests/inline.rs`) compares with inlining on and off:
  - one and two writes;
  - a write before an integer-overflow warning, and before an error;
  - a global written, and a global the first write defines;
  - the visibility of an `invisible` result;
  - locked and active targets, which no call runs inline;
  - the depth limit with an inlined `Eval` body innermost.
- **Condition 2.** Recorded on GNU R 4.6.2:
  - `tests/rt/inline/superassign.R`: the row's account; visibility at top level through a compiled caller; copy-on-modify of the value returned; two writes; a write before a warning and before an error, each written once; globals; a caller's local of the written name; and the caller's argument forced after a write of the variable it reads.
  - `tests/rt/capi/inline-eval-body.R`: package C writes the body's statements after the calls ran inline, and the next calls run the new statements, as GNU R's AST does.
- **Condition 3.** As admitted above.

Review: Sol, at `2591f576`, asked for one change: the new tests asserted
inline runs under gctorture, where collection pressure rightly makes the
runs give up. Fixed in `38461a89`. Sol found no difference from GNU R
4.6.2 in the inline and C API cases, under gctorture with the moving
collector, or in its own scripts.
