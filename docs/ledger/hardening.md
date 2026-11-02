# Ledger: hardening

Failures that end the process where GNU R reports an error, and the checks
that find them.

## 2026-11-02: catchable allocation failures, parser and RDS panics

Measured on `main` 01ca4547 against a local GNU R 4.5.0.

- `scripts/fuzz.py` (then a throwaway copy), 4 minutes each: `readRDS` 88,876
  runs with 821 process deaths in 5 buckets, `parse` 89,850 runs with 21
  panics in one bucket. After: `rds` 97,094 runs, 0 findings.
- `x <- "\é"` panicked in `line_col`: gram.c counts the byte after the
  backslash, so the error offset fell inside the character.
- Huge allocations: stable Rust cannot make a failed infallible allocation
  unwind, so `memory.rs` checks where a number becomes a length. Lengths of
  existing vectors bound everything else. Below 1 GiB there is no probe, so
  no vector benchmark pays for it.

This covers lengths, not memory running out. Below 1 GiB nothing is probed,
and two blocks that pass their probes one at a time can still exceed a tight
limit together: `matrix(0, 10000L, 10000L, byrow = TRUE)` under a 2 GiB
address limit aborts where GNU R succeeds, because it builds an index vector
beside the result. Fixing that needs fallible allocation everywhere.

Open gaps, each a script without a ratchet marker:

- `tests/r/vectors/lazy-long-seq.R`: compact sequences are stored expanded,
  so `length(1:1e15)` is an allocation error instead of GNU R's lazy answer.
- `tests/r/lang/escape-multibyte-message.R`: GNU R's message carries the
  lone lead byte, and messages here are `String`s, so robigo prints the
  whole character.
