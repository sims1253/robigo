## 2026-11-17: S3x-c, user interrupts and C-level tryCatch

- What: the third part of S3x (m2m3-plan.md §B), on top of S3x-b (#795):
  - user interrupts at GNU R's points, with GNU R's semantics;
  - C-level tryCatch;
  - on.exit code in a context C pushed.
- **User interrupts** (main.c:handleInterrupt; errors.c:onintrEx,
  signalInterrupt, R_CheckUserInterrupt).
  - **The handler.** The binary installs GNU R's SIGINT handler as
    setup_Rmainloop does. A process that starts with SIGINT ignored keeps
    it ignored, so a shell's background job is not interrupted, in GNU R
    as here. The handler is one async-signal-safe store
    (`robigo_rtcore::note_sigint`).
  - **The poll.** The next `R_CheckUserInterrupt` takes the signal into the
    isolate's `R_interrupts_pending`. The evaluator's count and byte code's
    `BC_CHECK_SIGINT` already poll at GNU R's points. The poll's fast path
    is one relaxed load.
  - **`onintrEx`** is rt's interrupt hook (rtcore's
    `Hooks::interrupt`, whose default is the old plain exit):
    - The `interrupt` condition, an empty list of class
      `c("interrupt", "condition")`, goes to the handlers for `interrupt`
      or `condition`. Calling ones run in the global environment; an
      exiting one jumps. This happens in a new `CTXT_RESTART` record
      (`Cx::restart_context`) that offers the internal restart `resume`.
      A handler that invokes `resume` lets evaluation go on.
    - Then `options(interrupt)` runs.
    - Then the console's newline, and `jump_to_top_ex(TRUE, ...)`.
      `options(error)` runs only while `options(interrupt)` is NULL,
      GNU R's "transition".
  - `R_CheckUserInterrupt` is a C API export now.
- **C-level tryCatch** (errors.c:R_tryCatch, R_tryCatchError,
  R_withCallingErrorHandler, do_tryCatchHelper). They were stubs that
  raised "not implemented".
  - **The C side.** api.c has GNU R's three functions, with
    `tryCatchData_t` in the C caller's frame.
  - **The R side** is robigo-rtapi's entries over rt's
    `eval_trycatch_callback` and `push_calling_error_handler`. GNU R's two
    callback sources are parsed once per isolate in the base namespace and
    kept (`R_PreserveObject`).
  - **`.Internal(C_tryCatchHelper(addr, sw, cond))`** calls the C body,
    handler and finally back through native entries. It keeps
    `R_interrupts_suspended` as GNU R does: enabled for the body if it was
    at the call, suspended in tryCatch's R code.
- **on.exit code in a context C pushed.** `Cx::end_foreign`
  (`endcontext` of a record `begin_foreign` pushed) panicked on such code.
  It now pops the record and returns robigo-rt's "not implemented yet"
  error for the C caller to raise. No production code pushes such a record
  today; rtcore's `native` tests pin the error.
- Cases, recorded on GNU R 4.6.2 and ratcheted:
  - `tests/rt/capi/trycatch` (its C in `support/trycatch.c`): the body's
    value; an R error and a C error caught; classes, with a warning and a
    custom condition; one condition not caught here; finally after the body
    and after a handler; the defaults; no body;
    `R_withCallingErrorHandler`'s calling handler before an outer exiting
    one.
  - `tests/rt/capi/interrupt` (its C in `support/interrupt.c`, which raises
    SIGINT and calls `R_CheckUserInterrupt`):
    - exiting handlers by `interrupt` and by `condition`;
    - a calling handler that sees `resume` first and invokes it;
    - an `error` handler that does not see the interrupt;
    - on.exit code during the unwind;
    - `options(interrupt)`, which an exiting handler skips;
    - suspension, then a later poll;
    - a poll in a loop;
    - the uncaught interrupt: the newline, then "Execution halted".
  - robigo-rtapi's `tests/sigint.rs`: SIGINT from outside to a looping
    script, uncaught and caught, with on.exit code. The transcripts are
    GNU R's, recorded in the oracle image by the same protocol.
