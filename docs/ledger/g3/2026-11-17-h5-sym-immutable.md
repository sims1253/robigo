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
  - `mark_seen_by_c` skips immortal objects: they never move, so they need
    no mark, and a shared object will take no writes.
  - Debug builds assert that the barriered node stores (`set_car`,
    `set_cdr`, `set_tag`) never take a symbol and that nothing reads a
    symbol's TAG. robigo-heap's three tests that stored into a symbol's
    value now store into an immortal node: `immortal_node_fields_are_traced`
    (was `symbol_values_are_traced`), `the_store_barrier_logs_immortals_once`,
    `a_move_forwards_every_edge`.
- Not in this step: the arena, shared PRINTNAMEs (the foreman's §4
  decision), the static immortal objects. F7a dropped the per-isolate
  TRUELENGTH table.
- Verified: (pending)
- Measured: (pending)
- Review: (pending)
