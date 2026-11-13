## 2026-11-13: robigo-rt's conformance column and the traced base subset (G1b)

- why: L1 (its own ratchet column), §14.3 (gated on 4.6.2) and §14.2 M1
  (the base functions `calls.R` reaches, loaded from the files
  `crates/robigo-rt/proto/base-subset.txt` names). G1 left these until the
  4.6.2 oracle landed (PRs #638, #639).
- runner, decision: one implementation. The old conformance test's runner
  moved into a crate, `robigo-conformance`, that both runtimes' tests
  depend on (dev-dependency): a `Column` is a binary's gated version,
  suites and ratchet directory. `OLD` is `robigo`, 4.5.0, `tests/r`,
  `passing/`, exactly as before; `RT` is `robigo-rt`, 4.6.2, all of
  `tests/rt` plus the `tests/r` cases it has marked, `passing-rt/` in each
  suite. Rejected: a parallel `scripts/rt-conf.py` (m1-plan's name), which
  would duplicate recording selection, ratchet rules and the byte
  comparison in a second language; a `#[path]` include of robigo-cli's
  test file, which hides a cross-crate dependency the CI selection must
  see; running `RT` from robigo-cli's test with `ROBIGO_CONF_BIN`, which
  would build the old runtime in the `rt` job. The crate is not an rt
  crate (robigo-cli uses it), so a change to it runs everything.
- runner, rules: an `R-V/` recording counts only as the full set
  `.Rout`/`.Rerr`/`.Rstatus`/`.Rsha` with the script's current hash
  (4.5.0's beside each script still need only `.Rout`: `tests/r/io/memcompress-xz`
  has no `.Rstatus`). In a gated run a case without one fails the run; in
  another version's run it is "not recorded", and a run that compared
  nothing fails. `ROBIGO_R_VERSION` is read as an `OsStr`: a non-UTF-8
  value is an error, not the gated default. `RT` runs the `tests/r` cases
  it has not marked only under `ROBIGO_CONF_SWEEP=1`, so its ratchet holds
  without running 600 cases it fails. `RT` may have an empty ratchet
  (`empty_ratchet_ok`) until L1 adds the first marker.
- runner, checks: 18 fixture tests in `crates/robigo-conformance/tests/runner.rs`
  with two fake binaries (`cat`, and a stub that fails like today's
  `robigo-rt`): stub fails every case and ratchets nothing; a pass
  ratchets only under `ROBIGO_CONF_UPDATE` and then a failure of it fails
  the run; marked `tests/r` cases run, the sweep runs the rest; missing,
  partial and stale recordings; another version; filters; the SHA-256
  against `sha256sum`. On this tree (local, x86_64, quick profile,
  `R_HOME` the 4.5.0 oracle's, `ROBIGO_NATIVES=/nonexistent`): `OLD`
  606/628 pass with the one local-only failure `graphics/cairo-devices.R`
  (no grDevices natives here, passes on CI), as on main; `OLD` against
  4.6.2 577/628 (report only); `RT` against the stub: `tests/rt/smoke/one.R`
  fails, nothing ratchets, the test passes; `RT` sweep: 0/629.
- recording: `scripts/regen-expected.sh` records `tests/rt` for 4.6.2 only
  (`tests/rt/R-4.6.2/`) and refuses it for any other version.
  `tests/rt/smoke/one.R` (`1`, GNU R 4.6.2: `[1] 1`) is the first case.
  The `rt` CI job (`scripts/rt-ci.py`) runs the `RT` column as a step of
  its own; the release and gctorture steps skip it (`--skip
  rt_conformance`), so it is not run three times. Conformance under
  gctorture is one of §16's modes and is left to T1, when cases run.
- base subset, method: `scripts/base-subset.sh` runs each M1 script on the
  4.6.2 image twice, untraced and traced (`scripts/base-subset-trace.R`, a
  user profile). After GNU R's start-up the tracer rewrites every base
  closure's body: before each statement, and at the start of each branch,
  loop body and switch arm, one `.Internal(list2env(...))` of constants
  records the closure and the primitives and internals written there. A
  task callback does the same for top-level code and instruments the
  script's own closures as they are bound. The arguments of closure calls
  are not rewritten (only their blocks and function literals), because a
  closure can deparse them. JIT off. The two runs must print the same bytes
  and exit alike (masking only the protect-stack depth of GNU R's "stack
  imbalance" warning, which capi `extptr`'s C triggers in both); a self-test
  fixture with an untaken branch and an uncalled closure runs first. The
  capi C is built with 4.6.2's `R CMD SHLIB`; `pairlist.c` gets a macro for
  `SET_TYPEOF` (gone from 4.6's API, in a function the case does not call).
  Calls a capi case's C builds or parses and evaluates run no R code that
  records them, so every primitive its C names in `install("...")` or in a
  string given to `R_ParseVector` counts too (`sqrt`, `+`, `*`; the
  generator checks `sqrt`). The four capi cases run with their natives, so this is more than "their
  R logic where runnable without natives". `weak-keys-collections.R` is not
  an input (foreman answer 3: A1's rt variant joins later). Two
  regenerations gave identical files; one run takes 12 s.
- base subset, result: 44 closures, 63 primitives, 33 internals.
  calls.R + fib + binarytrees alone reach 23 closures: `cat`, `new.env`,
  `structure`, `stop` (in `safe`), `tryCatch`, `seq` and what they call.
- base subset against m1-plan §A.5's static estimate: the generator also
  computes A.5's static bound on the 4.6.2 image (free variables,
  transitively through base closures, as `codetools::findGlobals`, which
  the image lacks; `.robigo/base-subset/{static,traced-only,static-only}.txt`).
  For the seven inputs it gives 77 closures, 78 primitives, 55 internals
  against the trace's 44/63/33; 122 entries are in both. The same walk,
  run once by hand on the three benchmarks alone, gives 70/76/49 (A.5: 69
  closures, about 80 primitives), and with weak-keys added 101/84/68 (A.5:
  100, about 90, 68), so it reproduces A.5.
  - Only the trace has 9 closures, 7 primitives and 2 internals (`pmin`,
    `print.default`): `sqrt`, which capi `eval` and `pairlist` call from C;
    S3 methods reached by
    dispatch (`print.default`, `seq.default`, `as.list.default`,
    `conditionMessage.condition`) and what they call (`pairlist`,
    `as.pairlist`, `chkDots`, `pmin`, `mostattributes<-`, `/`, `abs`,
    `is.finite`, `is.integer`), and replacement functions (`$<-`,
    `attributes<-`), which a free-variable walk sees as `$`, `attributes`.
  - Only the static walk has 42 closures, 22 primitives and 24 internals,
    all on branches these scripts do not take: warnings and condition
    formatting (`warning`, `gettextf`, `sprintf`, `deparse`, `conditionCall`,
    `sQuote`/`dQuote`, `ngettext`, `l10n_info`, `iconv`), `cat`'s
    connection paths (`pipe`, `close`, `stderr`), `withRestarts` and the
    restart internals, `do.call`, `match.arg`/`pmatch`, `%in%`, `unique`,
    `startsWith`, `substring`, `ifelse`, `rapply`.
  - Consequence for B2b (m1-plan): `match.arg`, `pmatch`, `do.call`,
    `sprintf`, `startsWith`, `substring`, `unique`, `%in%`, `ngettext` and
    `l10n_info` are not reached by M1's scripts as they run on GNU R; they
    are needed only when a script warns or takes an error path these do
    not. The trace is the floor M1 must load; the static list stays the
    bound for "a function referenced from a loaded one".
