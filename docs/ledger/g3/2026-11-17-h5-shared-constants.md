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
- No safe API writes a published constant. Sol's review found four that
  could, each reachable with R_NilValue, and each is fixed:
  - `Cx::set_object_bit` and `Cx::set_s4_bit` leave an unchanged bit
    unwritten and refuse to change a shared object's.
  - `Cx::mark_assignment_call` marks only a call, the one type its bit is
    read on.
  - `jit::closure_profile` and `jitcache::cmpfun` assert a closure before
    anything else.
  - `install("")` does not mark the shared `""` as a PRINTNAME.
  `shared.rs`'s `Sync` justification lists these writers.
- The owner check: `Isolate::owns` tests its heap first, then R_NilValue by
  address, then the shared id, all inline. A first form
  (`h == heap || h.is_shared()`) cost s3 +1.2%, and an out-of-line cold
  test of the shared id +1.7%: R_NilValue, now not of any heap, is checked
  about 7 times per s3 iteration, and the call cost every caller registers.
- Verified: the debug tests of robigo-heap, -rtcore, -rt, -rtbuiltins and
  -rtapi; the full `scripts/rt-ci.py` (every mode, 2,800 injected
  failures); Miri on robigo-rtcore (the two-thread test included: no data
  race) and the heap's layout and bitmap tests; `scripts/lint.sh`.
- Measured (this box, release, `scripts/perf.py rt-bars --main`): #811's
  head `b649558e` (sha256 8aa5d877…) against this branch (611e40d8…), each
  built in its own target from its own checkout. 0 failures. s3 +0.25%, the
  other calls rows within ±0.08%, fib +0.01%, binarytrees +0.13%, e7-paste
  +0.00%. Peak RSS is within ±0.8%.
- Review: sol-loop, 4 rounds with gpt-6.1-sol on high (extended for
  concurrency), approved. Rounds 1 to 3 found the writers above, and the
  statistics' allocation types.
- Not here: #816's deferred test that `alloc_node` and `alloc_object` agree.
  This branch predates #816, so it goes in its own PR on main.
