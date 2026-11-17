## 2026-11-17: C1-BC: BCODESXP as an object, the byte-code builtins, and cmp.R from source

Package C1-BC (M3, wave 0; `m2m3-plan.md` §B). GNU R's byte-code object
is now a heap type of robigo-rt's, with eval.c's internals over it, and
the compiler package's cmp.R runs from source on robigo-rt.

**The bar:** cmp.R's `cmpfun`, run from source on robigo-rt, gives a
`disassemble()` identical to the oracle's on the bytecode-fidelity
fixtures. It holds:
- `tests/rt/bytecode/fidelity.R` compiles each function the fixtures were
  made from: `tests/r/serialize/bytecode-fixtures/generate.r`,
  `generate-constructed.r`, `compiled-fixtures/source.txt`, and the live
  comparisons in `bytecode_construct.R` and `bytecode_compiled_files.R`.
  The fixture files themselves are serialized; reading them is L1's.
- `cmpfun.R` and `cmpfun-ops.R` add the constructs cmp.R inlines:
  control flow, complex assignment, arithmetic and math, logical
  operators, `missing` and `on.exit`, closures, `...`, `switch` and
  `tryCatch`.
- All six bytecode cases (`internals`, `closures`, `cmpfun`,
  `cmpfun-ops`, `compile`, `jit`), `fidelity.R` and
  `tests/rt/strings/grep.R` give their 4.6.2 recordings.

- **Tree:** branch `g3/c1-bc-bcodesxp`, from main `d25d0586`.
- **Oracle:** GNU R 4.6.2 (`scripts/oracle-run.sh 4.6.2
  scripts/regen-expected.sh`). The ports follow the 4.6.2 sources. Of the
  code ported here, only eval.c's `loadCompilerNamespace` differs from
  4.5.0's: 4.6.2 calls `R_FindNamespace`.

### The object

GNU R's BCODESXP is a cons cell: CAR the code (INTSXP), CDR the
constants (VECSXP), TAG the expression (`BCODE_CODE`, `BCODE_CONSTS`,
`BCODE_EXPR`). robigo-rt's node has the same layout:
- `View::Bytecode` (`crates/robigo-rtcore/src/handle.rs:689`) and
  `Ref::as_bytecode` (`:302`);
- `Cx::bytecode(code, consts)` (`crates/robigo-rtcore/src/fresh.rs:134`).

robigo keeps the code in its portable form: the version word, then the
opcodes and operands. GNU R threads it (each opcode an address), and the
threaded form is not observable: `inspect` prints a BCODESXP's header
alone, and `disassemble` and `serialize` decode it. So `mk_code` makes
`R_bcEncode`'s decisions and keeps the integers, and `decode`
(`R_bcDecode`, `crates/robigo-rtbuiltins/src/bytecode.rs:250`) copies
them.

The internals are in `crates/robigo-rtbuiltins/src/bytecode.rs`.
- **The opcode table:** eval.c's enum and `bcEval`'s `OP(name, argc)`,
  129 opcodes (`:33`). The opcode numbers and operand counts are cmp.R's
  `Opcodes.names` and `Opcodes.argc`; `disassemble` on the oracle checks
  them case by case.
- **`mk_code`** (`:235`) is `do_mkcode` with `R_bcEncode`'s decisions:
  - empty code gives `R_NilValue`;
  - a version outside 9..=12 gives `c(v, BCMISMATCH_OP)`;
  - otherwise version 12 is installed (2 for a mismatch code), and each
    opcode is checked ("unknown instruction code");
  - a double code vector is read as ints, low word first.
- **The rest:** `bcClose`, `disassemble` (recursive, the constants
  duplicated), `bcVersion`, `growconst`, `getconst`, and
  `is.builtin.internal`. The last reads names.c's `.Internal` table
  first (`crates/robigo-rtbuiltins/src/names.rs`, generated, 488 rows),
  then the symbol's INTERNAL.
- **`putconst`** writes its buffer in place even when the buffer is
  shared, as GNU R's `SET_VECTOR_ELT` does, so aliases see the write.
  The new primitive for that is `Cx::set_vector_elt_shared`
  (`crates/robigo-rtcore/src/edit.rs:227`).
- **`identical`:** `IGNORE_BYTECODE` compares each closure's
  `BODY_EXPR`. A BCODESXP compares its code, expression and constants
  (`crates/robigo-rtbuiltins/src/identical.rs`).
- **`body`, `bodyCode`, `args`, `dput`, deparse and print** read
  `bytecodeExpr` for a byte-code body. `environment<-` makes the body
  the source again (`R_ClosureExpr`, builtin.c:322-325). `enableJIT` and
  `compilePKGS` keep GNU R's state.

