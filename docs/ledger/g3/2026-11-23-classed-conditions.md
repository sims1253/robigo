# Classed conditions: the builder and its callers (S1, partialMatchWarning)

GNU R 4.6.2 builds its classed conditions in errors.c with one builder per
kind, `R_vmakeErrorCondition` and `R_vmakeWarningCondition`. Each makes the
list `(message, call, fields...)`, named, of class `c(subclass, class,
"error"|"warning", "condition")`. `R_setConditionField` fills the fields.
An error is signalled by `R_signalErrorCondition[Ex]`. A warning is
signalled by `R_signalWarningCondition`, which evaluates base's
`warning(cond)` in a new environment enclosed by the base namespace.

robigo-rt has one builder for both kinds, `conditions::make_condition`
(crates/robigo-rt/src/conditions.rs). It is reached through two isolate
hooks:
- `ErrorCond` / `Cx::error_condition_call` port `R_makeErrorCondition` +
  `R_signalErrorConditionEx`.
- `WarningCond` / `Cx::warning_condition_call` port
  `R_makeWarningCondition` + `R_signalWarningCondition`.

`Cx::partial_match_warning(call, argument, formal)` ports
`R_makePartialMatchWarningCondition`. It is the warning builder's only
user in 4.6.2.

## R_makePartialMatchWarningCondition: every caller

| GNU R 4.6.2 | when | robigo-rt | status |
|---|---|---|---|
| match.c:279 `matchArgs` | `warnPartialMatchArgs`, a closure's or a builtin's partial tag | args.rs `partial_warnings` (closures, S3 methods); robigo-rtbuiltins util.rs `match_args` (builtins) | ported |
| util.c:543 `check1arg` | `warnPartialMatchArgs`, a partial first tag | robigo-rtbuiltins util.rs `check1arg`; call_fn.rs (`call()`'s own check1arg) | ported |
| attrib.c:1554 `do_attr` | `warnPartialMatchAttr`, a partial match of "names" with no other match, names present | robigo-rtbuiltins attrib.rs `do_attr` | ported; the option is now honored |
| attrib.c:1581 `do_attr` | `warnPartialMatchAttr`, a partial match of another attribute | robigo-rtbuiltins attrib.rs `do_attr` | ported |

`warnPartialMatchDollar` (subset.c:1320, 1378) is not one of them. GNU R
reports it with a plain `warningcall`, not a classed condition. robigo-rt
still refuses the option ("not implemented yet").

## The error builder's users (for reference; nothing changed here)

| GNU R 4.6.2 | class (subclass) | robigo-rt |
|---|---|---|
| errors.c:2782 `R_MissingArgError_c`: eval.c:1184, 5834; envir.c:1923, 2471; arithmetic.c:1918, 1936; RNG.c:355 | `missingArgError` (`evalError`, `getvarError`, `getMissingError`, `log1Error`, `log2Error`, `getRNGError`) | rtcore `Cx::missing_arg_error`; eval.rs, env/mod.rs, envir.rs, math.rs, rng.rs |
| eval.c:3061, 3074 (`Tailcall`, `Exec`) | `missingArgError` (`tailcallError`, `tailcallRecError`) | tailcall.rs |
| arithmetic.c:1819 (`do_Math2`) | `missingArgError` (`MathMissingError`) | math.rs `do_math2_special` (`round`, `signif`; tests/rt/math/round-errors.R). The earlier row probed `sin()`, which is `do_math1` and reaches the arity check first; `do_Math2` is `round`/`signif`, which robigo-rt lacked ("robigo: the primitive 'round' is not implemented yet") |
| envir.c:2622 (`get0`/`exists`'s varex) | `missingArgError` (`getVarExError`) | not found in robigo-rt; not probed |
| subset.c:98, subassign.c:1602 `R_makeNotSubsettableError` | `notSubsettableError` | robigo-rtbuiltins subscript.rs |
| subset.c:108, subassign.c:1611, subscript.c:67 `R_makeMissingSubscriptError[1]` | `MissingSubscriptError` | subscript.rs |
| subset.c:260, 269, subassign.c:1765, subscript.c:57, 80 `R_makeOutOfBoundsError` | `subscriptOutOfBoundsError` | subscript.rs |
| errors.c:105 `R_makeCStackOverflowError` | `stackOverflowError` (`CStackOverflowError`) | rtcore limits.rs |
| errors.c:2995, 3001, 3007 (`R_InitConditions`) via eval.c:1072, 5523, memory.c:3403 | `stackOverflowError` (`expressionStackOverflowError`, `nodeStackOverflowError`, `protectStackOverflowError`) | limits.rs (expression, node); no protect stack in robigo-rt's heap (the C API's own reports GNU R's plain error) |
| gram.c:6769-6798 | `parseError` | not raised: `parse(text=)`'s error is a `simpleError` in both (probe) |
| sys-unix.c:878 (`R_system` timeouts) | `cmdError` | not ported (no `R_system` timeout path) |

## Cases

These fail on main and pass here:
- tests/rt/conditions/partial-match-args.R: class, fields, call, and a calling handler's `sys.calls()`, which includes base's `warning` frame. It covers a closure, an S3 method, a builtin, and `call()`'s check1arg.
- tests/rt/conditions/partial-match-attr.R: both `do_attr` sites, `exact = TRUE`, and the option off.
- tests/rt/conditions/partial-match-default.R: the deferred, warn = 1 and warn = 2 reports.
