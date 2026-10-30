# Ledger: startup-perf (time to run a script)

Metric: callgrind instructions (wall time and peak RSS as secondary,
recorded in the robigo-oracle:4.5.0 container) for robigo vs `Rscript`,
per scenario. Machine notes: this box runs ~7 agents; wall numbers are
medians under load and indicative only; instructions are the gate.
Earlier history (the lazy-base work, PR #36) is in its branch until it
lands; the natives work is PR #78 below.

## 2026-10-30 (round 9): rebased startup verification

- Release build passes on main 1b4670a plus #92/#99. A first attempt
  reused a shared parser artifact without parse_data; touching the
  syntax module's timestamp and rebuilding corrected the stale artifact.
  No parser source contents changed.
- Pinned Debian oracle integration: all 10,057 keys match, all 27
  stdout/stderr/status comparisons pass. The host Rscript launcher
  still invokes R 4.6.1 even with R_HOME; run oracle checks inside
  robigo-oracle:4.5.0. The host-built binary uses the mounted host
  loader and libraries at /host-lib (mount /usr/lib, not the Debian
  x86_64-linux-gnu subdirectory).
- Focused serialize/unserialize/connections: 451/458. The sole ratchet
  failure is the previously documented oversized-sequence case
  6260e145e42c04ad; the exact original df3d535 baseline artifact also
  fails it in the same 4 GB oracle container. No ratchet entries changed.
- Fresh library(stats), NULL packages, enumerated cache, natives off:
  173,087,503 instructions. Round 8 was 173,121,524; this is a fresh
  baseline after the rebase, not a claimed reader optimization.
  Profile self costs: VM run_with 7,929,388; Reader::int 5,081,622;
  read_item_flags0 4,442,153; read_bc_lang 3,140,109. Next optimize
  one reader allocation path and compare these exact settings.
- Seven gate tests, Python compilation, shell syntax and cargo fmt
  --check pass. Runtime crate tests are queued on the shared target.

## 2026-10-30 (round 9): remaining native bridge boundary

- Rechecked GNU R 4.5.0 src/main/names.c: La_rs has eval=11, arity=2,
  opcode 1 (and opcode 5 in the alternate HAVE_LAPACK configuration).
  src/main/lapack.c loads the module; src/modules/lapack/Lapack.c:La_rs
  is the object wrapper to port, not an eigenvalue algorithm to replace.
- Exact native symbol: dsyevr_. Binding: R_ext/Lapack.h:1328,
  F77_NAME(dsyevr), with arguments jobz/range/uplo, n/a/lda, vl/vu/il/iu,
  abstol/m/w/z/ldz/isuppz, work/lwork/iwork/liwork/info, followed by
  three Fortran character lengths. On this GNU Fortran ABI those are
  size_t values 1,1,1 (Rust usize). La_INT is the native integer ABI.
  Query both workspace sizes with lwork=liwork=-1 before the real call.
  The wrapper copies input because LAPACK overwrites it; JOBZ=N/V,
  RANGE=A, UPLO=L, ABSTOL=0. Preserve its errors and result list names.
- The stats archive pulls only its referenced LAPACK objects and does
  not provide dsyevr_ or the La_rs object wrapper. Next work crosses
  into native bindings/build ownership: build GNU R's LAPACK module
  against robigo's API, or expose this symbol and port that wrapper.
  Check eigenvalues/vectors, nonsquare input, zero dimensions and
  only.values=NA against the oracle. spectralnorm_math is still wrong,
  so exclude its instruction counts from speed claims.
- library(stats) remains at the round 8 measurement 173.122M, 0.667x
  GNU R. No additional startup speedup is claimed for the gate changes.
  Next profile the rebased reader, then measure a single reduction in
  Reader::int/read_item_flags0/read_bc_lang allocations before keeping it.

## 2026-10-30 (round 9): four-way exact-artifact measurement

One foreground robigo-oracle:4.5.0 container, R 4.5.0-3, valgrind 3.24.0,
same nbody_2 program/parameter 921, shared candidate natives/cache.
Executables copied into separate bin directories with no sibling cache.
Exact original artifacts: main df3d535 from run 40980827065 and candidate
d002511 from run 40982799591. Every output/status matches GNU R.

| executable | ROBIGO_RDBFLAT | instructions, run 1 | run 2 |
|---|---|---:|---:|
| base | unset | 872,645,331 | 872,605,219 |
| base | set | 836,290,008 | 836,307,843 |
| candidate | unset | 870,543,236 | 870,510,491 |
| candidate | set | 834,030,673 | 834,000,430 |

- Gate comparison, cache off: 872,605,219 -> 870,510,491 (-0.2401%).
- rdbflat diagnostic ONLY, excluded from gate: base saves 36,315,211;
  candidate saves 36,510,061 instructions. The base demonstrably takes
  the cache path. Candidate cache-off self costs include inflate_fast_help
  6,114,606, inflate_table 5,669,934, copy_match_runtime_dispatch 4,690,329,
  and adler32 4,667,797 instructions. The main VM run_with self count is
  identical with/without cache (30,751,320).
- The historical +22,540,288 remains unresolved. Neither cache-off nor
  cache-on like-for-like comparison reproduces it, and the complete
  cache effect is about 36.5M, not 22.54M. The old perf run discarded
  its profiles, so these data cannot prove which historical fetches
  missed their cache. Do not label the historical increase noise or a
  proven cache artefact. The new job retains every measurement/profile,
  including its four-way diagnostic, to identify a recurrence.
- Local raw profiles/results are in .robigo/r9, with the foreground
  reproduction harness four.py. Mount ~/.cache/robigo-bench at
  /root/.cache/robigo-bench in the oracle container so perf.py's external
  nbody source exists; no recordings or benchmark definitions changed.

## 2026-10-30 (round 9): feature-neutral performance gate

- Rebased #92 onto main 1b4670a. Stored counts can hide a regression
  when only the candidate uses a newly supported cache. Rechecking only
  flagged slowdowns does not detect that false pass.
- Compare every successful benchmark with both exact artifact executables,
  twice, with ROBIGO_RDBFLAT unset and executables copied into isolated
  directories without an automatically discoverable cache. Keep the 0.5%
  threshold, output checks, measurement errors, and original profiles.
- Run the four nbody_2 combinations in the same job as a separate cache
  diagnostic. Cache-on counts never enter the pass/fail comparison.
- Verification: 4 -> 7 gate tests pass. The new end-to-end adversarial
  test simulates a base that ignores the cache and a candidate that uses
  it: apparent 100 -> 80 becomes cache-off 100 -> 110 and fails at 10%.
  Failed oracle measurements also fail. cargo fmt --check and diff
  whitespace checks pass. No runtime source or ratchet changes.
## 2026-10-30 (round 8): confirm regressions with a paired gate

- GitHub-hosted diagnostic run 40985993806 succeeds (AMD EPYC 7763,
  Debian glibc 2.41-12+deb13u4, valgrind 3.24.0). Exact artifact pair:
  own resources base 836,703,649 / 836,707,540; candidate 834,454,992 /
  834,470,376. Shared cache/native directory: base 836,708,252 /
  836,713,676; candidate 834,422,960 / 834,465,977. Outputs and statuses
  match, and all 32 guards pass on each cache. Profiles and environment
  are in that run's perf-diagnostic artifact. This does not explain the
  original stable 859.6M candidate count; no claim of noise is made.
- Confirmed the local result through perf.py:run_one itself, including
  its temporary bench.R and working directory. Base 836,388,886 /
  836,394,996; candidate 833,959,576 / 833,911,532. The diagnostic
  script now uses the same temporary working-directory convention.
- The gate now downloads the baseline executable from the exact run
  whose stored metrics selected it. For cases flagged slower it runs
  baseline and candidate twice in the current job, sharing the cache,
  native directory and environment, and compares the minimums with
  the same 0.5% threshold. It checks both outputs/statuses against
  GNU R and retains the original metrics plus callgrind profiles.
  Stored baseline files remain unchanged. A failed measurement or
  mismatched output fails confirmation, rather than accepting a prefix.
- Four Python checks cover historical environment differences, actual
  slowdowns, wrong output and failed baseline runs. A real-artifact
  end-to-end confirmation of the logged nbody_2 failure gives paired
  base 836,441,083 / 836,373,747 and candidate 833,929,154 / 833,919,422,
  with identical GNU R output. The gate reports 0 slower, 0 broken.
- Applied this gate-only commit to #92 as 8966a92. Its runtime changes
  remain untouched; new CI is required before the coordinator merges.
  Original gate counts stay in this ledger as evidence, not as an
  alleged fixed runtime regression. Cache enumeration remains in #99.

## 2026-10-30 (round 8): #92's repeated nbody_2 failure is unresolved

- The stable CI increase must be investigated as a real regression, not
  called noise. Downloaded the exact robigo artifacts from main run
  40980827065 (df3d535) and PR run 40982799591 (d002511). The baseline
  perf-main artifact records 837,063,202 instructions. The recheck log
  prints the candidate's first and second counts, 859,603,490 and
  859,648,780, not a baseline remeasurement (perf.py:cmd_recheck). Thus
  the confirmed CI increase is 22,540,288 instructions, +2.69%.
- Exact artifacts in local robigo-oracle:4.5.0, with each artifact's
  own natives and flat cache, same utils,stats/locale/timezone and
  benchmark parameter 921: base 836,330,549 / 836,320,174; candidate
  833,981,641 / 833,967,367. Both produce identical energy output and
  exit 0. The CI direction is NOT reproduced locally. All 32 source
  size/mtime guards match in each downloaded cache, 6,793 entries each.
- Local self-profile delta is -2.35M, chiefly reader allocations and
  cloning, not a +22.54M hot-path cost. Disabling the candidate cache
  gives 870,527,390 instructions. Cache state matters, but this is not
  proof that CI lacked its cache. Per-lookup source stat validation is
  only done while loading a database, not on each successful lookup.
- Added a dispatch-only diagnostic to run these exact artifacts on the
  CI runner, with own and shared cache/native directories, two runs per
  pair. It saves callgrind profiles, cache guards, artifact hashes, and
  CPU/libc/valgrind/environment details. The existing gate and its
  threshold remain unchanged. Do not declare #92 cleared from the
  local result; inspect the CI profiles before fixing the cause.
- Rebased release binary passes the 10,057-key index comparison and
  all 27 transcript comparisons. Runtime unit tests: 42 pass; the same
  two host-glibc complex oracles fail. FastR serialize/connections:
  451/458 on both #92 and the rebased binary, with the identical known
  oversized-sequence serialization ratchet failure on both locally.

## 2026-10-30 (round 8): enumerate the real lazy-load indices

- Based on #92 head d002511, whose reader changes are still awaiting
  merge. Replace the recorded forcing battery with readRDS enumeration
  of every installed .rdx, including references and nested eagerKey /
  lazyKeys entries (R: lazyload.R:lazyLoadDBexec). Enumeration errors
  abort the builder instead of producing a partial recording.
- Pinned Debian oracle library: 5,894 keys in 16 databases from the old
  battery -> 10,057 keys in 30 databases from the indices. The added
  databases include help databases; every enumerated key is checked
  against the resulting flat index. No runtime fallback was removed.
- Integration verification: GNU R and robigo emit byte-identical key
  logs. All 27 stdout/stderr/status comparisons pass: NULL, utils+stats,
  default packages, each with AST/VM/JIT and full/absent/malformed
  caches. Out-of-bounds builder input fails without publishing a cache.
  CI now runs this check after building its cache.
- Builder rejects inconsistent compression flags, truncated/out-of-range
  entries, decoded-length mismatches and source metadata changes during
  its reads. Unknown compression remains an explicit build failure.
  Missing/stale/malformed runtime caches retain the original fetch path.
- Same #92 host-built binary, both cache versions, natives disabled,
  pinned oracle library, C.UTF-8/UTC, VM default. Callgrind instructions:

  | scenario | battery cache | enumerated cache | GNU R |
  |---|---:|---:|---:|
  | NULL, -e 1 | 38,090,360 | 38,220,237 | 87,667,059 |
  | NULL, library(stats) | 174,694,415 | 174,859,390 | 259,410,755 |
  | default, -e 1 | 391,057,625 | 391,198,488 | 660,611,163 |

  Full coverage costs 0.34%, 0.09%, and 0.04% respectively. Current
  library(stats) ratio is 0.674x; default-package startup is 0.592x.
  Host-built binaries require GLIBC_2.44, so these run in the oracle
  container via the mounted host loader/libraries. GNU R uses its
  container libraries. Both robigo sides have exactly the same loader,
  environment and package bytes. Do not compare to older container-built
  binary counts as a paired speedup.

- Re-profiled #92 with the complete cache. Self instructions remain
  VM run_with 7.92M, memcpy 5.12M, Reader::int 5.08M,
  read_item_flags0 4.44M, read_bc_lang 3.14M. No new reader optimization
  is claimed in this round. An int-inlining candidate was deferred
  before compilation while main advanced with #95; a useful next
  experiment needs a fresh paired baseline on that revision.
- Rebased cleanly onto d2ce510 (#95). Source changes in this round do
  not alter reader semantics; the release rebuild is queued behind
  other tracks' foreground builds. No build was killed.

La_rs follow-up: GNU R src/main/names.c registers La_rs with opcode 1,
internal/evaluated, arity 2. the LAPACK dispatcher calls src/modules/lapack/Lapack.c:La_rs calls dsyevr_ with
JOBZ=N/V, RANGE=A, UPLO=L, ABSTOL=0 and queries LWORK/LIWORK first.
Missing: the runtime DEFS registration and SEXP wrapper, plus a callable
native dsyevr_ binding. R_ext/Lapack.h already declares the function
with three trailing Fortran character-length arguments (size_t).
The stats native library only pulls the LAPACK archive members its
sources reference; it does not expose this libR LAPACK-module wrapper.
Next: build GNU R's LAPACK module against robigo's C API or add a
runtime wrapper over native reference-LAPACK dsyevr_ with both workspace
queries. Keep errors from Lapack.c verbatim; test values/vectors,
only.values=NA, zero-size, and nonsquare matrices against the oracle.


## 2026-10-30 (round 7): resume #92 and repair perf validation

- Took over #92 at 48556b1, preserving its commits and authorship, and
  rebased onto main 1074f6c. The failed run 40978398420 compared with
  main 4754027, not the PR's git merge base f668f60. All correctness
  jobs passed; only nbody_2 exceeded the perf threshold.
- Reproduced the original CI artifacts in robigo-oracle:4.5.0, with
  one shared ROBIGO_NATIVES and ROBIGO_RDBFLAT directory for both.
  Two runs each: base nbody_2 838,351,321 / 838,271,596 instructions;
  candidate 836,006,852 / 836,068,130. Both outputs match GNU R byte
  for byte. This does not reproduce CI's 839,190,277 -> 861,813,460
  regression; do not claim it is fixed until CI verifies the new head.
  Callgrind profiles show a 2.21M reduction, mostly reader allocations,
  Value clones, path hashing and native marker scanning. No evidence
  supports adding MAP_POPULATE or abandoning mmap.
- CI rechecks lost ROBIGO_RDBFLAT and ROBIGO_NATIVES at the step
  boundary (the recheck measured 871,047,321 instead of 861,813,460).
  Export both in the comparison step, matching the first measurement.
  The main baseline already enables the cache, so both sides use it.
- The spectralnorm_math FIXED report was false. Both original artifacts
  fail at the missing La_rs internal, with status 1. PR CI ran only
  robigo, discarded stdout, then copied GNU R's counts without deriving
  the wrong-output flag. Run both runtimes in PR perf jobs, so stdout
  and status are actually checked. No benchmark recordings changed.
- Fixed two ineffective Rc::try_unwrap(p.clone()) paths in the inherited
  reader by consuming the owned formals and dotted tail. Shared tails
  retain their clone fallback. Reject incomplete flat headers and
  overflowing offsets before slicing; use metadata from the open fd.
- The builder now publishes via atomic rename. Truncating a flat file
  that an existing process has mapped can SIGBUS; atomic replacement
  preserves the old inode. A live-mapping replacement probe confirms
  old readers retain their bytes and new readers see the replacement.

- Local verification before the #84 rebase (base 4754027 and candidate
  built on 1074f6c, whose runtime is identical to 4754027):
  library(stats) NULL flat 177,418,533 -> 175,388,419 instructions;
  plain 210,693,169 -> 208,834,362; default flat 397,858,068 ->
  391,988,954; default plain 479,938,389 -> 474,440,305; NULL flat
  38,868,211 -> 38,606,393. Natives disabled equally for these rows.
  Host peak RSS default flat 88,920 -> 69,680 KB (21.6% lower).
- Host paired nbody_2, same cache and natives for both: base
  840,603,706 / 840,604,286; candidate 838,184,690 / 838,167,755
  instructions (about 0.29% lower). The original CI artifacts were
  separately verified in the pinned oracle container above.
- 12 byte comparisons pass against GNU R 4.5.0: AST, VM and JIT each
  with full, absent, partial and stale caches; the probe exercises
  stats/lm, closures and GNU R-generated bytecode argument lists of
  lengths 0, 1, 8, 9 and 40. Atomic cache rebuild preserves all 32
  files byte for byte. Focused conformance 56/57 in all three tiers,
  zero ratchet failures, identical to base. FastR serialize/connections
  451/458 on both binaries, with the same oversized real-sequence
  serialization case failing locally on the base and candidate;
  pinned CI remains the judge. No ratchet entries changed.

- Final rebase onto df3d535 (#84 string handles) applied cleanly.
  Post-rebase runtime unit tests: 42 pass, including mapped-cache
  rejection tests; only c_library_oracle and c_library_trig_oracle
  fail by one ULP, matching the host glibc failures documented by
  sweep-data and core. No complex arithmetic or oracle files changed.
- Remaining stats profile before #84: VM run_with 7.92M self, memcpy
  5.12M, Reader::int 5.08M, read_item_flags0 4.44M, read_bc_lang 3.14M.
  Next measure these again on #84 before another reader optimization.
  Direct rdx-key enumeration remains a builder follow-up; mmap and
  CI cache wiring are complete. Regex construction and symbol handles
  overlap other tracks; do not change them here.

Reproduction: download the robigo artifacts from runs 40977240404 and
40978398420; run scripts/perf.py run --metric instr --runtime both
--filter 'rbench/shootout/(nbody_2|spectralnorm_math)$' --jobs 1 twice
per binary in the oracle container, with the same explicit cache and
native directories. On the host, perf.py overrides R_HOME via R RHOME;
use a PATH shim returning /home/m0hawk/.local/opt/R-4.5.0 for that query.


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

## 2026-10-29 (round 5): re-baseline on main 74e8c73

- Main moved a lot since round 4 (#79 zlib-rs, #81 release profile thin
  LTO + one codegen unit, #69 graphics, #77 utils, #80 matrix VM ops all
  landed after those measurements), so every scenario was re-measured
  from scratch (robigo-oracle:4.5.0, R 4.5.0-3, binaries built in
  robigo-build:4.5.0 with ~/.cache/robigo-startup-target; rdbflat cache
  rebuilt by the #82 battery; natives = the 8 packages CI builds).
  Harness: `.robigo/scen5.sh` (scratch; `def` = R_DEFAULT_PACKAGES unset,
  i.e. datasets,utils,grDevices,graphics,stats,methods — `search()`
  verified identical in both runtimes for every scenario).

  `-e 1` instructions:

  | scenario | plain | +rdbflat | +natives+flat | GNU R | best ratio |
  |---|---|---|---|---|---|
  | NULL packages | 46,113,680 | 41,253,451 | 41,234,945 | 87,757,913 | 0.470× |
  | utils,stats | 59,086,889 | 50,853,282 | 50,842,389 | 99,889,141 | 0.509× |
  | default (6 pkgs) | 516,289,615 | 439,100,029 | 441,158,450 | 660,550,300 | 0.665× |

  Other scenarios: script default 516.6M/441.6M vs GNU R 661.1M (0.668×);
  script NULL 46.6M/41.7M vs 88.2M; `library(stats)` from NULL 221.3M/
  191.7M vs 259.5M (0.739× — the weakest ratio: the library call itself
  costs robigo 175.2M vs GNU R's 171.7M). Natives overhead on def is
  +2.0M for 8 packages (round 4: +0.55M for stats alone); NULL and
  utils,stats are unaffected within noise.
- The old ledger's round-4 "utils,stats = 254-264M" rows do not
  reproduce on this main and GNU R itself measures 99.9M there today
  (NULL and def6 rows do reproduce to ~0.5%, so the harness agrees);
  those old rows were probably def5 (datasets,utils,grDevices,graphics,
  stats) runs. This table supersedes them.
- Where `default +rdbflat` goes now (441M, callgrind): serialize Reader
  ~75M (int 31M, read_item_flags0 23M, read_bc_lang 15M), mimalloc
  ~45M, VM ~27M, Value drops/clones ~21M, memcpy 11.1M, UTF-8 lossy
  decode 5.5M, regex_automata NFA build 2.75M (R code compiling fresh
  patterns; GNU R pays the same in TRE), SipHash (interning) ~1M.

## 2026-10-29 (round 5): serialize reader fast paths

- `read_charsxp`: binary formats now read the bytes straight from the
  stream (no `to_vec`), and valid UTF-8 interns as `Rc<str>` in one
  allocation (`str::from_utf8` fast path) instead of lossy-decoding
  chunks into a `String` and copying again — 3 allocations + 2 passes
  per string become 1 + 1. `int()`: `try_into` instead of four
  bounds-checked byte loads. `string_vec` pre-sizes; builtin names take
  the same UTF-8 fast path. Byte-identical by construction (the lossy
  path is the fallback).
- Measured (same harness): e1-def −19.5M (plain 516.3→496.8M, flat
  439.1→419.6M, natives+flat 441.2→421.6M); e1-us −1.6M; e1-NULL −1.0M.
- Gates: `cargo test --release -p robigo-runtime` green; conformance
  3 tiers ok; fastr serialize slice 165/169 (0 regressions, 2 new),
  connections/unserialize/gzfile slice 286/289 (0 regressions).

## 2026-10-29 (round 5, continued): the hill-climb

Five verified changes on top of the re-baseline (all gates green after
each: runtime tests, conformance 3 tiers, fastr slices with 0
regressions; commit messages carry the per-change numbers):

1. **read_bc_lang iterative** (−15.2M on def): the recursive reader
   rebuilt the suffix Value at every level of a bytecode chain and
   cloned its items into the next level up — O(n²) Arg clones with an
   intermediate Value per level. One flat items Vec now; repdef cells
   are contiguous slices of it. The base packages' bytecode has long
   spines, so this shows up directly in every scenario.
2. **complete rdbflat battery** (coverage, ~neutral measured):
   `scripts/build-rdbflat-force.R` forces every lazy binding of every
   base package (package env and namespace), so the cache covers every
   fetch any program can make — verified 793 fetches through 7 packages,
   0 misses. 52MB of flats (lazily read per rdb); complete coverage
   costs +0.65M on def-flat (index build) over the partial cache, won
   partly back by an inline FxHash for the flat index (SipHash cost more
   than the lookup). The battery takes ~1.5s.
3. **rdbflat on by default** (`rdbflat/` beside the exe, the
   build-natives.sh pattern; `ROBIGO_RDBFLAT` still overrides) and
   **wired into CI**: the build job builds it, the robigo artifact
   ships it, corpora and perf jobs point at it.
4. **rdx gzip cache** (−5.8M def, −1.2M us/NULL): each `.rdx` index is
   a gzip file inflated whole at every gzfile open; the builder writes
   each as a single (0,0) flat entry and `gz_open` serves it first.
5. **SYMSXP direct name read** (−2.6M def): no more throwaway
   one-string vector per symbol.

Where the scenarios ended (container; `flat` = cache, which is now the
default experience; GNU R unchanged):

| scenario | main 74e8c73 | this branch | Δ | GNU R | ratio |
|---|---|---|---|---|---|
| `-e 1` NULL, plain | 46,113,680 | 44,437,700 | −1.7M | 87,761,707 | 0.506× |
| `-e 1` NULL, flat | 41,253,451 | **38,420,534** | −2.8M | 87,761,707 | **0.438×** |
| `-e 1` utils,stats, flat | 50,853,282 | **45,956,897** | −4.9M | 99,891,741 | **0.460×** |
| `-e 1` default, plain | 516,289,615 | 478,935,749 | −37.4M | 660,539,218 | 0.725× |
| `-e 1` default, flat | 439,100,029 | **396,673,112** | −42.4M | 660,539,218 | **0.600×** |
| `-e 1` default, natives+flat | 441,158,450 | 398,634,424 | −42.5M | 660,539,218 | 0.603× |
| script default, natives+flat | 441,625,749 | 398,968,541 | −42.7M | 660,934,173 | 0.604× |
| `library(stats)` NULL, flat | 191,738,336 | **176,661,289** | −15.1M | 259,476,934 | **0.680×** |

Natives overhead on default packages is +2.0M (8 packages, static).

Secondary metrics (same container, shared machine — wall indicative):
robigo def `-e 1` ~91-94ms wall vs GNU R 122ms; NULL 11ms vs 22ms.
Peak RSS on def: 55.8MB without the cache, 89.4MB with it (the flats
are resident once read; GNU R 53.4MB) — mmap for the flats is open
item 4. The wall-time numbers above were taken on an idle moment; under
load both runtimes slow equally.

## 2026-10-30 (round 6): library(stats) hill-climb + rdbflat mmap

Branch `track/startup-perf-r6-stats` (PR #92), from main 24a5b8e (#88's
round 5 + #85's tracemem were in). Baseline re-measured first, all
scenarios reproduce round 5's table to <0.1%. GNU R targets unchanged
(`library(stats)` NULL 259.54M, def `-e 1` 660.64M, NULL `-e 1`
87.76M). Where `library(stats)`'s instructions go (callgrind, flat):
serialize reader ~15M self, mimalloc ~11M, VM ~12M (perf-vm), regex
compile ~13M inclusive (see below), Value drops/clones ~7M, memcpy 5M.
GNU R's own profile for the same scenario: inflate 18.6M (skipped by
the cache), xdr_int 14M, mkCharLenCE 7.6M, cons-cell writers 19.7M —
robigo is ahead in every column; the remaining mass is spread thin.

Five changes, all gates green after each (runtime tests, conformance 3
tiers on the touched areas, fastr serialize/connections/library slices,
0 regressions):

1. **CHARSXP interning — REJECTED** (details above): +5.3M.
2. **read_bc_lang inline spines** (SmallVec<[Level; 8]>): the
   iterative ReadBCLang grew its spine Vec by push (1-3 reallocs per
   bytecode chain). Chains are argument lists, ≤ 8 cells nearly always:
   lib-null-flat −1.4M, **def-flat −4.4M**.
3. **rdbflat mmap**: flat files are mapped read-only; entries are
   zero-copy slices of the mapping. Instructions unchanged (±0.1M),
   **def-flat peak RSS 89.6 → 71.1 MB** (GNU R 53.9). An intermediate
   pread-per-fetch design measured +1.9M instructions for 64 MB —
   rejected, instructions are the gate. `libc` dep added (already
   linked by mimalloc-sys/liblzma; rbench/simple + profiler-shared
   measured 0 slower, the memchr-layout lesson respected).
4. **rdbflat path maps FxHashed** (claims/loaded were SipHash per
   fetch): −0.17M lib, −0.39M def.
5. **fresh-tail moves in the reader**: ReadBCLang's spine cars and
   freshly-read pairlist/lang tails move into the items Vec instead of
   cloning (a tail reached through BCREPREF still clones — it is
   shared); CLOSXP formals and dotted-pairlist tails likewise: −0.22M
   lib, −0.83M def.

Where the scenarios ended:

| scenario | main 24a5b8e | this branch | GNU R | ratio |
|---|---|---|---|---|
| `library(stats)` NULL, plain | 209.89M | 208.23M | 259.54M | 0.802× |
| `library(stats)` NULL, flat | 176.56M | **174.70M** | 259.54M | **0.673×** |
| `-e 1` default, flat | 396.51M | **390.98M** | 660.64M | 0.592× |
| `-e 1` default, plain | 478.94M | 473.53M | 660.64M | 0.717× |
| `-e 1` NULL, flat | 38.36M | 38.18M | 87.76M | 0.435× |

Peak RSS: def+cache 89.6 → **71.1 MB**; NULL+cache 25.9 → 26.5 MB.
Wall (idle moment): def+cache ~89 ms vs GNU R ~128 ms. Numbers
re-measured after rebasing onto f668f60 (#90's inline-closure VM
change did not move startup): lib-null-flat 174.70M, def-flat 390.94M.

**Findings for other tracks:**

- **regex compile is 13M of library(stats)** (11.6M under `gsub_impl`
  → `Re::compile`): regex-automata's meta builder eagerly builds all
  engines, ~300k instructions per hairy POSIX pattern (strftime-style
  `(^|[^%])(%%)*%v` specs; only ~38 gsub calls in the whole run).
  Logged in conformance-strings' ledger; ~10M available if engines can
  be built lazily or pruned for the leftmost-longest path.
- `compile::Op` Vec growth (~1.0M def: Compiler::call/guard push
  without reserve) and `set_in_box`'s Arg growth (~0.6M) are perf-vm's
  files; noted in their ledger section of the open items below.
- The `read_item_flags0 → RawVec<compile::Op>` callgrind edge is
  misattributed (closure compilation is lazy — `Closure::code` is a
  OnceCell filled on first call); don't chase it.

## Open items (ranked)

1. **CHARSXP interning — measured, rejected (round 6).** A thread-local
   `HashMap<Box<str>, Weak<str>>` (FxHash) in `read_charsxp`, mirroring
   R: envir.c:mkCharLenCE's `R_StringHash`, made `library(stats)` from
   NULL **slower**: flat 176.56M → 181.82M (+5.3M), plain 209.89M →
   215.05M. The byte-at-a-time hash plus the `Box<str>` key copy (a
   second allocation per miss) cost more than the saved `Rc` allocs;
   the duplicate rate is too low for the early hits to pay for it.
   Ceiling check: `read_charsxp` self + `from_utf8` + the string share
   of malloc/drop is ~2-3M on this scenario — GNU R itself pays 7.6M in
   `mkCharLenCE`, so robigo's fresh-`Rc`-per-read is already cheaper
   than R's cache. Do not retry without open addressing with inline
   keys (no `Box` copy, chunked hash); even then the ceiling is ~2M.
2. **Symbol interner hashing** (`sym::Interner` uses SipHash; ~1.5M on
   def in sip+memcmp+memcpy during unserialize). FxHash it — but sym.rs
   is hot for every track; measure the perf suite, coordinate with
   perf-vm.
3. mimalloc is ~40M of the def-flat run (malloc+free paths) and drops
   ~8M: fewer Values/Vecs during unserialize is the structural fix
   (arena/region allocation for entry graphs, G6-snapshot style).
4. mmap the flats instead of `fs::read` (RSS: 52MB cache but only
   fetched pages would resident; instructions unchanged — reads are
   kernel-side). Needs libc or a new crate — watch the layout lesson.
5. From #36's list: S3 dispatch memsets during package loading (memset
   is 2.3M on def-flat), regex construction at startup (2.75M — R code
   compiling fresh patterns; GNU R pays the same in TRE, so parity),
   G6 snapshot (the endgame: skip the unserialize entirely).
6. Natives load is +2.0M for 8 packages — dlopen/mmap/relocation
   parity, not worth chasing.


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
