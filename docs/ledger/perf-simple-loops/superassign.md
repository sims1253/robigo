# perf-simple-loops: closed form through `<<-` (round 7)

- Why: superWhile (`f <- function() while (x < n) x <<- x + 1`, called once
  per `execute`) was 66x behind Ř at peak. #294 runs loops with `<<-`
  natively: in the loop tier a `<<-` assigns the slot that stands for the
  enclosing binding, written back at the loop's exit. Its loop still ran
  every iteration (~0.5 ns each; Ř 1.37e-4 s for 343780).
- Change (`jit/summary.rs`): in the loop tier (not in closures compiled
  whole, where the target is a cell), `Op::SuperSet` counts as an
  assignment of its slot, like `SetVar`, so such loops get a closed form.
  The write-back is #294's.
- Result: superWhile peak 1.7e-6 s (Ř 1.37e-4, FastR 1.79e-4, GNU R
  2.9e-2; #294 alone ~1.2e-4 by its ledger's ratio, main 1.2e-2).
- Verified: `tests/r/jit/loop-closed-form-super.R` (enclosing and global
  bindings, while and for loops, a non-integer bound, a loop that does not
  run, an NA bound, integer overflow with its warning, a last-iteration
  copy), GNU R 4.5.0, identical in all tiers and with closed forms off;
  #294's `tests/r/jit/loop-superassign.R` unchanged (7 of its loops now
  summarized).
