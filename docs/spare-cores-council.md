# Council review of the spare-cores designs (#373–#381) (2026-11-08)

Reviewed: issues #373–#381 as of 2026-11-08 against main @ 2a5ff0b3, with
`docs/design.md` §4.2/§5.5/§5.6, `docs/architecture.md`, `docs/core.md`,
`docs/ledger/perf-vm.md` (rounds 19–22), `docs/ledger/perf-numeric-loops.md`
and `docs/ledger/perf-vector-loops.md`. The measurements of #374 are being
gathered in parallel (`docs/ledger/spare-cores.md`, PR-linked from here once
open); every review reasons without them and states what #374 must confirm.

Council: GPT-6 Astra, Claude Opus 5.5, Claude Fable 5.1, Gemini 4 Argon.
Full reviews follow the synthesis. Status of each item: the list below, and
the issues themselves.

Verdict table (all four reviewers; ceilings are conditional, awaiting #374):

| issue | verdict | ceiling (assumption) |
|---|---|---|
| #374 measurements | build now, first; extended list below | gates everything |
| #378 parallel elementwise kernels | build now on today's runtime, first code track | memory-bound 1.5–3x, compute-bound ~0.8x/core-count per kernel; whole programs Amdahl-limited |
| #380 L1 fusion, one expression | build now, strip-mined existing kernels first | ~2x on memory-bound chains (7 memory streams → 3); 1.1–1.3x cache-resident |
| #375 background JIT | build on today's runtime, gated on #374 (compile ≥3–5% of wall somewhere real) | ≤ the compile share; for loops only compile × (VM/native ratio) |
| #376 compile ahead | after #375's queue/usefulness data | 1–5% on call-tree/polymorphic workloads |
| #377 cross-run code cache | reject as written; fold into G6's startup snapshot | ~0 once #375 lands (startup is 73 ms, mostly not JIT) |
| #379 pending vectors | reject standalone; becomes #380 L2 on robigo-core, warning-free ops only | ≤ min(op time, slack); expected ~2% |
| #381 parallel GC mark | robigo-core only, after the barrier/sticky-mark decision (core-council item 8) | GC share × ~0.6; 3–5x shorter mark pauses on wide heaps |

## Unanimous (all four) — these are binding

1. **#374 first, extended.** Beyond the issue's three measurements:
   compile time split four ways (analysis, CLIF build, `define_function`,
   finalize) for loops and closures separately, with each unit's VM-to-native
   speed ratio and the count of compiles never entered or abandoned;
   elementwise time by op class × result-length bucket (<1e3, 1e3–1e5, >1e5),
   with the per-op per-element cost at 1e4–1e7 setting each class's threshold;
   N-thread/1-thread STREAM ratio and futex wake latency on each target
   machine; first-touch page-fault share of large vector ops; the share of
   elementwise time on fresh temporaries (fusion) and the ops between a large
   op's return and its first payload read (#379's slack); a promise-force
   latency histogram.
2. **#378 now, in a `robigo-kernels` crate** that cannot depend on
   `robigo-runtime` (the only enforced "workers never touch the runtime"
   boundary; Rust's `Send` does not catch a `thread_local!` read). Same
   monomorphized kernel in sequential and chunked paths; per-class length
   thresholds, never one global threshold; an owned ~200-line pool (lazy,
   pid-checked after `fork()`, sized from `available_parallelism`, no rayon,
   no `Arc` refcounts, no worker that can see a `Value`/`Sym`/interpreter);
   the main thread's FP environment (x87 control word, MXCSR) copied to
   workers and asserted in debug; `x_is_dest` computed from whole-vector
   lengths before chunking, never per slice; recycling cursors are absolute
   (`i % m`), never chunk-relative; flag-type warnings ("NaNs produced",
   "NAs produced by integer overflow") OR-reduced after the join; **per-element
   warnings are a sequence**: `%%`'s accuracy warning is a count, and the
   nmath group's thread-local warning queues must be drained per chunk and
   replayed in strict chunk order (`arith.rs:385` already counts and replays
   sequentially); interrupts via a cancel flag checked per chunk, raised after
   the join; `ROBIGO_THREADS=1` stays the instruction-count gate, plus a
   `ROBIGO_THREADS=4 ROBIGO_PAR_MIN=1` conformance tier so small vectors
   exercise the parallel path, plus property tests comparing threads=1 vs N
   bit-for-bit over all 169×169 special-value pairs placed at chunk
   boundaries (the `pair_cmul_matches_cmul` method).
3. **#380 Level 1 now, with the narrowed contract.** Fuse only within one
   expression; every operand is an already-evaluated variable or constant
   (the inliner's leaf rule — GNU R evaluates `y` after `sqrt(x)` warns, so
   fusing call operands reorders observable events); attribute-free
   operands only; no object operands (S3/S4 group dispatch falls back); only
   flag-type ops in the fused kernel (`+ - * / ^`, comparisons, logic,
   `abs sqrt exp log log1p floor`); `%%`, `gamma`, `choose`, `beta` stay out;
   base guards per op as today. Warnings are a **transaction**: run the fused
   loop, and if any op's flag is set and an observer could run R code
   (calling handler, `warn=2`, `warning.expression`), discard and replay
   unfused in the VM — under `warn=0`/`warn=1` with no handlers, replaying the
   flags in op order is byte-identical. Buffer reuse only when no non-final
   op can warn. **Strip-mining first** (existing kernels on ~2048-element
   blocks into an L1 scratch buffer), no Cranelift kernel until strip-mining
   leaves >15% on the table against the bandwidth bound. Per-node NaN
   destination order preserved (the loop JIT already does this; keep the
   pinning test).
4. **#375's protocol, whichever split point**: the evaluator (main thread)
   resolves dependencies and lowers a complete plain-data snapshot (ops,
   constants by value, `Sym` ids, types/nonna/liveness as `jit_enter` derives
   them, inline targets resolved by **content**, not pointer); the worker owns
   its own `JITModule` (never move the thread-local one) and returns code
   pointers; installs happen only on the main thread at existing checkpoints,
   keyed by `Entry` identity/content, never raw addresses; requests hold the
   `Rc<Code>`s alive or the result is discarded; a stale-on-arrival result is
   dropped without feeding `give_up` (max ~2 retries); a guard change in
   flight needs no cancellation — the code is simply never entered; entry
   guards still re-check everything at every entry. `clo.jit` settlement
   (`jit_check`) never happens speculatively at request time.
5. **`ROBIGO_JIT_DELAY=n` before any thread exists**: install at the n-th
   entry after the request, single-threaded; conformance tiers run at
   n ∈ {0, 1, 7, 100}. Async-mode bugs are install-point bugs; this makes them
   reproducible without threads. The ledger's four entry/leave bugs (stale
   vector register, visibility leak, sol-loop's four, `entry_nonna`) are the
   reason this is not optional.
6. **Defer #376** until #375 reports how much VM-before-tier-up time remains
   and how many compiles are wasted. Never compile unobserved type variants.
   Eviction is impossible per function (cranelift-jit 0.137 frees whole
   modules): one module per generation, freed where no native frame is live.
   Native entry stays gated by `clo.jit`'s GNU-R settlement even when compile
   is ahead of demand (tracebacks and `sys.calls` observe it).
7. **#377 folded into G6's startup snapshot** (bytecode/`Code` snapshot, not
   machine code). The issue's cache key is wrong for this runtime: `Sym` ids
   shift with interning order, slot indices follow them, and the frozen base
   set is decided per closure instance at GNU R's compile moment. A cache
   would have to be keyed and relocated by names.
8. **#379 rejected on today's runtime**: hundreds of `&x.data` readers and
   the `strong_count == 1` in-place paths mean no central choke point, and
   merely holding an input in a pending op raises its `Rc` count so
   `Rc::make_mut` copies. On robigo-core it is #380 Level 2: an ALTREP-class
   node whose `DATAPTR` waits, **restricted to ops with no data-dependent
   warning or error** (`t <- sqrt(x)` must warn at that statement — Argon's
   `sqrt(-1) + {cat("rhs\n"); 1}` under `warn=2`).
9. **#381 robigo-core only, design now, code after the barrier/sticky-mark
   decision** (core-council item 8) fixes the header/bitmap layout it writes.
   Roots (conservative scan, `R_PPStack`, `root_fns` callbacks) stay on the
   main thread; the weak-ref fixpoint runs interleaved with each parallel
   round, finalization classification sequential; large pointer arrays are
   chunked onto work-stealing queues (a 2 GB STRSXP scanned by one worker
   starves the rest); `host_trace` proxies are buffered to the main thread
   until G3; `Block::marked` is not maintained during marking at all —
   derived from `count_ones()` at sweep (also removes a write from the
   sequential path); parallelize above ~16 MB live. This needs a
   collector-only exception to #373's "workers never touch objects" rule,
   stated explicitly.
10. **Never**: user R code on another thread in any form (early promises,
    speculation with rollback, parallel `lapply` of "pure" closures — the
    exception Opus sketches for scalar-only native call trees is a later
    candidate, not this track); reductions split within one output in exact
    mode; reassociation, FMA contraction, approximate vector math; observable
    output/condition reordering; blanket `Send`/`Sync` for `Value`/`Sexp`;
    global pool libraries; `nproc`-sized pools (affinity and cgroup quota
    matter; one thread in forked children); wall-time gates on CI runners
    (callgrind sums worker threads; `THREADS=1` + sync mode stay the gate).

## Majority (three of four)

11. **#375 build on today's runtime, but after #374 clears its gate**:
    compile ≥3% of wall on ≥5 suite programs or ≥5% median on package-test
    scripts (Opus), or the equivalent per Fable's formula (loop compiles
    weighted by the VM/native ratio). Astra would start after the same
    measurement; Argon gates at ≥5% on ≥3 programs. The backend share
    (lowering+regalloc+emit ≥60% of compile time) decides whether the cut is
    worth it: analysis and CLIF building stay on the main thread, so only the
    backend moves.
12. **#378's first milestone**: double `+ - * /` without attributes
    (`real_binary`/`real_binary_inplace`) and the cheap Math1 set
    (`sqrt exp log log1p abs`), chunked, with the unit tests of item 2 and
    `vecarith` wall −30% on 4 cores under the round-19 paired protocol.
    Compute-bound kernels (`exp` at n ≥ 1e5) are where cores pay;
    memory-bound ones stop at bandwidth.

## Split — decided by the lead

13. **`colSums`/`rowSums` in exact mode.** Argon and Fable lump them with
    reductions; Opus shows they are exact to parallelize *across output
    elements* (each column/row accumulates its inputs in the same sequential
    order, `array.c`'s `LDOUBLE` loop; the split does not reorder any
    accumulation). **Lead decision: Opus's reading is correct.** Splitting
    within one output stays forbidden; splitting across outputs is allowed
    once pinned by a bit-identity test (the 169×169 boundary method). Same
    reasoning admits stable `order`/`sort` (unique permutation) and `match`'s
    probe phase later.
14. **#380 L1 shape**: strip-mining first (Opus, Fable), Cranelift kernel
    only if the >15% criterion holds (item 3). Argon's `Op::VecFused`
    bytecode descriptor is compatible and is how the tree reaches the
    kernels.
15. **#381 sequencing**: design now; prototype against the standalone
    `robigo-core` heap benchmarks when convenient; land after G3 and the
    generational/barrier decision. Opus adds that sticky-mark generational
    marking removes more pause per unit of work than parallel marking —
    do that first.

## Corrections to the issues (attributed, for whoever picks them up)

- **#375** "make the compile input `Send`" understates the problem:
  `call_tree` needs `&mut Interp` and `Code` carries `Cell`/`RefCell` memos
  the main thread writes during evaluation (Opus §3.1, Fable, Astra item
  #375). "Lower thresholds are cheap" is wrong: `JIT_HEAT`'s comment records
  `nbody` +25% at threshold 16 (Opus). On two cores the compiler competes
  with useful work (Astra).
- **#376**: per-function eviction does not exist in cranelift-jit 0.137
  (Opus); entering native code ahead of GNU R's compile moment is observable
  in tracebacks/`sys.calls` (Argon, citing perf-vm round 10); startup
  precompilation of base functions is rejected (Opus).
- **#377**: the key omits `Sym`-id/interning-order shifts, slot indices and
  the per-instance frozen set (Fable §5.3, Opus, Astra); "numeric mode is
  already in the key" overstates architecture.md (Astra).
- **#378**: "warnings raised once from a flag" is false for `%%` (one
  `warning()` per element, `arithmetic.c:185`), `gamma`/`lgamma`
  (`gamma.c:152-213` via nmath.h:70), `beta`, `choose` (Fable §5.1, Argon §5.3);
  the nmath queue is a `thread_local!` workers would silently lose (Argon);
  `cum*` are sequential; `ifelse` is R code — the kernel is logical-mask
  `[<-`; char→numeric coercion touches `Rc<str>` (`!Sync`) and warns
  per element — only numeric-to-numeric coercion qualifies (Opus §5.4);
  `x_is_dest` depends on whole lengths and int-operand, and a length-1 tail
  chunk would flip it (Argon §5.4).
