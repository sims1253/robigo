## 2026-11-17: C1-BC: BCODESXP as an object, the byte-code builtins, and cmp.R from source

Package C1-BC (M3, wave 0; `m2m3-plan.md` §B). GNU R's byte-code object
is now a heap type of robigo-rt's, with eval.c's internals over it, and
the compiler package's cmp.R runs from source on robigo-rt.

**The bar:** cmp.R's `cmpfun`, run from source on robigo-rt, gives a
`disassemble()` identical to the oracle's on the bytecode-fidelity
fixtures. It held on this branch before BW (`978fed99`, with the
bound-only base). On BW's wide base the compiler namespace does not load
yet: cmp.R's top level forces every base binding, and 14 cannot be made
yet (below, "Waiting"). The five cases that load the compiler stay in the
PR with their recordings but without markers. What the bar covers:
- `tests/rt/bytecode/fidelity.R` compiles each function the fixtures were
  made from: `tests/r/serialize/bytecode-fixtures/generate.r`,
  `generate-constructed.r`, `compiled-fixtures/source.txt`, and the live
  comparisons in `bytecode_construct.R` and `bytecode_compiled_files.R`.
  The fixture files themselves are serialized; reading them is L1's.
- `cmpfun.R` and `cmpfun-ops.R` add the constructs cmp.R inlines:
  control flow, complex assignment, arithmetic and math, logical
  operators, `missing` and `on.exit`, closures, `...`, `switch` and
  `tryCatch`.
- At `978fed99` all seven bytecode cases gave their 4.6.2 recordings.
  Now `internals.R` and `closures.R`, which do not load the compiler, do
  and are ratcheted.

- **Tree:** branch `g3/c1-bc-bcodesxp`, from main `d25d0586`, with main
  merged in up to `a9c4fda3` (BW #767; RX #764, H7 #765 and L1 #752
  before it).
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

Before BW, the base functions cmp.R needs came in through base-subset.R's
inputs (a new input, `compiler`, and the `bytecode` area); BW now binds
all of base. Each internal they reach is a real port. Loading the
namespace surfaced these, each ported:
- `substitute` in a list (`list_env`);
- replacement functions and `UseMethod` targets, which base-subset.R's
  static walk now counts (kept with BW);
