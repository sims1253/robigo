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

| corpus | origin | license |
|---|---|---|
| `fastr` | FastR's `ExpectedTestOutput.test` (oracle/fastr, commit in `corpora.toml`); FastR-only tests excluded | GPL-3 (see `fastr/README.md`) |
| `gnur-tests` | GNU R 4.5.0's `tests/` | GPL-2 or later, part of R |
| `gnur-examples` | the examples of GNU R 4.5.0's help pages (`tools::Rd2ex`) | GPL-2 or later, part of R |
