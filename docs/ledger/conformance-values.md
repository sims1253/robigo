# conformance-values: operators and coercion on R's values

## 2026-10-31: complex.c -- `polyroot()` and `do_cmathfuns` (Re/Im/Mod/Arg/Conj)

- why: `polyroot()` was a placeholder and `Re`/`Im`/`Mod`/`Arg`/`Conj`
  dropped attributes and mishandled non-complex input, 6 + 11 `fastr`
  cases. Metric: `TestBuiltin_(Im|Re|Conj|Mod|Arg|polyroot)` 57/74 ->
  73/74 in this slice (the one remaining case is the known host-glibc
  last-bit difference in `hypot`; it matches on CI's Debian trixie, proven
  by the same C reference in the oracle container).
- what: `src/cpoly.rs` ports `complex.c:R_cpolyroot` with its TOMS 419
  helpers (`noshft`, `fxshft`, `vrshft`, `calct`, `nexth`, `polyev`,
  `errev`, `cpoly_cauchy`, `cpoly_scale`, `cdivid`) one arithmetic step
  at a time. C keeps `relstp` in a function-static that outlives calls;
  it is a thread local here for the same start state. `do_polyroot`
  ports `complex.c:do_polyroot`: complex coercion, trailing zeros dropped,
  non-finite coefficients error, `root finding code failed`.
  `do_cmathfuns` now coerces logical/integer input to double (so
  `typeof(Conj(FALSE))` is `"double"`), makes `Im()` of a real vector
  zero even for `NA`/`NaN`, and shallow-copies every attribute
  (`SHALLOW_DUPLICATE_ATTRIB`) instead of only names/dim/dimnames.
- tests: `tests/r/math/polyroot.R` (exact roots, repeated roots, integer/
  logical/complex coefficients, empty and constant polynomials, scaling,
  invalid coefficients and types) and `tests/r/math/complex-cmath.R`
  (result types, attributes incl. class and `scaled:center`, `Im`/`Arg`
  on `NA`/`NaN`/infinite input, complex input). Both recorded with GNU R
  4.5.0 in `robigo-oracle:4.5.0`; both pass the interpreter, VM and JIT
  tiers. One polynomial in `polyroot.R` prints `round(..., 12)` because
  its near-zero imaginary parts depend on glibc's `hypot` last bit: the
  C reference compiled inside the oracle container reproduces GNU R's
  values exactly, while this host's glibc 2.44 differs in that bit (the
  same documented host divergence as the two complex oracle unit tests).
- result: fastr `TestBuiltin_(Im|Re|Conj|Mod|Arg|polyroot)` 57/74 ->
  73/74; full `fastr` 31,137/31,374 -> 31,153/31,374 (+16, the same two
  pre-existing host regressions, `order/add626733541c799` and the
  17 GB compact-realseq `serialize` case that does not fit this host's
  memory). 16 new fastr ratchet markers plus `gnur-examples`
  `base/polyroot.R` (the help page now runs).


Metric: `fastr` passes within
`TestSimpleValues|TestSimpleArithmetic|TestSimpleComparison|TestBuiltin_(operators|log|asvector|asraw|intToBits)`
(10,501 cases; ~690 failing at the start; 10480/10501 on main after #47).

Owns: `crates/robigo-runtime/src/builtins/{arith,coerce,math}.rs`,
`crates/robigo-runtime/src/coerce.rs`, new `builtins/complex.rs`.

## 2026-10-28 (track/conformance-values-3, PR #61): ts attrs on relops/logic, validating attr sets, coercion gates

- baseline on main (45b4dd2): 10480/10501, 1 ratchet regression
  (`0bb20eaa0190ebab`, the pre-existing console 4096-byte chunking case).
- brought in the WIP from `track/conformance-values-2-wip` (env coercion
  error "environments cannot be coerced to other types" in coerce_vector):
  verifies on main, +1 (`intToBits/e1253cb9676faa7b`). kept.
- ported relop.c/logic.c/arithmetic.c's ts tail to `relop`, `do_logic`
  and `arith_binary` (tsp/class from the ts operand after dims,
  ERROR_TSVEC_MISMATCH when the only ts operand is shorter), and routed
  the result's dim/tsp/class through the validating `set_attrib` like
  R's setAttrib (dimgets product check: `m + 1:16` etc; tspgets
  zero-length and parameter checks; classgets). kept:
  10480 -> 10495 (+14; dims [product] cases, ts comparisons and `&`/`|`,
  both-ts Ops.ts alignment verified byte-identical incl. error paths).
