## 2026-11-17: V3: GNU R's byte code translated into the IR; #738's verified code

Package V3 (M3; `m2m3-plan.md` §B "V3"). A closure whose body is a
BCODESXP, and `eval()` of a BCODESXP, now run GNU R's own compiled code:
`crates/robigo-rt/src/compile/bctrans.rs` translates the opcode stream
into the IR under the `Bc` profile. The decisions cmp.R encoded in the
opcodes (what it inlined, behind which guard, what it folded, where a loop
needs a context) are followed, never made again. Every code object the VM
runs now passes one gate, `code::Verified` (#738).

- **Tree:** branch `g3/v3-bcodesxp-ir`, from main `fb332640` (C1-BC
  #771), with main merged in up to `78dcf1ee` (L2a #769, #772) and then
  L1b (#776, `48044290`).
- **Oracle:** GNU R 4.6.2. The ports follow its eval.c (`bcEval`) and
  cmp.R.

### The bar, and what waits

- **Translation of base's closures.** The opcode census of the oracle's
  base namespace (1,150 compiled closures, 96 opcodes) is covered: every
  opcode cmp.R emits there is translated. Two need a primitive robigo-rt
  lacks: `LOG`/`LOGBASE` (`log`, B5a; 1 closure each) raise "robigo:
  'translating byte code calling log (LOG)' is not implemented yet".
  `MATH1`, `SQRT`, `EXP` and `DOTCALL` translate to the builtin they name,
  so a closure calling `floor`, `ceiling`, `sign` or `exp` stops at that
  primitive's own not-implemented error until B5a.
- **The base.rdb test** (`oracle_base_closures_translate` in
  `crates/robigo-rt/tests/rdb_roundtrip.rs`, on L1b's data; it runs where
  the round trip does, `ROBIGO_ORACLE_LIBRARY`): every closure of the
  oracle's base.rdb, and each function it makes, translated without
  running and passed through the verifier (`compile::translation_gaps`).
  1,152 closures: 845 translate whole, 0 fail; 307 stop at a primitive
  robigo-rt does not have yet (310 stops: 238 `.Internal`s, 30 builtins,
  26 specials, 10 `MATH1` functions, 3 `.Call`, 2 `exp`, 1 `log`), each
  of which translates once the primitive exists. The test fails on code
  the translation cannot follow, an opcode it does not know, or the
  verifier's refusal. It also walks the byte code of promises
  (`MAKEPROM`) and of the functions they make. Its first run found one: a
  numeric `switch`'s empty alternative in non-tail position, whose `stop()`
  call cmp.R lets fall into the default with its value left on the stack
  (the 14 `Ops.*` and `Summary.*` group methods). That path now raises
  "robigo: 'a stack left unbalanced where the byte code joins (a 'switch'
  whose stop() returned)' is not implemented yet" after the call, which
  only a redefined `stop` that returns reaches: bcEval then goes on with
  the call's value on its stack, a later operation's operand (Sol, round
  3). The jumps to the default go on as the code says
  (`tests/rt/bctrans/loops.R`'s `k_switch_nontail`).
- **Real base code run:** `tests/rt/bctrans/basefuns.R` replays GNU R's
  installed byte code of 24 base functions (`.Internal(bodyCode(f))`,
  rebuilt with `mkCode`/`bcClose` in `.BaseNamespaceEnv`) and calls them:
  `paste0`, `lapply`, `sapply`, `vapply`, `Filter`, `match.arg`,
  `rev.default`, `ifelse`, `do.call`, `nchar`, `startsWith`, `trimws`,
  `rle`, `which`, `unique.default`, `setdiff`, `union`, `rep_len`,
  `Negate`, `isTRUE`, `xor`, `append`, `seq.default`. Their output is
  GNU R's.
- **Fidelity rows** (`tests/rt/bytecode/*`) ratchet when L1b and the
  compiler namespace are in; the five cases that load the compiler still
  wait on B5a's rows and L's attach + R_HOME (C1-BC's ledger).
- **Fixtures:** `scripts/bc-fixtures.sh` compiles each case of
  `scripts/bc-fixtures/*.R` with the oracle's `compiler::cmpfun` (or
  `compiler::compile` for top-level code), writes a script that rebuilds
  the byte code from its code and constants, and records it. Eight
  families, all ratcheted: `basic`, `consumers`, `subset`, `loops`,
  `assign`, `toplevel`, `basefuns`.

### The translation

GNU R runs byte code on a value stack. The translation walks the reachable
opcodes in order and keeps that stack as registers allocated in order;
paths that meet at a label meet with the same registers and stack (checked:
a disagreement is a translator bug, which stops a debug build and refuses
the code in a release build). A call's frame (`GETFUN`, `GETBUILTIN`,
`CHECKFUN`, `STARTSUBSET`) collects its pushes.

**Two passes.** The IR's operations want their value's register outside
their operands' window (`call_builtin_regs` clears the window after the
call; a builtin writes its value before it reads all its arguments), and
the from-source compiler allocates `dst` before it compiles the operands.
The stack code does not say where an operand's code begins, so pass 1 (on
a scratch compiler, no code cached) follows the stack and notes, for each
operation, where its first operand's code starts on each path; pass 2
reserves the value register there. The layout is then the from-source
compiler's, which the crosscheck compares. The first version wrote the
value over its first operand: `k_arith(1:3, 2L)` printed `NULL` and
`sqrt(x)` read its own result slot.

