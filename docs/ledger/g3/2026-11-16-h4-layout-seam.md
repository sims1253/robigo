## 2026-11-16: H4, the layout seam

- What: m2m3-plan H4 (docs/g3.md §8, the appendix's layout row). Base:
  `main` at `d25d0586`. A pure refactor: no object moves a byte.
  - **`robigo_heap::layout`** owns every per-type size, offset and header
    bit. `layout/defs.rs` has the offsets per body (`header`, `vector`,
    `charsxp`, `extdata`, `node` and its views `sym`, `env`, `promise`,
    `extptr`, `altrep`, `slots`), the sizes (`GRANULE`, `node::size(t)`,
    `vector_bytes`, `charsxp_bytes`, `data_off(t)`) and the header bits;
    `layout/access.rs` has `Sexp`'s field accessors over them (moved from
    lib.rs). The heap's allocator, collector, mover and trace arms, rtcore,
    rt, rtbuiltins and rtapi read them; `slots` re-exports its offsets.
  - **Per-type bit maps.** `flags` stays one map (one byte, all 8 bits
    used). GNU R's `gp` and robigo's `extra` are one map per type
    (`gp::{charsxp, vector, weakref, sym, cell, call, env, closure,
    promise}`, `extra::{closure, promise, cell, env, vector, code}`), each
    with a compile-time no-overlap check. The rt crates' own gp masks
    (`ACTIVE_BINDING`, `FRAME_LOCK`, `NOJIT_MASK`, `DDVAL`, `MISSING_MASK`,
    `PRSEEN_*`, ...) are now imports of the layout's. `extra::TYPE_BLIND`
    names the bits code tests without the type (`SEEN_BY_C`, and the cell
    code-source marks the setters' write barrier tests on any node): every
    other map keeps them clear.
  - **The freed marker:** `extra::code::WEAK_CACHE` (bit 8 of a code
    object's VECSXP map), reserved for C1's weak cache words. Before, all 16
    `extra` bits were taken; now each map lists its free bits (a closure
    6 and 8-12, 15; a cell 0-3, 6, 8-12, 14, 15; ...).
  - **C reads the same numbers.** robigo-heap's build.rs includes
    `layout/defs.rs` as a module and writes `robigo_layout.h` (one
    `ROBIGO_<name>` per entry of `layout::TABLE`, and an X-macro of them)
    into `OUT_DIR/include`; `links = "robigo_heap"` hands the directory to
    robigo-rtcore's and robigo-rtapi's build scripts. api.c's `RB_OFF_*`
    are the header's; its `robigo_rtapi_layout` is the header's table,
    which `check_layout` compares entry by entry with `layout::TABLE`.
    Offsets are asserted on both sides against a struct view: Rust
    `#[repr(C)]` `HeaderView`/`VectorView`/`NodeView` with `offset_of!`
    (and a fresh header's first word, `header::first_word`),
    C `_Static_assert(offsetof(...))` in api.c. Defn.h defines GNU R's gp
    masks from the header and asserts them against GNU R 4.6.2's literals.
  - **Miri** runs robigo-heap's non-FFI tests in CI's miri job: the
    layout's (pinned numbers, sizes, no-overlap, views) and the block
    bitmaps' (start bits, interior-word resolution, size classes). Miri
    found one Stacked Borrows error in the first view test (a reference
    held across raw writes), fixed.
- Grep: no raw `+16`/`+24`/`+32`/`+40` header offset outside `layout`
  (crates/robigo-{heap,rtcore,rt,rtapi,rtbuiltins}, `.rs`/`.c`/`.h`); the
  heap's tests that pinned numbers now name the layout's constants, and
  `layout::tests::the_layout_is_h32_n48` pins today's numbers in one place.
- Measured (this box, release, `scripts/perf.py rt-bars --main`, main's
  binary built from `d25d0586`): every row within ±0.01% of main in
  instructions (e7-match −0.01%, every other row 0.00%); peak RSS within
  ±0.5%. 0 failures. A first cut moved s3 +0.08% and binarytrees +0.03%:
  callgrind put it in `alloc_node` (an argument-passed `nil` made the header
  store precede, and may alias, its reload) and `alloc_vector_slow` (a
  `const fn` `match` for the size's overflow checks); the stores' order and
  the `?` form are back, and both functions' machine code is main's.
  Lesson for X3's runs: a second checkout built in the same target
  directory leaves this one's older-mtime sources looking fresh (cargo
  rebuilt nothing and copied main's binary); touch the sources or use the
  CI's in-tree checkout.
- Review round 1 on #750 (OCR, five findings, all fixed):
  - `robigo_rtapi::install` runs `check_layout` once per process, so a
    stale or mixed api.c build fails at startup; before, only a Linux test
    ran it.
  - api.c asserts `unsigned long long` is 8 bytes, as Rust reads it.
  - `set_typ`'s debug assertion checks its whole contract (`Sexp::retype_keeps_body`, with a test).
  - heap.rs's `BLOCK` and `DATA_OFF` are exported for the large-chunk test.
  - The build scripts name `DEP_ROBIGO_HEAP_INCLUDE` when it is missing.

  perf-rt after them: every row within ±0.01% of main except e7-unique at
  −0.31%. A separate callgrind run puts that row at −0.11% per iteration,
  all of it in glibc's stripped malloc functions (`libc.so.6`): every
  robigo-rt function counts the same. So e7-unique's instruction count
  depends on the C allocator's state, not only on the heap's addresses
  (data for D8/D13's guard, decision 2).
- Verified: `scripts/rt-ci.py` in full (release tests, the conformance
  308/314 and every mode: NO_INPLACE, MATERIALIZE, gctorture, GC_MOVE,
  PIN_REPORT, the small vector limit, E9's 400 injected failures,
  overshoot); Miri on robigo-rtcore and on the heap's layout and bitmap
  tests; the heap/rtcore/rtapi tests; actionlint (only main's two
  shellcheck notes).
- For X3 (D8 × D13), what the seam gives: h24 is `vector::OFF_DATA` (and
  `charsxp` keeps its own 32-byte layout; `data_off(t)` already separates
  them in `std_data`, `inline_data`, `vector_bytes`, api.c's `RB_DATA`);
  nT is `GRANULE` and `node::size(t)` (allocation, `has_aux` in the trace
  arms and `alloc_node`, `same_body` asserted in `set_typ`). What it does
  not give yet:
  - h24's TRUELENGTH has no header word: `Sexp::truelen`/`set_truelen`
    (`layout/access.rs`) must find the trailing word through the size
    class, a call into the allocator (heap.rs) that `defs.rs` cannot make
    (build.rs includes it); Defn.h then needs `TRUELENGTH` as an export,
    not an offset.
  - nT: `heap.rs`'s `DATA_OFF` (1,072) and `SIZES` are literals: 8-byte
    granules double the bitmaps, so `DATA_OFF`, `START_WORDS` and
    `class_index`'s shifts change with the classes. `SET_TYPEOF`
    LISTSXP→LANGSXP (40 → 48 bytes) cannot happen in place; `same_body`'s
    debug assertion finds every such call. ALTREP instances are made from a
    node and trace a 4th word: they need the 48-byte size.
  - The test `layout_is_compact` and `the_layout_is_h32_n48` move with the
    variant; the heap tests' `FILL`/`LARGE_FILL` already follow it.
  - L4's R_HOME tree must ship `robigo_layout.h` beside Defn.h (C that
    includes Defn.h needs it; today only rtcore's own C does).
