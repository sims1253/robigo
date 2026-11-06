# core-packages ledger

CRAN packages with native code: install them from source against
robigo's headers (the source-compatibility contract, `docs/core.md`) and
run their examples/tests with GNU R's output as the target.

Metrics, in order:

1. `capi` corpus (`scripts/corpus.py run capi`, 23 cases on main after
   #171, all passing).
2. Weighted census coverage of libR's exports
   (`crates/robigo-capi/coverage.py`, from `tests/capi/symbols.tsv`).
3. The gate: `fastmap` then `digest` install from source inside robigo
   (`docs/core.md`, exit gate G2).
4. Popular CRAN packages with compiled code that install and whose
   examples/tests give GNU R's output — `scripts/cran-gate.py`, the
   manifest `tests/cran/packages.tsv` is the ratchet.

## 2026-11-01: baseline and the last census symbols

- Measured on `674f277f`: capi 22/22 pass; census 773/831 symbols,
  weighted 5281/5339 = 98.9%; 25 symbols left, all weight 1 (the
  `ptr_*` function pointers, graphics/HTTPD entry points, `do_*`,
  `R_runHandlers`). The gate packages have not been tried yet.
- The 58 weight-1 remainder is *not* where packages fail: a real package
  fails on behaviour of the symbols that exist (argument checking,
  attribute rules, encodings), not on a missing `R_withCallingErrorHandler`.
  So: take the cheap remaining symbols that CRAN packages do import in
  bulk (`R_qsort*`, `Rf_substitute`, the environment predicates, streams),
  then switch to the gate.
- Done (this PR): `R_tryEvalSilent`, `R_LockEnvironment`,
  `R_EnvironmentIsLocked`, `R_PackageEnvName`, `R_IsPackageEnv`,
  `Rf_substitute` (ports `coerce.c:substitute`/`substituteList` verbatim,
  including the `R_GlobalEnv` no-substitute rule and `...` splicing),
  `IS_SCALAR`, `XTRUELENGTH`, `LOGICAL_IS_SORTED` and `LOGICAL_NO_NA`
  (the last was a stub; the first two are exported by libR even though
  R's own headers implement them inline),
  `R_qsort`/`R_qsort_int`/`R_qsort_I`/`R_qsort_int_I`/`Rf_cPsort`
  (the Singleton quicksort of `qsort-body.c`, one generic body shared by
  the four entry points, so index/sort behaviour matches GNU R exactly,
  including the "ties stay put" property packages rely on), and
  `R_InitInPStream`/`R_Unserialize`. `R_Unserialize` scans the item with
  the format's own reader (serialize.c's `ReadItem` grammar: XDR, binary
  and ASCII, versions 2 and 3) so that it consumes *exactly* one item and
  hands those bytes to the runtime's `unserialize`, which is where the
  object is built; the scan is the whole point, since the caller's stream
  position must be where the next item starts.
- Result: capi 22/22 still; census 786/831 symbols, weighted
  5294/5339 = 99.2%. Kept.
- Not done: those exports have no corpus case yet (the corpus compiles
  each case against both GNU R and robigo and records GNU R's output;
  `R_InitInPStream` is the one the capi ledger already lists as
  uncovered). The unit test in `crates/robigo-capi/tests/capi.rs` pins the
  stream consumption with a hand-written XDR stream, which is the part
  that can silently break.

## 2026-11-01: the CRAN gate harness (metric 4)

- Both gate packages pass on current main: fastmap 1.2.0
  (tests/testthat, 341 expectations) and digest 0.6.39 (inst/tinytest,
  325 expectations) install with `robigo CMD INSTALL` from source and
  match GNU R 4.5.0 exactly.
- Shipped the metric-4 harness so this stays true: `scripts/cran-gate.py`
  (run/try/install/test/record/list), the manifest
  `tests/cran/packages.tsv` + GNU R's recorded `tests/cran/expected.json`
  (written only by `record`, which runs GNU R in `robigo-oracle:4.5.0`,
  never robigo), and `crates/robigo-capi/gate/shim.R` — a base-R-only
  tinytest/testthat shim that counts expectations, identical under both
  implementations. Wired as the `cran` CI job.
- Proof: fresh-cache `run` (fetch → install → test → compare) 2/2 match;
  tampering expected.json (341 → 999) makes `run` fail with exit 1.
- Next: `try` the popular set (rlang, vctrs, cli, glue, jsonlite, Rcpp,
  data.table, magrittr, R6, stringi), fix the most-unlocking blocker,
  add each package to the manifest as it passes.

## 2026-11-01: lazy-load promises force in their own environment

- The other half of WIP b2098e9b. `do_lazy_load_db_fetch` evaluated a
  PROMSXP's code in the global environment; R's
  `serialize.c:do_lazyLoadDBfetch` calls `eval(val, R_GlobalEnv)`, and
  `eval.c`'s PROMSXP case (`ENSURE_PROMISE_IS_EVALUATED`) forces a
  promise in its own PRENV — the `R_GlobalEnv` argument never reaches
  the promise. A database built from an environment whose promises were
  created with `delayedAssign(..., eval.env = e)` therefore forced its
  code in `e`; the old port looked up symbols in the global
  environment and failed.
- Both fetch paths (the flat-db cache and the per-file read) now route
  through `force_promise`, the port of eval.c's forcing (eval in PRENV,
  cache the value, clear PRENV).
