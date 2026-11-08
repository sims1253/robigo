# robigo: profiling, fuzzing and dynamic analysis

How to find out what robigo (or GNU R) actually does when it runs:
valgrind's full suite, sampling profiles, the fuzzers, and the compiler's
own output. The instruction-count gate and its CI role live in
`docs/verification.md` (Performance, fuzzing); everything here runs
locally, on real workloads, where the iteration is fast.

## Which tool answers which question

| question | tool | notes |
|---|---|---|
| how many instructions did this benchmark take | `scripts/perf.py run --metric instr` | the gate; exact, repeatable |
| why do caches or the branch predictor hate this loop | `scripts/perf.py run --metric cache` | callgrind with cache and branch simulation: counters beside `instr`, estimated cycles; diagnostic, never a gate |
| which function got the instructions | `scripts/callgrind-diff.py WORK.R BASE.R UNITS` | subtracts a workload-sized base; startup cancels |
| how many allocator calls | `scripts/alloc-count.py` | `__rust_alloc`/`__rust_realloc` per benchmark |
| what does wall time really spend | `scripts/samply.sh SCRIPT.R` | perf sampling, opens the Firefox Profiler |
| where does the heap peak and grow | `scripts/valgrind.py massif` | `ms_print` graph |
| does robigo leak or touch bad memory | `scripts/valgrind.py memcheck` | needs the valgrind-visible build (below) |
| does this input crash the parser / `readRDS` | `scripts/fuzz.py`, `fuzz/` (cargo-fuzz) | CI runs `fuzz.py`; cargo-fuzz is the local, coverage-guided one |
| does robigo *behave* like GNU R on inputs nobody wrote yet | `scripts/fuzz-eval.py` | differential fuzzing against the 4.5.0 oracle |
| do the VM and the JIT agree on inputs nobody wrote yet | `scripts/fuzz-eval.py --tiers` | no oracle needed; any difference is a miscompile candidate |
| what did LLVM make of this kernel | `llvm-mca`, `cargo asm`, `-C remarks` | PGO: `scripts/pgo-build.sh` |

Allocation attribution for robigo itself lives in memcheck (with the
valgrind-visible build), `alloc-count.py` and massif — DHAT's interception
does not see a statically linked mimalloc (it does see GNU R's mallocs
fine). Compare robigo and GNU R under the same tool (`--runtime gnur` in
`scripts/valgrind.py`, or the same script twice in samply): a gap in
instruction counts is an answer, a gap in cache misses or branch misses is
the next hypothesis.

## Local prerequisites

`scripts/build.sh` (release binary), and for everything that compares
against GNU R, the pinned oracle — the host's `R` is usually newer and
disagrees in the last bits:

```sh
scripts/build-oracle.sh        # -> ~/.cache/robigo-research/R-4.5.0-build
```

