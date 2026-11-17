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
- grep.c (INTERIM(RX), below);
- `getRegisteredNamespace`, `isRegisteredNamespace` and `environmentName`;
- `[[<-` on a call (cmp.R's `flattenPlace`), below;
- `.GlobalEnv`;
- `eval` of byte code.

### grep.c (INTERIM(RX))

`crates/robigo-rtbuiltins/src/grep.rs` (Q3 (a)):
- **The entry points:** `do_grep` (`grep`, `grepl`) and `do_gsub`'s TRE
  and fixed paths (`sub`, `gsub`) are ports. They cover:
  - the argument checks, the warnings and errors (verbatim);
  - the mode decision (`useBytes`, `use_WC`, `use_UTF8`, native);
  - the translations: `wtransChar2` with its "unable to translate '%s' to
    a wide string" warning (CP1252 for latin1, as glibc's iconv), and
    `trCharUTF82`;
  - NWARN;
  - NA handling;
  - `string_adj`;
  - the empty-match advance and `REG_NOTBOL`;
  - the result encodings (`mkBytesNew`, `markBytesOld` with their
    `_R_REGEX_MARK_*` switches, `mkCharWLenASCII`, `markKnown`);
  - `SHALLOW_DUPLICATE_ATTRIB`.

  `fixed = TRUE` is exact.
- **The matcher** stands where TRE runs. It knows only the part of the
  ERE syntax whose meaning in TRE is plain:
  - literals, and a backslash before punctuation;
  - `.`, `^` and `$`;
  - bracket expressions, read by TRE's rules: a backslash is literal,
    ranges compare code points, and a `-` that two ranges share is an
    error;
  - `*`, `+` and `?`.

  It finds the leftmost-longest match by simulating the NFA from each
  start and keeping the longest end. A negated bracket is complemented
  the way `tre_parse_bracket` does it, including its quirk:
  `[^a-cb-e]` matches `d` and `e` (oracle-checked).
- **Refused** with "robigo: … is not implemented yet (INTERIM(RX))":
  - alternation, groups, back-references and intervals;
  - character classes, equivalence classes and collating symbols;
  - the escapes TRE gives a meaning (letters, digits, `\<`, `\>`);
  - minimal repetition (`*?`), and repetition of an anchor;
  - every ill-formed pattern: those are TRE's errors, so RX's;
  - a negated bracket with two items that start at the same character
    (qsort's order would decide);
  - `ignore.case` and `perl = TRUE`;
  - a latin1 string iconv cannot convert, in the C locale.

RX replaces `ere` and keeps the callers.

**The cases:**
- `tests/rt/strings/grep.R` covers the subset's edges: anchors, bracket
  ranges and TRE's negation, quantifiers on `.`, empty matches in `gsub`,
  the replacement escapes, `fixed`, NA in every argument,
  `invert`/`value` with names, attributes, encodings, unreadable strings,
  and the argument checks.
- A seeded random walk over the subset gave 6,500 cases, and all of them
  matched the oracle: `sub` and `gsub` with each match marked, `grepl`,
  and the result's encoding. The patterns are 0-4 atoms with quantifiers
  and anchors. The strings are ASCII, then UTF-8, with `useBytes` and
  `fixed = TRUE` variants. TRE's parallel matcher picks a match's end
  through its tags (`tre-match-parallel.c:444-480`); the walk is the
  evidence that this end is the longest for the subset. 300 of the cases
  are `tests/rt/strings/grep-generated.R`.

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

### Stop-gap inventory

| marker | what | where | owner |
|---|---|---|---|
| INTERIM(V3) | a BCODESXP body or value runs its source expression, compiled by robigo's own decisions | `rt/compile.rs:163-176`, `rt/eval.rs:117` | V3 |
| INTERIM(L2) | `loadNamespace("compiler")` builds the namespace from the pinned tarball's cmp.R; any other namespace is "not implemented yet"; `path` is the tarball's | `rt/ns.rs:5-11,206-226` | L2 |
| INTERIM(RX) | the regex matcher's subset and refusals (above) | `bi/grep.rs:7-40` | RX |
| INTERIM(L5) | `T` and `F` bound in base; `.ArgsEnv`/`.GenericArgsEnv` empty | `rt/base.rs:174-176,195` | BW (S1), then L5 |

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
- The loader stubs table (`2026-11-16-m1-exit-evidence.md`) loses `grep`
  and `sub`.

### Verification

VERIFICATION
