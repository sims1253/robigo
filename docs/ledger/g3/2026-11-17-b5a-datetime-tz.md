## 2026-11-17: datetime.c's time zones: GNU R's internal tzcode, ported (B5a, B5-tz)

- What: B5a ports datetime.c's `strptime` and `as.POSIXct`
  (`crates/robigo-rtbuiltins/src/datetime.rs`) for UTC and GMT only. In any
  other time zone, robigo-rt raises "robigo: ... is not implemented yet".
  The foreman's decision on what replaces that refusal is recorded here.
- How GNU R converts: on Linux, GNU R is built without
  `USE_INTERNAL_MKTIME` (`PATH 1`). It converts local time with the C
  library's `mktime` and `localtime_r`, after it sets `TZ` in the process
  environment and calls `tzset` (`set_tz`, `reset_tz`). robigo-rt does not
  do that, with or without a lock:
  - `setenv` is `unsafe` in Rust 2024;
  - a lock cannot stop another thread's `getenv`;
  - the environment and `tzset`'s state are process globals, which the
    isolate model (several isolates per process) rules out.
- The decision: robigo-rt uses the configuration GNU R builds on macOS and
  Windows, `USE_INTERNAL_MKTIME`. That is GNU R's own tzcode
  (`src/extra/tzone`: `localtime.c`, `strftime.c`, and the `R_mktime`,
  `R_localtime_r`, `R_gmtime_r` and `R_tzset` entry points that datetime.c
  calls in `PATH 2`). It reads the tzfile database directly, with explicit
  state and no process globals, so the port can keep its state per isolate.
  - It is its own package, **B5-tz**, after B5a's first wave.
  - Its bar: oracle cases across several zones, including DST transitions
    and the spring-forward gap.
- What this means for results: robigo's local-time results equal those of
  a GNU R built with the system's tzcode wherever both read the same
  zoneinfo. GNU R's internal tzcode and glibc both implement the tzfile
  format and POSIX `TZ` strings over the same database. A difference
  between the two would be a difference between GNU R's own builds, and
  the oracle (a Linux build, `PATH 1`) decides it.
- Until B5-tz lands, UTC and GMT need no zone data. robigo-rt computes
  them as GNU R does on every platform (`timegm00`, `gmtime_r`), and they
  are what GNU R's build, the oracle and the conformance tests use
  (`TZ=UTC`).
