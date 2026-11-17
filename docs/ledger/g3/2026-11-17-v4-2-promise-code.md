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
3. Everything that reads a promise's expression applies `bytecodeExpr`
   (GNU R's `R_PromiseExpr`): `substitute`, `deparse` of a promise,
   `match.call`'s dots, the conditions' calls. To check one by one.
4. `BcAssign`'s generic steps carry their promises' BCODESXPs in the plan
   the same way.
5. V3's `promise_code` caching on argument expressions goes.
6. Deferred from #788's OCR (r4599109571): the unused `expr_at` lookup in
   `bctrans.rs`'s `DOTSERR` arm goes too.

The from-source compiler's code (`Comp`) keeps its per-expression caching
until V4 retires it to the crosscheck; its `Call`s carry no codes, so their
path is unchanged.
