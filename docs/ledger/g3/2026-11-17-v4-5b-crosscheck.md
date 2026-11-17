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
- **Expected classes, with their reasons:**
  - `%%` and `%/%`: bcEval has no opcode for them (cmp.R's CALLBUILTIN);
    the port's Arith computes do_arith's MODOP and IDIVOP.
  - A tail return's shape: GNU R returns from each branch (a tail
    switch's, `return(if ...)`); the port jumps to one return, with the
    value's visibility set at its load (F_VIS). Top-level printing
    matches GNU R.
- **Removed:** the wrapper class.
- **The report** names symbol and primitive constants, a loaded
  constant, the flags, and a skip's promise, for triage.

### Result

base.rdb's crosscheck: 1006 the same; 268 expected, in six classes;
38 untranslated (specials robigo-rt lacks, CALLSPECIAL); 6 skipped;
**0 unexpected**. The wrapper class is gone. The conformance crosscheck
stays at 0 unexpected.

### Verification

On V4 step 2's head (with #824's OCR fix `d278f3c9`), at `6883d428`:
- `scripts/rt-ci.py`: exit 0. Conformance 536/542, with the 6 failing as
  on main. Every observation mode, the moving collector and fail-alloc
  pass (2800 runs, 0 differences). The crosscheck compared 135, 109 the
  same, 0 unexpected.
- The rdb tests, under the crosscheck: 3 passed. The translation test
  holds V3's numbers.
- perf-rt (`scripts/perf.py rt-bars --main`, main `2a7f3be6`'s binary):
  flat, every row within +0.25%, peak RSS within 2%. The handler changes
  reach every closure the JIT compiles from source; the M1 rows use none
  of the names they touch.
- `cargo fmt --check`, `scripts/lint.sh`: clean.
