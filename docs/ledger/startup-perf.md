# Ledger: startup-perf (time to run a script)

Metric: callgrind instructions (wall time and peak RSS as secondary,
recorded in the robigo-oracle:4.5.0 container) for robigo vs `Rscript`,
per scenario. Machine notes: this box runs ~7 agents; wall numbers are
medians under load (load avg 27-31 during the 2026-10-28 baseline) and
are indicative only; instructions are the gate.

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

instructions robigo/Rscript (wall ms median, load 27-31; RSS MB):

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

1. **Startup snapshot without R_HOME (G6)**: robigo now needs GNU R's
   `library/base` to exist (it already needed R_HOME for utils/stats).
   A robigo-proprietary snapshot of the bootstrapped base would remove
   that dependency; with base now loading from the lazy-load DB, the
   snapshot is "replay the rdx index + mmap the rdb", much smaller than
   snapshotting the evaluated environment. Carry-over for robigo-core
   (G3): the lazy-binding design (promises in the frame, forced on
   lookup, value cached in the promise) is exactly GNU R's and works
   with copy-on-modify refcounts unchanged.
2. **Per-fetch decompression**: each base fetch inflates its own chunk
   (1177 chunks, 45 B-31 KB); GNU R pays the same, but a
   decompress-once-into-an-indexed-arena (or robigo building its own
   uncompressed rdb at install time) would cut another ~40M from
   default-packages startup. Needs a cache-invalidation story if the
   rdb changes.
3. **wall/RSS**: with lazy base, `-e 1` NULL RSS should drop toward
   R's 20 MB (was 37.8 MB measured on the source path); re-measure and
   record; wall numbers need an idle machine to mean anything.
