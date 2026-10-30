## Your track: `conformance-vectors`

FastR's corpus (`tests/corpora/fastr`, 31,374 snippets with GNU R's recorded output; 28,624 pass on main as of 2026-10-28) has about 150 failing cases in your area. Your filter:

    scripts/corpus.py run fastr --filter 'TestBuiltin_(seq|rep|order|sort|unique|duplicated|match|cbind|rbind|which|rev|array|matrix|dim|subset|c_|crossprod)|TestSimpleVectors|TestSimpleMatrix|TestSimpleArrays' --keep-output

Inspect a case with `scripts/corpus.py show fastr <id>`. Group the failures by root cause first (write the grouping into your ledger), then fix the biggest groups by porting GNU R's C code. GNU R: `src/main/subset.c`, `subassign.c`, `subscript.c`, `seq.c`, `bind.c`, `unique.c`, `sort.c`, `array.c`. Also check the help-page examples of your area (`scripts/corpus.py run gnur-examples --filter '^base/...'`) and `docs/ledger/cli.md`'s ranked list.

You own `crates/robigo-runtime/src/builtins/{subset,subassign,subscript,seq,bind,unique,sort,array}.rs` (another agent may make small call-site changes to colSums & co. in `array.rs` for the numeric-policy work; coordinate by keeping your changes there minimal and rebasing often). Native routines of the stats/utils/graphics packages come from GNU R's own shared libraries later (G2): do not port them. Metric: `fastr` passes within your filter (record new passes with `--update`, commit the new markers under `tests/corpora/fastr/passing/`), with no regressions anywhere (CI's full run on your PR).
