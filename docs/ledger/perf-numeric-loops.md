# perf-numeric-loops: FastR's numeric loop wins

Metric: `scripts/rivals.py` peak wall time (fastest warmed-up iteration)
against GNU R 4.5.0, Ř and FastR on `mandelbrot_naive_ascii`,
`mandelbrot_noout`, `mandelbrot_noout_naive`, `awf/Mandelbrot`,
`real/convolution`, `real/convolution_slow` and `shootout/spectralnorm`.
Gate: `scripts/perf.py compare` of instruction counts against main (no
benchmark more than 0.5% slower) and conformance in all three tiers.

Start (main bf7332ee, this VM; rivals from `~/rivals-baseline.json`), peak ms:

| benchmark | robigo | GNU R | Ř | FastR |
|---|---|---|---|---|
| mandelbrot_naive_ascii | 40.0 | 53.0 | 16.9 | 4.47 |
| mandelbrot_noout | 14.3 | 20.1 | 19.4 | 7.88 |
| mandelbrot_noout_naive | 18.8 | 17.4 | 18.7 | 9.94 |
| awf/Mandelbrot | 0.199 | 0.465 | 0.153 | 0.0332 |
| convolution | 2.52 | 42.3 | 1.53 | 1.17 |
| convolution_slow | 5.19 | 59.5 | 21.5 | 2.42 |
| spectralnorm | 21.3 | 50.0 | 1.05 | 0.535 |

Where the time goes (perf, release build, benchmark body repeated):

- `mandelbrot_noout`: complex vector `z * z + c`, 3.4M multiplications per
  call; 1.33M of them take `__muldc3`'s recovery of infinities (escaped
  points sit at Inf + NaN i), each a call of `cmul_recover`: 53% of the
  time in the two. The rest is the VM around 13,050 vector iterations and
  the per-row vector code.
