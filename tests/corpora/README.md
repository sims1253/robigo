# Test corpora

Large collections of R code with GNU R 4.5.0's recorded behaviour, run by
`scripts/corpus.py` (see `docs/verification.md`). One directory per
corpus, described in `corpora.toml`:

- `cases/**/*.R` or `cases.jsonl` (`{"id", "code"}` per line): the code.
- `expected.jsonl.gz`: GNU R's stdout, stderr and exit status per case
  (`{"id", "sha", "out", "err", "status"}`; `unstable` marks cases whose
  output differed between two GNU R runs). Written only by
  `scripts/corpus.py record`.
- `passing.txt`: the ratchet, case ids that must keep passing.
- `support/`: files linked into each case's working directory.
- `build` (optional): a script in the corpus directory, run once before
  the cases. The `capi` corpus compiles its C sources (`support/*.c`,
  built `.so`/`.o` files are gitignored) with `R CMD SHLIB`; its cases
  are R drivers over the built libraries. Because the staleness check
  hashes only the R driver, re-record that corpus with `--all` after
  changing a `.c` file.

| corpus | origin | license |
|---|---|---|
| `fastr` | FastR's `ExpectedTestOutput.test` (oracle/fastr, commit in `corpora.toml`); FastR-only tests excluded | GPL-3 (see `fastr/README.md`) |
| `gnur-tests` | GNU R 4.5.0's `tests/` | GPL-2 or later, part of R |
| `gnur-examples` | the examples of GNU R 4.5.0's help pages (`tools::Rd2ex`) | GPL-2 or later, part of R |
| `gnur-demos` | every `demo()` of GNU R 4.5.0's base packages (`src/library/*/demo/*.R`), as `demo(..., echo = TRUE)` runs it | GPL-2 or later, part of R |
| `r-intro` | the runnable examples of GNU R's "An Introduction to R" (`doc/manual/R-intro.texi`), one script per chapter plus appendix A | GPL-2 or later, part of R |
| `cran-purer` | pinned pure-R CRAN package sources, help-page examples, demos and tests | MIT, GPL-2, GPL-3, LGPL-2.1-or-later, see `cran-purer/README.md` |
| `capi` | C cases over libR's exported API (`tests/capi/symbols.tsv` is the census; `scripts/capi-census.py` measures coverage) | written for robigo, GPL-2 or later |

`gnur-demos` and `r-intro` are the *script* corpora: whole, realistic R
programs rather than snippets, the measuring stick for "existing R scripts
just work". Both import from the R sources with
`scripts/import-corpus-scripts.py NAME` (see each corpus's README for what
is included and what is left out, and `docs/ledger/corpus-scripts.md` for
the ranked list of what still breaks them).


A named profile selects alternate default packages and recordings without
changing the normal run. FastR's `methods` profile prepares the methods
switch while environment finalizers are pending:

```sh
scripts/corpus.py run fastr --profile methods --filter 'S4|TestBuiltin_serialize' --jobs 2
scripts/corpus.py show fastr --profile methods S4/TestR5/474b742f1785a587
```

`record`, `run` and `show` accept `--profile`. The profile table in
`corpora.toml` overrides `default_packages` and `expected_file`.
Profile runs check the same `passing.txt`, write results as
`fastr.methods.json` (with a shard suffix when requested), and cannot
update the active ratchet through `--update` or `absorb`.
