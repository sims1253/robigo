## 2026-11-13: H2, the collection protocol in `robigo-heap` (steps 1–5 and 7)

- What: `robigo-heap`'s allocation never collects, collections take the
  embedder's roots as a typed `Trace` value, and the conservative scan
  reads only C segments (docs/g3.md §3; m1-plan H2 steps 1–5 and 7). Step
  6, the pin placement, waits for M0's experiment A verdict: today's order
  stays the default, `ROBIGO_GC_PIN_ONLY=1` the target, in one `if` of
  `Core::finish_collection` that the verdict reduces to its first arm.
- Step 1, poll-only allocation. `alloc_object` and the large path test two
  trigger words (`HeapSizes::over_trigger`: `nodes_in_use >= n_trigger ||
  valloc + cells > v_trigger`) and take a cold slow path past them. The slow
  path checks the limits, counts by type under `ROBIGO_GC_STATS`, runs
  gctorture's countdown, and when GNU R would collect before this
  allocation (`NO_FREE_NODES() || VHEAP_FREE() < alloc_size`, or
  `FORCE_GC`) sets `GC_DUE` in the poll word and starts the overshoot
  count. It never collects (a debug assertion in it checks that the
  collection count is unchanged). While a collection is due the triggers
  move to the limits, so the fast test stays one compare pair; under
  gctorture or the statistics they are 0, so every allocation takes the
  slow path. `gc_inhibit` and GNU R's `postpone` fold into this: every
  due collection is a postponed one, `HeapSizes::pending` and `postpone`
  are gone, and `gc_inhibit` now only makes safepoints decline (the heap
  sizes are no longer grown while a collection waits: they are robigo's,
  and the `gc()` report simulates GNU R's, §3 "Collection observables").
  gctorture's countdown waits while a collection is due, as `FORCE_GC`'s
  `gc_pending ||` does.
- The poll word: `Core::poll_word()` is the heap's own `AtomicU32`, or the
  embedder's (`unsafe fn set_poll_word`), so the facade's safepoint is one
  load and one branch on a word it also uses for interrupts and pending
  finalizers. The heap sets and clears only `GC_DUE` (bit 0), atomically,
  since a signal handler may set another bit.
- Step 2, `Core::alloc_rooted(t, n, roots)`: GNU R's allocVector3 where a
  collection may run. It collects first if a collection is due,
  gctorture's countdown runs out, or the request would make one due
  (`due(size)`; memory.c:2709-2714), then checks the limits (`mem_err_cons`,
  `mem_err_heap`), allocates, and collects once more if the system refuses
  a large mapping (`R_gc_no_finalizers`). With collections inhibited it
  checks only the vector limit and leaves the collection due, as
  `R_gc_internal`'s `!R_GCEnabled` branch does. `Core::make_room(req,
  roots)` is the same for a request the poll-only path refused.
- Step 3, failure. Every allocation under `&Cx` (`alloc_vector`,
  `alloc_node`, `alloc_extdata`, `mk_char`, `install`) returns
  `Result<Sexp, NeedGc>`. `NeedGc` is R1a's `robigo_rtcore::NeedGc`
  (the request: `sexptype()`, `length()`), packed into one word so that
  the result comes back in two registers (24 bytes went through memory,
  measured below); the facade re-exports it. It is returned past `R_MaxVSize`/`R_MaxNSize`, counting garbage (only a
  collection can tell it apart), for a refused large mapping, and for a size
  past the address space. `try_alloc_vector` merged into `alloc_vector`.
  The `&mut Cx` paths return `AllocError` (`VectorMemory`, `ConsMemory`,
  `CannotAllocate`) with GNU R's messages verbatim (`message()`). The limits
  are new in the heap: `set_max_vsize`/`set_max_nsize` port
  `R_SetMaxVSize`/`R_SetMaxNSize`, and `R_MAX_VSIZE` is read with
  startup.c's checks, as is macOS's default (the larger of physical memory
  and 16 Gb). A refused 1 MiB chunk for small objects still aborts, as
  before: GNU R's `mem_err_malloc` for a page is not ported.
- Step 4, `ROBIGO_GC_STATS`: unset or `0` off, `1` report, `check` report
  and panic at the offending request. Always counted (per collection, no
  allocation cost): collections per `Safepoint` kind (entry, loop, return,
  poll_alloc, alloc_rooted, retry, export, explicit); the overshoot,
  bytes from the allocation that set `GC_DUE` to the collection that
  served it (max, its live heap, total, and the collections over E4's
  max(32 MiB, 10% of the live heap), `overshoot_bound`); `&Cx` requests
  of `LARGE` (1 MiB) or more made while a collection was due (the large
  path only). Counted only with the statistics on (every allocation then
  takes the slow path): objects and bytes by SEXPTYPE, the bar section's
  "allocations per iteration". The report goes to stderr when the `Core`
  is dropped.
- Step 5, typed roots. `add_root_fn` and `Core::mark` are gone. Every
  collection entry (`collect`, `safepoint`, `alloc_rooted`, `make_room`)
  takes `roots: &mut impl Trace` and is an `unsafe fn` (the caller vouches
  that every edge is null or live). `Trace::trace(&mut self, v: &mut impl
  Visitor)` shows each pointer word as `&mut Sexp`: the marker reads it,
  H3's moving collector will rewrite it through the same impls. A trace
  cannot reach the heap (the visitor gives no access), so the old
  "a root callback cannot allocate/collect" runtime panics are now
  impossible to write and their two tests went. Impls: `Sexp`, the
  primitives and `String` (no edges), `Option`, `Box`, `Vec`, `VecDeque`,
  slices and arrays, `Cell`, `RefCell`, `UnsafeCell` (through `get_mut`),
  `&mut T` and tuples of up to six (`collect(&mut (&mut a, &mut b))`).
  `#[derive(Trace)]` is the new proc-macro crate `robigo-heap-derive`
  (syn 3.0.9, already in the lockfile; `#![forbid(unsafe_code)]`):
  structs, tuple structs, enums, generics (a `Trace` bound per type
  parameter); `#[trace(skip = "reason")]` with a required reason; a raw
  pointer field without it does not compile (a `compile_fail` doctest),
  nor does a skip without a reason. The per-type arms of the object
  trace moved to one dispatch point, `Sexp::trace_fields` (`trace.rs`),
  which the mark loop calls with the marker; N1, C1 and V1 add arms there
  with rows in `each_type_traces_exactly_its_pointer_words`, the test
  pattern ("an object of a kind shows exactly its pointer words").
