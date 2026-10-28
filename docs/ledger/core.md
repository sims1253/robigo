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
