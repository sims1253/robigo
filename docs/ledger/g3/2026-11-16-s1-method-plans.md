## 2026-11-16: S1, match plans for methods called with their generics' arguments

- What: the foreman's next item after #725. X/P1 had measured it at
  about -1.4% on `s3`. A closure call made from compiled code keeps its
  argument match as a plan in the caller's code, at the call site
  (`Plan`, `code::plan_bytes`). An S3 method called with its generic's
  arguments (`args::Source::Generic`, the `apply_with::<true>` instance)
  is called by dispatch, not by code, so it has no call site, and every
  dispatch ran `match_args`.
- `args::MethodPlans`, in dispatch's state (`CondState::method_plans`),
  keeps the 16 most recently made plans, newest first. The key is
  everything `match_args` reads besides the formals:
  - the callee code's stamp, which fixes its formals;
  - the formals' and the actuals' counts;
  - each actual's tag (symbols are immortal and never moved, or
    R_NilValue);
  - which actuals are R_MissingArg, as a bit mask (a formal matched by tag
    to an empty actual can be refilled by position).

  A hit fills the match as a site's plan does, its partial matches
  included, so `warnPartialMatchArgs` still warns on every call. A
  recompiled or reshaped code object has a new stamp and misses. Only
  the `GENERIC` instance looks there; every other call's path is
  unchanged.
- Measurements (WSL2 workstation, Ryzen 9 5900X, rustc 1.100.0; release
  `robigo-rt` from `~/.cache/cargo-target-s1`; the M1 bars,
  `scripts/rt-bars.sh` twice each against #725's head `881ce5c4`,
  `rt-bars-compare.py`; `data/2026-11-16-s1-method-plans/bars-compare.md`):
  s3 20,022.2 -> 19,797.4 instructions (0.9888) and 22,797.9 -> 22,674.4
  estimated cycles (0.9946), 1.08x -> 1.07x GNU R 4.6.2's 18,497.
  Every other row's instructions are unchanged (1.0000). Estimated
  cycles move with the binary's layout: superassign +1.65%, binarytrees
  +0.34%, the same in both runs, with the same instructions.
- Proof: `tests/rt/s3/method-plans.R`, recorded on 4.6.2 and ratcheted.
  It calls one method twice with each of nine tag patterns: none,
  positional, tagged, mixed, out of order, an empty actual, one into
  `...`. It also covers a partial match warned on both calls, the
  method redefined with other formals, an unused argument on every call,
  and 20 methods called in turn twice (past the memo's 16). All
  `tests/rt/s3` and `tests/rt/conditions` cases pass. `scripts/rt-ci.py`
  passes every step: conformance 286/291, gctorture, `ROBIGO_GC_MOVE` with
  gctorture, `ROBIGO_FRAME_QUARANTINE`, the small vector limit, injected
  allocation failures. `scripts/lint.sh` is clean. `robigo-rtcore` is
  unchanged, so Miri was not run.
