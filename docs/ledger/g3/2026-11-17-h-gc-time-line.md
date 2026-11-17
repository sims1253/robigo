## 2026-11-17: `ROBIGO_GC_STATS` reports collection time (X3's first prerequisite)

- Why: D13's bar (`2026-11-17-m2m3-bars.md`) asks that `nT`'s collection
  time per MB allocated be at most 3% worse, and says a run without a time
  line in the report is preliminary. robigo-heap already summed each
  collection's wall time (`collect_total_ns`, `collect_max_pause_ns`) but
  never printed it.
- What: one line after the collection counts,

  ```
  robigo gc: collection time 116821585 ns, max pause 29136582 ns, heap bytes allocated 195319824, 627157 ns per MiB
  ```

  - the time runs from the start of marking to the end of the sweep and
    the next trigger; the quarantine a collection releases first is not
    counted;
  - "heap bytes allocated" is the heap's own count (`count`'s bytes, what
    triggers collections), up to the report, so including what came after
    the last collection;
  - the heap test of the report checks the line's four numbers against
    each other.
- The denominator is a choice D13's runs depend on. It counts the heap's
  bytes, so a smaller node allocates fewer of them: if `nT` cuts
  `plist.R`'s bytes by 17% and its collections take the same time, its ns
  per MiB is 20% worse. The foreman settled it: D13 compares total
  collection time on the same program (`2026-11-17-d13-gc-time-reading.md`),
  and "ns per MiB" stays a report.
- Measured (this box, release, `scripts/perf.py rt-bars --main`): main
  `908cb765` (sha256 001325ce…, build id 69e70762…) against `754413b2`
  (49a89d42…, 11cddb43…), each built in its own target from its own
  checkout: 0 failures, every row ±0.00%, peak RSS within ±0.3%.
- Review: sol-loop, 1 round with gpt-6.1-sol on high, approved with no
  findings.
