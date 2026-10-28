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

## 2026-10-28: 390 -> 436 of 437

- why: port each root cause to GNU R's C code rather than patching output.
- fixes: `print.c:do_printdefault` arity; `util.c:Rf_checkArityCall`
  (`error` for `.Internal`, `errorcall` for primitives);
  `deparse.c:do_deparse` cutoff warning; `deparse.c:do_dput` cutoff 60;
  `attrib.c:classgets` keeps the class vector's attributes;
  `names.c:do_internal` evaluates arguments with the outer call;
  `eval.c` `R_MissingArgError(e, getLexicalCall(rho))`.
- result: **436/437**, 46 new passes, 0 regressions; 46 entries added to
  `tests/corpora/fastr/passing.txt`.
- remaining: the `%Z` case above (datetime owner). With `TZ=UTC`,
  `strftime(as.POSIXct(..., tz = "GMT"), "%Z")` must print `UTC` because
  `as.POSIXlt(x, tz = "")` (what `strftime()` does) leaves `tzone` empty and
  `localtime` reports the process zone; `format()` (which sets `TZ=GMT`)
  prints `GMT` and is already correct.

## gnur-examples (print/format/deparse pages)

Filter `^base/(print|print.dataframe|print.default|cat|toString|deparse|
deparseOpts|dput|format|format.info|format.pval|formatDL|formatc|
encodeString)$`: 11/14 -> **12/14**.

- `base/dput`: ported `hexNumeric` (`%a`) and complex `digits17` to the
  deparser (`R: deparse.c:vector2buff`); `format.rs` now has
  `encode_hex_real`/`encode_hex_complex` (glibc `%a` spelling) and the
  `%+.17g` imaginary part.
- `base/format`: blocked on the methods package native routine
  `R_initMethodDispatch`, which robigo does not implement (G2); not this
  track.
- `base/print`: blocked on `NextMethod()` dropping missing arguments
  (`R: objects.c:do_nextmethod` -> `match.c:patchArgsByActuals` keeps
  `R_MissingArg` in place). `T[, 2:8]` on a `table` therefore reaches the
  `[` primitive as `T[2:8]`, losing `dim`; `format()` then returns a plain
  vector and `print.table` prints `[1] ...`. Fix belongs to conformance-lang
  (`builtins/lang.rs`), not this track.
