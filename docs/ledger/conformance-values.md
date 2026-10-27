# conformance-values: operators and coercion on R's values

Metric: `fastr` passes within
`TestSimpleValues|TestSimpleArithmetic|TestSimpleComparison|TestBuiltin_(operators|log|asvector|asraw|intToBits)`
(10,501 cases; ~690 failing at the start).

Owns: `crates/robigo-runtime/src/builtins/{arith,coerce,math}.rs`,
`crates/robigo-runtime/src/coerce.rs`, new `builtins/complex.rs`.

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
