## 2026-11-17: bcEval's scalar paths for MATH1, SQRT, EXP, LOG, LOGBASE

The foreman's correctness item, ahead of V4 step 3; held until #803 (AC1)
merges, then measured on top of it. B's #802 (math1 functions, `log`)
found it: GNU R's byte code computes one number directly (bcEval calls
the C library), so in compiled code `1 / sin(-0)` is `-Inf`, where
robigo-rt always took the interpreted `do_math1` path and gave `Inf`
(`sqrt` and `atan` on main differ the same way). And GNU R reports a
scalar `log(v, b)`'s warning against the call `log(v, b)`, robigo-rt
against the calling function.

- **Tree:** branch `g3/c1-math1-scalar`, from main `c5809856` (#802).

### bcEval's paths (R: eval.c, 4.6.2)

- `SQRT`, `EXP` (`FastMath1`, `bcStackScalar`): a REALSXP scalar (a
  register's double, or a length-1 REALSXP without attributes) is
  `fun(x)`; a non-NA INTSXP scalar is `fun((double) i)`; a LGLSXP or NA
  integer goes to the builtin (`do_math1`).
- `MATH1` (`DO_MATH1`, `bcStackScalarReal`: an integer, NA too, made a
  double): `math1funs[i](x)`, the C functions as the table names them
  (`floor`, `ceil`, `sign`, `expm1`, `log1p`, `cos`, `sin`, `tan`, `acos`,
  `asin`, `atan`, `cosh`, `sinh`, `tanh`, `acosh`, `asinh`, `atanh`,
  `lgammafn`, `gammafn`, `digamma`, `trigamma`, `cospi`, `sinpi`,
  `Rtanpi`), never `do_math1`'s wrappers. Anything else: `do_math1` with
  `getPrimitive(CAR(call))`. `getMath1Fun` errors "math1
  compiler/interpreter mismatch" when the call's function is not the
  table's.
- `LOG` (`R_log`), `LOGBASE` (`logbase(x, base)`), `bcStackScalarReal`
  for each operand; else `do_log_builtin(call, log, args)`.
- Each: a NaN result of a non-NaN operand warns "NaNs produced" against
  the opcode's call (`warningcall(call, R_MSG_NA)`); a NaN operand's NaN is
  the operand itself (its payload kept). `R_Visible` is TRUE.

### The design

1. One IR op, `Math1`: `a` the value, `b` the operands' window (1, or 2
   for `LOGBASE`, `c`), `e` the call, `f` the slow path's builtin (0:
   `do_log_builtin`), `g` the kind (the `math1funs` index, or `SQRT`,
   `EXP`, `LOG`, `LOGBASE`). The verifier checks it like `CallBuiltin`.
2. The VM's arm: the fast paths above, exactly, on the registers; else the
   slow path through the window, as `CallBuiltin`'s.
3. The translation (`bctrans.rs`): `MATH1`, `SQRT`, `EXP`, `LOG`,
   `LOGBASE` → `Math1` (V3 had them as `CallBuiltin`; `LOG`/`LOGBASE` were
   not translated).
4. The from-source compiler under `Bc` (JIT-compiled closures run its
   code): cmp.R inlines these functions as these opcodes (`cmpMath1`, the
   `sqrt`/`exp` handlers' `cmpPrim1`, `log`'s handler), so `Comp` emits
   `Math1` under the same conditions (one argument, or `log`'s one or two
   unnamed; no `...` or empty argument), else what it emits now.
5. Fixtures: #802's unratcheted cases, and `1 / sin(-0)`-style ones for
   each kind, the warnings' calls, integers, logicals and NAs, recorded on
   4.6.2.

### What landed

- `Op::Math1` (`ir.rs`), its VM arm and two helpers outside the loop
  (`vm.rs`: `math1_fast`, the paths above on `num()`'s reading of a
  register, bcStackScalar's; `math1_slow`, the builtin or
  `call::call_fn_regs` with `do_log_builtin`), the verifier's rule (the
  window, the kind, the value outside the window) and the crosscheck's
  operand kinds.
