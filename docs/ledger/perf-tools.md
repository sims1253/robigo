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
  `CFLAGS=-DMI_TRACK_VALGRIND=1` into `.robigo/bin/robigo-vg` (cc ignores
  plain CFLAGS for fingerprinting: `cargo clean --release -p
  libmimalloc-sys` first). With it, memcheck sees ~460,000 allocations.
  Never measure performance with this binary.
- **DHAT still sees only libc-level blocks** even in the tracked build
  (its interception does not pick up mimalloc's client-requested blocks).
  It validates against GNU R fine. Allocation attribution for robigo:
  memcheck's leak report, `alloc-count.py`, massif.
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
  perf finally attribute JIT frames; callgrind does not read perf maps.
  Env-gated, one `OnceLock` read per compiled function when off.
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
