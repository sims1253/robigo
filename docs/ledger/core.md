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

## 2026-10-29: stats natives in CI: ported routines keep their Rust port

- CI perf on #70 (stats natives everywhere): `robigo/lm` +99%. Profile:
  the bridge copying string vectors (a model frame's 20,000 row names)
  into CHARSXPs and back for C_modelframe/C_modelmatrix/C_Cdqrls and
  the glm family routines (mk_char, rstr_of, UTF-8 checks).
- Kept: stats routines with a Rust port (and the distribution internals
  the runtime serves) keep the port while stats.so is loaded
  (`native::PORTS_FIRST`); stats.so serves the rest. Until G3 shares
  arguments. lm 758.8M -> 764.5M with natives (+5.7M, the accepted
  startup step of loading stats.so's LAPACK/BLAS/gfortran), knucleotide
  and dataframe +5.3M the same. gnur-examples stats and the fastr
  model/distribution slice: 0 regressions.
- `robigo/fannkuch` +17% in that CI run did not reproduce: the same
  artifact measures 1,309M (main 1,305.9M) directly, under perf.py, and
  in the oracle container, output correct; three repeats agree within
  0.1M. Watching the re-run.

## 2026-10-29: CI builds the base-package natives (pending-workflow.patch applied)

- With the gates enforced (#58), the build job builds GNU R's stats
  natives from source (cached R sources), ships them in the `robigo`
  artifact (`dist/natives`), and the corpora and perf jobs run with
  `ROBIGO_NATIVES`; setup installs libcurl4-openssl-dev (R's configure)
  and zlib1g-dev. The package list grows with the PRs that make each
  package run: splines tools parallel (#57), grDevices graphics grid
  (#69). `docs/ci/pending-workflow.patch` is gone; those PRs edit ci.yml.

## 2026-10-28: compact sequences (GNU R's compact_intseq / compact_realseq)

- `:`, `seq_len`, `seq_along` (also the VM's lazy ranges when they
  materialize) and compact row names make GNU R's compact sequences;
  `as.numeric` of one gives a compact real sequence (altclasses.c's
  Coerce method), and unserialize restores them.
- Serialization (version 3) writes them in GNU R's ALTREP form: class
  info `list3(compact_intseq, base, 13)`, state `c(n, n1, inc)`, no
  attributes. A conformance test compares byte lengths and bytes:
  `1:1e6` is 133 bytes as in GNU R. Copies (`c(1:3)`, `rev`, `+ 0L`), a
  modified copy (`y[2] <- 9L`) and vectors with attributes serialize
  regularly, as GNU R's are immutable and duplicate on modify.
- C API: GNU R's two ALTREP classes (`crates/robigo-capi/src/compact.rs`,
  a port of altclasses.c's methods: Length, Elt, Dataptr with expansion,
  Dataptr_or_null, Get_region, Is_sorted, No_NA, Sum, Duplicate,
  Coerce, Serialized_state, Unserialize). The bridge hands a compact
  sequence to C as one (no copy of the data) and takes an unexpanded
  one back as compact. capi corpus 15 -> 17/20 (`vectors`, `serialize`).
- Representation, measured: a `compact: bool` in `Vector<T>` made every
  vector 8 bytes bigger, and the suite showed vecarith +0.76%; CI's perf
  job fails above 0.5%. Compact sequences are therefore registered by
  allocation in a thread-local map of `Weak`s. The `Weak` pins the
  address, so no other vector can be mistaken for a registered one.
  Modification goes through `Rc::make_mut`, which moves a weakly
  referenced value to a new allocation, so a registered vector is
  unmodified and the check is O(1). Dead entries are pruned when the map
  has doubled. Suite vs main on this machine: 0 slower, binarytrees
  -0.69%.
- Ratchets: conformance all pass (+1 script); fastr shards
  7626/7499/7661/7537 (+12 vs before, the only regressions are main's
  known TestSimpleValues/operators set); gnur-examples, gnur-tests,
  r-intro, gnur-demos unchanged.
- Not done:
  - **deferred strings** (`as.character(<int/real>)` is an ALTREP
    `deferred_string` in GNU R; `serialize(row.names(df))` is 216 bytes
    there, 58 in robigo) need a back-reference from the string vector to
    its source;
  - **package ALTREP classes through serialization** (mori-style
    Serialized_state/Unserialize hooks) need the runtime to hold foreign
    ALTREP objects instead of materializing them: G3 territory.
- **G6 (methods ledger, 2026-10-28) — answer from core.** Agreed:
  methods stays a Rust port while the bridge copies arguments (measured
  here: the copies alone make vectorised stats natives ~7% slower, and
  `R_set_slot`'s in-place modification cannot work through a copy).
  `dyn.load` serves a base package from GNU R's sources only when
  `.robigo/natives/<pkg>` holds a build (#53), and `build-natives.sh`
  builds only the packages it is given; methods is never built by
  default. docs/core.md §13 lists methods.so's 22 core entry points for
  the later comparison under G3. Ask further questions as "G6 asks"
  entries here.
## 2026-10-28: base-package natives from GNU R's sources by default (stats)

Lead's decision: base packages' native code comes from GNU R's sources.

- `scripts/build-natives.sh [PKG...]` builds on the host, no docker. It
  configures the R 4.5.0 sources once (downloaded and SHA-256 checked
  when missing), then compiles the package's `Makefile.in` source lists
  with GNU R's Makeconf flags (`SAFE_FFLAGS` for `portsrc.f`) and
  `-DTESTING_WRITE_BARRIER`. stats: 41 s including configure.
- `dyn.load` of a base package loads `$ROBIGO_NATIVES/<pkg>/<pkg>.so`
  (else `natives/` beside the binary's directory) when it carries the
  marker, under GNU R's own path. A first attempt that made robigo's
  home the visible `R_HOME` changed 17 gnur-examples outputs that print
  `R.home()`/`searchpaths()`; it was dropped for this.
- Fixes found by the comparison:
  - nmath's MATHLIB_WARNINGs (robigo-numerics queues them) are now R
    warnings for C callers too, and nmath's `R_Calloc` failures are
    errors. 8 stats cases had lost warnings or the rwilcox error. A
    `pending()` flag keeps the per-element check cheap.
  - `log(...)` expands dots (a regression from #37's log dispatch fix;
    also pushed to #37).
  - `dsvdc_` (LINPACK SVD) ported, with Debian reference BLAS 3.12's
    `drotg` (the scaled LAPACK 3.10 algorithm) and `drot`. Fortran's
    NaN branch semantics are kept. loess (which uses it) prints GNU R's
    output, and GNU R's stats.so now has no unresolved imports.
- Measured, native stats.so vs Rust ports (same binary,
  `ROBIGO_NATIVES` pointed at an empty directory for the ports):

  | corpus | native | ports |
  |---|---|---|
  | fastr (4 shards) | 30,576 | 30,311 |
  | gnur-examples | 571 | 545 |
  | gnur-tests | 22 | 20 |
  | capi, r-intro, gnur-demos | equal | equal |

  0 regressions caused by the natives. Every remaining "regression" is
  shared by both modes: main's CI reports the same TestSimpleValues and
  operators cases, and the 17 gnur-examples ones are local-environment
  paths and dates.
- Speed: lm/glm suite benchmark: equal instruction counts (177.15M).
  `pnorm`/`qnorm`/`dbinom` over 2e6 values, 20 times: native 10.3-10.6
  s, ports 9.6-9.8 s, GNU R 9.9 s. The difference is the bridge's
  vector copies (G3 removes them); the ports stay until then.
- CI (no `workflow` scope for agents): `docs/ci/pending-workflow.patch`
  adds `libcurl4-openssl-dev` (R's configure needs it; r-base-dev
  already brings gfortran, BLAS and LAPACK). The build job builds the
  natives before the tests and ships `dist/natives` with the binary;
  the corpora and perf jobs set `ROBIGO_NATIVES`. The gctorture step
  from #28 is already in ci.yml.
- Local pitfall: after branch switches the shared cargo target
  sometimes reused a build-script output without the appl library or
  `ROBIGO_EXPORTS_HASH` (link errors: `vmmin` undefined, or the env var
  missing). `touch crates/robigo-capi/build.rs` (or robigo-cli's)
  fixes it; robigo-cli's build.rs now also reruns on its own change.
  CI builds from scratch.
- For G6: docs/core.md §13 lists the 22 non-API entry points methods.so
  needs from the core (the dispatch hooks, the slot functions,
  R_execMethod, ...) and the missing runtime S4 bit.

## 2026-10-28: src/appl in libR; stats from source vs the Rust ports

- GNU R's src/appl C files (`optim.c` + `lbfgsb.c`, `uncmin.c`,
  `integrate.c`) are vendored unmodified in `crates/robigo-capi/appl`
  and compiled into libR by build.rs. A `#[used]` table keeps them in
  the executable, which exports them. The two shims there: `Defn.h`
  (only `_()`) and an empty `config.h`.
- LINPACK with the Fortran ABI in `src/linpack.rs`:
  - dqrdc2, dqrsl, dqrls and the dqrutl drivers wrap the runtime's
    existing ports;
  - dqrdc, dpofa, dtrsl, dpbfa and dpbsl are new line-by-line ports;
  - reference BLAS level 1 (any stride) serves the appl objects and is
    not exported (`NOT_EXPORTED` in gen-stubs.py), so packages keep
    their own BLAS.
- capi corpus 14 -> 15/20 (`fortran`, to 12-15 significant digits);
  census 92.6% -> 93.5% weighted (569/831 symbols).
- GNU R's stats.so built from source (build-base-native.sh) now has only
  `dsvdc_` left among robigo's stubs. In a robigo home that uses it
  (`ROBIGO_NATIVES`), smooth.spline, optim (BFGS, Nelder-Mead,
  L-BFGS-B), integrate, nlm and lm.fit print exactly GNU R's output.
  - **gnur-examples `^stats/`: 141/275 with GNU R's stats.so, 113/275
    with robigo's Rust ports, 0 regressions.**
  - Full fastr corpus with the native stats.so: 2 regressions, both a
    traceback line (`Calls: ... -> complete.cases -> .External`). GNU R
    gives a foreign call a builtin context only in the AST interpreter;
    package code is byte-compiled at installation, and bytecode calls
    builtins without a context. The bridge now pushes its context only
    when the call is evaluated outside namespace code; both cases pass,
    and the capi corpus is unchanged.
  - Recommendation (measured): serve stats from GNU R's own sources, not
    the Rust ports. That needs the base-package sources and gfortran in
    the build (CI): a maintainer decision about the CI image. Until
    then, robigo's default home keeps the Rust ports.

## 2026-10-28: the gate — install.packages() from source inside robigo

**fastmap and digest install from CRAN sources inside robigo, and their
own test files pass** (shim `crates/robigo-capi/gate/shim.R`, GNU R
4.5.0 measured with the same shim in the oracle image):

| package | robigo | GNU R | the difference |
|---|---|---|---|
| fastmap 1.2.0 | 302 | 341 | three blocks stop on latin1 strings (the runtime's strings are UTF-8 only) |
| digest 0.6.39 | 325 | 325 | none |

Before, robigo ran these tests against packages GNU R had built: 291
and 241 expectations passed.

How it works (docs/core.md §8): `scripts/robigo-home.sh` builds robigo's
`R_HOME` (`.robigo/home`, found next to the binary when `R_HOME` is
unset). It contains:

- GNU R's own front-end scripts (`bin/R`, `Rcmd`, `INSTALL`, `SHLIB`,
  ...), pointed at the home, with robigo as `bin/exec/R`;
- `include/` with the vendored 4.5.0 public headers
  (`crates/robigo-capi/include`, unchanged: the public API is functions
  only);
- `lib/libR.so`, which carries only the soname: the API symbols bind
  to the executable;
- GNU R's `etc/`, with a `Renviron.site` limited to the home's own site
  library;
- GNU R's `share/` and `doc/`;
- the base packages, with `tools.so` built from source for robigo.

`robigo CMD X` runs `bin/R CMD X` in that home.

Steps measured on the way (each a GNU R port, with a conformance or
corpus test):

- `R CMD SHLIB`/`INSTALL` reach `tools:::.install_packages` in robigo:
  `-e`/`-f` arguments are unescaped as R's binary does (`~+~`).
- readDCF (dcf.c), utils' hashtab (unique.c, the `.External` entry
  points), getVarsFromFrame and lazyLoadDBinsertValue. The serializer
  gained the persistent-name hook, NAMESPACESXP/PACKAGESXP, and puts
  external pointers in the reference table. Its reader always did,
  which desynchronised every later reference.
- GNU R's `tools.so` built from source (`build-base-native.sh`, which
  now links a `robigo_base_native` marker that `dyn.load` checks before
  using a base package's shared object instead of Rust ports). For it:
  - `getConnection` hands C a real `struct Rconn`: GNU R's public
    layout, offsets tested, methods calling the runtime;
  - `begincontext`/`endcontext` run `cend` when an error unwinds past
    the context (RCNTXT offsets measured with GNU R's Defn.h);
  - the parser globals have GNU R's sizes: the 32-byte data stubs were
    smaller than the 256-byte `R_ParseContext` gramRd writes;
  - `.External2` is called as `fun(call, op, args, env)`;
  - RC_fopen, R_opendir/readdir/closedir, Rf_strchr, Rf_mbrtowc and
    R_NewHashedEnv.
- Runtime semantics the install path needs:
  - calls keep a tag on the function position (`names(e[-c(1, 2)])`,
    which `parseNamespaceFile` uses for `.registration`), and `as.call`
    drops it;
  - `!` of a zero-length non-logical gives `logical(0)`;
  - comparisons coerce a raw operand to the other operand's type (it
    used to panic);
  - `c()`/`unlist` keep NA names;
  - file connections keep GNU R's separate read and write positions:
    `parse_Rd` writes to `file()` and reads back from the start, and
    `seek` now follows fileconn exactly;
  - `Sys.setFileTime` and `extSoftVersion` (robigo reports "" for
    libraries it does not use).
- `download.file(method = "libcurl")`: libcurl.c's `curlDownload`
  ported over the system libcurl, which is `dlopen`ed (no link-time
  dependency). It has GNU R's multi-URL loop, progress output and
  messages; the libcurl constants were checked against curl.h.
- An external pointer that exists only in the runtime (restored by
  `unserialize`) shows C code a NULL address, as in GNU R. fastmap
  checks for NULL to rebuild its map; before the fix it dereferenced
  `R_NilValue` and crashed.

The ratchets still pass: conformance 186/186 including the 5 new
scripts, and the fastr corpus slices gained 79 newly passing cases with
0 regressions.

Open:

- robigo still starts without the default packages (methods does not
  load), so the gate runs `utils::install.packages`;
- latin1 strings (fastmap's encoding tests);
- a cpp11 package and an Rcpp package;
- the capi corpus built against robigo's headers.

## 2026-10-28: GC at scale (council item 9)

`crates/robigo-capi/bench/scale.sh` builds a 10M-string STRSXP of unique
CHARSXPs in C (`mkChar`), then churns 50M `allocVector(REALSXP, 4)` over
that live heap; GNU R 4.5.0 (oracle image) and robigo run the same
`scale.R`. robigo's collection time comes from `robigo_gc_seconds()`
(new export; GNU R's from `gc.time()`).

| | GNU R build | robigo build | GNU R churn | robigo churn |
|---|---|---|---|---|
| wall | 5.93 s | 4.13 s | 3.04 s | 1.29 s |
| in GC | 2.28 s | 0.11 s | 2.04 s | 0.67 s |

The first run showed robigo spending 2.3 s in GC while building. A
phase timer found the cause in the CHARSXP-table prune: on every
collection it read the mark bit of each table entry (a cache miss per
string, 1.3 s at 10M) even when nothing had died. It also rebuilt the
table at twice the needed size. Fixes:

- the collector counts marked cached CHARSXPs as it traces them, and
  immortal ones as they are made; when the two add up to the table's
  size, the prune is skipped (with a debug assertion that every entry is
  marked);
- the prune sizes the rebuilt table within insert's 3/4 load factor
  (2^24 slots for 10M strings, not 2^25: 128 MiB less).

Core-only check (a Rust test building the same 10M strings, 4
collections): GC total 2.34 s -> 0.26 s.

Memory is the open finding: peak RSS is 915 MB for GNU R and 2071 MB for
robigo.

- In C after the build: 739 vs 915 MB. The core heap alone holds 761 MB
  (560 MB of objects plus the 128 MiB table); the rest is the runtime.
- After returning to R: 746 vs 1305 MB. The bridge copies the STRSXP
  into the runtime's own string vector, so the dual object model holds
  the strings twice until the next collection.
- Churn: robigo collects after allocating 1.5x the live heap, so the
  dead core copy and up to ~900 MB of churn sit in fresh blocks before
  the first collection frees them. GNU R's generational collector keeps
  only 20-40% headroom.

The fixes for these are already on the list: the runtime moving onto
core objects (no copy) and sticky-mark generational collections (§11:
minor collections make a smaller growth factor affordable). A lower
growth factor alone would trade peak memory for proportionally more
marking of the full live heap.

The robigo binary now relinks when `exports.list` changes (cargo did
not track the file named in the link argument; build.rs feeds its hash
into the build).

## 2026-10-28: Preserve/Release as GNU R's list; the src/appl plan

- R_PreserveObject used a HashMap<Sexp, count>; GNU R conses onto a
  precious list (O(1) for LIFO churn, O(n) worst case). Now a stack
  released from the top, with the same multiset semantics: bench
  Preserve/Release 1.82 -> 0.97 of GNU R.
- stats.so built from source still imports 20 stubbed symbols, all GNU
  R's src/appl (vmmin, nmmin, cgmin, samin, lbfgsb in appl/optim.c +
  lbfgsb.c; optif9/fdhess in uncmin.c; Rdqags/Rdqagi in integrate.c;
  LINPACK dqrdc/dqrdc2/dqrls/dqrsl/dqrcf/dsvdc/dpbfa/dpbsl in Fortran).
  appl/optim.c includes Defn.h and lbfgsb/uncmin call BLAS and LINPACK,
  so they belong to the base-natives build (an `appl` object built from
  the R sources like stats.so and loaded before the base natives), not
  to robigo's cargo build (CI has no R sources or gfortran). That needs
  the executable to stop exporting stubs for those names (the
  executable comes first in symbol lookup and would shadow them):
  next step, together with switching a base package from its Rust
  ports to its natives.

## 2026-10-28: GNU R's stats natives built from source run on robigo (council item 1)

- GNU R's Defn.h defines USE_RINTERNALS unless TESTING_WRITE_BARRIER is
  set; with `-DTESTING_WRITE_BARRIER` the 39 internal-layout files
  compile to calls of exported functions. So base-package natives need
  no robigo Defn.h for now: `crates/robigo-capi/natives/build-base-native.sh
  stats OUT` configures the R 4.5.0 sources once (17 s) and builds
  stats.so in a build image (robigo-oracle + gfortran, `robigo-build`).
- stats.so: 69 objects (Trunmed.c is #included by another file), 299
  undefined symbols; robigo's executable exports all but IS_CACHED and
  the BLAS/LAPACK routines (ddot_, dnrm2_, dpotrf_, dswap_, dsyrk_,
  dtrmm_, idamax_; system libblas/liblapack). 30 of them are still stubs:
  GNU R's src/appl (LINPACK dqrdc_/dqrdc2_/dqrls_/dqrsl_/dqrcf_/dsvdc_/
  dpbfa_/dpbsl_, optimizers vmmin/nmmin/cgmin/lbfgsb/samin/optif9/
  fdhess, Rdqags/Rdqagi), Fortran helpers (rchkusr_, rwarn_, rexit_,
  intpr_, d1mach_, interv_), Rf_deparse1(line), Rf_allocLang,
  Rf_rPsort, UNIMPLEMENTED_TYPE, signrank_free, wilcox_free.
- smoke test: dyn.load of the built stats.so in robigo, `.Call` of the
  registered `cor` and `cov` routines: 0.99947644097685506 and 6.12, as
  GNU R. Next: build src/appl's C sources into robigo, then load these
  natives instead of the Rust ports, package by package, measured on the
  corpora.

## 2026-10-28: the caught-error benchmark found two unwind bugs

- the C bench's caught-error routine cost ~230k instructions per error.
  Profiled: (1) 8k per Rf_error zeroing an 8 KiB message buffer (now a
  per-thread scratch buffer); (2) every error after the first fell to
  the *default* handler (message formatting and traceback): the runtime
  pops its handler stack while signalling, as GNU R does, but GNU R
  restores the handler and restart stacks when a jump reaches a context
  (R_restore_globals) and libR.so's context record did not include
  them. The host now saves/restores them per context (two Rc clones;
  R_UnwindProtect's normal path 0.86 -> 1.52 of GNU R, to optimize).
- (3) comparing the routine's R-visible result with GNU R showed
  R_UnwindProtect returning the continuation after a jump; GNU R 4.5's
  continues the unwind itself after cleanfun (context.c: `if (jump)
  R_ContinueUnwind(cont)`). Fixed; cleanfun is now called outside any
  robigo context (cpp11 longjmps out of it). `tryCatch(.Call(<loop of
  R_UnwindProtect over error()>), error = ...)` now gives the outer
  handler's value, as GNU R ("outer handler"), instead of NULL.
- result: caught error 230k -> 45k instructions; capi 14/20; digest and
  fastmap test shims unchanged (241, 291 expectations).

## 2026-10-28: isolates, first step (lead A.2)

- libR.so's internal state (heap, host table, R_alloc stack) moves from
  a process-wide `static mut STATE` to a thread-local isolate pointer;
  robigo-core's context stack was already thread-local. The exported
  data symbols (R_NilValue, R_GlobalEnv, R_PPStack, R_Visible, ...) are
  still process-wide variables: making them per-isolate is the header
  work (macros over the isolate record), with the immutable singletons
  in a shared immortal segment (docs/core.md §10).
- cost: allocVector small 1.01 -> 1.03 of GNU R (a TLS load per API
  call); R_UnwindProtect regressed to 1.17 until the context save and
  restore fetched the isolate once (now 0.86). capi corpus 14/20.

## 2026-10-28: fork-safe marking (lead A.1) and the abandoned-bump-space bug

- measured first (`collecting_in_a_forked_child_does_not_copy_the_heap`,
  a ~229 MB live heap of 512-byte vectors, fork, one collection in the
  child, the child's Private_Dirty growth from /proc/self/smaps_rollup):
  header mark bits -> **226 MB copied**.
- change: marks live in a per-block mark bitmap next to the start bitmap
  (the block header grows 576 -> 1,072 bytes; 4,029 granules per block;
  medium classes recomputed for 64,464 data bytes) and, for large
  objects, an epoch word in the chunk's header (no clearing). Marking and
  the sweep read object headers (IMMORTAL, HOST) but never write live
  objects; the MARK_A/MARK_B header colors are retired. -> **17 MB
  copied** (the block headers and the collector's own memory).
- cost (C API bench, robigo/GNU R instructions): allocVector small 0.98
  -> 1.01, medium 0.35 -> 0.41, lang3 1.26 -> 1.29 (collections compute
  bitmap positions and clear bitmaps). Kept: fork safety is a lead
  requirement and the cost is small.
- found by ROBIGO_GCTORTURE (a test that collected on every allocation
  reserved 626 MB for 79 MB of data): the sweep reset every class's
  allocation cursor but re-listed only blocks that gained dead slots, so
  a block's remaining bump space was stranded until all its objects
  died — up to one partial block per class per collection. Now blocks
  with free slots or bump space are re-listed. A new test fails without
  the fix (2,016 blocks for 2,000 live 8 KiB vectors) and passes with it.
  The same sweep is in `experiments/memory` (memory track): its
  long-running workloads strand partial blocks too.

## 2026-10-28: medium objects share blocks (council item 13)

- why: every object above 2 KiB took a dedicated 64 KiB-aligned chunk:
  a 1000-double vector (8 KiB) reserved 64 KiB, and chunks were kept in
  a sorted Vec with O(n) insert/remove.
- change: 31 medium size classes, the largest 16-byte multiple of the
  block's 64,960 data bytes divided by k = 31 .. 1 (2,080 .. 64,960
  bytes), so k objects tile a block; only objects above 64,960 bytes get
  chunks. class_index binary-searches the medium table.
- result: 10,000 live 1000-double vectors reserve 79 MB (before: one
  64 KiB chunk each, ~625 MiB); `allocVector(REALSXP, 1000)` 0.92 ->
  0.35 of GNU R's instructions (bench). Core tests pass, also under
  gctorture; the request-pair overrun test covers every class that holds
  three objects per block (sampled sizes for medium classes).

## 2026-10-28: C API microbenchmarks vs GNU R (council item 9) and the first optimizations

- harness: `crates/robigo-capi/bench/` (capibench.c + bench.py):
  instructions inside each routine under callgrind --toggle-collect,
  GNU R 4.5.0 (oracle image) vs robigo; 100k operations per routine.
- baseline -> after (robigo/GNU R):
  | routine | baseline | after |
  |---|---:|---:|
  | allocVector(REALSXP, 4) | 1.25 | 0.98 |
  | allocVector(REALSXP, 1000) | 0.91 | 0.92 |
  | lang3 | 1.58 | 1.26 |
  | PROTECT/UNPROTECT | 1.00 | 1.00 |
  | mkChar hit | 0.69 | 0.59 |
  | mkChar miss | 1.02 | 0.82 |
  | install | 0.85 | 0.84 |
  | INTEGER loop | 0.35 | 0.35 |
  | SET_STRING_ELT | 0.86 | 1.14 |
  | Preserve/Release | 1.99 | 1.78 |
  | eval (trivial closure) | 0.67 | 0.67 |
  | R_UnwindProtect | 1.04 | 1.04 |
  (the caught-error row is not comparable: callgrind does not attribute
  GNU R's cost after a longjmp leaves the routine; robigo pays ~230k
  instructions per caught error, to investigate.)
- kept: (1) `Heap::alloc` inlines the fast path (free list, bump) and
  keeps the block switch cold; per-allocation statistics counters
  removed (bytes allocated accumulate at collection). (2) small atomic
  vectors skip the checked size arithmetic and the fill. (3) the store
  barrier (council item 8, first step): every API store goes through
  `fix_refcnt`, which hands immortal targets to `Core::write_barrier`;
  only immortals that received a store are traced as roots (instead of
  every non-CHARSXP immortal). (4) the CHARSXP cache is a flat
  open-addressing table of `Sexp`s with the hash in each CHARSXP's
  header (council item 10), rebuilt from survivors when pruned (the
  HashMap<u32, Vec<Sexp>> pruned bucket by bucket through malloc/free).
- open: SET_STRING_ELT regressed 0.86 -> 1.14 with the barrier check in
  fix_refcnt (to profile: likely lost inlining); Preserve/Release uses a
  HashMap (GNU R conses onto a list: O(1) for LIFO churn); lang3 still
  1.26 (refcount increments on construction, PROTECT in the list
  constructors).

## 2026-10-28: conservative scan limited to C frames (deterministic finalizers); connections

- the extptr case passed, then failed on the next run: its finalizer
  only runs if no stale copy of the external pointer lies in robigo's
  own interpreter frames, which the conservative scan read up to the
  thread's stack top. robigo-core gains `set_scan_top`; the bridge
  bounds the scan at its outermost entry (C frames below are scanned,
  the runtime's frames above hold core objects only through precise
  roots), and R-level `gc()` with no C code active scans no stack at
  all (the council's "scan conservatively only C segments"). extptr now
  passes 3/3 runs; collections outside native calls no longer read the
  interpreter's stack.
- R_GetConnection/R_WriteConnection/R_ReadConnection/Rconn_fgetc over
  the runtime's connection table (not in the census; the serialize case
  uses them). The case still differs: GNU R serializes `1:3` as a compact
  ALTREP sequence (183 bytes vs robigo's 93).
- capi corpus: 13/20 (alloc, attributes, ccallable, coerce, console,
  errors, eval, extptr, math, onintr, pairlist, registration, unwind).
  The earlier entry's "14" was an expectation, not a measurement.

## 2026-10-28: the bridge — capi corpus 0/20 -> 13/20; fastmap and digest run on robigo

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
  RNG). Census coverage by implemented exports: 90.7%
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
