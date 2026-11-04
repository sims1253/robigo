# perf-oop: closure- and object-heavy programs

Scope: the programs where robigo lost at peak to FastR and Ř (and on some
to GNU R's steady state): `awf/Bounce`, `Bounce_nonames`,
`Bounce_nonames_simple`, `Storage`, `shootout/binarytrees_2`,
`binarytrees_naive`. Metric: `scripts/rivals.py` peak wall time against
GNU R, Ř and FastR; gate: `scripts/perf.py compare` of instruction counts
(no benchmark more than 0.5% slower) and conformance in all tiers.

Measuring on these VMs: the per-iteration numbers below come from
`perf stat -e cycles:u` of one process running the program body K times
minus a process running it 0 times (best of 3; this VM's user clock is
about 4.3 GHz). They track rivals.py's peak within ~10%. rivals.py itself
gives this VM's rival numbers, about 1.5x slower than the scoreboard
VM's (`~/oopwork/rivals-here.json`):

| benchmark | GNU R | Ř | FastR | robigo (main bf7332ee) |
|---|---|---|---|---|
| Bounce | 0.0437 | 0.0315 | 0.177 | 0.0335 |
| Bounce_nonames | 0.0445 | 0.0154 | 0.0117 | 0.0288 |
| Bounce_nonames_simple | 0.0318 | 0.00831 | 0.00788 | 0.0250 |
| Storage | 0.046 | 0.0239 | 0.00409 | 0.0355 |
| binarytrees_2 | 0.0191 | 0.015 | 0.0103 | 0.0119 |
| binarytrees_naive | 0.0184 | 0.0135 | 0.00831 | 0.0129 |

## 2026-11-04: nested arithmetic as call arguments (binarytrees 4x)

- Why: binarytrees' `tree()` was compiled to native code (round 20) but never
  ran there: `tree(2L * item - 1L, depth - 1L)` has an argument that is
  arithmetic of arithmetic, and inline/native call arguments (`InlineArg`)
  were a leaf, one operator over two leaves, or an element read. The site
  left native code, so every run was abandoned and after 128 misses the
  body went back to the VM for good. `check()`, whose arguments are `t[[2]]`,
  ran at 46 cycles per node; `tree()` at 3,400.
- Change: `NativeArg` (compile.rs), a call argument that is arithmetic whose
  operands are leaves or arithmetic again, up to three operators deep, or
  unary minus (`-i`, as `Op::Neg`: `-0` stays `-0`, `-NA` stays NA, `-TRUE` is
  `-1L`). A call site whose arguments need it gets `CallSite::native_args`
  (`inline_args` stays `None`), which only `jit::closure::call_tree` reads:
  inside a native tree such an argument is `CallArg::Nested`, typed and
  generated recursively, each operator's function guarded at entry like the
  bodies' and every operator by its scalar fast path, leaving native code
  where that gives up (overflow, NA where a warning is due, a value that is
  not a plain scalar). The VM makes these calls as before, with promises.
- Rejected on the way, by the gate:
  - `InlineArg::Nested`, evaluated by the VM too (its inline path, its native
    entry, `caller_promargs`): `robigo/closures` +1.08% and
    `spectralnorm_naive` +0.74% without running any of it. With the new arm
    `site_arg` was in a recursive cycle and LLVM stopped inlining
    `run_inline_with` into `inline_site_call` (+37 instructions per inline
    call). A separate evaluator for nested operands plus `#[inline(always)]`
    on the runner and its `read` got `closures` to +0.31%, but then
    `robigo/apply` +0.77%, `lapply` +0.51%; with the VM declining nested
    arguments (an arm returning `None`) `profiler-shared` was still +0.67%
    (`native_site_call`, +17 instructions per call). A separate type leaves
    the VM's code as it was: worst +0.03% on all four.
  - `convolution_v` stays at +0.8%: all of it is mimalloc zeroing large
    blocks (`_mi_page_malloc_zero`, 160 instead of 161 calls), the heap's
    layout after compiling base code with one more field per call site, not
    work.
- Result (cycles per program body, this VM): binarytrees_naive 9.80 -> 2.49 ms,
  binarytrees_2 9.43 -> 3.00 ms (FastR 8.31 / 10.3, Ř 13.5 / 15.0). Instructions,
  startup included: binarytrees_2 -25.7%, binarytrees_naive -25.8%.
- Verified: `tests/r/jit/nested-arith-args.R` (trees, nested and unary
  arguments in pure closures, signed zeros, NA and logicals under unary minus,
  integer overflow inside a nested argument, never-forced arguments that would
  warn or fail, errors from forced ones, `%%`/`%/%`, operators rebound after
  compilation), recorded with GNU R 4.5.0, byte-identical in all tiers.
- Kept.
