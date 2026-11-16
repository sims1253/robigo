## 2026-11-16: T2b, a suspended AST activation resumes on the live list

- What: E6's third regression case (docs/g3.md §15.3), excluded from T2
  (#690) for want of a design, now decided by the foreman. §9.3: "a
  suspended compiled activation whose assumption broke deopts at its next
  uncommitted operation." Base: `g3/t2-observation` at `eceb4843` (#690),
  whose code floors this builds on.
  - **GNU R's granularity.** GNU R's AST reads its next step live:
    - `do_begin` reads the next statement of a `{` from the list after each
      statement;
    - `do_for` and `do_repeat` evaluate the body object they read when the
      loop started, at each iteration;
    - `do_while` also reads its condition (`CAR(args)`) at each iteration.

    So a write takes effect at the next such boundary. A running BCODESXP
    keeps running: byte code is an object of its own. robigo-rt's `Eval`
    profile (GNU R's AST) now checks at the same boundaries; its `Bc`
    profile (GNU R's byte code) has no checks.
  - **`Op::Resume`**, at the start of a `{` (after its guard, whose
    evaluation tick may run R code) and after each of its statements (the
    statement's value still in its register): if the
    running code is stale (its stamp below the isolate's AST floor, T2's
    `Cx::code_is_current`), the rest of the `{` is `do_begin`'s, from the
    cell after the one just done (re-read after each statement), with
    the value so far. Then a jump past the `{`'s code. A `return`,
    `break` or `next` in a statement evaluated there reaches its context
    as in GNU R (`find_context`).
  - **`Op::LoopResume`**, at the start of each iteration of `for`,
    `while` and `repeat`: if stale, the iteration is GNU R's. For `while`,
    the condition `CAR(args)` is evaluated, from the `args` cell the loop
    read at its start, and tested as `asLogicalNoNA` does (FALSE ends the
    loop); then the loop's body object is evaluated; then the back-edge
    (`next` and `break` land on the loop's context).
  - Top-level loops in the global environment are compiled as they start
    (`R_compileAndExecute`, as before), so they are byte code and have no
    checks, as in GNU R.
- Decisions:
  - **The check is one compare, inline, and only in `Eval` code**: the
    code object's stamp against the AST floor, read from the code register.
    The deopt is a `#[cold]` function of its own (`boundary`). `View` gains
    no field, since byte code's hot path reloads it after every call.
    Making the check out of line too cost more: every check paid a call.
  - **The value so far is the register's, not kept longer.** The check
    after a statement runs before the statement's register is cleared, so
    a `{` whose rest became empty returns that statement's value, as
    `do_begin`'s `s` does. No value lives longer than in GNU R (finalizer
    timing). The statements of a `{` in tail position keep their
    visibility, since a write that cuts the list can make any of them the
    last.
  - **Granularity is the statement or the iteration**, as GNU R's. Within
    a statement, robigo-rt's code runs on, where GNU R's AST could read a
    later argument or an `if` branch live; no case observes that yet, and
    `if` and argument lists are not among the boundaries the foreman set.
  - `suspended-deopt` leaves the conformance features: no case needs it.
- Proof (WSL2 workstation, release, private target `~/.cache/cargo-target-t2`):
  - `tests/rt/capi/suspended-body-write.R`, recorded on GNU R 4.6.2, runs
    in every build and is ratcheted. It covers:
    - AST halves: a NOJIT closure's later statement written by a callback
      in the statement before it (the new one runs); a closure's first
      call (AST); a `for` whose body call is written in its first
      iteration (-4 from later iterations); a `while` whose condition is
      written (3); a loop's `{` whose next statement is written (1230); a
      `{` cut after the statement under way (its value and visibility the
      `{`'s); a `while` given a new argument list (it keeps its own).
    - byte-code halves: the same closure past its threshold, and one
      compiled at its first call: the old statement runs.

    It passes in production, under `ROBIGO_NO_INPLACE=1`,
    `ROBIGO_MATERIALIZE=1`, gctorture and the moving collector. The moving
    collector found a stale pointer in the first version of the loop
    deopt: the body and call were raw words across an evaluation. They are
    rooted now.
  - Sensitivity: with both checks disabled the case prints "old statement"
    where GNU R prints the new one, and 6, 10, 6 where GNU R prints -4, 3,
    1230.
  - RT conformance 239/244 (the 5 failures are main's).
- Cost: callgrind instructions per iteration, `scripts/idiom-vs-r.py --rows
  benchmarks/rt/calls-rows.toml --no-r`, T2's head `eceb4843` (#690)
  against this branch, both release builds of robigo-rt:

  Default JIT:

  | row | T2 (#690) | this branch | change |
  |---|---:|---:|---:|
  | plain | 5,712 | 5,665 | -0.82% |
  | defaults | 6,873 | 6,825 | -0.70% |
  | named | 5,508 | 5,457 | -0.93% |
  | env-fn | 5,480 | 5,447 | -0.60% |
  | superassign | 8,778 | 8,885 | +1.22% |
  | s3 | 32,299 | 32,407 | +0.33% |
  | trycatch | 126,612 | 126,337 | -0.22% |
  | fib | 36,708,954 | 36,558,398 | -0.41% |
  | binarytrees | 6,610,248 | 6,590,802 | -0.29% |

  `R_ENABLE_JIT=0` (every closure as the AST):

  | row | T2 (#690) | this branch | change |
  |---|---:|---:|---:|
  | plain | 8,014 | 8,182 | +2.10% |
  | defaults | 8,729 | 8,895 | +1.90% |
  | named | 7,296 | 7,511 | +2.95% |
  | env-fn | 7,510 | 7,612 | +1.36% |
  | superassign | 10,700 | 10,938 | +2.22% |
  | s3 | 32,777 | 33,067 | +0.88% |
  | trycatch | 129,336 | 129,091 | -0.19% |
  | fib | 46,137,153 | 45,973,724 | -0.35% |
  | binarytrees | 10,107,067 | 10,116,861 | +0.10% |
  - Byte code has no check, and the rows that run as byte code are at or
    below T2. They are below because the VM loop's code is laid out a
    little better.
  - `superassign`'s `a$dep` is a small closure made inside another, which
    GNU R never compiles (NOJIT), so it runs as the AST at every call. Its
    three checks and its two statements' visibility cost 107 instructions
    an iteration (+1.22%).
  - `s3` (+0.33%) is libc's allocator: the same calls to `malloc` and
    `free`, more instructions inside them; the VM loop is 35 below T2.
  - With every closure as the AST, the checks cost up to 3%.
  - The first version had the whole check inline in the VM loop's match
    arm, and it cost +1.2% to +2.9% on rows that never run it:
    register allocation across the loop. Then everything moved out of line
    (one call per check): +0.75% on `superassign`, the byte-code rows flat.
    The final form keeps only the compare inline.
- Review: Sol (`gpt-6.1-sol`, high). Round 1, changes requested, four
  should-fixes, all fixed with regressions in `suspended-body-write.R`:
  - a write that cuts a `{` after a non-final statement made it the last,
    with the wrong visibility (`tf`, auto-printed as in GNU R);
  - `while` re-read the call's CDR where `do_while` keeps its `args`
    (`wa`: a callback gives the call a new argument list, GNU R keeps the
    old);
  - skipping the entry check for a code object's root `{` and a loop's
    `{` body was unsound, because the guard's evaluation tick may run a
    finalizer: the skip is gone;
  - the live walk kept every statement rooted until the `{` ended: a scope
    per statement now.

  With the first two fixes reverted, `tf` prints nothing where GNU R
  prints `[1] 2`, and `wa` gives 1 where GNU R gives 3.
