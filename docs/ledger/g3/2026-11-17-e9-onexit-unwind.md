## 2026-11-17: on.exit code run from a freed list after a refused allocation

- What: A1's E9 work found a hang on the production collector.
  - `tests/rt/conditions/onexit-unwind.R` with `ROBIGO_FAIL_ALLOC_AT=1249`
    printed "on.exit caught: inner", then looped until killed (a segfault
    on S3x-a's binary).
  - Under A1's prototype knob `ROBIGO_RETRY_TORTURE=1`, which refuses the
    first allocation of every `build_with`/`retry_alloc` once
    (`~/.cache/robigo-research/e9-retry-torture/`):
    - on the production collector, `capi/e8-unwind`, `onexit-unwind` and
      `parse/file` died;
    - under `ROBIGO_GC_MOVE`, `lang/on-exit`, `capi/e8-unwind`,
      `onexit-unwind` and `trycatch-finally` failed with "unimplemented
      type 'unknown' in 'eval'".
- **Cause** (gdb: the segfault in `run_conexit`, rooting a dead object).
  `endcontext` at an exit's intermediate stop (`Cx::end_at_stop`) read the
  record's `on.exit` list, cleared `conexit` (GNU R's "prevent recursion"),
  then called `R_FixupExitingHandlerResult`, which allocates the saved
  message. Between the clear and the run, nothing referenced the list.
  - A collection at that allocation, which a refused allocation forces,
    freed the list on the production collector, and its cells were then
    reused: the on.exit code ran from garbage, or not at all.
  - Under the moving collector, the list moved and the old address was
    run.
  - GNU R protects it there: `PROTECT(s)` in context.c:endcontext.
- **Fix:** both `endcontext` paths (`end_at_stop`, and `end_function`, where
  nothing allocates in that window today) root the list before they clear
  `conexit`, and run it from the root.
- Tests:
  - robigo-rtcore's
    `contexts::the_on_exit_list_survives_a_collection_in_the_stops_fixup`
    pins the window: an `exit_value` hook that collects and then reuses
    node slots, as a refused allocation's collection would. Without the
    fix the on.exit code does not run (`left: []`); with it, both
    expressions run in order.
  - `scripts/rt-fail-alloc.py`'s default scripts gain the five R cases
    above, so `rt-ci`'s E9 step samples 400 points of each.
- Proof:
  - At `ROBIGO_FAIL_ALLOC_AT=1249`, `onexit-unwind.R` now ends with GNU R's
    output and status.
  - Under the prototype knob (applied to a test build only), all five
    cases pass on both collectors. Without the fix, every one of A1's
    failures reproduces on the same build.
  - `scripts/rt-fail-alloc.py` at every allocation finds no difference:
    `onexit-unwind` 12,020 points, `trycatch-finally` 8,903,
    `capi/e8-unwind` 9,655, `lang/on-exit` 8,824, `parse/file` 13,991.
    Under `ROBIGO_GC_MOVE`, 2,000 points of each of the first four find
    none either.
