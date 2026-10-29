# Ledger: startup-perf (time to run a script)

Metric: callgrind instructions (wall time and peak RSS as secondary,
recorded in the robigo-oracle:4.5.0 container) for robigo vs `Rscript`,
per scenario. Machine notes: this box runs ~7 agents; wall numbers are
medians under load (load avg 27-31 during the 2026-10-28 baseline) and
are indicative only; instructions are the gate.

## 2026-10-29: rebase onto e3909b9; VM guards vs lazy base; memchr

- Rebased onto main e3909b9 (main still boots base from the vendored
  sources; core's PR #56 is the same lazy-base idea, unmerged).
  Ported #56's extra forcing site (the VM's `seq.default` test) onto
  `Interp::base_var`.
- **VM guards failed for every base closure under lazy base.**
  `Interp::resolves_to_base` compared the function `find_fun_cached`
  found with base's raw frame binding; with base from base.rdb the
  binding is a lazy-load promise, and `find_fun_cached` returns `None`
  for promises, so every guard on a base *closure* name (`seq`,
  `numeric`, ...) failed: the guarded op fell back to a full
  interpreter call and `for (i in seq(length = n))` loops iterated
  materialized vectors with boxed indices.
  `rbench/real/convolution_slow` was +387% (430.9M -> 2,100.6M
  instructions); `ROBIGO_VM_TRACE=1` showed `guard failed for seq` at
  every loop entry. Fix: `resolves_to_base` now looks the function up
  with `find_fun` (which forces promises, as R's findFun does) and
  compares against `base_var`'s forced value. convolution_slow:
  2,100.6M -> 202.5M (main's source path: 276.0M). This is also the
  likely cause of PR #56's lazy being *slower* with default packages
  (107 ms vs 95 ms): package loading thrashes these guards.
- **`native::is_robigo_build` scanned whole .so files byte-by-byte**
  (`windows().any()`, not vectorized) on every base-package dyn.load:
  18.1M instructions (6% of default-packages startup) on main. Switched
  to `memchr::memmem::find` (memchr is already in the tree via regex);
  `-e 1` with default packages 288.4M -> 270.4M. Report to the core
  track: the check runs per dyn.load and reads megabytes.
- Result vs Rscript 4.5.0-3 in the container (instructions; GNU R is
  the second number): `-e 1` NULL **53.5M/88.2M (0.61x)**; `-e 1` with
  default packages **270.9M/275.9M (0.98x)** (main 295.7M);
  `library(stats); lm` NULL **256.4M/262.7M**; with default packages
  **274.5M/279.1M**; `nrow(mtcars)` **273.0M/277.8M**. Every scenario
  is now faster than GNU R.
- Gates: conformance all three tiers; fastr slices (serialize,
  connections, functions, simple values, library/seq) 5956/5978,
  0 regressions; gnur-examples 615/1149, 0 regressions (77 passes new
  from main); perf 74 faster / 0 slower / 0 broken vs main e3909b9,
  geomean 0.964.

## 2026-10-28: zlib-rs and handoff audit

- Kept the `flate2` switch from `rust_backend` to `zlib-rs`. The former
  zeroes its roughly 250 KB inflate state for every lazy-load fetch.
  zlib-rs allocates the window lazily, like GNU R's C zlib.
- Inherited measurements: NULL startup 59.7M -> 53.6M instructions;
  default startup 264.0M -> 247.8M. Re-running the saved pre-rebase binary
  in `startup-perf-box` reproduced 53.505M and 247.824M. GNU R measured
  88.103M and 275.860M, giving ratios 0.607 and 0.898.
- Re-ran the saved performance comparison:
  `scripts/perf.py compare .robigo/perf/main-instr.json .robigo/perf/lazy2-instr.json`.
  All 78 benchmarks improved, none exceeded the 0.5% regression limit,
  none broke. These files predate the rebase and are local artifacts.
- The inherited measurement helper labelled process CPU time as wall
  time. The baseline's parenthesized milliseconds below are CPU time,
  **not wall time**. `scripts/startup-perf.py` now uses `perf_counter`
  and verifies stdout and stderr against GNU R before reporting results.
- Rebased onto main `49e0ee5`. Fixed the missing parser arena in the new
  baseloader path, which caused the previous CI build failure.
- Added R's `lazyLoadDBflush` internal, required to invalidate the new
  database cache when unloading namespaces. The focused conformance test
  overwrites a database and verifies cached reads, flushing, replacement
  reads, and repeated flushing. Recorded with Debian GNU R 4.5.0-3.
- Supporting shared paths: `crates/robigo-runtime/Cargo.toml`, Cargo.lock,
  `scripts/startup-perf.py`, and `tests/r/serialize/lazy-cache.*`.
- Saved optimized NULL-startup profile `/tmp/l2.cg` totals 53.650M.
  Inclusive base bootstrap is 22.888M, startup/profile loading 28.189M,
  and parsing 1.317M. Decompression is 8.717M, nested inside bootstrap
  and profile loading. These overlapping costs must not be summed.
  Default packages add about 194M in the saved binary, including namespace
  evaluation, database deserialization and LZMA.
- Container builds must use a private `CARGO_TARGET_DIR=$HOME/.cache/robigo-startup-target`
  (never a shared one: the host target has stale parser artifacts, and
  `~/.cache/robigo-trixie-target` belongs to perf-vm).

## 2026-10-28: load base from GNU R's own lazy-load database (baseloader)

- why: robigo evaluated base's R sources (~1.5 MB) at every startup:
  parse ~93M + eval ~48M instructions; GNU R instead runs the 5.4 KB
  `library/base/R/base` (baseloader.R), which binds every base symbol
  as a lazy promise over the pre-built `base.rdb` (values serialized at
  R install time) and materializes them on first use (`makeLazy` +
  `lazyLoadDBfetch`). `-e 1` was 159.9M instructions vs Rscript's 88.2M
  (1.81x).
- what:
  1. `bootstrap.rs`: evaluate `$R_HOME/library/base/R/base` in the base
     namespace when it exists (the exact file GNU R runs at startup),
     instead of the vendored `rlib/` sources; `ROBIGO_BASE_FROM_SRC=1`
     forces the old path, which remains the fallback when R_HOME has no
     installed base. rlib is unchanged.
  2. Lazy bindings are promises, so Rust code that reads base internals
     must force them like GNU R's findVar does. Added
     `Interp::base_var` (interp.rs, shared) and used it at the sites
     that read base directly: `.ArgsEnv`/`.GenericArgsEnv`
     (builtins/funs.rs do_args, print.rs print_special — R:
     builtin.c:do_args forces the PROMSXP), `.TAOCP1997init`
     (builtins/random.rs RNG_Init_R_KT — RNG.c uses findVar1),
     `.__S3MethodsTable__.` (dispatch.rs), and the closure-generic
     fallback of NextMethod (builtins/lang.rs). Without these, `args(c)`
     printed NULL, printing a primitive lost its `function (x)` header,
     Knuth-TAOCP seeding failed.
  3. `builtins/serialize.rs`: cache the whole database file after the
     first read (R: serialize.c readRawFromFile; 100 files, 10 MiB
     each) instead of open+seek+read per fetch.
- result (container, R 4.5.0-3):
  - `robigo -e 1` R_DEFAULT_PACKAGES=NULL: **159.9M -> 59.7M**
    instructions; Rscript 88.2M => **0.68x GNU R** (was 1.81x).
  - `-e 1` with default packages: 295.3M -> 264.0M; Rscript 275.9M =>
    0.96x (was 1.07x).
  - `library(stats); lm` lookup: 297.0M -> ~264M ≈ Rscript's 279.1M.
  - perf suite: 73 benchmarks faster, 0 slower, 0 broken; geomean
    robigo/robigo-main 0.958 (nbody was +0.50% before the file cache,
    the gate's threshold).
  - correctness: conformance suite all three tiers pass; gnur-examples
    522/1153 = exactly the source path's count (the 5 regressions
    there — Sys.info, Sys.which, find.package, round.POSIXt,
    detectCores — fail identically with ROBIGO_BASE_FROM_SRC=1; machine
    drift vs the recordings, pre-existing on main). fastr slices
    (serialize/unserialize, connections, RNG, functions/, environments,
    library/attach/search): 1064/1106, 0 regressions, 3 new passes;
    print/args/delayedAssign slice 794/823, 4 new passes (args/primitive
    printing fixed by the forcing fixes).
- kept. ROBIGO_BASE_FROM_SRC stays as an escape hatch and for installs
  without an R_HOME.

## Baseline 2026-10-28 (main fa86c67, container)

instructions robigo/Rscript (CPU ms median, previously mislabelled wall; load 27-31; RSS MB):

| scenario | pkgs | robigo | Rscript | ratio |
|---|---|---|---|---|
| `-e 1` | NULL | 159.9M (68) | 88.2M (68) | 1.81 |
| `-e 1` | default | 295.3M (141) | 275.9M (104) | 1.07 |
| `library(stats); lm` | NULL | 279.5M (117) | 262.7M (104) | 1.06 |
| `nrow(mtcars)` | default | 295.6M (101) | 277.8M (177) | 1.06 |

robigo `-e 1` NULL RSS 37.8 MB vs Rscript 20.2 MB; with default
packages 41.9 vs 39.2. Bootstrap breakdown (inclusive callgrind):
parse 93M, eval 48M, system Rprofile 17.7M, process+interp init 2.4M
(`--no-base -e 1`).

## Open items (ranked)

1. **S3 dispatch memsets during package loading** (dispatch.rs/subset.rs,
   conformance-lang/conformance-vectors): `library(stats)` spends 14.4M
   instructions (5.7%) in memset under `usemethod` -> `dispatch_with_temp_context`
   -> `dispatch_or_eval_sp` (~48 KB zeroed per dispatch, 2452 dispatches;
   `/tmp/ls.cg` in the container has the profile). Visible in every
   default-packages scenario.
2. **Regex construction at startup** (grep.rs, conformance-strings):
   `for (i in seq(length = n))`-adjacent startup work builds ~12 distinct
   patterns as ~31 `MetaRegex` objects (TreProg makes up to 4 per pattern
   for leftmost-longest): ~5M instructions of `-e 1`, 10% of it. GNU R's
   TRE build is ~10x cheaper per pattern.
3. **Startup snapshot without R_HOME (G6)**: robigo now needs GNU R's
   `library/base` to exist (it already needed R_HOME for utils/stats).
   With base loading from the lazy-load DB, a snapshot is "replay the rdx
   index + mmap the rdb". Carry-over for robigo-core (G3): the
   lazy-binding design (promises in the frame, forced on lookup, value
   cached in the promise) is exactly GNU R's.
4. **Per-fetch decompression**: each fetch inflates its own chunk; GNU R
   pays the same. An uncompressed rdb image built once (with
   invalidation on rdb change) would cut ~40M from default-packages
   startup.
5. **RSS with default packages**: 46 MB vs GNU R's 39 (`-e 1` NULL is
   21.8 vs 20.3 after this PR); the gap is the cached databases plus
   materialized base.
