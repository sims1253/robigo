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
