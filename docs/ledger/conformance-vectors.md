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
