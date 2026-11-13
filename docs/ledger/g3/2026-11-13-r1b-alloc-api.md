## 2026-11-13: R1b, collections and in-place writes in `robigo-rtcore`

- What: the second half of R1 (M1 plan §B R1) on R1a's API (PR #642)
  and H2's heap (PR #643): `cx.safepoint(kind)`, `cx.poll_alloc(bytes)`,
  `cx.gc_due()`, `cx.alloc_rooted`, `cx.gc_stats()`;
  `cx.shallow_duplicate`, `cx.unique_mut`, `cx.unique_list`,
  `cx.edit_pair`; `FreshVec::cx`. New modules `collect.rs` and `edit.rs`;
  the isolate gets its poll word. Additive: no R1a signature changes, so
  N1a (#645), C2 and B1 keep building on R1a's API.
- Decisions:
  - **The poll word is the isolate's** (`AtomicU32`, declared after the
    core so that it outlives it), and the heap sets `GC_DUE` there
    (`Core::set_poll_word`). A safepoint is one load and one branch; the
    cold path serves the collection with the isolate's roots (the
    derived-`Trace` `Roots`). Interrupts and pending finalizers will add
    their bits to the same word, so `safepoint` returns `RResult` already
    (always `Ok` today): callers handle the interrupt they will get.
  - **`poll_alloc(bytes)`** is a `PollAlloc` safepoint; `bytes` names the
    loop's chunk (§3 item 4: a builtin that cannot name its chunk does not
    merge) and is debug-asserted below `LARGE`, since a larger chunk is one
    allocation, which `alloc_rooted` serves.
  - **`alloc_rooted<T>(n)`** is H2's `Core::alloc_rooted` (collect first
    if one is due or this request makes one due, check the limits, then
    allocate, as `allocVector3` does), returning a `FreshVec` that keeps
    `&mut Cx` borrowed; `FreshVec::cx()` gives the shared capability for
    the fill. An `AllocError` becomes GNU R's message (`errorcall` with no
    call).
  - **`unique_mut::<T>(x)`** is GNU R's `if (MAYBE_SHARED(x)) x =
    shallow_duplicate(x)`, then a `&mut [T]` (`Unique`): the duplicate
    replaces the object in the root `x`, as GNU R's reassignment does (for
    an `Args` slot this is "the builtin writes its target back into the
    argument window"). GNU R's other rule (`MAYBE_REFERENCED` outside an
    assignment call) stays the caller's, with `maybe_referenced` and
    `shallow_duplicate`. A `tracemem`'d object that would be duplicated
    raises "robigo: 'tracemem' is not implemented yet"; an ALTREP target
    "robigo: 'writing into an ALTREP vector' is not implemented yet".
  - **`unique_list(x)`** (VECSXP, EXPRSXP, STRSXP) takes values as roots
    and stores with the barrier and GNU R's counts (`SET_VECTOR_ELT`,
    `SET_STRING_ELT`, which takes a CHARSXP only).
  - **`edit_pair(x, y, kernel)`**: `x` made unique, then the kernel gets
    `&mut [T]` of `x` and `&[U]` of `y`, or of a Rust-side copy of `y`
    when the two *memory ranges* overlap: GNU R's `if (x == y) y =
    shallow_duplicate(y)`, extended to C-owned (EXTDATA) buffers that
    alias, where object identity would not catch the overlap. The copy
    lives outside the R heap, so it is not observable.
  - **`shallow_duplicate`** ports `duplicate1(x, FALSE)`: the types GNU R
    returns as they are (symbols, environments, promises, CHARSXPs, ...);
    vectors allocated through `alloc_rooted` (collect first) and rooted in
    `out` at once, elements copied (pointer elements each gaining a
    reference), `COPY_TRUELENGTH`; attributes and pairlist spines built
    through `cx.build`, so a refused allocation retries after a
    collection. Closures (their JIT bits are V2's) and OBJSXP raise
    "not implemented yet". A call's head attributes are copied once (GNU R
    copies them twice, the second replacing the first: the same result).
  - **`FreshVec` counts stay exact.** g3.md §2.4 says "new-edge counts
    only, no old-value decrement"; R1a's `set` decrements the value it
    replaces. The old value is the immortal fill (its decrement is a no-op)
    unless the same slot is set twice, and then GNU R's `SET_VECTOR_ELT`
    decrements too: skipping it would leave a count GNU R does not have,
    and an overstated count copies where GNU R writes in place (visible to
    `tracemem` and to address identity). The cost is one load and compare
    per store.
  - Roots, `Out` slots, argument windows and globals count no reference
    (§4 bullet 1): `roots_and_stack_slots_count_nothing`.
  - **CI**: the `rt` job's gctorture pass also sets
    `ROBIGO_GC_STATS=check`. With a collection due almost always, any
    `&Cx` request of `LARGE` bytes or more in the rt crates' tests panics
    where it is made. H2's `a_large_request_while_due_is_counted` makes
    such a request on purpose, so it now pins `StatsMode::Report`.
  - **Pairlist copies poll** (§3 item 4): a spine is input-sized, so
    `shallow_duplicate` copies it in chunks of 1,024 cells (48 KiB) under
    `&Cx`. Between chunks the source position and the copy's last cell are
    in roots (its head is in `out`) and the copy calls `poll_alloc`; a
    refused allocation is answered by `need_gc`, and the copy resumes where
    it stopped. A cell's attributes are copied the same way, recursively.
- Proof (WSL2 workstation, 24 threads, Rust 1.100.0):
  - `cargo test -p robigo-rtcore`: 15 tests in `tests/alloc.rs` (a
    safepoint serving a due collection and doing nothing otherwise; a
    polled loop of 125 MiB of garbage with no overshoot past E4's bound;
    `alloc_rooted` collecting before a large request where a `&Cx` request
    is counted; GNU R's refusal message; duplicates of a named list and of
    a call with their counts, TRUELENGTH, OBJECT, S4 and attributes; a
    2,500-cell pairlist copied in chunks, and serving a due collection
    meanwhile; `unique_mut` in place and through a duplicate; tracemem's
    error; `unique_list` counts and its CHARSXP rule; `edit_pair` with
    `x == y` and with a shared `x`; roots and stack slots counting
    nothing), the 24 API tests and the doc tests.
  - `scripts/rt-ci.py`: release, then `ROBIGO_GCTORTURE=1` with
    `ROBIGO_GC_STATS=check`: all rt crates' tests pass.
  - Miri (`cargo +nightly miri test -p robigo-rtcore --test alloc --test
    api`, nightly 2026-11-01): the 10 R1b tests that do not collect and the
    22 API tests, no undefined behaviour.
- Review (sol-loop, gpt-6.1-sol): round 1 found the pairlist copy
  allocating an input-sized spine under `&Cx` without polling (S1) and
  OBJECT/S4 copied only with attributes (S2, GNU R copies them at
  `duplicate1`'s exit regardless); both fixed as above, with tests.
  Round 2: approved. Round 3, on the follow-ups from #642 and #645 below
  (the heap's drop releasing EXTDATA, the Miri guard): approved.
- Folded in from #642's third review round (OpenCodeReview, deferred to
  R1b by the foreman):
  - **EXTDATA at the heap's end** (medium): GNU R has no EXTDATA objects,
    and at exit it runs only the finalizers registered with `onexit =
    TRUE` (`R_RunExitFinalizers`), freeing nothing since the process ends.
    A robigo isolate can end inside a process that goes on (L6), and an
    EXTDATA release callback frees C-owned memory (no R-visible effect), so
    `Drop for Core` now runs the queued releases nobody took and then the
    release of every EXTDATA object still in the heap, each once;
    `alloc_extdata`'s contract says so. `extdata_release_runs_once_dead`
    checks the live object's release at the drop.
  - **trybuild records** (medium): all 8 cases have committed `.stderr`
    files; a missing record fails (trybuild writes the actual output to the
    target directory's `wip/`, checked by removing one locally, with and
    without `CI`), and a different one fails (seen on #642's CI).
    `every_case_has_its_record` makes a missing record a named failure.
  - **Miri job**: `--locked` on both steps. rust-cache declined: the rt-core
    step adds about 11 s to a job of 1 min 45 s.
  - **The derive's hygiene comment** said definition-site; it is
    `Span::mixed_site()` (local variables at the definition site, the rest
    at the call site), now said so.
  - **`#[trace = "..."]`** gets the derive's own message ("not a form the
    derive reads"), with a compile-fail doctest.
  - **No collection under Miri**, enforced: `begin_collection` panics under
    Miri with the reason (the collector calls the C shim; Miri's
    `thread_stack` stub is empty), instead of aborting as if the heap were
    used from another thread. `under_miri_a_collection_panics_with_its_reason`
    runs under Miri.
  - **Chunk layouts**: `Chunk::layout()` gives the layout a chunk was
    allocated with, from the one helper `try_new_chunk` uses;
    `free_large` and `Drop for Heap` deallocate with it.
- Raised on #645 against R1a's code, folded in here too:
  - `Cx::exit_info` and `Cx::resolve_exit` said "the exit `e` stands for"
    while reading the stack's top: the token is zero-sized and names no
    record, so their docs now say they act on the innermost pending exit,
    which is `e`'s while no later exit is pending. (A token with an index
    would make `RResult<Ref>` two words; C2's parking keeps the rule.)
  - The tests' panic-message helpers fail on a payload that is not a
    string instead of reading it as `""`.
  - The size assertions (tests and `lib.rs`'s `const` block) compare with
    `size_of::<usize>()`; `Slot` keeps 16, an `align(16)` fact.

