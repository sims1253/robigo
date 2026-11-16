## 2026-11-16: M1 exit: the lead's decisions on E1's wall time, E8's oracle cases and estimated cycles

Three decisions by Max, as lead, on 2026-11-16, made after the council's
first round on the M1 exit evidence (`2026-11-16-m1-exit-evidence.md`,
#732).

### (a) E1's wall time: a non-PGO binary is accepted for M1

- **The rule:** `docs/g3.md` §13 asks for wall time on "the shipped (PGO)
  build". robigo-rt has no PGO build path: `scripts/pgo-build.sh`
  profiles and builds only the old runtime (`-p robigo-cli`).
- **Decision:** a non-PGO binary is accepted for M1, as a lead exception.
  §13's full protocol, PGO included, applies from M3.
- **The medians this accepts:** `calls.R` one process, 11 interleaved
  rounds, the plain release build of main `c536597d`
  (`data/2026-11-16-m1-exit-c536597d/wall13.json`):

  | runtime | median | robigo-rt ÷ |
  |---|---:|---:|
  | robigo-rt | 0.321 s | |
  | GNU R 4.6.2 (oracle image) | 0.305 s | 1.050 |
  | Ř | 1.051 s | 0.305 |
  | FastR | 9.159 s | 0.035 |

  E1's bar, below Ř's and FastR's, is met outside the 2% tie. Against
  GNU R, the 1.050x is within this run's noise (robigo-rt's samples range
  from 0.273 s to 0.381 s at a load average of 4-9), so it is not
  established either way.

### (b) E8: GNU R-recorded cases, now

- **The gap:** E8's expectations are hand-written Rust assertions, which
  GNU R never recorded and which never run with inlining off.
- **Decision:** E8 gets GNU R-recorded cases now. They block M1's sign-off,
  but not #732's merge.
- **What they are:**
  - the `on.exit` unwind case, as a `tests/rt` R case recorded on the
    4.6.2 oracle and ratcheted;
  - `R_UnwindProtect` and `cend` across a C frame, with their C built for
    GNU R (`scripts/rt-capi-build.sh --gnur`, or the capi oracle path), so
    that GNU R gives the expectations.
- **How:** the Rust tests stay. The cases run in every profile and under
  `ROBIGO_NO_INLINE=1`, through the full rt-ci. They are one small PR,
  `g3/a1-e8-oracle`, after #732.

### Also blocking M1's sign-off: C1c, the link-count fix

C1c is owned by C1, on branch `g3/c1c-link-walk`.
- **The bug:** robigo-rt counts a link for every value-stack slot,
  builtins' Rust roots included. GNU R's `INCLNK_stack_commit` counts only
  byte-code node-stack values. A value that GNU R modifies in place is
  therefore copied.
- **The case:**

      x <- c("a","b","c")
      g <- function(...) .Internal(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0, TRUE))
      print(g())

  GNU R 4.6.2 prints `a Z c`, and robigo-rt on `c536597d` prints `a b c`.
- **The fix:** with it, by C1's report, the case prints `a Z c` and is
  ratcheted, and rt-ci and `perf-rt` pass.
- **Status:** listed with E8's cases in the exit evidence's "Blocking M1's
  sign-off".

### (c) Estimated cycles: a study before any gate change

- **The lead's words:** "The instruction gate was only meant as a proxy to
  improve walltime on a number of different hardware platforms. If number
  of cycles would be the better number, we should at least look at it."
- **Decision:** after (b), a study, with its own ledger entry. Does
  callgrind's estimated cycles (`--cycles`) predict wall time across
  hardware better than instructions do?
- **The rows:** those where the two disagree:
  - the D5 recycling re-runs;
  - the e7 rows;
  - fib and binarytrees;
  - the `calls.R` rows.
- **The data:**
  - pairs of commits, as changes;
  - wall time under §13's protocol on at least two CPUs: this desktop's
    5900X, m0pad, and a CI runner if wall time can be had there;
  - estimated cycles' run-to-run noise, and their time cost in
    `perf-rt`.
- **The output:** a recommendation (gate, report-only or nothing), with
  numbers. No gate changes until Max has read it. A report-only
  estimated-cycles column in `perf-rt` may be filed with the study, if it
  is cheap and helps collect the data.
