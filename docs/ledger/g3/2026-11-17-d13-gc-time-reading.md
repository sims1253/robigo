## 2026-11-17: D13's collection-time condition, read as total time per program

- **Replaces** one condition of D13's bar (`2026-11-17-m2m3-bars.md`,
  "D13: node sizes (X3)"): "collection time per MB allocated is at most 3%
  worse". The rest of that bar stands. No run has cited the bar yet, and
  this entry is committed before X3's runs.
- **Why:** the condition is there to stop `nT` making collections more
  costly. Divided by each cell's own allocated bytes, it charges `nT` for
  allocating less: its nodes are smaller, so the same program allocates
  fewer bytes. If `nT` allocates 17% fewer bytes on `plist.R` and its
  collections take the same time, its time per MB is 20% worse
  (`2026-11-17-h-gc-time-line.md`). The foreman decided this on
  2026-11-17.
- **The condition now:** `nT`'s total collection time on `plist.R` and on
  `henv.R` is at most 3% above `n48`'s on the same program. Each program
  is compared on its own. A cell's time is the median over the runs of
  `ROBIGO_GC_STATS`'s "collection time" (the total in ns). This equals
  dividing both cells' times by `n48`'s heap bytes.
- The report's "ns per MiB" stays a report and decides nothing.