**robigo's home.** Locally, `R RHOME` points at the host's R. Without its
own tree, robigo lazy-loads the *host's* package databases and prints
`package 'stats' was built under R version 4.6.1` warnings the oracle
never prints — every stdout/stderr comparison (corpora, `fuzz-eval.py`,
`perf.py`'s correct-output check) drowns in noise. Build robigo's home
from the oracle once per worktree (CI does the same, `docs/verification.md`
§ CRAN):

```sh
scripts/robigo-home.sh ~/.cache/robigo-research/R-4.5.0-build/bin/R .robigo/bin/robigo .robigo/home
```

Run scripts with `R_DEFAULT_PACKAGES=NULL` (as `scripts/fuzz-eval.py` and
`scripts/regen-expected.sh` do) so a local `R_LIBS_USER` cannot leak the
host's library tree into the run.

**valgrind.** Arch's valgrind 3.25.1 cannot start on current glibc: the
`ld-linux-x86-64.so.2` `memcmp` redirect it requires is gone from the
stripped dynamic loader ("Fatal error at startup"), which breaks the perf
gate, `callgrind-diff.py`, `alloc-count.py`, everything. Two fixes:

- Ask for a newer valgrind if the distribution ships one that handles the
  stripped loader (3.27.1 still does not); or
- build a private one — `scripts/perf.py` and `scripts/valgrind.py` pick
  up `~/.cache/rr-tools/bin/valgrind` automatically (`$VALGRIND`
  overrides):

  ```sh
  curl -fsSLO https://sourceware.org/pub/valgrind/valgrind-3.27.1.tar.bz2
  tar xjf valgrind-3.27.1.tar.bz2 && cd valgrind-3.27.1
  ./configure --prefix=$HOME/.cache/rr-tools
  # make the ld.so redirects advisory (Arch strips ld.so; the fatal check
  # otherwise aborts every tool): in config.h, comment out the
  # GLIBC_MANDATORY_STRLEN_REDIRECT define.
  make -j && make install
  ```

**A valgrind-visible robigo.** robigo allocates through a statically
linked mimalloc behind a custom global allocator, so valgrind's tools see
a few big mmaps and nothing else (770 "allocations" for a 200,000-element
script). `scripts/build-vg.sh` rebuilds mimalloc with
`MI_TRACK_VALGRIND=1` into `.robigo/bin/robigo-vg`; `scripts/valgrind.py`
prefers it, and `scripts/valgrind.supp` silences mimalloc's own THP probe
and the stripped loader's wide loads. With it, memcheck sees every block:
Rc-cycle leaks show up as definitely-lost, and the leak report becomes
evidence for the memory bake-off (`experiments/memory`,
`docs/ledger/memory.md`) — run `.robigo/profiling/leaks.R` before and
after any object-model change. DHAT stays blind to mimalloc blocks even
then; use memcheck, massif and `alloc-count.py` for allocation questions.
Never measure wall time or instruction counts with `robigo-vg`.

**perf / samply.** `scripts/samply.sh` needs `perf` (`pacman -S perf`) and
working perf events — a WSL2 or paravirt kernel may not have them; it
fails fast with a message when `perf stat` cannot read events.

## scripts/valgrind.py

```sh
scripts/valgrind.py memcheck  tests/r/env/cycles.R          # invalid access + leaks
scripts/valgrind.py cachegrind .robigo/profiling/fib.R      # Ir, cache and branch events
scripts/valgrind.py cachegrind --bench rbench/shootout/fib  # gate environment, gate naming
scripts/valgrind.py cachegrind --bench rbench/shootout/fib --runtime gnur   # GNU R, same tool
scripts/valgrind.py dhat        --bench robigo/binarytrees  # + viewer: xdg-open /usr/lib/valgrind/dh_view.html
scripts/valgrind.py massif      .robigo/profiling/grow.R
scripts/valgrind.py callgrind   SCRIPT.R --source           # cg-style source attribution
```

`--bench NAME` runs a `scripts/perf.py` benchmark with the gate's
environment (`R_DEFAULT_PACKAGES`, C.UTF-8, TZ=UTC), so numbers line up
with `perf.json`; `--runtime gnur` profiles GNU R's own binary the way
`perf.py` does under callgrind. Outputs and captured stderr land in
`.robigo/valgrind/`.

Reading cachegrind's columns: `Ir` instructions (what the gate counts),
`Bc/Bcm` conditional branches and their mispredictions, `Bi/Bim` indirect
jumps and theirs (the interpreter's dispatch lives here), `D1mr/DLmr`
data-cache misses (L1 / last level). Deterministic like callgrind, so a
redesign of a hot loop is judged on misprediction counts, not on wall
noise — but the *gate* stays instruction counts; cache misses are
diagnostic (they do not transfer across machines, and valgrind's predictor
is a simple model, not your CPU). The same counters are available per
benchmark as `scripts/perf.py run --metric cache` (stored in the result
JSON as a `cache` dict; `perf.py compare` refuses such files).

## Sampling: scripts/samply.sh

```sh
scripts/samply.sh benchmarks/fib.R            # opens the Firefox Profiler
scripts/samply.sh benchmarks/fib.R --save-only -o .robigo/samply/fib.json
scripts/samply.sh SCRIPT.R --env R_DEFAULT_PACKAGES=utils,stats
```

Sampling sees what deterministic counters never will: time in GNU R's
BLAS, in dlopen'd package code, in the JIT's machine code, and plain
waiting. Two caveats: samply needs `perf_event_paranoid <= 1` (the script
tells you the command), and JIT frames need `ROBIGO_PERF_MAP=1` — the
JIT then writes `/tmp/perf-<pid>.map` (`robigo_loop_N`,
`robigo_clo_N_K`, `robigo_clo_entry_N`), which samply and perf read;
callgrind does not, so JIT frames stay anonymous there. The profile is a
flamegraph over the *whole process*, GNU R's comparison runs included —
profile the same script on both runtimes and compare where the samples
go. A build-in flamegraph already exists: `--features profile` with
`ROBIGO_PROFILE=out.svg`.

## Fuzzing

Two layers, one contract: a panic, failed allocation, stack overflow or
fatal signal is a finding, and reproducers are kept.

- `scripts/fuzz.py parse|rds` — mutation fuzzing through the real binary,
  one process per input, seeded from `tests/r`. This is what CI runs (60 s,
  fixed seed on pull requests, fresh seeds on main).
- `fuzz/` — the same input classes in-process, with coverage feedback:
  orders of magnitude more executions per minute, so it reaches the deep
  paths (nested raw strings, encodings, unserialize stream mutations) the
  mutation pass only samples. Local, nightly toolchain:

  ```sh
  cargo +nightly fuzz run parse -- -fork=16 -max_len=65536 -max_total_time=600
  cargo +nightly fuzz run rds   -- -fork=16 -max_len=65536 -max_total_time=600 -detect_leaks=0
  ```

  Crashes land in `fuzz/artifacts/*` (`-fork` keeps the run going and
  collects them). The harnesses are deliberately minimal: `parse` feeds
  bytes through `from_utf8_lossy` into `robigo_syntax::parse_program`,
  `rds` feeds bytes into `serialize::unserialize` on a bare `Interp` — a
  malformed stream must end in an error, never a panic. LSan flags the
  runtime's exit-live heap, hence `-detect_leaks=0` on `rds`.

### Differential eval fuzzing: scripts/fuzz-eval.py

Mutates `tests/r` seeds (the same mutator as `scripts/fuzz.py`, restricted
to the ratchet where markers exist), runs each mutant on the GNU R 4.5.0
oracle twice and on robigo once, all `R_DEFAULT_PACKAGES=NULL`, and
compares normalized stdout, stderr and exit status (`scripts/corpus.py`'s
masking, plus the random `R CMD SHLIB` tmpdirs). Differences are candidate
conformance gaps — leads, not verdicts:

```sh
python3 scripts/fuzz-eval.py --seconds 300 --jobs 16        # against GNU R 4.5.0
python3 scripts/fuzz-eval.py --tiers --seconds 300 --jobs 16  # VM vs ROBIGO_JIT=0
```

- The oracle must be GNU R 4.5.0; newer R disagrees for reasons that are
  not robigo's bugs, so the script refuses.
- GNU R must answer identically twice, or the input is marked unstable and
  skipped (the same rule as the corpora).
- A robigo crash is a finding and fails the run, as in `scripts/fuzz.py`.
- Timeouts and diffs are reported, bucketed by first difference; one
  reproducer per bucket under `.robigo/fuzz/eval/`.
- `--tiers` needs no oracle: the mutant runs on robigo's default tier and
  with `ROBIGO_JIT=0`; the VM and the JIT must print the same thing
  (`docs/architecture.md`), so a difference is a miscompile candidate.
- Reduce a lead before working on it: a minimal
  `tests/r/<area>/<name>.R` with `scripts/regen-expected.sh`, no ratchet
  marker until it passes (`docs/workstreams.md`, protocol rule 8).

## Compiler output and sizes

- `llvm-mca` on a hot kernel's assembly: throughput, port pressure,
  register pressure. `cargo install cargo-show-asm` gives `cargo asm
  --rust robigo_numerics::policy::...`; or `cargo rustc --release -p
  robigo-numerics -- --emit asm`.
- Why a loop did not vectorize: `RUSTFLAGS="-C
  llvm-args=-pass-remarks-missed=loop-vectorize -C
  llvm-args=-pass-remarks-analysis=loop-vectorize" cargo build ...`.
- Monomorphization bloat (instruction-cache pressure): `cargo install
  cargo-llvm-lines; cargo llvm-lines --release -p robigo-runtime | head -30`.
- Type and padding sizes: `RUSTFLAGS="-Z print-type-sizes" cargo +nightly
  build` (grep the output for `Value`, `RString`, the VM registers).
- PGO stays in `scripts/pgo-build.sh` (`docs/ledger/pgo.md`, ~13%).

## CI vs local

| runs in CI | runs locally |
|---|---|
| `perf.py` instr gate, `perf-paired.py` confirmation | `callgrind-diff.py`, `alloc-count.py`, `valgrind.py` (all tools) |
| `fuzz.py` 60 s/target, fixed seed | cargo-fuzz long runs, `fuzz-eval.py` |
| corpora with ratchets | filtered runs, `triage.py` on one case |
| Miri (`value.rs`), clippy, gctorture | the same commands, before pushing |

The gate compares instruction counts only; cachegrind, DHAT, massif,
samply and fuzz-eval are hypotheses-and-leads tools. Bring their findings
into the gate by fixing the code, not by adding them to CI.
