# Ledger: core (robigo-core, robigo-capi; goalpost G2)

Track: `docs/workstreams.md` § core; design: `docs/core.md`. Metrics: (a)
`capi` corpus cases passing, (b) weighted census coverage by implemented
`libR.so` exports, (c) the gate packages `fastmap`, `digest`.

Local setup (this machine): system R is 4.6.1, so the capi corpus's
`R CMD SHLIB` build step runs GNU R 4.5.0 from the `robigo-oracle:4.5.0`
image through wrapper scripts (`~/.cache/robigo-core/bin/{R,Rscript}`:
`docker run --rm -u $(id -u):$(id -g) -v $HOME:$HOME -w $PWD
robigo-oracle:4.5.0 R "$@"`) put first on `PATH`; robigo itself runs on
the host with `R_HOME=~/.local/opt/R-4.5.0`.

## 2026-10-28: the bridge — capi corpus 0/20 -> 14/20; fastmap and digest run on robigo

- linkage (council item 15): the robigo executable links robigo-capi and
  exports exactly the API with `ld --dynamic-list` (exports.list,
  generated): 986 symbols; `-rdynamic` alone exported ~1,800 unrelated
  ones for a trivial Rust binary (measured). A soname-only stub libR.so
  satisfies objects linked with GNU R's `-lR`. The coordinator's shape
  (runtime inside libR.so, thin executable) needs ci.yml to ship libR.so
  next to dist/robigo; this token cannot push workflow changes, so the
  executable shape is used for now; the crate graph is the same.
- perf gate vs main @ 49c071a (instructions, all benchmarks): 0 faster,
  0 slower. (A first comparison showed fib/apply/binarytrees 1.4-2.8%
  slower; cause: the baseline was newer main with perf-vm's closure
  inlining; after rebasing the difference is gone. Linking capi without
  the runtime changes measured identical.)
- capi corpus: 0 -> 5 (bridge) -> 9 (errors signalled at raise time in a
  builtin context, onintr, sort.c, duplicated/identical) -> 12
  (environments, eval, parse, print, gc() finalizers) -> 13 (Rmath,
  RNG) -> 14 (serialize). Census coverage by implemented exports: 90.7%
  weighted.
