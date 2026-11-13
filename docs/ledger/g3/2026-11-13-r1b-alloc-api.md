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
- Proof (WSL2 workstation, Rust 1.100.0): see the PR.
