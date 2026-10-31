# conformance-sweep-small: small missing internals and error paths

## 2026-10-31: unlink, rowsum, gettext, lengths, max.col, system, envir

- why: a sweep of `fastr` cases where one small builtin was either
  missing, lacked `checkArity`, or reported the wrong message or call.
  Each is a direct port of the named C function; no evaluator changes.
- what:
  - `builtins/sysutils.rs`: `unlink` calls `checkArity(4)` first
    (platform.c:do_unlink) so three arguments report
    `3 arguments passed to .Internal(unlink) which requires 4`.
    `system(timeout=)` ports `unix/sys-unix.c:R_system_timeout`: the
    child runs in its own process group (`setpgid`), its stdout is
    drained concurrently so a full pipe cannot cause a false timeout,
    the deadline is timed from before the fork and slept to exactly, and
    on expiry the whole group is killed. Captured output is returned with
    `status = 124`, and `command '<cmd>' timed out after <n>s` is warned. `setSessionTimeLimit` exists
    as `.Internal` (sysutils.c:do_setSessionTimeLimit) and validates its
    two arguments; robigo has no watchdog hook yet, so the limits are
    accepted but not enforced (recorded here as an open item).
  - `builtins/unique.rs`: `rowsum_matrix` checks its `uniqueg` with
    `HashTableSetup`'s type set, so NULL/closure report
    `unimplemented type '<type>' in 'HashTableSetup'` instead of
    panicking on an out-of-bounds index.
  - `builtins/misc.rs`: `gettext(NULL, NULL)` returns NULL (errors.c:
    do_gettext returns the empty string argument as is);
    `bindtextdomain` returns the directory it was given and TRUE for
    NULL/NULL; `ngettext` converts `n` like `asInteger` (first atomic
    element only; a complex first element warns, a list is NA) and says
    `invalid 'n' argument`.
  - `builtins/bind.rs`: `lengths` validates `use.names` with asLogical
    (`invalid 'use.names' value`) and reports `'x' must be a list or
    atomic vector` against the call.
  - `builtins/array.rs`: `max.col(ties.method = "random")` ports
    appl/maxcol.c's reservoir sampling (`ntie * unif_rand() < 1`), with
    GetRNGstate/PutRNGstate around the first tie so the RNG stream is
    R's.
  - `builtins/envir.rs`: `lockEnvironment` evaluates `bindings` before
    the environment check like envir.c:do_lockEnv: length > 1 says
    `'length = N' in coercion to 'logical(1)'`, NA says
    `NA in coercion to boolean` (both `errorcall` on the internal's
    call, NA also for a list as `asLogical2` does), and a
    non-environment says `not an environment` with `error()`'s context. `parent.env` reports `argument is not an
    environment` against the caller's call.
  - `builtins/funs.rs`: `formals` warns `argument is not a function` for
    a non-function, as builtin.c does and `body()` already did.
- tests: `tests/r/basics/small-internals.R`, recorded with GNU R 4.5.0
  in `robigo-oracle:4.5.0`; it passes the interpreter, VM and JIT tiers
  and the full conformance suite stays green.
- result: `fastr` 31,183/31,374 -> 31,212/31,374 (+29, zero new
  regressions; the same two pre-existing host failures remain, the
  `order` locale case and the 17 GB compact-realseq serialize case).
  29 new ratchet markers. Known not addressed here: `mget`'s
  `ifnotfound` coercion and vector `mode` (3 cases in the same filters).
