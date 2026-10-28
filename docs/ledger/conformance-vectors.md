# conformance-vectors

## 2026-10-28: baseline and failure groups

- Baseline: 6,669/6,819 FastR cases pass, zero regressions, using
  R_HOME=$HOME/.local/opt/R-4.5.0. Results preserved locally in
  /tmp/vectors-baseline.json.
- 36 crossprod failures: C99 complex multiplication recovery for Inf/NaN
  is missing in array.rs. First implementation target.
- 52 SimpleVectors failures: mixed causes, including replacement error
  calls, recursive indexing, assignment evaluation order, complex printing,
  language names, and list-array printing. Split before implementing.
- 25 cbind/rbind failures: empty-vector dput line wrapping accounts for
  many cases (printing track); remaining cases include method dispatch
  and non-vector inputs.
- 8 rep/repint failures: factor dispatch recursion, complex printing,
  and coercion warnings.
- 13 array/dim/SimpleMatrix/SimpleArrays failures: coercion warnings,
  complex and list printing, dimension validation.
- Remaining 16: duplicated/unique coercion (4), match.arg/match.fun (7),
  data-frame missing arguments (2), sort collation (1), parse source
  attributes (1), dimassign printing (1).
- No implementation changes yet. No recorded oracle output changed.

## 2026-10-28: C99 complex matrix multiplication recovery

- R: array.c:CMATPROD_BODY, CCROSSPROD_BODY, CTCROSSPROD_BODY use
  C99 complex multiplication. The previous Rust helper performed only
  component arithmetic and lost infinite results when both raw components
  became NaN. Added compiler-runtime recovery for infinite inputs and
  intermediate overflow, preserving signed zero during normalization.
- Source for C99 recovery: LLVM compiler-rt lib/builtins/muldc3.c.
- Crossprod slice: 171/207 -> 207/207; 36 new passes recorded, no regressions.
- Full filter: 6,669/6,819 -> 6,705/6,819, zero regressions. Also
  recorded one previously unrecorded baseline pass via --update.
- Runtime tests: 17 unit tests and 2 lazyload tests pass. cargo fmt --check
  passes. Conformance: 163/164 scripts pass; datetime/zones.R fails on
  this host with byte-identical baseline/current output. Both compiled
  checks pass. No tests or oracle expectations weakened.
- Help examples base/{crossprod,matmult,matrix,array}: 3/4 before and after;
  base/matrix is the unchanged failure.
- robigo/matrix callgrind: 638,778,139 -> 638,729,249 instructions;
  perf.py compare reports no slowdown above 0.5%.
- Kept. CI full corpora and performance checks pending on PR #21.
- Next work: separate SimpleVectors error-call differences from evaluator
  assignment-order bugs; port incomparables coercion warnings in unique.rs.
  Printing differences and factor dispatch recursion need their owners.
- Empty binding cases inspected have correct dimensions and values; their
  dput line wrapping belongs to the printing track, so no binding change.

## 2026-10-28: incomparables coercion

- R: unique.c:duplicated3/any_duplicated3 use coerceVector for
  incomparables. Emit coercion warnings and reject closures/builtins and
  environments as coerce.c specifies, locally in unique.rs.
- duplicated/unique/anyDuplicated slice: 130/136 -> 136/136, zero
  regressions; six passes recorded (four within the primary track filter).
- Runtime: 17 unit and 2 lazyload tests pass; cargo fmt --check passes.
- Kept. Indexing baseline: 2,331/2,391 in SimpleVectors/SimpleMatrix/subset.

## 2026-10-28: replacement condition calls

- Port subassign.c:errorNotSubsettable/errorMissingSubscript/
  errorOutOfBoundsSEXP: pass R_CurrentExpression to condition signaling,
  retaining the printed <current-expression> symbol.
- Indexing slice: 2,331/2,391 -> 2,354/2,391, zero regressions, 23 new
  passes recorded. Runtime tests (17+2), subset conformance and both
  compiled checks pass; formatting passes. Kept.

## 2026-10-28: extraction allocation and list-matrix replacement checks

- Port subset.c:do_subset2_dflt's allocation error for a recursive index
  reaching a closure; port the complete subassign.c assignment switch to
  reject list-matrix NULL replacement while retaining valid list RHS types.
- Initial incomplete switch rejected valid list assignments and broke
  package loading; rejected and corrected against the full C switch.
- Indexing: 2,354/2,391 -> 2,356/2,391, zero regressions; two new passes.
- Runtime 17+2 tests and subset conformance/compiled checks pass.
- Kept locally; coordinator requested no further pushes to PR #21.

## 2026-10-28: rep counts warning experiment

- Tried warning-emitting count coercion per seq.c:do_rep. Metric stayed
  178/186, no regressions. Warning still absent on the error path.
- Rejected and reverted; investigate warning handling in dispatch before
  retrying. Factor recursion goes through rep.factor -> NextMethod and
  needs dispatch-owner changes.
