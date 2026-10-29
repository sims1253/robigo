# Ledger: startup-perf (time to run a script)

Metric: callgrind instructions (wall time and peak RSS as secondary,
recorded in the robigo-oracle:4.5.0 container) for robigo vs `Rscript`,
per scenario. Machine notes: this box runs ~7 agents; wall numbers are
medians under load and indicative only; instructions are the gate.
Earlier history (the lazy-base work, PR #36; the natives work, PR #78)
is in those branches until they land.

## 2026-10-29: what is left of PR #36 on top of core's #56

- Core's #56 (merged as d709287) loads base from base.rdb with a reused
  inflater, bulk unserialize reads and its own guard fix, absorbing most
  of PR #36. Re-measured both remaining pieces on main 6ddb7dc:
  - the rdb **file cache** (R's readRawFromFile) is ~neutral now
    (e1-def 272.68M -> 272.51M; stats-lm-null +0.4M, within noise):
    per-fetch open+seek+read was ~1-2k instructions and #56 removed the
    big per-fetch costs it used to sit next to. Dropped from this PR;
    the lazy-cache test and flush stay in #36's branch for the record.
  - the **flate2 `zlib-rs` backend** still wins: miniz_oxide's
    decompress_vec costs more per byte than zlib-rs even with one
    reset Decompress. Kept (this PR).
- `-e 1` instructions (container, R 4.5.0-3; GNU R = Rscript):

  | scenario | main | this PR | GNU R | ratio |
  |---|---|---|---|---|
  | NULL packages | 50.93M | **48.42M** | 88.19M | 0.55x |
  | default packages | 272.64M | **266.59M** | 275.93M | 0.966x |

- gates: conformance 3 tiers; fastr serialize/connections/gzfile slices
  451/458, 0 regressions (2 new); perf suite vs main's binary: 73
  faster, 0 slower, 0 broken (profiler-shared watched specifically
  after the memchr layout lesson — clean).