- **#379**: "peak memory is unchanged" is false generally; waiting in
  `REAL()` is insufficient (previously issued C pointers can mutate inputs);
  the premise of a payload-read choke point is false on today's `Value`
  (Argon §4, Fable §5.8, Opus §5.5).
- **#380**: the 800 MB-per-intermediate RSS claim ignores that robigo already
  writes into unshared operand buffers (`arith.rs:506`) — the real gain is
  memory passes, and the claim partially returns on robigo-core where dead
  temporaries live until a GC (Opus §5.6); "per-op flags emitted later
  preserve warning behavior" is false in general (Argon's counterexample);
  L2 must be warning-free only (all four).
- **#381**: `mark_obj` also increments `Block::marked` non-atomically and
  updates `marked_chars` and `host_trace` — races the issue misses (Argon
  §5.5, Opus §5.7); the weak-ref fixpoint is interleaved with marking rounds,
  not a sequential post-step (Opus §5.7); heaps dominated by chains and a few
  huge STRSXPs need array splitting to parallelize at all (Opus, Fable);
  R's own base packages and the collector exception to #373's rules must be
  written down (Astra).
- **#373**: pool sizing from `nproc` is wrong (Opus, Argon); fork safety is
  more than a PID check — quiesce workers, never touch the parent's pool
  locks in the child (Astra, Argon, Opus); code memory and the compiler
  thread are per-isolate when core.md A.2 isolates arrive (Fable §5.6);
  callgrind runs threads one at a time — another reason the instruction gate
  stays `THREADS=1` (Opus §5.8).
- **Docs**: `docs/architecture.md` still promises loading GNU R binaries
  unmodified; `docs/core.md` §1 and the lead's source-recompilation decision
  supersede it — stale prose to fix in a separate small PR (Astra §4).

## Lead decisions (Max, adopting the council)

- Build order: #374 lands first; #378 and #380 L1 may start immediately in
  parallel (different files, different owners); #375 starts only when item
  11's gate clears; #376 after #375's report; #377 into G6; #379 = #380 L2 on
  core; #381 per item 15.
- The restriction lists in items 2–5 are PR gates, not advice: a PR that
  parallelizes `%%`, fuses a call operand, installs from the worker thread,
  or lands async install without the deterministic delay tier is rejected on
  review.
- `robigo-kernels` is a new crate in the workspace with the same no-runtime
  dependency rule as `robigo-numerics`; the pool lives there.
- The stale binary-compatibility prose in `docs/architecture.md` gets a
  follow-up fix referencing the source-recompilation decision; not in this
  council's PR.
- The lead notes: the council endorses these designs; implementation tracks
  wait for `docs/ledger/spare-cores.md` to confirm the ceilings, per item 1.

## After the measurements (`docs/ledger/spare-cores.md`, PR #494)

The measurements landed on 2026-11-08 (callgrind over 21 suite/rbench
programs at main 2a5ff0b3, plus an N-copies wall microbenchmark). Where they
change the picture above:

- **#375's gate clears.** Loop compiles cost 5.9M–55.4M instructions
  (0.7–6.7 ms) each, paid synchronously at the 16th back edge: nbody 18.0%,
  spectralnorm 9.4%, mandelbrot 7.8%, fannkuch 2.9% of total instructions
  (fannkuch pays another 13.4% in `jit_enter` overhead — entry cost, not
  compile cost, which M2's install-at-checkpoint design also hides).
  regalloc2 is 52% of a compile, so the backend share criterion holds too.
  Closure-tree compiles are 6–59k instructions (~2 µs) — background
  compilation is about loops, as the gate's loop-first milestone assumed.
- **#378's gate is weak on the suite.** vecarith is 87.9% pure elementwise
  (6.4x Amdahl ceiling on this machine's idle cores), but every other
  measured program is ≤12% elementwise (≤1.07x whole-program ceiling). Only
  one workload clears Opus's "≥10% of wall on ≥3 realistic workloads" bar,
  so before kernels are written, the measurement must extend to real
  workloads the suite does not cover (package-test scripts, data-frame
  pipelines) — the suite may simply under-represent large-vector code.
- **#379: park, confirmed** — forcing machinery is 0.08–1.5% of instructions
  everywhere, and the lapply family pays ≈0 marginal (its fast path passes
  values, not promises). The per-force cost histogram remains unmeasured but
  the order of magnitude settles the question.
- **#377: into G6, confirmed** — compile is ≈0% on 17 of 21 short programs;
  if persistence is ever built, it is bytecode plus guard facts keyed on
  content (session-local epochs are not identities), as an enabler of lower
  thresholds, not a wall-time win.
- **#380 L1's traffic bound is still unmeasured** (the fusion share of
  elementwise time); its gate stays open.
- **Reconciled order after measurement:** #375 (M1 synchronous refactor,
  then M2) and #378 proceed in parallel — #375's gate is met on the programs
  that matter, #378's continues against real-workload numbers; #380 L1 waits
  for its traffic measurement; #376 follows #375's report; #379 and #377 per
  items 8 and 7.


---

*GPT-6 Astra council review of #373–#381 — full text below.*

The recommendation is to proceed with **#375 background compilation and #378
synchronous parallel kernels**, subject to #374's measurements. Make
restricted, within-expression fusion from #380 the third candidate. Defer
speculative compilation until background compilation exposes its remaining
misses; keep pending values and parallel GC on robigo-core.

The architectural boundary is sound: one thread executes R, while workers
compile immutable descriptions or compute over explicitly owned buffers.
Several issues, however, mistake unchanged arithmetic or warning order for
unchanged R semantics.

*Review by GPT-6 Astra. Read-only; no files changed. Estimates below are
conditional ceilings, not measurements from #374.*

**1. Verdicts and wall-time bounds**

For a fraction f of execution accelerated by p, the ideal whole-program
speedup is 1/(1−f+f/p). Queueing, memory bandwidth, additional VM execution
and contention reduce it. The examples below describe separate opportunities;
their gains cannot simply be added.

| Issue | Verdict | Conditional wall-time ceiling | Semantic / guard risk | Engineering size |
|---|---|---|---|---|
| #373 — track | Adopt, with boundary corrections below. | No independent gain; depends on eligible work and available resources. | Low if R execution remains sequential; high if "workers never touch objects" is silently relaxed. | Medium shared scheduling/lifecycle infrastructure. |
| #374 — measurements | Build now. Extend the proposed measurements. | No direct speedup; establishes ceilings for every other item. | Low; instrumentation must not distort the windows being measured. | Small–medium. |
| #375 — background JIT | Build on today's runtime, after measurement. | At unchanged compilation decisions, hiding a 10% compilation share gives at most 1.11×; 25% gives 1.33×, before delayed native entry costs. | Low arithmetic risk; high ownership and guard-publication risk. | Large: separate runtime analysis, compiler input, executable ownership and installation. |
| #376 — compile ahead | Defer until #375 supplies queue and usefulness data; then feasible today. | If earlier native entry eliminates 15% of residual runtime, ceiling 1.18×. Compilation time alone does not bound this benefit. | Medium semantic risk from accidentally resolving/forcing R code; high dependency and invalidation risk. | Medium incremental to #375. |
| #377 — persistent native cache | Defer implementation; design relocations with #375. | Removing a residual 5% compile cost gives 1.053×, before loading. Earlier native entry can add a separate benefit. | High stale-code, relocation and cross-run identity risk. | Large. |
| #378 — parallel elementwise kernels | Build a narrow whitelist on today's runtime. | If eligible kernels are 40% of time and accelerate 4×, whole-program ceiling 1.43×. Cheap arithmetic will often saturate memory bandwidth first. | Medium numeric/condition risk; low guard risk when builtin dispatch is completed on the evaluator. | Medium for the first kernels; large for the proposed full list. |
| #379 — pending vectors | Defer; implement only on robigo-core if substantial slack survives #378. | Per operation, additional saving is at most min(kernel time, independent slack), less overhead. A 1 ms kernel with 50 µs slack saves at most 50 µs. | Very high ownership, effects and C-access risk; high risk to JIT vector-view assumptions. | Very large. |
| #380 — fusion | Build restricted synchronous expression fusion today if measured; defer general lazy, cross-statement fusion to core. | For `exp(x)*2+y`, ideal payload traffic falls from roughly 56 to 24 bytes/element, assuming allocated intermediates: 2.33× traffic ceiling, not necessarily runtime speedup. Expensive `exp` can dominate. | High without effect restrictions; medium guard risk with existing binding checks. | Medium–large restricted; very large general lazy version. |
| #381 — parallel marking | Build only on robigo-core, initially as a gated prototype. Runtime benefit follows G3. | If GC is 20% of runtime, marking 80% of GC, and marking accelerates 4×: 1.136× overall; GC pause becomes 2.5× shorter if its other phases remain unchanged. | High collector correctness risk; low direct guard risk, but JIT roots are critical. | Large. |

Engineering size describes the proof and implementation surface, not an
argument against a faster design.

**2. Recommended next tracks and gates**

*First: #374 → #375, starting with a single existing loop-compilation path.*

The first milestone should preserve current thresholds and native-entry
checks, compile an immutable snapshot on one worker, and install only at the
existing evaluator boundary. Include stale-result rejection and executable
lifetime management before tuning thresholds.

#374 must record more than compilation percentage: snapshot construction,
queue delay, compilation, installation and first successful native entry
separately; remaining executions after a compilation request, and
VM-versus-native cost during that interval; compile requests that become
obsolete, functions that finish before compilation completes, and variants
compiled but never used; cold process, first invocation, repeated invocation
and warmed steady state separately. The acceptance condition is positive net
saved wall time after extra VM execution and contention. "Compilation
consumed 10%" is insufficient. This follows the evidence in
`docs/ledger/perf-vm.md`, whole-closure round 19: instruction parity
concealed 40% indirect-branch misprediction and a wall-time loss. Round 22
also explicitly says the once-per-benchmark CI gate missed the
repeated-closure improvement. Background compilation needs both measurement
regimes.

*Second: #378, beginning with one plain-double arithmetic kernel and one
expensive, audited Math kernel.*

First milestone: synchronous entry/join, no object access on workers, no
callbacks, exact scalar-kernel behavior, and evaluator-thread condition
delivery. Include equal lengths, scalar recycling and general recycling
before broadening coverage. #374 must establish: eligible time after
excluding dispatch, ALTREP callbacks and unsupported storage; serial kernel
time by length, type, shape and operation; pool handoff/join costs,
attainable memory bandwidth and scaling on 2 and at least 8 cores; whether
spare cores actually exist after accounting for BLAS and other sessions. If
handoff costs H, serial work K, and effective speedup p, require
K(1−1/p) > H, with margin. The issue's 10^5 threshold is a hypothesis, not a
policy.

*Third: restricted #380, contingent on temporary-traffic measurements.*

First milestone: one guarded expression over already available,
attribute-free numeric operands, with no callbacks or condition boundaries
crossed, no reassociation and no FMA contraction. It should compose with
#378 but first demonstrate a serial benefit. Measure allocation bytes,
existing buffer reuse, bytes transferred and time in eligible expression
chains. `perf-vm.md` already records a multiply-add fusion attempt and its
later rejection on payload correctness; establish what current code actually
does before claiming eliminated temporaries. Choose #376 instead if #375's
measurements show that missed native-entry windows dominate and eligible
fusion traffic is small. Do not prioritize known-hot startup guesses without
measured use.

**3. Hardest design problems**

*#374: measuring opportunity without manufacturing it.* Per-promise
timestamping can cost more than the force latency. Use sampling or
low-overhead counters, report instrumentation overhead against an
uninstrumented build, and distinguish a payload read from metadata access.
For #379, first-read latency is only an upper bound: mutation of an input, a
callback, `fork()`, or an observable condition may require an earlier
synchronization. Measure the first required synchronization, not only the
first result read.

