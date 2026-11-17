## 2026-11-17: a longer timeout for one case in the moving collector's gctorture step, until H8

The foreman's decision. It is interim: H8, H's generational torture,
removes it.

### Why

`tests/r/memory/finalizers-self-ref-envs.R`, ratcheted by BW, is marginal
in rt-ci's "conformance under ROBIGO_GC_MOVE with gctorture (E3)" step.
- **The work:** it makes about 2.4 million collections there. Under
  `ROBIGO_GC_MOVE=1` each collection moves the whole live heap, about 5,000
  objects. That takes about 470 s locally.
- **The margin:** main's whole step takes about 536 s on CI, against the
  step's 600 s per-case timeout (`ROBIGO_CONF_TIMEOUT`). GitHub's runners
  vary by about 13%, so a slow runner fails the case with identical work.
- **#769 failed that way.** L proved every `ROBIGO_GC_STATS` counter equal
  to main's: the same work, on a slower runner.

The case stays in the step, so its coverage under the moving collector is
kept. Only its timeout changes, and only there. A global bump would give
every case in every collector step room to slow down unnoticed.

### What changes

- **The runner** (`crates/robigo-conformance`): `ROBIGO_CONF_TIMEOUTS=CASE=SECS,...`
  gives the named cases a longer timeout than `ROBIGO_CONF_TIMEOUT`.
  - A run refuses an entry that names no file, so a renamed or deleted
    case cannot leave a stale override behind.
  - The value is parsed strictly: a malformed pair, a non-number or a case
    named twice is an error.
- **`scripts/rt-ci.py`:** `GC_MOVE_SLOW`, marked `INTERIM(H8)`, maps
  `tests/r/memory/finalizers-self-ref-envs.R` to 1,200 s. Only the "GC_MOVE
  with gctorture" conformance step passes it. Every other step and case
  keeps 600 s, or the runner's 20 s.
- **`scripts/test-rt-ci.py`** checks three things:
  - exactly that step carries the overrides;
  - each named case exists;
  - each override is longer than the step's own timeout.

### When it goes

H8 makes a gctorture collection cost the young objects, not the whole live
heap. Then the case's step time falls well under 600 s, and `GC_MOVE_SLOW`
is deleted with its runner mechanism, or kept for the next case that needs
it, as H8 decides.

### Proof

- **The case with the override:** a release build ran it alone under
  `ROBIGO_GCTORTURE=1 ROBIGO_GC_MOVE=1`, with `ROBIGO_CONF_TIMEOUT=600` and
  the step's `ROBIGO_CONF_TIMEOUTS`. It passed, with stdout, stderr and the
  exit status matching GNU R 4.6.2.
  - It took 506 s on the desktop, which other agents were also using (load
    about 10 on 24 threads).
  - That is 84% of the old 600 s limit, for this one case alone.
- **A stale entry:** `ROBIGO_CONF_TIMEOUTS=tests/r/nope.R=10` stops the run
  before any case starts: "ROBIGO_CONF_TIMEOUTS names tests/r/nope.R, which
  is no case".
- **The tests:**
  - `cargo test -p robigo-conformance`: 4 library tests and 18 runner tests
    pass. The library tests include the new parser test.
  - `scripts/test-rt-ci.py` passes, with its new test.
