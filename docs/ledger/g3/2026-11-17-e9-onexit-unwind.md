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
- L's case `tests/rt/conditions/onexit-handler-result.R` (cherry-picked
  from `g3/l2-onexit-case`, recorded on 4.6.2 and ratcheted) covers the
  same window through the handler-result path, with `parse()` of a file.
  It segfaults under the mode without the fix. L's own fix of the
  `parse/file.R` segfault was this window, and L dropped it.
- Proof:
  - At `ROBIGO_FAIL_ALLOC_AT=1249`, `onexit-unwind.R` now ends with GNU R's
    output and status.
  - Under A1's prototype knob (applied to a test build only), all five
    cases and L's pass on both collectors. Without the fix, every one of
    A1's failures reproduces on the same build.
  - With main's `retry-torture` feature (#780,
    `2026-11-17-retry-torture.md`), the conformance slice under
    `ROBIGO_RETRY_TORTURE=1` on the production collector fails no
    ratcheted case: 473/481. The 8 failing cases fail on main without the
    mode too, and none of them is ratcheted: the three
    `assign/link-walk-*` and the `bytecode` ones. Under `ROBIGO_GC_MOVE=1`
    with the mode, it is the same: 473/481, the same 8. So the mode's CI
    steps can come (A1's).
  - The full `scripts/rt-ci.py`, the debug tests of rtcore, rt,
    rtbuiltins and rtapi, Miri on robigo-rtcore and the perf-rt gate
    (0 failures, every row within ±0.14% of main's binary) pass on the fix
    at main `48044290`, before the rebase onto `81eeb1ab` (#780).
  - `scripts/rt-fail-alloc.py` at every allocation finds no difference:
    `onexit-unwind` 12,020 points, `trycatch-finally` 8,903,
    `capi/e8-unwind` 9,655, `lang/on-exit` 8,824, `parse/file` 13,991.
    Under `ROBIGO_GC_MOVE`, 2,000 points of each of the first four find
    none either.