*#375: define compilation as an immutable transaction, installed by the
evaluator.* Today's mutable state includes `NativeSlot` counters and entry,
`Entry.valid`, variants and last-variant caches, `Specs`, loop memo tables,
and the thread-local `JITModule`. These are visible in `jit/closure.rs` and
`jit.rs`. Moving `JITModule` alone is insufficient. The required protocol is:
(1) The evaluator resolves admissible dependencies and creates a complete
plain-data snapshot. Workers cannot inspect `Code.consts`, dereference an
encoded `Value` address, or consult a symbol interner. (2) A request carries
runtime identity, body identity, specialization key and a generation
identifying the requested compilation. (3) The compiler owns mutable
compiler state exclusively. It returns completed executable code and
immutable metadata, not partially updated runtime caches. (4) The evaluator
checks that the request is still wanted and validates current dependencies
before installation. New code receives a fresh entry identity; old cached
frame-read masks and validation memos cannot carry over accidentally. (5)
Code and constants remain alive through outstanding requests, installed
entries and active native frames. A deopt while compilation is in flight does
not require waiting. Ordinary data-dependent bailout need not invalidate
reusable code; a disabled slot, superseded request or changed dependency must
prevent a late result from resurrecting the old specialization. Entry guards
must still run after installation. Installation validation is not a permanent
certificate. Round 19's pending-assignment flush and eager-argument bugs,
round 21's superassignment binding rules, and round 22's environment-specific
validation all remain mandatory. Native entry at a later back edge must
reconstruct current registers, cells and vector views, not snapshot-time
values. On core, keeping a Rust container alive is not sufficient rooting:
`docs/core.md` §3 explicitly requires registered or traced roots for
off-stack containers.

*#378: preserve each operation's exact arithmetic and condition boundary.*
Independent elements permit parallelism; they do not prove byte identity.
`perf-vm.md`, round 5, reports 102/768 competing-NaN payload mismatches in
the recovered fusion patch. `perf-numeric-loops.md` records deliberate SSE
operand ordering and exhaustive special-value tests for two-lane complex
arithmetic. Workers must reuse those exact rules, including signed zero,
mixed-type behavior, recycling index origin, libm entry points and
floating-point environment. For post-loop warnings such as integer overflow,
workers may return flags and the evaluator may emit the warning at the
original boundary. GNU R's `arithmetic.c` supports this specific treatment.
Do not generalize that result to every Math function or coercion. Audit
whether each operation accumulates one flag or signals during processing;
uses `warning` or `warningcall`; can invoke R, ALTREP methods or native
callbacks; can observe mutable global or thread-local state. `options(warn=2)`
and calling handlers must run on the evaluator before any subsequent R
operation. Workers may check a cancellation token; they must not invoke R's
interrupt machinery or unwind through R contexts. Scoped borrowing works
today precisely because the evaluator joins before reentry into R.
Thread-safe reference counting is unnecessary.

*#380: preserve operation boundaries, not merely per-element arithmetic
order.* The issue's claim that per-op flags emitted later preserve warning
behavior is false in general. For example:

```r
sqrt(-1) + { cat("rhs\n"); 1 }
```

The inner warning occurs before evaluating the right operand. With `warn=2`,
the right operand must not run. A calling handler can mutate bindings or
inputs relevant to subsequent work. Computing everything and then emitting
warnings "in op order" does not preserve this. Likewise, checking all shapes
up front can raise an outer recycling warning before an inner condition that
GNU R raises first. Initially fuse only across boundaries proven free of
observable effects. Warning-capable chains need staged execution or a
proof-and-fallback scheme that reaches the warning before executing dependent
R work. Reads of promises or active bindings are effects too. The numeric
proof must also preserve each original operation's intermediate binary64
rounding and shape-dependent NaN operand rules. Removing a memory store does
not authorize FMA or changing the arithmetic expression. The vector-loop
ledger reinforces the ownership requirement: round 1 needed full downstream
liveness and retained resized buffers; round 2 caught `x[x] <- v` aliasing.
Lazy or fused views must not bypass those protections.

*#381: parallel graph traversal must retain the complete collector protocol.*
Atomic mark bits are necessary, not sufficient. Current `mark_obj` also
updates block counts, handles large-object marks and pushes work. Those paths
need a race-free design; workers cannot share `&mut Core`. The difficult
boundary includes: accurate termination detection across worker queues and
in-progress tracing; serial root capture and tracing of mutable immortal
children; sequential handling of host bridge callbacks; weak-reference
fixed-point processing, followed by finalization eligibility classification;
cache pruning and sweep only after all marking phases finish. `docs/core.md`
§3 explicitly describes the weak-reference fixed point. A single parallel
strong-mark pass followed by finalizers is not equivalent. Keep sweep
sequential initially. Parallel sweep adds allocator/free-list coordination
without establishing the marking hypothesis.

**4. Specific corrections to the issues**

- #375: "lower thresholds become cheap" is too strong. Compiler CPU, snapshot
  work, instruction-cache pressure, code memory and queue contention remain.
  On two cores the compiler directly competes with useful work.
- #377: epoch values are not persistent identities. A new process can have
  the same epoch number with different bindings. Cache dependency
  descriptions and relocations; validate actual current bindings on load.
  Include helper ABI, object layout, compiler/code-generation version, CPU
  features and numeric policy. The issue's claim that numeric mode is "already"
  in the compiled-code key overstates `docs/architecture.md`, which says
  future specialized kernels must include it.
- #379: "peak memory is unchanged" is false generally. Pending tasks retain
  inputs and outputs beyond their eager lifetimes, and successive operations
  can coexist. Equal allocation timing for one output does not imply equal
  peak RSS.
- #379: waiting in `REAL()` is insufficient by itself. Previously issued C
  pointers and externally mutable foreign storage can mutate inputs without
  another accessor call. Initial eligibility should exclude such storage
  unless exclusive read ownership is established.
- #380: lazy vectors cannot defer arbitrary observable conditions. ALTREP
  materialization supplies an access mechanism, not a general license to
  change evaluation timing.
- #381 conflicts literally with #373's "workers never touch … symbols" rule.
  GC workers necessarily traverse object graphs. Specify a collector-only
  exception over a stopped heap, with no evaluator calls, ordinary mutation
  or reference-count operations.
- Fork safety needs more than a PID check. A child may inherit locked queues,
  allocator state and a compiler module whose owning thread vanished. Quiesce
  owned workers before supported forks and define child reset without
  acquiring abandoned locks.
- Pool capacity should respect affinity and CPU quota. `nproc` plus a
  compiler thread plus the evaluator is not automatically a sensible total
  concurrency budget.
- Existing architecture prose is stale about binary compatibility.
  `docs/architecture.md` still promises loading GNU R binaries unmodified;
  `docs/core.md` §1 and the lead's decision in `design.md` §5.6 supersede it.
  This review assumes source recompilation.

**5. What I would not do**

- No parallel R-level evaluation, early promise forcing, or speculative
  statements with rollback. Compilation may predict calls; it must not execute
  them to discover dependencies.
- No buffering of console output or conditions to justify reordered
  evaluation.
- No exact-mode tree reductions, reassociation, FMA contraction or
  substituted approximate vector math.
- No blanket `Send`/`Sync` for `Value`, `Sexp` or interpreter state.
- No general pending-vector implementation on today's `Rc` runtime.
- No persistent cache of raw function pointers, object addresses, validation
  memos or process-local epoch numbers.
- No claim that sync-mode conformance validates async publication. Keep
  deterministic instruction measurements, but add forced delayed compilation,
  rebinding, invalidation, shutdown and fork schedules in async tests.
- No assumption that spare-core work explains today's largest gaps. The
  numeric ledger identifies poor native loop code and VM boundaries; the
  vector ledger records 23.9 ms → 0.371 ms from whole-loop compilation. Those
  are sequential optimization wins that background work cannot replace.

*Signed: GPT-6 Astra*

---

*Gemini 4 Argon council review of #373–#381 — full text below.*

# Council Review: The Spare-Cores Design Space (`#373`–`#381`)

**Reviewer:** Gemini 4 Argon
**Context:** `docs/design.md` (§4.2, §5.5, §5.6), `docs/core.md`,
`docs/architecture.md`, `docs/ledger/perf-vm.md` (rounds 19–22),
`docs/ledger/perf-numeric-loops.md`, `docs/ledger/perf-vector-loops.md`.

## Executive Summary

The core architectural premise of `#373` is sound: **the R evaluator,
environments, symbol table, condition stack, RNG, and object reference counts
stay strictly single-threaded**, while spare cores are used as a subordinate
job system around the evaluator (background Cranelift compilation,
data-parallel slice kernels over pre-validated vectors, and stop-the-world
parallel GC marking).

However, several issues as written harbor subtle **conformance traps** against
GNU R 4.5.0 and **architectural collisions** with the G2→G3 transition to
`robigo-core`:

1. **Not all elementwise warnings in R are boolean flags raised once after
   the loop (`#378`, `#380`).** Double `%%` (`fmod_warns`),
   `gamma`/`lgamma`/`digamma`/`trigamma` (`nmath::WARNINGS`), and
   string-to-numeric coercions emit **per-element warnings** inside the loop,
   which under `options(warn = 1)`, `options(warn = 2)`, or
   `withCallingHandlers(warning = ...)` can print or `longjmp` at the *exact*
   failing index k.
2. **Double `+` and `*` NaN payload preservation (`#378`, `#380`).**
   `arith_exact.rs` (`x_is_dest`) selects which operand's NaN payload wins
   based on the *whole* operand lengths `(nx, ny)` and whether an operand was
   integer (`1+NA` vs `0/0`). Slicing a vector into chunks where a chunk
   happens to have length 1, or letting Cranelift emit commutative
   `fadd`/`fmul` in `#380`, will flip `NA_real_` (payload 1954) to `NaN` or
   vice versa.
3. **Cross-statement lazy vectors (`#380` Level 2) and pending vectors
   (`#379`) on today's `Rc<Vector<T>>` runtime would be throwaway work that
   fights the G2/G3 bridge.** Building lazy/pending storage variants before G3
   would require retrofitting every `Value` pattern-match and the `robigo-capi`
   twin/EXTDATA bridge (`docs/core.md` §6), only to rewrite it on
   `robigo-core`'s `Sexp` header and ALTREP dispatch table.
4. **Parallel GC mark (`#381`) has a non-atomic bug in `Block::marked` and
   `Core::marked_chars`, plus a `HOST` proxy problem.** In `gc.rs`, `mark_obj`
   increments `(*b).marked` non-atomically alongside setting the mark bit; in
   `sweep()`, `(*b).marked` can be replaced entirely by summing
   `marks[w].count_ones()` during the sweep pass.

## 1. Verdict per Issue

