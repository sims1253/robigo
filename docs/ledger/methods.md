# methods (G6: methods/S4)

Metric: fastr S4 cases and S4-related builtins, gnur-examples `^(methods|stats4)/`,
`library(methods)` + Rcpp loading, then the corpora with methods as a default
package; all byte-identical to GNU R 4.5.0, no regressions.

Slice used throughout ("S4 slice"):
`scripts/corpus.py run fastr --filter 'S4|TestBuiltin_(setS4Object|isS4|slot|attr|standardGeneric|isVirtualClass|asS4|new|setClass|setGeneric|setMethod|validObject|methods|getClass|is\b|inherits|existsMethod|getGeneric|showMethods|selectMethod|removeMethod|isGeneric|unclass|oldClass|class)'`

## Hook contract (for the VM and other evaluator paths)

S4 lives in `crates/robigo-runtime/src/s4.rs` (evaluator side: GNU R's
objects.c/attrib.c/eval.c parts) and `native/methods.rs` (methods' natives).
What other evaluator paths must keep:

- **Closure frames.** `standardGeneric` finds its generic by walking
  `interp.frames` from the top for a non-builtin frame whose `function` has a
  `generic` attribute (R: `get_this_generic`), and `R_execMethod`
  (`Interp::exec_method`) takes `call`, `sysparent` and `promargs` of the
  innermost non-builtin frame (`current_frame()`), which must be the generic's.
  So any call path that runs a closure body must push a `Context` with the
  right `function`, `call`, `cloenv`, `sysparent` and `promargs` (as
  `exec_closure` does) and pop it with `end_context`. Inlining a closure
  without a frame is only safe if its body cannot reach `standardGeneric`,
  `sys.function`, `sys.call`, `parent.frame` or `missing()`.
- **Methods run through `exec_closure`.** `exec_method` builds the method's
  frame itself (bindings copied from the generic's frame, missingness kept as
  `BIND_MISSING_DEFAULT`, unforced default promises replaced by the method's
  own defaults) and then calls `Interp::exec_closure(call, method, promargs,
  newrho, sysparent)`; a VM frame push must accept a prebuilt environment.
- **Missingness.** Dispatch reads it with `s4::binding_missing(env, sym)`: a
  binding flagged `BIND_MISSING_DEFAULT` or holding the missing argument. A
  VM that binds arguments itself must set the same flag for defaulted formals.
- **Internal dispatch.** Primitives that dispatch must go through
  `try_dispatch`/`try_dispatch_group`/`dispatch_with_temp_context`: they try
  the S4 methods of the primitive (`R_possible_dispatch`) for S4 objects
  before S3. Fast paths may skip them only for values without a class
  attribute (`is_object()` false implies `is_s4()` false for everything but
  `asS4()` on a classless value, which has no methods anyway).
- **OBJSXP** is `Value::Obj(Rc<Vector<Value>>)` with empty `data` (see the
  perf entry below for why it shares List's payload type).
- **The S4 bit** is `Attrs.1` (`Value::is_s4`/`set_s4`). Code that rebuilds a
  value's attributes must carry it the way GNU R does: `copy_most_attrib`
  copies it, `set_attrs(box)` keeps whatever the box has, `clear_attrs`
  drops it (GNU R's `CLEAR_ATTRIB`).

## 2026-10-28: Rcpp's own test suite (in progress)

- `tinytest::test_package("Rcpp")` cannot run on robigo yet, for two
  reasons outside S4: tinytest opens `grDevices::pdf(nullfile())` per test
  file (grDevices' PDF native), and it reads each expression's srcref for
  line numbers (`env$lst - env$fst` is NA without source references:
  robigo's `parse(keep.source = TRUE)` keeps none).
- Measured instead with a minimal stand-in for tinytest's `expect_*`
  (`sys.source` per test file, `RunAllRcppTests=yes`), the same script on
  GNU R 4.5.0 (Rcpp 1.1.2 built there) and on robigo (Rcpp 1.1.2 built by
  `install.packages()` inside robigo), one process per file on robigo
  because a test file crashed it.
- Result (assertions): GNU R 1,624 passed / 4 failed; robigo 1,101 / 13
  before, then with the C API added for it (R 4.5's
  `R_ClosureEnv/Formals/Body/Expr` through the bridge; Rmath's
  `Rf_tetragamma`, `Rf_pentagamma`, `Rf_ftrunc`): test_function 9 -> 22/22,
  test_support 0 -> 5/5. test_sugar (493 assertions on GNU R) now loads
  its object but segfaults in it (being located). Smaller differences in
  test_date (3), test_listof (2), test_matrix (3), test_misc (1),
  test_string (1), test_vector (3), test_xptr.
- Still unexported Rmath functions (pointer arguments, need hand-written
  wrappers in robigo-capi): bessel_{i,j,k,y}_ex, dpsifn, lgammafn_sign,
  logspace_sum, pnorm_both, and dtukey/rtukey/rnbeta/rnf/rnt.

## 2026-10-28: the S4 cases lost with methods attached, re-checked

- The 12 fastr S4 cases (TestS4, TestR5) that failed only with methods
  attached, run against GNU R with methods attached: 11 now match. The
  remaining one prints a reference class generator; its "Class Methods"
  list is `names(def@refMethods)`, i.e. `ls(sorted = FALSE)` of a hashed
  environment, which GNU R returns in hash-bucket order (PJW hash of the
  name, table size and growth from envir.c's R_HashSizeCheck/R_HashResize).
  robigo's environments do not emulate that order (env.rs, not this
  track).
- Also open from that set: reference class `finalize` methods
  (`reg.finalizer` on environments with `gc()`; robigo does not collect
  environments), `asS4(NULL)` setting the S4 bit of the global NULL
  (bug-compatible, not emulated), `substituteDirect` cases (pending
  re-recording).

## 2026-10-28: S4 dispatch overhead per call

- callgrind (scripts/callgrind-diff.py, 2e4 - 1e4 calls of a one-method
  generic): 14,398 -> 13,192 instructions per call (-8.4%):
  - the calling handler around each signature argument's evaluation
    (`R_evalHandleError`) is skipped when the argument's binding is a
    value or a forced promise: evaluating it runs no code, so nothing can
    signal (the handler cost an Rc closure, an entry and a message string
    per argument per call);
  - the dispatch's fixed symbols (`.AllMTable`, `.SigArgs`, `.SigLength`,
    `target`, `defined`, `nextMethod`, `allMethods`) are interned once;
  - the signature label is built in one String (no Vec<String>, join).
- What remains of the dispatch: `find_var_in_frame` (~10 frame lookups per
  call: the generic's tables, `.target`/`.defined`/`.Method`), defining
  `.target`/`.defined`/`.nextMethod`/`.Method` in the generic's frame and
  copying them into the method's (GNU R does the same). The rest is the VM
  running the generic's and the method's frames.
- Wall time is too noisy on this shared machine to compare with GNU R
  (GNU R's own numbers moved 2-3x between runs); roughly 1.3-2.3x slower
  on S4-heavy loops (generic calls, `+` methods, `new()`).

## 2026-10-28: S4 deparse; recording fastr with methods (memory)

- Deparse of S4 objects ported (deparse.c's IS_S4_OBJECT branch); the
  parts that need R (getClassDef, R_do_slot of `.Data`, asS3) are computed
  by `Interp::deparse_prepared` before the interpreter-free deparser runs
  (thread-local table keyed by object identity, cleared afterwards).
  Printing a generic, dput/deparse of S4 objects: byte-identical
  (tests/r/s4/deparse.R). fastr S4 cases recorded with methods attached
  (recorded chunks): 115 -> 118/124.
- Recording fastr with methods: chunks 0, 1, 4-9 recorded; 2, 3, a-f not
  yet. Chunk a/b was OOM-killed under the 6 GiB container cap (docker
  events: exit 137), also with the stats distribution groups excluded.
  The cap did its job; the case is being found with a probe recording
  (`ulimit -v 3000000` per process, scratch copy, compared with the old
  expectations).

## 2026-10-28: reg-S4 (GNU R's S4 regression test) with methods attached

Climbing gnur-tests' reg-S4 (recorded with methods attached); each stop
fixed and checked against the oracle:

- line 285: `cbind(m)` with an S4 argument: bind.c:do_bind dispatches S3
  methods on `R_data_class2` and falls back to `methods:::cbind`/`rbind`
  when any argument is S4 (builtins/array.rs, minimal). tests/r/s4/bind.R.
- line 470: `setMethod("as.numeric", ...)` must also serve `as.double()`:
  base binds `as.numeric` to the `as.double` primitive, whose method tables
  hold both; robigo's builtin passes the name it was called by
  (`s4::prim_op` maps it).
- line 660: `2 == <S4 list>`: robigo's relop rejected every list. GNU R
  4.5.0 compares lists by coercing them to the comparison's type
  (`coerceVectorList`; non-character targets need `isVectorizable`, else
  "'list' object cannot be coerced to type 'double'"); expressions go the
  same way (the expression check is `#ifdef previous_R_versions`).
  builtins/arith.rs, minimal. tests/r/s4/groups.R.
- After these, reg-S4 runs to its end; what remains is `getSrcref()`
  (robigo returns NULL for any function parsed with keep.source, not
  only methods: parser/print tracks) and a timing line.
- Measurement trap: a robigo binary copied to `/tmp/x/robigo` loads base
  natives from `/tmp/natives` (`<exe>/../natives`), where another agent
  had built GNU R's methods.so (which fails: `DDVAL`). Copies used for
  measuring must live where `../natives` does not exist.

## 2026-10-28: Matrix from source: blocked on grid (not S4)

- `install.packages("Matrix")` inside robigo first builds its dependency
  lattice, whose lazy-loading step loads grid: `.Call(C_initGrid)` is not
  implemented (grid's natives: core's `scripts/build-natives.sh grid` or
  a port; graphics engine, not this track). Retry Matrix once grid
  loads; it is the S4 stress test after Rcpp (dgCMatrix & co).

## 2026-10-28: methods as a default package, per corpus (host, same binary)

Recorded with methods in `default_packages` (oracle image, uid 1000,
`--memory 6g`), robigo with the matching default packages:

| corpus | without methods | with methods | lost | gained |
|---|---|---|---|---|
| gnur-examples | 591 | 603 | file.access, system (recording artifacts: uid, seccomp) | 14 (some date-dependent) |
| gnur-tests (without reg-large) | 21 | 19 | reg-S4-examples (grDevices `devholdflush` once `plot()` is reached), reg-packages (the host's `R CMD` subprocesses use system R 4.6.1) | 0 |
| gnur-demos | 2 | 2 | | |
| r-intro | 4 | 4 | | |
| fastr | (recording) | | | |

The losses are cases that "passed" only because GNU R and robigo stopped
at the same early error without methods; with methods GNU R gets further.
None is an S4 failure. For the switch: record as root (as CI's container
runs), leave reg-large to CI, and decide with the maintainer how a case
whose recorded behaviour changes is treated in the ratchet (it is the
same script with a different oracle).

## 2026-10-28: S4 speed vs GNU R (measured, not yet improved)

- Wall time, robigo vs GNU R 4.5.0 (oracle image): 1e5 calls of a
  one-method generic 0.85 s vs 0.64 s; 2e4 `+` with an S4 method 5.84 s
  vs 4.44 s; 2e4 `new("P", x = i)` 16.25 s vs 9.32 s. robigo is
  1.3-1.7x slower.
- callgrind on `new()` (2000 - 1000 calls): 884k instructions per call.
  The dispatch machinery (`dispatch_generic` without the method,
  `load_method`, `do_slot`) is ~1%; the rest is the VM running methods'
  own R code (`new` -> `initialize` -> the default method, `validObject`,
  `getClass`). 13.6% is `vm::guard_ok_slow`: frames of namespace closures
  (parent = the methods namespace, not on the search path) miss the guard
  memo's fast paths, so every guarded call site rechecks. resolves_to_base
  itself is negligible.
- Next: make namespace-closure frames hit the guard memo (perf-vm,
  vm.rs), then re-measure; and an S4 benchmark in benchmarks/ (generic
  call, `new`, a primitive method) so perf CI tracks it.

## 2026-10-28: #52 merged; CI results; trace(); recording with methods

- #52 merged. CI, branch vs main's latest run: fastr 30320 -> 30369
  (+49), gnur-examples 559 -> 605 (+46), gnur-tests 20 -> 21 (+1), no case
  lost in any corpus; perf job green.
- `traceOnOff`/`debugOnOff` (debug.c) replaced placeholders that returned
  FALSE: `.doTrace` now runs, so `trace(f, quote(...))` and
  `trace("g", ..., signature = "numeric")` (methods' `.TraceWithMethods`)
  print GNU R's output (tests/r/s4/trace.R). Open: `trace("f")` without a
  tracer goes through `.primTrace` and the RTRACE bit, which needs the
  evaluator/VM call path to print `trace: f(1)` (VM owner).
- Methods as a default package, measurement in a scratch worktree
  (corpora.toml `default_packages` + methods, recorded in the oracle
  image): gnur-examples 591 -> 603 passing on this host with the same
  binary; the 2 "losses" are recording-environment artifacts (Docker uid
  for file.access, seccomp's "Function not implemented" for system()).
- **Memory incident:** recording gnur-tests with `--jobs 4` ran its
  `reg-large` case (8+ GB) next to three other recordings; the box (14 GB,
  shared) ran out and the core agent was OOM-killed. Now: `free -g` check
  before each recording, `docker run --memory 6g --memory-swap 6g`,
  `--jobs 1` for gnur-tests with `--filter '^(?!reg-large$)'`; reg-large
  is left to CI. fastr is recorded in 8 chunks (last hex digit of the id)
  because `record` writes only at the end.

## 2026-10-28: bare robigo starts with GNU R's default packages; startup cost

- `robigo script.R` without `R_DEFAULT_PACKAGES` forced it to NULL ("cannot
  load the default packages yet"); it now gets the profile's defaults,
  methods included, as GNU R does (crates/robigo-cli/src/main.rs, one
  block removed; the conformance runner now sets NULL itself, as
  regen-expected.sh records). Corpora, conformance and benchmarks set the
  variable explicitly and are unchanged.
- Startup, 5 runs each: robigo 230 ms without methods, 315 ms with;
  GNU R 4.5.0 (oracle image) 369 ms without, 672 ms with.
- Where methods' 162M startup instructions go (callgrind, `-e
  'invisible(1)'`, methods vs NULL): ~35M is zlib set-up, not inflating:
  `miniz_oxide::InflateState::new_boxed` and its zeroed allocation once
  per lazy-load fetch (273 fetches). One reused inflate state would remove
  it (builtins/serialize.rs, startup track). The rest is unserialize
  (`read_item`, bytecode `read_bc_lang`) and `..onLoad`'s R code.

## 2026-10-28: relation to core.md §13 (methods.so's entry points)

- core.md §13 lists the 22 non-API entry points GNU R's methods.so would
  import. Every one of them now exists in the runtime (`s4.rs`), and the
  S4 bit and OBJSXP cross the bridge both ways. The methods natives
  themselves are served by `native/methods.rs`; `scripts/build-natives.sh`
  builds only stats by default, so `dyn.load` keeps using the port unless
  someone builds methods.so explicitly.
- To make methods.so loadable later (a measurement, not a switch), the C
  side of those entry points would be thin host callbacks into `s4.rs`:
  `R_set_standardGeneric_ptr`/`R_set_quick_method_check` (store the C
  function pointers and call them from `Interp::standard_generic` /
  `possible_dispatch`), `R_execMethod`, the `prim_*` tables,
  `R_findVarLocInFrame`/`R_GetVarLocMISSING` (`s4::binding_missing`).
  Expected costs, from the bridge's design (core.md §6): argument copies on
  every dispatch and a lost in-place `R_set_slot` (which `initialize()`
  relies on), so the port stays the default until G3.
- Already in libR (robigo-capi `objects.rs`, for packages): `R_do_slot`,
  `R_do_slot_assign`, `R_has_slot`, `R_do_new_object`, `R_do_MAKE_CLASS`,
  `R_getClassDef`, `Rf_asS4`.

## 2026-10-28: Rcpp loads and runs; S4 environments; zero-cost OBJSXP; full corpus

- **Rcpp from source inside robigo.** With robigo's R_HOME
  (`ROBIGO_NATIVES=<dir with tools/tools.so> scripts/robigo-home.sh`,
  tools.so from core's `build-base-native.sh`), `utils::install.packages("Rcpp")`
  compiles Rcpp 1.1.2 against robigo's headers and passes R's load tests.
  `library(Rcpp)`, `evalCpp("1 + 1")`, `cppFunction()` (scalars, vectors,
  `List::create`), `Rcpp::stop()` caught by `tryCatch`, and an
  `RCPP_MODULE` class (`new(Acc, 1)`, `$add()`, `$get()`) all work.
  Needed: libR's S4 entry points (`crates/robigo-capi/src/objects.rs`:
  `R_do_slot`, `R_do_slot_assign`, `R_has_slot`, `R_do_new_object`,
  `R_do_MAKE_CLASS`, `R_getClassDef`, `Rf_asS4`), ported over core objects
  so that they modify in place as GNU R's do; S4 objects extending
  "environment" in `get`/`exists`/`assign`/`ls`/locking/`environment<-`/
  `parent.env`/`new.env`/`eval`/`[[`/`$`/`[[<-`/`$<-`/`is.environment`
  (GNU R's `simple_as_environment`, `R_getS4DataSlot`); `mget`,
  `list2env` and `delayedAssign` reject them as GNU R does.
- **R_execMethod with constant defaults.** robigo binds constant defaults
  without a promise; such a binding of a missing argument is the
  generic's default too and becomes the method's own (callNextMethod
  example). 
- **OBJSXP costs nothing.** With `Value::Obj(Rc<Obj>)` (its own payload
  type), `perf.py compare` against main showed fib +0.58% (> 0.5%): callgrind
  (scripts/callgrind-diff.py, fib(21) - fib(20)) put +29 instructions per
  call into drop glue that LLVM stopped inlining into `Rc<EnvData>::drop_slow`
  once `Value`'s drop glue had another distinct arm. `Obj` is now
  `Vector<Value>` (List's payload, `data` empty): the arms are identical
  and merge. Result: **0 of 78 benchmarks slower or faster than 0.5%**.
- **Full fastr corpus** (4 shards, host): 0 regressions against main's
  binary on this host. 13 complex NaN-payload cases (TestSimpleValues,
  TestSimpleArithmetic, one TestBuiltin_operators) and the
  `c_library_oracle` unit test fail for main and this branch alike on this
  host (#47 encodes gcc/glibc bits of CI's image). Of the 528 cases that
  pass beyond the ratchet, main passes 484: **+44 over main**, all S4.
- Slices now: S4 slice 608 -> 639/662; gnur-examples methods/stats4
  0 -> 41/50. Conformance: `tests/r/s4/{classes,dispatch,refclass,bits}.R`
  (recorded in the oracle image) pass and are in the ratchet.
- Open (outside this track, noted for their owners):
  - stats4 `mle`/`update-methods`: the stats `optim` native (Rust port
    missing; GNU R's stats.so built from source has it, core's ledger);
  - methods `setMethod`: grDevices `devholdflush`; `showMethods`:
    `lazyLoadDBflush` (unloadNamespace); `cbind2`: `trace()`;
    `GenericFunctions`/`implicitGeneric`: the `<bytecode>` line when
    printing base closures (corpus-scripts ledger);
    `Methods_for_Nongenerics`: a timestamp inside IGNORE_RDIFF (never
    byte-identical); `inheritedSlotNames`: Matrix not installed here;
    `refClass` (gnur-examples): GNU R's recording has codetools, the host
    R_HOME does not ("not checked for suspicious field assignments");
  - `f <- function(...) lapply(1, log, ...); f()` fails on main: `...`
    reaching a special through lapply (conformance-lang/vectors).
  - Rcpp's help pages: `RdTags(Rd) == "COMMENT"` errors during install
    (tools' Rd code on robigo).
- Next: methods as a default package (re-record every corpus with methods
  in `default_packages`, robigo's startup attaching it), Rcpp's own tests,
  `trace()` for S4 (`.primTrace`, methods' trace machinery).

## 2026-10-28: methods loads; setClass/new/setGeneric/setMethod/show

- why: `library(methods)` failed in `R_initMethodDispatch` (no port), and the
  runtime had no S4 bit, no OBJSXP, no `standardGeneric`, no slots.
- baseline (main 414ae44): S4 slice 608/662 (TestS4 84/102, setS4Object 1/21,
  asS4 0/1); gnur-examples methods/stats4 0/50 (all "package or namespace load
  failed for 'methods'").
- what:
  - S4 bit in `Attrs`, `Value::Obj` (OBJSXP); serialize reads/writes both
    (methods' lazy-load DB is full of S4 objects and S4-bit closures).
  - `s4.rs`: `standardGeneric` (+ `dispatchNonGeneric`), primitive-method
    tables, `R_possible_dispatch`, `R_execMethod`, `asS4`/`setS4Object`,
    `R_do_slot`/`R_do_slot_assign`/`R_has_slot`, `R_getS4DataSlot`,
    `R_data_class`, `S4_extends`/`.cache_class`/`.class2`,
    `R_do_new_object`, `PrintObjectS4`.
  - `native/methods.rs`: the 26 registered routines of methods.so
    (table dispatch `R_dispatchGeneric`, `R_quick_dispatch`, old MethodsList
    dispatch, `R_nextMethodCall`, `R_getGeneric`, `R_getClassFromCache`,
    `R_loadMethod`, ...).
  - `conditions.rs`: Rust calling error handlers
    (`R_withCallingErrorHandler`), with GNU R's handler call
    `h(simpleError(msg, call))` in the converted error.
  - `@`, `@<-` (+ `checkAtAssignment`), `isS4`, `class<-`/`oldClass<-`
    S4 warnings, `oldClass` `.S3Class`, `inherits` (R_data_class / S4
    extends), `attributes<-` keeps the bit, `copyMostAttrib` copies it,
    `[<-` embeds OBJSXP values (embedInVector), print of S4 objects without
    methods.
  - **R_set_slot in place.** GNU R's `R_set_slot` modifies its argument and
    methods' default `initialize()` relies on it (`"slot<-"(.Object, ...)`
    without assignment). robigo rebinds the caller's variable the `object`
    promise came from, if it still holds that very object.
- result: S4 slice 608 -> 636/662; gnur-examples methods/stats4 0 -> 38/50;
  a setClass/new/setGeneric/setMethod/show/contains="numeric" script is
  byte-identical to GNU R.
- **Why Rust ports, not methods.so built from source.** main serves every
  base package's natives from Rust ports (`native/`); GNU R's own `.so`s
  (core's build-base-native.sh, #37) are opt-in and need R sources and a C
  toolchain in CI (a maintainer decision, docs/ledger/core.md). For methods
  the bridge would also be wrong and slow: it copies arguments (every
  dispatch would copy every argument's slots), `R_set_slot`'s in-place
  modification cannot reach the caller through a copy, and dispatch calls
  back into the evaluator for every frame lookup. methods' C code is 1,500
  lines; the evaluator side (objects.c/attrib.c/eval.c), which is most of
  the work, is needed in Rust either way. Once the runtime is on core
  objects (G3), methods.so can be measured against the port.
