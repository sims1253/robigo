# conformance-print: print, format, deparse

Metric: `fastr` passes in
`TestBuiltin_(print|format|deparse|dput|cat|toString|printdefault)|TestSimplePrinting`
(437 cases on main, 2026-10-28). Run everything with
`R_HOME=$HOME/.local/opt/R-4.5.0` (the system R is 4.6.1; robigo loads its
`utils`/`stats` R code and would otherwise warn that the packages were built
under a later R).

## 2026-10-28: baseline and failure grouping

- why: measure before changing anything.
- baseline: **390/437 pass** (47 fail; stdout 432, stderr 394, status 436).
- grouping of the 47 failures by root cause:
  1. **`.Internal` arity messages** (40): every `TestBuiltin_printdefault`
     case (31) calls `.Internal(print.default(x, ...8 args))`; R rejects it
     with `Error: 8 arguments passed to .Internal(print.default) which
     requires 3`. robigo reports `invalid 'quote' argument` because
     `do_printdefault` never checks arity. The nine `format.POSIXlt` arity
     cases are the same class of bug but report against the call
     (`Error in format.POSIXlt(...) : ...`) instead of the context
     (`Error: ...`) that R's `checkArity` uses for `.Internal`s.
  2. **`deparse` cutoff** (1): an out-of-range `width.cutoff` is a warning in
     R (`do_deparse` warns and uses the default 60), not an error.
  3. **class attribute keeps its own attributes** (2 deparse/dput): robigo's
     `class_gets` strips the attributes of the class vector
     (`c2.clear_attrs()`), so `class = structure("integer(0)", names = "c0")`
     deparses as `"integer(0)"` instead of `c(c0 = "integer(0)")`. R's
     `classgets` installs the value untouched (`R: attrib.c:classgets`).
  4. **dput width** (2): `do_dput` uses `deparse1()` = cutoff 60
     (`R: deparse.c:do_dput`); robigo used 70, so line wrapping differs.
  5. **missing argument in `.Internal`** (1 `format` case): R's `do_internal`
     evaluates the inner call's arguments with the *outer* `.Internal(...)`
     call (`R: names.c:do_internal` -> `evalList(args, env, call)`), so
     `argument 10 is empty` is reported against `.Internal(format(...))`.
  6. **missing argument's call** (1 `cat` case): R reports the missing
     argument against the *lexical* call of the environment holding the
     binding (`R: eval.c` -> `R_MissingArgError(e, getLexicalCall(rho))`),
     i.e. `foo(42, )`, not the inner `cat(a, b)` call.
  7. **`%Z` for `tz = "GMT"`** (1): with `TZ=UTC`, GNU R prints `UTC` (the
     zone name from `localtime`), robigo prints the literal `GMT`. This is
     in `builtins/datetime/mod.rs`, outside this track's files.