- perf gate (callgrind vs main, jobs 1): matrix 759.45M -> 758.76M,
  vecarith 4481.33M -> 4481.21M, vecops 2541.10M -> 2540.89M — no
  slowdown. conformance suite + robigo-runtime tests pass.
- hill-climbed the coercion suites outside the filter (same files):
  isVectorizable gates (empty-list elements -> NA, NULL elements error,
  expression sources never qualify), as.expression per-element split
  and names kept, as.pairlist(symbol) error, do_asfunction port (list
  arg first, plain env errors, CheckFormals), storage.mode<- plain
  errors + 'single' message + isValidString, rawShift C truncation
  (misc.rs, unowned: minimal change), deparse_one_line joins with "\n"
  (deparse.rs, print's file: minimal change, R-exact deparse1line_ex).
  kept: +33 fastr coercion suites, +1 lang/print slice, +1 gnur-tests;
  filter holds 10495; perf re-run unchanged. absorbed 12 previously
  unrecorded passes on main (TestSimpleVectors ×3, Math ×8, misc ×1).
- remaining failures in the filter, all outside the track's files:
  - 4 × asvector pairlist `attr(,"myattr")` printing (print.rs —
    conformance-print; the attribute survives in robigo, printing drops
    it).
  - `0bb20eaa0190ebab` (+ `3a43295fa2f63026` outside the ratchet): a
    source line >4095 bytes; robigo's console read chunks cut the token
    (cli echo.rs / parser gend=0; see 2026-10-27 entry below).
  - `TestSimpleArithmetic/f7fa6afb895ab94a`: `%*%` with
    `options(matprod='blas')` — the expected NA at [3,3] is the
    recording BLAS's NaN-payload accumulation order (openblas kernel
    reordering); array.rs is conformance-vectors' file and the result
    depends on the recording machine's BLAS. not attempted.

## 2026-10-27: baseline and failure taxonomy

- baseline: 9755/10501 pass (746 failing) with the filter above, on main
  (9633f1e). Note: 9 cases in the filter pass but are missing from the
  ratchet (absorbed nowhere yet); 1 ratchet regression on main, see below.
