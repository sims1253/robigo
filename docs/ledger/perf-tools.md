# Perf tools: valgrind, sampling and fuzzing flows (2026-11-08)

This session made the dynamic-analysis flows run on the local machine and
extended them (council: GPT-6 Astra, Claude Opus 5.5, Claude Fable 5.1;
Argon's input arrived via Max). Docs live in `docs/profiling.md`; this
ledger records what was found and what was decided. No CI changes, no PRs.

## Environment findings (Arch + WSL2)

- **System valgrind 3.25.1 cannot start** on current glibc: the mandatory
  `memcmp` redirect into the stripped `ld-linux-x86-64.so.2` is missing
  ("Fatal error at startup"). This broke *every* valgrind tool locally —
  including `perf.py`'s callgrind gate, `callgrind-diff.py` and
  `alloc-count.py`. Fix: a private valgrind 3.27.1 built into
  `~/.cache/rr-tools` (the path `perf.py` already prefers), with
  `GLIBC_MANDATORY_STRLEN_REDIRECT` undefined in `config.h` (the redirects
  become advisory; only ld.so's own strlen/strcmp/memcmp go untracked).
  Upstream 3.27.1 still has the check; Arch's debuginfod was down (503),
  which is the other fix path.
- **robigo's heap is invisible to valgrind** until mimalloc is compiled
  with `MI_TRACK_VALGRIND`: allocations go through a static mimalloc
  behind a custom `#[global_allocator]` (`robigo-cli/src/main.rs`), not
  through an interceptable `malloc` symbol — memcheck saw 770 allocations
  for a 200,000-element script. `scripts/build-vg.sh` rebuilds with
  `CFLAGS=-DMI_TRACK_VALGRIND=1` into `.robigo/bin/robigo-vg`, now in its
  own target directory (`.robigo/target-vg`), so the shared target keeps
  no instrumented mimalloc and plain builds never flip-flop. With it,
  memcheck sees ~460,000 allocations.
  Never measure performance with this binary.
- **DHAT works against the tracked build** (it honours mimalloc's
  client-requested blocks): `dhat fib` resolves 738 blocks / 331,296 bytes
  with real frames. The first-round "DHAT stays blind" note was wrong —
  the 35,529-byte reading came from the review code's own 64-site walk
  cap, fixed with the findings.
- **perf 7.3.1 works** on this WSL2 kernel (cycles, instructions,
  branch-misses), but **LBR does not** (`perf record -b`: no branch stack
  sampling; Hyper-V has no PEBS) — `perf c2c` and `perf mem` are out.
  samply needs `perf_event_paranoid <= 1`
  (`echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid`).
- **The oracle trap, confirmed**: with host R 4.6.1 on PATH and no
  `.robigo/home`, robigo lazy-loads the *host's* package databases and
  prints `package 'stats' was built under R version 4.6.1` — stderr noise
  that drowns every stdout/stderr comparison (corpora, fuzz-eval, perf's
  wrong-output check). Fix:
  `scripts/robigo-home.sh ~/.cache/robigo-research/R-4.5.0-build/bin/R .robigo/bin/robigo .robigo/home`.
  `scripts/fuzz-eval.py` refuses oracles whose version is not 4.5.0.

## New flows

- `scripts/valgrind.py memcheck|cachegrind|callgrind|massif|dhat` — one
  entry point over the tools the gate does not cover; `--bench NAME` uses
  the gate's environment, `--runtime gnur` profiles GNU R the same way;
  `scripts/valgrind.supp` carries the mimalloc/ld.so suppressions.
- `scripts/samply.sh`, and a perf-map writer in the JIT: `ROBIGO_PERF_MAP=1`
  makes `/tmp/perf-<pid>.map` name every compiled function
  (`robigo_loop_N`, `robigo_clo_N_K`, `robigo_clo_entry_N`), so samply and
  perf finally attribute JIT frames (samply.sh sets it); callgrind does
  not read perf maps. `jit/perf_map.rs` owns it: a compile notes each
  function and flushes once, names come from the module's declarations,
  and the disabled path is one `OnceLock` read per compiled function.
- `scripts/perf.py --metric cache` — callgrind with cache and branch
  simulation, counters stored as a `cache` dict beside `instr` with
  bench.py's estimated-cycle model; `compare` refuses non-`instr` result
  files explicitly (diagnostic, never a gate input).
- `scripts/fuzz-eval.py` — differential eval fuzzing against the 4.5.0
  oracle (stability-filtered, corpus masking + Rtmp tmpdirs), and
  `--tiers`: VM vs `ROBIGO_JIT=0` on the same mutants with no oracle.
  Seeds come from the `tests/r` ratchet.
- `fuzz/` — cargo-fuzz harnesses (parse, rds) as an out-of-workspace
  crate; in-process, coverage-guided (~28k execs/s vs fuzz.py's
  process-per-input). `fuzz.py` stays the CI check: it alone covers
  process-exit semantics (rlimits, allocation failure, signals).
- `scripts/build-vg.sh`, `scripts/samply.sh` as above.

## Measured

- **autogc verified from outside**: `memcheck` on the cycle probes
  (env self-cycle, closure→env→binding cycle, list→closure cycle, 200k
  `lapply` churn): clean, `definitely lost: 0`, `indirectly lost: 0`;
  19,211,114 bytes in 416,005 blocks still reachable at exit (the runtime
  exits without teardown — expected). The 61 initial error contexts were
  all mimalloc's THP probe and stripped-ld.so AVX loads (suppressed).
  Evidence for the memory bake-off ledger: today's Rc runtime + `autogc`
  collects what GNU R's GC collects here.
- **cachegrind fib(22)** (caches 32 KiB/64 B/8-way, LL 64 MiB
  direct-mapped, valgrind's simple predictor model):

  | | robigo | GNU R | robigo/GNU R |
  |---|---:|---:|---:|
  | Ir | 99,144,181 | 338,766,575 | 0.29 |
  | D1 read misses | 289,834 | 1,176,473 | 0.25 |
  | LL read misses | 20,508 | 37,012 | 0.55 |
  | cond. mispredicts | 440,635 | 1,757,967 | 0.25 |
  | indirect mispredicts | 770,456 | 633,783 | **1.22** |
  | est. cycles (sensitivity model) | 113,952,841 | 404,721,855 | 0.28 |

  robigo runs fib in a third of GNU R's instructions but with 22% *more*
  indirect-mispredict events: the VM's dispatch and call indirection is
  the visible target for the perf-vm track (branchy builtin dispatch).
- **DHAT fib(22)**: 35,529 bytes in 122 blocks total — the scalar cache
  and promise-free argument passing make a call-heavy program allocate
  almost nothing at the R level. **massif grow.R**: peak heap
  18,293,403 bytes, attributed (`lang.rs` assignment paths).
- **Differential eval fuzzing leads**: recorded
  `tests/r/lang/parse-invalid-multibyte.R` (oracle: GNU R swallows the
  invalid byte in `zz \xff yy` and reports `unexpected end of input`;
  robigo reports `unexpected input in "zz …"`). GNU R's message varies
  with byte position — mid-expression with trailing tokens it says
  `invalid multibyte character in parser (<input>:L:C)` — so more shapes
  may become scripts. (836→~600 runs/30-60 s per smoke; 2-3% oracle-unstable
  inputs correctly filtered.)
- **Cache sample (2026-11-08, robigo suite)**: 55 of 89 benchmarks with
  callgrind cache/branch counters (`--metric cache`; simulated caches,
  diagnostic only — the gate stays instruction counts). Geomean est.
  cycles/instruction 1.25. Highest LL read-miss rate: `vecarith` 0.49%,
  `vecops` 0.08%, `vectorWhile`, `convolution_v` — the vector workloads
  stream memory as expected. Highest D1 read-miss rate: `convolution_v`
  1.70%, `vecops` 1.58%, `vecarith` 1.30%. Highest indirect-mispredict
  share: `Bounce` 4.7%, `fib` 4.5% — closure-call dispatch. Collected in
  ~2 min batches under setsid after three detached runs were swept
  (13:28/13:54/14:20, silent SIGKILL, period unknown); excluded as
  unmeasurable in that window: flexclust trio, knucleotide_brute trio
  (`knucleotide_brute_3` alone is 117G instructions); `volcano` timed out.
  Raw counters: `.robigo/perf/cache-sample.json`.
- **Tier differential**: 4,565 mutants clean (0 mismatches, 0 crashes,
  0 unstable — robigo boots with a fixed RNG seed, so mutants are stable).

## Council verdicts

- **P1 cache metric: ADOPT (3/3).** Via callgrind, not cachegrind — one
  run yields Ir + cache + branch counters and the call graph;
  `experiments/memory/bench.py`'s est-cycles formula reused as a labeled
  sensitivity model; geometry and provenance recorded; `compare` refuses
  cache-only files; diagnostic only, never a gate input (#236's noise is
  the reason). Done.
- **P2 sampling: ADOPT (samply now).** Both Fable and Opus named the real
  gap: **JIT frames are anonymous** to samply/perf/callgrind — a
  perf-map writer (`/tmp/perf-<pid>.map`) in `jit.rs` is the small,
  safe prerequisite. pprof (`--features profile`, `ROBIGO_PROFILE`)
  already exists. LBR/c2c: unavailable on this host (verified).
- **P3 gungraun: SKIP (3/3).** `ldouble-callgrind.sh` already does
  matched-baseline subtraction; use callgrind's `--toggle-collect` to
  drop the baseline run. Revisit only with dozens of kernels.
- **P4 crabgrind: SKIP now (Fable/Opus) / later-with-purpose (Astra).**
  Startup exclusion: `--collect-atstart=no --toggle-collect=<entry>`;
  client requests add instructions to the gated binary. The real payoff —
  MALLOCLIKE/NOACCESS hooks inside robigo-core's own heap — belongs in
  the G2 design behind one feature.
- **P5 dhat-rs: SKIP.** Asserts on the allocator G2 replaces; budgets
  would be per-iteration deltas at best.
- **P6 differential eval fuzzing: ADOPT, staged.** Tier-differential
  first (done: `--tiers`), oracle stage second (done: 4.5.0-only oracle,
  stability filter, masking). Remaining: AST-level mutation instead of
  bytes, sandboxing (mutants can run `system()`/`unlink()`; today only
  `HOME=/tmp` and no network), reproducer minimization.
- **P7 cargo-fuzz: ADOPT (parser).** Done, complement not replacement.
  rds target: keep local (LSan flags the exit-live runtime — run with
  `-detect_leaks=0`); seed corpora beyond one file later. The CI-ready
  piece for later: a minimized corpus replayed as a plain `cargo test`.
- **P8 valgrind wrappers: ADOPT.** Done (`scripts/valgrind.py`,
  `build-vg.sh`, `valgrind.supp`); massif/dhAT numbers above feed the
  memory bake-off. Next: cycle-slope measurement (N vs 2N DHAT/memcheck
  deltas), memcheck over the CRAN-native C (where the real memory bugs
  live).
- **P9 sizes: one-off notes, not scripts** (Value/Vector layouts change
  with G2); static asserts for hot layouts belong in perf-vm work.
- **P10 ASan/LSan: post-G2**, behind the same sanitizer interface as the
  valgrind hooks (poisoning freed slots, `#[no_sanitize(address)]` on the
  conservative stack scan). Miri's gap here is test coverage
  (`serialize.rs` readers, `arith_exact.rs` kernels have no
  `#[cfg(test)]`), not configuration.

Also raised, not yet built: provenance columns in perf results (binary
identity, source revision — `callgrind-diff.py` defaults to the shared
target binary another worktree can replace); a `ROBIGO_GC_STATS` counter
so #236-style flags (gc frequency driven by allocator geometry) triage in
one glance; fuzz targets for the bytecode decoder and the rdbflat reader.

## 2026-11-12: slow perf creep on main (`history.py check`)

The pull-request gate allows 0.5% per benchmark per pull request, so many
small regressions can add up on main unseen. Borrowed from blaze
(`harness/benchhistory.py check`, docs/benchmarks.md "History and
regressions"): after each main run, CI compares robigo's instruction counts
on `progress-data` with the median of the 7 runs before and with an anchor,
the newest run at least 7 days older. Band: 1% or 3 × 1.4826 × MAD of the
window per benchmark, 0.5% (or its own spread) for the geometric mean,
doubled below three runs. A regression the previous run did not show opens
or comments on a `perf-regression` issue, naming the pull request with the
largest rise; main never fails on it.

- **Noise in the history** (348 main runs, 2026-10-27 to 11-12): robigo's
  change between consecutive runs (the last 60) has median 0.005%, 90th
  percentile 0.08%, 99th 0.65% (the 99th is real changes). GNU R's counts,
  which robigo cannot change, move by 0.01-0.03% on some benchmarks
  (`pidigits`, `dataframe`, `strings` flip back and forth: two runner
  machines), and jumped 1000-8000% for the `knucleotide*`,
  `reversecomplement*` and `regexdna` programs at #485 (2026-11-07) when
  their input began to exist. Those jumps were the first anchor
  "regressions" the check found; it now skips a benchmark whose GNU R count
  moved past the band too (its program or input changed, not robigo) and
  lists it as changed.
- **Replay over the real history** (every main run checked against the
  runs before it): 20 of 348 runs would have opened or commented on the
  issue, all before 2026-11-07, most in the first two days' churn. One is a
  real slow step the gate let through: #471 (rjit, 2026-11-05) +0.87%
  geomean against the median, 36 benchmarks past 1%, `Mandelbrot` +1.54%.
  The newest run (0cf1a7b5, #585): nothing regressed; against the anchor
  (2026-11-05) 80 of 80 compared benchmarks improved past 1%, geomean
  -24.7%, 8 skipped as changed programs.
- **Why "new only"**: a step stays past its band until the window's median
  moves (4 runs), a creep against the anchor until the anchor passes it;
  reporting only what the previous run did not show keeps it to one
  comment per event.
- **Synthetic regressions** (`scripts/test-history.py`, 22 cases, run in the
  perf job): a 2% step is caught against both bases and an 0.8% one is
  not; a 0.3% rise every tenth run (under the gate's 0.5%) never trips the
  median but trips the anchor; a 0.7% rise of every benchmark trips only
  the anchor's geomean; a benchmark that moves by ±1% between runs gets a
  wider band; wrong, failed, new and changed programs are not compared; the
  report file is written only when there is news.
- `scripts/history.py` and `scripts/test-history.py` select no perf
  benchmarks (`benchmarks/selection.toml`, no-effect).
- actionlint 1.7.7 on `ci.yml`: no new findings (the two SC2086 notes are
  on main already).

## Open

1. `ldouble-callgrind.sh`: `--toggle-collect` would drop the baseline
   run, but `policy_bench`'s kernels are Rust-mangled symbols with a
   build hash — a toggle pattern breaks on every rebuild. Keep the
   baseline subtraction until the benchmark exposes `#[no_mangle]`
   entry points.
2. Reduce the remaining `fuzz-eval.py` leads (multibyte parse errors have
   more shapes: `zz \xff yy` is recorded as
   `tests/r/lang/parse-invalid-multibyte.R`; a byte mid-expression with
   trailing tokens reports `invalid multibyte character in parser`).
3. cargo-fuzz rds: oracle-generated corpus (versions, cycles, ALTREP,
   bytecode), bytecode/rdbflat targets.
4. Cache/branch counters in `perf-paired.py`'s flagged-benchmark output
   (D1/LL deltas next to Ir for triage).

## 2026-11-12: floors and causes

robigo measured itself only against GNU R, a rival. Two borrowed ideas
measure against the work itself instead: blaze's rule 67 ("design within
10% of the theoretical best ... every measurement reports the ratio", its
`docs/v2/plan/bounds.md`) and relax's README table that splits a gap by
cause and ends "Every row is a work item."

**Floors** (`scripts/idiom-vs-r.py --idioms`, `benchmarks/idioms.toml`).
Eight idioms, one per kind of per-iteration cost; each carries a plain C
version of the same work, measured the same way (callgrind, N against 2N
after `cc -O2`, GCC 16.2.1) and stored with its arithmetic. The C programs
keep their results alive with a zero-instruction asm sink rather than
printing them: printf's cost varies with the value and leaked 0.006
instructions per iteration into the first floors; for the same reason N
and 2N need as many digits (strtol). N = 20,000 (1,000 for vec-arith),
because robigo's counts vary by a few hundred instructions per run (hash
seeds): at N = 2,000 scalar-add read 10, 14 and 12 in three runs. Floors
for whole benchmark programs are not kept: their C versions would be
ports of the programs.

Measured (robigo at a29b978d, GNU R 4.5.0 from
`~/.cache/robigo-research/R-4.5.0-build`, instructions per iteration):

| idiom | robigo | GNU R | floor | robigo÷GNU R | robigo÷floor |
|---|---:|---:|---:|---:|---:|
| scalar-add `s <- s + i` | 12 | 236 | 6 | 0.05 | 2.01 |
| int-add `k <- k + i` | 20 | 244 | 7 | 0.08 | 2.86 |
| branch `if (i %% 2L == 0L) s <- s + 1` | 18 | 1,192 | 5.5 | 0.02 | 3.30 |
| vec-read `s <- s + x[i]` | 15 | 978 | 2.5 | 0.02 | 5.98 |
| vec-write `x[i] <- i` | 20 | 435 | 2.25 | 0.05 | 8.88 |
| vec-arith `y <- x * i + 1` (1000 elements) | 8,317 | 25,246 | 3,509 | 0.33 | 2.37 |
| closure-call `s <- f(s)` | 10 | 3,468 | 2.5 | 0.00 | 4.07 |
| s3-dispatch `s <- s + area(sq)` | 17,795 | 13,615 | 2.5 | 1.31 | 7,118 |

The JIT'd loops are 20-350× ahead of GNU R and still 2-9× above the floor:
the floor vectorises vec-write four at a time and unrolls vec-read and the
inlined closure by two, and the int-add floor folds the overflow test into
a branch-free flag. vec-arith is 2.4× its one-pass floor (R allocates `x * i`
and `+ 1` as two vectors and makes two passes). S3 dispatch is the outlier,
7,000× the floor and 1.3× *GNU R*: it is not compiled at all. Also fixed on
the way: `idiom-vs-r.py` ran GNU R's `exec/R` without `R_HOME/lib` on the
library path (the 4.5.0 build's `libRblas.so` failed to load) and, with no
exit-status check, reported 0 instructions for GNU R; it now sets the path
as R's front end does and stops on a failed run.

**Causes** (`scripts/callgrind-diff.py --bench NAME --causes`,
`benchmarks/causes.toml`). A benchmark against an empty program in the same
environment (startup cancels), callgrind's lines mapped to causes by regex.
FILE decides only where inlined code is the cause wherever it lands (Rc's
counts, the allocator wrapper in `robigo-cli/src/main.rs`, libc, zlib);
everything else goes by the function's module path, because std's Option
and slice code inlined into `Value::get_attr` is attribute handling (a first
cut by file put 10% of Bounce in "Other"). Share of each benchmark's own
instructions (startup excluded):

| Cause | fib | Bounce | nbody | vecarith |
|---|---:|---:|---:|---:|
| Dispatch and evaluation | 78.8% | 27.1% | 55.1% | 2.6% |
| Allocation, reference counts and GC | 16.6% | 30.1% | 21.8% | 0.6% |
| Builtins and numerics (incl. libm) | 2.1% | 25.5% | 12.9% | 95.5% |
| Attribute handling | <0.1% | 10.3% | 3.8% | <0.1% |
| Values (lengths, element access, boxing) | <0.1% | 3.4% | 1.6% | 0.6% |
| Guards and type checks | <0.1% | 1.4% | 1.8% | <0.1% |
| libc (memcpy, memset, ...) | <0.1% | 0.8% | 1.1% | 0.5% |
| Lazy loading during the run | 0.1% | 0.9% | 0.9% | <0.1% |
| JIT-generated code | 1.6% | <0.1% | <0.1% | <0.1% |
| JIT and bytecode compilation | 0.5% | 0.1% | 0.1% | <0.1% |
| Other (std hashing, iterators) | 0.1% | 0.5% | 1.0% | 0.1% |
| **Instructions** | 169.8M | 387.4M | 500.6M | 3,602.7M |

Every row is a work item: fib is the VM's call path (`run_inline_args`,
`nested_call_inline`) plus dropping the argument SmallVecs (16.6%); Bounce
spends 30% on allocation and reference counts (`drop_glue<Value>`,
`mi_free`) and 10% on `Value::get_attr`, an attribute lookup per `$` and
`[[`; nbody is the VM's `run_with` and variable lookup (`plain_lookup`,
`Vars::flush_pending`) and 22% allocation; vecarith is libm (`log1p`
1.46G, `exp` 0.72G) and the arithmetic builtins, i.e. at its floor's
shape already. Limit: a guard inlined into the VM loop counts as the VM;
the JIT's code is one anonymous row (callgrind does not read perf maps).
`scripts/test-callgrind-diff.py` pins twenty real lines to their causes
and checks the idiom data; both new data files and the two scripts select
no perf benchmarks (`benchmarks/selection.toml`, no-effect).

# Counter gate (2026-11-12)

A whole-benchmark instruction total hides a path that got ten times worse.
Modelled on blaze's `harness/v2/guards.py` / `guards-known.toml`: count the
work that should be zero, hold it on fixed fixtures, and keep a known-failures
list that may only shrink. `alloc-count.py` was the starting point, but it
counts `__rust_alloc` calls from callgrind's call graph: it needs valgrind,
takes minutes, and cannot see inlined events (a copy inside `Rc::make_mut`,
a promise or frame built inline), so nothing could gate on it.

## Decided

- **A cargo feature, not callgrind.** `--features counters`
  (`robigo-runtime/src/counters.rs`): vector payload copies (a counted
  `Clone for Vector<T>`, which is what `Rc::make_mut` calls on a shared
  vector), promises (each `Promise` constructor), environments
  (`Env::with_frame`), allocator calls (robigo-cli's global allocator,
  alloc + realloc, as alloc-count counts them), VM ops (total and by kind)
  and JIT loops/rejects/deopts (`vm::jit_enter`). One JSON line on stderr
  at exit (`sys::at_exit`). It replaces `--features op-stats` (its only users
  were docs): the op-by-kind table is now the line's `ops`.
- **Free in release, shown on the binary.** Without the feature `count` is an
  empty `#[inline(always)]` fn. Release binaries of main (a29b978d) and the
  branch: same `.text` size (12,203,395 bytes), all 21,727 function symbols
  the same size, normalized disassembly differs only in moved blocks and one
  branch polarity in `jit_enter` (once per loop entry); `perf.py compare`
  over 89 benchmarks (all but knucleotide_brute*/flexclust_kcca): 0 slower,
  0 faster (> 0.5%), per-benchmark -0.14% to +0.05%.
- **Per iteration, per tier.** `scripts/counters.py check` runs each
  `tests/counters/*.R` fixture at n = 1000 and 3000 in the ast, vm and jit
  tiers and divides the difference: startup and warm-up cancel, numbers are
  exact and repeat bit for bit (checked: two runs identical; raw counts
  linear in n). The tiers matter: the JIT runs every fixture but one with
  no counted event at all (it even keeps `y <- x; y[1] <- i` in its own
  buffer, no copy), so copy-on-modify is only visible in the VM and the
  interpreter.
- **Bounds in fixture headers** (`#| [tiers] counter <= n`), the design
  target where it is zero, the measured number where a fixed cost is the
  design (the interpreter's 18-21 allocations an iteration, a frame and a
  promise per call in the VM). Known failures in `tests/counters/known.toml`.
- **CI**: a `counters` job (heavy; its own quick build with the feature,
  like bytecode-fidelity; the checks take under a second).

## Measured

- 105 bounds over 7 fixtures, 102 hold, 3 known:
  `l$a[i] <- v` (`dollar_index_assign`) makes 37 allocator calls an
  iteration in every tier and no copy. The bytecode compiler leaves the
  nested assignment to the `<-` special (`CallSpecial`), whose `*tmp*`
  path builds the `$<-`/`[<-` calls and subscripts; the JIT leaves native
  code there every iteration and rejects the loop (4 VM ops an iteration).
  `x[i] <- v` costs 0 in the VM and runs natively.
- **Planted regressions caught.** A second reference held in
  `set_index_fast` (forced copy): `index_assign[vm]` and
  `index_assign_fn[vm]` fail with 1 copy and 2 allocations an iteration,
  exit 1. `jit_enter` returning `None` (no loop native): 11 bounds fail,
  every jit-tier fixture (vm_ops 4-14, the free-variable call's frame and
  promise, `shared_assign`'s copy), while the known entries hold.

## Follow-ups

1. JIT exit reasons: another agent is adding a JIT exit report with named
   reasons and deopt counts. Each reason becomes a counter (one line in
   `counters!`, one `count` at the exit) and fixtures can bound it
   (`#| jit jit_exit_<reason> <= 0`). Closure-JIT calls (`jit::closure`)
   are not counted yet.
2. Compile `l$a[i] <- v` (and `x[[i]][j] <- v`) in the VM, then take the
   three known entries out.
3. More fixtures as paths matter: `lapply`/`vapply` over closures,
   S3 dispatch in a loop, `x$name <- v` on a data frame column, string
   building with `paste0` in a loop.
