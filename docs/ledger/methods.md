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
- **The S4 bit** is `Attrs.1` (`Value::is_s4`/`set_s4`). Code that rebuilds a
  value's attributes must carry it the way GNU R does: `copy_most_attrib`
  copies it, `set_attrs(box)` keeps whatever the box has, `clear_attrs`
  drops it (GNU R's `CLEAR_ATTRIB`).

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
