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
global frame) and then **conservatively scans the Rust stack and
registers** (a small `global_asm` shim spills them); interior pointers
are resolved to object starts in O(1) with a start-bit scan. Handles
are plain `Copy` pointers; reference counts exist **only for
copy-on-modify** (heap-to-heap references and bindings, exactly GNU R's
post-4.0 REFCNT), so `MAYBE_SHARED`-style decisions are exact while
Rust locals cost nothing.

Both use the same 16-byte header layout, the same size classes, and
the same allocator, so allocation speed itself is held constant and the
measurement isolates the *management* strategy.

## Running it

```sh
cargo test --release                 # unit tests, both managers
cargo build --release
python3 bench.py run --reps 5        # wall/RSS/pause + callgrind Ir
python3 bench.py table results/*.json
```

`MM_RC_FACTOR` (purple buffer divisor, default 2), `MM_GC_MIN` (GC
floor, default 8 MiB) are experiment knobs.

## The numbers

Machine: 2 shared cores (measurements taken while other agents worked —
wall times and pauses below are min-of-7-reps, the least contended
sample; **Ir is exact and load-independent** and is the primary
metric). Scale 1.0, release build, `lto`, `panic=abort`.

Throughput — total instructions (callgrind Ir):

| workload | rc (M) | gc (M) | rc/gc |
|---|---:|---:|---:|
| binarytrees | 856 | 442 | **1.94×** |
| fib | 497 | 295 | **1.68×** |
| lapply | 66 | 56 | 1.18× |
| listgrowth | 43 | 28 | 1.52× |
| strings | 293 | 304 | **0.96× (rc wins)** |
| cycles | 163 | 36 | **4.52×** |
| capi | 1998 | 1240 | 1.61× |
| *geometric mean* | | | *1.70×* |

Wall time (min of 7) — note how it diverges from Ir:

| workload | rc ms | gc ms | rc/gc |
|---|---:|---:|---:|
| binarytrees | 130 | 116 | 1.12× |
| fib | 43 | 60 | **0.72× (rc wins)** |
| lapply | 8 | 10 | 0.80× |
| listgrowth | 6 | 4 | 1.50× |
| strings | 38 | 60 | **0.63× (rc wins)** |
| cycles | 30 | 10 | 3.00× |
| capi | 408 | 414 | 0.99× |

Reference counting executes ~1.7× more instructions but they are dense,
predictable, cache-local increments on the objects being built; the
gc's fewer instructions are spread over a heap 3–6× larger with
mark-phase pointer chasing. On this shared, cache-squeezed machine that
almost exactly cancels the instruction gap outside `cycles` — a
reminder that Ir ranks managers, wall times rank systems.

Peak RSS and longest collector pause:

| workload | rc RSS | gc RSS | rc pause | gc pause | rc cols | gc cols |
|---|---:|---:|---:|---:|---:|---:|
| binarytrees | 22.6 MB | 61.6 MB | 5.1 ms | 15.5 ms | 13 | 4 |
| fib | 2.4 MB | 10.9 MB | — | 0.9 ms | 0 | 17 |
| lapply | 13.3 MB | 18.0 MB | — | 1.8 ms | 0 | 2 |
| listgrowth | 10.4 MB | 8.9 MB | — | — | 0 | 0 |
| strings | 16.9 MB | 17.8 MB | — | — | 0 | 0 |
| cycles | 5.0 MB | 11.1 MB | 1.05 ms | 1.0 ms | 22 | 3 |
| capi | 3.4 MB | 10.9 MB | 4.2 ms | 5.2 ms | 234 | 47 |

Both managers hold memory bounded on `cycles` (the workload asserts
< 4 MB live after collection). RC's RSS advantage is eager reclamation
plus a small retention window; the gc's 8 MiB floor and 1.5× growth
policy cost 4–40 MB depending on churn (2× growth cost 74 MB peak RSS
on binarytrees for −11% Ir — a dial, not a law). RC's *pause* advantage
on binarytrees/capi is real at these settings: trial deletion processes
~65k-candidate batches while the gc's mark is monolithic over the whole
live set; an incremental/incremental-sweep gc would close it.

## Where the costs live (callgrind, post-optimization)

- rc/binarytrees: workload code + inline refcount traffic (`build`,
  `check_tree`) 39%, allocation 24%, the deferred-free cascade
  (`drain_frees` + `free_object`) 27%, `maybe_purple` 7%, the cycle
  collector itself **5%** (was 53% before the edge-list rewrite and the
  live-scaled trigger).
- gc/binarytrees: workload 39%, allocation 29%, mark+sweep+scan 31%.
- The remaining rc overhead is *design-inherent*: every handle read is
  an incref/decref pair (`check_tree` costs 2.4× its gc twin), every
  container decrement feeds the purple buffer, and every death walks
  the object's children to release their counts.

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
with these caveats and borrowings:

1. **Evaluator throughput is what robigo is.** The interpreter's hot
   loop is fib/lapply/cycles shapes: environments per call, lists,
   cycles. There the gc needs 1.2–4.5× fewer instructions, and — more
   important for a Rust runtime — handles are `Copy`: no decref glue on
   every value that flows through a match arm. The 1.68× on `fib` is a
   direct preview of the cost of counted handles in every builtin.
2. **The C API is the gc's home ground.** PROTECT = precise roots,
   interior pointers resolve, arbitrary C code is safe without stack
   maps. This is the same contract GNU R gives CRAN.
3. **RSS is a tuning dial, not a property.** The 8 MiB floor and 1.5×
   growth are policy; 1.25× saves another ~10% peak RSS for +11% Ir.
   Keep RC's one good habit where it applies: eagerly reusable free
   lists (Immix already does) and a lazy intern table.
4. **Pauses at these settings favor rc on churn-heavy workloads**
   (5.1 vs 15.5 ms on binarytrees). If real workloads ever make that
   matter, the fix is incremental marking / opportunistic sweeps
   (mark-region is designed for it), not a switch to rc.

The rc variant should be kept compiling in this workspace as the
control for future allocator changes: it pins the shared heap's
correctness from the other side (the checksum cross-check is only
meaningful with two managers).

## Open

- Incremental/opportunistic marking to close the pause gap (mark-region
  supports it; the current collector is stop-the-world by choice).
- Generational separation (a nursery for env-per-call frames) — measured
  here only indirectly via `MM_GC_MIN`.
- The conservative scan's false-positive rate on R numerics was not
  measured directly; the RSS numbers bound its effect (doubles on the
  stack rarely land in heap ranges, and the O(1) resolve makes misses
  cheap).
- A moving/compacting collector is out of scope by design (`REAL(x)`
  stability is a C API promise).
