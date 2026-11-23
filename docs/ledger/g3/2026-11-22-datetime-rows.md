## 2026-11-22: B, the datetime rows and strptime's per-element allocations

- What: two report-only rows in `benchmarks/rt/calls-rows.toml` (group
  `datetime`, no bar cites them), and the per-element costs behind them.
  - `datetime-zone-strptime`: strptime of 1e5 strings in Europe/Berlin.
  - `datetime-zone-asposixct`: as.POSIXct of a 1e5-element POSIXlt in the
    same zone.
  - Each call is one time zone session (#954: set_tz, a conversion per
    element, reset_tz).
- Why: on #954, the foreman asked whether the session's per-conversion lock
  explains strptime's 1.47x against GNU R.
  - It does not. The wrapper around glibc's `mktime` is ~99 instructions
    per conversion, 1.4% of the row; the lock is ~4 of them.
  - Most of the gap was per-element work in `do_strptime`:
    - x and the format copied out of their CHARSXPs;
    - both converted again to `Vec<u32>` in `r_strptime` (and copied once
      more when cut at a NUL);
    - each row's zone name copied into its own `Vec`, then made a CHARSXP
      (`mkChar`) again for every row.
- Changes (crates/robigo-rtbuiltins/src/datetime.rs):
  - `r_strptime` converts x into a buffer kept across elements.
  - The format is converted once for the elements it serves
    (`strptime_format`, recomputed when `i % m` moves to another element).
    Its error stays after the input's, as GNU R converts `buf` first.
  - Rows store a zone's place in a small table (`Zones`), and `make_lt`
    makes each name's CHARSXP once. GNU R's per-row `mkChar` finds the
    same cached CHARSXP each time, so the result is identical.
  - `do_as_posixct` reads its components' data once before the loop, not
    through `view()` per element and component, and indexes a full-length
    component without a division.
  - The new case `tests/rt/datetime/strptime-recycle.R` covers:
    - recycled formats (m > 1);
    - NA elements between them;
    - zone names shared by rows;
    - the input-against-format error order and the 1000-character limits.
- Method: `scripts/idiom-vs-r.py --rows`, N = 2 against 4, instructions per
  iteration.
  - CPU: AMD Ryzen 9 5900X; glibc 2.44 for both sides.
  - GNU R is the local 4.6.2 build (`R-4.6.2-build`, `RSCRIPT_BIN` and
    `R_HOME_DIR`), not the oracle image's stock rival: that image has no
    valgrind.
  - robigo-rt binaries: main ae9940b79, sha256 `4ee0d643df12a250…`, and
    this branch, `18184068ac4b968a…`.

  | row | main | this branch | GNU R 4.6.2 | ratio after |
  |---|---:|---:|---:|---:|
  | datetime-zone-strptime | 731.5M | 369.2M | 504.3M | 0.73x |
  | datetime-zone-asposixct | 227.1M | 194.0M | 203.9M | 0.95x |

- What is left in each row:
  - glibc's `mktime` dominates both rows. It re-reads `TZ` through `getenv`
    on every call, 36.8M of either row, and GNU R pays the same.
  - strptime's own parse (`Strptime::internal`) is next.
