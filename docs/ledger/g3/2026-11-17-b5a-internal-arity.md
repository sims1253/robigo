## 2026-11-17: an .Internal's arity, checked at compile time for the VM's register calls (B5a, #820)

- What: GNU R checks an `.Internal`'s argument count in each function, with
  `checkArity` (util.c `Rf_checkArityCall`) as its first statement.
  robigo-rt reproduces that once, against names.c's arity column, so that
  no builtin reads past its arguments (`.Internal(radixsort())` and
  `.Internal(dyn.load())` panicked before).
  - Excepted are the rows whose C does not start with checkArity
    (`OWN_ARITY_CHECK` in `crates/robigo-rt/src/call.rs`).
  - They were found by calling every fixed-arity `.Internal` of GNU R 4.6.2
    on the oracle with one argument too many and one too few, and by reading
    each one's C before its checkArity.
- Where the check runs:
  - **The VM's register call** (`call_builtin_regs` → `invoke_builtin`): not
    at run time. Both compilers test a known count with `internal_arity_ok`:
    the from-source compiler (`compile.rs` `builtin_args_call`) and the
    byte-code translation (`bctrans.rs` `call_builtin`). A wrong count is
    compiled as its arguments, evaluated in order, then checkArity's error
    (an `ErrorCall` against the context, as `Rf_checkArityCall`'s `error()`
    for an `.Internal`), so GNU R's own byte code with such a call runs as
    GNU R runs it. A runtime check on the register path cost binarytrees
    +0.16% to +0.53% instructions, because its code generation shifted
    (#820's description has the per-function numbers); this way the path
    is main's code.
  - **A call with `...` or an empty argument**: the list path
    (`call_builtin_list`), whose `run_builtin` checks the expanded count.
  - **Every other path** that calls a builtin's function (`run_builtin`,
    compiled assignment steps and applyMethod through `enter_builtin`, the
    registry's specials through `apply_special_args`): one compare against
    the row's arity inline, the rest `#[cold]`.
  - **The VM's call of a registry special** (`call_special_args`): its rows
    check themselves (`lapply`, `vapply`), or take any count (`rep`).
- Case: `tests/rt/bctrans/internal-arity.R`, from
  `scripts/bc-fixtures/internal-arity.R`: GNU R's compiled
  `function() .Internal(Sys.getenv())` (the message, and the closure's call
  as the condition's), a three-argument one whose second argument prints
  before the error, and a right-count `paste0`.