- Step 7, C segments. `scan_top` defaults to 0 (no open segment). The
  `.Call` trampoline opens a segment at every native entry: on x86-64
  the stub sets its top just above the routine's stack arguments (before,
  it only narrowed a segment that was already open above it); elsewhere,
  and with `ROBIGO_CALL65_PLAIN=1`, at the trampoline's frame address; it
  restores the previous top as the routine returns or jumps. An export
  closes it first thing (`close_scan_segment`: the node in its frame is the
  low end, above it only the registers its prologue saved, C's values),
  and reopens it as it returns. `scan_segments` (was `scan_stack`) reads
  the closed segments only; a collection with a segment open panics. No
  Rust frame is scanned, the collector's included. Stub narrowing and
  zeroing are kept. Not done here: the six-argument trampolines (ALTREP
  methods, `R_ToplevelExec`, C finalizers) do not open a segment yet,
  so the C frames they run are scanned only through an export's segment
  above them; C2 reworks those trampolines into `CTXT_CCODE` records and
  opens it there.
- Tests (`tests/heap.rs`, 58; `tests/trace.rs`, 3, no FFI; 11 unit; 3
  doctests, 2 of them `compile_fail`), against H1's 41 + 11. Rewritten
  as "sets due, collects at the safepoint":
  `the_node_count_reaching_r_nsize_makes_a_collection_due`,
  `a_full_vector_heap_makes_a_collection_due`,
  `an_inhibited_collection_waits_for_the_first_safepoint_after`,
  `gctorture_counts_allocations` (one safepoint per allocation). New:
  `allocation_never_collects_under_gctorture` (vectors small and large,
  nodes, CHARSXPs, symbols, external data), `the_poll_word_carries_gc_due`,
  `the_overshoot_runs_from_due_to_the_serving_collection`,
  `a_large_request_while_due_is_counted` (and panics under `check`),
  `statistics_count_allocations_by_type_and_collections_by_kind`,
  `allocation_errors_say_what_gnu_r_says`, `need_gc_is_one_word`,
  `a_vector_limit_refuses_under_cx_and_collects_under_mut_cx`,
  `a_node_limit_refuses_under_cx_and_fails_on_live_nodes`,
  `alloc_rooted_collects_before_a_request_that_crosses_the_trigger`,
  `each_type_traces_exactly_its_pointer_words`, `derived_roots_keep_their_fields`,
  `rust_locals_keep_nothing`, `c_words_keep_objects_alive`,
  `a_collection_with_a_c_segment_open_panics`, `a_key_only_a_c_word_reaches`
  (finalizer ready under pins, kept under today's order; memory valid in
  both). `native_call_frames_keep_nothing` is ported from robigo-capi
  (the dead object is a weak reference's key; the routine collects through
  an export stand-in that closes its segment), and
  `stack_parameters_of_a_routine_are_roots` now runs with the segment the
  trampoline opens. The tests that read Rust frames (interior, one past
  the end, the heap's top, the next object's address) pass their words as a
  routine's stack arguments instead (`robigo_heap_test_collect65`).
  Without stray Rust words the counts are exact: `rooted_objects_survive`
  keeps exactly 2,001 objects, all 10 dead EXTDATA are released, the
  weak-reference test queues exactly one.
- Sensitivity, checked by breaking the code: with no segment opened at
  the native entry (a null slot), 7 tests fail (the C-word ones and
  `stack_parameters...`); with `ROBIGO_CALL65_PLAIN=1` under today's order
  and the skip removed, `native_call_frames_keep_nothing` fails, as
  experiment A found ("stub off, old order"); with
  `ROBIGO_GC_PIN_ONLY=1` it passes either way.
- Runs (WSL2 workstation, Ryzen 9 5900X, 24 threads, rustc 1.100.0):
  `cargo test --release -p robigo-heap -p robigo-heap-derive`,
  `ROBIGO_GCTORTURE=1`, `ROBIGO_GC_PIN_ONLY=1`, and `ROBIGO_GC_PIN_ONLY=1
  ROBIGO_CALL65_PLAIN=1`: all pass. `cargo clippy --all-targets -D
  warnings` on both crates: clean. Miri (nightly 2026-11-01):
  `tests/trace.rs` clean; the lib's unit tests (allocation through
  `Core::new`, CHARSXPs) need R1a's `thread_stack` under Miri (`2a223654`):
  with it applied locally (not committed), the 9 chars and heapsize tests
  pass, and Miri then reports the heap's 1 MiB chunks leaked at exit, which
  R1a's `Drop for Heap` frees. CI's miri job does not cover robigo-heap
  yet (R1a adds robigo-rtcore, which runs the heap's allocation).
- Cost: callgrind instructions per iteration, N = 10^6 against 2N, of two
  loops on the heap alone (no runtime yet runs on it): `alloc`, a scalar
  INTSXP and a LISTSXP node holding it, all garbage; `live`, a rooted
  200,000-node list, then one garbage node per iteration (each collection
  marks the list). Base: robigo-heap at `ddb09e33` (H1; allocation
  collects; the stack scan on, which roots the vector across the node's
  allocation). Head: this branch, with a safepoint per iteration as a
  builtin's return makes one (a load and a branch on the poll word, then
  `safepoint`). Both make the same number of collections (5 and 3 at N).
  Release builds (codegen-units 1, thin LTO), valgrind 3.25.1, the
  workstation below; two runs each, head identical, base within 0.002:

  | loop | base | head | change |
  |---|---|---|---|
  | `alloc` | 218.53 | 211.40 | -3.3% |
  | `live` | 123.99 | 124.24 | +0.2% |

  The first head build was +7.5% on `live`: `Result<Sexp, NeedGc>` was 24
  bytes, returned through memory, and nodes tested the vector trigger. One
  word for `NeedGc` and no vector test for objects without vector data
  (`HeapSizes::over_trigger`) brought it to the table above. The fast test
  itself is cheaper than H1's (two compares against `pending`, gctorture's
  counter, the node count and the free vector heap). Marking through
  `trace_fields` and the marker costs what H1's inline arms did (40.0 M
  against 41.0 M instructions in `trace` at 2N).
- R1a (`g3/r1a-cx-api`, not filed): its heap changes (`HeapId`,
  `Core::id`, `Sexp::heap_id`, `Drop for Heap`, `thread_stack` under Miri)
  are not in this PR. Against this branch, `heap.rs` and the `gc.rs` hunks
  apply cleanly; `lib.rs` conflicts on the `pub use gc::{..}` line, and
  its appended test needs `.unwrap()` on the now fallible allocations. For
  rtcore: `try_alloc_vector(t, n).ok_or(NeedGc{..})` becomes
  `alloc_vector(t, n)`, `alloc_node`/`mk_char`/`install` return `Result`
  (the slots its comment reserves for them), `robigo_heap::NeedGc`
  replaces `exit::NeedGc` (same fields), `AllocError::message()` replaces
  `alloc_failure_message`, and `gc_inhibit = 1`/`set_scan_top(0)` are no
  longer needed (allocation never collects; 0 is the default).
