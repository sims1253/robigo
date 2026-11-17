## 2026-11-17: V4 step 2: promise code per promise (draft)

Package V4, step 2 of 5. The bar: perf-rt flat (the foreman: if a row
moves, the measurement comes with the PR).

- **Tree:** branch `g3/v4-2-promise-code`, from V4 step 1's head
  (`862ae65a`, #788); main is merged in once #788 lands.

### The problem

GNU R's `MAKEPROM` makes a promise whose `PRCODE` is the BCODESXP the
compiler made for that argument, so forcing it runs that code (`bcEval`).
V3 keeps a translated promise's code on the argument's *expression* in the
call (`crate::code::cache_on`, `bctrans.rs`'s `promise_code`). That is per
expression: two code objects whose calls share an argument expression
share one cache slot, and the last translation wins.

### Where robigo-rt makes an argument promise

- The translated `CALL` is the IR's `Call` (a = value, b = function, c =
  call site, e = call); `vm.rs` → `call::call` → `args::call_closure`
  (`args.rs:2101`) → `apply(.., Source::Site(site), ..)`.
- Each actual of the call is an `Actual { val: the argument's expression,
  from: Origin::Expr }`. Its slot is `SlotInit::Delayed { expr, value:
  None, bytecode }` (`args.rs:1516`); in a list (`...`), `list_element`
  (`args.rs:1010`) makes a promise and marks it
  `extra::promise::BYTECODE`.
- The slot's `META_BYTECODE` (`robigo-heap/src/slots.rs:86`) makes forcing
  call `Hooks::eval_bytecode` (`robigo-rtcore/src/hooks.rs:160`), which
  evaluates the expression under the `Bc` profile, its code cached on the
  expression (`eval::expr_code`).
- `BcAssign`'s generic steps make their promises the same way
  (`bcassign.rs:927`).

### The design

1. The translated `CALL`'s `Call` carries a constant (`f`): the call's
   arguments' `MAKEPROM` BCODESXPs, one per argument cell (R_NilValue for
   one that is not a promise: a constant, `...`, a missing argument). The
   verifier checks it as a pool index or 0.
2. `apply` gets those codes for `Source::Site`. The delayed slot (or a
   `...` list's promise) of call argument `j` takes code `j` as its
   expression, GNU R's `PRCODE`, still `META_BYTECODE`. Forcing it then
   evaluates the BCODESXP: `eval`'s BCODESXP case translates it once and
   keeps the code on that node (`compile::bytecode_code`). The mapping goes
   by call argument cell, since a `...` argument expands into several
   actuals.
3. Everything that reads a promise's expression applies `bytecodeExpr`:
   GNU R reads it only through `PREXPR`, defined as `R_PromiseExpr`
   (`Rinternals.h:1269`), at 15 sites in six files: `coerce.c` (2,
   `substitute`), `deparse.c` (1), `identical.c` (2), `match.c` (2),
   `unique.c` (1), `envir.c` (7: `R_isMissing`, `delayedAssign`, ...).
   robigo-rt gets one rtcore helper, `R_PromiseExpr`'s port (a BCODESXP's
   first constant, else the code), used at the equivalent of each site and
   wherever a delayed slot's expression is read as an expression
   (`ArgValue::Delayed`, `ArgSlot`: `missing()`, `substitute()`); forcing
   keeps the raw code.
4. `BcAssign`'s generic steps carry their promises' BCODESXPs in the plan
   the same way.
5. V3's `promise_code` caching on argument expressions goes.
6. Deferred from #788's OCR (r4599109571): the unused `expr_at` lookup in
   `bctrans.rs`'s `DOTSERR` arm goes too.

The from-source compiler's code (`Comp`) keeps its per-expression caching
until V4 retires it to the crosscheck; its `Call`s carry no codes, so their
path is unchanged.

### What landed

