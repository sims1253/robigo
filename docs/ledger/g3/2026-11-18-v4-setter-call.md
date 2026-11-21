## 2026-11-18: compiled code's setter call of `[<-` and `[[<-` (after V4 step 5d)

A compiled complex assignment whose setter is a special is called on
bcassign's generic path when the index has `...` (`x[...] <- v`,
`x[[...]] <- v`). bcEval's SETTER_CALL calls the special as
`PRIMFUN(fun)(call, fun, args, rho)` (eval.c:8495-8511):
- `call` is the compiler's call constant, with `*tmp*` in it;
- `args` is a duplicate of its arguments, the first an evaluated promise
  of `*tmp*` and the last a promise of the value.

The special's dispatch (`R_DispatchOrEvalSP`) hands a method `call`, so
`sys.call()` shows `` `[<-.foo`(`*tmp*`, ..., value = 9) `` and
`match.call()` shows `x = *tmp*`. robigo-rt built one call out of the
promises and ran that, so a method saw the object's value (`list(1, 2)`)
in place of `*tmp*` (E's example, `delegates/E.md` §8). #853 had done
the same fix for `@<-` alone.

### The port

- `specials::subassign_apart` and `subassign2_apart`: `do_subassign` and
  `do_subassign2` with the arguments read from `ev_call` (a call over the
  promises) and `call` the one dispatch and the default method's messages
  use. `dispatch_or_eval_sp` takes the two apart. The AST's
  `do_subassign` calls it with its own call twice.
- bcassign's generic special path calls them, and #853's `slot_gets` for
  `@<-`, with the compiler's call. There is no R_Visible bracketing
  around the special on this path, since SETTER_CALL calls `PRIMFUN`
  directly. Every other special keeps `apply_special`.
- Deferred low: specials.rs's redundant ALTREP guard before
  `materialize_args`, which checks it inline (#810's review).

### Cases

`tests/rt/jit/setter-call.R`, recorded on 4.6.2 and ratcheted, has
methods for `[<-`, `[[<-` and `$<-` that print `sys.call()` and
`match.call()`, run:
- compiled, with and without `...`, with `drop = FALSE`, and with an empty
  index;
- in the AST;
- as `return()` of such assignments after `invisible()`, showing the
  special's own visibility.

Before the fix, the two `...` cases printed the object's value.

### Verification

On V4 step 5d's head (#879, `bdab1d6dc`):
- The conformance crosscheck (`ROBIGO_BC_CROSSCHECK=1`): 0 unexpected,
  132 the same. Conformance is 582/588; the 6 failing are unratcheted.
- The base.rdb crosscheck: 0 unexpected (999 the same, 147 expected).
- The full `scripts/rt-ci.py`: exit 0. Every mode passes, and injected
  allocation failures show 0 differences in 2800 runs.
- perf-rt (`scripts/perf.py rt-bars --main`, against 5d's binary): flat.
  The worst row is superassign at +0.19%; the guard rows are within
  ±0.02% and s3 at +0.00%. `scripts/lint.sh`: clean.
- Sol (codex `gpt-6.1-sol`, high): round 1, APPROVE, no findings.