L1's API note went to the foreman at `6489de4e`: `mk_code`, the view,
`decode`, `version_ok` and `expr`. It has not changed since.

### For V3: translating BCODESXP (lead decision F9)

Every `Bc` body will come from translating a BCODESXP. The object is
made for that:
- **The input:** the portable code, its constants (the first is the
  source expression; cmp.R puts the `expressionsIndex` and, with source
  references, the `srcrefsIndex` last) and the opcode table with each
  opcode's operand count. These are exactly what cmp.R's `bcDecode`
  reads. Labels are code offsets, as in GNU R.
- **The seam:** one function, `compile::source_body`
  (`crates/robigo-rt/src/compile.rs:171`). It is the only place that
  turns a BCODESXP body into what robigo compiles. Today it returns
  `bytecodeExpr` (INTERIM(V3)). V3 replaces it with the translation and
  keeps its callers, `compile_closure` and `compile_closure_bc`.
- **The cache:** a closure whose body is a BCODESXP gets robigo's
  compiled mark and code (`jit::set_compiled`,
  `crates/robigo-rt/src/call.rs:218`). The next call takes the common
  path. `jit::bytecode_body` and `is_compiled` say "compiled" for either.
- **The trust boundary (#738, the foreman's decision, V3's scope).**
  rt's VM trusts its IR: `code::Built` has public fields
  (`crates/robigo-rt/src/code.rs:62`), and `code::build`'s `# Safety`
  asks only for liveness. That holds while rt's own compiler is the only
  source. V3 adds GNU R byte code from `.rdb` files as a second source,
  so the translation must produce a `Verified` code type, and the VM
  takes only that type. Before it exists, the translation checks:
  - each operand index against the constant pool and the frame shape;
  - each jump target;
  - the register counts.

  V3 also brands `Cenv<'g>` (`crates/robigo-rt/src/cenv.rs:88`). The V3
  PR closes #738, together with X/P1's `Act` fix.
- **The fallback:** `R_disable_bytecode` or a version mismatch runs the
  source (`jit::closure_profile`), as `bcEval` does. A BCODESXP
  evaluated directly (`eval(compile(e))`, `crates/robigo-rt/src/eval.rs:117`)
  evaluates its expression (INTERIM(V3)).

### The compiler namespace from source (INTERIM(L2))

`loadNamespace("compiler")` has no real implementation before L2.
`crates/robigo-rt/src/ns.rs` fills the seam L2 will fill (Q1 (a)):
- `find_namespace` (`:191`) is `R_FindNamespace`, and `get_ns_value`
  (`:86`) is a full `R_getNSValue`: the exports environment, lazydata,
  and "'%s' is not an exported object from 'namespace:%s'".
- `load_namespace` (`:210`) loads "compiler" and raises "not implemented
  yet" for anything else.
- `load_compiler` (`:226`) builds the namespace as `makeNamespace` does:
  - the imports environment (parent the base namespace) and `.__NAMESPACE__.`
    with `spec` (version 4.6.2), `exports`, `lazydata`, `imports`, `path`,
    `dynlibs` and `S3methods`;
  - `.__S3MethodsTable__.`, then registration.

  It then evaluates the pinned tarball's cmp.R unmodified, runs
  `.onLoad`, binds NAMESPACE's exports and locks both environments. An
  error unregisters the namespace.

The base functions cmp.R needs come in through base-subset.R's inputs (a
new input, `compiler`, and the `bytecode` area). Each internal they reach
is a real port. Loading the namespace surfaced these, each ported:
- `substitute` in a list (`list_env`);
- replacement functions and `UseMethod` targets, which the static walk
  now counts;
