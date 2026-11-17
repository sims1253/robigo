## 2026-11-17: S3x-d, log()'s missing `x` is GNU R's classed error

- What: the foreman's S3x-d (b), from B's math work (#802). `log()` with
  `x` missing gave the right message and call, but as a plain
  `simpleError`. GNU R raises it with `R_MissingArgError_c("x", call,
  subclass)` (arithmetic.c:do_log_builtin):
  - `log1Error` with one argument that is missing or not tagged `x`
    (`log(base = 2)`);
  - `log2Error` after `matchArgs_NR` otherwise (`log()`, `log(b = 2, 3)`).

  Both have class `c(subclass, "missingArgError", "error", "condition")`.
- How: the two sites call `Cx::missing_arg_error`, S3x-b's port of
  `R_MissingArgError_c`, through rtcore's classed-error hook.
- Not here: `MathMissingError` (arithmetic.c:do_Math2, for `round` and
  `signif` without `x`). robigo-rt has no `do_Math2` yet; its port should
  raise it the same way.
- Case: B's `tests/rt/math/log-missing-class`, recorded on GNU R 4.6.2,
  now passes and is ratcheted.
