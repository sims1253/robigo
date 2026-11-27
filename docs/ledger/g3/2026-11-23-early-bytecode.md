# A translated call's arguments computed early (step 1a, 2026-11-23)

X/P1. A translated call (`Op::CallCodes`, byte code from GNU R's compiler)
passes each argument as its promise code, a BCODESXP. `early::value`, which
computes an argument before the call when that cannot be observed (docs/g3.md
§6.2), knew symbols and calls only. So every translated call lost early
evaluation. Under C1's ROBIGO_V4_PROBE (user closures compiled by cmp.R and
translated; class A of C1's analysis of translated code, P3), plain went
from 1,895 to 5,187 instructions an iteration.

## What changed

- **The arm.** `early::value` takes a BCODESXP. Its translation is the one
  an earlier force made: `code::cached` on the byte code's 4th word, where
  `compile::bytecode_code` keeps it. A translation cache's stub is no code
  object, so nothing is computed before the first force has translated.
  The code is run by `early::run`, the same evaluator and check as a call's.
  It gives up where eval.rs's BCODESXP arm would evaluate the source instead:
  the Eval profile forced, byte code disabled, or a version bcEval does not
  run. PRCODE is unchanged, and so is what a promise holds.
- **The counts.** Forcing a BCODESXP promise makes two counts: `eval`'s
  evaluation count (eval.c:1085-1099, robigo's `tick`) and then bcEval's
  entry poll (eval.c:7524-7525 with 6237-6243, `bc_tick`), plus one
  evaluation level (eval.c:1146-1147). A symbol makes the first only, a
  compiled call the second only. The force hook now carries the
  expression's SEXPTYPE instead of a call flag (rtcore's `EarlyForceFn`).
  `early_force` makes both counts for byte code, or neither when either
  would poll or the depth would pass `expressions`. S1 read it against
  GNU R's forcePromise → eval → bcEval path.
- **Early-pure code is marked once.** `code::build` marks code that is all
  `early::run` runs: constant loads, variable reads, arithmetic, clears, a
  visible `Ret`, at most its registers, no tick or depth step. The mark is
  `ir::MARK_EARLY_PURE`, beside 1b's `MARK_SINGLE_READ` in the header's
  mark byte, `Header::marks`. The byte-code arm refuses unmarked code before
  it reads any variable.
- **A decline for good is learned.** A byte-code argument declines for good
  when its translation exists and either is not early-pure or its run
  refused. Base passes its formals on, unforced, on every call. The site's
  plan learns such a decline, as it learns wasted values (`Plan::no_early`),
  so later calls skip it. A missing translation is never learned, since the
  next call may take the value.

## What it cost, and how each was removed

Each found by a per-function callgrind diff of the row against main (N
against 2N, the function table only), on one machine:

| step | what | removed by |
|---|---|---|
| named +0.60% (b8444b74a) | the larger `early_force` no longer inlined into the inliner's `enter` | the inliner keeps the two-way `early_force_ast` (CallCodes never reaches it; a debug_assert says so), the byte-code case out of line |
| binarytrees +0.49% | a second call of `early::run` kept it out of `early::value` | `run` `#[inline(always)]`, the arms separate |
| binarytrees +0.103% | the byte-code arm's body, inlined, moved the call arm's code | the body out of line (`bytecode_value`) |
| superassign +3.76% (probe) | `i %% 7` translates to a builtin call: read, then refused | the early-pure mark |
| lapply +0.25%, map +0.41% (rdb) | base's arguments read to no end | the arm's own checks after the run; declines learned |

The hook's remaining cost is one type test at its entry (`cmp $0x15`), with
the byte-code case a tail call to a cold function (objdump).

## Measured

main 8afcad0c6 against 8d0181ceb, release builds, callgrind instructions
per iteration.

**The gate's rows (`perf.py rt-bars`), both boots:** 0 failures.
- Every bar row is within ±0.011% except binarytrees +0.063% under both
  boots and s3 +0.049% under rdb (0.000% under m1).
- The guards are within ±0.08% except guard-dots +0.163%. d0-paste and
  d0-paste0 are +0.075%, d0-unlist +0.073%.
- binarytrees and guard-dots pay the decline's type test in `early_args`
  (a symbol's or a call's decline).

**The jit rows (`jit-bars.toml`):** 0 failures.
- Under rdb, −2.05% to −2.20% a compile (jit-scale_by −2.048%, jit-area
  −2.175%). The compiler itself is translated byte code there, so its own
  calls now take early values.
- Under m1, where the compiler is robigo's compile of its source, +0.04% to
  +0.14%: the decline's type test.

**Under the probe** (`ROBIGO_V4_PROBE=1`, rdb; every row R_VSIZE=1G,
R_NSIZE=20M on both binaries, since fib's window collected otherwise):

| row | main | 1a | change |
|---|---:|---:|---:|
| fib | 34,857,073 | 30,002,609 | −13.93% |
| plain | 5,209.7 | 4,601.6 | −11.67% |
| defaults | 6,314.8 | 5,707.7 | −9.61% |
| env-fn | 4,526.5 | 4,156.3 | −8.18% |
| named | 4,868.5 | 4,499.3 | −7.58% |
| binarytrees | 5,263,358 | 4,939,550 | −6.15% |
| superassign | 9,371.4 | 9,314.4 | −0.61% |
| d0-unlist | 11,415.9 | 11,360.9 | −0.48% |
| s3 | 16,690.5 | 16,619.5 | −0.43% |
| guard-dots | 8,963.7 | 8,978.7 | +0.17% |
| d0-paste, d0-paste0 | | | +0.08% |

The 15 other rows are within ±0.05%. Under the probe, plain went from 1,895
to 5,187 instructions; it is 4,602 now. What remains is the inliner on
CallCodes, C1's step 2.

**The apply rows under rdb**, no probe (every row R_VSIZE=1G, R_NSIZE=20M:
sapply's and mapply's windows collected otherwise), with the translation
cache on and off:

| row | cache on | cache off |
|---|---:|---:|
| lapply | −0.06% | −0.06% |
| vapply | −0.16% | −0.06% |
| sapply | −0.42% | −0.46% |
| mapply | −0.48% | −0.40% |
| map | +0.08% | +0.08% |
| do-call | +0.17% | +0.17% |

The early path's effect is the same with the cache on and off. With the
cache on, the first force materializes the translation from its stub, and
the calls after it take the early value.

## The cases

- **`crates/robigo-rt/tests/early_bytecode.rs`.** GNU R 4.6.2's compiled
  closures, rebuilt with `mkCode`/`bcClose` (the crate's sessions have no
  compiler), run with early byte code on and off (`set_early_bytecode`).
  The value and both counters are equal, with at least 500 byte-code
  arguments taken early. The hook declines one count before each poll and
  at `expressions`. Each half of the byte-code counts removed fails it.
- **`tests/rt/jit/early-bytecode-depth.R`**, recorded on 4.6.2. Under
  `expressions = 100`, a byte-code promise argument costs its force's one
  level against a constant (86 against 87), with GNU R's error text. It
  matches GNU R under both boots, before and after 1a.
- **`tests/rt/jit/single-read-promise.R`**: its depth probe is now at
  `expressions = 500` with `deep(1000)`. That is the same error path at a
  tenth of the depth, for the gctorture shard's time. Its recorded output is
  unchanged.

## Not here (recorded for the fold where `force_arg` learns the read's kind)

1. GNU R has a second path for these promises, bcEval's inlined force:
   DO_GETVAR → START_BCFRAME_PROM (eval.c:5944-5947; setup_bcframe_prom,
   7455). It takes the depth step and R_Visible, but neither eval's count
   nor bcEval_loop's entry poll; only START_BCFRAME_CALL polls (7404).
   robigo-rt's ordinary force counts both there. Early and ordinary agree
   with each other, and both overcount against GNU R when a compiled callee
   reads the argument (poll timing only). #1008's `run_single_read` is on
   that path and makes the same counts as `vm::exec`. (S1.)
2. `early_force` checks the depth but not `R_CheckStack`, which GNU R's
   `eval` runs at that step. This predates 1a for symbols and calls. (S1.)
3. `Cx::force_arg`'s early branch returns before its `safepoint(Entry)`. B's
   `force_x_i` (#1018) declines on pending safepoint work and checks the C
   stack's headroom. (X, from #1018's review.)

Also here: B's doc move (5611463ec), #1018's recorded low.
`promise_unforced_ast` and `set_promise_forced` now sit above
`force_promise`'s doc block, so that doc and its `// R: eval.c:forcePromise`
attach to `force_promise` again.
