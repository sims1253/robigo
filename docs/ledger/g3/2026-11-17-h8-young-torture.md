## 2026-11-17: H8, the moving collector's young torture (`ROBIGO_GC_MOVE=young:N`)

- Why: under `ROBIGO_GC_MOVE=1` with gctorture every collection moved the
  whole live heap. `tests/r/memory/finalizers-self-ref-envs.R` made 2.41M
  collections of a ~5k-object heap: ~470 s here, ~536 s for the step on CI,
  marginal against the 600 s timeout (A1's INTERIM(H8) override, 1200 s).
  Measured at 1/50 scale: a plain torture collection 30 µs, a moving one
  207 µs (5,038 objects moved each): the moving was the cost.
- Decision (foreman, 2026-11-17): option (b) of the note
  (`~/.cache/robigo-research/h8-generational-torture-note.md`), a
  torture-only policy; generations stay M8's decision.
- What (`crates/robigo-heap/src/gc/mover.rs`):
  - `ROBIGO_GC_MOVE=young:N`: every N-th moving collection moves everything
    as before; the others move only the young objects, those no collection
    has moved yet (the header's free flag bit, `mover::OLD`, clear on every
    fresh object, set on every copy).
  - A young collection takes the pins first (`pin_segments`: the C
    segments' words pin what they may point at, and mark nothing), then
    marks as always, with one difference: the first time the marker reaches
    a young object that may move, it copies it and rewrites the edge it
    came through, and every later edge to the old copy gets the copy
    (`mark_young`). Every edge is visited by the marking, so there is no
    barrier and no second (forwarding) pass. The heap's own lists (precious
    set, weak references and their fields, the finalizer queue) take the new
    addresses as they are marked; the weak pass reads keys through
    forwarding; the CHARSXP cache's entries of moved strings are replaced in
    place (`CharTable::replace`); then the old copies are poisoned and freed.
  - The marking functions are generic over the policy (`const Y: bool`):
    the shipped path is the `false` instance, today's code. Both instances
    are wrapped in robigo-heap's own functions (`mark_heap_roots`,
    `mark_heap_roots_young`, ...), and the young state is one boxed field
    (`mover::Young`): the debug collector adds nothing to the core's layout.
  - `mark_young`'s fast path is `mark_obj`'s (null, immortal or marked
    returns; an old object is marked inline); only a young object or an old
    copy takes a call.
- Tests (heap.rs): a stale local to a young object reads poison at every
  young collection; an old object stays through the young ones and moves at
  the N-th; a pinned young object stays and its fields are forwarded; a
  weak reference whose young key moved keeps it and is finalized when it
  dies; the CHARSXP cache gives the moved string. robigo-rt's two identity
  tests skip under any moving policy, not only `=1`.
- CI: rt-ci's conformance step under gctorture moves with `young:N`; the
  crates' tests under gctorture keep `ROBIGO_GC_MOVE=1`. The nightly
  (`.github/workflows/nightly.yml`, job `rt-full-move`, when main moved)
  runs the slice with `=1`, `rt-ci.py --full-move`, so a path that runs
  once is still caught by some run: there the slow case keeps its 1200 s
  (`FULL_MOVE_SLOW`, no longer interim).
- N by measurement (this box, release, `scripts/rt-ci.py`'s conformance
  step command and environment; the case alone with `ROBIGO_GC_STATS=1`;
  the desktop was shared, load 5-14, so CPU time is the steadier number):

  | mode | the case | the whole step (wall) | step CPU | step CPU / gctorture |
  |---|---:|---:|---:|---:|
  | gctorture | 82 s | 103 s | 204 s | 1.00x |
  | young:8 | 166 s | 230 s | 461 s | 2.26x |
  | young:16 | 141 s | 149 s | 297 s | 1.45x |
  | young:32 | 105 s | 141 s | 282 s | 1.38x |
  | `=1` (every collection) | 560 s | 734 s | 1470 s | 7.20x |

  Every step passed the same 462/470 (the ratchet's). The case moved 12.1G
  objects under `=1`, 764M under young:16 (its 150,590 full collections
  account for nearly all of them: a young collection copied ~5 objects).
  A larger N is cheaper (fewer full collections) and catches a stale
  pointer to an old object less often, so the choice is the smallest N
  within ~1.5x of gctorture: **young:16** (`GC_MOVE_YOUNG`).
  The case runs in 141 s under it, well inside the default 600 s, so the
  INTERIM(H8) override goes from the job's step.
- The first young marking cost 2.5x gctorture: `mark_young` was a call on
  every edge (callgrind at 1/200 scale: 12.6G instructions against
  gctorture's 6.0G). With `mark_obj`'s fast path inline (an old object
  marked in place, only young objects and old copies calling out) and an
  edge written only when it changed: 7.85G, 1.31x. Most of the rest is the
  sweep walking the blocks a moved young object left a dead copy in.
- A perf lesson: the first build cost `s3` +0.89% instructions (18,840 to
  19,008, with nothing in the shipped marking changed). Bisected over the
  commits, then over the pieces: a third monomorphized instance of the
  embedder's root walk (`Roots::trace::<MarkYoung>`, besides `Mark` and the
  full mover's `Forward`) in robigo-rtcore's single codegen unit, which
  moved LLVM's inlining there (`usemethod` and `dispatch_method` inlined
  into `scope` closures). Both moving modes now walk the roots through one
  `trace::<dyn Visitor>` instance: `s3` 18,848.
- Measured (this box, release, `scripts/perf.py rt-bars --main`), main
  `ca0aa238` against this branch `2372446f`, each built in its own target
  from its own checkout, sha256 checked to differ: 0 failures, every row
  within ±0.06% (`s3` -0.00%).
- Verified: the rt crates' debug tests (53 binaries), and all of them
  under `ROBIGO_GCTORTURE=1 ROBIGO_GC_MOVE=young:16` (before the last
  refactors); the full `scripts/rt-ci.py` (every mode, the new `young:16`
  step, 800 injected failures); `scripts/test-rt-ci.py`; Miri on
  robigo-rtcore and the heap's layout and bitmap tests; `scripts/lint.sh`;
  actionlint on the nightly.
- Review: sol-loop, 2 rounds with gpt-6.1-sol on high, approved (no
  findings in either).
