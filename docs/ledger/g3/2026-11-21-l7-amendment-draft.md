## 2026-11-21: L7 narrowed: a draft for the lead's sign-off

**Status: draft. It takes effect only when the lead signs it in this pull request, and the date placeholders (YYYY-MM-DD) in `docs/g3.md` become that date.** Until then, L7 stands as written on 2026-11-13.

### What changes

L7 (2026-11-13) says that `gc()`'s cell counts, trigger sizes, collection counts and levels, and `gcinfo(TRUE)`'s contents equal GNU R's. It says robigo reproduces them by a logical account kept off the hot path. That account was never built. `2026-11-18-h-memory-internals.md` measured what it would have to model: GNU R's boot, which is 50,595 Ncells and 132,978 Vcells against robigo's 8,102 and 4,034; first-use loading and compilation; per-call promises and argument pairlists; every frame's bindings; and each builtin's temporaries.

This draft narrows L7 to what a program can depend on. It replaces three passages of `docs/g3.md`:
- **§0, the "fully compatible" list:** "`gc()` (its cell and collection counts too: L7)" becomes "`gc()` (its shape, limits and the cells a builtin's data keeps: L7)".
- **The decisions table, row L7:** the new text, dated as amended.
- **§3, "Collection observables":** the "*Exact by L7*" paragraph is replaced by "*Exact by L7* (amended)" and "*robigo's own*". See the diff for the exact words.

### Why, and the conditions (the council of 2026-11-21)

Three models agreed on narrowing as an explicit exception approved by the lead.
- **The justification** is the full account's permanent cost on the evaluator's hot paths, which §13 would measure on every call, against numbers no program's result depends on. It is not the effort of building it.
- **The conditions:**
  1. **The lead amends L7,** and with it the matching text in §0 and §3. That is this pull request, once signed.
  2. **A ratchet case pins what stays exact** (the cases below, recorded on GNU R 4.6.2). They land before or with the amendment's merge.
  3. **The text says outright that collection counts, levels and `gcinfo` report frequency, including under `gctorture(TRUE)`, are robigo's own.** That is §3's "*robigo's own*" paragraph.

### The cases that pin what stays exact

All are under `tests/rt/memory/` and recorded with `scripts/oracle-run.sh 4.6.2 scripts/regen-expected.sh`. Each prints only what L7 keeps exact, and none prints an absolute total.

1. **`gc-shape.R`:** `gc()`'s and `gc(full = FALSE)`'s class, `dim` and `dimnames` (rows `Ncells`, `Vcells`; columns `used`, `(Mb)`, `gc trigger`, `(Mb)`, `max used`, `(Mb)`), and the `limit (Mb)` column when a limit is set. Also each column's type, and the `(Mb)` columns as the cell columns in GNU R's units (`round(cells × size / 2^20, 1)`), checked as relations.
2. **`gc-limits.R`:** `mem.maxVSize()` and `mem.maxNSize()` read and set, including `Inf`; their errors and warnings for an invalid value and for one below what is in use; the limit column; and the limit errors (`vector memory limit of … reached`, `cons memory exhausted`) at a limit just above the heap in use.
3. **`gc-reset.R`:** after `gc(reset = TRUE)`, max used equals used. After an allocation, max used is at least used and never decreases until the next reset.
4. **`gc-trigger.R`:** the ported trigger rules as relations: the trigger at least what is in use after a collection, and growth and shrinkage in the direction and order of `AdjustHeapSize`'s rules for a growing and then shrinking vector heap. No trigger value is printed.
5. **`gcinfo-format.R`:** `gcinfo(TRUE)`'s and `gc(verbose = TRUE)`'s report lines matched against GNU R's format, with the numbers replaced by placeholders (`Garbage collection N = a+b+c (level L) ...`, the cons-cell and vector lines with their `Mbytes` and `%`). Also the `gcinfo` switch's return value.
6. **`gc-deltas.R`:** the cells kept alive by data a builtin makes, as the difference of two `gc()` deltas. One measures the object kept; the other is the same measuring call with nothing kept, which cancels the measuring call's own cells (GNU R 4/3, robigo 12/70). The objects are `numeric(1e5)`, `.Internal(as.vector(1:1000, "list"))` and `integer(1e4)`. `memory.profile()`'s deltas by type are taken for the same objects. The 2026-11-18 entry found these agreeing except for CHARSXPs that robigo makes on first use, so the case warms those up first.

### What is not pinned, and is robigo's own

- the absolute used and trigger values;
- the number of collections, their levels (robigo reports level 2, `0+0+N`, for every collection), and when `gcinfo` reports;
- all of these under `gctorture(TRUE)`.

What `gctorture(TRUE)` makes happen stays exact under §3's first paragraph: finalizers run and weak references are cleared.