| Issue | Verdict | Expected Wall-Time Ceiling (with Assumption) | Risk to Bit-Identical Semantics | Risk to Guard-Fact System | Eng. Size |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **#373** (Track & pool architecture) | **Build now** (pool scaffold + determinism switches) | Sets the multi-core ceiling for `#375`, `#378`, `#380`, `#381`. Assumes ≥ 2 vCPUs. | **None** if `ROBIGO_THREADS=1` and `ROBIGO_JIT_SYNC=1` gate `perf.py --metric instr` and CI tiers. | **None** (workers never touch `Interp`, `Env`, or `Sym`). | **S** (~250 LOC) |
| **#374** (Measurements) | **Build now** (first, gates all others) | **0%** (diagnostic only; zero overhead in default builds via feature flag). | **None.** | **None.** | **S** (~300 LOC) |
| **#375** (Background JIT compile) | **Build now on today's runtime** | **5–20% wall-time** on short/medium scripts (0.1–0.5s runs where 1–5 Cranelift compiles take 2–15 ms on the main thread); **~0%** at peak after warmup. | **Low**, *provided* `JType::Dyn` `h_d_const` lifetime and `rjit` compile-timing rules are respected. | **Low–Medium**: snapshot must freeze `guard_syms`, `inline_call` targets, and `entry_nonna` on the main thread before sending. | **M** (~700 LOC) |
| **#376** (Ahead-of-demand JIT) | **Defer** until after `#375` lands & `#374` measures variant/callee miss stalls | **2–8% wall-time** on call-tree/polymorphic workloads (`spectralnorm`, `s4-*`), assuming main-thread tier-up wait (512 calls) dominates after `#375`. | **Low** (entry guards still gate execution), *except* for `rjit` traceback visibility if entered before 2nd call. | **Medium**: pre-compiling with unverified `nonna` or wrong cell types can cause immediate deopt/bail churn. | **M** (~600 LOC) |
| **#377** (Persistent native code cache) | **Reject / Defer past G3 & G8** | **10–30 ms** max per process (startup itself is 73 ms total, mostly parsing/evaluating base R, not Cranelift). | **Medium**: stale CPU feature flags, glibc C23 `sinpi`/`cospi` resolution, or `NumericPolicy` mismatch. | **High**: `Code` embeds `Sym` IDs, `consts` pointers, `Slot` indices, and helper addresses that change across runs and after G3. | **XL** (~2,500+ LOC) |
| **#378** (Parallel elementwise kernels, exact) | **Build now on today's runtime** (narrowed to pure slice ops; carries straight into G3) | **2.5–5× on kernel wall-time** on 8 cores for compute-bound ops (`exp`, `log`, `^`, `sqrt`, complex `*` at n ≥ 10^5); **1.3–2×** for memory-bandwidth-bound `+ - * /`. | **Medium**: high risk if per-element warning ops (`%%`, `gamma`, string coercion) or naive NaN destination selection are parallelized without care. | **None** (operates below VM/JIT inside builtins after dispatch and guard checks). | **M** (~600 LOC) |
| **#379** (Pending vector results on worker) | **Defer** (re-evaluate **only on `robigo-core` after G3**, and only if `#374` shows slack) | **0–15% wall-time** on rare pipelines that interleave large vector math with independent CPU work; **0% (or negative)** when next statement reads or branches on the vector. | **High**: asynchronous warnings/errors, `fork()`, C API `DATAPTR`, in-place mutation hazards, and out-of-order condition signaling. | **Medium**: JIT `VecView`s and `plain_lookup` bypass accessor methods unless gated at `Sexp` payload resolution. | **L** (~1,500 LOC) |
| **#380** (Fused vector expressions) | **Split:** **Build Level 1 now** (intra-expr bytecode/JIT fusion); **Build Level 2 only on `robigo-core` (G3)** | **Level 1:** **1.5–3× wall-time** and **50–80% peak RSS reduction** on multi-op chains by cutting memory traffic to 1 read + 1 write pass. **Level 2:** +10–30% on cross-statement chains. | **Medium (L1) / High (L2)**: L1 must preserve x86 SSE NaN operand order and per-op warning order; L2 risks lifetime retention and out-of-order warnings. | **Medium (L1)**: requires base-guard verification for all fused operators before entering the fused loop. | **M** (L1: ~800 LOC) / **L** (L2: ~1,500 LOC) |
| **#381** (Parallel GC mark on `robigo-core`) | **Build only on `robigo-core`** (prototype in `robigo-core` standalone now; full payoff at **G3**) | **3–5× reduction in GC mark pause** on 8 cores for pointer-heavy heaps (≥ 16 MB live); GC share × ~0.6 wall-time. | **Low–Medium**: finalizer queueing and weak-ref classification must remain sequential; `HOST` proxy callbacks cannot run in parallel before G3. | **None.** | **M** (~500 LOC) |

## 2. Recommended Order for the Next 2–3 Tracks

### What `#374` Must Measure (Including Missing Metrics to Add)

Beyond the three items already in `#374`, add these four measurements so the
gates below are decisive:

- **1b. JIT bail/miss and variant count:** How many closures/loops hit
  `NotNow`, `entry_nonna` widening, or `variants.len() > 1`? And what is the
  wall time of a single Cranelift `jit::compile` vs `jit::closure::compile`
  (broken down by IR build vs `define_function` regalloc/codegen)?
- **3b. Multi-op expression chain depth and temporary allocation bytes:** In
  `benchmarks/` (`vecarith`, `vecops`, `mandelbrot_noout`, `lm`, `glm`), what
  share of vector allocations are dead immediately after feeding the next
  elementwise builtin? (Gates `#380` Level 1.)
- **3c. Page-fault vs compute time in large vector ops:** For n = 10^5, 10^6,
  10^7, how much of `real_binary` wall time is `Vec::with_capacity` +
  first-touch kernel page faults vs arithmetic? (Informs `#378` chunk
  thresholds and `#380` destination reuse.)
- **4. `robigo-core` GC mark vs sweep time and work-stack fanout:** On the
  `experiments/memory` / `robigo-core` workloads (`binarytrees`, 2 GB
  `STRSXP`), what is the ratio of `trace()` time to `scan_stack()` and
  `sweep()` time?

### Recommended Track Order

1. **Track 1: `#374` (Measurements) → `#378` (Parallel Elementwise Kernels) +
   `#380` Level 1 (Intra-Expression Vector Fusion)**
   - **Why first:** Vector operations and intermediate allocations are the
     largest remaining wall-time gap on data-heavy R workloads. Crucially,
     pure slice kernels (`&[f64]`, `&[i32]`, `&[Complex]`) and bytecode-level
     fused expression kernels are **100% reusable across G3** because they
     operate on raw payload slices, independent of `Value` vs `Sexp`.
   - **Gating measurement from `#374`:** Confirm that (a) for `#378`, the
     crossover length N_min where parallel chunk dispatch beats
     single-threaded execution on 2 and 8 cores is stable (expected:
     N_min ≈ 1.6 × 10^4 for transcendental Math1 `exp`/`log`/`pow`, and
     N_min ≈ 2.5 × 10^5 for bandwidth-bound `+ - * /`); and (b) for `#380`
     Level 1, fused single-pass execution reduces memory traffic and wall time
     by ≥ 30% on `vecarith` / multi-op idioms at N ≥ 10^3.
   - **First milestone:** Build the shared fork-safe worker pool
     (`ROBIGO_THREADS`, pid-checked lazy re-init on `fork()`) and parallelize
     `math1` (`exp`, `sqrt`, `log`, trig) + `real_binary` (`+ - * / ^`) +
     `complex_binary` (`*`, `^`), followed by VM bytecode fusion (`Op::VecFused`)
     for trees of attribute-free elementwise ops.
2. **Track 2: `#375` (Background JIT Compilation)** *(only if `#374` confirms
   compile share)*
   - **Why second:** Completely decouples Cranelift compile latency (~0.5–3 ms
     per closure tree or complex loop) from the evaluator thread, and isolates
     `cranelift_jit::JITModule` onto a dedicated compiler thread (cleaning up
     the `thread_local! MODULE` ahead of G8).
   - **Gating measurement from `#374`:** Cranelift compilation accounts for
     ≥ 5% of wall time on at least 3 benchmark scripts or package test runs,
     OR lowering `THRESHOLD` (from 512 to ~64–128) when compile cost is
     off-thread yields ≥ 5% wall-time improvement on mixed workloads without
     hurting instruction counts in `ROBIGO_JIT_SYNC=1`.
   - **First milestone:** Background loop JIT (`jit.rs`): extract a
     `Send + 'static` `LoopCompileJob` (bytecode ops + `SlotInfo` +
     `InlineCall` snapshots with `Sym(u32)` IDs, zero `Value`/`Rc` references),
     compile on one persistent background compiler thread, and publish
     `Arc<JitLoop>` via an atomic/channel poll at `loop_enter_state`.
3. **Track 3: `#381` (Parallel GC Mark & Block Sweep on `robigo-core`)**
   - **Why third:** Self-contained inside `crates/robigo-core/src/gc.rs`, can
     be benchmarked immediately on the `robigo-core` heap benchmarks without
     touching `robigo-runtime`, and will be ready when G3 switches the
     evaluator onto `robigo-core`.
   - **Gating measurement:** Confirm that `trace()` + `sweep()` dominate
     collection pauses (≥ 75% of `collect()` time on heaps ≥ 16 MB live, vs
     root/stack scanning).
   - **First milestone:** Replace `Block::marked` non-atomic increments during
     `mark_obj` with popcount-at-sweep (`marks[w].count_ones()`), make
     `Block::marks` atomic (`[AtomicU64; START_WORDS]`), and run parallel
     work-stealing `trace()` + parallel per-block `sweep()` when
     `live_bytes >= 16 MiB` and `self.host_trace.is_none()`.

## 3. Hardest Design Problem in Each Endorsed Item

### `#375` (Background JIT Compilation)

1. **What compiler state is mutable-shared or non-`Send`, and how
   `JType::Dyn` constants cross threads:** Today, `Code` contains `Rc`,
   `RefCell`, `Cell`, and `consts: Vec<Value>`. Worse, in whole-closure JIT,
   `Whole` embeds `consts: code.consts.as_ptr()`, and generated native code
   passes `consts + idx * sizeof(Value)` directly to `h_d_const`, which
   dereferences `*v` and clones the `Rc` on the main thread when the native
   function runs. The exact rule required: the compiler worker must **never**
   receive a `Value`, `Rc`, `Env`, or `Closure`, and must never call
   `Sym::name()` (which touches `thread_local! INTERNER`). `Sym` comparisons
   are `Send`-safe. For `jit/closure.rs`, `call_tree(interp, clo)` and
   `inline_call(interp, ...)` **must run synchronously on the main thread** at
   request time, because they inspect live environments and settle `clo.jit`.
   The main thread builds a pure-data `CompileRequest` containing cloned
   `Vec<Op>` (with `Op::CallBuiltin` builtin names pre-classified into an enum
   so the worker never reads `code.consts`), formal slots, `consts_base_addr`
   (the numeric address of `code.consts.as_ptr()`), and the analyzed
   `Region`/`Fun` specs. To guarantee `consts_base_addr` remains valid when
   the compiled code runs, the `Entry` on the main thread holds `Rc<Code>` for
   every function in the call tree, and if the `Code` is dropped before
   compilation finishes, the worker's result is discarded on arrival.
   Separate `JITModule` ownership: move `JITModule` ownership exclusively to
   the compiler thread (or behind a single mutex for `ROBIGO_JIT_SYNC=1`).
   Because `cranelift_jit::JITModule` allocates pages via `mmap` and
   `mprotect`s them `PROT_READ | PROT_EXEC` inside `finalize_definitions()`,
   the compiler thread can safely allocate and finalize new code pages while
   the main thread is simultaneously executing previously finalized functions
   in earlier pages.
2. **In-flight invalidation, deopt, and `entry_nonna` races:** Suppose at
   iteration 16 of a loop, variable `s` is a non-NA integer
   (`entry_nonna = true`), and the main thread queues a background compile of
   the loop with `s.nonna = true`. At iteration 25 (while compilation is still
   in flight), the VM loop encounters `s == NA_INTEGER` or a type change, or
   `search_epoch()` bumps. When the worker publishes the finished `JitLoop` at
   iteration 40: **Guarantee 1 (Entry guards):** `jit_enter` already checks
   `jl.guards`, `jl.calls` identity/formals/bytecode, and every slot's runtime
   `JType` and `s.nonna` on *every* entry before calling `(jl.func)(...)`.
   Thus an epoch bump or type change while compilation was in flight **cannot
   cause a wrong result**. **Guarantee 2 (State transition on `entry_nonna`
   failure):** If the VM observed `s == NA_INTEGER` while
   `JitSlot::Compiling { entry_nonna: true }` was in flight, it transitions to
   `JitSlot::Failed { attempts: 0, entry_nonna: false }` and queues a widened
   compile. Any late-arriving `JitLoop` from an older request whose
   `(enter, entry_nonna)` generation does not match the current `JitSlot`
   state must be **dropped on receipt**, never overwriting a newer
   `Failed { entry_nonna: false }` or `Never` state.

### `#378` (Parallel Elementwise Kernels, Exact Mode)

1. **Bit-identical IEEE-754 and NaN payload rules across chunks:** As
   `arith_exact.rs` documents, when *both* operands are NaN, x86-64
   `addsd`/`mulsd` keeps the **destination** operand's quieted payload, and
   GNU R's choice of destination (`x_is_dest(int_operand, nx, ny)`) depends on
   the **original full vector lengths** and `int_operand`. **Rule:** Compute
   `let x_dest = x_is_dest(int_operand, nx, ny);` **once on the main thread**
   from the full vector lengths before chunking, and pass it as a const-generic
   to the worker chunks. Never let a worker recompute `x_is_dest` from its
   slice length (a tail chunk of length 1 would flip `ny == 1`!). For general
   recycling, worker c covering output range [s, e) must start its operand
   indices at `i = s % nx` and `j = s % ny`, stepping with wrap-around just
   like the sequential kernel.
