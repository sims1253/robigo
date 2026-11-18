## 2026-11-17: V4 step 5b: base.rdb's crosscheck at 0 unexpected

Package V4, step 5, second half; the first was #830, #778's three CI lows.
V3 left base.rdb's crosscheck (base's installed byte code against the
from-source compiler's code of its source) at 56 closures not classed
and 160 in a class of their own, "a simple .Internal wrapper the port
calls". Both were owed to V4. Both are now 0.

- **Tree:** branch `g3/v4-5b-crosscheck`, on V4 step 2's
  (`g3/v4-2-promise-code`, #824), filed after it lands.

### The session (most of the gap)

The translation test's session had none of base's R code: `Session::jit()`
installs the evaluator and a few test builtins only. So the from-source
compile found none of base's closures (`match`, `inherits`, ...), never
inlined a `safeBaseInternals` wrapper, and called what cmp.R inlines.
That was the whole wrapper class and much of the rest.

The crosscheck now has a test of its own, `oracle_base_closures_crosscheck`.
Its isolate loads base (`base::load`) and forces every binding first, as
GNU R's build had all of base defined when it compiled base (makebasedb.R).
A definition robigo-rt cannot evaluate yet stays a promise, and a compile
that needs it is skipped. `oracle_base_closures_translate` keeps its bare
session and with it V3's measure (875 of 1152 closures whole, 315 stops
waiting on robigo-rt). With base loaded, a stub would stand where a missing
primitive is the gap it counts.

### The port against cmp.R 4.6.2 (compile.rs, cenv.rs)

- **Default handlers by base's binding.** cmp.R's handler table gives
  `cmpSpecial` to every base name whose value is a special and
  `cmpBuiltin` to every builtin (`local()` over `ls('package:base')`).
  The port went by its own table's names:
  - `rep`, a special by names.c (eval 0) that the registry's table has
    as a builtin row, was called generically;
  - the aliases were missed. GNU R 4.6.2's base binds two names to a
    primitive of another name, found on the oracle: `as.numeric` and
    `is.name` (`cenv::PRIMITIVE_ALIASES`). An alias is its primitive's,
    ahead of robigo-rt's own row of the name (it has an `is.name`).
- **`::` and `:::`** (base closures in GNU R, robigo-rt specials):
  `cmpMultiColon` calls them generically with the names as strings
  (`cmpCallSymFun`), or declines. Either way it is the IR's generic call,
  where the port made a special call.
- **`.Internal` of a builtin internal** declines when an argument is `...`
  or empty, as `cmpBuiltin` does: GNU R makes the generic call. Builtin
  is by names.c's eval column, not robigo-rt's object.
- **Stubs:** an unimplemented internal's stub takes names.c's eval
  (`robigo_rtbuiltins::names::internal_eval`). So `cbind`'s, `rbind`'s and
  `Recall`'s are specials, as `is.builtin.internal` sees them.
- **The translation:** `GETBUILTIN` of an alias reads its primitive, as
  `getPrimitive` reads the symbol's value (`as.numeric` → `as.double`).

### The crosscheck (crosscheck.rs)

- **Normalized:** an inlined simple internal compares by the call it
  reports. The port keeps the wrapper's call in `Internal`'s `g` beside the
  inlined call; GNU R's byte code has it as the call constant. Checked on
  the oracle (compiled `nchar(x, "zz")`, `rep.int(x, -1)`, `rep_len(x, -1)`):
  both report the wrapper's call.
