## Your track: `conformance-lang`

FastR's corpus (`tests/corpora/fastr`, 31,374 snippets with GNU R's recorded output; 28,624 pass on main as of 2026-10-28) has about 110 failing cases in your area. Your filter:

    scripts/corpus.py run fastr --filter 'TestBuiltin_(do.call|match.arg|sys|environment|eval|quote|substitute|switch|tryCatch|stop|warning|UseMethod|get|exists|class)|functions/|TestEnvironments' --keep-output

Inspect a case with `scripts/corpus.py show fastr <id>`. Group the failures by root cause first (write the grouping into your ledger), then fix the biggest groups by porting GNU R's C code. GNU R: `src/main/eval.c`, `envir.c`, `context.c`, `errors.c`, `objects.c` (S3 dispatch), `coerce.c` (do.call, substitute). Also check the help-page examples of your area (`scripts/corpus.py run gnur-examples --filter '^base/...'`) and `docs/ledger/cli.md`'s ranked list.

You own `crates/robigo-runtime/src/builtins/{lang,envir,eval}.rs`, `dispatch.rs`, `conditions.rs` (and `interp.rs` for evaluator semantics, minimal and called out: other tracks touch it too). Native routines of the stats/utils/graphics packages come from GNU R's own shared libraries later (G2): do not port them. Metric: `fastr` passes within your filter (record new passes with `--update`, commit the new markers under `tests/corpora/fastr/passing/`), with no regressions anywhere (CI's full run on your PR).
