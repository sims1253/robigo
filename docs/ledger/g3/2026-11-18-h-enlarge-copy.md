## 2026-11-18: EnlargeVector's copy as a block, and CLEAR_VECTOR's fill chosen once

- Why: X8's premise entry (`2026-11-18-x8-premise.md`) measured robigo's
  growth.
  - `EnlargeVector` copied a plain vector into its replacement through
    `alloc_with`'s closure, at 11 instructions per element (`growdbl.R`:
    213 M of 333 M in `enlarge_vector`).
  - `CLEAR_VECTOR` of a replaced list cost about 42 instructions per
    element in `UniqueList::set_na`, which on every element made a bounds
    assert, chose the fill by type, and loaded the isolate.
  - Neither is what the store or the reference counts must cost.
- What:
  - `Cx::alloc_copy_fill(xs, n, fill)` (`robigo-rtcore/src/fresh.rs`): a
    block copy of `xs`, then `fill` written through raw pointers into the
    uninitialized tail. `enlarged`
    (`robigo-rtbuiltins/src/subassign.rs`) uses it for logical, integer,
    double, complex and raw vectors. GNU R copies these element by element
    through `*_ELT`, which for a non-ALTREP vector is bitwise
    (`subassign.c:EnlargeVector`), and then writes NA (0 for raw, as
    `allocVector` leaves it).
  - `UniqueList::set_all_na` (`robigo-rtcore/src/edit.rs`): the fill
    (R_NilValue, or `NA_STRING` for a character vector) chosen once, each
    element still stored through `set_elt`, with its barrier and counts.
    `clear_vector` uses it (GNU R's `CLEAR_VECTOR`, `subassign.c:108-114`).
  - Lists are still copied element by element: each element's count is
    incremented, as `SET_VECTOR_ELT` does; that loop has little slack
    (`enlarge_vector`'s own 39 M for `listgrow.R`).
- Measured (robigo-rt release, `60d6027db` against main `deb9c8d32`, each
  built in its own checkout and target):
  - rt-bars: 0 failures. `grow-dbl` −1.07%, every other row within
    ±0.04% instructions.
  - Programs (callgrind, instructions): `benchmarks/rt/mem/growdbl.R`
    6.020 G → 5.825 G (−3.2%); `benchmarks/rt/mem/listgrow.R` 1.506 G →
    1.413 G (−6.2%).
  - Peak RSS: `listgrow.R` 45,476 → 45,356 KiB.
- Verified (2026-11-18, on `60d6027db`):
  - The full `scripts/rt-ci.py` passed: every mode, and 2,800 injected
    failures with no difference.
  - Miri on robigo-rtcore passed. `scripts/lint.sh` passed.
  - Debug `cargo test` of robigo-heap, -rtcore, -rt, -rtbuiltins and
    -rtapi passed but for one conformance case, `tests/rt/dynload/tools-natives.R`
    (`.requires r-home-tools`). The local debug target's `R_HOME` tree is
    stale: it has `tools.so` but not the `share/` files that main's
    `rt-home.sh` now installs. rt-ci's release run, which rebuilds the
    tree, passes it, and CI's debug job skips it (`R_HOME` set).
- Review: sol-loop, 2 rounds with gpt-6.1-sol on high, approved. Round 1
  found a `&mut [T]` over the uninitialized tail (`from_raw_parts_mut`'s
  contract); the tail is now written through raw pointers.
