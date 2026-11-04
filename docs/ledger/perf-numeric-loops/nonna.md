# perf-numeric-loops: integer values known not to be NA in native loops

Stacked on the loop-entry PR (#397). Metric as in
`docs/ledger/perf-numeric-loops.md`.

## 2026-11-04: convolution 1.26 -> 1.05 ms (FastR 1.17), convolution_slow 2.25 -> 1.86 ms

- Why: convolution's inner loop (`ab[i + j] <- ab[i + j] + a[i] * b[j]`)
  is native, but every integer operation tested its operands for NA and
  selected NA for the result, every integer-to-double conversion selected
  NA_REAL, and the loop variables' NA flags were hoisted into registers
  that then spilled. The loop variables cannot be NA.
- What (`jit.rs` `nonna_flow`, `Gen::nn_leaf`): a forward analysis of the
  integer and logical registers and variables that cannot be NA before
  each op: loop variables, constants, checked integer `+ - *` of such
  values (an overflow leaves native code, otherwise NA comes only from an
  NA operand), integer comparisons of them, and variables NA-free at entry
  (`SlotInfo::nonna`: `jit_enter` refuses to enter with NA there). Merges
  intersect; the exhausted branch of a `for` step keeps its facts;
  variables that change type get none; an analysis over its step budget
  gives none. A callee run inline gets its own analysis, its formals
  seeded by the arguments known not to be NA (spectralnorm's
  `eval_A(i, j)`). Codegen then drops those NA tests: integer
  `+ - *`, int-to-double conversions in double arithmetic and comparisons,
  integer comparisons, and `if`/`while` conditions.
- Rejected, measured: loading vectors' data pointers and lengths from
  their views where used instead of holding them in variables (to relieve
  the register pressure): -4% cycles on convolution but `vectorFor` +3.4%
  and `convolution_slow` +0.65% instructions. Using Cranelift's
  `sadd_overflow` for the overflow test: convolution +2.7% instructions.
- Result, peak (rivals.py): convolution 1.26 -> 1.05 ms (FastR 1.17, Ř
  1.53), convolution_slow 2.25 -> 1.86 ms (FastR 2.42); cycles -17%
  on both. Instructions against main: 15 faster, 0 slower; against #397:
  convolution -2.6%, convolution_slow -4.0%, robigo/mandelbrot -2.4%,
  robigo/spectralnorm -2.4%. spectralnorm with #393 and these: 1.05 ->
  0.95 ms (Ř 1.05).
- Verified: `tests/r/jit/loop-nonna.R` (NA from vectors, constants and
  entry values, NA conditions, overflow warnings), all `tests/r/jit`
  scripts byte-identical in all tiers; full conformance (this VM's LAPACK
  digits only, as on main); fastr loops/arithmetic/comparison slice
  999/1000, no regressions; sol-loop 3 rounds (round 1: facts were kept
  when the analysis ran out of steps), approved.
