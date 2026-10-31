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
