# Ledger: memory (the memory-manager bake-off)

Track: `docs/workstreams.md` § memory. Work lives in `experiments/memory/`
(its own Cargo workspace, excluded from the root workspace). The full
write-up, numbers and recommendation: `experiments/memory/README.md`.

## 2026-10-27: immortal block counts — skip-clean's accounting made exact (verification of the locality branch)

- why: re-verifying the resumed branch's skip-clean sweeps line by line
  found that `mark_obj` counted every *encounter* of an immortal
  object (`marked += 1`, no per-round guard). Every immortal is
  reached several times per collection — the intern-table roots,
  traced references (cons tags, env bindings holding cached ints) and
  the conservative stack scan all hit the same objects — so blocks of
  cached ints/symbols systematically overcounted, read as dirty, and
  never skipped: the optimization was disabled on exactly the hot
  blocks of fib/capi-shaped loads. Worse, an overcount that happened
  to cancel a dead neighbour's count (`marked == live_objects` by
  accident) made the sweep skip the block and retain the neighbour
  forever — a latent leak the checksum cross-check cannot see.
- fix (kept): `Block::immortals`, counted once at creation
  (`Heap::note_immortal` beside every `set_flags(IMMORTAL)` site);
  `mark_obj` returns on immortals without counting (a flags load and a
  branch); the skip test becomes `marked + immortals == live_objects`;
  the intern-table root-marking loops are gone (9216+ dead mark calls
  per collection on fib). A missed `note_immortal` can only cost a
  walk (safe); an immortal block can never empty, so counts never go
  stale. New regression test: dead mortal ints die next to a
  double-rooted immortal, and a quiet heap's second collection walks
  zero blocks.
