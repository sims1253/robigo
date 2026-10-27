# The memory-manager bake-off

`robigo-core`'s object model (goalpost G2, `docs/architecture.md` "The
core object model") needs a memory manager. This workspace decides which
one by measurement: two fully working managers behind one Rust trait
(`mm-core::Mem`), sharing one block allocator (`mm-heap`), driven by
identical, generic workloads (`mm-bench`). The workloads are the shapes
R actually executes:

| workload | what it models |
|---|---|
| `binarytrees` | recursive list allocation + traversal (`benchmarks/games`); a long-lived tree plus short-lived trees |
| `fib` | one environment per closure call, 10^6 calls, deep recursion |
| `lapply` | list traffic plus an environment per call |
| `listgrowth` | `r[[length(r)+1]] <- v` growth, `x[i] <- v` in-place updates guarded by the sharing count, with and without an alias |
| `strings` | string vectors over interned CHARSXPs (hot cache + all-miss + `nchar()` walk) |
| `cycles` | environment/closure cycles that die (memory must stay bounded) plus a retained keep-list |
| `capi` | C-API shape: `extern "C"` routines through a volatile function pointer, `PROTECT`/`UNPROTECT` traffic, an interior `REAL(x)` pointer held across allocations |

Every workload returns a checksum; the two managers must agree on all
of them or the harness refuses to time anything. That cross-check is
how nearly every bug below was found.

## The two managers