- Tests: `tests/r/serialize/lazy-load-promise.R` builds a database with
  `tools:::makeLazyLoadDB` and forces a promise after `lazyLoad()`;
  full `tests/r` conformance 310/310 on the combined WIP.
## 2026-11-01: topenv honors matchThisEnv

- Took over WIP b2098e9b from the previous agent and finished it.
  `.Internal(topenv)` dropped its second argument: `envir.c:do_topenv`
  passes `CADR(args)` (matchThisEnv, the `topLevelEnvironment` option
  that `sys.source` sets to its `envir`) into `envir.c:topenv`, where
  the walk also stops at it. Code sourced with `sys.source` that calls
  `topenv(parent.frame())` — `methods`'s `setClass(where = ...)` shape —
  must define in the sourcing environment, not the namespace behind it.
- Port: `do_topenv` reads the second argument (anything but an
  environment means NULL, as R's `target != R_NilValue &&
  TYPEOF(target) != ENVSXP` check), and `topenv_from` adds the target to
  the stop set. R's loop tests `env != R_EmptyEnv` *before* `env ==
  target`, so the empty environment is never returned even when it is
  the target; mirrored by testing for the empty env first.
- Tests: `tests/r/lang/topenv-match-this-env.R` (recorded with GNU R
  4.5.0, pins the empty-env edge); full `tests/r` conformance 310/310
  on the combined WIP; `cargo test -p robigo-runtime` unchanged (two
  `builtins::complex` libm-oracle tests fail on this machine with and
  without the change).
## 2026-11-01: metric (d) baseline; parse(text=) coercion

- Baseline (cran-gate after #175, GNU R 4.5.0 oracle vs robigo, shim
  counts): fastmap 341/0 and digest 325/0 both match (the gate is
  green). First metric-(d) probes: jsonlite 1527/11 vs GNU R 1555/6;
  R6 562/17 vs 567/12; glue 124/114 vs 106/116 (needs dep parity in
  the oracle lib before the numbers mean anything); magrittr crashes
  the shim; cli fails to install (`robigo: bzip2 compressed file
  connections are not implemented yet` building its sysdata DB);
  vctrs installs, suite slow, still unmeasured; rlang known-hard
  (#174's author).
- jsonlite's two S4 failures: `parse(text = <expression>)` — R's
  `source.c:do_parse` runs `coerceVector(CAR(args), STRSXP)` first
  (an expression or call deparses into text) and errors only when the
  argument had length but the coercion produced none. The port
  demanded an already-character `text`. Fixed in `do_parse`; pinned
  by `tests/r/lang/parse-text-coerce.R` (GNU R's own answers,
  including `parse(text = quote(a + b))` → `expression(+a, b)`).
- R6's five extra failures are all `reg.finalizer` timing: after
  `local(obj <- BC$new())`, robigo's `gc(full = TRUE)` still sees the
  object reachable (the finalizer fires at a later gc, after
  `capture.output` closed); GNU R collects it there. The minimal
  `reg.finalizer` cases all match, so this is the frame-retention
  problem the core track's ledger already tracks — left to them.
- jsonlite's remaining gap is `url()` connections (7 tests). The
  R6/S4 work above also showed `methods::setClass` under `sys.source`
  needs #196's `topenv` matchThisEnv fix: without it `where` walks
  past the sourcing environment into a locked namespace
  ("cannot add bindings to a locked environment").
## 2026-11-01: return raises the catchable no-target error at the jump site

- magrittr 2.0.5 crashed the CRAN-gate shim: `1 %>% { return(.) }`
  evaluates the block from magrittr's C code via `Rf_eval`, and the
  unconsumed `Flow::Return` reached top level uncatchably ("Error: no
  function to return from ... Execution halted"). GNU R raises that as
  an ordinary *catchable* error — `context.c:findcontext` calls
  `error()` at the jump site, inside whatever handlers are active.
- First attempt (rejected): converting the flow in the bridge's
  `h_eval`. Sol review showed both failure modes: `tryCatch(1 %>%
  {stop("boom")}, error=conditionMessage)` lost "boom" (the conversion
  intercepted valid handler-result returns crossing C), and
  `1 %>% {tryCatch(return(.), error=function(e) 42)}` gave the outer
  error instead of 42 (the conversion happened after the inner
  handler's frame was gone).
- Kept fix: port findcontext where R has it — `do_return` checks
  `frames` (rev) for a non-builtin context whose `cloenv` is the
  return's frame (closure frames push one; `do_eval` pushes one with
  `cloenv = envir`, matching R's CTXT_RETURN context), and raises
  `Flow::error("no function to return from, jumping to top level")`
  when none matches. The VM's non-body `Op::Return` does the same
  check (`compile.rs` compiles `return()` calls in closure bodies to
  the consumed `is_body` path, so only eval'd units reach it).
  Break/next keep bubbling: R's loop contexts (CTXT_LOOP) are not
  modelled yet, so `{ break }` from C still reaches top level.
- Measured: Sol's two cases match GNU R (42, "boom"); magrittr's shim
  12 passed / 13 failed (GNU R 53/0); full tests/r conformance
  310/310; capi corpus 24/24, 0 regressions; robigo-runtime unit
  tests unchanged (the two libm-oracle complex failures are
  machine-specific, identical on the base commit).
- Still open for magrittr (next blockers, in order): promises leak
  from rlang's `r_env_bind_lazy` lazy bindings ("object of type
  'promise' is not subsettable", "invalid 'envir' argument of type
  'promise'", "Can't splice an object of type <promise>"), and
  `1 %>% identity() %>% return()` returns an external pointer instead
  of 1 (the trailing bare-`return` rhs in pipe.c's masking path).

## Eval depth limit and stack overflow conditions (#198, #199)

- `options(expressions=)` only ran through validation: the evaluator
  compared against a constant 5000. Overflow was a `simpleError`, so
  `tryCatch(stackOverflowError=)` missed it, and the C stack error was
  one too. GNU R's sources and the 4.5.0 oracle define what follows.
- Rejected: restoring the limit at `Flow::Return` landing sites, the
  literal port of `R_restore_globals`. In the interpreter tier every
  `return()` is a `Flow::Return`, so `invokeRestart`'s own R code dropped
  the limit under the running handler, which overflowed again. GNU R
  runs base compiled, where `return` is no jump. It also added a store
  to the return path.
- Kept: restore where the jump starts (`goto_exiting_handler`,
  `.invokeRestart`), in `default_error` (errors.c:`restore_inError`) and
  at top level; the `on.exit` raise is scoped to the `on.exit` code.
- Oracle facts: `lapply` recursion reaches the C stack; a compiled
  closure calling itself hits `nodeStackOverflowError` first, which
  robigo has no stack for. At `expressions = 100` GNU R aborts with
  "no more error handlers available" on `invokeRestart`; at 1000 it
  restarts, so the fixture uses 1000.
- Measured: `tests/r/conditions/{eval-depth-limit,c-stack-overflow}.R`
  byte-identical in all three tiers. fastr slice 1150/1151, 0
  regressions. Local fastr needs a 4.5.0 library: with the host's 4.6.1,
  every case reports "built under R version" noise on stderr.
- Open: GNU R raises `R_CStackLimit` by 1/0.95 during C stack recovery,
  so the deepest `on.exit` still runs; robigo can overflow again there
  and skip that one. Raising `stack_limit` needs the real stack margin
  checked first.

## options(error=) and tracebacks (#135, #136, part of #133)

- `options(error=)` was stored and never run, and `.Traceback` was never
  written. `default_error` now does `jump_to_top_ex`'s part with the
  frames still live, which is why `dump.frames()` from a handler works.
- #135's acceptance is wrong. GNU R 4.5.0 prints the handler's output,
  goes on with the script and exits 0 (`main.c:check_session_exit`).
  `catches_script_errors` already did that; recorded output wins.
- Recordings showed what the issues did not: `geterrmessage()` includes
  the `Calls:` line; warnings raised while the handler runs are dropped
  (`errors.c:476`); a symbol is refused; a quoted handler sees frame 0.
- `.Traceback` goes through `force_set`: base is locked and `set_local`
  fails quietly.
- Measured: `tests/r/conditions/error-option.R`,
  `tests/r/debug/{traceback,dump-frames}.R` byte-identical in all three
  tiers. fastr slice 763/764, 0 regressions.
- Sol: fixed a stale `geterrmessage()` after a failed handler and the
  missing `  ...` mark. Declined `.Internal(traceback(1+1i))` dropping the
  coercion warning: `.traceback` calls it only after `is.numeric`.
- Open: srcrefs on traceback entries (needs #186), `try_jump_to_restart`,
  `CTXT_BUILTIN` frames for `.Call`, and `recover()`'s menu (#132).

## 2026-11-02: gap trackers for the metric-(d) blockers

One conformance script per open gap found while probing the popular
CRAN packages (unmarked = known failure; #207 lists them on the
progress page):

- `tests/r/io/url-connection.R` — `url()` connections are not
  implemented; jsonlite's test-network-Github.R (7 of its 9 remaining
  failures; robigo 1542/9 vs GNU R 1555/6 after #204/#205).
- `tests/r/io/bzfile-roundtrip.R` — bzip2 file connections; cli's
  sysdata DB is bzip2-compressed, so `R CMD INSTALL cli` dies with
  "robigo: bzip2 compressed file connections are not implemented yet".
- `tests/r/env/reg-finalizer-local-gc.R` — the finalizer for an object
  whose scope ends in `local()` does not run during the following
  `gc(full = TRUE)` (the dead frame's value register keeps it
  reachable); R6's 5 extra failures (562/17 vs GNU R 567/12) and the
  gc ledger's retained-frame diagnostics.
- `tests/r/lang/return-no-target-do-call.R` (marked, passes) — pins
  #205: `do.call("return", list(1))` raises the catchable error.
  Tier-checked (compiled_matches_interpreted with the three new
  scripts green).

CRAN packages measured today (not in packages.tsv — the manifest is a
strict gate, these do not match GNU R yet): cli (install blocker:
bzip2 connections), magrittr 12/13 vs 53/0 (rlang `r_env_bind_lazy`
promises leak into subsetting/eval envir; `1 %>% identity() %>% return()`
returns an external pointer instead of 1), rlang (shim dies: the same
promise-leak family), vctrs (suite incomplete in 25 min under robigo),
glue 124/114 vs GNU R 106/116 (return/promise holes like magrittr's
plus Cairo svg devices), jsonlite 1542/9 vs 1555/6 (url() above).

## 2026-11-02: bzfile connections (libbz2); cli installs

- `bzfile()` was a hard error ("robigo: bzip2 compressed file
  connections are not implemented yet"), and `gzfile()`/`file()` in
  read mode — R's `connections.c:do_gzfile` dispatches on the file's
  magic (`comp_type_from_file`, with the PR#18768 block-size and
  pi-magic checks) and opens a bzfile connection for BZh data, which is
  how tools reads a bzip2-compressed sysdata during
  `R CMD INSTALL` — so **cli could not install**.
- Port: `ConnKind::Gz` gains a codec (gzip | bzip2); read decompresses
  the whole file at open (bzip2 crate, MultiBzDecoder — mirroring the
  existing whole-file gzip model), write compresses the buffer at
  close, warnings copied from `connections.c:bzfile_open`
  ("cannot open bzip2-ed file ...", "file '...' appears not to be
  compressed by bzip2"). `bzfile()` and magic-retyped `gzfile()`
  produce class "bzfile" connections, as in R; `file()` keeps class
  "file". xz/zstd stay unimplemented errors.
- Measured: `tests/r/io/bzfile-roundtrip.R` (the #215 gap tracker) now
  passes and is marked; new marked
  `tests/r/io/gzfile-bzip2-dispatch.R` pins the retype
  (`summary(gzfile(<bz2>))$class == "bzfile"`); `saveRDS(compress =
  "bzip2")` round trips; conformance 310+/310 with the two scripts in
  all three tiers (compiled_matches_interpreted green); lint clean.
- **cli 3.6.6 installs from source now** (was: install failure): its
  suite under the shim gives 1962 passed / 115 failed where GNU R 4.5.0
  gives 610/204 — the shim itself cannot express cli's snapshot and
  progress expectations, so GNU R fails many more; the remaining work
  is reconciling the failure lists (89 difference), tracked in the
  ledger, not yet in packages.tsv.
  - Sol round 1 findings fixed: a valid empty bzip2 stream
    (`bzfile(p, "wb"); close`) failed to reopen (success was keyed on
    non-empty output) — decoding now keys the warning on failure, not
    length; and `bzfile(p, "w", compression = 0)` panicked at close —
    it now warns "initializing bzip2 compression for file '...' failed"
    and fails the open, as bzfile_open's BZ_CONFIG_ERROR does. One
    documented model nuance stays: on non-bzip2 data the open succeeds
    and reads are empty in both, but robigo's warning fires at open
    (whole-file decode) where R's fires at first read (bzfile_read).
  - Sol round 2 (S3): trailing non-bzip2 bytes after a valid stream
    discarded the decoded prefix — the decoder now keeps it and warns
    "file '...' has trailing content that appears not to be compressed
    by bzip2" (bzfile_read's message), where data that never was bzip2
    still warns "appears not to be compressed" and reads empty.
    `bzfile-roundtrip.R` pins it. Found while testing: the
    `memCompress`/`memDecompress` .Internal pair is missing — tracked
    by unmarked `tests/r/io/memdecompress.R`.
  - Sol round 3 (S4): every decoder error warned, but R warns only for
    BZ_DATA_ERROR_MAGIC — a file truncated mid-header (a bare "BZh9")
    or mid-stream, or a block-magic mismatch after a valid "BZh" header
    (BZ_DATA_ERROR), is silent and keeps the bytes read. The bzip2
    crate calls libbz2, so its error kinds are bzlib's codes; the port
    now keys the two warnings on DataMagic alone and pins the
    truncated, mid-block-header and bad-block-magic cases in
    `bzfile-roundtrip.R` (GNU R-recorded, byte-identical). The strict
    PR#18768 magic stays where R applies it: the gzfile()/file()
    dispatch (comp_type_from_memory).

## 2026-11-02: memCompress/memDecompress

- The `.Internal` pair was missing entirely (tracked by #215's
  `memdecompress.R`, found while testing cli's bzip2 sysdata path).
  Ported `connections.c:do_memCompress`/`do_memDecompress`: the zlib
  format at compress()'s default level for "gzip" (flate2), bzip2
  through libbz2 at block size 9, `type = "unknown"` detection per
  `comp_type_from_memory` including the zlib prefix `78 9c` and the
  "unknown compression, assuming none" warning, and R's switch default
  (an unknown type returns the input).
- xz and zstd are not linked: both directions raise explicit
  "robigo: ... not implemented yet" errors; tracked by unmarked
  `tests/r/io/memcompress-xz.R`.
- `memdecompress.R` passes and is marked (GNU R-recorded round trips,
  `unknown` dispatch, the assuming-none warning). Conformance green in
  all three tiers; lint clean.
  - Sol round 1: memDecompress now stops after the first stream, as
    zlib/libdeflate and BZ2_bzBuffToBuffDecompress do (concatenated
    members and trailing bytes return only the first member), and
    "unknown" detection raises R's lzop error ("this is a
    lzop-compressed file which this build of R does not support") —
    all pinned in `memdecompress.R` (GNU R-recorded, byte-identical).
  - Sol round 2 (S4): the failure statuses are now R's verbatim — the
    gzip/zlib path reports libdeflate's (this R build has libdeflate;
    any data error is status 1), the bzip2 path reports bzlib's codes
    (magic -5, data -4, end-of-stream -7), all pinned in
    `memdecompress.R` (GNU R-recorded, byte-identical).

## 2026-11-02: C-assembled promises bind as runtime promises

- rlang's `r_env_bind_delayed` (magrittr's pipe masks bind `.` with it)
  assembles a PROMSXP with `Rf_allocSExp` + `SET_PRVALUE/SET_PRCODE/
  SET_PRENV` and defines it. robigo's bridge stored C-env bindings as
  opaque `wrap_core` proxies, so R-level reads of `.` saw an unforced
  promise object instead of forcing it: "object of type 'promise' is
  not subsettable", "invalid 'envir' argument of type 'promise'",
  vctrs' "Can't splice an object of type <promise>".
- Fix: `from_sexp` converts an assembled PROMSXP to a runtime `Promise`
  (mirroring the CLOSXP arm: CAR=PRVALUE, CDR=PRCODE, TAG=PRENV;
  opaque when the code expression is unusable), and `defineVar` no
  longer c-owns promise nodes — the proxy layer keeps them live for C
  (`to_sexp` of a runtime promise is a proxy whose PRVALUE/PRCODE/
  PRENV read and write through).
- Measured: magrittr 2.0.5 shim 48 passed / 5 failed (GNU R 53/0; was
  12/13 with the leak, halt before #205) — the 5 left are pipe
  visibility (`withVisible` through the C pipe), a separate gap. Every
  promise-leak repro (subset, math, `%$%`/with, splice, compound)
  matches GNU R byte-for-byte; rlang's shim no longer crashes (suite
  still running after 25 min, vctrs-class slowness). Conformance green
  in all three tiers; capi corpus 25/25 + the new `cpromise` case
  (C-assembled promise pinned, ratcheted).
  - Sol round 1 fixed: the conversion is memoized by node
    (CPROMISE_MEMO, cleared on gc as an implementation cache), so all
    bindings of one C promise share one runtime promise and forcing
    happens once (GNU R's shared-PROMSXP semantics), and
    SET_PRVALUE/SET_PRCODE/SET_PRENV through the original node update
    the memoized promise; NULL code is a promise of NULL and an
    already-set PRVALUE is honored whatever PRCODE is. The extended
    `cpromise` case pins shared bindings with an observable side
    effect, the retained-node write, NULL code and a pre-forced value;
    capi corpus 26/26, conformance green in all three tiers.
  - Sol round 2 fixed, promise-node identity now both ways: the memo is
    node → weak reference plus a `STORED` entry (which preserves the
    node while anything references the promise and releases both in
    `sweep_stored`), so the association survives gc() instead of being
    cleared; the cons setters (SETCAR/SETCDR/SET_TAG, which the SET_PR*
    macros expand to) sync a memoized promise node behind a PROMSXP
    type gate; and the runtime writes a forced C promise's node through
    a new NativeBridge::promise_forced hook, gated by a `from_c` flag on
    the promise (out of line, free for ordinary promises), so
    PRVALUE/PRENV reads through the original node stay in step. The
    `cpromise` case pins all of it (shared forcing, SETCAR write,
    PRVALUE after an R-side force, gc() then a C write); capi 26/26,
    conformance green in all three tiers, lint clean.

## 2026-11-03: url() connections (libcurl)

- `url()` was a hard error ("robigo: url() connections are not
  implemented yet"); jsonlite's test-network-Github.R (7 of its 9
  remaining failures) needs it.
- Port: `file://` descriptions delegate to the file connection (class
  "file", as GNU R), http/https/ftp(s) build class "url-libcurl"
  connections, fetched into memory at open through a new
  NativeBridge::fetch_url — libcurl's easy interface in robigo-capi
  (system libcurl, the stable ABI option numbers), following redirects
  and failing on HTTP errors; failures warn "cannot open URL '...',
  reason '<curl message>'" and the open fails, like connections.c.
  Reads serve from the buffer (whole-resource model, like the gz
  port); text/binary comes from the open mode; writes error. The
  `headers` argument is accepted and ignored for now (noted below).
