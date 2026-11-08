# spare-cores: what background and speculative work could save (#374)

Measured 2026-11-08 on this machine (24 cores; loads 1–7 from concurrent
agents during the wall runs — every wall number below is a min over
repetitions; instruction numbers are exact callgrind counts). Binary:
`scripts/build.sh` at main 2a5ff0b3 (`.robigo/bin/robigo`), GNU-R-fidelity
env of `scripts/perf.py` (`R_DEFAULT_PACKAGES=utils,stats`, C.UTF-8, TZ=UTC;
host R supplies `R_HOME` only). Feeds #375/#376 (background/ahead-of-demand
JIT), #377 (cross-run code cache), #378 (parallel elementwise kernels),
#379/#380 (pending/fused vector results).

## Summary: measured ceiling per design issue

| issue | idea | measured ceiling (this suite) | verdict |
|---|---|---|---|
| #375+#376 | compile on a background thread / ahead of demand | 8.0–18.0% of instructions on the 4 loop-compiled programs; single synchronous stalls of 0.7–6.7 ms per hot loop; ~0% on the other 17 | do first; biggest measured pain |
| #378 | parallel elementwise kernels | 87.9% of `vecarith` is pure elementwise work → up to **6.4x** there (24 cores, Amdahl); ≤12% and ≤1.08x on the other vector-ish programs | do second; huge on vector chains, rare elsewhere |
| #377 | cache compiled code across runs | direct: ≤0.05% on 17 of 21 programs, 8–18% only where #375 already hides it; suite-wide geomean ≪1% | only as the enabler of lower thresholds, after #375/#376 |
| #379 | evaluate promises early on another core | promise-forcing machinery is **0.08–1.5%** of instructions everywhere measured | park; nothing to save |
| #380 | fused/pending vector results | not measured (allocation/copy split of the kernels not attributed) | park until measured |

Recommendation order: **#375/#376 → #378 → #377 → #379/#380**.

One number that is *not* compile cost: `fannkuch` pays 13.4% of its
instructions in `vm::jit_enter` self cost (per-entry native dispatch over
~108k tiny loop entries) on top of 2.9pp of compilation. A background
compiler cannot remove that; entry gating can.

## Q1 — JIT compile cost today (#375, #376)

**Method.** Count compiles: `ROBIGO_JIT_TRACE=1 .robigo/bin/robigo
--vanilla FILE 2>&1 | grep -c 'jit: compiled loop'` (closure trees:
`grep -c ': compiled$'`). Attribute cost: one callgrind run per program
(`valgrind --tool=callgrind --callgrind-out-file=X.cg .robigo/bin/robigo
--vanilla FILE` in the env above), then `callgrind_annotate
--inclusive=yes --threshold=100 --auto=no`: inclusive cost of
`robigo_runtime::jit::compile` (loops) + `robigo_runtime::jit::closure::new_entry`
(trees), self cost of `vm::jit_enter` reported separately (it includes
execution-path work, not compilation). Startup baseline: `-e 'cat(1+1)'`
= 191,516,450 instructions; all shares include it, as `perf.py` does.
Wall cost per compile: N=40 textual copies of each hot function
(`eval(parse(text = paste0(fn, k, " <- ", deparse(fn))))`, each copy has its
own `Code` and compiles its own loops; verified by trace), timed in-process
once vs twice (`proc.time` around `for (k in 1:N) invisible(funs[[k]](param))`);
per-compile = one/40 − (two−one)/40.

**Numbers** (21 programs: 16 from `benchmarks/suite`, 5 rbench drivers).

| program | loops/trees compiled | compile instr | % of total | +`jit_enter` self |
|---|---|---|---|---|
| nbody | 1 loop | 55.4 M | **18.0%** | +0.01% |
| fannkuch | 2 loops | 17.8 M | 2.9% | **+13.4%** |
| spectralnorm | 4 loops | 23.5 M | **9.4%** | +0.3% |
| mandelbrot | 2 loops | 18.1 M | **7.8%** | +0.02% |
| fib | 1 tree | 12.7 k | 0.004% | — |
| apply | 2 trees | 12.1 k | 0.002% | — |
| binarytrees | 2 trees | 27.8 k | 0.008% | — |
| queens | 1 tree | 58.6 k | 0.02% | — |
| all other 13 | 0 (base load: 50 k) | 50 k | ≤0.03% | — |

