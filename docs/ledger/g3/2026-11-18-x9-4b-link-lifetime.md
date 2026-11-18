# X9-4 step (b): GNU R's link lifetime is not ported — an observable deviation, deferred (2026-11-18)

X/P1. The foreman dropped X9-4 step (b) as a performance step (X9-4 ends at
#881, step (a)). This entry records the difference it would have removed:
it is observable from R today, it has a measured price, and its port is
deferred.

## The rule

GNU R 4.6.2 gives the byte-code node stack's values a link (a reference
count) before an operation that may write in place, and drops each link
when its scope ends:
- **A complex assignment** (STARTASSIGN/STARTASSIGN2, eval.c:8209, 8423)
  commits the stack up to the current protection top
  (`INCLNK_stack_commit`, eval.c:1012). That top is the activation's entry
  top (eval.c:7206, 7369), raised by a for loop over its own state
  (eval.c:7595, 7704) and by `INCLNKSTK` (eval.c:8702). At the committing
  activation's end, `restore_bcEval_globals` (eval.c:7211-7214; also for
  in-loop calls, eval.c:7319, 7331) drops the links above the caller's
  protection top and restores the committed mark. A for loop's links end at
  ENDFOR (eval.c:7817), a region's at `DECLNKSTK` (eval.c:8709).
- When an activation ends, `DECLNK_stack` drops the links committed at or
  above the protection top saved at its entry (its caller's), and the
  committed mark goes back to the caller's. Links it committed below that
  saved top stay. So when `h` calls `g`, which calls `k`, and `k` makes a
  complex assignment, `h`'s pending operand lies below `g`'s protection
  top: its link outlives `k` and lasts into the rest of `g`. This is the
  recorded "returned callee" row. A callee called straight from the
  expression that holds the operand (`x + { k(); …; 0 }`) commits nothing
  below its caller's top that is still pending, so there the link is
  dropped when `k` returns. This is GNU R's accounting as it is.
- `INCLNK`/`DECLNK` (eval.c:8691-8697) link one operand. The 4.6.2
  compiler never emits them: `checkNeedsInc` is `FALSE`, cmp.R:2104. The
  VM supports them for older byte code.
- **`applydefine`** (the AST's complex assignment) commits and releases
  around the assignment (eval.c:3381, 3558). The AST's `evalList` links
  the arguments it has evaluated while it evaluates later ones (eval.c:2805).

robigo-rt (`crates/robigo-rt/src/links.rs`, `bcassign::run`,
`assign::applydefine`) links every register of the live VM activations at
each complex assignment and drops the links when that assignment ends:
`applydefine`'s lifetime, used for both.

## Observable from R today

Writes made by complex assignments agree, because both commit at the
assignment itself, and robigo-rt's walk covers at least GNU R's linked
values there. `tests/rt/capi/link-lifetime.R` covers these shapes,
recorded on 4.6.2, passing and ratcheted; C compares the addresses before
and after each write:
- a value read for an index, an operand and a builtin's argument, then
  written;
- two writes in one activation;
- a write after other writes in a loop;
- a loop's own sequence written in the loop;
- `<<-` on a caller's pending operand;
- a write after a callee that wrote has returned.

A builtin that writes its argument in place unless `MAYBE_SHARED`, with no
commit of its own, sees the longer lifetime. `comment<-` is one
(attrib.c:525: no `IS_ASSIGNMENT_CALL` refinement, unlike `names<-`'s).
Sol found the shape in review. `tests/rt/alloc/link-lifetime.R` records it
on 4.6.2. A global `x` is pending as a caller's operand (`x + g()`), and
`g` calls `.Internal(`comment<-`(get("x", globalenv()), "changed"))`.
Every closure is compiled first, so that a JIT compile inside the call,
which makes its own assignments, does not link the operand.

| `g` does, before the `comment<-` | GNU R 4.6.2 | robigo-rt |
|---|---|---|
| a complex assignment, with no pending caller | in place | in place |
| nothing | in place | in place |
| a complex assignment | **copied** | in place |
| a complex assignment, after the `comment<-` | in place | in place |
| calls a callee that made one and returned | **copied** | in place |
| complex assignments in a loop | **copied** | in place |

So in robigo-rt a global's value can change under a caller's pending
expression where GNU R's copies. The case fails on robigo-rt and is not
ratcheted.

Other readers:
- **Count readers.** GNU R's `.Internal(refcnt())`, the `REF(n)` of
  `.Internal(inspect())`, and C's `MAYBE_SHARED`/`REFCNT`/`NAMED` read the
  count, and would differ in the same windows. robigo-rt has neither R
  reader, and its C API exports none of the three.
- **Identity and copy observers.** `tracemem()` and `.Internal(address())`
  do not read counts. They show a difference only where a write's outcome
  differs, as `comment<-`'s does. robigo-rt has neither yet.

## Why the port is deferred, and its price

Measured in `~/.cache/robigo-research/x-p1/x94b-study/` (probe patch,
bars, traces):
- The cheapest faithful hot path is one test at each `vm::exec` activation
  exit ("is the committed mark above this frame?"), with a cold release.
  It costs about **7 instructions per real activation**: trycatch +0.23%,
  binarytrees +0.12%, measured against C1's 5d `8b09a49d`. The call rows
  are flat only because their callees run inline. That is before the
  loop and region scopes, which a full port also needs.
- What it saves: the link walks of a cmp.R compile would fall by at most
  42% (−180k of 427k per compile after #881), and trycatch would save
  nothing.

So the port is a fidelity item with a price on every real call, not a
speed item: its cost is the per-activation save and restore of the marks,
not the walks. It belongs to C1-TM's wave-5 bar, "the address relations equal
the oracle's". The port must reproduce GNU R's accounting, including the
links that outlive a returned callee, and `tests/rt/alloc/link-lifetime.R`
is its first case.
