## 2026-11-15: D2s decision: the overflow area, by the bar

- Bar: docs/ledger/g3.md, "M1 decision bars", D2s. `shapes` is adopted if
  its geomean instructions over D2S are at least 5% below `overflow`'s and
  no CALLS row is more than 0.5% slower; otherwise `overflow`.
- Runs: `2026-11-15-d2s-runs.md` (`g3/x1-d2s-shapes` at `87272ace`,
  against main `af7a7f19`, both at D2's decided `inline` frames).
- Result:
  - D2S: shapes ÷ overflow = 1.0052. `shapes` is 0.52% slower, not 5%
    faster.
  - CALLS guard: `plain` +0.56%, `named` +0.59% and `env-fn` +0.63% are
    over the 0.5% limit (`defaults` +0.46%, `superassign` +0.41%; `s3`
    −10.6%).
  - Both conditions fail. Judgement, not measured: even a transition
    with no overhead would very likely fail the first one on robigo-rt,
    since two of the four rows never overflow (`d2s-assign` is a cmp.R
    local, `d2s-eval` binds without an overflow block) and the block a
    transition saves on `d2s-callee-assign` costs about 0.9% of that row
    (the estimate in the runs entry puts the D2S geomean's fall near 0.5%).
- **Verdict: `overflow`, the default on main.** Nothing merges.
- For P1 (from the runs):
  - S3 methods' frames overflow on every call (the dispatch's variables
    are not in a method's shape); removing that saved 10.6% of `s3` here.
    A way that costs nothing on other calls (the dispatch variables in
    every method's shape, or a transition taken by dispatch only) is P1's
    to weigh against E1's `s3` row.
  - Why `eval(quote(v <- 1))` binds in the frame without an overflow
    block, at 3 LISTSXP per call, is not settled (`2026-11-15-d2s-runs.md`).
- Deleted: `g3/x1-d2s-shapes`, once this PR merges.
