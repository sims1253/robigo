## 2026-11-19: H's deferred heap lows (m2m3-plan §A.5): GNU R's errors near the vector limit and for a refused page

- Why: m2m3-plan §A.5 left five heap items from M1 for H, none of them
  dependent on X3's layout. They are:
  - `make_room`'s misreport near the VECREC limit;
  - GNU R's "memory exhausted" for a refused small-object chunk;
  - #689's >1 MiB-frame refusal, which Sol never reviewed;
  - #671's `try_call65` nargs contract;
  - #652's heap test names.
- What:
  - **Near the vector limit.** The poll-only path cannot collect. When a
    collection was due it allocated whenever the request stayed under
    the limit, garbage counted. GNU R collects at that point, and
    `AdjustHeapSize` grows the vector heap only while what is in use, the
    request and `R_MinVFree` (a fifth of the initial heap) stay below
    `R_MaxVSize`. So near a limit GNU R errors where robigo kept going.
    - `HeapSizes::short_after_collection` asks whether GNU R's collection
      could leave the heap short of the request. Garbage is counted, so
      it is an upper bound.
    - `poll_slow` then refuses (`NeedGc`), and the retry collects in
      `make_room`, which decides as GNU R does. Only the slow path changes,
      and only while a vector limit is set.
  - **The misreport.** A retry loop (`Cx::build_with`, `retry_alloc`,
    the compiler's rounds, the weak-reference loops) re-runs its whole
    attempt after `make_room`. Near a limit, the attempt's own earlier
    allocations fill the room again, and the same request is refused
    twice. The loop then said "cannot allocate vector of size ..." (or,
    in the compiler, "robigo: no room to compile an expression"). GNU R,
    which collects at each allocation, reports the limit there.
    - The heap now notes why it refused (`Core::take_refusal`: the vector
      limit, the node limit, or a refused page).
    - `Cx::refused_again`, the one error every retry loop uses, gives GNU
      R's message for that cause: `mem_err_heap`, `mem_err_cons` or
      `mem_err_malloc`. It gives "cannot allocate vector of size ..." only
      when the system refused a large mapping or the size does not fit.
  - **A refused page.** The system's refusal of a chunk for small objects
    aborted the process (`handle_alloc_error`); GNU R's `GetNewPage`
    collects and then raises "memory exhausted" (`mem_err_malloc`).
    - `Heap::alloc` now returns null when it cannot get the chunk.
      `alloc_fast`, the inlined path, is unchanged; the null test follows
      the cold call.
    - `Core::alloc_object` turns the null into `NeedGc` and notes the
      cause. A second refusal after the retry's collection is "memory
      exhausted", in `alloc_rooted` as well.
    - The debug mover's copies, made during a collection, still abort
      (`Heap::alloc_or_abort`).
    - `Heap::refuse_chunks` (`Cx::set_refuse_chunks`) is the test knob.
  - **#671:** `robigo_heap_try_call65`'s `nargs` is already documented
    (0..=65, the stub clamping it to 6..=65 on x86-64), and
    `Cx::call_native65` asserts it in debug builds. Nothing to change.
  - **#652:** `a_six_argument_entrys_locals_are_roots` and
    `jmpbuf_is_the_c_librarys` are renamed.
  - **#689:** its >1 MiB-frame refusal (`Core::alloc_large`, ENVSXP) was
    part of this PR's Sol review (Review below).
- Measured: GNU R 4.6.2 and robigo-rt with a limit 1 Mb past the vector
  trigger, filling it with `numeric(100)`s kept in a list. Rows are the
  VECRECs the requests leave free below the limit:

  | slack | GNU R | robigo, main | robigo, this PR |
  |---|---|---|---|
  | 600,000 | fits | fits | fits |
  | 300,000 | limit error | fits | fits |
  | 250,000 | limit error | fits | fits |
  | 200,000 | limit error | fits | limit error |
  | 150,000 | limit error | fits | limit error |
  | 120,000 | limit error | fits | limit error |
  | 50,000 | limit error | limit error | limit error |

  The boundary still differs, by robigo's smaller heap in use: GNU R holds
  about 128,000 more VECRECs after boot. That is L7's difference
  (2026-11-18-h-memory-internals.md), not this rule's. The case
  `tests/rt/alloc/near-vector-limit.R` sits at 150,000.
- Tests:
  - `tests/rt/alloc/near-vector-limit.R`, recorded on GNU R 4.6.2
    (`.requires volume`: about 55,000 allocations near a full heap).
  - Heap unit tests: the trigger-time refusal near a limit, then
    `make_room`'s `VectorMemory`; a refused page, `take_refusal`'s
    `MemoryExhausted`, `alloc_rooted`'s "memory exhausted", and allocation
    again once chunks are allowed.
  - rtcore: a build that keeps vectors until a page is refused ends in
    "memory exhausted".
- Conformance features: the near-limit case needs three conditions,
  each now a `.requires` feature:
  - GNU R's default heap sizes. The new `default-vsize` is off where
    `R_VSIZE` or `R_MAX_VSIZE` sets them (rt-ci's small vector limit).
  - GNU R's collection schedule. `collection-schedule`, defined as in
    #861, is off under gctorture and retry torture, whose extra
    collections move `AdjustHeapSize`.
  - A normal allocation count (`volume`).

  The list is built with `lapply`, so `ROBIGO_NO_INPLACE` does not copy it
  at every element. #861 adds the same `collection-schedule` lines:
  whichever lands second merges main.
- The refused-page tests skip under `ROBIGO_GC_MOVE`: the debug mover
  copies every object during the collection and aborts without a chunk.
- Verified (2026-11-19):
  - Debug `cargo test` of robigo-heap, -rtcore, -rt, -rtbuiltins and
    -rtapi, and Miri on robigo-rtcore and on robigo-heap's `layout::`,
    `heap::bitmap_tests` and `heap::medium_tests`, all passed on
    `9f8aeae30`. Since then only tests, conformance features and a merge
    of main changed.
  - The full `scripts/rt-ci.py` passed on `7b5486228`: every mode, and
    2,800 injected failures with no difference.
  - robigo-conformance's tests and `scripts/lint.sh` passed.
  - After merging main (`747b7ee61`), robigo-conformance's tests and the
    `tests/rt/alloc` cases passed again.
- Measured (perf, `747b7ee61` against main `eec051b83`, each built in its
  own checkout and target): rt-bars, 0 failures. No row is worse than
  +0.03% instructions (plain +0.00%, env-fn +0.02%, s3 −0.02%,
  e7-paste −0.23%).
- Review: sol-loop, 2 rounds with gpt-6.1-sol on high, approved.
  - Round 1 found the near-limit check unreachable once a collection was
    due: `retrigger` raised the vector trigger to the limit. It is now
    `HeapSizes::due_v_trigger`, with a regression test (nodes make the
    collection due, then vectors cross `v_size`) that fails without the
    fix.
  - Round 2 also reviewed #689's >1 MiB-frame refusal. Sound: large
    frames refuse before a collection, keep their capacity in `NeedGc`,
    and are the first allocation after it.
