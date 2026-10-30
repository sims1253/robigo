## Your track: `conformance-strings`

FastR's corpus (`tests/corpora/fastr`, 31,374 snippets with GNU R's recorded output; 28,624 pass on main as of 2026-10-28) has about 160 failing cases in your area. Your filter:

    scripts/corpus.py run fastr --filter 'TestBuiltin_(paste|substr|sprintf|grep|sub|regexpr|nchar|toupper|strsplit|chartr|format|trimws|startsWith|endsWith|encodeString|agrep|grepRaw|iconv)' --keep-output

Inspect a case with `scripts/corpus.py show fastr <id>`. Group the failures by root cause first (write the grouping into your ledger), then fix the biggest groups by porting GNU R's C code. GNU R: `src/main/character.c`, `paste.c`, `grep.c`, `sprintf.c`, `util.c`, `printutils.c` (encodeString), and TRE/PCRE semantics. Also check the help-page examples of your area (`scripts/corpus.py run gnur-examples --filter '^base/...'`) and `docs/ledger/cli.md`'s ranked list.

You own `crates/robigo-runtime/src/builtins/{character,paste,grep,sprintf,character_widths}.rs`. Native routines of the stats/utils/graphics packages come from GNU R's own shared libraries later (G2): do not port them. Metric: `fastr` passes within your filter (record new passes with `--update`, commit the new markers under `tests/corpora/fastr/passing/`), with no regressions anywhere (CI's full run on your PR).
