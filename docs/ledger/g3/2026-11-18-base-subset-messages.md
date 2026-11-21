## 2026-11-18: base-subset's failures say which of two reasons, and what to run

- **Why:** agents adding a `tests/rt` case read base-subset's red check as
  a failure of their code. The foreman's queue item (A1).
- **The two reasons, each with its own message:**
  - **The list is stale:** the inputs now reach other base names. This is
    the common case: a new conformance case in one of base-subset.R's
    `tests/rt` areas. `base-subset.sh --check` now checks all three files
    before failing (it stopped at the first). For each one,
    scripts/base-subset-stale.py lists the names gained, with the inputs
    that reach them (from `reached-by.txt`), and the names lost. It says
    nothing in the code failed and gives the command to run and the files
    to commit. The full diffs go to `.robigo/base-subset/*.diff`.
  - **The traced run differs from the untraced one:** the case prints
    something the tracer changes, and regenerating cannot fix it. The
    message names the stream, the first differing line on both sides and
    the saved transcripts, says why it happens, and gives the two ways out.
  - A third, rarer one: an input exits with another status than its
    recording. The message now says so and gives the command that
    re-records it.
- **Earlier warning:** `scripts/regen-expected.sh` (GNU R 4.6.2 recording
  tests/rt) notes when a case it recorded is a base-subset input. It asks
  base-subset.R itself (`Rscript scripts/base-subset.R --inputs`), so the
  list of inputs stays in one place. It does not regenerate the subset:
  that takes about 7 minutes (400 s here), too long for every recording.
- **CI:** the base-subset job runs scripts/test-base-subset-stale.py, and
  its artifact now also keeps the diffs and the mismatch transcripts.
- **Proof:** both failures were reproduced end to end with throwaway
  cases, recorded and then deleted:
  - `tests/rt/env/zz-probe-stale.R` calls chartr. The report listed
    `+ closure chartr character.R (reached by env:zz-probe-stale)` and
    `+ internal chartr` for base-subset.txt and base-bound.txt, with exit 1.
  - `tests/rt/env/zz-probe-traced.R` prints `deparse(body(paste0))`. The
    report gave stdout line 1, `.Internal(paste0(...))` untraced and `{`
    traced.

  `base-subset.sh --check` passes on main. The first run found a `set -e`
  trap: `diff` exits 1 when files differ, which ended the loop after the
  first file.
