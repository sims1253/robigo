## 2026-11-17: #757's two deferred OCR nits on unique.rs

- `duplicated_flags` filled the whole answer after a full table (each
  lookup failing again, a placeholder written) only to discard it. It now
  fills through `Cx::alloc_try_with` (robigo-rtcore), which stops at the
  first error: the table's `Full` ends the fill.
  - The first form, a `full` test in the fill closure, cost e7-unique
    +1.61% (~2 instructions per element on the success path). The
    fallible fill has no extra test: its loop leaves on the error the
    closure already returned.
  - Measured (this box, release, `scripts/perf.py rt-bars --main`, main
    `798b0fce` against `9a003283`, each built in its own target, sha256
    checked to differ): 0 failures; e7-unique -4.07% (123,061 to 118,056),
    all of it in `duplicated_flags`' fill (17,437 to 12,427 per iteration,
    callgrind), every other row within ±0.08%.
- `keep_unique`'s doc comment had two summaries; one remains.
- Verified: the full `scripts/rt-ci.py`, the rt crates' debug tests (54
  binaries), Miri on robigo-rtcore, `scripts/lint.sh`.
