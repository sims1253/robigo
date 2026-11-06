# perf-fasta: paste() borrows character arguments

Track ledger: `docs/ledger/perf-fasta.md` (this file holds one PR's entry).

## 2026-11-04: paste() without copying its character arguments

- Why: with fastaredux_naive's loop native, `paste(seq, collapse = "")`
  per line was 11% of the program, and `paste_args` began by cloning every
  character argument's element vector (an allocation and a reference count
  increment and decrement per string).
- What (`builtins/paste.rs`): `paste_args` borrows character vectors
  (`Cow`) unless some argument is an object whose `as.character` method runs R code (which, or native code it calls, may write strings in place); coerced arguments are new vectors. `file.path` takes owned
  copies as before.
- Result: `paste(x, collapse = "")` of 60 strings 0.80 -> 0.75 us;
  fastaredux_naive, body repeated 20 times: 1,514.7M -> 1,477.3M
  instructions (-3.4% of the steady state).
- Verified: `tests/r/io/cat-paste-strings.R` and the FastR paste/cat/format
  slices; full conformance (failures as main); gate against the rep branch: 3 faster (fastaredux_naive -0.71%, fasta_naive -0.54%, fastaredux -0.52%), 0 slower. Sol review: round 1 found the borrow live across `as.character` methods, fixed.
