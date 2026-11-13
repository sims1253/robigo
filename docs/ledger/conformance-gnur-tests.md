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

## 2026-11-03: `options(OutDec=)` in `as.character()` of doubles/complex (PR #246)

- why: R's `coerce.c:StringFromReal` calls `EncodeRealDrop0(..., OutDec)` and
  `do_ascomplex`'s `EncodeComplex(..., OutDec)`, so `as.character(1.5)` under
  `options(OutDec=",")` is `"1,5"`; robigo ignored the option.
- what:
  - `format.rs`: `OutDec` is a `Copy` struct stored in a thread-local `Cell`
    (modeling options.c's `sdec` C global, `strncpy`'d to 10 bytes). R drops
    trailing zeros before replacing `"."` by `dec` (printutils.c:
    EncodeRealDrop0), so `encode_real_drop0_dec` follows that order.
  - Perf: the first version read the option per element through a
    `RefCell<String>` and cost +0.55%/+0.97% on robigo/csv and robigo/strings.
    The merged version reads it once per vector in `coerce::to_str`, keeps the
    default path on the untouched `encode_real_drop0` branch, moves the
    non-default path into `#[cold] #[inline(never)]` helpers, and pins
    `encode_real_drop0`/`drop_trailing_zeros_in` as `#[inline(always)]` so the
    inlined shape matches main's. Final local instr vs main: csv −0.004%,
    strings −0.012% (perf check green in CI).
  - callgrind bisect trick: `callgrind_annotate` per-function diffs on a
    400k-conversion repro showed the residual cost was `drop_trailing_zeros_in`
    becoming an outlined call (400k × ~17 instr) after the cold caller was
    added.
- gap kept: R warns "the decimal mark is more/less than one character wide"
  when OutDec isn't exactly one character (EncodeReal0); tracked as an
  unmarked case in tests/r/io/outdec-coerce.R output.
- result: merged as PR #246. gnur-tests count unchanged (36/59); the fix is
  covered by tests/r/io/outdec-coerce (GNU R-recorded output).

## 2026-11-03: honest allocation probe (`primitives` OOM, PR pending)

- why: `gnur-tests/primitives` died with the 3 GB cgroup OOM-killing robigo
  (exit 137) inside the "testing non-generic 'seq_along'" iteration — the
  case that previously made robigo grow to 8 GB and get the desktop killed.
- bisect: loop 1's `length` iteration leaves `length.testit` (body `"OK"`)
  behind; loop 2's `seq_along(x)` then dispatches `length` exactly like
  `seq.c:do_seq_along`, `asInteger("OK")` gives NA + the coercion warning,
  and `R_compact_intrange(1, NA_INTEGER)` asks for a descending realseq of
  length 2147483650. GNU R stores that as ALTREP and never touches it; robigo
  materialises compact sequences, so `compact_realseq` collected 17 GB.
- what: `memory.rs:probe_vector` now bounds the request by the honest limits
  before asking the allocator: the cgroup memory ceiling minus its usage
  (walking v2's hierarchy, v1's memory.limit_in_bytes) and RLIMIT_AS via
  /proc/self/limits (the crate denies unsafe, so no getrlimit).
  Overcommitting kernels let `try_reserve_exact` succeed for far more than
  the process may touch, which is why the probe never fired; now the request
  fails with R's own `memory.c:allocVector3` message ("cannot allocate
  vector of size 16.0 Gb") and `try()` can catch it. MemAvailable stays out
  of the bound: it is advisory, and the first version that counted it
  regressed fastr's serialize(1:2147483649) — GNU R answers that from a
  virtual sequence while CI's roomier runner still materialises it.
- result: `primitives` runs to its last loop (no OOM); remaining diffs are
  new root causes: (1) `abs(zZ = NULL)` through an `.ArgsEnv` stub must be
  "unused argument (zZ = NULL)" but robigo positional-matches and evaluates
  abs(NULL) → "non-numeric argument to mathematical function"; (2) robigo
  does not print the `tracemem[0x.. -> 0x..]: ...` line the transcript
  expects.
- gap (closed 2026-11-12, #460): `tests/r/vectors/giant-seq-gap.R` records
  GNU R answering `length(1:-2147483648L)` with 2147483650 from a virtual
  sequence. robigo's compact sequences now stay unexpanded
  (`value/compact.rs`), and it passes.