- `robigo_rtbuiltins::math::BC_MATH1`: eval.c's `math1funs`, the C
  library's functions (libm's `expm1`, `log1p`, `acosh`, `asinh`, `atanh`
  through robigo-numerics, Rust's own formulas rounding differently) and
  nmath's; `bc_log`, `bc_logbase`.
- The translation: `MATH1` (getMath1Fun's check that the table's name is
  the call's function), `SQRT`, `EXP`, and now `LOG`, `LOGBASE` (V3 had
  them untranslated).
- The from-source compiler under `Bc`: `Inline::Math1` for the 24 table
  names, `sqrt` and `exp` (one argument, no `...` or empty one), `log`'s
  handler (one or two unnamed). Under `Eval` these stay the generic call
  (`known()`): the AST's `do_math1` wraps the C functions.
- Fixtures: `tests/rt/bctrans/math.R` (8 cases: `1 / sin(-0)`, the signed
  zeros, integers, NAs, logicals and vectors through the builtin, the
  NaN warnings' calls), recorded on 4.6.2; #802's `math1-compiled.R` and
  `log-compiled.R` ratcheted. `log-missing-class.R` (the condition
  classes of `log()`'s missing argument) is not this path; it stays B's.
- After #803 (AC1, `1:n` compact): the slow path's `call_fn_regs` expands
  a compact sequence argument as `run_builtin` does (`do_log_builtin`
  reads `REAL()`), and the fixtures call `log(1:3)`, `log(1:4, 2)`,
  `log(8, 1:3)`. Merging main found main itself broken the same way
  (`log(1:3)` panicked in #802's `do_log` special): #810, merged into
  this branch until it lands on main.

### Review

- Sol round 1 (`e96fcc9d`): CHANGES, S1 [blocker]: the fast path dropped
  nmath's queued warnings (`ML_WARNING`) for a non-NaN result (compiled
  `gamma(1e-310)` must warn "value out of range in 'gammafn'"). Fixed in
  `7718ce96`: `BC_MATH1` marks nmath's functions (as #802's `do_math1`
  does); the arm drops stale queued warnings before the computation and
  raises those it queued after it, before "NaNs produced"; fixture
  `k_gamma_range` (the warning's call is the closure call's, as GNU R's).
- Sol round 2 (`7718ce96`): APPROVE, S1 resolved, no new findings.
- Sol round 3 (`9bc60ecb`, the delta after round 2: the arm out of
  `vm::run`, `call_fn_regs` expanding a compact sequence argument):
  APPROVE, no findings. (Recorded with V4 step 2's PR: a push to #812
  would have restarted its CI.)

### Verification

On main `5f5f63f4` (#803) merged, with #810, at `4ae13f3a`:

- `scripts/rt-ci.py`: exit 0. Conformance 524/532 (the 8 failing as on
  main: `link-walk-mapply`, `link-walk-reduce`, the 5 `bytecode/`
  compiler-namespace cases, `math/log-missing-class`); every observation
  mode, gctorture, the moving collector, fail-alloc (2800 runs, 0
  differences); the crosscheck 123 compared, 98 same, 0 unexpected.
- `scripts/lint.sh`, `cargo fmt --check`: clean.
- perf-rt (`scripts/perf.py rt-bars --main`, main `5f5f63f4`'s binary,
  this runner): flat, every row within ±0.04% of main (plain -0.01%,
  defaults +0.04%, named +0.01%, env-fn, superassign, s3, fib,
  binarytrees, e7-* within ±0.02%), peak RSS within noise; 0 failures.
  A first measurement with the arm inline in `vm::run` failed: plain
  +0.48%, defaults +0.50%, named +0.61%, env-fn +0.33%. The same set of
  functions stayed out of line (no inlining flip), but the arm shifted
  `run`'s register allocation, ~8 instructions a closure call. The arm
  is now one call (`vm::math1`, `#[inline(never)]`) and a reload, which
  measures flat.