- result (`results/skipclean-immortal-count.json`, 5 interleaved reps,
  load ~3.1, before = `results/locality-final.json`): every workload
  improved on every deterministic counter — gc Ir fib 307→294M
  (−4.4%), binarytrees 407→404M, lapply 51→50M, listgrowth 34→33M,
  strings 312→311M, cycles 38→36M, capi 1365→1334M (−2.3%); D1m down
  1–10% everywhere (binarytrees 6.0M→5.7M, capi 18.4M→16.9M). rc/gc
  geomeans: Ir 1.65×→1.69×, est cycles 1.38×→1.42× (1.24×→1.28× at a
  contended 14/250). rc counters reproduce its baseline exactly (the
  shared heap's layout change is rc-invisible), checksums still agree
  on all 7 workloads. Wall at this load: gc ahead on medians for
  binarytrees/fib/strings/cycles/listgrowth, ties on capi/lapply.

## 2026-10-27: the locality study — the wall-time disagreement was cache behaviour; gc fixed, recommendation reaffirmed

- why: the bake-off recommended the gc on Ir (1.70× geomean) but rc won
  wall time on exactly the evaluator-shaped workloads (fib 0.72×,
  lapply 0.80×, strings 0.63×) on a busy shared box. The decision for
  robigo-core must rest on experienced time, so the disagreement had to
  be explained deterministically and, if it was the design's fault,
  fixed.
- measurement added: every workload × manager under callgrind
  `--cache-sim=yes` (bench.py): Ir, D1mr/D1mw, LLmr/LLmw, estimated
  cycles `Ir + 10·D1m + 100·LLm` (recomputable at other penalties with
  `table --d1/--ll`); wall reps interleaved per variant pair with
  min/median/max/spread and `getloadavg` recorded around every run.
- finding (baseline, 8 MiB gc floor): rc recycles cache-hot slots, the
  gc streams fresh memory between collections — fib: 12k D1 misses
  (rc) vs 5.4M (gc); est cycles: binarytrees a dead heat (0.99×),
  lapply 0.92× and strings 0.93× for rc; geomean gc advantage 1.70×
  (Ir) → 1.36× (est cycles). That is rc's wall win, measured
  deterministically.
- fix 1 — skip-clean sweeps (kept): per-block `marked` counters from
  the mark phase (immortal objects and the manager's symbol/int tables
  counted as roots), sweep skips blocks where nothing died; mark colors
  alternate so survivor headers are never rewritten. A collection now
  costs work proportional to garbage, not to the heap. Found and fixed
  on the way: the sweep never added small-block survivors to the
  `live_objects` total (large-only; stats read ~0 without large
  objects) — the two gc tests asserted against that undercount and now
  assert above a captured bootstrap floor.
- fix 2 — 2 MiB collection floor (kept, was 8 MiB): the churn window is
  the allocator's streaming footprint; dial over {8M, 2M, 1M, 512K}:
  fib LL misses 149k→51k→29k→19k but per-collection fixed costs grow
  (capi est cycles 1424M→1553M→1763M→1986M). At the 2 MiB knee the
  est-cycle geomean ties 8 MiB (247.0M vs 247.4M) while wall flips to
  the gc on the evaluator shapes (fib 87→34ms vs rc 47; capi 359→293
  vs rc 323; cycles 7→4 vs rc 18), RSS halves on churn workloads (fib
  10.9→4.5 MB) and pauses drop 4–40× (fib 23.8→0.6 ms).
- reverse direction, improving rc where it is weak (rejected): the
  purple-trigger dial MM_RC_FACTOR ∈ {1,4,8} is worse everywhere —
  factor 1: cycles RSS 5→35 MB, 61 ms pauses; 4/8: more collections
  (binarytrees 13→36/79), more instructions, longer capi pauses. rc's
  weak spots (binarytrees 2.02× Ir, cycles 4.35× Ir) are
  design-inherent; keep factor 2.
- result (after both fixes): gc wins est cycles 1.38× (1.24× at a
  contended-machine 14/250 model), Ir 1.65×, and wall time on
  fib/capi/cycles; ties elsewhere (binarytrees remains rc's cache-miss
  win and the gc's RSS/pause weak spot: 61 MB, 15–40 ms marks).
  **Kept**: recommendation = mark-region gc, 2 MiB floor, skip-clean
  sweeps; the hybrid worth having is borrowed-read APIs + incremental
  marking, not rc.

## 2026-10-27: the bake-off result — recommend the mark-region gc

- why: G2 decides `robigo-core`'s memory manager by measurement; both
  designs had to be made as fast as possible first, then compared on
  callgrind Ir (exact), peak RSS, longest pause, bounded memory on
  cycles.
- result (scale 1.0; Ir exact, wall/pause min-of-7 on a contended box):
  gc needs 1.70× fewer instructions (geomean; 4.52× on cycles, 1.94× on
  binarytrees, 1.68× on fib; rc wins strings 0.96×). Wall times nearly
  cancel outside cycles (rc's dense refcount code is cache-friendlier:
  rc wins fib/strings). RSS strongly favors rc (eager freeing; gc pays
  an 8 MiB floor + 1.5× growth). Pauses at these settings favor rc on
  binarytrees/capi (small trial-deletion batches vs monolithic marks).
  **Kept**: recommendation = (b) mark-region + conservative stack scan +
  copy-on-modify counts; keep rc compiling as the checksum control.
  (Superseded in detail by the locality study above: the "cache-friendlier
  rc" observation is quantified there and the gc's side of it fixed.)

## 2026-10-27: optimizing both managers before comparing

- why: the bake-off is only meaningful if both designs are well
  implemented. Callgrind said rc/binarytrees spent 53% of all
  instructions in collect_cycles, and gc's resolve() could walk
  thousands of granules per non-object stack word.
- rc: edge-list collector (one cold subgraph walk instead of 4–5),
  live-scaled purple trigger (max(4096, live/2), parked objects
  excluded — counting them deadlocked the trigger: 0 collections, 961k
  parked, 92 MB RSS), lazy intern-table release, wide header stores.
  binarytrees Ir 1829M -> 856M.
- gc: O(1) resolve (start-bit mask + leading_zeros), leaner
  alloc_small (no panic edges, inlineable), in-place protection-stack
  iteration, heap growth 2× -> 1.5× live (binarytrees RSS 74 -> 62 MB
  for +11% Ir there).
- rejected: caching the purple trigger across collections (+25% Ir on
  binarytrees — staleness fired collections far too often); 1.25×
  growth (little further RSS gain, +collections everywhere);
  CHUNK_BLOCKS 16 -> 4 (no RSS change — fragmentation is across
  *partially live* chunks, not chunk granularity); a per-block mark
  bitmap to skip survivors' headers in sweep (estimated < 10% of an
  already-smaller collect cost; not worth the layout churn).
- result: kept the first two bullets' changes; see README for the
  final table.

## 2026-10-27: verifying the resumed snapshots

- why: the branch carried two usage-limit WIP snapshots whose state was
  unknown.
- found and fixed: `fib_shaped_envs_survive` asserted a miscomputed
  expected value (the correct checksum is fib(20)+S(20)=53110) and only
  stressed collections via an env var nothing sets — it now collects at
  the recursion leaves; mm-rc's object_size under-counted CHARSXP by
  the NUL padding (live_bytes drift); the WATCH debug scaffolding was
  removed from the scan loop the bake-off measures; bench.py's `table`
  command could not read `run`'s output format.
- result: all unit tests pass, all 7 workloads agree on checksums
  under both managers; kept.

## 2026-10-27: bake-off infrastructure

- why: G2 (`docs/design.md` §5.5) decides the `robigo-core` memory
  manager by measurement: (a) reference counts with a trial-deletion
  cycle collector (Bacon–Rajan) vs (b) a non-moving mark-region heap
  with conservative stack scanning and copy-on-modify-only counts.
- building: `experiments/memory/`, a standalone Cargo workspace with
  `mm-core` (one `Mem` trait both managers implement), `mm-heap`
  (shared block/size-class allocator so the comparison isolates the
  *management* strategy), `mm-rc`, `mm-gc`, `mm-bench` (workloads
  written once, generic over `Mem`), `bench.py` (callgrind + RSS +
  pause harness).
- result: superseded by the entries above; the recommendation is in.