- Measured: creation/class/isOpen/close, `readBin(url(..., "rb"))`,
  `readLines(url("https://example.com/"))` (576 chars, GNU R-identical)
  and the file:// delegation are byte-identical with GNU R 4.5.0 —
  including GNU R's own error for readBin on a default (text) url
  connection. `tests/r/io/url-connection.R` (the #215 tracker) passes
  and is marked, extended with the file:// case (no network in the
  suite; the CRAN gate exercises the fetch). jsonlite's shim: 1543
  passed / 8 failed (was 1542/9; the network tests now run). capi
  corpus 26/26; conformance green in all three tiers; lint clean.
- Open: the `headers` argument is ignored (CURLOPT_HTTPHEADER), and
  `method = "internal"`/`"wininet"` are not distinguished.
  - Sol round 1 (six shoulds) fixed: text/binary now derives from the
    CURRENT mode in url_open too (open(u, "rb") after a lazy create);
    write modes print Curl_open's "can only open URLs for reading"
    (REprintf, no newline) and fail with the generic error;
    CURLOPT_ACCEPT_ENCODING is gone (R's backend does not decompress,
    gzip bytes stay gzip); the `timeout` option is honored as
    curlCommon's connect and transfer timeouts (ms); failures print
    download_report_url_error's messages verbatim — "cannot open URL
    '<effective>': HTTP status was '404 Not Found'" from the ported
    http/ftp_errstr tables, the timeout variant, or the generic
    "URL '...': status was '<curl msg>'" — and the open fails with
    "cannot open the connection to '<url>'". A transfer that ends short
    keeps the bytes received (CURL_PARTIAL_FILE) with the status
    warning, so partial reads succeed as with the streaming backend;
    fully streaming Curl_read is still future work. All six cases are
    byte-identical with GNU R 4.5.0; capi 26/26.
