## 2026-11-17: S3x-b, classed error conditions and options(expressions =)

- What: the second part of S3x (m2m3-plan.md §B), on top of S3x-a
  (#773). GNU R raises some errors as condition objects of their own
  classes (errors.c:R_makeErrorCondition), so a handler can catch them by
  class and read their fields. robigo-rt raised all of them as simple
  errors with the right message.
- **The mechanism.** rtcore gets a second error hook,
  `Hooks::error_condition`, and `Cx::error_condition_call` /
  `Cx::error_condition`. An `ErrorCond` holds what
  `R_makeErrorCondition` takes: class, subclass, message, extra fields,
  and `R_signalErrorConditionEx`'s `exitOnly`. rt's hook ports
  `R_vmakeErrorCondition` and `R_setConditionField`: the list
  `(message, call, fields...)`, named, of class `c(subclass, class,
  "error", "condition")`. It then ports `R_signalErrorConditionEx`:
  - the handlers for the condition's classes (`R_signalCondition`), with
    the handler stack restored after them;
  - then the default handling of the message.

  rtcore, rtbuiltins and rt can then raise them at GNU R's own sites.
  Without rt, the hook records the message, as `errorcall` does.
- **missingArgError** (`R_MissingArgError`): its subclass is the site's.
  - eval's `evalError`;
  - byte code's `getvarError`, which is code under the `Bc` profile, as
    GNU R's JIT compiles it;
  - `get()`'s and `findFun`'s `getMissingError`.
- **Stack overflows** (`R_InitConditions`, `R_makeCStackOverflowError`):
  - `expressionStackOverflowError` past `R_Expressions`, with no call;
  - `CStackOverflowError` with its `usage` field. Calling handlers are
    skipped for it (`exitOnly`, and `vsignalError` while
    `R_OldCStackLimit` is set), and `options(error)` does not run for it
    (jump_to_top_ex).
  - `nodeStackOverflowError` when robigo's value stack is full.

  GNU R makes the expression and node overflows' conditions once at
  startup; robigo-rt makes one per signal, with the same value.
- **options(expressions =)** (options.c) sets `R_Expressions` and
  `R_Expressions_keep`. It was refused as unhonored.
- **Subscript errors** (`R_makeOutOfBoundsError`,
  `R_makeMissingSubscriptError[1]`, `R_makeNotSubsettableError`), at GNU R's
  sites in subset.c, subassign.c and subscript.c. rtbuiltins'
  `subscript::{out_of_bounds, missing_subscript, not_subsettable}` raise
  them through the hook.
  - `subscriptOutOfBoundsError` carries `object`, `subscript` (the 1-based
    dimension, NA for none) and `index`. The index is the largest position
    for `integerSubscript` and `realSubscript`, the name for a string
    subscript, and the subscript itself for `[[`. The out-of-bounds failure
    of `make_subscript` and of the matrix subscripts now carries that
    position or name to the raise.
  - `MissingSubscriptError` has an `object` except from `get1index`.
    `notSubsettableError` has an `object`.
  - `get1index` took `length(s)` as 0 for a symbol, so `x[[]]` said
    "attempt to select less than one element". GNU R's `length()` of a
    symbol, the missing argument among them, is 1, so it is
    `MissingSubscriptError`, "missing subscript".
- Not here:
  - `MathMissingError` and `log1Error`/`log2Error` wait for math1 (B).
  - `parseError`'s classes wait for `stdin()` (L2a), which their cases
    read.
  - `tests/r/conditions/subscript-error-fields` prints its matrices, so it
    waits for `printMatrix` (B5a). `tests/rt/conditions/subscript-errors`
    covers the same forms and compares the objects instead.
- **A C stack finding for the foreman** (reported 2026-11-17): robigo-rt
  takes about 3.3 KB of C stack per R closure call, and GNU R under
  1.6 KB. At the default limits, GNU R stops a runaway recursion at depth
  4,988 with expressionStackOverflowError. robigo-rt stops it at 2,387,
  with CStackOverflowError. `tests/r/conditions/eval-depth-limit` and
  `tests/r/jit/native-closures` wait for that; the new cases keep their
  limits below it.
- Cases, recorded on GNU R 4.6.2 and ratcheted:
  - `tests/rt/conditions/missing-arg-error`: each subclass, the fields,
    handlers by class, try()'s condition;
  - `tests/rt/conditions/expression-depth`: the class, the 500 levels of
    handler headroom, the limit put back after a jump, options() in a
    handler, the range check, the uncaught report.
  - `tests/rt/conditions/subscript-errors`: each class and its fields for
    `[[`, `[`, `[[<-` and `$<-`, by class.

  Two `tests/r` cases now pass and are ratcheted:
  `conditions/c-stack-overflow` and `basics/nested-inline-calls`.