- **`%%` and `%/%`, an expected class:** bcEval has no opcode for them
  (cmp.R's CALLBUILTIN); the port's Arith computes do_arith's MODOP and
  IDIVOP. The class holds only for the same primitive, the same call,
  flags and two operands.
- **Removed:** the wrapper class.
- **The report** names symbol and primitive constants, a loaded
  constant, the flags, and a skip's promise, for triage.

### Sol's round 1: the crosscheck checked, and what it then found

Sol's four blockers: the new test passed whatever it found (nothing
counted the verdicts), and three expected classes matched more than
their reasons (any `Ret` against a jump or `SetVis`; a visibility-only
difference by the constants' count; `%%` by the opcode alone).

- **A verdict per closure.** `compare` returns a `Finding`: its
  `Verdict` (same, expected, unexpected, skipped, untranslated) and its
  line. `crosscheck_installed` returns the verdict, and the test fails on
  an unexpected one and when fewer than 1100 closures are compared.
- **The last attempt's.** A compile that requests a constant fold runs
  again once the fold is evaluated (`attempt`). Every attempt reported a
  line, so a closure compared before its fold was known had a stale
  line beside its real one. Only the last attempt reports now. "A
  constant cmp.R folds" was made of those stale lines alone and is
  removed; in base.rdb "guard of a declined inline handler" and "`...`
  as a value" were too (the conformance suite still has one and three).
  The earlier counts were lines, not closures: 1318 lines for 1152
  closures.
- **The ways out of the code, compared.** Instead of classes for a tail
  return's shape, the normal form reads every way out as one `Ret`: a
  `Ret`, a `Return`, and a jump or a `SetVis` that leads to one through
  jumps, `Move`s, `Clear`s and `SetVis`. It carries the value returned
  (read through the `Move`s) and the visibility it leaves. An F_VIS on
  the instruction that falls through to it alone is that visibility,
  unless a `SetVis` or the `Ret` sets it. Visibility is compared where it
  is observed, so both visibility classes are gone.
- **A translation bug it found (V3).** cmp.R compiles `return(x)`'s
  operand outside the tail (`make.nonTailCallContext`). So in
  `return(if (c) NULL else y)` the first branch reaches the RETURN
  through a GOTO. The translation set F_VIS only when the next opcode was
  RETURN, so that branch returned with stale visibility. bcEval's loads
  set `R_Visible`, so after an `invisible()` call GNU R prints `NULL` and
  robigo-rt printed nothing. A value that reaches a RETURN through GOTOs
  is now in tail position (`bctrans::returns_after`). The case is
  `tests/rt/bctrans/visible.R` (from `scripts/bc-fixtures/visible.R`). It
  fails on the step's first cut (`k_retif`, `k_retif_const`) and passes
  now.
- **A port gap it found (V2), step 5d.** cmp.R's non-tail operand also
  changes what GNU R's byte code returns. A value missing there is
  visible: `return(if (FALSE) 1)`, a switch miss, `return(x <- 1)`.
  Checked on the oracle: compiled, `return(switch(x, "a", "b"))` with
  `x = 3` returns a visible NULL; the AST and a tail switch return it
  invisible. The port compiles the operand in the tail, so JIT-compiled
  code returns these invisible. The foreman approved step 5d, which
  ports cmp.R's non-tail operand. Until then the gap has a class of one
  instance in base.rdb (`mode`'s `return(switch(...))`): the same NULL
  just loaded, returned visible by the translation and invisible by the
  port. 5d drops it.

### Sol's round 2

S1-S4 resolved, three new blockers in what replaced them:

- **A tail `&&` set visibility at a RETURN others share.** Its AND2ND
  set a flag the next RETURN read (`vis_at_return`). In
  `return(if (c) x && y else invisible(1))` that RETURN is the `else`'s
  too, which came back visible. The GOTO tail above reached the first
  branch, but the last branch had it already. The translation now does
  as the from-source code does: a SetVis at the `&&`'s own join, with
  AND1ST's jump patched to land on it. bcEval's AND1ST and AND2ND both
  set R_Visible. `visible.R` has the cases (`k_retand`, `k_retand_last`,
  `k_tailand`); the binary before the fix printed `[1] 1` for two of them.
- **F_VIS is not a way out's visibility for every instruction.** A
  `GetVar` sets it before it forces a promise, whose code may call
  `invisible()` (bcEval's DO_GETVAR alike). The fold now takes only a load
  that runs no code after it: a constant (`LdConst`), `Quote`,
  `MkClosure`.
- **A Clear on the way out is not neutral.** `exit()` reads the value
  back through the Moves and Clears in order, and a Clear of the value,
  or of a Move's source before the Move, makes the way out none. Unit
  tests (`crosscheck::tests`) pin it.

### Sol's round 3

S6 and S7 resolved. The `&&` pairing was not (S5, S8): AND1ST and AND2ND
were paired by a stack, and an AND2ND no path reaches is never
translated. In `a && { b && return(TRUE); FALSE }` the outer AND2ND took
the inner AND1ST, whose jump then landed on the outer SetVis, so the
call returned TRUE for `(TRUE, FALSE)` where GNU R returns FALSE. An
AND2ND now finds its AND1ST by the join, the code position AND1ST
jumps to, which is right after its AND2ND. `visible.R`'s
`k_and_inner_return` is the case.

### Sol's round 4

S8 resolved. The tail `&&`'s visibility was still wrong when its AND2ND
is unreachable (S9): in `{ a <- FALSE; a && return(TRUE) }` no SetVis was
translated, so AND1ST's path returned FALSE invisible after the
assignment, where GNU R returns it visible. bcEval's AND1ST and AND2ND
each set R_Visible, and the IR now says so: `AndOr1` and `AndOr2` honor
F_VIS (vm.rs, one flag test in their arm). The translation sets it on
both when the `&&`'s join leads to a RETURN, and the from-source
compiler sets it on both in the tail. The SetVis, the pairing by join
and the re-patch are gone. `visible.R`'s `k_and_return` and
`k_or_return` are the cases.

The crosscheck's numbering had to follow: a join only a jump reaches
(no path falls into it, the jumps agree) takes the jumps' values, not
those the code before it left. Elsewhere values stay numbered in the
code's order.

### Sol's round 5

S5 resolved. S9 was resolved in the VM, but the inline evaluator
ignored the flag: it honors F_VIS only for a whitelist of ops, and
`AndOr1`/`AndOr2` were not on it (S10, blocker). They are now. No fixture
case shows it, because an inline run cannot leave R_Visible false before
the `&&`. The crosscheck's rule for a join that only jumps reach ignored
back edges (S11, should): it now skips a target that a back edge reaches.

### Merging main, and H5

Main `e20eef0d1` was merged at `e0b5a866`. It brought H5 (#811):
symbols are immutable, and a symbol's INTERNAL lives in the isolate's
table, not in its tag. This step's decline for a `.Internal` call with
`...` or an empty argument (cmpDotInternalCall's cmpBuiltin) still read
the tag, so it never declined. The port then compiled
`.Internal(pmax(na.rm, ...))` and the like as a direct `Internal`,
where GNU R calls `.Internal` (9 unexpected in base.rdb: pmax, pmin,
order, sprintf, options, radixsort, icuSetCollate). The decline reads
`internal_raw` now, and base.rdb is back to 0 unexpected.

### The cost of the visibility test

perf-rt at `e0b5a866` was +0.9% to +1.1% on the closure-call rows
(plain, defaults, named, env-fn) against main's binary. V4 step 2
merged with the same main was flat on them, so the cost is this step's.
With main's vm.rs, this step was flat, so it came from the one vm.rs
change: the `if i.fl & F_VIS != 0 { set_visible(true) }` in the AndOr
arm. Callgrind put all of it in `vm::run`'s self cost (+11 instructions
an iteration on plain), which runs no AndOr: the branch shifted the
function's code. Moving the whole arm out of line, as `math1` is, still
cost +0.6%.

bcEval's AND1ST, AND2ND, OR1ST and OR2ND each set `R_Visible = TRUE`
unconditionally, after the operand's check (eval.c:8361-8409), and the
AST's `do_logic2` is visible too. The arm now does that: a store, no
test. F_VIS stays in the IR, where the crosscheck compares it and the
inline evaluator reads it. Callgrind against main `046c437c`: plain
+0.13%, defaults, named, env-fn and guard-subset-dbl within ±0.1%.

### Result

One verdict per closure. base.rdb: 980 the same, 145 expected in five
classes (a nested function's code 63, a loop's shape 60, a switch's
stop() 12, `%%`/`%/%` 9, return()'s operand 1), 21 untranslated
(specials robigo-rt lacks, CALLSPECIAL), 6 skipped, **0 unexpected**.
The conformance crosscheck: 122 the same, 21 expected, 3 skipped,
**0 unexpected**.

### Verification

The first cut was verified on V4 step 2's head (with #824's OCR fix
`d278f3c9`), at `6883d428`:
- `scripts/rt-ci.py`: exit 0. Conformance 536/542, with the 6 failing as
  on main. Every observation mode, the moving collector and fail-alloc
  pass (2800 runs, 0 differences).
- perf-rt: flat, every row within +0.25%.

After Sol's rounds 1 to 4:
- robigo-rt's conformance: 537/543. The new `bctrans/visible.R` passes and
  is ratcheted; the 6 failing are the same as before.
- The rdb tests, under the crosscheck: 3 passed. The translation test
  holds V3's numbers.
- perf-rt after round 1 (`scripts/perf.py rt-bars --main`, main
  `2a7f3be6`'s binary): flat. The worst row is binarytrees at +0.30%, the
  call rows are 0.2% faster, and peak RSS is within 2%. Round 2 changes
  where a tail `&&`'s SetVis goes in translated code. Step 5c's perf-rt
  measures it with this step whole. The translation's tail change
  reaches base's translated closures, but the M1 rows' code is compiled
  from source.
- The full `scripts/rt-ci.py`, with the observation modes and the moving
  collector, ran on step 5c's head, which has this step whole.
- `cargo fmt --check`, `scripts/lint.sh`: clean.