- failure groups (first-difference signature over expected vs got):
  1. ~184 complex `%%`/`^`: GNU R errors "unimplemented complex operation"
     for `%%`/`%/%`, robigo returns NaN+NaNi; complex `^`/`log`/math1
     functions unimplemented or wrong (`0.2010479+0.236866i` vs
     `0.150515+0.3410941i`).
  2. ~50 complex NaN vs NA: want `NaN+NaNi`, `NaN+Infi`, `-Inf+NaNi`...;
     robigo prints `NA` (NaN complex values collapse to NA) or
     `NaN+NaNi` where the parts differ.
  3. ~170 error-call deparse of `%%`/`%/%`: GNU R prints `a%%b` (no spaces
     around the operator) in `Error in ...`; robigo prints `a %% b`.
     [deparse.rs — conformance-print's file]
  4. ~50 `as.vector`/`as.*` error call and messages: GNU R reports the
     inner `.Internal` call `as.vector(x, mode)` for `cannot coerce type`
     errors but the user call for `invalid 'mode' argument`; robigo reports
     the evaluated user call. Plus `cannot coerce type 'environment' to
     vector of type 'symbol'` ordering and `invalid type/length (symbol/3)`
     lengths. [builtins/coerce.rs]
  5. ~30 `log(x, base)` warnings: NA-in must not warn; base=1/-1 warns
     plain `NaNs produced` (no `In log(...)` prefix — R's warningcall with
     the *closure's* call, hidden by promise-forcing display rules; needs
     do_log port). [arith.rs do_log]
  6. ~20 `as.raw`/`intToBits`: want warning "out-of-range values treated
     as NA" (robigo: "NAs introduced by coercion"); intToBits of NA /
     out-of-range / character inputs wrong. [coerce.rs + util?]
  7. ~12 `&&`/`||`/`&`/`|`: missing "invalid 'x' type in 'x && y'" errors
     (robigo returns NA instead), arity message "1 argument passed to '&'
     which requires 2". [arith.rs logic]
  8. ~7 arithmetic drops non-standard attributes (want `attr(,"myattr")`
     kept on `+` results). [arith.rs copy_operand_attrs vs copyMostAttrib]
  9. ~9 numbered "Warning messages:\n1: ..." shapes for multiple coercion
     warnings (robigo merges or emits unnumbered). [coerce warnings]
  10. outside the track: `%%` deparse spacing (3), ts printing (7),
      `registered S3method` (1), `dims [product 4] do not match` (4),
      formula as.vector (1).
- pre-existing main ratchet regression (not this track):
  `builtins/TestBuiltin_operators/0bb20eaa0190ebab` — a single source line
  longer than 4095 bytes whose number token is cut at the console's
  4096-byte read boundary (`crates/robigo-cli/src/echo.rs` chunking +
  robigo-syntax reporting an error where R's gend=0 parse says incomplete).
  Also fails in CI's fastr artifact of main's last run (40547366630); the
  case was added to passing.txt (23d4907) after that run. For the
  cli/parser tracks: the case passes only with a gend=0-aware console
  parser.

## 2026-10-29: + and * on doubles keep GNU R's NaN payload (NA vs NaN)

- why (perf-vm's note): with two NaN operands, which payload survives
  decides whether R prints NA or NaN, and robigo's `+`/`*` kernels were
  plain Rust, which LLVM may commute. `c(NA, NaN) * c(NaN, NA)` (operands
  computed in place) printed `NaN NA` (GNU R `NA NaN`); `NA_integer_ + NaN`
  gave `NA` (GNU R `NaN`); `NA_integer_ + c(NaN, NaN)` and
  `c(NA, NA, NA, NA) + c(NaN, NaN)` were also wrong.
- the rule, from GNU R 4.5.0's own results (`scripts/oracle-real-arith.R`:
  16 special doubles in every pair, five shapes, plus NA_integer_/3L with
  doubles): x86-64 SSE keeps the destination operand's payload, and gcc's
  destination differs by loop. Doubles: x for scalar-scalar, `n2 == 1` and
  equal lengths; y for `n1 == 1` and general recycling (arithmetic.c:
  real_binary's four loops). An integer operand: y for scalar-scalar
  (do_arith's scalar fast path), x in every vector shape (MOD_ITERATE2).
  Logicals count as doubles (coerced first). `-`, `/`, `%/%` always keep
  x's; `^` and `%%` were already right (R_pow's `x + y`, myfmod).
- what: `+`/`*` as inline-assembly `addpd`/`mulpd` (two elements per
  instruction) with the destination chosen by `x_is_dest`, in
  `real_binary`, `real_binary_inplace` and the scalar fast path; loops
  unrolled by four (LLVM does not unroll loops around inline assembly),
  slices zipped by `chunks_exact`, operation and destination as const
  generics. First version (bounds-checked pairs, run-time flags): vecarith
  +6.6%, spectralnorm_alt_2 +4.1%; final: geomean 0.9986 over all 78
  programs (vecarith -0.85%, worst +0.13% nbody). The VM's `arith_fast`
  (vm.rs, `real_op`) is unchanged.
- rebased on afe56d9 (#50, #55, #65), CI's gate found Bounce_nonames,
  fasta_naive_2, nbody_naive, matrixFor, fannkuch +0.54..+0.91%: all in
  `vm::run_with`'s own instructions (same calls), i.e. LLVM compiled the
  VM loop differently once arith.rs grew, although nothing new is called
  from it. With the kernels in their own module (`arith_exact.rs`) all of
  them are at or below main (-0.03..-0.09% locally against afe56d9; CI: 2
  faster, 0 slower). Another case for the build-profile decision
  (codegen units/LTO, `build-profile.md`).
- tests: `gnu_r_oracle` in arith.rs (9,440 GNU R results through
  `real_binary` in each shape); `tests/r/math/nan-payload-arith.R` (every
  shape, temporaries and variables, integers, logicals, long vectors, a
  recycled matrix column; main's binary differed on 18 lines); the oracle
  scripts through the binary in all three tiers: 0 of 12,256 differ;
  conformance in all tiers; fastr arithmetic slice (12,200 cases): the
  same pass set as main.