2. **Exact R warning and error ordering when chunks run out of order:**
   Elementwise operations fall into **three distinct warning classes**:
   - **Class A (Pre-loop errors and warnings):** S3 `Ops`/`Math` group
     dispatch, non-numeric type errors, conformability errors, recycling
     warnings. All Class A checks stay on the main thread before spawning
     chunks.
   - **Class B (Single boolean post-loop flag `naflag`):** Integer overflow
     and `math1` domain NaNs. Because boolean OR is associative, running
     chunks in parallel and OR-reducing on the main thread after `join`
     produces **100% identical warning behavior** (including under
     `warn = 1` or `warn = 2`, because the loop has already finished in GNU R
     too when `if (naflag) warning(...)` executes).
   - **Class C (Per-element warnings inside the loop — TRAP!):** (a) Double
     `%%` calls `interp.warning("probable complete loss of accuracy in
     modulus")` **once per offending element**. Since the message does not
     include the index, counting k = Σ k_c across chunks and calling
     `interp.warning(...)` k times after the loop is exact. (b) `nmath`-backed
     functions push per-element strings into `thread_local! WARNINGS`. Worker
     threads would push into their own thread-local and the main thread would
     **lose all worker warnings**! Each worker chunk must drain its own
     `nmath::take_warnings()` into a per-chunk `Vec<String>`, and the main
     thread must replay them in **strict chunk order** before checking
     `naflag`. (c) String coercions: `CoerceWarnings`' flags OR-reduce exactly.
3. **Interrupts and `fork()` safety:** Workers must never call
   `R_CheckUserInterrupt`. Instead, workers check an `AtomicBool` interrupt
   flag between sub-chunks and return early if set; the main thread checks and
   signals the interrupt after `join`. On `fork()`, POSIX only clones the
   calling thread; worker threads vanish while potentially holding internal
   pool mutexes. Store `pool_pid: AtomicU32`; if `libc::getpid() != pool_pid`,
   leak the dead parent pool structure (never `Drop` or lock its mutexes in
   the child) and initialize a fresh pool lazily.

### `#380` Level 1 (Intra-Expression Fused Vector Expressions)

1. **Preserving exact evaluation order, S3 dispatch, and multi-op warning
   order in a single pass:** In GNU R, `exp(x) * 2 + y` evaluates
   inner-first, left-to-right, and `sqrt(-1) + stop("boom")` emits the
   warning **before** the right-hand side is evaluated (under `warn = 2` the
   `stop` never runs). Conversely, if all leaf operands are
   already-evaluated variables or constants, and all operators pass their
   base guards, and no leaf value `is_object()`, then no R code can run
   between the ops unless a warning handler or error triggers. The exact
   fusion contract (Level 1): fuse only when (1) every operator's guard
   resolves to base (`guard_ok`); (2) all leaf operands are evaluated up
   front; (3) no operand `is_object()`; (4) pre-flight shape & recycling
   validation in op post-order with `(len, type, has_attrs)` metadata only —
   and when a node could produce a data-dependent warning AND an outer node
   could warn, either bail out of fusion when `options(warn) >= 1` or calling
   handlers are installed, OR execute the fused loop first, collect per-node
   `naflag[j]` bits, and replay warnings and recycling checks in strict
   post-order with each node's exact `call` (as `warningcall`). Under
   `warn = 0`/`warn = 1` with no handlers, replay is observationally
   identical; under `warn = 2` the output buffer is dropped when the replayed
   `warningcall` errors — the fused computation leaves no observable trace.
   NaN payload fidelity: for `+` and `*` nodes in the fused kernel, emit the
   exact SSE operand order determined by `x_is_dest` at that node. In-place
   buffer reuse: if any leaf operand has refcnt 1 and is not read again, reuse
   its allocation as the output buffer.

### `#381` (Parallel GC Mark on `robigo-core`)

1. **Eliminating shared mutable state in `mark_obj` and `trace`:** Four
   concrete races to resolve: (1) `(*b).marks` and `(*b).marked`: change to
   `[AtomicU64; START_WORDS]` with `fetch_or` **but do NOT atomically
   increment `(*b).marked` per object** (cache-line bouncing); delete
   per-object `(*b).marked` updates during marking and in `sweep()` compute
   the marked count by summing `marks[w].count_ones()` — that removes a write
   from every `mark_obj` even in single-threaded mode. (2) Large-object epoch
   word: make the pseudo-header an `AtomicU64` with
   `swap(self.epoch, Relaxed)`. (3) `self.marked_chars`: thread-local per
   worker, summed after the join. (4) Large pointer vectors (`STRSXP`,
   `VECSXP`, `EXPRSXP`): a single 10^7-element vector popped by one worker
   starves all others; workers must **chunk large pointer vectors** (~4,096
   elements) onto work-stealing deques. (5) `HOST` proxies (`host_trace`):
   `trace_proxy` is **not thread-safe**. Until G3 deletes `HOST` proxies,
   either buffer marked `HOST` objects into a per-worker `Vec<Sexp>` and drain
   on the main thread, or only enable parallel marking when
   `self.host_trace.is_none()`.

## 4. What I Would NOT Do, and Why

1. **Do NOT build cross-statement lazy vectors (`#380` Level 2) or pending
   vectors (`#379`) on today's `Rc<Vector<T>>` runtime** — retrofit now, throw
   away at G3.
2. **Do NOT evaluate promises speculatively or in the background** — promise
   forcing order is observable (`missing`, `substitute`, `sys.calls()`,
   `parent.frame()`), and R's reflection makes rollback unbounded.
3. **Do NOT parallelize R-level closures in `lapply`/`vapply`/`mapply`/`Map`**
   — even "pure" closures hit slow paths that fall back to the VM; keep
   `lapply` single-threaded.
4. **Do NOT build a persistent on-disk native code cache (`#377`) now** —
   startup is 73 ms and mostly not Cranelift; JIT code embeds raw `Value`
   pointers, `Sym` IDs, and helper addresses rewritten at G3.
5. **Do NOT allow asynchronous pending vectors (`#379`) to include any
   operation that can signal a data-dependent warning** — every subsequent
   I/O op, condition handler change, `options()` call and function call would
   have to barrier-wait on pending ops, destroying the slack window. Expect
   #374 item 3 to show #379 is not worth the complexity; defer indefinitely
   unless #374 proves a >10% win.
6. **Do NOT use thread-level speculation with rollback, or reorder reductions
   (`sum`, `mean`, `cumsum`, `colSums`) in exact mode.** Exact mode requires
   sequential 80-bit `LDouble` accumulation. Parallel tree reductions belong
   strictly to `ROBIGO_NUMERICS=fast`.

## 5. Contradictions with Architecture or GNU R Semantics in the Issues

1. **`#376` vs GNU R's `R_CheckJIT` compile-timing rule:** whether a caller is
   running as compiled bytecode is **observable in R** via default
   error/warning tracebacks and `sys.calls()` (perf-vm round 10). Ahead-of-
   demand compilation may *compile* a callee before 512 calls, but native
   entry must **never** happen while `clo.jit.inline_blocked()` is true.
2. **`#375`/`#376` vs `compiled_matches_interpreted` when native execution
   does not advance call/heat counters:** settling `clo.jit` (`jit_check`)
   must happen only when the native loop is actually entered on the main
   thread, never speculatively during background compilation.
3. **`#378` "data-dependent warnings are raised once, after the join"** is
   false for `%%` on doubles (per element, a count not a bool) and for
   `gamma`/`lgamma` (thread-local `Vec<String>` queues in element order).
4. **`#378` & `#380` "compute each element independently" vs NaN payload
   destination rules:** neither issue mentions `x_is_dest`; in `#378` it must
   be computed from whole-vector lengths before chunking; in `#380` a
   Cranelift fused loop cannot emit commuted `fadd`/`fmul` when both operands
   can be NaN.
5. **`#381` "marks already live in per-block side bitmaps"** overlooks the
   non-atomic `Block::marked` increment, `marked_chars`, and `host_trace`;
   computing `marked` via popcount at sweep fixes the skip-clean invariant and
   removes the header write from the mark phase entirely.

*Signed: `gemini-4-argon`*

---

*Claude Opus 5.5 council review of #373–#381 — full text below.*

# Council review: spare cores inside one R session (#373–#381)

*Claude Opus 5.5. Reviewed against main @ 2a5ff0b3 and issues #373–#381 as of
2026-11-08. Code citations use current line numbers; several line numbers in
the issues are out of date (see §5).*

**Headline.** I agree with the frame in #373: the evaluator and `Value`s stay
single-threaded, workers get only plain data, and an exact path stays
bit-identical. Three items are worth building, and two of them can start
before G3:

- **#378**: parallel elementwise kernels.
- **#375**: background compilation, built with a narrower cut than the issue
  proposes.
- **#380 level 1**: fusion within one expression, built first as strip-mining
  over the existing kernels rather than as Cranelift codegen.

The rest should wait or be merged into other work: **#376** after #375;
**#381** robigo-core only, and after a generational nursery; **#377** folded
into G6's startup snapshot; **#379** folded into #380 level 2 on robigo-core,
rejected as a standalone item.

Two claims in the issues are wrong for today's runtime: #375 is not "make the
compile input `Send`" — compilation reads live interpreter state (§3.1); and
#380's memory argument (800 MB per intermediate result) does not hold for
robigo today, because the arith kernels already reuse temporaries in place
(§5).

## 1. Verdict per issue

The ceilings below are wall time, assuming 8+ cores, a dual-channel DDR5
desktop (about 20–25 GB/s from one core, 60–80 GB/s from all cores), glibc
libm, and a 5–50 µs thread wake.