- coerce.c's `as.function.default`;
- grep.c (first an interim matcher; RX's #764 now, below);
- `getRegisteredNamespace`, `isRegisteredNamespace` and `environmentName`;
- `[[<-` on a call (cmp.R's `flattenPlace`), below;
- `eval` of byte code.

`T`, `F` and `.GlobalEnv` were stop-gaps here; BW binds them from the
system profile (`Common.R`'s `.GlobalEnv <- globalenv()`), so they are
gone.

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
- **subscript.c:** `vectorIndex` takes pairlists, so calls and `...` too
  (`isPairList`, subscript.c:354, :366). zzz.R's `as.data.frame.<class>`
  code indexes into a call with `bdy[[c(2L, 3L, 2L, 3L)]]`. Case
  `tests/rt/subset/recursive-index-call.R`.
- **version.c:** `Version` (`R.version`), with `Rversion.h`'s 4.6.2
  values and configure's triplet for linux-gnu (another platform is "not
  implemented yet").
- **platform.c:** `R.home` (sysutils.c's `R_HomeDir`: the environment
  variable `R_HOME`, else "unable to determine R home location") and
  `dir.exists`.
- **paste.c:** `file.path`.
- Case `tests/rt/builtins/version-paths.R` covers `Version`, `dir.exists`
  and `file.path`. `R.home` has no oracle case: robigo-rt's
  `Sys.setenv` is not implemented yet, so a case cannot set `R_HOME`.

**B5a-file ports, for the foreman:**
- builtin.c: `body`, `bodyCode`, `parent.env` (rtbuiltins `builtin.rs`);
  `dput`, `args`, `environmentName` (rt `builtin.rs`).
- platform.c: `R.home` and `dir.exists` (rt `platform.rs`); sysutils.c's
  `R_HomeDir` with it.
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

### Waiting: the compiler namespace on BW's wide base

cmp.R's top level (cmp.R:2499-2508) runs `basevars <- ls('package:base',
all.names = TRUE); types <- sapply(basevars, function(n)
typeof(get(n)))`, so loading the compiler forces every one of base's 1,407
bindings. On BW's base, 30 raised; GNU R raises none.
- **Fixed here (16):**
  - the 12 `as.data.frame.<class>` (complex, Date, difftime, factor,
    integer, logical, noquote, numeric, numeric_version, ordered, POSIXct,
    raw): vectorIndex's pairlists;
  - `R.version`, `R.version.string`, `version`: `Version`;
  - `.Library.site`: `dir.exists`.
- **Waiting (14):**
  - **getLoadedDLLs and base's native-symbol lookup** (B5a, Rdynload.c):
    `.C_R_addTaskCallback`, `.C_R_getTaskCallbackNames`,
    `.C_R_removeTaskCallback`, `.F_dqrcf`, `.F_dqrdc2`, `.F_dqrqty`,
    `.F_dqrqy`, `.F_dqrrsd`, `.F_dqrxb`, `.F_dtrco`;
  - **strptime** (B5a, datetime.c): `.leap.seconds`;
  - **L2a's attach**: `.AutoloadEnv`, which BW does not run
    (Common.R's `attach(NULL, name = "Autoloads")`);
  - **robigo-rt's R home**: `.Library` is `file.path(R.home(),
    "library")`. `R.home()` reads `R_HOME`, which GNU R's front end
    always sets and robigo-rt's does not set;
  - **Rprofile.unix's translations block**: `.popath` needs
    `.libPaths()` (so `.Library`), `file.exists`, `dirname` and
    `bindtextdomain`.
- **Which case hits what:** `cmpfun.R`, `cmpfun-ops.R`, `compile.R`
  and `fidelity.R` call `compiler::`, and `jit.R`'s `enableJIT(3)` loads
  the compiler. So each runs the scan, and each waits on all 14: the
  first binding that raises (`.AutoloadEnv`) stops the scan. What a case
  reaches after the scan cannot be known until the scan passes.
- **The ratchet:** whoever lands the last of these runs the five cases
  and ratchets them.

### Stop-gap inventory

| marker | what | where | owner |
|---|---|---|---|
| INTERIM(V3) | a BCODESXP body or value runs its source expression, compiled by robigo's own decisions | `rt/compile.rs:163-176`, `rt/eval.rs:117` | V3 |
| INTERIM(L2) | `loadNamespace("compiler")` builds the namespace from the pinned tarball's cmp.R; any other namespace is "not implemented yet"; `path` is the tarball's | `rt/ns.rs:5-11,206-226` | L2 |

**The base heap and the moving-collector step, before BW.** The bound-only
loader bound each base closure as a promise of its parsed right-hand
side, and cmp.R's static walk grew the bound to 415 closures, 226
internals and 9 variables, against 188 closures on main `fb429442`:
- the startup heap grew to 51.4k cells, against 15.6k;
- `tests/r/jit/loop-closed-form-super.R` under GC_MOVE+gctorture took
  598.5 s, against 154 s, with identical output;
- peak RSS rose 5.7% (listgrow) and 2.9% (churn).

The foreman's decisions: an INTERIM(BW) 1200 s timeout for that step,
then waiting for BW and rebasing onto it. BW's index-only startup (8,009
objects against 15,502) removed the growth. The timeout is back to
600 s, and the numbers are below.

A candidate for H, after X3: generational torture. GNU R's gctorture runs
mostly level-0 collections (memory.c's `num_old_gens_to_collect`), so it
does not trace base's old objects at every allocation. robigo-heap is not
generational: every collection is full (`crates/robigo-heap/src/gc.rs:2273`).

**Gaps, not marked:**
- `options(keep.source = TRUE)` is not implemented, so the fixtures'
  source-referenced closure (generate.r's `f`) is compiled without its
  srcrefs. Owner: M4 (options).
- `dput` writes only to connections 1 and 2.
- `getRegisteredNamespace` of anything but a string or a symbol is "not
  implemented yet" (L2a's coerces to a symbol, and replaces it).
- `print` of a compiled closure shows `<bytecode: 0x…>` with an address,
  as GNU R does. The cases print `deparse` instead.
- `setEncoding` returns a new vector with `x`'s attributes, where GNU R
  writes `x` (or its duplicate). Only the identity differs.

### perf-rt and the S3 methods table

- **Before BW**, against main `fb429442`:
  - The first run showed s3 at +31.0% instructions and e7-unique at
    +4.0%. Base's `.__S3MethodsTable__.` had entered the bound as a lazy
    promise, and dispatch.rs's method-lookup memo declined every search
    whose table binding was a promise.
  - This branch's fix brought s3 to +0.18% and e7-unique to −0.19%. It
    read a forced promise as its value, checked the defining environment's
    box instead of re-reading that binding, and skipped the lookup in a
    table that surely binds nothing (`Cx::frame_surely_empty`).
  - BW's dispatch.rs fixes the same problem: a forced promise read as its
    value, an absent method checked by the table's version box, the table
    read once per dispatch. This branch takes BW's.
  - The empty-table skip, measured on top of BW's for the only check that
    still looks the method up (found after the table): s3 −0.06%,
    e7-unique −0.01%. Other rows move as much as ±0.03% between runs, so
    that is not measurable. The skip and `Cx::frame_surely_empty` are
    dropped.
- **`tests/rt/s3/methods-table.R`** puts a method into base's table after
  calls that missed it, then removes it. It matches the oracle; main
  `fb429442` failed it at once ("object '.__S3MethodsTable__.' not
  found").
- **Now, against main `a9c4fda3`** (Ryzen 9 5900X, callgrind
  instructions per iteration): 0 failures.

  | row | change |
  |---|---:|
  | plain | +0.02% |
  | defaults | −0.01% |
  | named | +0.06% |
  | env-fn | −0.01% |
  | superassign | +0.01% |
  | s3 | +0.19% |
  | fib | −0.00% |
  | binarytrees | −0.00% |
  | e7-paste | −0.00% |
  | e7-match | −0.06% |
  | e7-unique | +0.02% |

  Peak RSS: binarytrees 26,460 against 26,592 KiB, listgrow 46,068
  against 45,848 (+0.5%), churn 74,332 against 73,820 (+0.7%).

### Verification

VERIFICATION
