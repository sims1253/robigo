## Your track: `conformance-print`

FastR's corpus (`tests/corpora/fastr`, 31,374 snippets with GNU R's recorded output; 28,624 pass on main as of 2026-10-28) has about 80 failing cases in your area. Your filter:

    scripts/corpus.py run fastr --filter 'TestBuiltin_(print|format|deparse|dput|cat|toString|printdefault)|TestSimplePrinting' --keep-output

Inspect a case with `scripts/corpus.py show fastr <id>`. Group the failures by root cause first (write the grouping into your ledger), then fix the biggest groups by porting GNU R's C code. GNU R: `src/main/print.c`, `printvector.c`, `printarray.c`, `printutils.c`, `format.c`, `deparse.c`, `builtin.c` (cat). Also check the help-page examples of your area (`scripts/corpus.py run gnur-examples --filter '^base/...'`) and `docs/ledger/cli.md`'s ranked list.

You own `crates/robigo-runtime/src/{print,format,deparse}.rs`. Native routines of the stats/utils/graphics packages come from GNU R's own shared libraries later (G2): do not port them. Metric: `fastr` passes within your filter (record new passes with `--update`, commit `passing.txt`), with no regressions anywhere (CI's full run on your PR).