- **The codes.** `bctrans`'s `CALL` collects its pushes' `MAKEPROM`
  BCODESXPs (symbols' too: cmp.R's `cmpCallArgs` gives a symbol argument
  byte code, `GETVAR`) into a list constant, `Call`'s `f`; each is
  translated with the code it runs in (the caller's compile environment,
  its frame's slot indices) and the translation kept on the BCODESXP
  itself (`code::cache_on`), no longer on the call's argument expression.
  A complex assignment's generic step carries its promises' codes the same
  way (`bcassign`'s `S_CODES`). `args::arg_code(codes, pos)` reads code
  `pos`, null for none (a constant, `...`, an empty argument, `bquote`'s,
  the from-source compiler's calls: `f` 0).
- **Where a promise is made.** `call::call` takes the codes; a closure's
  delayed slot (by the site's plan or the matcher, by the actual's call
  position), a `...` list's promise (`list_element`) and a complex
  assignment's closure argument (`closure_args`) hold code `pos` as their
  code, still byte code's (`META_BYTECODE`, `promise::BYTECODE`). A
  builtin called through `CALL` (or a complex assignment's builtin getter
  or setter) runs the argument's code in place, `MAKEPROM`'s `bcEval(code,
  rho)`: `eval::bc_eval_inline`, no `eval` level, bcEval's entry poll.
- **Forcing.** `eval_bytecode` of a BCODESXP is `forcePromise`'s
  `eval(PRCODE)`: `eval`'s BCODESXP case (its count and level), the
  translation kept on the code, bcEval's poll. An early value
  (`crate::early`) reads the translation as it reads a call's code, and
  the early force counts what that force counts: the early-force hook
  takes the code's type (`SYMSXP`, `LANGSXP`, `BCODESXP`, both counts for
  byte code). Pure-closure inlining takes the codes as its arguments
  (`inline::actual`), evaluates them as calls' code, and fuses `GetFun` +
  `Call` only for a call without codes.
- **Readers (PREXPR).** rtcore's `bytecode_expr` (`bytecodeExpr`,
  `R_PromiseExpr`): `frame_arg_raw`'s `ArgValue` (`missing()`,
  `R_isMissing`), `findRootPromise`, `substitute()`, `match.call()`'s
  `...`, deparse's `<promise: >`, the unused-argument message. `ArgSlot`
  stays PRCODE (forcing, aliases). `identical()` of promises is still
  not implemented in robigo-rt (its error stands).
- **Serialization.** An unforced promise of a translated call is written
  with its code, the BCODESXP, as GNU R writes it; robigo-rt refused that
  before ("serializing a promise of byte code is not implemented yet",
  which still covers the from-source compiler's promises).
- **Fixtures.** `tests/rt/bctrans/promise.R` (13 cases: `substitute`,
  `missing` through a symbol argument, `match.call`'s `...`, the unused
  argument, forcing, `...`, a builtin through `CALL`, inlining, a setter's
  and a builtin getter's arguments, an environment's serialization byte
  for byte and read back), recorded on 4.6.2; ratcheted.
- **The crosscheck.** A translated `Call`'s codes were already outside
  its comparison (`f`); a complex assignment's plan now is compared but
  for its steps' codes (`bcassign::same_but_codes`), which the
  from-source compiler's steps have none of. The first full rt-ci run
  flagged the two new assignment fixtures as unexpected without it.

### Review

- Sol round 1 (`01564e8c`): APPROVE, no findings.

### Verification

On main `f7378ac3` with #810 (`9987b86b`) merged, at `a3149e98`:

- `scripts/rt-ci.py`: exit 0. Conformance 523/533: the 8 failing as on
  main, plus `math/math1-compiled.R`, which #812 fixes. Every observation
  mode, gctorture, the moving collector and fail-alloc pass (2800 runs,
  0 differences). The crosscheck compared 127, 101 the same, 0 unexpected.
- Miri, `cargo +nightly-2026-11-02 miri test -p robigo-rtcore` (rtcore
  changed: `bytecode_expr`, the early-force hook, `frame_arg_raw`): 121
  passed, 10 ignored, 0 failed.
- `scripts/lint.sh`, `cargo fmt --check`: clean.
- perf-rt: measured after #812 lands, on top of it (S1's and C1's
  vm.rs order: whoever lands second re-measures).
