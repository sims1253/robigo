## 2026-11-21: C1: translateToNative's warning for a name given as a string

Outside a UTF-8 locale, a name given as a string that does not translate
(`"中" <- v`, `missing("中")`) is installed with escapes
(`<U+4E2D>`) after installTrChar's warning "unable to translate '%s' to
native encoding" (sysutils.c, `translateToNative(mustWork = 2)`). Until now,
robigo-rt never gave that warning (before #953 it did not even translate).
The cases run under `LC_ALL=C` through `.locale` sidecars, recorded with
GNU R 4.6.2.

### Permanent

- **The warning at each evaluation**, where GNU R's interpreter gives it,
  and for `missing` in compiled code too (cmp.R leaves `missing` to the
  special). It names the function's call and comes before the value is
  evaluated. robigo-rt compiles an `Op::Internal` call of GNU R's own
  `as.vector(name, "symbol")` in front of the special:
  - its name is an attribute-free string of the first element, so nothing
    dispatches;
  - its value is dropped, and its site is the special's call;
  - it is an existing instruction, so `vm::run` gets no new arm.

  A name whose escapes take it past MAXIDSIZE gives the warning, then
  install's error. Cases: `jit/translate-warning-eval.R`,
  `translate-warning-long.R`.
- **cenv's findLocals port** (`string1_sym`) installed such a name's raw
  bytes, so a `Bc` frame's local named a different symbol from the one the
  code assigned. It now translates (`cenv::install_tr`).
- **#953's six lows:**
  - `bytecode_code`'s doc;
  - one `index_codes(…, shape)` helper;
  - `frame_shape_raw` returning Some only for a shape vector;
  - three bctrans docs.
- **The twins.** `jit/lazy-promise-code.R` and `jit/shared-body.R` load the
  compiler and took 147–266 s under the torture steps; they are `volume`
  now.
  - Their small twins (`-small.R`) unserialize closures that GNU R 4.6.2's
    `compiler::cmpfun` compiled, from the ascii serialization embedded in
    each, with the commands in their headers.
  - Nothing loads the compiler, and each takes about a second under
    gctorture and both moving-collector modes.
  - GNU R's recordings of the twins equal the originals'.

### Interim until V4 step 3 (cmp.R's own code compiling)

Where GNU R's JIT compiles an assignment to such a string, the warnings are
cmp.R's own, at the compile:
- findLocals' `addVar` gives `assign(v, 1, envir = vars)` once per pass of
  findLocalsList's loop;
- cmpAssign gives `as.name(getAssignedVar(e, cntxt))`;
- the compiled code gives none.

robigo-rt's from-source compile imitates them (`cenv::CompileWarnings`,
`compile::Prelude`):
- they are raised with the JIT off, as GNU R compiles with
  `R_jit_enabled = 0`;
- findLocals' warnings are raised before anything else of the compile: the
  attempt stops at the end of findLocals, so before `mayCallBrowser` or
  any force;
- the compile is then made again, keeping that findLocals' locals, so a
  handler that changes the environment (a global `+`, a delayed binding,
  `local`) is seen as GNU R sees it;
- the crosscheck's own compile forgets its warnings.

Case: `jit/translate-warning-bc.R`. Sol's rounds 1–4 each found a handler
that runs R code while the warnings are raised, and each was fixed by
ordering. The family converges only when cmp.R's code runs the compile,
so all of this goes at V4 step 3.

**Not yet GNU R's.** Each has an `INTERIM(C1)` marker, and V4 step 3
closes it:
- **`options(warn = 2)`** (issue #964): GNU R's `tryCmpfun` takes cmp.R's
  warning, now an error, with its innermost handler, and the closure runs
  uncompiled. Here the error leaves the compile with cmp.R's call.
  `jit/translate-warning-warn2.R`, recorded, not ratcheted; a Rust-level
  handler (S1's condition-system work) would also close it.
- **No R frames on the stack** while the warnings run: GNU R has the
  called function's and cmp.R's (`tryCmpfun` … `findLocals1`, `addVar`,
  `assign`). A handler's `sys.calls()` and an error's traceback differ.
  `jit/translate-warning-frames.R`, recorded, not ratcheted.
- **cmpAssign's `as.name` warnings** come after the compile, not during
  it, so a handler of one does not change the rest of the compile.
  `jit/translate-warning-asname.R`, recorded, not ratcheted.

### The rtcore port, and a gap it leaves

The run-time warning at a `<<-` site names the assignment call itself
under byte code, as GNU R's does, so `Cx::bc_interpreter_expression`
(rtcore `exit.rs`) gains `maybeAssignmentCall` (eval.c:7041). It is
applied only where `inflateAssignmentCall` leaves the call as it is.
- **The check (permanent):** case `jit/translate-warning-length.R`. The
  conditions and errors slices are the same before and after it, and S1
  owns those cases.
- **The gap (INTERIM(C1)):** a direct replacement call that GNU R
  inflates, such as `` `dim<-`(1, value = 2L) `` reported as
  `dim(1) <- 2L`, still gives the function's call. That predates this
  PR, as the inflated call is not made on that path.
  `jit/bc-expression-replacement.R`, recorded, not ratcheted.

### The pattern gctorture found twice today

A raw value held across something that can warn or run R code:
- #953: the frame shape held across `attempt`'s retries, Sol's round 1;
- here: the compile's fresh code object held while its warnings ran.

Both segfaulted only under gctorture or the moving collector. When a
change makes a warning, a condition or a forced promise happen inside a
build, look for every raw `Sexp` that lives across it.
