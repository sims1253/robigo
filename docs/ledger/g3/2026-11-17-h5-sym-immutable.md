## 2026-11-17: H5, step 1: symbols are immutable after creation

- Why (m2m3-plan §H5, docs/g3.md §5.3): symbols move into one
  process-shared arena (H5's next step), so nothing may write a symbol once
  it is made. The design note is `~/.cache/robigo-research/h5-design.md`.
  This step is the per-heap refactor that comes first: the same heaps, the
  same layout.
- What:
  - `INTERNAL` (GNU R keeps it in the symbol's TAG) is a per-isolate table
    by symbol id in rtcore's `SymCells`, next to the base binding and the
    global-cache entry: `Cx::internal`, `Cx::internal_raw` (the compiler's
    and the evaluator's raw objects) and `Cx::set_internal`. The table is
    traced with the rest of `EnvState`. Readers: the `.Internal` special in
    the compiler, `cenv::simple_internal`, `NextMethod`'s dispatch, the
    bytecode translator, `is.builtin.internal`. `Sym::internal` is gone.
  - The bits a symbol gets from its name are set when it is made, before
    anything sees it: rtcore registers `sym::symbol_bits` once per process
    (`robigo_heap::set_symbol_bits`), and `Core::install` applies it.
    These are `SPECIAL_SYMBOL` (names.c's `Spec_name`), `DDVAL` (`..N`,
    `mkSYMSXP`) and `NO_TRACKREFS` on `.Last.value`. Before, `Cx::install`,
    `EnvState::new` and `install_rsyms` set them after the install.
  - `mark_seen_by_c` and C's `seen()` (api.c) skip immortal objects: they
    never move, so they need no mark, and a shared object will take no
    writes. The C layout header exports `ROBIGO_FLAG_IMMORTAL` for it.
  - A base binding's `ASSIGNMENT_PENDING` (complex assignment, `raw.rs`)
    is its `SymCell`'s bits, which the base store already read; before, the
    setter wrote the symbol's gp, so the base store never saw the bit (its
    old value's count was not decremented: not visible from R).
  - Debug builds assert that the barriered node stores (`set_car`,
    `set_cdr`, `set_tag`) never take a symbol and that nothing reads a
    symbol's TAG. robigo-heap's three tests that stored into a symbol's
    value now store into an immortal node: `immortal_node_fields_are_traced`
    (was `symbol_values_are_traced`), `the_store_barrier_logs_immortals_once`,
    `a_move_forwards_every_edge`.
- Not in this step: the arena, shared PRINTNAMEs (the foreman's §4
  decision), the static immortal objects. F7a dropped the per-isolate
  TRUELENGTH table.
- Verified: the debug tests of robigo-heap, -rtcore, -rt, -rtbuiltins and
  -rtapi with the asserts on (they found the compiler's `.Internal`,
  `simple_internal` and `NextMethod` readers); the full
  `scripts/rt-ci.py` (every mode, 2,800 injected failures); Miri on
  robigo-rtcore and the heap's layout and bitmap tests; `scripts/lint.sh`.
- Measured (this box, release, `scripts/perf.py rt-bars --main`): main
  `5f5f63f4f` (sha256 ed79841c…) against this branch merged into it
  (c6aab95d…), each built in its own target from its own checkout: 0
  failures, every row within ±0.05% instructions (fib +0.00%, the calls
  rows −0.04% to +0.01%), peak RSS within ±1%.
- Review: sol-loop, 2 rounds with gpt-6.1-sol on high, approved. Round 1
  found C's `seen()` and the base binding's `ASSIGNMENT_PENDING` (both
  fixed above).
