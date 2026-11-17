## 2026-11-17: bcEval's scalar paths for MATH1, SQRT, EXP, LOG, LOGBASE (draft)

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
