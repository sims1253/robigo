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
| what is a benchmark's cost made of | `scripts/callgrind-diff.py --bench NAME --causes` | instructions by cause (dispatch, allocation, guards, attributes, JIT code, builtins, ...); below |
| how far is an idiom from the best possible | `scripts/idiom-vs-r.py --idioms` | per iteration: robigo, GNU R and a plain-C floor; below |
| how many allocator calls | `scripts/alloc-count.py` | `__rust_alloc`/`__rust_realloc` per benchmark |
| what does wall time really spend | `scripts/samply.sh SCRIPT.R` | perf sampling, opens the Firefox Profiler |
| which VM op of a JIT function gets the time | `ROBIGO_PERF_MAP=jitdump`, `perf inject --jit` | per op and per machine instruction (below) |
| where does the heap peak and grow | `scripts/valgrind.py massif` | `ms_print` graph |
| does robigo leak or touch bad memory | `scripts/valgrind.py memcheck` | needs the valgrind-visible build (below) |
| does this input crash the parser / `readRDS` | `scripts/fuzz.py`, `fuzz/` (cargo-fuzz) | CI runs `fuzz.py`; cargo-fuzz is the local, coverage-guided one |
| does robigo *behave* like GNU R on inputs nobody wrote yet | `scripts/fuzz-eval.py` | differential fuzzing against the 4.5.0 oracle |
| do the interpreter, the VM and the JIT agree on inputs nobody wrote yet | `scripts/fuzz-eval.py --tiers` | no oracle needed; the bucket names the tier (or the `ROBIGO_JIT_OPT` variant) that disagrees |
| why did this loop or closure (not) become native code, how often did it deopt | `ROBIGO_JIT_OPT=report robigo SCRIPT.R` | per site at exit: runs, deopts by op, compile time, the reason it was refused or given up |
| which loop or closure's native code changes what this script prints | `scripts/fuzz-eval.py --bisect FILE.R` | runs each native site alone (`only=ID`), then halves |
| does code that never runs change what robigo prints | `scripts/fuzz-eval.py --emi` | equivalence modulo inputs, no oracle needed |
| what is the smallest script that still fails this way | `scripts/reduce.py FILE.R` | delta debugging on GNU R's parse tree; `--corpus NAME --case ID` for a corpus case |
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
`MI_TRACK_VALGRIND=1` into `.robigo/bin/robigo-vg` (its own target
directory, so the shared target keeps no instrumented mimalloc and plain
`scripts/build.sh` never flip-flops); `scripts/valgrind.py` uses it for
memcheck, massif and dhat (cachegrind and callgrind keep the normal
build), and `scripts/valgrind.supp` silences mimalloc's own THP probe
and the stripped loader's wide loads. With it, memcheck sees every block:
Rc-cycle leaks show up as definitely-lost, and the leak report becomes
evidence for the memory bake-off (`experiments/memory`,
`docs/ledger/memory.md`) — run `.robigo/profiling/leaks.R` before and
after any object-model change. DHAT works against the tracked build too
(it honours mimalloc's client-requested blocks). Never measure wall time
or instruction counts with `robigo-vg`.

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

## Floors and causes

GNU R is a rival, not a target: a ratio against it says who is ahead, not
how far there is to go. Two tools measure against the work itself.

**The floor.** `scripts/idiom-vs-r.py --idioms [REGEX]` runs the idioms of
`benchmarks/idioms.toml` and reports instructions per iteration of robigo,
GNU R and the floor, with robigo÷GNU R and robigo÷floor. The floor is a
plain C version of the same work stored beside each idiom (`[idiom.c]`),
measured the same way (N and 2N iterations under callgrind) after `cc
-O2`, with its arithmetic stated (`arithmetic`): what an implementation
that knew the types and the class in advance would run. The rule this
borrows (blaze's rule 67) is to design within 10% of the floor and report
every measurement as a ratio to it. `--floors` compiles and measures the C
versions again beside the stored numbers; do that when adding an idiom
(it needs its C version and a measured floor) or changing compilers. The
set is one idiom per kind of per-iteration cost: scalar double and integer
arithmetic, a branch, a vector read and write, a whole-vector operation, a
closure call and an S3 dispatch. Floors for whole benchmark programs are
not kept: their C versions would be ports of the programs. For GNU R 4.5.0
point `RSCRIPT_BIN` at `~/.cache/robigo-research/R-4.5.0-build/bin/exec/R`
and `R_HOME_DIR` at that build; a failed R run stops the script.

**Causes.** `scripts/callgrind-diff.py --bench NAME --causes` profiles a
`scripts/perf.py` benchmark as the gate runs it, minus an empty program in
the same environment (so startup cancels), and splits the instructions
into causes: dispatch and evaluation, allocation, reference counts and GC,
guards and type checks, attribute handling, values, JIT-generated code,
JIT and bytecode compilation, builtins and numerics, libc, startup and
lazy loading, printing. The map is data, `benchmarks/causes.toml`: regexes
over callgrind's `file:function [object]` lines, first match wins. Each
row names its largest function, and each row is a work item. It also takes
two scripts (`WORK.R BASE.R UNITS --causes`) like the per-function table.
Limits: a guard inlined into the VM loop counts as the VM; the JIT's code
is one anonymous row under callgrind.

## Sampling: scripts/samply.sh

```sh
scripts/samply.sh benchmarks/suite/fib.R      # -> .robigo/samply/fib.json
samply load .robigo/samply/fib.json           # opens the Firefox Profiler
scripts/samply.sh SCRIPT.R --env R_DEFAULT_PACKAGES=utils,stats
```

Sampling sees what deterministic counters never will: time in GNU R's
BLAS, in dlopen'd package code, in the JIT's machine code, and plain
waiting. Two caveats: samply needs `perf_event_paranoid <= 1` (the script
tells you the command), and JIT frames need `ROBIGO_PERF_MAP=1`, which
the script sets — the JIT then writes `/tmp/perf-<pid>.map`
(`robigo_loop_N#tT`, `robigo_clo_N_K#tT`, `robigo_clo_entry_N#tT`, `T` the
compiling thread's kernel id), which samply and perf read;
callgrind does not, so JIT frames stay anonymous there. The profile is a
flamegraph over the *whole process*, GNU R's comparison runs included —
profile the same script on both runtimes and compare where the samples
go. A build-in flamegraph already exists: `--features profile` with
`ROBIGO_PROFILE=out.svg`.

### JIT code per VM op: perf's jitdump

`ROBIGO_PERF_MAP=jitdump` writes the map and, besides it, perf's jitdump
(`jit/perf_map.rs`): each JIT function's machine code and a line table from
its pcs to the VM ops it was built from (Cranelift source locations, one per
op). `perf inject --jit` turns them into ELF objects, so samples land on
ops and `perf annotate` shows each op above its instructions:

```sh
export JITDUMPDIR=$PWD/.robigo/jitdump        # default ~/.debug/jit
ROBIGO_PERF_MAP=jitdump R_DEFAULT_PACKAGES=NULL \
  perf record -k mono -e cycles:u --call-graph fp --user-callchains \
  -o jit.data -- .robigo/bin/robigo --vanilla SCRIPT.R
perf inject --jit -i jit.data -o jit.inj.data
perf report -i jit.inj.data --no-children -g none --sort sym,srcline --stdio
perf annotate -i jit.inj.data --stdio -s 'robigo_loop_1#t12345'
```

- Each run gets a new directory `$JITDUMPDIR/robigo-XXXXXX` with
  `jit-<pid>.dump` and one listing per function, `<name>.ops`: the VM code
  as `ROBIGO_VM_DUMP` prints it, line `pc + 1` being op `pc` and the last
  line the function's glue (entry, exits to the VM, deopts). A `srcline`
  such as `robigo_loop_1#t12345.ops:84` is op 83 of that listing. `perf
  inject` adds `jitted-<pid>-<n>.so` beside them.
- A closure inlined into a JIT function counts as the op that calls it.
  Code Cranelift makes on its own (rematerialized constants, the egraph's
  rewrites) has no source location; it sits just before its first use, so
  it counts as the next op.
- Attribution is per VM op, not per R source line: `Rscript` keeps no
  srcrefs, and the VM's code does not record them. The listing's ops name
  their variables and functions; `ROBIGO_VM_DUMP=1` prints the R
  expression each code was compiled from.
- `-k mono` is required: the dump's timestamps are `CLOCK_MONOTONIC`, and
  `perf inject` matches them against the samples'.
- The code in the dump is as Cranelift emitted it, before relocation:
  `perf annotate` shows calls of helpers with a zero target.
- samply reads jitdump too (via the same mapping); this was checked only
  with perf, since samply needs `perf_event_paranoid <= 1`.

**Call stacks through JIT code.** JIT functions keep frame pointers
(Cranelift's `preserve_frame_pointers`; without it a leaf function with no
stack use would omit them; cost within ±0.01% on the instruction gate), so
`--call-graph fp` walks every JIT frame, a recursive closure's too, back
into the Rust that called it (`vm::jit_enter`, `vm::native_site_call`).
`--call-graph dwarf` does not work there: perf's DWARF unwinder has no CFI
for JIT code and records an empty stack. The walk stops at that first
Rust frame, because Rust omits frame pointers; for whole stacks (every
JIT sample down to `robigo::real_main`), profile a build with them:

```sh
CARGO_TARGET_DIR=~/.cache/cargo-target-fp RUSTFLAGS="-C force-frame-pointers=yes" \
  cargo build --profile quick -p robigo-cli    # never for measurements
```

**WSL2 and `perf_event_paranoid = 2`** (no root): `perf record` works for
user-space events (`-e cycles:u`), and call graphs need
`--user-callchains` (without it every event fails to open); samply needs
paranoid ≤ 1 and stops there.

## The JIT's switches

`ROBIGO_JIT=0` turns native code off. Everything else is one comma-separated
token list, `ROBIGO_JIT_OPT` (`crates/robigo-runtime/src/jit/config.rs`;
after relax's `RELAX_OPT`). An unknown token stops robigo with an error, so
a misspelt switch never silently tests the default. No switch changes what
an R program prints.

| token | effect |
|---|---|
| `-loops`, `-closures` | no native loops; no closures compiled whole |
| `-summary` | no closed forms of loops (`jit/summary.rs`) |
| `-inline` | the VM never runs closure bodies inline (nor natively) |
| `eager` | compile at the first opportunity: loops at their first entry, closures at their first call, and no cost model (native code is not given up for leaving too often, small bodies compile too) |
| `sample=SEED:SHARE` | only a deterministic share of the loops and closures gets native code, by a hash of their bytecode and `SEED`: mixed VM/native runs |
| `only=ID:ID...` | only these sites (ids from the report) get native code |

The selection is of compilation roots (native loops, closures compiled
whole): a closure they run inline goes with its root, so the report's native
ids select exactly the native code of a run.
| `verify` | Cranelift's IR verifier and register-allocation checker on every compilation, in release builds too; a failure panics |
| `report` | at exit, on stderr, one line per loop and closure the JIT looked at |
| `trace` | each decision as it happens, on stderr |
| `dump` | the Cranelift IR and machine code of each compilation |

`+name` turns an optimization back on.

```sh
ROBIGO_JIT_OPT=report robigo script.R
# == robigo JIT report: 4 loops (2 native), 1 closures (1 native)
# loop    67307c9f57a18ecc never      unsupported types (at 12: Call { dst: 9, f: `g`, site: 0 }) | for (j in 1:2000) ...
# closure 2ad6cc40da90f197 native     runs 1489, bails 0, variants 1 in 0.43 ms | if (x > 1) g(x - 1) + x else 1
# loop    e13a76f066ed24e1 native     runs 1, deopts 1, compiled 1 in 0.28 ms (at Call 1) | while (i < 100) ...
```

A line is `KIND ID STATUS DETAILS | LABEL`. `native`: compiled; runs and
how many continued in the VM (loops: deopts, by the op they continued at;
closures: bails, runs abandoned and made again by the VM), compilations and
their time. `never`: refused for good, with the reason (`unsupported op`
names the op). `not-now`: refused for the values seen, by reason and count.
`gave-up`: compiled, then sent back to the VM (it left too early or too
often). `unselected`: left out by `sample=`/`only=`. The id is a hash of
the site's bytecode: the same in every run of the same program by the same
binary, whatever the switches, so `only=ID` selects it in the next run.
Sites appear only once the JIT decides something about them, so a loop
that never got hot is not listed. Off, the report costs nothing (its hooks
sit on paths that were already out of line; `docs/ledger/perf-vm.md` has
the perf gate run).

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
  ASAN_OPTIONS=detect_leaks=0 \
    cargo +nightly fuzz run rds -- -fork=16 -max_len=65536 -max_total_time=600
  ```

  Crashes land in `fuzz/artifacts/*` (`-fork` keeps the run going and
  collects them). The harnesses are deliberately minimal: `parse` feeds
  bytes through `from_utf8_lossy` into `robigo_syntax::parse_program`,
  `rds` feeds bytes into `serialize::unserialize` on a bare `Interp` — a
  malformed stream must end in an error, never a panic. LSan flags the
  runtime's exit-live heap; under `-fork` libFuzzer's own
  `-detect_leaks=0` does not silence it — set `ASAN_OPTIONS=detect_leaks=0`.
  On `rds`, the expected finding class is `unserialize`'s unguarded stream
  lengths: a mutant demanding a terabyte-scale allocation makes ASan stop
  the worker (counted as oom; in the real binary it is the same failed
  allocation `scripts/fuzz.py` hunts).

### Differential eval fuzzing: scripts/fuzz-eval.py

Generates R programs, runs each on robigo and on a reference, and compares
normalized stdout, stderr and exit status (`scripts/corpus.py`'s masking,
plus the random `R CMD SHLIB` tmpdirs), all `R_DEFAULT_PACKAGES=NULL`.
Differences are leads, not verdicts:

```sh
python3 scripts/fuzz-eval.py --seconds 300 --jobs 16          # against GNU R 4.5.0
python3 scripts/fuzz-eval.py --tiers --seconds 300 --jobs 16  # interpreter vs VM vs JIT
python3 scripts/fuzz-eval.py --emi --seconds 300 --jobs 16    # with and without dead code
python3 scripts/fuzz-eval.py --tiers --gen template --reduce 120  # then reduce each new bucket
python3 scripts/fuzz-eval.py --bisect .robigo/fuzz/eval/tier-1234abcd.R  # the native code to blame
```

Inputs (`--gen`, default `mix`):

- `ast`: a `tests/r` seed (the ratchet's, where markers exist) mutated on
  GNU R's own parse tree (`scripts/rast.py`): literals, operators, names
  and subexpressions replaced, statements dropped, duplicated or run in a
  hot loop. About 99% of mutants parse.
- `template`: programs built to break what the VM and the JIT assume
  (`scripts/fuzz_gen.py`, after Csmith): a loop past the loop JIT's 16
  iterations, or a closure past the closure JIT's 512 calls, in which a
  variable's type, length or attributes change, `<<-`, `assign()`,
  `eval()`, `rm()` or `delayedAssign()` reach the loop's frame, an integer
  overflows to NA, a shared vector is modified, or an S3 method, a callee or
  a base function is defined after warm-up.
- `bytes`: `scripts/fuzz.py`'s byte mutator. Only about 13% of its mutants
  parse, so it mostly tests the parser; 10% of `mix`.

Checks:

- Default: GNU R 4.5.0 twice (it must agree with itself, or the input is
  unstable and skipped, as in the corpora), robigo once. The oracle must be
  4.5.0; newer R disagrees for reasons that are not robigo's bugs, so the
  script refuses.
- `--tiers`: no oracle. The default tier (twice, for stability),
  `ROBIGO_JIT=0` (the VM) and `ROBIGO_COMPILE=0` (the interpreter) must
  print the same (`docs/architecture.md`). Two tiers that agree outvote the
  third, and the bucket names it: `jit`, `vm`, `interp` (or the VM and the
  JIT share a bug), or `all`. When the three agree, the JIT tier runs
  again with `--variants` (default 3) `ROBIGO_JIT_OPT` settings from
  `MATRIX` (after relax's `fuzz_tiers.rs`): `eager,verify` always, so that
  small programs reach the closure JIT (512 calls otherwise) and Cranelift
  checks every compilation; then a random pick of one optimization off,
  loops or closures alone, and seeded `sample=` selections that mix VM and
  native code. A variant that differs is blamed as `jit[OPT]` (seeds
  dropped from the bucket key), and its reproducer keeps the full setting
  in `NAME.opt`, which `scripts/reduce.py` reruns.
- `--bisect FILE.R [--jit-opt OPT]`: which loop or closure's native code
  changes what `FILE.R` prints (after rustd's `difftest.py --bisect`). The
  JIT tier (`ROBIGO_JIT_OPT=OPT,report`, default `eager`) is compared with
  the VM; each native site of the report runs alone (`only=ID`), and when
  none differs alone, halves of them do. It prints the sites and the
  `ROBIGO_JIT_OPT=...,only=...` line that reproduces the difference.
- `--emi`: no oracle (equivalence modulo inputs, Le et al., PLDI'14). The
  input and the input with blocks that never run inserted before existing
  statements (`if (FALSE && "emi") { ... }` and an opaque `if (.emi.dead)`)
  must print the same in the default tier. The dead blocks assign, `rm()`,
  `return()`, `break`, define functions and so on, which is what a compiler
  analyses. The interpreter decides first whether the insertion is neutral
  (a program that prints its own functions sees the new code).
- A robigo crash is a finding and fails the run, as in `scripts/fuzz.py`.
- Each bucket keeps one reproducer under `.robigo/fuzz/eval/`
  (`diff-`, `tier-`, `emi-` with its `.orig.R`, `crash-`), keyed by what
  differs: each differing stream with its first differing line on each
  side (`scripts/rcheck.py`), independent of where in the script.

### Reducing a failure: scripts/reduce.py

```sh
python3 scripts/reduce.py .robigo/fuzz/eval/tier-1234abcd.R    # mode from the name
python3 scripts/reduce.py case.R --mode oracle --seconds 300
python3 scripts/reduce.py --corpus fastr --case builtins/TestBuiltin_adist/a293491ce8a97832
```

Hierarchical delta debugging over GNU R's parse tree: the statements of
each block, the arguments of each call, comments, then each expression
replaced by one of its subexpressions, to a fixed point or `--seconds`.
Every candidate parses, kept source text stays byte for byte, candidates run
in parallel and are cached. A candidate is kept only if it fails the same
way: the same verdict key as the input (`--loose`: the same streams
differ). The modes are fuzz-eval's checks plus `crash`. A corpus case
(`scripts/triage.py` prints the command for each failing one) runs the way
`scripts/corpus.py` runs it, live against GNU R. The result is checked once
more with the reference run twice and lands next to the input as
`NAME.min.R`. Turn it into a `tests/r/<area>/<name>.R` with
`scripts/regen-expected.sh`, no ratchet marker until it passes
(`docs/workstreams.md`, protocol rule 8). `scripts/test-reduce.py` tests the
reducer with a robigo stand-in that fails in a known way.

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
| `perf.py` instr gate, `perf-paired.py` confirmation | `callgrind-diff.py` (causes too), `idiom-vs-r.py` (floors), `alloc-count.py`, `valgrind.py` (all tools) |
| `fuzz.py` 60 s/target, fixed seed | cargo-fuzz long runs, `fuzz-eval.py` |
| `test-reduce.py` (the reducer and its parse trees) | `reduce.py` on a lead |
| corpora with ratchets | filtered runs, `triage.py` on one case |
| Miri (`value.rs`), clippy, gctorture | the same commands, before pushing |

The gate compares instruction counts only; cachegrind, DHAT, massif,
samply and fuzz-eval are hypotheses-and-leads tools. Bring their findings
into the gate by fixing the code, not by adding them to CI.
