## 2026-11-17: finalizers in GNU R's RunFinalizers order across collections

- Why (found reviewing #767, the foreman's follow-up): GNU R keeps a
  ready weak reference in `R_weak_refs` (newest first) until its finalizer
  runs, and one `RunFinalizers` pass walks that list once from its head,
  not reentered while a finalizer runs (memory.c). robigo moved ready
  references to a FIFO queue and ran it until empty. Two differences
  follow, both in `tests/rt/builtins/finalizer-order-batches.R` (recorded
  on 4.6.2):
  - of two references readied by two collections inside one finalizer,
    GNU R runs the newer first (3 before 1); robigo ran the one readied
    first;
  - a reference newer than the running finalizer, readied during it,
    waits for GNU R's next pass (after the next collection); robigo ran it
    in the same pass.
- What (a port of memory.c's model):
  - a ready reference stays in `weak_refs` (oldest first: `R_weak_refs`
    reversed), flagged `READY_TO_FINALIZE`; the queue is gone;
  - a pass takes, each time, the newest ready reference older than the
    last it took (`Core::pop_finalizable_before`), so one readied or
    registered above the walk waits;
  - `R_finalizers_pending` is a flag again: set by each collection that
    leaves a reference ready (CheckFinalizers), and by the exit finalizers;
    cleared when a pass ends (`Core::end_finalizer_pass`);
  - the collector's weak pass treats ready references as GNU R's does: their
    fields are not roots before the classification, so a key that only a
    ready reference's finalizer reaches is dead for its own reference (it
    was kept live before; a heap test now pins GNU R's answer); every
    reference keeps its fields for the round.
- A taken reference leaves a null entry until the pass ends, compacted
  then: removing it shifted every newer entry, which made a pass quadratic
  (50k dead references under 50k live ones: 0.64 s; 200k: 6.2 s; now
  0.39 s and 1.66 s, linear).
- GNU R's singly linked list can lose a reference registered during a
  pass (`last == R_NilValue` overwrites `R_weak_refs`); robigo keeps it, as
  before (`a_reference_registered_during_a_finalizer_batch_waits_for_the_next_round`).
- Measured (this box, release, `scripts/perf.py rt-bars --main`): the
  branch against its base (#797's head), each built in its own target from
  its own checkout, sha256 checked to differ: 0 failures, every row within
  ±0.04%.
- Verified: both finalizer-order cases match GNU R 4.6.2 (the new one
  ratcheted); the full `scripts/rt-ci.py` (every mode, 800 injected
  failures); the rt crates' debug tests (53 binaries); Miri on
  robigo-rtcore and the heap's layout and bitmap tests; `scripts/lint.sh`.
- Review: sol-loop, 1 round with gpt-6.1-sol on high, approved (it also ran
  both cases with the moving collector off, `=1` and `young:3`, and an
  erroring finalizer mid-pass).
