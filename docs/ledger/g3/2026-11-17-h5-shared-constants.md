## 2026-11-17: H5, step 2: the process's shared immutable constants

- Why (m2m3-plan §H5, docs/g3.md §5.3; design note
  `~/.cache/robigo-research/h5-design.md`): symbols become process-shared
  in the next step. Every object's header holds its heap's `R_NilValue`
  (the attribute word) and a symbol's TAG holds it too, so the shared
  `R_NilValue` must come first. The note's step order changed to: 1.
  symbols immutable (#811); 2. these constants; 3. the arena.
- What (`crates/robigo-heap/src/shared.rs`):
  - `R_NilValue`, `NA_STRING`, `R_BlankString`, `R_UnboundValue` and
    `R_MissingArg` are one set per process, in a 64 KiB-aligned segment
    made once (`OnceLock`) and never written after it is published. They
    are built exactly as `Core::new` built its own: the same headers, gp
    bits, hashes and symbol ids (0 and 1).
  - The segment's pseudo block header holds a static sentinel's address in
    the remembered-set word, so `Sexp::heap_id` answers "shared"
    (`HeapId::is_shared`). Its magic is not a block's.
  - Each `Core` points at them. Its CHARSXP cache is seeded with the shared
    `""`, and its own symbol ids start at 2. It counts the five objects as
    if it had allocated them (the poll and the count of
    `alloc_object`), and the sweep adds them to `nodes_in_use`, so
    `gc()`'s numbers are what they were.
  - A store into a shared object is refused in the barrier's slow path,
    before anything is written. `Isolate::owns` accepts shared objects,
    on its cold path only: the owner check is one compare as before.
- Deferred from #811's review (the foreman's list):
  - `flags::IMMORTAL` documents its contract: not traced until a store
    passes the barrier, no count, `SEEN_BY_C` or `MARK_NOT_MUTABLE` writes,
    and no store at all into a shared constant.
  - `set_attrib` joins the debug assertion that no barriered store takes a
    symbol.
  - A heap-level test of `set_symbol_bits`
    (`a_new_symbol_gets_its_names_bits_when_it_is_made`).
- Tests:
  - robigo-heap: `the_constants_are_shared_by_every_heap` (two heaps and a
    heap made on another thread; the cache's `""`; a collection writes no
    constant; the counts), `a_store_into_a_shared_object_is_refused`. Two
    tests that took the constants for one heap's changed:
    `every_object_carries_its_heaps_id` (they are shared), and
    `a_store_into_an_immortal_char_does_not_hide_dead_cache_entries`
    (a heap's own immortal CHARSXP).
  - robigo-rtcore: `isolates_on_two_threads_share_the_constants`, two
    isolates made at the same moment on two threads, also under Miri.
- Verified: (pending)
- Measured: (pending)
- Review: (pending)