| # | Verdict | Wall-time ceiling | Bit-identity risk | Guard-fact risk | Size |
|---|---|---|---|---|---|
| 374 | **Do first** | n/a | none | none | S |
| 375 | **Build now**, CLIF cut (§3.1) | ≤ the share of time spent compiling. Assumed 1–5% on `benchmarks/` and 5–20% on short scripts or test suites full of one-off compiles. Zero on 1 core. | none by design; existing VM/JIT mismatches become timing-dependent | **medium**: facts can go stale between request and install | M–L |
| 376 | Build **after #375 data** | ≤ VM time before tier-up that #375 still leaves. Likely 1–5%. | none by design | **medium-high**: compiles for types never observed rely entirely on the entry checks being complete | M, L with eviction |
| 377 | **Defer** into G6's snapshot | ≈0 on multi-core once #375 lands; only 1-core and energy gain | low | **high**: guard facts are per-run counters | L–XL |
| 378 | **Build now** (kernels in a crate with no runtime dependency) | Per kernel: memory-bound (`+`, `<`, `&`) 1.5–3x; compute-bound (`exp`, `log`, `^`, `gamma`) about 0.8x the core count. Whole programs limited by Amdahl's law. 2-vCPU CI: ≤1.6x. | **low** if the conditions in §3.2 hold | none | M, then S per kernel class |
| 379 | **Reject standalone**; fold into #380 level 2 on robigo-core | ≤ min(op time, slack). Expected ≈0 for typical code, where results are read by the next op. | medium-high (warning timing) | low | XL |
| 380 L1 | **Build now** as strip-mining; Cranelift only if measured worthwhile | Memory-bound chains: about 2x (`exp(x)*2+y`: 7 memory streams down to 3). Cache-resident n: 1.1–1.3x. RSS: small today (§5). | **medium** for codegen (FMA, NaN operand order); **≈0** for strip-mining | low-medium (one guard per op, leaf binding) | L |
| 380 L2 | **robigo-core only**, warning-free ops only | as L1, plus temporaries that are never materialized | high unless warning-free | low | XL |
| 381 | **robigo-core only, after a generational nursery** | Mark phase 3–5x on wide heaps, ≈1x on chain-shaped heaps. About 10–20% wall on allocation-heavy workloads, ≈0 on numeric ones. | none (reachability doesn't depend on order) | none (roots from the main thread) | M–L |

Notes on the verdicts:

- **#375 on today's runtime.** Only the Cranelift backend moves to the
  worker. Everything that reads `Value`, `Env` or `Interp` stays on the main
  thread, so the change carries over to G3 unchanged.
- **#378 survives G3.** Kernels take `&[f64]`/`&mut [f64]`, and a borrowed
  `f64` slice is `Send + Sync` while the main thread waits inside the scope.
  Put them in a `robigo-kernels` crate that, like `robigo-numerics`, cannot
  depend on `robigo-runtime`. That crate boundary is the only real enforcement
  of "workers never touch the runtime". Rust's `Send` catches `Rc`, but it
  does not catch a `thread_local!` read: on a worker that silently returns the
  default value. Examples are `jit::trace()` (`jit.rs:582`), numeric-mode
  flags and options.
- **#381 waits for G3.** It does nothing for `Rc` values. Core-council item 8
  (sticky-mark generational marking) removes more pause time per unit of work
  than parallel marking does.

## 2. Recommended order, first milestones, gates

The tracks touch different files: `jit.rs`/`jit/closure.rs`/`vm.rs` for one,
`builtins/arith.rs` plus a new crate for the other. So Track A and Track B
can run **in parallel**. Track C follows Track B because it reuses B's pool
and kernels.

**#374 first, with these additions.** None of the gates below can be
evaluated without them:

- Compile time split four ways: (a) `call_tree` + `analyse`, (b) building
  CLIF, (c) `define_function` (lowering, regalloc2, emit), (d)
  `finalize_definitions`. Also: compiles per run, code bytes, time to first
  native entry, and the share of compiled entries whose *first* entry check
  fails (stale on arrival).
- On each target machine (2-vCPU CI, WSL2, 8+ core desktop): futex wake-to-run
  latency; a STREAM-style ratio of N threads to 1 thread for
  `a[i]=b[i]+c[i]` at n = 10^5, 10^6, 10^7; the share of first-touch page
  faults in allocating a fresh 80 MB result.
- Elementwise time by kernel class (memory-bound or libm-bound) and length
  bucket.
- For fusion: the share of elementwise time spent on operands that are fresh
  temporaries (`Rc` count 1, produced by an elementwise op in the same
  expression), with tree depth and op mix.
- For #379: the time between an op finishing and the first read of its
  payload, for n ≥ 10^5.

**Track A, #375.**

- **M1**, a synchronous refactor with zero behaviour change. The main thread
  builds `ir::Function`s whose calls name `UserExternalName{ns, idx}` (helper
  index, local function index) instead of `FuncId`s of the thread-local
  `MODULE`. A `CompileService` trait does define + finalize. The only
  implementation in M1 is inline. Gate: `perf.py compare` 0 slower,
  conformance in all 3 tiers.
- **M2**, the asynchronous service: a compiler thread that *creates* its own
  `JITModule` (do not move one there), a pending table on the main thread,
  polling at the existing checkpoints, and `ROBIGO_JIT_SYNC`.
- **M3**, retune the thresholds.
- **Gate before M2** (#374 must confirm): compile time ≥3% of wall on at
  least 5 suite programs, or ≥5% median on package test scripts; backend
  (c+d) ≥60% of compile time; otherwise the cut saves too little, and moving
  analysis off the main thread is a G3-sized job.

**Track B, #378.**

- **M1** is a refactor plus the pool: move the double binary arith and Math1
  kernels out of `arith.rs` into `robigo-kernels` (instruction gate 0
  slower); add a scoped pool with park/unpark, a cancel flag, the main
  thread's FP environment copied to workers, quiescing before `fork()` and a
  pid check after it; add `ROBIGO_THREADS` and `ROBIGO_PAR_MIN`; add a **CI
  tier with `ROBIGO_THREADS=4 ROBIGO_PAR_MIN=1`**, so the thousands of small
  conformance vectors exercise the parallel path (otherwise nothing under
  10^5 elements ever tests it); add property tests that compare threads=1
  against threads=N bit for bit on inputs containing NA_real_, NaN with other
  payloads, ±0, ±Inf and recycled lengths.
- **Gate** (#374): N-thread/1-thread STREAM ratio ≥1.8x at n=10^6 on 8 cores;
  wake+join median ≤10 µs; ≥10% of wall in elementwise ops with n ≥ 10^5 on
  at least 3 realistic workloads (`vecarith`, a package test suite, a
  simulation script). The crossover length is measured per kernel class; it
  is not one global threshold.

**Track C, #380 level 1.**

- **M1** adds an `Op::Fused` tree descriptor emitted by the bytecode compiler
  for attribute-free trees of `+ - * / ^`, comparisons and Math1 (excluding
  `cum*`), with variable or constant leaves. It runs **strip-mined**: per
  block of about 2048 elements, the *existing* kernels run into an
  L1-resident scratch buffer. No Cranelift. Gate: fresh-temporary chains with
  n ≥ 10^5 are ≥5% of wall on realistic workloads, and those ops are
  bandwidth-bound on the machine. A Cranelift kernel comes only if
  strip-mining leaves more than about 15% over the bandwidth bound.

**Later.** #376, once #375 reports how much time is still spent in the VM
before tier-up and how many compiles are wasted. #381 after G3 plus a
generational nursery, gated on a GC share ≥10% and a max pause that matters
on the G3 bake-off workloads. #377 inside the G6 snapshot design.

## 3. Hardest design problem in each endorsed item

### 3.1 #375: the compile input is not a snapshot of `Code`

Compiling reads live interpreter state, so the hard part is drawing the line
between the threads.

**State that is mutable and shared.** `call_tree(interp, root)`
(`jit/closure.rs:564`) resolves callees through live environments.
`new_entry` reads `clo.inline` and `clo.jit`. `Entry`, `Fun` and `Specs` hold
`Rc<Code>`, `Rc<[Arg]>` and `RefCell`s. `Code` itself contains
`Cell`/`RefCell` memos that the main thread writes during evaluation:
`fresh_guards` (`compile.rs:811`), `CallSite::memo` (`:640`), `guard_memo`,
`native` and `jit`. It also contains `consts: Vec<Value>` (`:782`). Loop
compiles look at *current slot values* (`Reject::NotNow`). `MODULE`, `NEXT`
and `NEXT_ENTRY` are thread-locals shared by both tiers (`jit.rs:582`,
`jit.rs:4225`, `closure.rs:169`). A worker that merely *reads* `Code` while
the main thread writes those `Cell`s is a data race. So analysis and CLIF
building stay on the main thread; constants and helpers go in as immediates,
and the main thread keeps the request's `Rc<Code>`/`Rc<Entry>` in its pending
table. The worker receives only `ir::Function`s plus a name table, and
returns code pointers.

**What an install must guarantee.** It happens only on the main thread, at a
checkpoint: `LoopEnter`, the back edge where `rec.count == JIT_THRESHOLD`
(`vm.rs:5726`), or the closure counter path. It is keyed by `Entry::id`,
never by address — the code already notes that addresses are reused
(`closure.rs:174`). The worker publishes with Release, and the main thread
reads with Acquire. cranelift-jit's `finalize` already calls
`pipeline_flush_mt`, which is what aarch64 (G5) needs. It `.expect`s, though,
so the worker needs `catch_unwind`, falling back to "never compile". Every
fact the compiled code assumes must be re-checked by the existing entry
checks. That holds only if the entry checks are **complete**. #375's "the
existing entry checks already do this" has to become an audited list: every
value-derived fact in `analyse` and in the loop compiler (`nonna`,
`writable`/`owned`, matrix dims, `Valid.insts`, cell bindings), each with the
entry check that covers it. The `JitSlot::Failed { entry_nonna }` fix
(commit 32951553) shows these facts are subtle. The audit is a precondition
for M2.

**A deopt or invalidation while a compile is in flight.** robigo's
invalidation is lazy (epochs, `fn_version`, `Weak<Closure>`), not pushed, so
a stale result simply fails its first entry check. Nothing breaks, but
`NativeSlot::outcome(false)` would count a miss and could reach `give_up`
(`MAX_MISSES = 128`, `closure.rs:67`) because of a compile that was stale on
arrival. Fix: install compares the epoch snapshot taken at request time. If
it is stale, discard and requeue; don't count a miss, and allow at most 2
retries. If the body reached `NEVER` or its `Entry` was dropped while
pending, the result is discarded. Before `fork()`, the compiler is quiesced.
A child drops its pending table and recreates the thread lazily.

**Polling cost.** Sync mode must add **zero** instructions to the dispatch
loop. The ledger shows 1% swings from register pressure alone (perf-vm
rounds 19 and 20). So the readiness check lives in the cold counting path:
after a request, the loop's count is reset so the check runs every 256 back
edges. Add a `ROBIGO_JIT_DELAY=n` fuzz tier that installs after n polls with
a seeded pattern. It turns "when native code starts cannot change results"
into something tested, not just asserted.

### 3.2 #378: bit identity and warning order when chunks run out of order

**Same instructions per element.** Every chunk calls the *same*
monomorphized kernel as the sequential path. Scalar and packed SSE/AVX give
identical IEEE results and the same NaN-propagation rule (first source
operand), so vector-width peeling at chunk edges is harmless. This matters
because `NA_real_ + NaN` and `NaN + NA_real_` print differently in R.
Recycling: robigo avoids a per-element division (design.md §4.2), so each
chunk must start its recycle cursor at `start % ny`.

**Same FP environment.** MXCSR (rounding, FTZ/DAZ) and the x87 control word
are per thread. `%%` and `%/%` use x87 long double. Copy the main thread's
state at dispatch, and assert it in debug builds. Any C code calling
`fesetround` otherwise makes workers compute differently.

**Warnings and errors.** After the main thread's checks, no elementwise
kernel can error. GNU R raises "NaNs produced" and "NAs produced by integer
overflow" once, after the loop, from a flag (`arithmetic.c:809–862`,
`naflag`). Per-chunk flags OR-merged after the join reproduce that exactly.
Interrupts: GNU R checks every 10^7 iterations (`NINTERRUPT`,
`arithmetic.c:36`). Workers check an atomic cancel flag per chunk, and the
main thread raises the interrupt after the join. The result is dropped,
exactly as an interrupted GNU R loop drops it. In-place reuse
(`real_binary_inplace`, `arith.rs:506`) stays legal, because element i only
reads index i and writes index i.

### 3.3 #380 level 1: evaluation order around warnings

In GNU R, `sqrt(x) + y` evaluates `y` *after* `sqrt`'s loop, and `sqrt`'s
warning is signalled in between. Two things follow.

**(a) Leaves must be checked before anything runs.** A fused op must look up
all its leaves before running, and fall back to the unfused path if any
lookup fails. Otherwise "object 'y' not found" would come without GNU R's
"In addition: Warning message: In sqrt(x) : NaNs produced". Promises that
aren't forced yet and active bindings are not fusable leaves.

**(b) R code can run between two ops.** That happens only when op k's flag is
set *and* some observer runs code: a calling handler for warning or
condition; `options(warning.expression=)`; `warn=2` turning the warning into
an error with calling error handlers. In that case discard the fused result
and re-run unfused from the start. To make that safe, the fused output must
not reuse an operand's buffer when any non-final op can warn. With `warn=0`
or `warn=1` and no handlers, emitting the flags in op order is byte-identical.

**Strip-mining needs no new floating-point code.** It runs the existing
kernels on L1 blocks, so FMA contraction, operand order and libm choice
cannot change. A Cranelift kernel would have to guarantee all of that itself:
no FMA, operand order kept (check that Cranelift's mid-end does not reorder
float ops), the same Rust libm entry points as the builtin, and every
intermediate rounded to double.

## 4. What I would not do

- **Evaluate promises early, or speculate on later statements with rollback.**
  Promise forcing order is observable (`missing`, `substitute`, defaults like
  `n = length(x)` that are never forced). R's reflection (`sys.function`,
  `parent.frame`, active bindings) makes rollback unbounded.
- **Parallel R-level evaluation of user code**, including `lapply`/`vapply`
  over "pure" closures. The narrow possible exception, a later candidate and
  not part of this track: a native call tree (round 19) whose types are only
  scalar or `VecView`, writes no cells and has no `JType::Dyn` (round 20 `Dyn`
  slots hold `Rc` values), mapped over elements. A bail at element k would
  mean re-running sequentially from k.
- **Parallel or "more exact" reductions in exact mode.** Kahan sums or
  superaccumulators are reproducible but differ from GNU R's sequential
  long-double `sum` (round-off differences are bugs, per the porting
  contract). Note, though, that `colSums`/`rowSums` *are* exact to
  parallelize across output elements: `rowSums` accumulates each row in long
  double in column order (`array.c`, `LDOUBLE *rans`, outer loop over `j`),
  and splitting the rows preserves that order. Likewise stable `order`/`sort`
  (the permutation is unique) and the probe phase of `match`. #378 wrongly
  lumps these with reductions (§5).
- **Parallelize anything that allocates R objects or CHARSXPs** (`paste`,
  `gsub` results, `as.character`). The global CHARSXP table and the allocator
  belong to the main thread. On robigo-core, `grepl`/`regexpr` matching over
  read-only CHARSXPs is a good later candidate; building the result strings
  is not.
- **Atomic reference counts or `Arc` values** (design.md §5.5 decision).
  **Concurrent GC marking** while the mutator runs, before the barrier hook
  (core-council item 8) has proven itself.
- **Reorder observable output**: no background flushing of console or
  connections, nothing that changes interleaving with C stdio (core-council
  item 23).
- **Spin workers indefinitely, size the pool with `nproc`, or create full
  pools inside `mclapply` children.** That oversubscribes N² and burns the CI
  Docker slots. Use `available_parallelism` (affinity and cgroup quota).
  Default to 1 thread in forked children unless `ROBIGO_THREADS` is set. Spin
  briefly (≤50 µs), then park.
- **Precompile base functions at startup** (#376's third bullet). It costs
  CPU on every `Rscript -e` in shell loops and batch farms. That belongs in
  the G6 snapshot.
- **Drop the instruction gate in favour of wall time.** Sync and `THREADS=1`
  stay the gate; wall time is reported beside it.

## 5. Contradictions with the architecture or GNU R semantics

1. **#375, "make the compile input `Send`" / "snapshot of constants and
   symbols".** This understates the problem: `call_tree` needs `&mut Interp`
   and live environments, and `Code` carries `Cell` memos written during
   evaluation (§3.1). Line numbers in the issue are out of date:
   `JIT_THRESHOLD` is `vm.rs:4044` (not 2795), `Code::consts` is
   `compile.rs:782` (not 669), the thread-locals are `jit.rs:582`/`4225` and
   `closure.rs:169` (not 353/1982/1335).
2. **#375, "re-tune thresholds; lower is cheap".** Compile cost is not the
   only cost. `vm.rs:4047–4051` records that compiling at 16 made `nbody` 25%
   slower because inner loops compiled before the outer one. Lower thresholds
   also trigger more `give_up`s. Thresholds are a policy question; measure
   them as one.
3. **#376, "evict entries that are never entered".** cranelift-jit cannot
   free single functions. `JITModule::free_memory(self)` frees the whole
   module. Eviction needs one module per generation, freed at a quiescent
   point where no native frame is live (top-level statement boundaries).
   Otherwise "Code memory stays valid while any frame can be executing it"
   becomes a use-after-free.
4. **#378 candidates.** The Math group includes `cumsum`/`cumprod`/`cummax`/
   `cummin`, which are sequential. `ifelse` is R code (`rlib/base/R/ifelse.R`);
   what can be parallelized is `rep` and the logical-mask `[<-`. Numeric
   coercion from character touches `Rc<str>` (`!Sync`) and warns "NAs
   introduced by coercion". Only numeric-to-numeric coercion qualifies.
   Placing `colSums` under "reductions stay sequential" is wrong (§4).
5. **#379, "copying instead would make `x[i] <- v` loops copy".** On today's
   runtime, holding the input in a pending op raises its `Rc` count, and that
   alone makes `Rc::make_mut` copy. The design needs a separate borrow count
   that `make_mut` ignores but in-place writes wait on. Another reason it is
   robigo-core only. There, a pending vector is naturally an ALTREP-class node
   whose `Dataptr` waits. It shares the existing ALTREP check, so `REAL()` pays
   no new test, and it is the same mechanism as #380 level 2.
6. **#380.** The RSS claim ("800 MB per intermediate") ignores that robigo
   frees temporaries as soon as they are consumed and that arith writes into
   the buffer of an unshared operand (`arith.rs:506`; design.md §4.2).
   `exp(x)*2+y` already allocates one buffer today. The gain is memory passes
   (7 streams to 3), not memory. On robigo-core, where dead temporaries live
   until a GC, the claim comes back partly. Level 2's "materialize at
   DATAPTR" would raise "NaNs produced" at whatever later point first reads
   the payload, possibly inside C code, after later statements' output. That
   violates GNU R's warning order. Level 2 must be restricted to
   warning-free ops (as #379's phase 1 is). Deferred allocation failures are
   acceptable, because out-of-memory behaviour depends on the machine and is
   not part of the oracle.
7. **#381, "weak references and finalizers sequential, after marking".** R's
   weak-ref marking is a fixpoint *interleaved* with marking: values of weak
   refs with marked keys get marked, then marking repeats (`memory.c`,
   `RunGenCollect`). The sequential step therefore runs after *each round*,
   and each round can be parallel. The hard part #381 doesn't mention: R
   heaps are dominated by a few huge `STRSXP`/`VECSXP` (data-frame columns of
   10^7 CHARSXPs) and by serial pairlist chains. Parallel marking pays only
   with **array splitting**, where a large pointer array is scanned in ranges
   by several workers. Chains don't parallelize at all. Atomic `fetch_or`
   must be test-then-set, and the sequential path must contain no atomics, so
   the `THREADS=1` instruction counts don't move.
8. **#373, "pool sized from `nproc`".** It should use `available_parallelism`,
   with 1 thread in forked children by default (§4). Also, callgrind runs
   threads one at a time, so the instruction metric counts total work, never
   wall time. That is a further reason `THREADS=1` and sync mode are the gate.

*Claude Opus 5.5*

---

*Claude Fable 5.1 council review of #373–#381 — full text below.*

# Fable 5.1 council review: the spare-cores design space (#373 to #381)

Reviewed: issues #373-#381 as of 2026-11-08, `docs/design.md` §4/§5.5/§5.6,
`docs/architecture.md`, `docs/core.md` §3/§10/§11, `docs/ledger/perf-vm.md`
rounds 19-22 and the 2026-10-28 entries, `docs/ledger/perf-numeric-loops.md`,
`docs/ledger/perf-vector-loops.md`, and the code the issues cite (`jit.rs`,
`jit/closure.rs`, `vm.rs`, `builtins/arith.rs`, `arith_exact.rs`,
`robigo-core/src/gc.rs`, `robigo-numerics/src/lib.rs`).

**Headline.** The rules in #373 are right and should be kept verbatim:
workers touch slices and plain snapshots only, bit-identical or it belongs to
fast mode, determinism switches for the instruction gate. Of the seven
designs, two have a large and measurable ceiling and no contact with the
guard-fact system: parallel elementwise kernels (#378) and fusion inside one
expression (#380 level 1). Background compilation (#375) is sound but its
ceiling is the compile time itself, and for hot loops only a fraction of
that; it should wait for #374's number and come with a deterministic
install-delay mode, because the ledgers show the JIT's entry and leave checks
are where bugs have hidden. Pending results (#379) collapse into #380 level 2
and belong on `robigo-core`. Parallel marking (#381) is correct and worth
doing, after G3 and after the barrier decision. Compile-ahead (#376) and the
on-disk code cache (#377) should be deferred; #377's key as written is wrong
for this runtime.

The one place the issues contradict GNU R is the claim in #378 that
Math-group warnings are flags raised once after the loop. For `gamma`,
`lgamma`, `beta`, `choose` and `%%` on doubles they are per-element
`warning()` calls inside the loop (nmath.h:70 `MATHLIB_WARNING →
warning(fmt,x)`; gamma.c:152-213; choose.c:81; arithmetic.c:185). robigo
already handles this sequentially with an ordered queue
(`robigo_numerics::take_warnings`, `arith.rs:385-388` counts and replays),
and the parallel design must preserve that sequence per chunk.

A probe run before this review: native loops keep GNU R's destination-operand
NaN rule (`NA + NaN` is `NA`, `NaN + NA` is `NaN`), with 8 loops compiled.
That removes one hazard I suspected for fused kernels; the review still asks
for it to be pinned by a test.

## 1. Verdict per issue

Assumptions behind the ceilings: a desktop with 8 or more cores for wall
time, the 2-core CI VM for the instruction gate; today's shootout runs are
10-300 ms after a 70 ms startup; `vecarith` (2e6 doubles, 80 elementwise ops
and 20 reductions per run) is the only suite benchmark with vectors above
1e5.

| issue | verdict | wall-time ceiling (assumption) | bit-identity risk | guard-system risk | size |
|---|---|---|---|---|---|
| #374 measure | build now, first | n/a | none | none | 1 week |
| #375 background JIT | build now, but after #374 confirms compile ≥ 3-5% of wall on a real workload; ship with `ROBIGO_JIT_DELAY` | ≤ total compile time; for loops only compile × (VM speed / native speed), ~10% of the compile time | low if entry checks are complete; the ledgers show they were not, four times | medium: identity of inlined callees across the in-flight window | 2-3 weeks; also the vehicle for moving JIT state out of `thread_local!`s (A.2 isolates) |
| #376 compile ahead | defer; do the threshold re-tune in #375 first; never compile unobserved type variants | the VM time before a unit reaches 512 calls / 16 iterations; ≈ (512 − new threshold) × (3,000 − ~100) instructions ≈ 0.3 ms per hot closure | none for observed variants; untested code paths for predicted ones | low | 2 weeks plus per-batch `JITModule`s for eviction |
| #377 code cache | reject as written; fold into G6's startup snapshot as a bytecode (`Code`) snapshot; revisit machine code after G3 if steady-state compile share is > 10% | ≤ compile time per run, which #374 bounds | none | medium: a stale entry whose frozen-base set (`vm/rjit.rs`) differs is a wrong result | large: relocation, keys, trust |
| #378 parallel kernels | **build now on today's runtime**, first code track | memory-bound ops (`+ - * /`): 2-3x at 4+ threads, bandwidth-limited; compute-bound (`exp`, `log`, `sqrt`, `^`, `%%`): near-linear to 8 threads; `vecarith` 0.72 s → ~0.4 s on 8 cores; suite geomean barely moves | low, with three concrete rules (§3) | none | 1-2 weeks: a ~200-line owned pool, 10 kernels, the warning replay |
| #379 pending results | build only on `robigo-core`, and only as #380 level 2 with eager background materialization; #374 item 3 will show the slack is near zero for `z <- op(op(x))` chains | the ops between a fill and its first read; I expect < 2% | medium: every `Rc::get_mut` / `strong_count == 1` in-place path (`arith.rs:508`, `:1289`) is a data race with a worker writing a pending buffer unless pending is checked first | none | large on today's `Value` (no central payload accessor); medium on core (a header bit tested in `DATAPTR`) |
| #380 fusion, level 1 | **build now** (it is tier 5 of §4; the `a*b+c` one-pass entry of 2026-10-28 is the precedent), second code track | `(x*2+y)/(1+abs(x-y))`: 288 MB of traffic → 48 MB, 5 temporaries → 0; 2-4x on memory-bound chains, multiplicative with #378 | medium: operand evaluation order and warning order (§3); solved by fusing only evaluated operands and by transaction-and-replay | low: the guards are the existing base-binding guards on `+`, `exp`, ... | 3-4 weeks in `compile.rs` + a Cranelift kernel; carries to G3 (`VecView` in, `VecView` out) |
| #380 fusion, level 2 | only on `robigo-core`, only for ops with no data-dependent warning or error, after #374 counts cross-statement chains | the same traffic saving across statements; peak RSS can go **up** (inputs kept alive) | high unless restricted to warning-free ops: `t <- sqrt(x)` must warn at that statement | low | large: a storage variant every reader understands, i.e. core's `DATAPTR` |
| #381 parallel mark | build only on `robigo-core`, after G3 and after the barrier/sticky-mark decision (council item 8, core.md §11.1) | GC share × ~0.6: 3-12% on allocation-heavy code, 0 on scalar code; the big win is the 2 GB live heap workload (core-council item 9) | none (the reachable set is a set) | n/a | 2-3 weeks; design now, code after G3 |

## 2. Recommended order and gates

**Track 1: #374, measurement (1 week).** No runtime code beyond
`--features op-stats` counters. It must produce, for `benchmarks/suite`, the
rivals `real/*` programs and the largest package test suites that run today:

1. Elementwise op time by result-length bucket: < 1e3, 1e3-1e5, > 1e5, per op
   class (add/mul, div/pow, transcendental, comparison/logic, coercion). Gate
   for #378: the > 1e5 bucket is ≥ 10% of wall on at least one non-synthetic
   workload (`vecarith` will say 80% and proves nothing). Also the per-op
   cost per element at 1e4, 1e5, 1e6, 1e7 single-threaded, which sets each
   class's threshold.
2. Compile time **by kind**: loop compiles and closure compiles separately,
   each with the unit's measured VM-to-native speed ratio, because the saving
   differs (§3, #375). Gate for #375: Σ closure compile time + Σ loop compile
   time × (VM/native) ≥ 3% of wall somewhere real. Also the count of compiles
   that are never entered or are abandoned (`NativeSlot::outcome`, `bailed`),
   which bounds what lower thresholds would waste.
3. Slack and chains: VM ops between a large op's return and the first payload
   read (#379), and the share of large results consumed by exactly one
   further elementwise op in the same expression (#380 level 1) or in a later
   statement with the input dead after (#380 level 2).
4. Promise-force latency as a histogram, to close the "promises early"
   question for good.

**Track 2: #378 (after item 1 is in).** First milestone: double
`+ - * /` without attributes (the `real_binary` and `real_binary_inplace`
paths) and `sqrt exp log log1p abs`, chunked over a small owned pool with a
per-class length threshold. Gate: `ROBIGO_THREADS=1` instruction counts
within 0.5% of main; all conformance tiers unchanged; a unit test that
chunked == sequential bit for bit over all 169×169 special-value pairs
placed at every chunk boundary (the `pair_cmul_matches_cmul` method,
numeric-loops ledger); a recycling test with chunk sizes coprime to the
shorter length; `vecarith` wall −30% on 4 cores on an idle machine with the
paired protocol of round 19.

**Track 3: #380 level 1 (can start in parallel with track 2, different
owner).** First milestone: chains of double `+ - * /` whose operands are
variables or constants, no attributes, one Cranelift loop, sequential; then
run the kernel in chunks on track 2's pool. Gate: peak RSS and wall on
`vecarith` and two new vector-chain benchmarks; conformance unchanged; a test
that every op's warning is raised in op order and that a chain with a warning
gives GNU R's output under `warn=0`, `warn=1` and `warn=2`.

**Then #375**, if #374's item 2 clears its gate; its first milestone is the
state move (JIT state into the interpreter instance, the compile input as a
`Send` snapshot) with `ROBIGO_JIT_DELAY=n` landing before any thread exists.

## 3. The hardest design problem in each endorsed item

**#378.** Three rules make chunking bit-identical and ordering-identical:

- *Same scalar function in both paths.* `exact::pair` and `exact::one`
  (`arith_exact.rs`) implement the SSE destination rule for NaN payloads; a
  chunk boundary must not change whether an element goes through the packed
  or the scalar remainder path in a way that changes bits. Today lane
  semantics equal scalar semantics, so it holds; pin it with the boundary
  test above. The x87 paths (`%%`, `%/%`, `myfmod`'s long double) run on each
  worker's own FPU; assert at pool start that each worker's control word and
  MXCSR match the main thread's (nearest rounding, FTZ/DAZ off). glibc's
  `exp`/`log` pick one ifunc per process, so all threads compute the same
  bits.
- *Per-element warnings in element order.* Flag-type warnings ("NaNs
  produced", "NAs produced by integer overflow") are per-chunk booleans ORed
  on the main thread and raised where `arithmetic.c:834` raises them: fine as
  #378 says. But nmath's `ml_warning` and `%%`'s accuracy warning are a
  **sequence** that depends on element order and count: `warn=0` lists them,
  `warn=1` prints each, `warn=2` turns the first into an error that aborts
  the op. The mechanism exists: each worker drains its thread-local
  `take_warnings()` queue at the end of its chunk and returns it with the
  chunk; the main thread concatenates in chunk order and replays, and
  `arith.rs:385` keeps counting `fmod_warns`. The result buffer is discarded
  when the replay errors.
- *In-place output with recycling.* `real_binary_inplace` writes into a
  unique operand; chunked, the output chunk aliases its input chunk element
  by element, which is safe only because the op is elementwise. The recycled
  operand's index must be the absolute `i % m`, never chunk-relative.
  Interrupts: GNU R checks every 10M elements (`NINTERRUPT`) and aborts the
  op; the main thread checks between chunk joins and raises the same error
  after the join.

Also: the pool must be lazy, pid-checked (#144), and sized from `nproc` with
`ROBIGO_THREADS`; a job cannot be in flight at `fork()` because fork happens
from R code after every join.

**#380 level 1.** Two problems, both about *when* things become observable:

- *Operand evaluation order.* `exp(x) * 2 + y` in GNU R runs `exp` (and
  raises its warning) before `y` is evaluated. If `y` is a call that prints,
  fusing moves `exp`'s warning after the print. So fuse only when every
  operand is an already-evaluated variable or constant, exactly the inliner's
  rule ("constant or variable arguments", §4.2 ledger) and what the `a*b+c`
  entry did. Then no R code runs between the first op and the last.
- *Warnings as a transaction.* Even then, a calling handler for the first
  op's warning can run R code that rebinds `*` before the sequential path
  would call it. The fused loop cannot interleave. Answer: the fused kernel
  is a transaction in the sense of `jit/closure.rs`: run it; if any op's flag
  is set, discard the result and replay the ops one at a time in the VM. The
  common case pays nothing, the warning case pays one extra pass, and the
  output is the VM's by construction. Only flag-type ops may be fused
  (`+ - * / ^`, comparisons, logic, `abs sqrt exp log log1p floor`); `%%`,
  `gamma`, `choose`, `beta` stay out.
- *Codegen fidelity.* Each op in the kernel must be the builtin's scalar
  function: the destination-operand rule for `+`/`*` (verified on the loop
  JIT today: `NA + NaN` is `NA`, `NaN + NA` is `NaN` in 8 compiled loops,
  same as GNU R 4.6.1 on this machine; keep a test under `tests/r/jit/`),
  `r_pow`, int→double coercion at the same point, integer overflow to NA with
  its flag, no FMA contraction, no commuting.

**#375.** The mutable-shared state is: `MODULE: RefCell<Option<JITModule>>`
(`jit.rs:581`), the `INTERNER` (`sym.rs:150`), `R_EPOCH`/`SEARCH_EPOCH`
(`env.rs:109-134`), and the per-`Code` slots `jit` and `native`. The design
that needs none of it shared: the main thread lowers a request to a `Send`
snapshot (ops, scalar constants by value, string constants by address with
the `Rc<Code>` held in the ticket, `Sym` ids, the types/nonna/live vectors
`jit_enter` computes at `vm.rs:4476-4514`, the resolved inline targets by
**content**, not pointer); the compiler thread owns the `JITModule` and
returns function pointers plus metadata; the main thread installs at
`LoopEnter` / closure entry by `try_recv`, through the existing entry check,
never from the worker. What a mid-evaluation install must guarantee: the
installed code is conditional on its guards and types, which the entry
re-derives, so the only in-flight hazard is identity: an inlined callee's
`Code` or `Closure` freed and its address reused during the compile. Hold
strong `Rc`s in the ticket or key by bytecode identity as round 22 does. A
guard change in flight needs no cancellation; the code is simply never
entered. Code memory is never freed, as today. Non-negotiable companion:
`ROBIGO_JIT_DELAY=n` installs at the n-th entry after the request,
single-threaded; run the conformance tiers at n ∈ {0, 1, 7, 100}. Async
mode's bugs are install-point bugs, and this makes them reproducible without
threads.

**#381.** `mark_obj` (`gc.rs:1160-1192`) does three writes that become
shared: the bitmap word (`fetch_or`, relaxed; the returned old value says who
claimed it), the per-block `marked` count, which skip-clean sweeps need exact
(per-worker tallies summed per block at the end, or an atomic add), and a
large object's epoch word (CAS to claim). `mark_stack: Vec<Sexp>` becomes
per-worker deques with stealing and a termination barrier. Roots stay on the
main thread: the conservative scan, `R_PPStack`, the precious set, and above
all `root_fns` callbacks, which call into the runtime. The cutoff: waking 4
threads costs 20-50 µs; a 1 MB mark costs ~0.3 ms; parallelize above ~16 MB
live. Sweep is already per block and should go lazy before it goes parallel.
Build it after the sticky-mark/barrier decision fixes the header and bitmap
layout it writes into.

## 4. What I would not do

- **Evaluate user code on another thread**, in any form: early promises,
  speculative statements with rollback, parallel `lapply` under the hood.
  `sys.function`, `on.exit`, active bindings, connections, the RNG stream,
  `options()`, the warnings list and `Rprof` make the sequential order the
  specification. #373 says so; agree.
- **Parallel reductions in exact mode.** `sum`, `mean`, `cumsum`, `prod`,
  `colSums`, `var` accumulate in x87 long double sequentially; any split
  changes bits. Fast mode only, as #378 says.
- **Predicted type variants** (#376): code for types no run has produced is
  code no test has run. Compile observed variants only; use the compiler
  thread's idle time for queued *observed* work.
- **A machine-code cache on disk** before G3 and G6, and never keyed by body
  hash alone (§5, item 3).
- **Lazy values on today's `Value`.** A new variant that every builtin must
  understand is G3 work done twice; `docs/design.md` §5.6 already says new
  builtins wait for G3.
- **`rayon` or any global pool**, atomics on `Rc` counts, or workers that can
  see a `Value`, `Sym` or the interpreter. A 200-line owned pool whose jobs
  take `&[T]`/`&mut [T]` is the whole requirement.
- **Lowering `JIT_THRESHOLD`/`THRESHOLD` before #375 and a measurement.**
  `JIT_HEAT`'s comment (`vm.rs:4047`) records what happened at 16 on `nbody`
  (+25%).
- **Wall-time gates on shared CI runners.** Wall time belongs to
  `scripts/perf.py --metric time` on an idle machine with repeated batches
  (round 19: a single millisecond reading says nothing); the instruction gate
  stays with `ROBIGO_THREADS=1 ROBIGO_JIT_SYNC=1`, and callgrind would
  otherwise sum worker instructions into it.
- **Building #381 against the bridge-era heap**, whose roots are host-proxy
  callbacks that disappear at G3.

## 5. Contradictions with the architecture or GNU R

1. **#378, "data-dependent warnings are raised once, after the join ... as
   GNU R does from a flag."** True for `naflag` (arithmetic.c:834, `math1`'s
   "NaNs produced"); false for `%%` on doubles (arithmetic.c:185, `warning()`
   per element), `gamma`/`lgamma` (gamma.c:152-213 via nmath.h:70), `beta`,
   `choose` (choose.c:81, the message carries the element's value). The
   sequence and count are observable under `warn=0/1/2`. Use the queue replay
   (§3).
2. **#380 level 1, "the compiler sees a call tree of elementwise builtins
   with guards."** Needs the extra condition that every operand is already
   evaluated; otherwise R's left-to-right builtin argument evaluation
   interleaves user code between fused ops. And warnings need the transaction
   rule, not just "raised in op order".
3. **#377's cache key** (body hash, guard set, variant, numeric mode,
   version, CPU) omits what the ledger shows is per run: `Sym` ids shift with
   interning order ("interning moves everything", perf-vm 2026-10-28), `Slot`
   indices follow them, and `vm/rjit.rs`'s frozen set of base bindings is
   decided at GNU R's compile moment per closure instance. Cached code must be
   keyed and relocated by names, which is most of the work.
4. **#376's "evict entries that are never entered"** is not possible per
   function with cranelift-jit 0.137 (`Cargo.toml:22-26`):
   `JITModule::free_memory` frees a whole module. Eviction needs one module
   per compile batch and a safe point at which no native frame is live, which
   the tree-abandon model gives.
5. **#375's "when native code starts running cannot change any result"** is
   the invariant, not a fact: perf-vector-loops round 1 (Sol review: stale
   vector register at a leave, 3 for 33), round 19 (sol-loop: four observable
   differences), and the visibility leak at a leave were all entry/leave
   gaps. Async install adds a new axis along which such gaps show; hence the
   deterministic delay mode.
6. **#373's "one compiler thread" vs core.md A.2 isolates.** Code memory and
   `Sym` ids are per isolate; the compiler thread, or at least its
   `JITModule` and queue, must be per isolate. Stateless pool workers are fine
   process-wide.
7. **#381's "stop the world, as today"** is compatible with §11's sticky-mark
   and concurrent directions, but the order matters: the barrier hook
   (core-council item 8) decides the header bits and the bitmap the parallel
   marker writes. Decide that first.
8. **#379's premise** that payload reads have a natural choke point is false
   on today's `Value` (hundreds of `&x.data` sites, and the unique-owner
   in-place paths) and true on core only if the pending bit is checked in
   `DATAPTR`/`REAL` for C code too. The issue draws the right conclusion; the
   lead's order in #373 (item 7, "only if #374 shows slack") should become
   "only on core, as #380 level 2".
9. Minor: #378 lists `ifelse`'s fill loops; `ifelse` is R code over `[<-`
   with a logical index, so the kernel is logical-mask subassignment, a
   different builtin from the arithmetic ones and worth its own line in
   #374's size histogram, together with `x[mask]` gathers from `vecops`.

*Fable 5.1, council review of #373-#381, 2026-11-08.*