**(a) `mm-rc` — reference counting + trial-deletion cycle collector**
(Bacon–Rajan synchronous, the shape of CPython's gc). Handles own
counted references; every heap edge increments and decrements. Purple
buffer of decremented-but-alive containers, collected when it reaches
`max(4096, live/2)`; dying objects that are still buffered are parked
until the collection validates the buffer. Interned CHARSXPs are
released lazily at collection time (a weak table with counted
entries). Collection was rewritten around an edge list so the
purple-reachable subgraph is walked once, not 4–5 times (see commit
history).

**(b) `mm-gc` — non-moving mark-region heap (Immix-shaped)**: 64 KiB
blocks, size classes, free lists rebuilt by the sweeper, fully-dead
blocks recycled through a heap-wide pool, fully-dead chunks released to
the OS. Marking starts from precise roots (the protection stack, the
global frame, the manager's own symbol/intern tables) and then
**conservatively scans the Rust stack and registers** (a small
`global_asm` shim spills them); interior pointers are resolved to object
starts in O(1) with a start-bit scan. Handles are plain `Copy` pointers;
reference counts exist **only for copy-on-modify** (heap-to-heap
references and bindings, exactly GNU R's post-4.0 REFCNT), so
`MAYBE_SHARED`-style decisions are exact while Rust locals cost nothing.
The sweeper skips blocks where nothing died (per-block live counts from
the mark phase, alternating mark colors so survivor headers are never
rewritten), and the collection floor is 2 MiB so the churn window
between collections — the allocator's streaming cache footprint — stays
inside the LLC (see "The cache study" below).

Both use the same 16-byte header layout, the same size classes, and
the same allocator, so allocation speed itself is held constant and the
measurement isolates the *management* strategy.

## Running it

```sh
cargo test --release                 # unit tests, both managers
cargo build --release
python3 bench.py run --reps 5        # interleaved wall/RSS/pause + callgrind
                                      # Ir, D1/LL misses, est cycles
python3 bench.py table results/*.json            # ratios at D1=10,LL=100
python3 bench.py table --d1 14 --ll 250 results/*.json  # contended-machine model
```

`MM_RC_FACTOR` (purple buffer divisor, default 2), `MM_GC_MIN` (GC floor,
default 2 MiB) are experiment knobs; `MM_GC_DEBUG=1` prints a
skipped/walked/freed line per gc sweep.

## The numbers

Machine: 2 shared cores (measurements taken while other agents worked;
load is recorded in every results file). Scale 1.0, release build,
`lto`, `panic=abort`. **Callgrind counters (Ir, D1/LL misses) are exact
and load-independent**; estimated cycles use
`Ir + 10·D1m + 100·LLm` (≈10 cycles per L1 miss that resolves in a lower
cache, ≈100 per last-level miss; `table --d1/--ll` recomputes at other
penalties — the conclusion below survives 14/250).

### The instruction counts (why the gc looked right)

| workload | rc Ir (M) | gc Ir (M) | rc/gc |
|---|---:|---:|---:|
| binarytrees | 856 | 424 | **2.02×** |
| fib | 497 | 306 | **1.63×** |
| lapply | 66 | 55 | 1.21× |
| listgrowth | 43 | 34 | 1.28× |
| strings | 293 | 313 | 0.94× |
| cycles | 163 | 38 | **4.35×** |
| capi | 1998 | 1364 | 1.47× |
| *geometric mean* | | | *1.53×* |

(gc numbers at the 2 MiB floor; at the old 8 MiB floor the geomean was
1.70× — more frequent collections cost instructions and buy locality.)

### The cache study (why the wall times disagreed)

The first bake-off's wall times contradicted Ir exactly on the
allocation-heavy workloads: rc won fib 0.72×, strings 0.63× while
executing *more* instructions. Callgrind's cache simulation
(`--cache-sim=yes`, deterministic) shows why — **reference counting
recycles the same cache-hot slots; a mark-region gc streams fresh memory
between collections**:

| workload | rc D1m | gc D1m | rc LLm | gc LLm | rc est cyc (M) | gc est cyc (M) | rc/gc |
|---|---:|---:|---:|---:|---:|---:|---:|
| binarytrees | 7.2M | 6.8M | 0.6M | 4.4M | 988 | 930 | 1.06× |
| fib | **12k** | 6.4M | **10k** | 51k | 498 | 375 | 1.33× |
| lapply | 330k | 1.3M | 185k | 250k | 88 | 93 | 0.95× |
| listgrowth | 456k | 1.2M | 138k | 112k | 61 | 57 | 1.08× |
| strings | 1.8M | 3.1M | 301k | 334k | 341 | 377 | 0.90× |
| cycles | 2.9M | 0.9M | 62k | 51k | 199 | 52 | 3.85× |
| capi | 12.9M | 18.4M | **24k** | 53k | 2130 | 1553 | 1.37× |
| *geometric mean* | | | | | | | *1.32×* |

At the old 8 MiB floor the picture was worse for the gc (fib: 5.4M D1
misses, 149k LL misses; binarytrees est cycles a dead heat at 0.99×;
lapply 0.92×, strings 0.93× for rc) — **the misses ate the entire 1.7×
instruction advantage on the workloads shaped like the evaluator**. On a
contended machine (shared LLC, memory-bandwidth pressure) LL misses cost
well above 100 cycles, which is exactly where rc's wall wins came from.

Two gc changes fix the locality deficit, measured at every step
(`results/floor-dial-*.json`):

1. **Skip-clean sweeps**: each block counts the objects the mark phase
   found live (`Block::marked`, immortal included, manager-owned tables
   marked as roots); the sweeper skips blocks where nothing died, and
   mark colors alternate so survivors' headers are never touched just to
   clear bits. A collection now costs work proportional to the *garbage*,
   not to the heap.
2. **A 2 MiB collection floor** (was 8 MiB): the churn window between
   collections is the allocator's streaming footprint; 2 MiB keeps it
   inside the LLC. The dial over floors {8M, 2M, 1M, 512K}: fib LL
   misses 149k → 51k → 29k → 19k, but per-collection fixed costs grow
   (capi est cycles 1424M → 1553M → 1763M → 1986M); **2 MiB is the
   knee** — est-cycle geomean ties 8 MiB (247.0M vs 247.4M) while wall,
   RSS and pauses improve sharply.

### Wall time (what users feel), after the fix

Interleaved reps on the shared box, load 1.9–3.7 during these samples
(min of 9; medians and spread in `results/locality-final.json`):

| workload | rc ms | gc ms (2 MiB floor) | rc/gc |
|---|---:|---:|---:|
| binarytrees | 123 | 128 | 0.96× |
| fib | 47 | **34** | **1.38× (gc)** |
| lapply | 9 | 11 | 0.82× |
| listgrowth | 7 | 5 | 1.40× |
| strings | 46 | 49 | 0.94× |
| cycles | 18 | **4** | **4.5× (gc)** |
| capi | 323 | **293** | **1.10× (gc)** |

Before the fix the gc lost fib 0.72× / capi 0.99× / strings 0.63×; after
it the gc wins the three evaluator-shaped workloads outright and the
rest are ties within measurement noise. Wall on this box has 1.5–2×
spread between min and max under load; the deterministic counters above
carry the conclusion.

Peak RSS and longest pause after the fix:

| workload | rc RSS | gc RSS | rc pause | gc pause | rc cols | gc cols |
|---|---:|---:|---:|---:|---:|---:|
| binarytrees | 22.6 MB | 61.4 MB | 5.1 ms | 15–40 ms | 13 | 5 |
| fib | 2.4 MB | 4.5 MB | — | 0.6 ms | 0 | 69 |
| lapply | 13.2 MB | 16.5 MB | — | 2.5 ms | 0 | 3 |
| listgrowth | 10.4 MB | 8.2 MB | — | 0.8 ms | 0 | 2 |
| strings | 16.5 MB | 18.4 MB | — | 3.2 ms | 0 | 2 |
| cycles | 5.0 MB | 5.0 MB | 4.7 ms | 0.2 ms | 22 | 9 |
| capi | 3.4 MB | 4.5 MB | 1.2 ms | 8.7 ms | 234 | 195 |

The 2 MiB floor cut gc RSS by half on the churn workloads (fib
10.9 → 4.5, capi 10.8 → 4.5, cycles 11.1 → 5.0 MB) and its pauses by
4–40× (fib 23.8 → 0.6 ms). binarytrees stays the gc's weak spot: a
10.6 MB live tree × 1.5 growth plus partial-death fragmentation — an
RSS/pause dial (incremental marking), not a manager defect.

## Where the costs live (callgrind, post-optimization)

- rc/binarytrees: workload code + inline refcount traffic (`build`,
  `check_tree`) 39%, allocation 24%, the deferred-free cascade
  (`drain_frees` + `free_object`) 27%, `maybe_purple` 7%, the cycle
  collector itself **5%** (was 53% before the edge-list rewrite and the
  live-scaled trigger).
- gc/binarytrees: workload and allocation dominate; the collector's
  share is mark (proportional to the live tree) plus a sweep that since
  skip-clean only pays for blocks with garbage. The remaining
  *locality* cost is the mark's pointer chase over the live set
  (4.4M LL misses vs rc's 0.6M on this workload).
- The remaining rc overhead is *design-inherent*: every handle read is
  an incref/decref pair (`check_tree` costs 2.4× its gc twin), every
  container decrement feeds the purple buffer, and every death walks
  the object's children to release their counts. A borrowed-read API
  (no owned handle for immediately-dropped reads) would remove part of
  it — see Recommendation §6.

## Safety and the C API

The bug history of this bake-off is itself evidence for the decision:

- **rc** needed: uncounted-handle fixes in every read accessor,
  counting the intern table's own reference, a mid-drain re-entrancy
  guard, parking dead-but-buffered objects so the root buffer never
  validates freed memory, an edge-list collector rewrite, lazy table
  release (address-reuse soundness), and a double-decref of attributes
  on free that no workload exercised. Every heap mutation is a
  refcounting site; the invariants are smeared across the whole
  manager.
- **gc** needed: rebuilding the starts bitmap from survivors, exact
  copy-on-modify counts at `env_take`, purging pooled blocks of released
  chunks, and pruning the weak char table *before* memory can be
  reused. The invariants are concentrated in the collector and the
  sweep — and each is guarded by a test (`MM_GC_MIN` forces tiny heaps).

For `libR.so` (G2) the gc side maps 1:1 onto what C extensions already
do: `R_Protect` is the precise root stack (push only, no counting),
`REAL(x)` stays valid across allocations because objects never move,
and an interior `REAL(x) + i` held across a call is found by the
conservative scan — no stack maps, no recompilation, no porting of
extension code. Under rc, protects must be counted references (or a
flag scheme with park-on-zero) and *every* C-held temporary must be
counted correctly or the count silently corrupts copy-on-modify.

Finalizers (`reg.finalizer`) run at collection time under both designs;
GNU R itself only promises "when the collector runs", so neither loses
compatibility.

## Recommendation

**Adopt (b): the non-moving mark-region heap with conservative stack
scanning, plus copy-on-modify reference counts** (the `mm-gc` design),
with the collection floor at 2 MiB and skip-clean sweeps. The first
bake-off's Ir-based recommendation survives the cache study — but only
after the gc's locality deficit was measured and fixed; the numbers that
settled it:

1. **The wall-time disagreement was real and had one cause.** rc's wins
   on fib/strings/lapply were allocation locality, proven
   deterministically: 12k D1 misses under rc vs 5.4M under the gc on fib
   at the old floor. Instructions alone (1.70× for the gc) ranked
   managers; users feel `Ir + misses`.
2. **The gc's locality deficit is policy, not architecture.** A 2 MiB
   churn window (fits LLC) plus sweeps that cost work proportional to
   garbage made the gc win wall time on fib (34 vs 47 ms), capi (293 vs
   323) and cycles (4 vs 18), halve RSS on churn workloads, and cut
   pauses 4–40× — while est cycles still favor it 1.32× (1.18× even at
   a contended-machine 14/250 miss model) and Ir 1.53×.
3. **rc could not be improved where it is weak.** The purple-trigger
   dial (factor 1/4/8) is worse everywhere (factor 1: cycles RSS
   5 → 35 MB; 4/8: more collections, more instructions, longer
   pauses). Its costs are design-inherent: an incref/decref pair per
   handle read, purple-buffer traffic per container decrement, a child
   walk per death. `check_tree` still costs 2.4× its gc twin.
4. **The C API is the gc's home ground.** PROTECT = precise roots,
   interior pointers resolve, arbitrary C code is safe without stack
   maps — the same contract GNU R gives CRAN. Under rc, protects must
   be counted references and *every* C-held temporary counted correctly
   or copy-on-modify silently corrupts.
5. **What remains against the gc** is binarytrees-shaped: a big live
   set re-marked every collection (LL misses 4.4M vs rc's 0.6M; est
   cycles still favor the gc 1.06×, but only just), 61 MB RSS vs rc's
   23, and 15–40 ms stop-the-world pauses. The known fixes are
   incremental/opportunistic marking and heap-growth tuning —
   mark-region supports both; switching to rc is not one of them.
6. **One borrowing from rc regardless:** the API lesson, not the
   allocator. A `Mem` trait with borrowed reads (read a slot without
   owning a handle) would remove rc's largest design-inherent penalty
   (`generic_elt` + `discard` pairs) *and* help the gc's callers;
   robigo-core's object API should have it from day one.

Confidence: **high** on the mechanism (cache-sim counters are exact and
reproducible: `results/locality-baseline.json`, `results/floor-dial-*`),
**medium** on the wall margins (a shared 2-core box at load 1.9–5.6;
walls reported as interleaved min/median with spread and load recorded —
the gc-vs-rc ordering on fib/capi/cycles was stable across every load
we sampled, the ties elsewhere are within noise).

The rc variant stays compiling in this workspace as the control for
future allocator changes: it pins the shared heap's correctness from
the other side (the checksum cross-check is only meaningful with two
managers).

## Open

- Incremental/opportunistic marking to close the remaining pause gap on
  big-live workloads (binarytrees 15–40 ms; mark-region supports it; the
  current collector is stop-the-world by choice).
- Generational separation. The floor dial captures most of a nursery's
  win for env-per-call churn (the analysis in the ledger: minor
  collections would over-retain young→young garbage held only by dead
  young roots with com > 0, so a real nursery needs remembered sets on
  com-incref sites — worth it only if incremental marking is not
  enough).
- binarytrees RSS (61 MB): growth-policy and fragmentation tuning, or
  line-granularity recycling inside partially-dead blocks.
- The conservative scan's false-positive rate on R numerics was not
  measured directly; the RSS numbers bound its effect (doubles on the
  stack rarely land in heap ranges, and the O(1) resolve makes misses
  cheap).
- A moving/compacting collector is out of scope by design (`REAL(x)`
  stability is a C API promise).
