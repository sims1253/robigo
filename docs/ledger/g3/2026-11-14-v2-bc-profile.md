## 2026-11-14: V2, GNU R's JIT decisions and the `Bc` profile's cmp.R decisions (`robigo-rt`, `robigo-rtcore`, `robigo-heap`)

- What: m1-plan §B V2 (docs/g3.md §6.1, §9.1 bullets 2-3, §9.3). Base:
  `g3/s1-dispatch` at `3201bc0b` (S1, #662, which waits for this to make
  `tests/r/basics/compiled-isvector-missing` pass for real). New in
  `robigo-rt`: `jit.rs` (eval.c: `R_CheckJIT` with `R_JIT_STRATEGY`,
  `MIN_JIT_SCORE`/`LOOP_JIT_SCORE`, `R_ENABLE_JIT`, `R_DISABLE_BYTECODE`;
  `JIT_score`; `R_execClosure`'s JIT step; builtin.c's `do_envirgets`),
  `jitcache.rs` (eval.c: `R_cmpfun`, the JIT cache, `hashfun`,
  `make_cached_cmpenv`, `jit_env_match`, `jit_expr_match`,
  `jit_srcref_match`), `cenv.rs` (cmp.R: `makeCenv`, `frameTypes`,
  `findCenvVar`, `getInlineInfo` at `optimize` 2, `isBaseVar`,
  `findFunDef`, `findLocalsList`/`findLocals1`/`getAssignedVar`,
  `mayCallBrowser`, `is.simpleInternal`/`simpleFormals`/`simpleArgs`,
  `languageFuns`, `safeBaseInternals`). The `Bc` compiler (`compile.rs`)
  takes cmp.R's decisions: `cmpCall`/`tryInline` with frozen inlining and
  `BASEGUARD`, `cmpSimpleInternal`/`inlineSimpleInternalCall`/`checkCall`
  /`cmpDotInternalCall`, `cmpBuiltin`, the `local` handler, the `function`
  handler (`make.functionContext`), `R_compileExpr` for top-level loops.
  The VM: `BaseGuard` (eval.c `DO_BASEGUARD`/`SymbolValue`), `TopLoop`
  (`R_compileAndExecute`), `MkClosure` of compiled code (`MAKECLOSURE`).
  The old runtime's `vm/rjit.rs` was read as the map of GNU R's decisions;
  its mask encoding (decisions recovered at guard failures for eagerly
  compiled code) is not carried over: rt compiles at GNU R's moment, so
  the decisions are cmp.R's taken then, in the code itself.