Per-compile wall cost (copies method, min of 3, and the consistent
instruction rate it implies for Cranelift's scalar passes):

| program | per-copy compile (2 loops except nbody 1) | per loop | instr/loop | instr/s |
|---|---|---|---|---|
| nbody | 6.7 ms | 6.7 ms | 55.4 M | 8.3 G/s |
| mandelbrot | 2.4 ms | 1.2 ms | 9.2 M | 7.7 G/s |
| fannkuch | 2.5 ms | 1.2 ms | 8.9 M | 7.4 G/s |
| fib (tree) | <0.1 ms | ~13 k instr ≈ 2 µs | — | — |

Single-shot first-vs-second call (`t0;f();t1;f();t2` scripts, param 120)
shows the same stall: mandelbrot first 4–7 ms vs second 1–2 ms; nbody
first 9 ms vs second 2–3 ms.

Where it stalls: synchronously inside `vm::jit_enter` at the 16th back
edge of a hot loop (`JIT_THRESHOLD = 16`, `vm.rs:4044`) or at the 513th
call of a body (`THRESHOLD = 512`, `jit/closure.rs:63`). One stall per
loop. Inside one compile, regalloc2 is 52% of the instructions (nbody:
28.4 M of 55.4 M), `Context::compile` 45.7 M, i.e. the cost is codegen,
not robigo's analysis (which is the ~1 M remainder).

**Implication.** Background compilation (#375) hides 0.7–6.7 ms single
stalls; compile-ahead (#376) additionally removes the 16-iteration/512-call
warmup the thresholds impose. Both only matter for loop-heavy programs
(4 of 21 here), but there the prize is 8–18% of the whole run. Tree
(closure) compiles are 3–4 orders of magnitude cheaper than loop compiles
and never need a background thread.

## Q2 — cross-run cache potential (#377)

**Method.** From Q1's callgrind table: compile share of each program's
total (startup included, the short-script regime — the suite programs run
0.05–0.5 s). Code reading for what a cache must hold: `jit.rs`
(`JitSlot::Ready` hangs off `Code`), `jit/closure.rs` (`Entry`,
`Variant { args, cells }`, guards), `vm/rjit.rs` (`JitState` frozen sets),
`env.rs` (`search_epoch`, `r_epoch`, `fn_version`).

**Numbers.** For a short script that runs once, compilation is 8–18% of
the run on nbody/fannkuch/spectralnorm/mandelbrot and ≤0.05% on the other
17 — including every real program measured (csv, lm, dataframe,
flexclust_kcca) and fib/apply/vecarith/vecops. Base loading itself
compiles only 50.1 k instructions (0.026% of the 191.5 M startup): nothing
to pre-cache there. So a perfect cache saves ~0% suite-wide (geomean ≪ 1%),
and saves 8–18% only on the four programs where a background thread (#375)
already hides the same cost.

What would need persisting, and the invalidation problem:

- **Bytecode** (`Code`: ops, vars, consts, call sites) — content-hashable,
  robust. Worth it mainly as the cache *key*.
- **Guard facts**: the guarded function symbols (must still resolve to
  base), observed entry types (`JType` variants, closure-tree
  `Variant { args, cells }`), and `rjit`'s frozen-set decisions. Today
  these are memoized against **session-local counters** — `search_epoch`,
  `r_epoch` (bumped on any evaluation), per-env `fn_version` — all of which
  start at 0 in a new session, so none can key a persisted cache. A cache
  must key on content (body hash + base-package identity/version) and
  re-validate guards by ordinary lookup on first use; the existing
  per-epoch memoization already re-memoizes cheaply, so this is a
  re-validation cost of a few lookups per entry, not a correctness hazard.
- **Native code**: embeds robigo's internal ABI (`JType` tags, `Reg`/`Slot`
  layout, the `HELPERS` table) and is Cranelift-registered memory, not
  relocatable artifacts; persisting machine code would need per-build +
  per-arch versioning and buys only the ~8 G instr/s codegen pass.

**Implication.** Don't build #377 for the direct saving (≈0% on short
scripts). Build it, if at all, as the *enabler* for #376's lower
thresholds: with bytecode + guard facts cached, a script's hot loops could
be compiled on the background thread during startup, before first
execution, making thresholds 0 without paying codegen per run. Persist
facts, not machine code.

## Q3 — promise force latency (#379)

**Method.** Same callgrind runs; self cost of everything matching
`force_promise*` (`Interp::force_promise{,_object,_raw}`),
`interp::plain_value`, `vm::caller_promargs` (the issue's three names),
as a share of total instructions.

**Numbers.**

| program | promise machinery | % of total |
|---|---|---|
| flexclust_kcca (real) | 458.1 M | **1.46%** |
| closures | 11.1 M | 1.25% |
| mandelbrot | 3.14 M | 1.35% |
| fib | 3.14 M | 0.87% |
| apply / lapply / lapplyDots | 3.15–3.32 M | 0.5–1.2% |
| vecarith | 3.14 M | 0.08% |

3.14 M of every row is the constant paid during base loading; the
program-marginal share is ≤1.5% everywhere, and ≈0 for the lapply family
(robigo's `lapply` fast paths pass values, not promises). The per-force
instruction cost and the creation→force gap histogram are **not measured**
(callgrind gives no call counts on this host; the gap needs the opt-in
counter #374 asks for).

**Implication.** The forcing machinery itself is ≤1.5% — there is no
measured saving for a background/early forcer to capture on these
programs. Any #379 win must come from overlapping the *body* evaluation of
promises that stay unforced a long time, and the issue's own guess (most
arguments forced within nanoseconds at entry) is consistent with what the
shares show. Park #379 until the gap histogram exists.

## Q4 — large-vector elementwise share and slack (#378, #380)

**Method.** Same callgrind runs; self cost of `builtins::arith::`
(arith/rel/Math-group kernels: `real_binary*`, `do_exp`, `do_log1p`,
`integer_binary`, `relop`, …), `builtins::logic::`, plus the libm leaves
those kernels call (`exp`, `log1p`, `sin`, … in libm.so). Instruction
share is the wall-share proxy (the kernels are vectorized native code).

**Numbers.**

| program | elementwise instr | % of total | Amdahl ceiling @24 cores |
|---|---|---|---|
| vecarith (2e6-elem chains) | 3,533 M | **87.9%** | **6.4x** |
| vecops | 277 M | 11.7% | 1.07x |
| lm | 50.1 M | 7.7% | 1.05x |
| dataframe | 34.1 M | 5.7% | 1.04x |
| matrix | 29.9 M | 4.6% | 1.03x |
| csv | 34.1 M | 4.0% | 1.03x |
| flexclust_kcca | 1,024 M | 3.3% | 1.02x |
| apply | 12.1 M | 2.3% | 1.01x |

vecarith splits 1.35 G kernel loops + 2.18 G libm (`log1p` 36.4%,
`exp` 17.9% of the whole program). The taskset pinning experiment
(1 core vs all cores) is **not measured** — the runtime is single-threaded
and 23 of 24 cores are idle while these run, so the slack is the cores;
the ceiling above assumes perfect parallelization of the elementwise share
with bit-identical exact mode. The allocation/copy share of the kernels
(what fusing temporaries would save, #380) is **not measured**.

**Implication.** #378 is a 6.4x ceiling on vector-chain workloads like
vecarith — but those are the exception: every other program measured has
≤12% elementwise share (≤1.07x). Elementwise parallelism pays only on
long-vector chains; the workload detector should be "result length",
matching the issue's 10⁵-element instinct. #380 stays unmeasured; measure
the kernels' malloc/memcpy share before designing it.

## Reproduce

```sh
scripts/build.sh                     # once; binary .robigo/bin/robigo
ENV="env LANGUAGE=en LC_ALL=C.UTF-8 TZ=UTC R_ENVIRON_USER= R_PROFILE_USER= \
     R_DEFAULT_PACKAGES=utils,stats R_HOME=$(R RHOME)"
# instruction totals + attribution (per program):
$ENV valgrind --tool=callgrind --callgrind-out-file=X.cg \
     .robigo/bin/robigo --vanilla benchmarks/suite/X.R
callgrind_annotate --threshold=100 --auto=no X.cg            # self costs
callgrind_annotate --inclusive=yes --threshold=100 --auto=no X.cg
# baseline startup: -e 'cat(1+1)' -> 191,516,450 instructions
# compile events:  ROBIGO_JIT_TRACE=1 ... 2>&1 | grep -c 'jit: compiled loop'
# per-compile wall: N=40 deparse/parse copies, timed once vs twice
#   (pattern in this file, Q1 "Method"); min of 3, idle-ish machine.
```

## Not measured

- Per-force instruction cost and the creation→force gap histogram (#379's
  real question) — needs the opt-in counter of #374 or callgrind call
  counts; neither available in this pass.
- taskset 1-core vs all-cores wall deltas (slack is inferred from the
  single-threaded runtime + idle cores, not measured directly).
- Wall-time share of kernels separate from instruction share; the
  allocation/memcpy share of the kernels (#380's fusion bound).
- Wall numbers were taken under concurrent-agent load (min-of-N);
  instruction numbers are unaffected.
