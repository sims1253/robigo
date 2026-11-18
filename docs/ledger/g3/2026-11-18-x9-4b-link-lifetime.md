# X9-4 step (b): GNU R's link lifetime is not ported — a latent deviation, not observable today (2026-11-18)

X/P1. The foreman dropped X9-4 step (b) as a performance step (X9-4 ends at
#881, step (a)). This entry records the difference it would have removed,
what shows it, and what it costs.

## The rule

GNU R 4.6.2 gives the byte-code node stack's values a link (a reference
count) before an operation that may write in place. Each kind of link has
its own lifetime:
- **A complex assignment** (STARTASSIGN/STARTASSIGN2, eval.c:8209, 8423)
  commits the stack up to the current protection top
  (`INCLNK_stack_commit`, eval.c:1012). That top is the activation's entry
  top (eval.c:7206, 7369), raised by a for loop over its own state
  (eval.c:7595, 7704) and by `INCLNKSTK` (eval.c:8702). The links are
  held until the committing activation ends (`restore_bcEval_globals`,
  eval.c:7211-7214; also for in-loop calls, eval.c:7319, 7331), its loop
  ends (ENDFOR, eval.c:7817) or its region ends (`DECLNKSTK`,
  eval.c:8709).
- **The compiler's `INCLNK`** links one pending operand across code with
  side effects (`x + g()`) until its `DECLNK` (eval.c:8691-8697). `INCLNK`
  also commits the stack.
- **`applydefine`** (the AST's complex assignment) commits and releases
  around the assignment (eval.c:3381, 3558).

robigo-rt (`crates/robigo-rt/src/links.rs`, `bcassign::run`,
`assign::applydefine`) links every register of the live VM activations at
each complex assignment and drops the links when that assignment ends:
`applydefine`'s lifetime, used for both.

## Is it observable?

Only through reference counts, and robigo-rt has no reader of them yet.
- **From R:** GNU R's readers are `.Internal(refcnt())`, the `REF(n)` of
  `.Internal(inspect())`, `tracemem()` and `.Internal(address())`.
  robigo-rt implements none of them ("not implemented yet").
- **From C:** robigo-rt's C API exports none of `MAYBE_SHARED`, `REFCNT`
  or `NAMED` (crates/robigo-rtapi), so a routine cannot read whether a
  value is shared.
- **At writes**, which C can observe through addresses: robigo-rt copies
  exactly where GNU R does. `tests/rt/capi/link-lifetime.R` covers these
  shapes, recorded on 4.6.2 and passing on main `eec051b8`, ratcheted:
  - a value read for an index, an operand and a builtin's argument, then
    written;
  - two writes in one activation;
  - a write after other writes in a loop;
  - a loop's own sequence written in the loop;
  - a caller's pending operand written by the callee through `<<-`;
  - a write after a callee that wrote has returned.

  Every write that writes in place in GNU R also writes in place in
  robigo-rt, and every copy is a copy. robigo-rt's links are taken where an
  in-place write is decided, at the assignment itself, and they cover at
  least GNU R's linked values at that point. Its own registers are covered,
  and builtin arguments are skipped while the builtin runs. So the extra
  lifetime GNU R gives its links never changes a write's outcome.

What would differ is the counts in between. `tests/rt/alloc/link-lifetime-refcnt.R`
records GNU R's: a global `x` pending as a caller's operand reads
`refcnt` 2 before, during and after the callee's assignments, in its loop
and after a callee of its own has returned, and 1 once the caller has
returned or without a pending caller. robigo-rt's counts would read 1
outside its assignments. That case fails on robigo-rt today at
`.Internal(refcnt())` and is not ratcheted. It is the target for whoever
implements `refcnt()`, `inspect()`, `tracemem()` or the C API's
`MAYBE_SHARED`/`REFCNT`. When any of them lands, this deviation becomes
observable and the port is due.

## Why the port is deferred, and its price

Measured in `~/.cache/robigo-research/x-p1/x94b-study/` (probe patch,
bars, traces):
- The cheapest faithful hot path is one test at each `vm::exec` activation
  exit ("is the committed mark above this frame?"), with a cold release.
  It costs about **7 instructions per real activation**: trycatch +0.23%,
  binarytrees +0.12%, measured against C1's 5d `8b09a49d`. The call rows
  are flat only because their callees run inline. That is before the
  operand links (`INCLNK`) and the loop and region scopes, which a full
  port also needs.
- What it saves: the link walks of a cmp.R compile would fall by at most
  42% (−180k of 427k per compile after #881), and trycatch would save
  nothing.

So the port is a fidelity item with a price on every real call, not a
speed item. It belongs to C1-TM's wave-5 bar, "the address relations equal
the oracle's", together with the readers that make it observable.