- coerce.c's `as.function.default`;
- `T` and `F`;
- grep.c (first an interim matcher; RX's #764 now, below);
- `getRegisteredNamespace`, `isRegisteredNamespace` and `environmentName`;
- `[[<-` on a call (cmp.R's `flattenPlace`), below;
- `.GlobalEnv`;
- `eval` of byte code.

### grep.c: RX's ports (#764) replaced the interim matcher

This branch carried an interim matcher (Q3 (a), INTERIM(RX)). It ported
`do_grep` and `do_gsub`'s entry points around a leftmost-longest NFA for
TRE's plain ERE subset, and refused everything else. RX's #764 merged
first: TRE, PCRE2 and grep.c's fixed path for `grep`, `grepl`, `sub`,
`gsub`, `regexpr`, `gregexpr`, `regexec` and `strsplit`. Under the
foreman's rule the second to merge drops its matcher, so this branch
removed `grep.rs` and its four registry rows and kept the cases.
- `tests/rt/strings/grep.R` covers the edges: anchors, bracket ranges
  and TRE's negation quirk (`[^a-cb-e]` matches `d`), quantifiers on
  `.`, empty matches in `gsub`, the replacement escapes, `fixed`, NA in
  every argument, `invert`/`value` with names, attributes, encodings,
  strings the matcher's encoding cannot read, and the argument checks.
- `tests/rt/strings/grep-generated.R`: 300 seeded random `sub`/`gsub`/
  `grepl` cases over that subset, in ASCII and UTF-8, with `useBytes`
  and `fixed` variants, each printing the result's encoding.
- Both pass on RX's ports. They need `Encoding` and `setEncoding`, which
  this branch ports (util.c, below).

### Other ports

- **subassign.c:** `[[<-` on a pairlist or call (`SimpleListAssign`,
  `listRemove`, `GetOneIndex`), also when recursing into one. Several
  subscripts on a pairlist stay "not implemented yet".
- **util.c:** `Encoding`, `setEncoding`.
- **eval.c:** `do_eval` evaluates a BCODESXP, as GNU R's
  `isByteCode(expr)` case does.
- **base:** `.GlobalEnv` is bound to the global environment
  (`crates/robigo-rt/src/base.rs`). GNU R's base build evaluates eval.R's
  `.GlobalEnv <- environment()` at top level; a promise in the base
  namespace would give base.

**B5a-file ports, for the foreman:**
- builtin.c: `body`, `bodyCode`, `parent.env` (rtbuiltins `builtin.rs`);
  `dput`, `args`, `environmentName` (rt `builtin.rs`).
- envir.c: `topenv`, `isNamespaceEnv`, `getRegisteredNamespace`,
  `isRegisteredNamespace`, and `R_getNSValue` for `::` and `:::`. These
  are in rt's `ns.rs`, which already held envir.c's `do_colon2` and
  `do_colon3`. robigo-rt has no `envir.rs`.

**The overlap with L2a** (`g3/l2-base-source`; the foreman's rule:
whoever merges second keeps the other's, unless theirs is closer to the
C).
- **Ported here:**
  - `isNamespaceEnv`, `topenv`, `getRegisteredNamespace`,
    `isRegisteredNamespace`, `R_FindNamespace`, and `R_getNSValue` for `::`
    and `:::` (exports, lazydata, the not-exported error), all in rt
    `ns.rs`;
  - `environmentName` (rt `builtin.rs`);
  - `substitute(expr, list)` (rt `specials.rs`, `list_env`).
- **Not ported here:**
  - `registerNamespace`, `unregisterNamespace` and
    `getNamespaceRegistry` as internals. The INTERIM(L2) loader writes
    rtcore's registry directly (`cx.namespace_registry()`, the
    `defineVar` that `registerNamespace` does). Once both are on main, it
    registers through L's.
  - `importIntoEnv`, `attach`, `detach`, `search`.
- **The foreman's decision** (L compared both branches at `805824ea`):
  - **The same behaviour, so the first to merge wins:**
    `substitute(e, <list>)`, `isNamespaceEnv`/`R_IsPackageEnv`, `topenv`
    and `environmentName`. For `topenv`, this one stays because its shape
    cache is faster.
  - **L's win whichever merges first,** because they are closer to the C:
    - `R_getNSValue`. `::` loads a missing namespace through R's
      `loadNamespace` (`callR1`, so the load shows in `sys.calls()` and
      in error calls). The "not exported" path goes through R's
      `getNamespaceName`. String names are translated and installed.
    - `getRegisteredNamespace` and `isRegisteredNamespace`: a name that
      is neither a string nor a symbol is coerced to a symbol. This
      branch raises "not implemented yet" there instead.
  - The INTERIM(L2) compiler loader becomes a hook in L's `callR1` step:
    `compiler` goes to it, everything else to `loadNamespace`.
  - If C1-BC merges first, these stay as they are and L replaces them on
    rebase. If L2a merges first, C1-BC drops its three and registers the
    hook.

### Stop-gap inventory

| marker | what | where | owner |
|---|---|---|---|
| INTERIM(V3) | a BCODESXP body or value runs its source expression, compiled by robigo's own decisions | `rt/compile.rs:163-176`, `rt/eval.rs:117` | V3 |
| INTERIM(L2) | `loadNamespace("compiler")` builds the namespace from the pinned tarball's cmp.R; any other namespace is "not implemented yet"; `path` is the tarball's | `rt/ns.rs:5-11,206-226` | L2 |
| INTERIM(L5) | `T` and `F` bound in base; `.ArgsEnv`/`.GenericArgsEnv` empty | `rt/base.rs:174-176,195` | BW (S1), then L5 |
| INTERIM(BW) | rt-ci's "conformance under ROBIGO_GC_MOVE with gctorture (E3)" step allows 1200 s per case, up from 600 (below) | `scripts/rt-ci.py` | BW (S1) reverts it |

**The base heap and the moving-collector step** (the foreman's decision:
option 3, as INTERIM(BW)).
- **Cause:** robigo-rt binds each base closure as a promise of its parsed
  right-hand side, so the bound's ASTs are on the heap from startup.
  The static walk now takes in what cmp.R names transitively: 415
  closures, 226 internals and 9 variables, against main's 188 closures.
  - At startup, after `gc()`: 51.4k cells, against 15.6k on main `fb429442`
    (3.3×).
  - Under `ROBIGO_GCTORTURE=1 ROBIGO_GC_MOVE=1`, every allocation runs a
    full moving collection, so a case costs its allocations × the heap.
    `tests/r/jit/loop-closed-form-super.R` alone takes 598.5 s on this
    branch, against 154 s on main, on the same machine. In the parallel
    conformance run it passes 600 s. The output is identical.
  - A 20k-iteration probe of its `<<-` loop: 40.8 s against 10.8 s, and
    the same ratio under gctorture alone.
- **The interim:** that step's per-case timeout is 1200 s
  (`# INTERIM(BW)` in `scripts/rt-ci.py`). The other gctorture step keeps
  600 s.
- **The fix is BW's** (S1): index-only startup with a per-name fetch
  promise, nothing lowered until forced, and M1's eager closures made
  lazy too. BW reverts the bump and reports this case's time.
- **A candidate for H, after X3:** generational torture. GNU R's
  gctorture runs mostly level-0 collections (memory.c's
  `num_old_gens_to_collect`), so it does not trace base's old objects at
  every allocation. robigo-heap is not generational: every collection is
  full (`crates/robigo-heap/src/gc.rs:2273`).

**Gaps, not marked:**
- `options(keep.source = TRUE)` is not implemented, so the fixtures'
  source-referenced closure (generate.r's `f`) is compiled without its
  srcrefs. Owner: M4 (options).
- `dput` writes only to connections 1 and 2.
- `getRegisteredNamespace` of anything but a string or a symbol is "not
  implemented yet".
- `print` of a compiled closure shows `<bytecode: 0x…>` with an address,
  as GNU R does. The cases print `deparse` instead.
- `setEncoding` returns a new vector with `x`'s attributes, where GNU R
  writes `x` (or its duplicate). Only the identity differs.

### perf-rt and the S3 methods table

Measured against main `fb429442`'s binary, on this machine (Ryzen 9 5900X).
- **The first run:** s3 +31.0% instructions, e7-unique +4.0%.
  - Base's `.__S3MethodsTable__.` is now bound, lazily as a promise, as
    GNU R binds it.
  - dispatch.rs's method-lookup memo declined every search whose table
    binding was a promise, so each dispatch ran the full search.
  - The fix on this branch: the memo reads a forced promise's value, skips
    the lookup in a table that surely binds nothing (rtcore's
    `Cx::frame_surely_empty`, HASHPRI 0), and checks the defining
    environment's box instead of re-reading a forced-promise table binding.
  - With it: s3 +0.18%, e7-unique −0.19%, every other row within ±0.1%.
- **`tests/rt/s3/methods-table.R`** puts a method into base's table after
  calls that missed it, then removes it. It matches the oracle; main
  fails at once ("object '.__S3MethodsTable__.' not found").
- **Peak RSS** fails the 2% gate: listgrow 45,188 → 47,764 KiB (+5.7%),
  churn 73,684 → 75,832 KiB (+2.9%). This is the same startup heap growth.
- **The foreman's decision:** wait for BW and rebase onto it. S1 measured
  BW at 8,009 startup objects against main's 15,502. On the rebase:
  - take BW's dispatch.rs, which fixes the same memo problem: a forced
    promise read as its value, absent methods checked by the table's
    version box, the table read once per dispatch, rtcore `Cx::env_version`;
  - keep `frame_surely_empty` and the empty-table skip only if they still
    help measurably on top of BW's;
  - drop the INTERIM(BW) timeout bump if BW has not;
  - re-measure RSS, the GC_MOVE step and every perf-rt row.

### Verification

VERIFICATION
