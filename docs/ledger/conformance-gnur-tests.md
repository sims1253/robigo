# conformance-gnur-tests: GNU R test suite and demo suite conformance

Metric: passing cases of `gnur-tests` (31 of 59 pass on main at f523d79a) and `gnur-demos` (14 of 16).

Baseline on `main` (`f523d79a`):
- `gnur-demos`: **14/16** pass (`base/is.things` and `base/error.catching` failing).
- `gnur-tests`: **31/59** pass.

---

## 2026-10-31: `is.na` on expressions and non-vector types (`base/is.things`)

- why: `gnur-demos` `base/is.things` diverged in stderr on `is.na(e13)` where `e13 <- expression(1 + 3)`. GNU R emits:
  ```
  Warning in fn(obj) :
    is.na() applied to non-(list or vector) of type 'expression'
  ```
- what:
  - In GNU R `src/main/coerce.c:do_isna`, the `switch (TYPEOF(x))` covers atomic vectors, lists, pairlists, and NULL. `EXPRSXP` is not handled in the switch and falls into `default:`.
  - In `default:`, GNU R warns `"%s() applied to non-(list or vector) of type '%s'"` with `"is.na"` and `R_typeToChar(x)` and fills the result vector of length `n = xlength(x)` with `0` (`FALSE`).
  - In `crates/robigo-runtime/src/builtins/coerce.rs:do_isna`, `Value::Expr` was grouped with `Value::List`, omitting the warning. Furthermore, the `other` branch returned `vec![FALSE]` of fixed length 1 rather than matching `other.len()`.
  - Disentangled `Value::Expr` from `Value::List` and updated `other =>` to return `vec![FALSE; other.len()]`.
- probes / tests:
  - `python3 scripts/corpus.py run gnur-demos --filter '^base/is\.things$'`
- result: `gnur-demos/base/is.things` passes; `gnur-demos` passing count moves from 14/16 to 15/16. Kept.

## 2026-11-01: handover + `arith` and `reg-packages` after the `is.na` fix

- Full local run (`--filter '^(?!reg-large$)'`): **32/58** pass, 1 new (`arith`),
  1 regression (`reg-packages`).
- `arith` newly passes because `is.na(expression(...))` is now a warning with an
  `xlength`-sized result, exactly as `coerce.c:do_isna`'s `default:` branch.
  Ratcheted.
- `reg-packages` regression is environmental, not code: the case shells out to
  `R CMD build` via `R.home("bin")`. Our local `$R_HOME` is built against
  ICU 76, the machine now only has ICU 78, so the child GNU R dies with
  `libicuuc.so.76: cannot open shared object file`. Nothing in robigo changed;
  the marker stays (CI/oracle image has the right libraries). Verified by
  re-running the case twice (same failure) and by `ldd libR.so`.

## 2026-11-01: `printDeferredWarnings` prefix (`reg-IO2`)

- why: `gnur-tests/reg-IO2` differed on one stderr line. Inside `try()`, GNU R
  prints
  ```
  Error in scan(...) : line 1 did not have 4 elements
  In addition: Warning message:
  In read.table("foo3", header = TRUE, col.names = letters[1:4]) : ...
  ```
  while robigo printed the warning block without the `In addition: ` prefix.
- what:
  - `try()` (base `New-Internal.R`) prints the error itself with `cat()` and
    then calls `.Internal(printDeferredWarnings())`, whose C implementation
    `errors.c:R_PrintDeferredWarnings` does
    `if (R_ShowErrorMessages && R_CollectWarnings) { REprintf("In addition: "); PrintWarnings(); }`.
  - robigo's `do_print_deferred_warnings` called `print_warnings()` directly,
    dropping the prefix (the uncaught-error path in `default_error` already
    had it, which is why only `try()` transcripts diverged).
- probes: minimal `read.table` warning+error under `try()`; `gnur-tests/reg-IO2`;
  fastr `TestBuiltin_(try|tryCatch|warning|stop|...)` slice 15/15.
- result: `gnur-tests/reg-IO2` passes and is ratcheted. Kept.
## 2026-11-01: deparse of non-finite complex elements (`complex`)

- why: `gnur-tests/complex` diverged on one line. `dput(as.complex("NaN"))` gave
  `NaN+0i` where GNU R gives `complex(real=NaN, imaginary=0)`.
- what:
  - R: `src/main/deparse.c:vector2buff` handles `CPLXSXP` elements with
    `ISNAN(Re) || !R_FINITE(Im)` before the generic `EncodeElement` path, and
    emits `EncodeNonFiniteComplexElement` (`complex(real=%s, imaginary=%s)`),
    because `1+Infi` / `3+NaNi` are not valid complex literals. That function
    formats the single element with `formatComplex(&x, 1, ...)` (deparse sets
    `R_print.digits = DBL_DIG`, scipen/na_width from `PrintDefaults()`) and
    `EncodeReal0` for each part.
  - robigo's `deparse.rs` had no such branch, so it fell through to
    `encode_element_for_deparse`, which writes `a+bi` unconditionally.
  - Added `format::encode_nonfinite_complex` (port of
    `deparse.c:EncodeNonFiniteComplexElement`) and the `vector2buff` branch; it
    sits after the both-NA (`NA_complex_`) case, matching R's order.
- probes: `dput` of `NaN+0i`, `c(1+2i, NaN+0i)`, `Inf+1i`, `1+Inf*1i`,
  `0+NaNi`, `NA+I`, `NA_complex_`; compared against the docker oracle.
- result: `gnur-tests/complex` passes and is ratcheted. The 33-case ratchet
  (all markers + `arith`) is 32/33 with only the environmental `reg-packages`
  failure; `fastr` `TestBuiltin_(format|deparse|print|dput|cat|complex)`
  slice is 453/453. Kept.
