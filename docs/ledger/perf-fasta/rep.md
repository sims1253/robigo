# perf-fasta: rep() in runs

Track ledger: `docs/ledger/perf-fasta.md` (this file holds one PR's entry,
so PRs of the track do not conflict).

## 2026-11-04: rep() with `times` vectors and `each` extends runs at once

- Why: in `fasta`, after its generator loop went native, `outer(psum, rs,
  "<")` is the biggest piece left of each line, and 40% of `outer` was
  `do_rep`: `rep(Y, rep.int(length(X), length(Y)))` (a `times` vector) and
  `rep(X, times = ...)`, ~20k instructions per call of 900 elements. `rep4`
  pushed elements one at a time with a length check each, and `counts_of`
  collected through `Result`, which reallocates.
- What (`builtins/seq.rs`): the `each` and `times`-vector paths of `rep4`
  write each run with one `Vec::resize`; `counts_of` validates integer
  counts in one pass (NA is negative) and converts in another. The results
  are the same elements in the same order.
- Result: `rep(x, rep.int(15L, 60L))` 1.10 -> 0.80 us, `outer(psum, rs,
  "<")` 5.75 -> 5.40 us. Gate against the track's tip: 3 faster
  (`spectralnorm_alt_2` -7.4%, `spectralnorm_alt` -2.5%, `fasta` -0.9%),
  0 slower.
- Left: the rest of `do_rep` is evaluating its arguments in the AST
  interpreter (`dispatch_or_eval_special`, ~7.6k instructions per call:
  `rep.int(...)` is a closure call there), general interpreter cost.
- Verified: `tests/r/vectors/rep-runs.R` (each, times vectors with zeros,
  length.out cutting runs, every type, empty results, outer), recorded
  with GNU R 4.5.0, identical in all tiers and to the previous build; the
  FastR `rep` corpus slice 186/186.