**Per opcode family** (eval.c's semantics → the IR the from-source
compiler emits for the same decision):
- arithmetic, comparison, logic, `!`, unary `-`/`+` → `Arith`, `Relop`,
  `Logic`, `Arith1`, `Not`; `&&`/`||` → `AndOr1`/`AndOr2`.
- `SQRT`, `EXP`, `IS*` (no call operand: the call is the `expressionsIndex`
  entry, as `R_getBCInterpreterExpression` reads it), `SEQLEN`,
  `SEQALONG`, `COLON`, `MATH1` (the builtin `CAR(call)` names, as
  `DO_MATH1`'s slow path), `DOTCALL` → `CallBuiltin` of the builtin: each
  opcode's fast path is that builtin's value for plain operands.
- `CALLBUILTIN` keeps the `expressionsIndex` expression when it differs
  from its call (a `.Internal` wrapper's call), for `error()`.
- `CALLSPECIAL` → the language special compiled, or `SpecialFn` for any
  other special (the registry's `rep`).
- `CHECKFUN` → a frame over the computed function; `Call` raises its
  "attempt to apply non-function" against the call under `Bc` (bcEval's
  `error()`), a fix in `call.rs` the from-source code shares.
- `STARTSUBSET_N` with `VECSUBSET`/`MATSUBSET`/`SUBSET_N` and their `[[`
  forms, and the default method's `STARTSUBSET`/`DFLTSUBSET` → `StartSubset`
  + `DfltSubset`; `DOLLAR` → `Dollar`.
- loops: `STARTLOOPCNTXT`/`ENDLOOPCNTXT` → a `Loop` region; a loop without
  a context stays jumps as cmp.R compiled it; `STARTFOR`/`STEPFOR`/`ENDFOR`
  → `ForPrep`, `ForStep` + a back-jump to the body, `ForDone`. A backward
  `GOTO` → `BackJmp` (safepoint and poll).
- `SWITCH` → `Switch` and its jump tables, one per offset.
- complex assignment, `STARTASSIGN` to its `ENDASSIGN` → one `BcAssign` of
  a plan built from the steps' opcodes: `VECSUBASSIGN`… → `K_SUB`,
  `SETTER_CALL`/`GETTER_CALL` → `K_GENERIC`, `DOLLARGETS`/`DOLLAR` →
  `K_DOLLAR`, `DFLTSUBASSIGN`/`DFLTSUBSET` → `K_SUB_PUNT`. Each argument's
  inline code (an index, a `PUSHARG`'d value, `CHECKFUN`'s function) is a
  slice of the stream, translated as a code object of its own and kept on
  its expression in the step's call, which is what `bcassign::eval_arg`
  runs; a `MAKEPROM`'s code is kept the same way. Pass 1 finds the slices
  (`Tr::open`: where the code toward each depth's slot begins).
  `INCLNKSTK`/`DECLNKSTK` are stack marks only: `BcAssign` commits the
  stack's links itself, as in the from-source code.
- `RETURNJMP` → `Return`; `DDVAL_MISSOK` → `GetVarMissOk` (as the
  from-source `missok_arg`).
- An opcode not translated yet (`PRINTVALUE`, `DUP`, `GETGLOBFUN`,
  `GETSYMFUN`, `STARTC`, `DOTSERR`, `ISNUMERIC`, `INCLNK`/`DECLNK`, none
  in base) makes the code raise "robigo: 'translating … (OPCODE)' is not
  implemented yet" when it runs; so does code the translation cannot
  follow. Neither runs the source instead.

**`eval()` of a BCODESXP** runs its translation, kept on the BCODESXP's
4th word; with byte code disabled or of a version bcEval does not run, its
source (bcEval's fallback). `compile::source_body` is that fallback for
the `Eval` profile, no longer an interim: both `INTERIM(V3)` stop-gaps of
C1-BC's inventory are gone.

### V4 handoff (foreman, Q1)

Code is cached per expression: a `MAKEPROM`'s BCODESXP is translated and
kept on its argument's expression in the call (beside any AST code), a
step's argument code on its expression in the step's call, top-level byte
code on its BCODESXP. V4 makes a promise's code per promise.

### #738: one gate for every code object

- `code::Verified` (`crates/robigo-rt/src/code.rs`) wraps `Built`;
  `code::build` takes only `&Verified`. `Verified::new` checks, in one
  pass, every register operand and window (argument windows, names,
  `for`'s state) against `nregs`, every pool index, jump target, call
  site, `Switch` table and `LdScalar` constant, that the code cannot run
  off its end, and (new in V3) that a value register lies outside its
  operands' window and a binary operation's operands are `b`, `b + 1`
  (what the slow path passes). The from-source compiler's code goes
  through the same gate; a refusal is a compiler bug and panics. Build
  time only: the VM's loop is unchanged.
- `Cenv<'g>` is branded with the `Cx` lifetime (`cenv.rs`).
- #738 closes from this PR together with X/P1's `Act` fix.

### ROBIGO_BC_CROSSCHECK (foreman, Q2 (a))

At translation time, a closure's source is also compiled by the
from-source compiler (V2's cmp.R port, the closure's environment), and
the two IRs are compared normalized (`compile/crosscheck.rs`): values
numbered by their definitions (a `Move` forwards), `Clear`s and clear
flags dropped, unreachable code dropped, a jump to a `Ret` read as the
`Ret`, `LdScalar` as `LdConst`, constants by `identical()`. The first
difference is reported to `ROBIGO_BC_CROSSCHECK_REPORT`, classed. The
from-source compile goes first, so the argument code kept on an expression
both share is the translation's; a compile that would force a lazy binding
is skipped (reported). `scripts/rt-ci.py` runs conformance under it and
fails on an unexpected difference or an untranslated closure.

Over the conformance suite: 0 unexpected; expected, each a divergence of
the from-source port (or a translation choice) to settle later:

| class | count | what |
| --- | --- | --- |
| a loop's shape | 10 | cmp.R skips a loop's context when no `break`/`next` can leave it from elsewhere (`checkSkipLoopCntxt`) and puts a `for` loop's step after its body (`cmpForBody`); the port makes a context always, the step first. |
| a constant cmp.R folds | 5 | `constantFold` folds `1:2` and `-1L`; the port computes them. |
| a nested function's code | 5 | the port compiles a nested function's source at once; the translation translates its byte code at its first call. |
| an unbalanced join the translation refuses | 1 | a non-tail numeric `switch`'s empty alternative (above): the port falls into the default. |
| guard of a declined inline handler | 1 | `tryInline` puts `BASEGUARD` before `cmpCall` when the handler declines (`sum(...)`); the port calls unguarded. |

**On base's installed code** (the base.rdb test under the crosscheck,
compiled from source for base's namespace as installed code): 72 nested
functions, 33 loop shapes, 21 folds, 6 skipped, 358 untranslated (the
primitives robigo-rt lacks, above), and 221 first differences not classed
yet. Most (161) are a call of a base closure GNU R's installed code
inlines as a simple `.Internal` (`match`, `inherits`: `cmpSimpleInternal`)
where the from-source compile, in this session's lazily bound base, calls
it. Classing them is the from-source port's (V2) to settle, so this part
is reported only: CI's rt job runs the rdb tests (where the data is) with
the crosscheck on and prints the counts per class; the job fails only when
the base.rdb test does.

Fixed rather than listed: the from-source `switch` now ends an empty
numeric alternative as cmp.R does in tail position (a jump to the end
after the `stop()` call, dead at run time), and a tail `&&`/`||` is
visible on every path to its `RETURN`.

Not a crosscheck class but a divergence of both from GNU R: bcEval's
`GOTO` counts toward its interrupt poll (`BC_CHECK_SIGINT`) forward as
well as backward; the IR's forward `Jmp` does not.

### robigo-rt gaps the fixtures met (not V3's)

Cases were rewritten around them: `forceAndCall` (special); the internals
`mapply`, `toupper`, `Recall`, `tabulate`, `strtoi`; the primitives
`floor`, `ceiling`, `sign`, `exp`, `log`; `for` over a factor
(`asCharacterFactor`); `[<-` with matrix subscripts; `[` with array
subscripts; printing a matrix.

### Verification

On `556f2d56` (main `78dcf1ee` merged), unless noted:
- **`scripts/rt-ci.py`, full:** every step passes. The new crosscheck
  step first failed on one difference the classes missed (a `for` loop's
  step placement), fixed in `556f2d56` and rerun clean: 21 expected, 3
  skipped, 0 unexpected.
- **Conformance:** 463/471 against GNU R 4.6.2; the failures are the
  three `link-walk-*` and the five compiler cases, as on main.
- **perf-rt against `78dcf1ee`** (Ryzen 9 5900X, callgrind instructions
  per iteration): 0 failures. Every row within ±0.03% but e7-match
  (+0.15%, address-sensitive: the median over heap shifts); peak RSS at
  most +0.4%.
- **Miri:** not run; robigo-rtcore and robigo-heap are unchanged.
- **`cargo fmt --check`, `scripts/lint.sh`:** pass.
- **Sol** (gpt-6.1-sol, high), round 1: two blockers, both fixed in
  `a2c63078`. `RETURNJMP`'s operand is now in tail position, so
  `function() g({ invisible(0); return(1) })` returns visibly
  (`basic.R`'s `k_returnjmp`, which the round-1 binary failed). The
  verifier checks a `Loop` region's continuation, `target + 1`, with a
  rejection test. Round 2: APPROVE.
- **After L1b's merge:** the base.rdb test (above) and the switch join.
  Sol round 3, on that commit: S3 [blocker] dropping the falling path's
  values changed what a returning `stop` gives (GNU R 12, robigo 15 in
  Sol's case), so that path now raises not-implemented; S4 [should] the
  test skipped promises' byte code, now walked; S5 [should] the rt-ci
  report step could not run in CI (the data is restored after rt-ci), so
  CI's rdb step runs with the crosscheck instead. Fixed; not re-reviewed
  (the loop's budget of three rounds).
