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
    at run time. The from-source compiler (`compile.rs` `builtin_args_call`)
    tests a known count with `internal_arity_ok`, and routes a wrong one
    through the list path (`call_builtin_list`). That path evaluates the
    arguments, then raises checkArity's error, as GNU R does. A runtime check
    on the register path cost binarytrees +0.16% to +0.53% instructions,
    because its code generation shifted (#820's description has the
    per-function numbers); this way the path is main's code.
  - **Every other path** that calls a builtin's function (`run_builtin`,
    compiled assignment steps and applyMethod through `enter_builtin`, the
    registry's specials through `apply_special_args`): one compare against
    the row's arity inline, the rest `#[cold]`.
  - **The VM's call of a registry special** (`call_special_args`): its rows
    check themselves (`lapply`, `vapply`), or take any count (`rep`).
- The one difference from GNU R, accepted by the foreman: **GNU R byte
  code with a wrong-count `.Internal`** (a CALLBUILTIN of the internal) is
  refused by the byte-code translation (`bctrans.rs` `call_builtin`).
  Calling the closure raises "robigo: translating byte code calling an
  .Internal with a count its checkArity refuses is not implemented yet",
  where GNU R raises checkArity's error. Byte code's frame holds the
  arguments already evaluated, so it cannot take the list path.
  - Case: `tests/rt/bctrans/internal-arity.R`, from
    `scripts/bc-fixtures/internal-arity.R`: GNU R's compiled
    `function() .Internal(Sys.getenv())`. It records GNU R's
    "0 arguments passed to .Internal(Sys.getenv) which requires 2" and is
    not ratcheted. Its second closure, a right-count `paste0`, translates
    and agrees.
