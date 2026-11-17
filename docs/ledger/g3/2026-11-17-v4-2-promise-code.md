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
  byte code, `GETVAR`) into a list constant, the `f` of an op of its own,
  `CallCodes` (`Call` with codes; the from-source compiler's `Call`, its
  VM arm and `call::call` stay as they were on the hot path); each is
  translated with the code it runs in (the caller's compile environment,
  its frame's slot indices) and the translation kept on the BCODESXP
  itself (`code::cache_on`), no longer on the call's argument expression.
  A complex assignment's generic step carries its promises' codes the same
  way (`bcassign`'s `S_CODES`). `args::arg_code(codes, pos)` reads code
  `pos`, null for none (a constant, `...`, an empty argument, `bquote`'s,
  the from-source compiler's calls).
- **Where a promise is made.** `CallCodes`' arm (`vm::call_codes`, out of
  line) calls `call::call_codes`, which takes the codes; a closure's
  delayed slot (by the site's plan or the matcher, by the actual's call
  position), a `...` list's promise (`list_element`) and a complex
  assignment's closure argument (`closure_args`) hold code `pos` as their
  code, still byte code's (`META_BYTECODE`, `promise::BYTECODE`). A
  builtin called through `CALL` (or a complex assignment's builtin getter
  or setter) runs the argument's code in place, `MAKEPROM`'s `bcEval(code,
  rho)`: `eval::bc_eval_inline`, no `eval` level, bcEval's entry poll.
- **Forcing.** `eval_bytecode` of a BCODESXP is `forcePromise`'s
  `eval(PRCODE)`: `eval`'s BCODESXP case (its count and level), the
  translation kept on the code, bcEval's poll. A promise of byte code
  gets no early value (`crate::early` computes them for a symbol or a
  call), so the early force and its hook are as they were. A translated
  call with codes does not run inline yet (`CallCodes` is no op
  `crate::inline` runs): the real call makes its promises. Both come back
  for translated calls with an entry of their own when translated code is
  the common path. A first version threaded the codes through
  `inline::enter`'s matching and `call::call`, and every call of the
  from-source compiler's code paid for it (perf-rt plain +4.8%, fib +7.5%;
  then, with inlining declined but the codes still a parameter of
  `call::call`, plain +0.83%; see Verification). Inlining translated calls
  comes back when translated code is the common path (V4 retires the
  from-source compiler), with an entry of its own (a const-generic
  `enter`) so the from-source path stays as it is. "No codes" is a null
  `Sexp` on the hot paths (a register test, no load).
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
- **The crosscheck.** `CallCodes` compares as `Call`, its codes (`f`)
  outside the comparison; a complex assignment's plan is compared but
  for its steps' codes (`bcassign::same_but_codes`), which the
  from-source compiler's steps have none of. The first full rt-ci run
  flagged the two new assignment fixtures as unexpected without it.

### Owed: a translated call's perf row (for V4 step 3, after L5)

perf-rt's rows run robigo-rt's own from-source compile (`Comp`): its
`Call`, `call::call` and inlining are main's, so "flat" above says nothing
about a translated closure's call (`CallCodes`, no inline run, no early
values). #824's OCR (r4604947677) asked for that path to be gated once
translated code is the common path. Owed with V4 step 3: a
`calls-rows.toml` row whose closure is GNU R's byte code (rebuilt with
`.Internal(mkCode())` and `.Internal(bcClose())`, as `scripts/bc-fixtures.R`
does, or compiled by cmp.R once the compiler namespace loads), measured
against GNU R and main. Then the inlining and early values of calls with
codes come back, with an entry of their own, measured by that row.

### Review

- Sol round 1 (`01564e8c`): APPROVE, no findings.
- Sol round 2 (`5ccf7ae2`, after the redesign below): CHANGES, S1
  [should]: `Source::Site` carried the codes as a raw `Sexp` across
  points that collect (the entry safepoint, `closure_code`, warning
  handlers, the `...` list's allocation retries), stale after a moving
  collection. Fixed in `d232ca12`: the site carries the codes' pool index
  in the caller's code; `args::prepare` reads them from the caller's
  rooted code where it uses them, after anything that collects (each
  allocation attempt too); the builtin path reads them just before rooting.
- Sol round 3 (`d232ca12`): APPROVE, S1 resolved, no new findings.

### Verification

On main `2a7f3be6` (#812, #808) merged, at `d232ca12`:

- `scripts/rt-ci.py`: exit 0. Conformance 536/542 (the 6 failing as on
  main: the 5 `bytecode/` compiler-namespace cases and
  `math/log-missing-class`); every observation mode, gctorture, the
  moving collector, fail-alloc (2800 runs, 0 differences); the crosscheck
  135 compared, 110 the same, 0 unexpected. `bctrans/promise.R` also
  under `ROBIGO_GC_MOVE=1 ROBIGO_GCTORTURE=1`.
- Miri, `cargo +nightly-2026-11-02 miri test -p robigo-rtcore` (rtcore:
  `bytecode_expr`, `frame_arg_raw`): 122 passed, 10 ignored, 0 failed.
- `scripts/lint.sh`, `cargo fmt --check`: clean.
- perf-rt (`scripts/perf.py rt-bars --main`, main `2a7f3be6`'s binary,
  this runner): flat. plain -0.03%, defaults +0.01%, named +0.07%,
  env-fn -0.02%, superassign +0.02%, s3 +0.15%, fib +0.02%, binarytrees
  +0.25%, e7-* within ±0.02%; peak RSS within 2%; 0 failures.
- The road there, on the same runner and baseline:
  1. The codes as a `Call` field, threaded through `call::call`,
     `inline::enter` and the early force: plain +4.8%, defaults +3.4%,
     named +4.8%, env-fn +3.1%, superassign +1.6%, binarytrees +1.9%,
     fib +7.5%. Callgrind: `inline::enter` +79 instructions a call.
  2. Inlining declined for calls with codes, "no codes" a null `Sexp`:
     plain +0.83% (`early_force` out of line; the codes still a
     parameter of `call::call`).
  3. `CallCodes` an op of its own, its arm one out-of-line call taking
     `&View`: plain +3.2% (`call::call`, now with two callers, outlined
     from `vm::run`).
  4. `call::call` pinned inline: plain +1.7% (`reload!` of the `View`:
     the out-of-line helper took its address).
  5. `CallCodes`' arm shaped as `Call`'s, its path its own functions
     (`call::call_codes`, `args::call_closure_codes`,
     `call_builtin_list_codes`), `call::call`, `inline` and `early` as on
     main: flat.
