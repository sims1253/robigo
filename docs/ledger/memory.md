# Ledger: memory (the memory-manager bake-off)

Track: `docs/workstreams.md` § memory. Work lives in `experiments/memory/`
(its own Cargo workspace, excluded from the root workspace).

## 2026-10-27: bake-off infrastructure

- why: G2 (`docs/design.md` §5.5) decides the `robigo-core` memory manager by
  measurement: (a) reference counts with a trial-deletion cycle collector
  (Bacon–Rajan) vs (b) a non-moving mark-region heap with conservative stack
  scanning and copy-on-modify-only counts.
- building: `experiments/memory/`, a standalone Cargo workspace with
  `mm-core` (one `Mem` trait both managers implement), `mm-heap` (shared
  block/size-class allocator so the comparison isolates the *management*
  strategy), `mm-rc`, `mm-gc`, `mm-bench` (workloads written once, generic
  over `Mem`), `scripts/bench.py` (callgrind + RSS + pause harness).
- result: in progress.