- Decisions, each where g3.md, the plan or the foreman left a choice:
  - **A profile is per code object, decided as GNU R decides it.** The
    isolate-wide profile (V1's stand-in) is gone except as
    `ROBIGO_RT_PROFILE`'s forced mode for tests. A closure call runs `Bc`
    when GNU R holds the body as byte code (installed, JIT-compiled now or
    before, made by compiled code), else `Eval`; `eval()` and the REPL run
    `Eval`; a top-level loop in the global environment runs `Bc`
    (compiled as it starts, every time, as `R_compileAndExecute` does).
  - **GNU R's JIT state is GNU R's bits plus one mark for "BODY is a
    BCODESXP".** `NOJIT`/`MAYBEJIT` are the closure's gp bits 5 and 6 (as
    C1b's `duplicate` already copied them). M1 makes no BCODESXP (§9.1), so
    a header `extra` bit, `robigo_heap::extra::BYTECODE`, stands for it;
    the compiled code is the `Bc` code object in the closure's 4th word.
    `duplicate` (and `shallow_duplicate`, added here: GNU R's `duplicate1`
    CLOSXP case for both) copies the mark and the code, as GNU R's copy
    shares the BCODESXP; `environment<-` clears both and keeps the gp bits
    (builtin.c:323-325). The mark is never inferred from a closure's
    environment.
  - **Base's closures are installed byte code by provenance.** The base
    loader marks the function literals of each lazy binding's code (those
    not inside another function); `MkClosure` of a marked literal makes a
    compiled closure, compiled at its first call with cmp.R's decisions
    for the base namespace. Installed code inlines base's *shipped*
    definitions of `safeBaseInternals` (the loader's bindings, kept per
    isolate), as GNU R's build compiled it, whatever base binds later.
  - **A promise compiled code makes holds byte code.** GNU R's `MAKEPROM`
    puts a BCODESXP in PRCODE; here the promise (or the delayed argument
    slot, slot meta flag `META_BYTECODE`, carried into the PROMSXP when the
    slot is materialized and into retired arguments' residues) is marked
    `extra::BYTECODE` and keeps its expression. rtcore's forcing calls the
    new hook `eval_bytecode` for a marked one, which compiles the
    expression `Bc` with cmp.R's decisions for the promise's environment:
    a compiled closure's frame shape holds cmp.R's locals of its body (its
    compile frame), so the lazily compiled promise code sees what the
    enclosing compile saw. Defaults are never byte code (cmp.R compiles
    only bodies); `...`'s double promise has a promise as its code.
  - **Nested functions are compiled with their creator** (cmp.R's
    `function` handler): eagerly, into the creator's constant pool, with
    the creator's compile environment plus their own frame; `MkClosure`
    of compiled code makes a compiled closure sharing that code. A body
    that may call `browser()` is not (the closure is made uncompiled).
  - **The JIT cache is GNU R's.** 1024 direct-mapped entries holding body,
    code, top environment and the snapshot's names; reuse only on env
    match, identical body and srcref match; an env mismatch marks NOJIT.
    The hash walks GNU R's structure but hashes symbols by id and longer
    vectors not at all (GNU R uses addresses, which a moving collector
    changes and which are GNU R's own), so which bodies collide and evict
    each other can differ from GNU R's. A hit whose formals are other
    objects (an identical body defined again) reuses the cached code with
    a new stamp, empty match plans and the new formals' frame shape
    (`code::reshape`): the decisions are the cached ones, as GNU R's.
  - **`error()` and `warning()` under byte code report what
    `getCurrentCall` reports** (`R_BCIntActive`,
    `R_getBCInterpreterExpression`): rtcore keeps `bc_active` (set by a
    `Bc` activation, cleared by an `Eval` one, saved and restored around
    each) and the current expression, which the VM sets around each
    builtin and internal it calls in `Bc` code (the wrapper's call for an
    inlined simple internal, the `.Internal` call for one written out). A
    symbol, a closure wrapper's call, a primitive's call or no known
    expression report the innermost function's call, so a loop record or
    complex assignment's record does not hide it. Assignment-call
    inflation is not ported (no expression rt sets is one).
- Not ported, each observable only where stated:
  - **cmp.R's complex assignment** (`cmpComplexAssign`, `cmpSetterCall`,
    `cmpGetterCall`, the setter inline handlers, bcEval's
    `STARTASSIGN`/`SETTER_CALL`/`VECSUBASSIGN` family): compiled code
    still runs the AST's `applydefine`, so its replacement calls hold the
    right-hand side's value where GNU R's hold its expression (or
    `*vtmp*`), and `*tmp*` is bound in the frame where bcEval binds none.
    `tests/rt/jit/setter-calls.R` (the old runtime's
    `vm_setter_jit_oracle` fixture, recorded on 4.6.2) shows it and is not
    ratcheted. A follow-up package.
  - cmp.R's constant folding and its constant pool's `identical()`
    sharing (`tests/r/basics/compiled-zero-constants.R`), and the inline
    handlers whose code differs from the call only in speed (`switch`'s
    arms, `cmpIs`, `cmpPrim1/2`'s ops, `[`/`[[` dispatch): rt calls the
    special or builtin instead. bcEval's loops push a context only when
    cmp.R decides one is needed; rt's always do (no call reports differ,
    by the `getCurrentCall` rule above).
  - Code compiled lazily for a promise made by a compiled top-level loop
    sees the global frame's bindings, not the loop's locals.
- Proof (WSL2 workstation, release, private target): `cargo test
  --release -p robigo-rt -p robigo-rtcore -p robigo-rtbuiltins -p
  robigo-heap` (new `tests/jit.rs`: R_CheckJIT's states, closures and
  promises of compiled code, top-level loops, the cache's reuse across
  formals and its NOJIT, `environment<-`); conformance (`RT`, 4.6.2):
  `tests/r/basics/compiled-isvector-missing` passes (its restored marker
  holds); new `tests/rt/jit/{threshold,frozen,wrappers,cache}.R` recorded
  on 4.6.2 and ratcheted (threshold is E5's `getBindingType` before and
  after the JIT threshold, for top-level, nested, looping, browser and
  `environment<-` cases, top-level loops, copies); `tests/rt/args/
  binding-types.R`'s JIT-threshold lines now match (it still stops at
  `rm`'s `match.call`, B2's). The `tests/r` sweep newly passes nothing
  else. `scripts/rt-ci.py`: see the PR.