- Rebased on main (2026-11-06): main's `builtins/internet.rs` already
  dlopens libcurl for `download.file`, so the fetch moved there
  (`internet::url_fetch`) and the robigo-capi `#[link(name = "curl")]`
  bridge is gone. Linking libcurl into the binary had cost every
  program its loading (CI's perf gate: +0.7% to +7% instructions on all
  74 benchmarks); with dlopen on first use, 10 of the flagged benchmarks
  measure within ±0.012% of main. The fetch is Curl_open's: curlCommon
  (R's user agent, `timeout`, redirects, CA bundle), FAILONERROR,
  TCP_KEEPALIVE, the `headers` (CURLOPT_HTTPHEADER, no longer ignored),
  fetchData's multi loop to the end of the transfer and
  download_report_url_error's warnings verbatim (the old bridge also set
  the transfer timeout through option 157 instead of
  CURLOPT_TIMEOUT_MS, 155). do_url follows connections.c's checks:
  "URL scheme unsupported by this method" for any other scheme or a
  path, the `wininet`/`internal` method errors, `blocking`, and the
  object class "url" (the connection class stays "url-libcurl").
  Network cases (readLines/readBin over https, a 404, an unresolvable
  host, request headers) are identical with GNU R 4.5.0 apart from the
  verbose trace GNU R prints when `internet.info` is unset (as under
  R_DEFAULT_PACKAGES=NULL, main's behaviour too).
- Open: the open waits for the whole transfer, so a stream that stays
  open (GNU R returns from Curl_open once data arrives and reads as it
  comes) blocks until EOF or the timeout; and a transfer that fails
  after data arrived fails the open (the warnings, then "cannot open
  the connection to '...'"), where GNU R serves those bytes and then
  fails the read with "cannot read from connection". Both need
  Curl_read's streaming.
- Review of fecf11ac: a failing curl_multi_perform or curl_multi_wait
  ends the transfer as a failure (GNU R keeps calling perform, with an
  interrupt check robigo's loop has no equivalent of, so a perform that
  never progresses would spin); a NUL in the URL or a header is an
  error; url() takes its slot through NextConnection's gc-and-retry.