- `mandelbrot_noout_naive`: `Z^2` (two `cmul`s per element through
  `cpow`/`r_cpow_n`, 32%), `X[, , k] <- v` through the general array
  assignment (a flat index vector of 52,900 entries per call, 17%),
  `hypot` (20%, glibc's, which GNU R calls too) and `exp` (17%).
- `mandelbrot_naive_ascii`: scalar complex arithmetic in the VM
  (`leaf_op_slow`/`complex_op`): native loops have no complex type.
- `awf/Mandelbrot`: only the innermost loop is native; the pixel loop calls
  `bitwShiftL`/`bitwXor` (closures over `.Internal`, not inlinable), and
  14-iteration loops never reach the 16-iteration threshold in one entry.
- `convolution`: 93% in the native loop already, but its code is poor:
  integer indexes go through a double conversion and back, NA and overflow
  checks on loop variables that cannot be NA, values spilled around the
  back edge.
- `convolution_slow`: as above, plus the outer loop over
  `seq(length = nx)` stays in the VM (`ForSeqLength` is not native).
- `spectralnorm`: the helper closure recreated per call (#296), then the
  outer loop (`double(n)`, `%*%`) in the VM.

## 2026-11-04: complex vector kernels (noout -9%, noout_naive -40%)

- What (`builtins/complex.rs`, `arith.rs`, `subassign.rs`):
  - `complex::pair`: `__muldc3` on two elements at a time, the products as
    packed SSE2 `mulpd`/`subpd`/`addpd` in inline assembly with the
    scalar code's operand order (each lane follows the scalar NaN rules),
    and the recovery of infinities branch-free over both lanes, kept where
    a lane needs it. Equal-length `*` uses it.
  - Integer powers `z^k` (`k` in 2..65536, one exponent for all
    elements): `R_cpow_n`'s multiplication schedule run on two elements at
    once (a pair with a zero goes through `cpow`, which has its own case).
  - `x[, , k] <- y` (whole leading dimensions, in-range scalar trailing
    subscripts, a double or integer `y` of the block's length and type):
    one copy of the block instead of a flat index per element.
- Rejected, measured:
  - The two-lane products built from `[Complex; 2]` arrays: LLVM moved
    them through the stack with overlapping loads (store-forwarding
    stalls), noout 14.3 -> 26.5 ms. Loading the slices directly fixed it.
  - The recovery as an out-of-line function: 21.7% of noout in the call
    alone; inlined.
  - Always computing the recovery (no branch on "any lane NaN + NaN i"):
    noout 13.1 -> 19.3 ms; the branch stays.
  - `exp()` returning +0 below -746 without glibc (whose underflow path
    sets errno): no measurable gain on noout_naive, `robigo/vecarith`
    +1.76% instructions. Dropped.
- Result, peak ms (rivals.py): `mandelbrot_noout` 14.3 -> 13.0,
  `mandelbrot_noout_naive` 18.8 -> 11.4 (now faster than GNU R and Ř;
  FastR 7.9 / 9.9), `mandelbrot_ascii` 11.3 -> 10.7. Instructions against
  main: 3 faster (`mandelbrot_noout_naive` -31.0%, `mandelbrot_noout`
  -8.4%, `mandelbrot_ascii` -6.8%), 0 slower.
- Verified: unit tests `pair_cmul_matches_cmul` (all 169 x 169 pairs of
  special values, both lanes, mixed with recovering partners) and
  `pair_cpow_n_matches_cpow` against the scalar kernels, and the GNU R
  oracle test of complex arithmetic; `tests/r/math/complex-vector-mul-pow.R`
  and `tests/r/vectors/array-block-assign.R` recorded with GNU R 4.5.0,
  byte-identical in all tiers.
- Open: what is left of noout is about half the VM around vector code
  (13,050 iterations of `z <- z * z + c; i <- i + 1L` with `iter` a global)
  and half the multiplications' recovery; noout_naive is a third `hypot`
  and `exp` from glibc, which GNU R calls too.

## 2026-11-08: native complex scalars and bitwise internals on #485

- Why: complex constants rejected native loop compilation, and the base
  bitwise closures could not run inline in native regions.
- What: two-lane complex loop scalars with exact deoptimization; native
  lowering of all six bitwise internals through existing builtin-call
  bytecode. Keep base-function guards, traced-call fallback, NA results,
  unsigned shifts, and warning-producing coercions in the VM.
- Measured against `2a5ff0b3`: `mandelbrot_naive_ascii` 736,058,072 ->
  281,868,693 instructions (-61.71%). The pinned awf Mandelbrot source at
  size 100 is 216,103,308 -> 98,610,134 (-54.37%), with output recorded
  independently using GNU R 4.5.0. Its standard size-14 fixture stays
  below native outer-loop heat thresholds and is effectively unchanged.
- Gate: all 14 selected benchmark outputs match GNU R, zero regressions
  above 0.5%, worst increase +0.03075% (`nbody_naive`). Kept both changes.
  The existing bitwise/split-type fixtures caught an initial mismatch
  between tagged-number analysis and call generation; fixed before shipping.
- Verification and full counts: [native-scalars.md](perf-numeric-loops/native-scalars.md)
  and [native-scalars.csv](perf-numeric-loops/native-scalars.csv). Two existing
  C-library complex unit-test failures reproduce bit-for-bit on untouched
  main; all other runtime unit and integration tests pass.
- Open: native `ForSeqLength` and broader complex operations. No heat-policy
  change or shared compiler/builtin/environment/interpreter edits.

## 2026-11-08: ForSeqLength inside native regions

- Native regions now accept `ForSeqLength` with an invariant length.
  Entry checks flush pending assignments, check base-function guards, and
  call the VM's `seq_length_range` for the length and S3 dispatch checks.
  Each op has a separate entry-buffer value. Native code reconstructs the
  range at the op, preserving register liveness and deoptimization.
  Outer ranges also accept compact integer sequences through `range_of`,
  bounded by the native index limit. Convolution first reaches the loop
  with that representation; accepting only unboxed ranges left it in the VM.
  A failed check returns the entire region to the VM. Lengths written in
  the region, including indexed writes and callee cell writes, stay in the VM.
- Verification: three runtime tests prove native compilation, compact outer
  sequences, dispatch changes
  after warmup, and rejection of a changing length. The extended
  `basics/for-seq-length.R` oracle was recorded in `robigo-oracle:4.5.0`.
  All 50 selected fixtures match across interpreter, VM, and JIT.
  Focused JIT conformance passes its gate, with 47/49 oracle matches; the
  unratcheted active-binding and negative-zero failures match the untouched
  branch binary. Formatting and both lint configurations pass.
  Use `--test conformance conformance -- --exact` for the filtered
  oracle check: `compiled_matches_interpreted` ignores the environment
  filter, so the three-tier check was restricted explicitly to these fixtures.
- Measured against `8b865d5a` with release builds and
  `perf.py run --metric instr --filter 'convolution|Mandelbrot|mandelbrot'`:
  `convolution_slow` fell from 312,665,282 to 287,815,207 instructions
  (-7.95%). All nine outputs match GNU R. `perf.py compare --threshold 0.5`
  reports one faster, none slower; worst increase is
  `rbench/shootout/mandelbrot_ascii` (+0.00954%).
