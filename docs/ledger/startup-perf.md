# Ledger: startup-perf (time to run a script)

Metric: callgrind instructions (wall time and peak RSS as secondary,
recorded in the robigo-oracle:4.5.0 container) for robigo vs `Rscript`,
per scenario. Machine notes: this box runs ~7 agents; wall numbers are
medians under load and indicative only; instructions are the gate.
Earlier history (the lazy-base work, PR #36) is in its branch until it
lands; the natives work is PR #78 below.

## 2026-10-29: what is left of PR #36 on top of core's #56

- Core's #56 (merged as d709287) loads base from base.rdb with a reused
  inflater, bulk unserialize reads and its own guard fix, absorbing most
  of PR #36. Re-measured both remaining pieces on main 6ddb7dc:
  - the rdb **file cache** (R's readRawFromFile) is ~neutral now
    (e1-def 272.68M -> 272.51M; stats-lm-null +0.4M, within noise):
    per-fetch open+seek+read was ~1-2k instructions and #56 removed the
    big per-fetch costs it used to sit next to. Dropped from this PR;
    the lazy-cache test and flush stay in #36's branch for the record.
  - the **flate2 `zlib-rs` backend** still wins: miniz_oxide's
    decompress_vec costs more per byte than zlib-rs even with one
    reset Decompress. Kept (this PR).
- `-e 1` instructions (container, R 4.5.0-3; GNU R = Rscript):

  | scenario | main | this PR | GNU R | ratio |
  |---|---|---|---|---|
  | NULL packages | 50.93M | **48.42M** | 88.19M | 0.55x |
  | default packages | 272.64M | **266.59M** | 275.93M | 0.966x |

- gates: conformance 3 tiers; fastr serialize/connections/gzfile slices
  451/458, 0 regressions (2 new); perf suite vs main's binary: 73
  faster, 0 slower, 0 broken (profiler-shared watched specifically
  after the memchr layout lesson — clean).

## 2026-10-29: the cost of GNU R's stats natives, removed

- why: core's PR #70 builds `stats` from the R 4.5.0 sources in CI
  (`scripts/build-natives.sh`, `ROBIGO_NATIVES`). Loading the resulting
  stats.so dlopens liblapack.so.3, libblas.so.3 and libgfortran.so.5:
  resolving their relocations cost **+3.8M instructions** on every run
  with `utils,stats` (285.1M vs 281.3M; #70 measured +4.3M on its
  build). The lead accepted it as one-time; this wins it back.
- where the 3.8M went (callgrind diff, with-natives vs without):
  symbol lookups 2.4M (`do_lookup_x`+`_dl_lookup_symbol_x`+`check_match`),
  marker scan 0.40M (`is_robigo_build` reading the 2.1 MB .so),
  memset 0.47M, `_dl_relocate_object` 0.09M.
- what (scripts/build-natives.sh — core's file, changes called out in
  the PR):
  1. **Static LAPACK/BLAS** (`-Wl,-Bstatic` around LAPACK_LIBS/BLAS_LIBS):
     only the objects stats references are pulled in (+26 KB; stats
     needs dpotrf and four reference-BLAS kernels — the LINPACK routines
     resolve against robigo's exports). liblapack.so.3 and libblas.so.3
     are no longer mapped at startup: -1.6M.
  2. **R's xerbla** compiled into the .so (a copy of src/main/print.c's,
     which routes BLAS/LAPACK errors to `error()`). Without it the
     static LAPACK pulls its own xerbla, dragging libgfortran's I/O back
     in; with it, `--as-needed` drops libgfortran entirely (stats'
     objects reference no `_gfortran` symbol).
  3. **-Wl,-Bsymbolic-functions**: the package's ~700 PLT calls to its
     own global functions (compcases, deriv, ...) bind at link time
     instead of a loader lookup each: -0.35M.
- what (native/mod.rs — core's file, called out): `is_robigo_build` now
  reads the ELF's dynamic string table (~10 KB) instead of the whole
  file (2.1-2.5 MB): -0.4M with natives, and **-17.6M without** them —
  main's `windows().any()` scan of GNU R's stats.so costs that on every
  run, with or without natives.
- rejected: doing the string-table scan with the `memchr` crate
  (`memmem`). The dependency itself shifted the runtime crate's code
  layout and cost rbench/simple/profiler-shared +1.5% (6.081B vs main's
  5.992B; callgrind showed the extra instructions in env frame lookups
  and drops — no changed code path). Scanning the 10-60 KB table with
  `windows().any()` instead keeps the win (0.1M) with no new dependency;
  profiler-shared is back to 5.979B. Lesson for all tracks: a new crate
  in robigo-runtime can move hot-code layout by more than the change
  itself; measure the suite, not just the target.
- result (`-e 1`, R_DEFAULT_PACKAGES=utils,stats, container):
  | binary | instructions |
  |---|---|
  | main, no natives (the goal line) | 281,254,779 |
  | main, natives (dynamic libs) | 285,075,617 (+3.8M) |
  | this branch, no natives | 263,755,247 |
  | **this branch, natives** | **264,079,767 (+0.32M, 6.1% below the goal)** |
  The natives overhead is 92% gone; both modes now beat main by ~6%
  (263.4M / 263.9M without the memchr crate; see the rejected
  experiment below).
  For scale: GNU R itself pays 262.9M - 156.7M = 106.2M to go from
  `--default-packages=utils` to `utils,stats` (namespace code plus its
  own stats.so + liblapack/blas/gfortran load); robigo's own load cost
  (+0.32M) is an order of magnitude under GNU R's.
- correctness: qr/lm/chol battery byte-identical to Rscript with the
  static-linked natives and with the Rust ports (xerbla's error text
  matches); conformance 3 tiers; gnur-examples `^stats/` identical with
  and without natives (140/275, 0 regressions, 27 new from main); fastr
  library/attach + functions slices 0 regressions; perf robigo/ suite
  vs main's binary: 11 faster, 0 slower, 0 broken.
- note for core (owns these files): the ELF parse replaces the memmem
  scan PR #36 added (same function); xerbla is a candidate for a libR
  export (GNU R's libR exports it) so CRAN natives get R-faithful
  LAPACK errors too — the local copy covers base packages either way.

## 2026-10-29 (round 4): rebase onto 7ff8a140 (#79 in); re-measured

- Rebased #78 onto main 7ff8a140 (merge of this morning's wave + #79).
  Contrary to the handoff note, **#79 carried only the flate2/zlib-rs
  switch** (one commit: Cargo.toml/lock + this ledger) — the ELF marker
  scan and the build-natives.sh static-linking changes never landed on
  main, so the whole PR still applies. Real conflicts: this ledger
  (both branches created it) and `sources()` in build-natives.sh
  (#79's `@BUILD_AQUA_TRUE@` handling vs #78's LINKFLAGS — kept both).
  Also dropped the duplicated doc lines and two unneeded `mut`s the
  rebase left in `is_robigo_build`.
- Re-measured (robigo-oracle:4.5.0 container, R 4.5.0-3, binaries built
  in robigo-build:4.5.0 with the private target dir):

  `-e 1`, `R_DEFAULT_PACKAGES=utils,stats`:
  | binary | instructions | natives overhead |
  |---|---|---|
  | main, no natives | 254,199,385 | — |
  | main, +dynamic natives | 257,536,932 | +3.34M |
  | this PR, no natives | 235,109,769 | — |
  | **this PR, +static natives** | **235,649,764** | **+0.55M** |
  | Rscript | 262,809,555 | 0.897x |

  `-e 1`, default packages (incl. methods):
  | binary | instructions | natives overhead |
  |---|---|---|
  | main, no natives | 563,487,045 | — |
  | main, +dynamic natives | 567,419,471 | +3.93M |
  | this PR, no natives | 543,573,324 | — |
  | **this PR, +static natives** | **544,125,839** | **+0.55M** |
  | Rscript | 661,159,388 | 0.823x |

- Correctness after the rebase: qr/lm/chol battery byte-identical to
  Rscript with the static natives and with the Rust ports.
  `cargo test --release -p robigo-runtime`: green except
  `complex::c_library_oracle`, a last-bit sqrt mismatch against the
  recorded glibc oracle that also fails on main on this host (glibc
  2.44 vs the recording's; CI's trixie is the judge).
- Where default-packages startup goes now (callgrind, branch, 544M):
  zlib-rs inflate path ~71M (adler32 16.2M, tables 10.6M, inflate
  ~45M — the per-entry streams of the lazy-load DBs), serialize
  Reader ~45M (int 25.5M, read_item_flags0 20.0M, read_bc_lang 12.7M),
  vm 19.6M, mimalloc ~30M, memcpy 11.1M, drops 12M.

## Open items (ranked)

1. Same as PR #36's list (S3 dispatch memsets during package loading,
   regex construction at startup, G6 snapshot, per-fetch inflate).
2. The remaining +0.32M of the natives load is stats.so's own mmap,
   .bss zeroing (Fortran COMMON) and relocations — at parity with what
   any dlopen pays; not worth chasing.

## 2026-10-29 (later): rebase onto d709287 (core's #56 merged); perf-gate artifacts

- Rebased the natives work onto main d709287, which merged core's #56
  (lazy base from base.rdb, their own version of the guard fix, a reused
  inflater). Startup `-e 1` utils,stats on top of that main:
  240.0M without natives, 240.6M with them (the natives overhead is
  +0.5M there; the earlier afe56d9-based numbers are in the table above).
- The CI perf gate flags flexclust +10.2%, flexclust_no_s4 +9.3%,
  csv +2.3% as slower. They are baseline artifacts: with identical
  environment, this branch's binary and main d709287's binary are
  instruction-identical on flexclust's partial path (4,717.8M both, both
  dying at `setClass`) and its full path with methods attached
  (5,183.9M both). Main's recorded flexclust baseline is 4.718M-instruct
  sized — the `setClass`-error prefix — i.e. the baseline run died at
  `source()` and compares as "faster" than any complete run; the same
  comparison reports spectralnorm_math as FIXED (main's recorded run for
  it was wrong). flexclust/csv are also #70's two natives-sensitive
  benchmarks and the self-hosted runner shares its workspace across
  concurrent PR runs. Documented on PR #78; #72 (compare with the PR's
  own base commit) is the systemic fix.

## 2026-10-29 (round 4, PR #82): flat lazy-load cache (rdbflat)

- Where startup went after the natives work: every `lazyLoadDBfetch`
  inflates its own zlib stream (Huffman tables per stream, adler32 over
  the output) — ~71M instructions of `-e 1` with the default packages
  (zlib-rs functions under callgrind), ~25M of the utils,stats run.
  GNU R pays the same per fetch; this removes it for robigo.
- `rdbflat.rs`: a build-time cache of pre-inflated entries.
  `scripts/build-rdbflat.sh` records a battery of runs
  (`ROBIGO_RDBLOG`; the fetch builtin logs them) and
  `scripts/build-rdbflat.py` writes per-rdb flat files (format in the
  module docs); `ROBIGO_RDBFLAT=<dir>` enables it at runtime.
  `do_lazy_load_db_fetch` serves entries as zero-copy slices; missing
  or stale entries (rdb size/mtime guard, bounds-checked index) take
  the old path, so behaviour is byte-identical with and without the
  cache — verified with a battery exercising methods/stats reference
  environments (nested fetches through the envhook work: entries are
  borrowed from an `Rc`, not a held lock). Flat files load lazily per
  rdb: a NULL-packages run reads only base.rdb's.
- numbers (container, this branch's binary, cache built by the battery):

  | scenario | plain | +rdbflat | GNU R |
  |---|---|---|---|
  | `-e 1` NULL packages | 48,368,428 | **43,513,795** (−10.0%) | |
  | `-e 1` utils,stats | 254,744,724 | **225,315,906** (−11.6%) | 262,809,555 |
  | `-e 1` default packages | 564,855,495 | **487,920,614** (−13.6%) | 661,159,388 (0.738×) |

  RSS: default 54.7 → 60.7 MB (the flats stay resident; they are the
  same bytes the old path inflated transiently); NULL 21.8 → 24.0 MB.
- open: coverage is the recorded battery (fallback correct, unsaved);
  a complete builder enumerating the rdx keys (the runtime can
  readRDS), mmap instead of read, and wiring `ROBIGO_RDBFLAT` into
  robigo-home/CI (perf-gate runs equally for baseline and candidate)
  are the follow-ups. Not wired into CI here: the feature is opt-in
  and byte-identical, so ratchets and the gate are untouched either
  way.
