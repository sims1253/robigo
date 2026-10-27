# Ledger: memory (the memory-manager bake-off)

Track: `docs/workstreams.md` § memory. Work lives in `experiments/memory/`
(its own Cargo workspace, excluded from the root workspace). The full
write-up, numbers and recommendation: `experiments/memory/README.md`.

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