- found by real packages: arguments must carry the caller's sharing
  (REFCNT from the runtime value's reference count): digest's AES
  duplicates its argument only when referenced, so a fresh copy with
  REFCNT 0 was encrypted in place and the write-back changed the
  caller's variable. R_Serialize callbacks see GNU R's write pattern
  (header fields as separate pieces, integer fields 4 bytes): digest's
  spookyhash skips a byte count into the stream.
- the gate, measured with GNU R 4.5.0-built source installs (public API
  only, so source-compatible) and the packages' own test files under a
  tiny expectation shim (`crates/robigo-capi/gate/shim.R`; tinytest
  itself needs `file.info`, testthat is too heavy for now):
  - digest: robigo 241 expectations pass, GNU R 322; every difference is
    a whole file aborting on a runtime gap (`file.info` in test_blake3
    and test_misc, `eSoftVersion` in test_sha1); no expectation that ran
    fails.
  - fastmap: robigo 291, GNU R 341; the 5 failing blocks hit runtime
    gaps (latin1 strings in iconv, builtins/character.rs; unserializing
    closures with empty formals, builtins/lang.rs).
- remaining capi cases: vectors (1:6 must arrive as a compact ALTREP
  sequence), altrep (the ALTREP class framework), strings (a CHARSXP
  returned to R prints `<CHARSXP: ...>`: needs print.rs support),
  fortran (GNU R's src/appl LINPACK/optimizers: build from source, §8),
  objects (needs methods, G6), envir (lazy-loaded base bindings are
  promises in GNU R: startup track).

## 2026-10-28: council item (c) — contexts and unwind

- C trampolines in the shim (`robigo_try_call65`, `_sexp_ptr`,
  `_void_ptr`, `_void_ptr_int`) call package code directly: no Rust frame
  lies between a context's setjmp and the C code that may longjmp. Used
  for `.Call`/`.External`/`.C`, `R_init_<pkg>`, `R_unload_<pkg>`, C
  finalizers, and the bodies and cleanups of R_ToplevelExec,
  R_ExecWithCleanup and R_UnwindProtect.
- context record (`errors::Ctx`): R_PPStackTop, R_PPStackSize (red
  zone), vmax, gc_inhibit, R_Visible, R_interrupts_suspended.
- R_UnwindProtect pops its context before cleanfun; a cleanfun that jumps
  wins; the continuation's car is a HOST proxy owning the flow token, so a
  never-continued continuation discards its condition when collected.
- tests (`crates/robigo-capi/tests/capi.rs`, a stub host): allocation and
  attributes across 20k allocations, Rprintf, Rf_error caught by
  R_ToplevelExec three times with a stable protect depth and context
  depth 0, R_UnwindProtect jump/no-jump with cleanup, the abandoned
  continuation discarding its condition, Rf_warning jumping under
  warn=2, allocVector(-1)'s message, a 3-argument .Call through the
  65-argument trampoline. Pass, and pass under ROBIGO_GCTORTURE=1.
- not done: the Result refactor for outward jumps (frames are plain by
  construction today); catch_unwind belongs to the host callbacks
  (bridge); C++ exceptions stay fatal as in GNU R (reason in core.md §5).

## 2026-10-28: lead's requirements — parallelism/isolates, GC lessons

- recorded in docs/core.md §10/§11. Settled now, before the layout and
  exports harden: (A.1) marks move to per-block side bitmaps so a forked
  child's collections do not dirty copy-on-write pages (header mark bits
  retired with the barrier work); (A.2) isolates: all mutable state in a
  per-thread isolate record, per-isolate exported data as header macros,
  immutable singletons and symbols in a shared immortal segment, a
  frozen read-only segment for zero-copy sharing, packages opt in to
  isolates. Parity packages (nanonext, mirai, mori, future) join G4; the
  parallel benchmarks and the MMTk build-vs-adopt comparison join (d).

## 2026-10-28: council items (a) and (b) — collector correctness, gctorture

- release callbacks of dead host proxies no longer run inside
  `collect(&mut self)` (aliasing); they and the finalizers run at safe
  points (`safe_point()`: after each native call, `R_gc`, `gc()`),
  never inside an allocation. Finalizable weak references stay on the
  collector's queue (rooted) until popped, and are protected while their
  finalizer runs.
- conservative scan: a word on a granule boundary also resolves `w - 1`
  (one-past-the-end pointers, Boehm's rule), and `resolve` now checks the
  slot end: before, any word pointing into free space after an object
  resolved to it (false retention). The new test fails without the
  `w - 1` rule (verified by disabling it) — its first version passed
  without the fix because of exactly that missing end check.
- TRACKREFS on construction: no refcount on the car of EXTPTRSXP,
  BUILTINSXP, SPECIALSXP (an address/offset, not an object).
- thread binding: the core records its thread's stack bounds; a
  collection from another thread aborts with a diagnostic; libR.so sets
  R_CStackStart/Dir/Limit (95%, as setup_Rmainloop) and ports
  R_CheckStack/R_CheckStack2 ("C stack usage  %ld is too close to the
  limit").
- `ROBIGO_GCTORTURE=1`: collect on every allocation, poison freed slots.
  Core tests: 14/14 pass normally (0.02 s) and under torture (4.9 s).
  The CI step (`ROBIGO_GCTORTURE=1 cargo test --release -p robigo-core
  -p robigo-capi` in the build job) could not be pushed: the agent's
  token lacks the `workflow` scope; it is in the PR description for a
  maintainer. The rooting rule is in
  docs/core.md §3.

## 2026-10-28: the design council's decisions

- The council (Astra, Opus 5.5, Fable 5.1) reviewed docs/core.md @
  e3eeac8; synthesis and reviews copied to `docs/core-council.md`, the
  25 decisions tracked in docs/core.md §12. Order of work set by the
  coordinator: (a) correctness of what exists, (b) gctorture + rooting
  rule, (c) contexts and unwind, (d) measurements vs GNU R, (e) store
  barrier, node/env shape, CHARSXP table, encodings, fallible allocation,
  (f) linkage (runtime inside libR.so). I take (f) before (d)/(e): it
  decides how the runtime-side bridge is written, and writing that bridge
  against dlsym first would be thrown away.

## 2026-10-28: layout decided by the lead — source compatibility; council fixes

- why: the design council (Astra) verified that GNU R's own natives are
  compiled with `USE_RINTERNALS` (Defn.h:49-51), so loading GNU R's
  binaries unmodified would pin GNU R's `SEXPREC`. Audit: 39 C files of
  the base packages include `Defn.h` (stats cov/deriv/model/optim/
  optimize/splines, graphics base/graphics/par/plot/plot3d, 13 grDevices
  files, methods, utils, tools, parallel, tcltk); macro uses counted over
  their sources: CDR 579, REAL 578, INTEGER 482, CHAR 210, CADR 198,
  LENGTH 153, TYPEOF 121, CADDR 94, LOGICAL 48, XLENGTH 21, TAG 16, RAW
  15, COMPLEX 10, PRINTNAME 8, plus single uses of CLOENV, FORMALS, BODY,
  ATTRIB, ENCLOS, SYMVALUE, INTERNAL, DDVAL, ENC_KNOWN, IS_CACHED,
  ENSURE_NAMEDMAX, (SET_)RTRACE, SET_OBJECT, SET_TYPEOF, EXTPTR_TAG/PROT.
  The installed stats.so imports 11 of 14 core accessors as functions
  (from its other files). CRAN packages cannot use USE_RINTERNALS (R >=
  4.3) and import accessors as functions.
- tried: switching robigo-core to GNU R's SEXPREC (sxpinfo bit layout,
  32-byte header, payload at +48, collector bits in mark/gcgen/gccls).
  It built and its tests passed. **Reverted** after the lead's decision:
  robigo is source-compatible (packages and base-package natives are
  built from source against robigo's headers), so the layout is ours and
  the compact one wins on memory (nodes 40 bytes = 48-byte class instead
  of 56 = 64; vector header 32 instead of 48), with TYPEOF a byte load.
- kept, from the council's review:
  - allocator: `alloc_small` bumped by the request's granules while
    free-list slots of a class are reused for any request in the class
    (classes above 256 bytes span four 16-byte sizes): a 273-byte request
    in a slot carved for 257 bytes overran its neighbour. Now every slot
    is carved at the class size; a test over every request-size pair per
    class fails without the fix. The same allocator is in
    `experiments/memory` (memory track): its bake-off numbers predate the
    fix and should be re-measured there.
  - immortal objects' mutable edges (symbol values, attributes on
    constants) are traced every collection (new test).
  - weak references follow memory.c CheckFinalizers: every dead-key
    reference is ready (finalizer or not); all keep key/value/finalizer
    alive for the round.
  - `R_NewPreciousMSet` & co. use memory.c's `CONS(store, npreserved)`
    with the initial size as tag (it protects members only while itself
    protected), not a private list.
  - TRACKREFS is GNU R's `!spare` (default on) instead of a positive bit.
- open: the unwind discipline (no Drop/guards across calls that can
  jump) holds by construction today; the council's stronger form
  (Result-returning inner functions, jumps only in thin wrappers or C
  trampolines) is planned before G3.

## 2026-10-28: baseline

- capi corpus: 0/20 pass (stdout 0, stderr 0, status 2 match): `dyn.load`
  registers no code, `.Call` fails with "native routine ... is not
  implemented yet".
- census coverage by robigo exports: 0 / 5339 (no `libR.so`).
