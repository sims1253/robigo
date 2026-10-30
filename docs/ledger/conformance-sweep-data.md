# conformance-sweep-data

## 2026-10-30 (round 10): compiler construction internals

- Claim builtins/bytecode.rs and its registration, plus construction fixtures.
  GNU R 4.5.0 defines these functions in eval.c, not codegen.c.
- Port mkCode, bcClose, bcVersion, disassemble, is.builtin.internal,
  growconst and getconst. Reuse portable instruction validation through a
  visibility-only serialize.rs change; construction builds cell records in
  its owned module.
- putconst remains blocked on core ownership: eval.c:do_putconst mutates
  VECSXP aliases with SET_VECTOR_ELT, which Rc<Vector<Value>> cannot express
  safely. Do not clone the buffer and claim success. Next step: core supplies
  shared list mutation, then port constant deduplication with identical flags
  16 and run compiler::cmpfun layout comparisons.
- The construction script starts at bcVersion (0/3 tiers); oracle fixtures
  were recorded with robigo-oracle:4.5.0. GNU R accepts, executes and
  byte-identically reserializes all four exported RDS artifacts.
- Extended probes preserve attributed/shared language constants, discard
  instruction-vector attributes, validate arity, and use only the first
  atomic getconst count element (coerce.c:asInteger). Oracle arity probes
  exposed check_arity's GNU R spelling: util.c:Rf_checkArityCall uses
  .Internal(name), without quotes, for internals.
- `load.from.file`/`save.to.file` remain startup-owned I/O; putconst remains
  the shared-mutation dependency. No R-level compiler replacement added.
- Verified on the pre-merge base: all 254 scripts in all three tiers
  (759/762 -> 762/762, the only other mismatch being main's unlisted
  compiled-isvector probe), data 8,410/8,426 with identical outcomes,
  startup 44,182,690 -> 44,182,091 (-0.001%), and 44 runtime unit tests plus
  two lazy-load integrations (the two documented host libm oracle tests are
  excluded locally only).

## 2026-10-30 (round 11): rescue #105 onto opt-in bytecode fidelity

- Take over #105 at e145aba0 after it conflicted with main. The original
  merge conflict was substantive: round 10 expected the eager Bytecode
  representation from round 9, but #100 merged round 9 behind the
  `bytecode-fidelity` feature with a lazy encoded instruction/constant
  representation and restored the default import layout.
- First merged main 0bee8896, keeping #104's LANGSXP cell registry and the
  48-byte PNode reader beside the bytecode writer, and taught the new
  reachability graph about Value::Bytecode. Both substitute cell tests and
  all three bytecode fixtures pass. That merge is in the branch history;
  main then moved to #100/#116/#117.
- Rebased the round-10 construction work onto merged main (2ac8f810):
  adapted mkCode to build Decoded instructions plus a replay cell stream
  (BytecodeSpan/BytecodeWords), import consts/rep_values lazily, and use
  `Bytecode::code()/constants()`. `bcClose` still rejects attributed
  formals. `getconst`/`is.builtin.internal` remain registered in default
  builds (GNU R base defines them) and are owned by the construction module
  under the feature, so there is exactly one definition per build.
- Default-build performance is back to main's layout because bytecode
  fidelity is opt-in. Against a freshly built 2ac8f810: 0 faster, 0 slower
  above 0.5%, 0 broken over 73 measured benchmarks. Targeted pre-rebase
  experiments (out-of-line bytecode attribute copy and eval arm) reduced
  nbody_2/nbody_3 but were superseded by main's `bytecode_attrs_slot`; the
  remaining nbody_naive_2 delta was heap-layout sensitivity of the eager
  representation and disappears with the opt-in default.
- Verification on df28a332: default conformance 282/283, the only failure
  main's unlisted compiled-isvector-context probe; feature conformance
  3/3 bytecode scripts with stdout/stderr/status exact in interpreter, VM
  and JIT; six bytecode_lazy tests; 62 runtime unit + 2 C API with only the
  two documented host-libm oracle tests excluded locally; data 8,410/8,426
  with the known host order/add626 collation case. Both merged ratchet
  lists are gone; all branch entries have markers, including
  bytecode_construct.R.
- `putconst` remains blocked on core shared-list mutation: the compiler
  fills one VECSXP alias with SET_VECTOR_ELT and reads it back without an
  assignment, which Rc<Vector<Value>> cannot express in place. Next: core
  supplies shared list mutation, then port constant deduplication with
  identical flags 16, add compiler::cmpfun layout cases, and re-enable
  those probes in the feature CI job.

## 2026-10-30 (round 9): final append-conflict rebase

- Rebased onto 805e5fb after newer main ratchet entries caused an append
  conflict in tests/r/passing.txt. Keep both sides. No source conflict in
  the bytecode implementation; serialize.rs, value.rs and funs.rs are
  byte-for-byte unchanged from the fully verified code batch.
- Rebuilt the combined head and reran standard conformance, VM guards and
  compiled_matches_interpreted: all three checks pass. Formatting and
  source diff checks pass; the worktree is clean after this ledger update.
- Latest exact-base startup: 44,167,789 -> 44,175,555 instructions
  (+0.018%). Earlier base-specific numbers below remain experiment history.
- Compiler construction/introspection internals and the perf-vm internal
  call handoff remain the concrete next steps. No vm.rs/compile.rs edits.


## 2026-10-30 (round 9): delivery candidate

- Keep direct special/reference-cell reads and immediate NULL, symbol,
  integer/logical scalar records. Against exact d81d29c: startup
  44,157,996 -> 44,177,552 instructions (+0.044%).
- Both new oracle scripts improve 0/6 -> 6/6 across interpreter, VM and
  JIT. All 252 scripts pass in every tier (756/756). Add the bytecode
  promise script to the ratchet. Fixtures compare serialize/saveRDS bytes
  for XDR v2/v3, ASCII import, nested code and shared cells, version 9/13
  normalization/fallback, attributes, scalar constants, and promise PCODE
  before/after forcing. eval/bodyCode, body replacement and equality match.
- Exact base and final candidate data both hold 8,409/8,426 with identical
  per-case outcomes. Absorb 0b6df291 and daaeabd5, two verified
  main-inherited setter-context gains. The existing host order/add626
  collation mismatch remains unchanged.
- Final recursive benchmarks: binarytrees 1,174,327,731 -> 1,174,212,332;
  fib 880,117,572 -> 880,024,448. perf.py compare reports zero slowdowns
  over 0.5%, broken results or newly incorrect programs.
- Final local checks pass: 44 runtime unit, two C API, seven CLI unit,
  three standard conformance/VM checks and two lazy-load tests. Only the
  two documented host libc numeric oracle tests were excluded locally;
  Debian CI runs them unmodified. cargo fmt --check and diff checks pass.
- Code head ca8ca47 passed CI build/unit/parser/conformance and every
  corpus job. Downloaded all ten corpus result artifacts and compared them
  against exact d81d29c artifacts: identical pass/fail outcomes in every
  corpus, zero regressions. No patch-specific corpus gains to absorb;
  only the two independently verified incoming setter gains were added.
  Full CI performance remains pending at ledger finalization.
- Follow-up: port GNU compiler construction/introspection internals
  (bcVersion, mkCode, bcClose, disassemble) and generate shared-cell records
  for newly constructed bytecode. This batch retains immutable GNU R
  bytecode imported from serialized streams and executes its source.
  No compile.rs or vm.rs changes. ASCII output remains startup work.
- Core handoff: bytecode owns code, expression, constants, cell items and
  attributes. Include those edges when extending runtime reachability for
  native finalization; do not classify BCODESXP as an atomic owner.


## 2026-10-30 (round 9): immediate cell records on d81d29c

- Rebased checks pass: 251 scripts across all tiers (753/753), standard
  conformance/VM guards, 44 runtime unit tests, two C API and two lazy-load
  checks. The two known host libc numeric tests remain excluded locally.
- Exact base and candidate data slices both score 8,409/8,426 with identical
  case outcomes. The two gains over f5dae40 belong to incoming main work.
- Base startup is 44,157,996, candidate 44,522,847 (+0.826%). Encode
  NULL, symbols and unadorned integer/logical scalars directly in the cell
  operation stream. All fixture bytes still match; startup 44,468,390,
  still above the target. Next inline ReadItem's special/reference cases
  into bc_item, retaining generic attribute handling for other objects.
- Added an oracle fixture for bytecode promise expressions and serialization
  before and after forcing. It already passes through Interp::eval; reject
  a speculative compile.rs fallback change because the probe shows no gain.
  compile.rs and vm.rs remain unchanged relative to the PR base.


## 2026-10-30 (round 9): integration rebase onto d81d29c

- Rebased over #94, #101, #92 and #102. Preserve main's SmallVec inline
  ReadBCLang spine and moved-tail optimization; remove the now-redundant
  scratch pool, retain exact item capacity and shared head registration.
  Keep every incoming and new conformance ratchet entry.
- Rebuild in the private target and remeasure against this base before
  delivery. The earlier f5dae40 measurements remain historical results.


## 2026-10-30 (round 9): verified persistence on f5dae40

- Keep bytecode persistence with moved CARs, shared head registration and
  pooled scratch. Startup: 44,371,718 -> 44,571,940 instructions (+0.451%).
  Binarytrees: 1,174,133,259 -> 1,174,062,320; fib: 879,739,190 ->
  879,735,563. perf.py compare reports zero slowdowns above 0.5%.
- All 244 conformance scripts match the oracle in interpreter, VM and JIT
  (732/732). New bytecode fixture improves 0/3 -> 3/3; add its passing
  entry. Runtime and C API crate checks pass with the two documented host
  libc numeric oracle tests excluded; Debian CI runs those unmodified.
- Expanded data stays 8,407/8,426, with identical per-case outcomes. The
  existing order/add626 host collation mismatch remains the sole listed
  failure on both binaries. No new FastR pass to absorb from this slice.
- Main subsequently merged setter, C API writeback and startup work.
  Rebase and verify their combination before final delivery.


## 2026-10-30 (round 9): scratch reuse and materialization costs

- Fully corrected compact candidate passes all 244 scripts in all tiers,
  732/732 oracle checks. CI build/unit/parser/conformance also passes.
  Exact CI startup: 45,445,824; private host build: 45,433,751.
- Reuse ReadBCLang scratch spines across constants, inline recording helpers
  and reserve operation/object streams from instruction length. Host startup
  falls to 44,813,473 (-620,278 versus compact host), but remains about 1%
  above the 44,371,718 rebased CI baseline. Continue optimizing.
- Next remove cloned CARs during spine materialization, use exact tail
  capacity, and register the constructed head itself for BCREPDEF instead
  of making a second head. This also matches GNU R's head identity.
- Common serialization slice excluding the existing long-vector host
  timeout: 317/321, no regressions. Its regex excludes four differently
  named cases as well; compare per-case outcomes against the saved baseline.
- The interruption stopped the shared build before binary delivery.
  Rebuilt project crates in a private reflink copy of the target directory
  to avoid shared metadata contamination and repeated build-lock queues.

## 2026-10-30 (round 9): first persistence candidate, rejected storage cost

- Rebased f5dae40 baseline startup: 44,371,718 instructions. First
  candidate: 45,380,739 (+2.274%). Reject this recording representation.
  Recursive benchmark counts improve slightly: binarytrees 1,174,133,259
  -> 1,174,069,667; fib 879,739,190 -> 879,711,752.
- XDR v2/v3 saveRDS/serialize bytes, ASCII import, standalone bytecode,
  bodyCode, body replacement and supported-version normalization match.
  Future-version probe exposed serialize.c:ReadItem's final conversion
  to R_BytecodeExpr. Port that conversion, including closure JIT metadata.
- Debian CI caught the new eval(bytecode) probe returning the bytecode
  object in do_eval's self-evaluating branch, causing raw pointer differences
  across tiers. Port eval.c:do_eval's BCODESXP branch to use Interp::eval.
  This is a minimal builtins/envir.rs type-integration change, not vm.rs.
- Callgrind identifies bc_item's unnecessary take/restore of the recording
  vector and destructor visits to integer records. Replace tagged record
  objects with packed integer operations plus a separate object vector.
  Nested read_bc already isolates its recording state; ordinary read_item
  does not need an additional state save. Candidate measurements pending.
- Broad fixed d2ce510 data baseline is 8,407/8,426. Only the documented
  order/add626 host collation case fails a recorded passing entry.

## 2026-10-30 (round 9): bytecode persistence and ownership

- Start from main d2ce510. Round 8 PR #96 remains open and is not in
  this baseline. Its parser reduction changes remain separate.
- VM handoff to perf-vm (#94): lapply/245e62b143e432a3 still needs
  GNU compiler GETINTLBUILTIN.OP/CALLBUILTIN.OP for is.vector before
  forcing X. GNU R bypasses the R wrapper, so its error lacks that
  closure's traceback frame. Implement in the internal-call compilation
  path; do not filter traceback names globally. No vm.rs edits here.
- Ownership audit of remaining data failures: factor/Calls contexts,
  replacement capture order and unused-argument diagnostics belong to
  evaluator/lang; match.arg list coercion to values; raw-address tracemem
  cases need oracle-owner investigation; order/add626 is host collation.
  No additional unclaimed data-filter fix was identified in the ledgers.
- Rebased onto main f5dae40 after #96 and #97 merged, keeping both
  ledger histories and all incoming ratchet entries.
- Exact d2ce510 CI artifact baseline: three-tier oracle checks 726/729;
  the new bytecode fixture fails in all tiers. serialize/unserialize
  slice 320/326, with the existing long-vector 6260e145 host timeout.
  Startup is 44,364,038 callgrind instructions. Broad data run pending.
- Candidate retains portable code, all constants and repeated-cell records;
  exposes BCODESXP/bodyCode and preserves the executable source body.
  R_bcEncode/R_bcDecode normalization uses R 4.5.0 opcode argument counts.
  ASCII input also needed InString's zero-length guard. ASCII output
  remains an existing unimplemented feature, outside this change.
- Shared edits: closure constructors, Value/SEXPTYPE exhaustive matches,
  bytecode printing/deparsing/evaluation, and C API bytecode proxies.
  No vm.rs or execution-bytecode changes. Candidate checks pending the
  shared build queue. Oracle fixtures generated in robigo-oracle:4.5.0 retain source
  references, nested bytecode and versions 2 and 3.


## 2026-10-30 (round 8): syntax reduction records

- Baseline 4754027. Requested branch name was already attached to another
  worktree; use track/conformance-sweep-data-8-records in this checkout.
- Parser corpus baseline: 17/28, all eleven parse-data cases fail. New
  oracle-recorded runtime fixture: 0/3 across interpreter, VM and JIT.
- Port gram.y:YYLLOC_DEFAULT, token_, record_, recordParents,
  modif_token and finalizeData through an optional syntax-side LR recorder.
  Tables are extracted from GNU R 4.5.0's generated gram.c; the generator
  is committed alongside them. The ordinary AST parser remains unchanged.
- Initial raw-matrix differential check: all 47 Debian oracle programs
  match exact row order, numeric token codes, IDs, parent links, text and
  locations. Include assignment direction, nested grammar-only reductions,
  formals, calls, missing arguments, comments, tabs, UTF-8 and pipe rewrites.
  Two line-directive probes were added after that check. Runtime integration
  attaches the matrix to the source environment for base getParseData.
- Fixed-binary broad baseline is 8,407/8,426. Its sole recorded-pass
  failure is the existing order/add626 host collation mismatch.
- BCODESXP handoff (core/startup): read serialize.c:ReadBC1/WriteBC1,
  ReadBCConsts and ReadBCLang together. Preserve code, every constant,
  nested BCODESXP constants and the shared BCREPDEF/BCREPREF language
  graph; expose the source expression separately for body()/evaluation.
  serialize.rs:read_bc currently discards code and all but constant zero.
  Value has no BCODESXP case and the writer cannot emit one. Port
  eval.c:do_mkcode/do_bcclose only after that representation exists;
  bcVersion alone does not unblock compiler::cmpfun. Read core #84's
  PR body: strings must preserve raw bytes and encoding marks through
  serialization; never serialize the UTF-8 display view instead. #84
  remained open during this batch, so no oracle expectations were changed
  for its unmerged representation.
- perf-vm handoff: lapply/245e62b needs GETINTLBUILTIN/CALLBUILTIN for
  is.vector(X, "any") before forcing missing X. A builtin-side change
  cannot remove the extra R is.vector closure frame. Compile and execute
  that internal call in vm.rs, then check the error and traceback against
  the existing oracle in all tiers. No vm.rs edit in this batch.
- First runtime candidate: parser corpus 17/28 -> 28/28 (+11), zero
  regressions. Added all eleven verified passes to the FastR ratchet.
  Runtime oracle fixture improves 0/3 -> 3/3 across interpreter, VM and JIT.
- Expanded raw oracle now matches all 59 programs, also with the final
  newline omitted. This caught and corrected empty-production locations
  for {;} and {;;}. Malformed #line comments have empty recorded text,
  matching SkipComment. A final runtime rebuild follows these corrections.
- Syntax crate passes all nine unit tests, the raw-data differential test
  and the curated parse-tree oracle. The optional external corpus oracle
  remains ignored by default.
- Final fixed-binary parser/subset comparison: 169/192 -> 180/192,
  exactly eleven gains and zero regressions. Broad data holds at
  8,407/8,426, with identical per-case outcomes and difference categories.
  The only listed failure on both binaries is the known order/add626
  host collation case. Candidate corpora use a private TMPDIR to avoid
  collisions with other worktrees' runs.
- Strict stdout/stderr/status comparison in all three tiers improves
  714/720 -> 717/720. All 239 passing scripts pass in every tier; the
  unlisted closure_source_print_utf8 probe still fails in all tiers on
  both binaries pending core #84. Added parse_data.R to the fixture ratchet.
- Final startup: 44,821,179 -> 44,819,937 callgrind instructions (-0.003%).
  All Debian CI jobs on code commit 43a05b8 pass, including every corpus
  shard and the performance gate. CI runs cargo test --release --workspace:
  all 43 runtime unit tests, both lazy-load integrations, nine syntax unit
  tests, the raw-data differential and curated parser oracle pass without
  host exclusions. Local cargo fmt --check and source diff checks pass.
- Build failure was rejected as a measurement: the shared target reused
  another checkout's syntax artifact and reported no parse_data module.
  Seeded a private .robigo/runtime-target, forced this checkout's syntax
  and runtime to rebuild, and verified its fixed candidate binary above.
  No build was killed. Initial LR replay also needed xxexprlist's
  EatLines reset before the empty-brace expression matched the oracle.
- Further metadata work: port #line source aliases in SourceRefs together
  with PS_SRCFILE/PS_ORIGINAL transfer; the new recorder already keeps
  physical parsed-line positions and LINE_DIRECTIVE records. Audit
  MAXELTSIZE text truncation for very long strings/comments against
  StringValue/RawStringValue/SkipComment before extending that coverage.
  No whole-program AST reconstruction or R-level getParseData override.


## 2026-10-30 (round 7): closure source metadata and ownership audit

- Baseline main 29cac51, branch track/conformance-sweep-data-7.
  Two new Debian 4.5.0-3 oracle fixtures fail in each execution tier
  (0/6). Startup baseline is 44,820,180 callgrind instructions.
- Port eval.c:do_function's transfer of CADDR(args) to the closure's
  srcref attribute. Preserve the source environment by reference, body
  attributes independently, nested functions and defaults. Serialization
  versions 2 and 3 already preserve these attributes and shared srcfiles;
  no serialize.rs change is needed.
- Port print.c:PrintLanguage for closures with integer srcrefs and
  print.c:do_printdefault's useSource validation. Extract the source via
  the base as.character generic, retaining original comments and spacing.
  Shared edits are confined to builtins/lang.rs and print.rs; no vm.rs,
  value.rs, serialization or finalizer edits.
- Kept: both fixtures improve **0/6 -> 6/6** across interpreter,
  VM and JIT. They cover nested/default closures, environment changes,
  body/formal replacement, comments, serialization versions 2 and 3,
  original spacing and source printing inside a list. Added both passes
  to tests/r/passing.txt.
- Startup **44,820,180 -> 44,817,780** instructions (-0.005%).
  Fixed-binary broad sweep holds **8,407/8,426 -> 8,407/8,426**,
  with identical per-case outcomes. The sole recorded-pass failure on
  both binaries is order/add626's known host collation mismatch.
  Parser/subset holds **169/192 -> 169/192**, identical per-case outcomes.
- Full oracle comparison improves **708/717 -> 714/717** across the
  interpreter, VM and JIT. All 236 original scripts pass in every tier,
  the two new fixes gain six checks, and the separate UTF-8 probe fails
  in all tiers on both binaries. No tier differences or regressions.
- Runtime checks pass: 36 unit tests and two lazy-load integration tests,
  with the two documented host libc oracle tests excluded. Debian CI
  runs those tests unmodified. cargo fmt --check and source diff checks
  pass. Oracle recordings retain GNU R's trailing whitespace verbatim.
  Final shared ratchet diff only appends the two verified fixture paths.
- Rejected folding the UTF-8 print probe into the passing fixture: base
  as.character.srcref sets Encoding(lines) to bytes, and core's current
  implementation rejects non-ASCII bytes marking. Retained the exact
  Debian oracle probe separately as closure_source_print_utf8.R, without
  a passing entry. It fails before and after this batch. Follow up with
  core #84's byte-window work; do not replace base source extraction with
  a guessed string slice. Tabs and UTF-8 source references themselves
  remain covered by round 6's passing source_refs.R.
- Rejected an extra global as.character.srcref override probe: GNU R's
  base-environment call does not invoke that global override. The probe
  produced no distinct behavior, so its proposed buffering change and
  redundant fixture extension were removed before the next build.
- Parser audit: parser.md leaves eleven parse-data cases unclaimed.
  gram.y:token_, record_, recordParents, modif_token and finalizeData
  need token IDs, grammar reductions, token reclassification and comment
  parents. The current arena AST has discarded those reductions and the
  lexer discards comments. A correct next slice must add an optional
  syntax-side recording mode before attaching runtime parseData. Do not
  reconstruct the grammar history from the runtime AST or synthesize a
  getParseData replacement. This batch keeps syntax ownership unchanged.
- VM coordination for perf-vm: lapply/245e62b still needs GNU compiler's
  GETINTLBUILTIN/CALLBUILTIN behavior for is.vector before X is forced.
  The available traceback frames cannot fix that by filtering names.
  Next step belongs to the internal-call compilation path in vm.rs.
- Bytecode coordination for startup/core/perf-vm: serialize.rs:read_bc
  explicitly discards the instruction vector and all but constant zero.
  Value still has no BCODESXP variant. Faithful mkCode/bcClose and compiler
  round trips need that representation and serializer support together.
  No version-only builtin or guessed bytecode representation was added.

## 2026-10-29 (round 6): parsed source-reference attributes

- Port gram.y:makeSrcref, attachSrcrefs, xxexprlist and xxdefun in
  builtins/parsing.rs. Environment srcfiles now enable source references
  on expression vectors and braces, with a source reference as the third
  function argument. Preserve byte positions, UTF-8 display columns, tab
  stops, source-file identity and bounded parse ranges. The syntax change
  only exposes the byte offset consumed by program_n.
- Focused subset case b4bfbcb9: **0/1 -> 1/1**; recorded the gain.
  New Debian GNU R 4.5.0-3 fixture: **0/1 -> 1/1**, passing interpreter,
  VM and JIT. All existing conformance scripts plus the fixture pass in
  all tiers: **702/702** checks. Added the fixture to the ratchet.
- Parser slice holds at **168/191 -> 168/191**, zero regressions.
  Startup callgrind: **45,025,118 -> 45,032,920** instructions (+0.017%).
  Kept. Fixed-binary data comparison: **8,406/8,426 -> 8,407/8,426**,
  exactly the subset gain and zero per-case regressions. The existing
  order/add626 host collation failure remains on both binaries. The
  baseline also reports the newly ratcheted subset case, as expected.
- Rejected the first broad sweep as a before/after measurement: its
  mutable binary path changed when the candidate build completed. Re-run
  both revisions from fixed binary copies in .robigo/sweep6.
- Extra syntax probes exposed gram.y:StringValue/RawStringValue's
  double increment of byte positions for literal multibyte characters.
  Backtick names use the same path; ordinary identifiers do not. Preserve
  that GNU R quirk and extend the Docker-recorded fixture with quoted,
  raw-string and backtick UTF-8 cases. The initial candidate failed this
  extension; final corrected candidate verification follows below.
- Final quoted-UTF-8 correction: extended fixture passes in all three
  tiers, and all extra syntax probes match the Debian oracle. Final
  parser-plus-subset slice is **169/192**, zero regressions. Startup is
  **45,037,852** instructions against **45,025,118** (+0.028%). Kept.
  The correction changes only quoted source offsets; the fixed-binary
  broad comparison above covers the original ASCII subset gain.
- Final local crate rerun passes: 36 runtime unit tests, two runtime
  integration tests, nine syntax unit tests, and the curated parser
  oracle. The optional corpus oracle remains ignored by default. Only
  the two known host libc tests below were excluded. cargo fmt --check
  and git diff --check pass. Initial PR revision passed Debian CI's
  build/unit/conformance job, performance gate and all non-FastR corpus
  jobs; the final quoted-offset correction is undergoing CI.
- Initial runtime unit checks: 36 pass, two pre-existing host libc
  mismatches in complex c_library_oracle (sqrt, one ULP) and
  c_library_trig_oracle (csin, one ULP). Run the remaining crate checks
  with those two excluded locally; Debian CI checks both unmodified.
- Still open: gram.y parse-data records (RecordParents, finalizeData) for
  the eleven getParseData cases; #line aliases and source metadata transfer
  onto evaluated closures. The current slice covers parsed objects.
- Evaluator follow-up: lapply/245e62b143e432a3 still adds an erroneous
  `Calls: lapply -> is.vector` after missing-X forcing. Trace the signal
  point against eval.c before changing traceback filtering. Oracle
  compiler::disassemble(lapply) shows GETINTLBUILTIN.OP/CALLBUILTIN.OP
  for is.vector(X, "any"): GNU R bypasses the R is.vector closure.
  Robigo executes that closure, adding its frame before forcing X. This
  case needs the VM owner's equivalent internal-call optimization, not
  global suppression of builtin names in traceback. R_ConciseTraceback
  explicitly includes CTXT_BUILTIN as well as function contexts.
- Compiler investigation: base/identical still fails at bcVersion().
  eval.c:do_bcversion returns R_bcVersion, but cmpfun next needs
  mkCode/bcClose and an actual BCODESXP representation, absent from Value.
  A version-only patch does not unblock the example. Coordinate that
  representation with core and VM owners; no guessed bytecode substitute.

## 2026-10-29 (round 6): source-reference ownership

- Claim the finished parser track's source-reference work in runtime
  builtins/parsing.rs and a minimal syntax consumed-offset accessor.
  Source ranges on parsed expressions, braces and function literals are
  the first slice; parse-data records remain a separate follow-up.
- Baseline is main dbf6754. No edits to vm.rs, serialization, S4,
  finalizers or the core object model.

## 2026-10-29 (round 5): isolate loaded-bytecode metadata

- Full CI artifact comparison against main caught two regressions outside
  the data filter: options/4d3e4548 and stats/family gained a spurious
  bytecode line on strOptions()$formatNum and Gamma()$linkinv. Both
  callbacks are interpreted in GNU R. Rejected initializing loaded bodies
  as COMPILED: that also changed jit_inherit's treatment of their children.
- Store loaded BCODESXP identity in a spare low-byte flag of JitState,
  independently of its execution state. State transitions preserve that
  flag, and state comparisons mask it out. Loaded namespace functions
  retain the previous IGNORE inheritance behavior. No layout or vm.rs edit.
- Extended the Docker-oracle closure fixture with both callbacks and a
  loaded match closure printed before/after its first call. The previous
  candidate differs by exactly the two spurious callback lines; the new
  candidate passes the extended fixture in all tiers. All four fixtures
  hold (12/12), and the options regression returns to passing (1/1).
- Final metadata-only startup: **44,817,056 instructions**, compared with
  original main's 44,819,159 (-0.005%, effectively unchanged). This also
  avoids the INHERITED variant's 74 unnecessary local scans. Kept.
- Re-running the full data slice, blocker cases, and crate checks on this
  final representation. Earlier gains/three-tier counts below describe the
  preceding candidate; CI's full before/after comparison is the last gate.

## 2026-10-29 (round 5): final rebased sweep and next steps

- Final default VM/JIT data slice on e25e6cc: **8,404/8,426 ->
  8,406/8,426**. Only the two split diagnostics change outcome. The
  sole ratchet failure, order/add626, is unchanged from the saved main
  binary and comes from this host's unavailable en_US.utf8 collation.
- All 139 former blocker cases across interpreter, VM and JIT improve
  **131/139 -> 135/139** in each tier, with no regressions. Four
  TestConditionHandling parse-context cases also gained; absorbed all
  four. The five known bytecode blockers hold, including Sys.getenv
  with 0, 1, 10, 40, 80 and 160 synthetic variables in each tier (18/18).
- Remaining mget cases in the blocker slice are unrelated to printing:
  3aa1ccfb (ifnotfound environment coercion), c98ec841 (vector modes),
  cb3e95e3 (ifnotfound length), e945049d (numeric environment validation).
  Next owner should port envir.c:do_mget argument validation/coercion.
- Remaining data work: parser source metadata (subset/b4bfbcb9 needs
  keep.source attributes and parse-data records); evaluator traceback
  contexts (factor/lapply), replacement order and rhs display, and unused
  argument value printing; values' as.character-list errors. Tracemem's
  four raw-address recordings need oracle-owner treatment; do not crash
  deliberately to match GNU R's status -11 recordings.
- base/identical still needs compiler .Internal(bcVersion()) and the
  remaining compiler internals before its cmpfun example can execute.
- This is a coherent batch: loaded/JIT closure printing, stable Sys.getenv
  recording, six FastR context-diagnostic gains and two help-example gains.
  Final crate and full interpreter/VM slice checks run after this commit.

## 2026-10-29 (round 5): avoid rescanning loaded bytecode bodies

- Measured startup with R_DEFAULT_PACKAGES=NULL and callgrind on
  `robigo --vanilla -e 1`. Initial bytecode candidate: 45,259,491
  instructions versus original main's 44,819,159 (+0.98%). Callgrind
  attributes 435,899 instructions to 74 unnecessary function_locals
  scans: deserialized bytecode was marked INHERITED with an empty mask.
- Initialized deserialized closures directly as COMPILED. There is no
  creator's frozen set to filter, and the source body was already compiled
  by GNU R. Isolated rebased comparison on e25e6cc: **45,238,280 ->
  44,882,771 instructions (-0.79%)**. Final startup is +0.14% against
  the original main baseline, within the 0.5% gate. Kept.
- Four Docker-oracle fixtures pass across interpreter, VM and JIT
  (12/12), including a new per-line tab-stop fixture. Full final data,
  blocker slice across tiers, and crate checks follow this measurement.
- Shared edit: vm/rjit.rs constructor state only. No changes to vm.rs.

## 2026-10-29 (round 5): full CLI parse-error context

- R: gram.y / Defn.h:PARSE_CONTEXT_SIZE is 256 bytes, not 50.
  R: source.c:getParseContext / tabExpand splits the context into lines,
  resets tab stops per line, and displays at most 192 bytes per line.
  Ported both limits; changing only the ring size failed the long-line
  oracle, so that incomplete form was rejected before committing.
- Focused split cases d93fcb5b and ef3c791b: **0/2 -> 2/2**, no
  regressions. Both gains absorbed into the FastR ratchet.
- Two small CLI fixtures recorded in robigo-oracle:4.5.0 match all three
  tiers (6/6). The closure print fixture also holds in all tiers (3/3).
- Rebased onto main e25e6cc. Resolved only the append conflict in
  tests/r/passing.txt, preserving every incoming and existing entry.
  No edits to vm.rs. Earlier CLI tests: all seven unit tests and all
  three conformance checks passed; final rebased verification follows.
- Shared edit: crates/robigo-cli/src/echo.rs, confined to error context.

## 2026-10-29 (round 5): closure bytecode identity

- R: print.c:PrintClosure. Print the bytecode line from GNU R's visible
  JIT state, independently of robigo's eager execution bytecode.
- Preserve BCODESXP identity when serialize.c:ReadItem's closure body is
  reduced to its source expression. The JitState helper initializes it
  as already compiled. No changes to vm.rs or the object layout.
- Former blocker slice (all mget/get/syscall/ConditionHandling cases):
  **131/139 -> 131/139**, identical per-case outcomes. Sys.getenv passes
  under all six synthetic environments with bytecode printing enabled.
- Focused help examples **1/4 -> 3/4**: base/get and base/match gained,
  Sys.getenv held; base/identical still errors on missing bcVersion.
  Both gains were absorbed into the ratchet. TestBuiltin_exists holds
  at **279/279**, including one pre-existing unrecorded pass.
- New closure-bytecode.R was recorded in robigo-oracle:4.5.0. It matches
  stdout, stderr and status under the interpreter, VM and JIT (3/3).
- Fixed-binary full data sweep: **8,404/8,426 -> 8,404/8,426**,
  identical per-case results. The sole recorded-pass failure on both is
  order/add626 (host collation). Candidate uses --jobs 6, baseline --jobs 2.
- Runtime tests: 35 pass, two host C-library oracle tests fail (sqrt one
  ULP, and csin(0.5+2i) imaginary component one ULP). The latter test was
  added on main with the Debian glibc trig ports; complex.rs is unchanged
  here, and baseline/candidate CLI sin output agrees at digits=22. Both
  lazy-load integration tests pass. Debian CI remains the numeric judge.
- Shared files: print.rs closure branch, serialize.rs body tag, vm/rjit.rs
  two small state accessors. The evaluator/VM execution files are untouched.

## 2026-10-29 (round 5): environment-independent Sys.getenv recording

- Started from main `24a5b8e` on `track/conformance-sweep-data-5`.
- Re-recorded `base/Sys.getenv` in robigo-oracle:4.5.0 with
  `record --all --mask-against-previous`. Re-recording alone moved the
  redundant R_SESSION_TMPDIR mask from index 94 to 65. The recorder now
  omits masks inside IGNORE_RDIFF blocks and on their markers, leaving
  those blocks to the existing content-based Rdiff comparison. Masks
  outside the block remain intact. No comparison or oracle was weakened.
- Removed the host_dependent skip. Oracle output and baseline robigo both
  compare equal with 0, 1, 10, 40, 80, and 160 synthetic environment
  variables, including pull_request-shaped additions. Metric: skipped
  -> scored and passing. Two Python regression checks pass.
- Shared tooling edits: scripts/corpus.py recording only, manifest and
  oracle-generated expected.jsonl.gz. Runtime comparison is unchanged.


## 2026-10-29 (round 4): silent repeated tracing and distinct symbol identities

- Started from origin/main `74e8c73` on `track/conformance-data-round4`.
- R: debug.c:do_tracemem sets RTRACE unconditionally. Removed the
  invented `object already traced` warning. Symbols now use their interned
  name allocation as identity; environments use their own allocation.
  Untracing a different symbol, NULL, or an environment no longer clears
  a traced symbol's mark.
- Focused oracle metric: **0/2 -> 2/2** scripts match stdout, stderr, and
  status byte for byte. Both scripts were recorded by
  scripts/regen-expected.sh in robigo-oracle:4.5.0. Kept.
- Existing tracemem corpus baseline: **4/8**, zero regressions; the four
  failures contain raw addresses and duplication reports. Copy reporting
  still needs allocation hooks; no oracle masking was changed.
- Both new scripts match the oracle under the interpreter, VM, and JIT
  (6/6 checks). All 226 conformance ratchet scripts pass; the standard
  conformance harness, VM guards, and compiled/interpreted comparison pass.
- Runtime tests: 35 unit + two integration pass. The complete invocation
  reproduces the previously documented host-only c_library_oracle complex
  sqrt mismatch (one ULP, at complex.rs:914). No complex code changed.
  CI's Debian build passes, including its runtime tests.
- Fixed-binary expanded data comparison: **8,404/8,426 -> 8,404/8,426**,
  identical per-case pass/fail outcomes. Only the previously documented
  order/add626 host collation case fails an existing passing entry on
  both binaries. Tracemem corpus stays **4/8 -> 4/8**, zero regressions.
- Commands: scripts/corpus.py run fastr --filter '<expanded filter below>'
  --jobs 2 --keep-output, with --robigo pointing first to the saved main
  binary and then to this worktree's candidate; separate TestBuiltin_tracemem
  runs. R_HOME=$HOME/.local/opt/R-4.5.0 throughout. Formatting passes.
- All CI jobs on implementation commit 1b68739 pass, including every
  corpus shard and the performance gate. Final follow-up changes only this
  verification ledger; no hot runtime paths or shared runtime files changed.
- Ownership audit: lapply's sole remaining 245e62b case needs evaluator
  traceback contexts. The srcref case b4bfbcb9 needs parse-generated
  metadata: do_parse ignores keep.source, while subset.c:VectorSubset's
  srcref transfer is already ported. Parser ledger owns source metadata
  and truncation. Complex assignment order and match traceback remain
  evaluator work. The GNU R status -11 recordings require oracle-owner
  investigation, not an intentional runtime crash.

## 2026-10-29 (round 3), part 3: dim<- coercion warnings; tracemem marks
## and returns the address

- **dim<- coercion warnings** (attrib.c:dimgets runs coerceVector(val,
  INTSXP) before validating): do_dimgets now coerces non-integer atomic
  values through interp.coerce, so `dim(x) <- c(12+10i)` warns
  `imaginary parts discarded in coercion` and `dim(x) <- c(2147483648)`
  warns `NAs introduced by coercion to integer range` next to
  `the dims contain missing values`, all against the `dim(x) <- ...`
  call. The four TestBuiltin_dim cases pass (fastr filter
  8,393 -> 8,397). Note: the bulk `attributes(x) <- list(dim = ...)`
  path (attrib.rs set_attrib, no interp) still coerces quietly — same
  gap class as before, no corpus case exercises it.
- **tracemem family** (debug.c: R_MEMORY_PROFILING build): tracemem
  marks the object in a thread-local pointer set, warns
  `object already traced` on a re-trace, and returns the
  `<0x...>` address string; untracemem clears the mark (NULL,
  invisible); retracemem returns the address visibly for a traced
  object, NULL invisibly otherwise, and re-marks with `previous`.
  The copy-duplication report (`tracemem[0x... -> 0x...]:`) stays
  unimplemented: it prints raw addresses no recorded output can match.
  The seven TestSimpleLists cases that `invisible(tracemem(...))` now
  pass (tracemem no longer errors); the four TestBuiltin_tracemem
  address-printing cases still fail on stdout only (8,397 -> 8,404).
- Final expanded data filter on this branch: **8,404/8,426** (round
  start: 8,388), 0 regressions beyond the host collation case.
  gnur-tests 23/59 (reg-BLAS, print-tests recorded earlier), runtime
  tests and the conformance ratchet unchanged (host complex cases only).
- Remaining 22 failures of the filter, each with a verified owner:
  - 6 factor + 1 lapply (245e62b) + 1 vectors (502b171d): GNU R's
    `Calls:` line omits builtin contexts and prints `<Anonymous>` for
    the auto-print call; contexts live in interp.rs/conditions.rs
    (evaluator/conformance-lang).
  - 5 TestSimpleVectors: 0b6df291/daaeabd5 `[[<-` error shows the rhs
    VALUE where GNU R shows the expression (lang.rs fast_replace,
    conformance-lang's ledger); 8745c2a5/94cf5bf3 `*tmp*` capture order
    (complex assignment in interp.rs: index expr, then rhs, then
    capture — robigo captures before the rhs and drops its mutation).
  - 2 cbind (44215c10) + 2 rbind (51a1b61d + e2d15825): unused-argument
    error prints the argument VALUE `(1)` where robigo prints the
    expression `(v)` (match_args in interp.rs); e2d15825 additionally
    expects status -11 (GNU R segfaults, not reproducible).
  - 2 matcharg (139373f1, 85380d0c): conformance-values' ledger
    (as.character-list deparse + error call).
  - 2 split (d93fcb5b, ef3c791b): parse-error context truncation shows
    a different remainder of the line (robigo-syntax, parser track).
  - 1 subset (b4bfbcb9): srcref in subset (conformance-vectors).
  - 4 tracemem (stdout addresses): unpassable as recorded.
  - 1 order (add626): host collation locale; passes on CI.

## 2026-10-29 (round 3), part 2: apply-family checkArity, env2list names,
## tracemem verdict

- **apply family arity** (apply.c/mapply.c/envir.c all start with
  checkArity): do_lapply/vapply/eapply/rapply/mapply/islistfactor now
  error `N argument(s) passed to .Internal(<name>) which requires N`
  (util.c:Rf_checkArityCall — internals use error(), the context call).
  fastr `.Internal(lapply)` arity cases 74c6e083/951ce209 pass.
- **env2list of an empty environment** (envir.c:do_env2list): the C
  returns the list BEFORE setAttrib(names) when k == 0, so
  `as.list(new.env())` and `lapply(X = environment(), FUN = ...)` print
  `list()`, not `named list()`. fastr TestBuiltin_lapply/2c077844
  passes (envir.rs, conformance-lang's file: 4-line change, the fix for
  the lapply case this track owns).
- fastr expanded data filter **8,390 -> 8,393/8,426** (+3 recorded);
  lapply/apply/env 380/381; gnur-tests 23/59 and gnur-examples base+stats
  show no new regressions. PR #83 carries both batches.
- **tracemem verdict (4 failing cases, not attempted)**: the remaining
  TestBuiltin_tracemem cases print the recorded run's raw addresses
  (`[1] "<0x...>"`, `tracemem[0x... -> 0x...]:`), which no masking covers
  (corpus.py's ADDRESS regex only masks `<environment|bytecode|pointer|
  promise|weak reference|externalptr: 0x...>` forms, mirroring GNU R's
  tools::Rdiff, which also does not mask tracemem lines). The addresses
  differ on every GNU R run too, so no implementation can pass the cases
  as recorded; they would need re-recording with unstable-output masking
  first (corpus.py record --repeat). The other 4 tracemem cases pass
  without tracing (they only exercise errors/untracemem warnings).
- Remaining open items, re-attributed after this round:
  - `*tmp*` evaluation order (fastr TestSimpleVectors 8745c2a5,
    94cf5bf3): `x[x[4]<-2] <- (x[4]<-100)` — GNU R evaluates the index
    expression, then the rhs, then captures `*tmp*` (x[4]=100 persists);
    robigo captures `*tmp*` before the rhs, so the rhs mutation is lost.
    Needs the complex-assignment order in interp.rs (evaluator, shared).
  - factor traceback ×6 + lapply 245e62b + match traceback ×1: GNU R's
    `Calls:` line omits builtin contexts (`Calls: <Anonymous> ...`
    auto-print vs robigo's `Calls: print ...`; `lapply -> is.vector`);
    contexts for builtins live in interp.rs/conditions.rs
    (conformance-lang's area).
  - unused-argument printing ×2 (matcharg 139373f1, 85380d0c) —
    conformance-values' ledger already claims them (as.character-list
    pairlist deparse + error call).
  - rbind/cbind ×2 expect status -11 (GNU R segfaults on those inputs);
    robigo cannot reproduce a segfault. parse-error truncation ×2 —
    parser track. srcref ×1 — subset.rs (conformance-vectors).
  - utf8ToInt invalid bytes / `"\304"` / iconv: blocked on core's
    per-CHARSXP encoding window. base/identical page: blocked on
    bytecode-print/compile-moment alignment. base/options page: echo
    default (cli track), bitmapType, pager path (host).

## 2026-10-29 (round 3), part 1: identical row.names, malformed factor,
## options simd/max.print, round/signif missing digits

- Started from origin/main 7ff8a140 in a fresh worktree. Baseline on this
  host: expanded data filter **8,388/8,426** (the one "regression" is
  `TestBuiltin_order/add626`, the known en_US.utf8 collation case this
  host lacks; CI has the locale), gnur-tests 21/59, gnur-examples
  `^base/` 288/380 (12 host-only: Platform, Rhome, as.environment,
  getCallingDLL, getLoadedDLLs, library.dynam, regmatches, search,
  srcfile, strsplit, strwrap, system.file).
- **identical row.names through getAttrib** (identical.c's R_RowNamesSymbol
  special case, both ATTR_AS_SET paths): `attr(x,"row.names")` stored
  `c(NA, +3)` vs `c(NA, -3)` now both expand to `1:3` and compare equal.
  gnur-examples `base/cbind` passes (recorded). `base/identical` is still
  blocked on `<bytecode: ...>` printing of compiler::cmpfun closures
  (needs the compile-moment alignment, see round 2's rejected experiment).
- **asCharacterFactor errors** (coerce.c:asCharacterFactor ported to
  RResult): "attempting to coerce non-factor", "malformed factor" for
  non-character levels AND out-of-range codes; drops the names copy the C
  never made. Callers updated: misc.rs (the .Internal), lang.rs + vm.rs
  for-loops, unique.rs match_transform, attrib.rs as_char_simpl (now
  RResult), robigo-capi bridge.rs. fastr factor 57 -> 59/65: the two
  direct `as.character(x)` cases pass and are recorded; the other six now
  differ ONLY in the traceback's first entry — GNU R prints
  `Calls: <Anonymous> ... print.factor` for the auto-print at top level,
  robigo prints `Calls: print ...`. That is the evaluator's call-stack
  handling of auto-print (interp/print), not asCharacterFactor; left for
  the owner of the traceback machinery.
- **options**: `matprod = "default.simd"` accepted silently (the Debian
  r-base-core 4.5.0-3 build robigo tracks has OpenMP SIMDRED; the warning
  is `#if !defined(HAVE_OPENMP_SIMDRED)`); max.print/nwarnings now use
  R's asInteger with its CoercionWarning, so `options(max.print = Inf)`
  warns `NAs introduced by coercion to integer range` then errors
  `invalid value for 'max.print'`. gnur-tests reg-BLAS and print-tests
  both pass and are recorded (gnur-tests 21 -> 23/59, 0 regressions).
  `base/options` still fails on host/other-track lines only (bitmapType
  cairo vs Xlib, pager path, echo default).
- **round/signif with a missing digits formal** (core's handoff): the
  bug was in math.rs's private `eval_list_keep_missing`, which lacked
  eval.c's `R_isMissing` check, so evaluating the `digits` symbol (bound
  to the caller's unmatched formal) errored. Deleted the duplicate;
  do_Math2 now uses subset.rs's `eval_keep_missing` (the existing
  evalListKeepMissing port, visibility pub(crate)). First attempt kept
  the symbol expression as the arg value and broke x (`non-numeric
  argument`); the shared port pushes Value::missing_arg, and the default
  then applies in do_math2_group as in C. `f <- function(x, digits)
  signif(x, digits); f(1.23456789)` prints 1.23457; `round` likewise;
  missing `x` now reports `Error in round(x) : argument "x" is missing,
  with no default` (R_MissingArgError_c uses do_Math2's call).
  grDevices/xyTable passes and is recorded.
- Complex %*% NaN: re-verified on new main (`as.complex(NA) %*% NaN`
  prints NaN+NaNi; the two ratcheted complex scripts fail on the BASELINE
  binary on this host exactly as the ledger documented — host NaN-payload
  issue, main's CI green, not from this branch).
- Metric: expanded data filter **8,388 -> 8,390/8,426** (+2 recorded),
  0 real regressions; gnur-examples `^base/` 288 -> 289/380 + xyTable
  (+3 recorded with cbind; eapply/list2env were pre-existing unrecorded
  passes from the morning wave, recorded too); gnur-tests 21 -> 23/59.
  cargo test -p robigo-runtime: only the documented host complex oracle;
  conformance ratchet: only the two documented host complex scripts
  (verified against the baseline binary). Build note: the shared
  ~/.cache/cargo-target again served another worktree's robigo-capi
  build-script state (`main/g_her_glyph.c` not found); this round used
  CARGO_TARGET_DIR=~/.cache/robigo-data3-target throughout.

## 2026-10-29 (later): pretty cell shrink, identical row.names finding

- **Kept fix**: `pretty(0, min.n = 4)` returned `-2 -1 0 1 2`; GNU R gives
  `-0.4 -0.2 0 0.2 0.4`. The R_pretty port applied the shrink_sml/min_n
  cell adjustment only inside the max(|lo|,|up|) branch, but pretty.c
  adjusts every i_small input — including `cell = 1` from `lo == up == 0`.
  Restructured to the C's two-step shape and ported the
  `R_pretty(): infinite range` warning (r_pretty now takes the interpreter
  and returns RResult). gnur-examples `base/pretty` passes and is
  recorded; the full data filter stays 8,387/8,425, 0 regressions.
- Found while bisecting gnur-examples `base/cbind` (still failing):
  **identical.rs must compare row.names through getAttrib** (identical.c
  treats R_RowNamesSymbol specially in both ATTR_AS_SET paths). GNU R:
  `x` with stored `c(NA, +3)` and `y` with `c(NA, -3)` are `identical`
  (both expand to `1:3`); robigo compares the raw stored attributes and
  says FALSE, so `row.names(d2) <- attr(d3, "row.names");
  stopifnot(identical(d2, d3))` fails after rbind.data.frame. robigo's
  attr GET/SET compaction itself already matches GNU R (verified against
  the oracle: assignment stores the sign as given; sequences 1:n compact
  to `c(NA, +n)`). The ~6-line special case belongs in identical.rs,
  which no track lists as owned.
- Corpus flake seen once: three vapply/whichmax "REGRESSION" lines from a
  run that raced another worktree's corpus jobs (per-case tmp dirs are
  keyed by corpus+case id only); the identical rerun and a focused
  47-case slice both pass with zero regressions.
- One fastr slice of note: `utf8ToInt` must return NA for invalid byte
  sequences (utf8 gnur-test); blocked on per-CHARSXP encodings, see below.

## 2026-10-29: second sweep (branch track/conformance-sweep-data-2)

- Started from origin/main e3909b9 (PR #29 and the complex-matprod branch
  were merged; `as.complex(NA) %*% NaN` verified to print `NaN+NaNi`).
- Baseline on main: expanded data filter **8,386/8,425** (39 failing),
  gnur-tests 21/59, gnur-examples 602/1,149 with 11 ratchet entries failing
  for host reasons only (local `R.home("doc")` lacks NEWS.2/AUTHORS content,
  `searchpaths()` shows the local R_HOME). Recorded 75 new gnur-examples
  passes and one unrecorded fastr pass (`TestSimpleDataFrames/c4345940`).
- Kept fix: **cbind/rbind split language objects into their components**
  (coerce.c:coerceVector → PairToVectorList; tags become names, so
  `cbind(substitute(x(y=3,z)))` gets row names y/blank). `coerceVector` on a
  symbol now warns `'symbol' object cannot be coerced to type 'list'` and
  yields NULL as coerce.c:coerceSymbol does. fastr +2 recorded
  (`TestBuiltin_cbind/9927c42`, `88301329`), filter → **8,387/8,425**,
  0 regressions; matrix conformance scripts pass except the pre-existing
  host-dependent complex_nan.R (below).
- Shared-target collision: another worktree's robigo-capi build script
  (compiling `appl/pretty.c`) reran against this tree's sources and failed;
  built with a private `CARGO_TARGET_DIR=/tmp/robigo-data-verify-target`
  from then on.

### Found for other owners (verified repro on current main)

- **perf-vm (vm.rs `element`)**: a compact-range `x` indexed with 1 returns
  the whole vector: `f <- function() { x <- 1:5; x[1] }` prints `1 2 3 4 5`
  (also `x[[1]]`, any scalar index whose value is 1, promise or literal).
  `element()` returns `x.clone()` for any non-`V` RVal at `i == 0` — right
  for scalar `I`/`D`/`L` RVals, wrong for `RVal::Range`. Global `x`,
  `x[2]`, `x[1, drop=FALSE]` and the interpreted path are unaffected.
  Fails fastr `TestSimpleVectors/9313fe3a` and likely more outside the
  corpus (base R code uses `x[i]` on ranges in closures everywhere).
- **conformance-lang (lang.rs `fast_replace`)**: complex-assignment error
  calls show the rhs *value*: `` `[[<-`(`*tmp*`, i, value = 10) `` where
  GNU R shows the *expression* `value = v` (fastr TestSimpleVectors/
  0b6df291325c0b6c, daaeabd510342a4c). `rcall` should carry `rhs_expr`.
- **conformance-values (coerce)**: `as.character(list(as.raw(1)))` must be
  `"as.raw(0x01)"` (coerceVectorList deparse1line's non-string elements);
  ours gives `"01"`, so `pmatch`'s base wrapper matches `'01'` and
  match.arg returns instead of erroring (fastr TestBuiltin_matcharg/
  85380d0ca4207480). Same file: `as.character` errors must report the
  `as.character(table)` call, not the caller's (matcharg/139373f181119b60).
- **options.rs**: `options(matprod = "default.simd")` must be accepted
  silently (gnur-tests reg-BLAS; our warning only fits builds without
  OpenMP SIMD), and `options(max.print = Inf)` must warn
  `NAs introduced by coercion to integer range` (gnur-tests print-tests).
- **misc.rs**: `asCharacterFactor` must error `malformed factor` when the
  levels attribute is missing or not a character vector (8 fastr factor
  cases, envir::as_character_factor ignores the levels type).
- **complex.rs (host-dependent)**: on this machine `cargo test -p
  robigo-runtime` fails `builtins::complex::tests::c_library_oracle`
  (sqrt ULP) and ratcheted `tests/r/matrix/complex_nan.R` differs at line 7
  (`NA NaN` vs `NA NA`) — both fail with this branch's changes stashed, and
  main's CI is green, so they look compiler/machine-dependent, not semantic.

### Still open in this slice (unchanged causes)

- fastr 37 remaining: factor 8 (misc.rs), tracemem 7 (debug.rs), `*tmp*`
  evaluation order 2 and match traceback 1 (evaluator), `[[<-` rhs display
  2 (lang.rs), unused-argument value printing 2 (arg matching), rbind/cbind
  GNU-R-segfault expectations 2 (expected status -11; not reproducible),
  parse-error truncation 2, srcref 1 (subset), lapply 4 (apply.rs),
  `x[1]`-on-range 1 (vm.rs), as.character-list 2 (coerce).
- utf8 gnur-test: `utf8ToInt` must validate bytes and return NA for invalid
  sequences; robigo stores byte strings as Latin-1 chars (lang.rs
  const_to_value), so byte-level validation needs per-CHARSXP encodings
  (value.rs) — same blocker as iconv/`Encoding<-`.
## 2026-10-29: handoff from core (graphics examples, #69)

Found while bringing up the graphics engine; reproduce with
`scripts/corpus.py show gnur-examples <case>` (needs #69's natives).

- `"\304"` in R source is the single byte 0xC4 (an invalid UTF-8 string
  in GNU R, an Adobe Symbol code in `symbol()`); robigo's lexer makes it
  U+00C4 ("Ä", two bytes). `grDevices/plotmath` then warns "font metrics
  unknown for character 0x84" where GNU R draws circlemultiply;
  `gnur-demos graphics/Hershey` stops at `rawToChar(..., multiple =
  TRUE)` ("strings that are not valid UTF-8 are not supported"). Needs
  byte strings (the encoding window, core.md §4.1).
- `signif(x, digits)` / `round(x, digits)` with `digits` a missing
  argument must use the default: in GNU R both are specials (names.c
  eval 0, arith.c do_Math2 checks `CADR(args) == R_MissingArg`); robigo
  evaluates the argument and fails with 'argument "digits" is missing,
  with no default' (`grDevices/xyTable`:
  `f <- function(x, digits) signif(x, digits); f(1.23456789)`).

## 2026-10-28: resume after PR #29 merged


- PR #29 was already merged with passing checks. The worktree was clean,
  with no commits absent from origin/main. Rebased onto 49e0ee5 and started
  a follow-up branch.
- GNU R sources: array.c:CMATPROD_BODY, CCROSSPROD_BODY, TCCROSSPROD_BODY;
  Rcomplex.h:toC99. Leave matprod_real unchanged.
- Reproduced TestSimpleArithmetic/6f36cc66d662530c: NA printed instead of
  NaN+NaNi. The C99 recovery itself is needed for infinities; NaN operand
  selection before recovery loses the distinction between NA and NaN.
- Experiment: explicit NaN payload selection on the exceptional path.
  The first ordering failed five of 64 oracle pairs. Correcting the order
  and expanding to 625 pairs exposed eight additional real-component
  differences; corrected those too. The standalone helper now agrees on
  all 625 NA/NaN/finite classifications with Debian GNU R 4.5.0.
  Full runtime verification then exposed tcrossprod selecting the reverse
  operand order; reversing its helper inputs matches all 625 pairs too.
- Added a conformance test recorded by scripts/regen-expected.sh in
  robigo-oracle:4.5.0, including all three products and accumulation.
  An initial dput-based test exposed existing complex deparse differences;
  replaced it with component checks to isolate arithmetic.
- Residual groups inspected: tracemem is a debug builtin; factor/rep
  recursion and method argument matching belong to language dispatch;
  replacement evaluation order and printed calls belong to the evaluator;
  names(quote(x(y=z))[2]) loses the call head tag in Lang::from_items and
  requires a value.rs representation change. Those files are outside scope.
- Build environment: shared cargo artifacts initially selected stale syntax
  metadata without Arena. Touched syntax lib.rs to force rebuilding without
  changing its contents. Used the shared build lock and separate baseline
  and candidate binary copies for measurements.

Filter (the earlier branch's broader filter plus the queued regression and
crossprod):

```text
TestBuiltin_(crc64|rbind|cbind|extract_dataframe|Date2POSIXlt|POSIXlt2Date|formatPOSIXlt|strptime|asPOSIX|allequalPOSIXt|Sys|dump|filecopy|fileaccess|filecreate|fileexists|fileinfo|filepath|fileremove|filerename|factor|rep|repint|rep_len|seq|seqint|seq_len|seq_along|order|sort|sortfastpass|psort|qsort|radixsort|unique|duplicated|match|which|array|matrix|dim|subset|subset2|subset_dataframe|list|unlist|lapply|vapply|split|paste|format|substr|sprintf|nchar|strsplit|print|toString|cat|deparse|dput|makeunique|strrep)|TestSimpleVectors|TestSimpleLists|TestSimpleDataFrames|TestSimpleMatrix|TestSimpleArrays|6f36cc66d662530c|TestBuiltin_crossprod
```

- Host baseline: gnur-tests 19/59 (two unrecorded passes). r-strict-tst's
  sole difference was elapsed CPU time, 0.001 instead of 0. The base/utils
  example slice was 346/500 with 16 pre-existing recorded-pass failures,
  including platform paths, missing GNU R documentation files, and today's
  date in round.POSIXt. Do not change the oracle or remove passing entries.

- Current-main FastR baseline: **8,377/8,426**, including the queued
  arithmetic regression. The previous 8,218-case filter is **8,170/8,218**.
  Failure counts by group: library/base/TestSimpleVectors: 9, builtins/TestBuiltin_factor: 8, library/base/TestSimpleLists: 7, builtins/TestBuiltin_dim: 4, builtins/TestBuiltin_lapply: 4, builtins/TestBuiltin_rep: 4, builtins/TestBuiltin_cbind: 3, builtins/TestBuiltin_matcharg: 2, builtins/TestBuiltin_rbind: 2, builtins/TestBuiltin_split: 2, builtins/TestBuiltin_repint: 1, builtins/TestBuiltin_subset: 1, library/base/TestSimpleArithmetic: 1, library/base/TestSimpleDataFrames: 1.

- Candidate runtime now matches the complete recorded test byte for byte:
  625 operand pairs for each of three products, plus 625 accumulation
  pairs and the queued scalar regression. Runtime unit tests: 23 passed;
  lazy-load integration tests: 2 passed. Formatting passes.

- Performance experiment: branching on op inside every complex product
  raised a finite complex-matrix workload from 178,799,017 to 182,989,313
  instructions (+2.34%). Rejected that placement; move operand selection
  into the NaN-only path and remeasure before keeping the implementation.

- Moving only the branch still cost 182,921,899 instructions (+2.31%).
  Separate the exceptional handling into a cold function, keeping the four
  multiplications and two component additions inline for finite products.

- Cold extraction still cost 184,156,240 instructions (+3.00%); rejected
  as the finite-path solution. Follow array.c's finite-input split instead:
  scan for nonfinite components once and specialize the inner product loop.
  Keep the existing serial component sums; the real matrix path is untouched.
- Full expanded filter: **8,377 -> 8,378 of 8,426**. The queued regression
  is restored; all 207 crossprod cases still pass, no remaining regressions
  within the filter. gnur-tests: 19 -> 20/59 (only timing changed), examples
  unchanged at 346/500 with the same host-only failures.

- Added finite overflow/underflow oracle cases. These exposed an existing
  ccrossprod BLAS difference: ZGEMM's transposed-A branch writes ALPHA*TEMP,
  even for ALPHA=(1,0). Ported that final component multiplication from
  src/extra/blas/cmplxblas.f:ZGEMM. It matters when TEMP overflows.
- A real-matrix comparison exceeded 0.5%, but callgrind attributed the
  entire difference to different parser artifacts reused from other
  worktrees. matprod_real's component instruction counts were identical.
  Copied the cached build dependencies into a private target directory,
  forced this checkout's syntax crate to rebuild, and rebuilt both baseline
  array.rs and the candidate against those same frozen dependencies.

Reproduce the complex performance workload with R_DEFAULT_PACKAGES=NULL:

```r
x <- matrix(rep(1+2i, 2500), 50L)
for (i in 1:5) { y <- x %*% x; z <- tcrossprod(x) }
print(y[1,1]); print(z[1,1])
```

Run each binary under valgrind --tool=callgrind. The isolated build uses
CARGO_TARGET_DIR=/tmp/robigo-data-verify-target with scripts/build.sh;
its baseline restores array.rs from 49e0ee5, all other sources identical.

- Isolated final performance: complex workload **178,821,262 -> 162,054,932**
  instructions (-9.38%); real robigo/matrix
  **606,416,867 -> 606,231,030** (-0.03%). compare reports no slowdown
  above 0.5%. Final overflow cases and exhaustive component tests match
  GNU R byte for byte. The earlier PR head's full CI was green, including
  all corpora and performance.

## 2026-10-29 (evening): rebase onto main (PRs #50/#55/#65 merged)

- **Rejected experiment**: the `<bytecode: ...>` print line. Correct per
  print.c when a closure carries compiled code, but robigo attaches
  bytecode on the FIRST call while GNU R's default JIT compiles a few
  calls later, so closures printed after one call gained a spurious line:
  fastr `TestBuiltin_mget/42b0be374d04c8a9`, `TestBuiltin_get/…`,
  `TestBuiltin_syscall/…`, `TestConditionHandling/aa85fee057f56512` and
  `base/Sys.getenv` (gnur-examples) all regressed on CI. Reverted; needs
  PR #51's compile-moment alignment first (and base lazy-load bytecode
  from PR #56 for the base/match case).
- CI on the first push failed only on `base/Sys.getenv` in gnur-examples;
  it passed after rebasing (the failure came from the pre-#65 merge state,
  not from this branch's changes — verified locally: 10/10 base/Sys*
  examples pass on the rebased build).
- perf-vm's PR #65 fixed the compact-range `x[1]` VM bug found and
  reported by this sweep: fastr `TestSimpleVectors/9313fe3a33cc9033` now
  passes and is recorded (+1; filter 8,388/8,426 on the rebased tree).
- Main re-recorded the gnur-examples host cases with cross-machine masks
  and dropped them from `host_dependent`; the remaining local-only
  failures (base/Platform, Rhome, searchpaths, strsplit's NEWS.2/AUTHORS
  reads, `TestBuiltin_order/add626`'s collation) all pass on the CI
  platform — left to CI to arbitrate, none recorded from here.
- gnur-examples `methods/GenericFunctions` passes locally but not on CI's
  earlier run; NOT recorded to keep the ratchet platform-stable.
- `base/Sys.getenv` failed on every CI run of this PR while main's CI is
  green: its recording masks exactly one line (index 94, the recording's
  R_SESSION_TMPDIR line), and a pull_request run's slightly larger
  environment shifts the dump so that index 94 can land on the
  IGNORE_RDIFF_END marker, which the mask then eats, breaking the
  ignore-block drop. Reproduced the sensitivity with corpus.py's run_case
  under synthetic environments; not a semantic regression (stdout compares
  equal under every env tried locally). Skipped the case via
  `host_dependent` (it dumps the machine's environment); the proper fix is
  a re-record with `record --mask-against-previous` on the oracle platform.

## 2026-10-30: repair #100's paired performance regression

- Round 10 is paused on track/conformance-sweep-data-10 / #105. Repair
  #100 first; do not merge either PR. Resume construction from merged main.
- Failed CI 41005186119, head 3fe3f4b: nbody_naive_2 sample 1 rises
  1,259,933,960 -> 1,270,394,353 instructions (+10,460,393, +0.83%).
  Recovered the exact retained base/candidate callgrind profiles.
- Attribution: ReadBC now reserves both operations and heap-item vectors
  per code object (5,372 reserve calls absent on base); the heap-item
  reservation scales with twice the instruction count despite packed scalar
  leaves. Its Bytecode clone also deep-copies consts and cell vectors.
  The ordinary attrs_slot_mut body grows by 4,055,936 exclusive instructions,
  and Interp::eval by 3,847,973 after its new direct self-recursive branch.
  These hot-path lowering costs accompany importer allocations; they are
  not evidence that all of the regression is spent parsing instructions.
- Rebase onto 532be45 (#104): preserve main's 48-byte PNode, cold call-tail
  reader, and lang-cell writer arguments; union passing.txt including
  substitute-dots-cell.R. No changes to that PNode layout or main's cell logic.
- Candidate: retain the validated cell stream and a shared repetition table;
  materialize constants and non-source, unreferenced language objects only
  on inspection. Serialization replays the stream without forcing metadata.
  Rc shares immutable cells and forced constant pools across attribute COW.
  Remove the oversized heap-item reservation. Keep bytecode evaluation and
  bytecode attribute copying in cold helpers to protect ordinary evaluation.
  Minimal bridge edit forces constants only when C API inspection asks.
- Matched main binary built at 532be45 in a private cached target. Paired
  cache-OFF verification targets all five reported regressions plus robigo/fib
  and rbench/simple/bytecodes, using the real paired harness with threshold
  0.2 and the supplied Debian R 4.5.0 home (host R RHOME is 4.6.1).
- First lazy candidate: round-trip/promise fixtures remain 6/6, all 256
  other oracle scripts pass per tier (only main's known unlisted user-closure
  probe differs). Bounce and nbody_2 meet 0.2%, but nbody_3 is +0.326%:
  reject as the final repair. Profiling still attributes the residual to
  import allocation/page work, not the now-cold evaluator/attribute arms.
- Second candidate records nested bytecode as indexed views in one validated
  cell stream per ReadBC root. No nested Bytecode objects, separate operation
  vectors or separate heap-item vectors are created until inspection. The
  shared repetition table is moved into the backing data, not cloned.
  Writers and lazy readers skip nested spans; canonical serialization does
  not materialize those objects. This directly removes the remaining eager
  nested allocations. Root PNode and main's LANGSXP call-tail handling stay
  unchanged. Add oracle-backed tests for deferred pools, shared metadata
  before/after forcing, nested views, and the scalar constant pool.
- Initial new-test compile failed because Flow lacks Debug; use an explicit
  fixture match instead of Result::unwrap. No performance or conformance
  result was claimed for that failed test build.

- Reject the second, global i64 stream: fixtures 6/6 and crate tests pass,
  but nbody_2 +1.23%, nbody_3 +1.38%, nbody_naive_2 +1.58%,
  profiler-shared +0.21%. Its large contiguous allocation adds about
  8.0M exclusive instructions in _mi_malloc_generic and 5.73M in mi_find_page
  on nbody_naive_2. Restore per-code streams before the next experiment.
- Third candidate validates binary instructions directly over encoded words,
  preserving R_bcEncode version normalization without an INTSXP allocation.
  Keep unused atomic vectors as encoded payloads and attribute pairlists;
  materialize typed vectors and attribute maps on inspection. Bounded shared
  byte chunks own only retained payloads; Reader/input ownership APIs stay
  unchanged. All six bytecode fixture runs and all previously passing oracle
  scripts still pass. Against 532be45: nbody_2 -0.123%, nbody_3 -0.132%,
  nbody_naive_2 +0.193%, bytecodes -0.745%, fib -0.010%, but Bounce +0.250%
  and profiler-shared +1.411%. Reject as the final repair. profiler-shared's
  remaining cost is allocator page work: _mi_malloc_generic +35,505,774,
  mi_find_page +25,672,332 and mi_bin +9,043,412 exclusive instructions.
- Compact the replay records with signed varints and cap initial reservation
  at 4 KiB. All six bytecode fixture runs pass. Isolated paired measurement
  against the same 532be45 binary reduces profiler-shared to +0.777%,
  5,782,813,664 -> 5,827,721,539. Still above 0.2%; not a final repair.
- Rebase again onto 1a591ff (#106/#107/#110). passing.txt keeps both sides,
  including all LAPACK and escape-origin additions. Build an exact matching
  main binary in the private target before measuring the rebased candidate.
- Next experiment freezes each finished compact replay log into the bounded
  shared byte arena, releasing its separate Vec allocation after import.
  This targets retained heap size classes rather than removing validation.

- Shared compact-log arena candidate: profiler-shared +0.124%, Bounce +0.128%,
  nbody_2 +0.173%, nbody_3 +0.050%, fib +0.136%, but nbody_naive_2 +0.337%
  and bytecodes +0.240%. All recorded-script ratchets hold in all three
  tiers (265/266 scripts per tier, only main's unlisted compiled-context
  probe differs); both bytecode fixtures remain 6/6. Crate tests: 45 runtime
  unit, 4 metadata integration, 2 lazyload and 2 C API tests pass. The two
  documented host glibc complex oracles fail unchanged; unmodified CI tests
  remain the judge. Reject this as the final repair; retain the measured
  allocation improvement while addressing eager nested source expressions.
- The new methods-enabled FastR serialize/unserialize slice is 317/322.
  Its sole listed regression, serialize/6260e145e42c04ad, serializes the
  huge compact sequence 1:2147483649 and times out on this machine. The
  exact matching main-1a591ff binary also times out on that case. No oracle
  or ratchet changes; leave the full pinned-platform result to CI.
- Next: source expression cache is lazy for nested ReadBC constants. Those
  constants are replayed/inspected but are not needed to evaluate the outer
  source body. Repetition definitions are still materialized when referenced;
  the source is reconstructed from the validated log on demand.

- Reject nested source-cache experiment: fidelity still 6/6, all script
  ratchets hold; Bounce +0.228%, nbody_2 +0.225%, nbody_3 +0.097%,
  nbody_naive_2 +0.378%, bytecodes +0.292%, fib +0.174%,
  profiler-shared +0.126%. Restored the shared-arena candidate. The initial
  new cache test assumed the first nested source was a call; GNU R's
  disassembler shows it is the symbol x. Corrected that experimental test
  (5/5 then pass) before reverting the whole unsuccessful design.
- Next experiment packs scalar real constants as their exact IEEE bits in
  the replay log. This avoids a separate encoded-vector descriptor and span
  for every scalar, extending the existing scalar integer/logical encoding.
  Added a Debian GNU R fixture with negative zero, R NA, NaN, infinities
  and complex data; check its bits and bytes before/after inspection.

- Packed scalar-real candidate preserves all five metadata tests, all
  recorded-script ratchets, and the six bytecode fixture runs. Paired
  against 1a591ff (cache off, two measurements per side):
  rbench/awf/Bounce: 619,198,108 -> 620,104,773 (+0.146%).
  rbench/shootout/nbody_2: 867,567,020 -> 869,177,206 (+0.186%).
  rbench/shootout/nbody_3: 967,113,209 -> 967,743,364 (+0.065%).
  rbench/shootout/nbody_naive_2: 1,258,198,001 -> 1,261,754,444 (+0.283%).
  rbench/simple/bytecodes: 231,206,116 -> 231,833,211 (+0.271%).
  rbench/simple/profiler-shared: 5,782,936,386 -> 5,792,841,598 (+0.171%).
  robigo/fib: 958,970,882 -> 960,354,463 (+0.144%).
  Not a final repair: naive_2 and bytecodes still exceed 0.2%.
- Next remove duplicate flag reads, avoid spines for unmaterialized
  prefixes, and specialize the common one-to-three-byte replay records.
  No validation, repetition registration, or serialized records are omitted.

- Reject the combined small-record fast paths: all five metadata tests,
  six bytecode fixture runs and script ratchets hold, but the explicit
  integer path adds 2,182,529 exclusive instructions on Bounce. Paired:
  rbench/awf/Bounce: +0.405%.
  rbench/shootout/nbody_2: +0.376%.
  rbench/shootout/nbody_3: +0.242%.
  rbench/shootout/nbody_naive_2: +0.413%.
  rbench/simple/bytecodes: +0.961%.
  rbench/simple/profiler-shared: +0.200%.
  robigo/fib: +0.305%.
  Restore the scalar-real shared-arena candidate.
- Reject an explicit cold bytecode destructor wrapper: all metadata and
  script checks hold, but the two remaining blockers stay at naive_2
  +0.285% and bytecodes +0.280%. Revert the wrapper and its unsafe drop.
- Next screen fixed-width replay words with a full-i64 escape. This keeps
  the bounded backing arena but removes per-byte varint loop work.

- Fixed-width bytes with an i64 escape also fail the screen: naive_2
  +0.418%, bytecodes +0.969%; metadata/script checks all pass. Revert it.
- Next use native i64 builder vectors and bounded shared i64 chunks for the
  validated log. A record is one Vec push again. Copy each finished builder
  once into backing chunks and release its capacity; do not retain either
  the code-sized reservation or a single large global vector. The separate
  encoded-byte arena still defers instruction and atomic-vector R objects.

- Native-word chunk screen now clears both blockers: nbody_naive_2
  1,258,197,030 -> 1,260,195,691 (+0.159%); bytecodes
  231,232,586 -> 231,305,608 (+0.032%). All five metadata tests, 45 runtime
  unit tests, two lazyload and two C API tests pass (the same two host glibc
  oracles excluded only from the local follow-up run). The unchanged full
  unit command still exposes those documented host failures. Script ratchets
  hold across all three tiers, and the bytecode fixture pair remains 6/6.
  Keep this candidate for the final seven-benchmark paired run.

- Full native-word run on 1a591ff clears the five reported regressions and
  bytecodes, but fib is +0.223% (958,976,714 -> 961,117,373). Do not push
  this as a completed repair. Main then advances to b7ee268 (#98).
- Rebase onto b7ee268. Keep main's weak-reference graph in the bridge conflict
  and add Bytecode to the existing fast proxy-root branch. Preserve every
  ratchet entry. The new exhaustive collector requires a Bytecode identity,
  closure metadata edges and separate owners for shared cell, constant-pool
  and repetition-table allocations. Trace only initialized caches; do not
  force constants/instructions. A new cycle test exercises shared metadata
  both before and after forcing. Three new main Closure test constructors
  initialize gnu_bytecode=None. This necessary integration is the only edit
  outside the existing bytecode import files.
- All 56 runtime unit tests, five metadata tests, two lazyload and two C API
  tests pass locally with the documented two host glibc oracles excluded
  from the follow-up run. All recorded-script ratchets pass in each tier
  (276/277 scripts; only the unlisted compiled-context probe differs).
- Also append the Bytecode enum variant to retain the existing value tag
  order. Full paired run against the exact new main binary still misses
  naive_2 narrowly and fib, so not a final repair. Results:
  rbench/awf/Bounce: 619,721,642 -> 620,412,086 (+0.111%).
  rbench/shootout/nbody_2: 868,231,998 -> 869,485,282 (+0.144%).
  rbench/shootout/nbody_3: 967,741,551 -> 968,018,415 (+0.029%).
  rbench/shootout/nbody_naive_2: 1,258,902,619 -> 1,261,428,078 (+0.201%).
  rbench/simple/bytecodes: 231,968,845 -> 232,023,825 (+0.024%).
  rbench/simple/profiler-shared: 5,769,770,660 -> 5,762,600,600 (-0.124%).
  robigo/fib: 957,442,967 -> 960,035,901 (+0.271%).
- fib's profile attributes the residual to import and allocator/copy work,
  not the hot arithmetic loop: memcpy +1,695,419, _mi_malloc_generic
  +1,118,528, mi_find_page +804,357 exclusive instructions. The new cell
  spine also contributes 565,488 drop-glue instructions. Next screen only
  removal of duplicate leaf flag reads, without the failed integer fast path.

- Duplicate-leaf-flag screen: naive_2 now +0.179%, fib +0.246%.
  The remaining fib copy is directly attributed by caller edges:
  BytecodeWords::append +890,483 memcpy instructions across 2,507 calls;
  allocator reallocations +745,399 across 891 added calls. The language
  spine's explicit reserve-copy difference is only 8,710 instructions,
  so do not guess that its layout is the main copy source.
- Next transfer finished native-word buffers into shared backing storage
  by ownership, eliminating that append allocation/copy. Keep the capped
  initial reservation and all lazy typed metadata; no unsafe code is needed.

- Reject moving every replay buffer into storage without packing. All
  fidelity, metadata and script checks pass, and fib/bytecodes improve, but
  retained buffer capacities increase allocator work on nbody and profiler.
  rbench/awf/Bounce: -0.125%.
  rbench/shootout/nbody_2: +0.433%.
  rbench/shootout/nbody_3: -0.039%.
  rbench/shootout/nbody_naive_2: +0.504%.
  rbench/simple/bytecodes: -0.583%.
  rbench/simple/profiler-shared: +1.071%.
  robigo/fib: -0.032%.
  Restore packed backing storage. Next increase the transient builder
  reservation to instructions.len()*4, then release it after packing. This
  targets the observed 891 added reallocations and their 745,399 copy
  instructions without retaining the oversized capacity.

- Reserve-four candidate still fails: profiler-shared +0.797%, fib +0.204%.
  Fixtures, metadata tests and all script ratchets hold. Reject as final.
- Keep the reserve-eight candidate: retain packed backing storage, release
  every temporary builder after packing, and reserve enough for the observed
  source-cell expansion to avoid growth copies. The real unchanged paired
  harness compares exact b7ee268 main and candidate binaries with cache OFF,
  identical drivers/parameters, R 4.5.0 and two samples per side (minimum):
  rbench/awf/Bounce: 619,734,433 -> 619,274,834 (-0.0742%).
  rbench/shootout/nbody_2: 868,246,785 -> 868,338,976 (+0.0106%).
  rbench/shootout/nbody_3: 967,680,015 -> 967,006,794 (-0.0696%).
  rbench/shootout/nbody_naive_2: 1,258,876,962 -> 1,260,316,460 (+0.1143%).
  rbench/simple/bytecodes: 231,921,199 -> 230,933,261 (-0.4260%).
  rbench/simple/profiler-shared: 5,769,774,970 -> 5,761,489,684 (-0.1436%).
  robigo/fib: 957,423,730 -> 959,026,818 (+0.1674%).
  All seven satisfy <=0.2%; zero broken cases. Bytecode fixtures remain
  6/6; all recorded-script ratchets hold in interpreter, VM and JIT
  (276/277 scripts per tier, only the known unlisted compiled-context probe).
  Runtime tests: 56 pass with only the two documented host glibc oracles
  filtered from the follow-up run; metadata 5/5, lazyload 2/2, C API 2/2.
  cargo fmt --all --check and git diff --check pass.
- Shared changes are narrow: the new Bytecode fields/cold attribute helper,
  evaluator bytecode helper, C API metadata access, and #98 collector edges.
  Main's PNode stays 48 bytes. passing.txt retains every main and PR entry.
- Still open: full pinned-platform CI; coordinator merge only. Then resume
  round 10's construction work on a fresh branch from merged main, adapting
  mkCode/bcClose to the lazy metadata API. #105 is saved and paused. Future
  import work can also defer unused string-vector containers; the remaining
  scalar/encoding paths currently keep their existing ReadItem behavior.

- Final infrastructure rebase: 5bff0caf (#111/#112 marker migration and #113
  methods activation) changes no runtime, build, library, benchmark or perf
  sources relative to b7ee268. Preserve main's markers and convert all four
  PR additions to markers; no legacy list is resurrected or entry removed.
  scripts/ratchet.py check passes: 32,230 cases, including 275 R scripts.
- Rebuilt candidate after rebase is byte-identical to the measured binary:
  SHA256 a923370fcadcd6216ac50ea241ab838627a57ca5879830f9791f62fa45694e09.
  Repeat the full seven-case paired run on the rebased tree against the
  identical b7ee268 runtime base. Final cache-OFF minima (two per side):
  rbench/awf/Bounce: 619,709,501 -> 619,311,140 (-0.0643%).
  rbench/shootout/nbody_2: 868,209,053 -> 868,363,620 (+0.0178%).
  rbench/shootout/nbody_3: 967,690,636 -> 966,979,020 (-0.0735%).
  rbench/shootout/nbody_naive_2: 1,258,885,224 -> 1,260,347,012 (+0.1161%).
  rbench/simple/bytecodes: 231,951,512 -> 230,962,117 (-0.4266%).
  rbench/simple/profiler-shared: 5,769,773,754 -> 5,761,492,229 (-0.1435%).
  robigo/fib: 957,419,844 -> 959,025,225 (+0.1677%).
  All seven meet <=0.2%; zero broken cases. Worst delta +0.1677%.
  Every one of the 275 markers passes in all three tiers; fixture pair 6/6.
- The new marker-based CLI conformance runner passes all seven release tests
  for the serialization filter, including both bytecode scripts. The debug
  CLI attempt fails before tests while linking native graphics symbols
  REAL0, INTEGER0, Rf_reEnc3 and mbcsTruncateToValid; no graphics files changed.
  Runtime/C API unit and metadata checks above remain valid (same sources).
- With the activated methods oracle, serialize/unserialize is still 317/322;
  only the previously reproduced main timeout for the huge compact sequence
  is listed as a regression locally. No expectations or masks are adjusted.
  Full pinned-platform corpora and perf gates remain CI's responsibility.

## 2026-10-30: reproduce cc2bb64's remaining CI performance failures

- Prior local <=0.2% claim did not reproduce the gate. CI sets ROBIGO_NATIVES
  to its built native libraries; the local isolated binary run omitted that
  override. Additionally, host-linked binaries require GLIBC_2.44 and cannot
  stand in for trixie's binaries. Withdraw that local result as gate evidence.
- Exact failing base is 5bff0caf, run 41031760764. Downloaded both exact
  binaries, native libraries and retained profiles from run 41038742565.
  Replayed the unchanged paired harness in robigo-oracle:4.5.0, both caches
  disabled and both executables using the same candidate natives, matching CI.
  fannkuchredux +0.576%, naive_2 +0.702%, profiler-shared +0.297% on this CPU.
- CI's naive_2 delta is 11,387,976 instructions. Exclusive increases include
  _mi_malloc_generic 5,440,874; mi_find_page 4,374,147; memcpy 1,122,617.
  No bytecode materializer appears in that profile; vm.rs does not access
  GNU metadata caches. The two proposed hot-access hypotheses are unsupported.
- Experiment: defer unused STRSXP containers and CHARSXP decoding. Validate
  headers/lengths without allocating R strings; fall back to ReadItem for
  attributed CHARSXP children. Replay canonical encoding flags and byte-order
  headers; materialize through the same RString constructor on inspection.
  GNU-recorded new fixture covers UTF-8, latin1, bytes, NA, empty/named vectors
  and native/XDR round trips. Six metadata tests and all prior crate checks pass.
  Container screen: fannkuchredux -0.199%, naive_2 +1.071%, profiler -0.030%.
  Reject as final: descriptor allocations worsened naive_2.
- Next inline descriptors without attributes into existing cell words; keep
  the existing attributed descriptor path and GC ownership unchanged. This
  removes small retained descriptor allocations, rather than adding padding
  or changing allocator/test settings. Every marker still passes in all three
  tiers and the two original bytecode fixture scripts remain 6/6 in trixie.

- Inline-descriptor screen also fails naive_2: +1.062%; fannkuchredux
  -0.200%, profiler-shared -0.029%. Revert both runtime experiments. Keep
  only the GNU-recorded encoding fixture and extra round-trip coverage.
  No runtime/perf improvement is claimed for this push. #100 remains blocked
  on performance; round 10 remains paused.

### Proposed split, pending a default-behavior decision

- Cargo feature `gnu-bytecode-fidelity`, off by default. Compile out the
  Bytecode variant, Closure metadata field and collector metadata owners in
  the default build, and use main's expression-only ReadBC path. Merely
  suppressing metadata allocation would leave the changed closure layout.
- Feature-enabled builds retain the full current serialization behavior,
  exact GNU fixtures and all existing marker entries. CI must run the whole
  conformance suite with the feature enabled, including the unchanged 6/6
  byte comparisons and native/XDR encoding tests. The default binary gets
  its own paired perf gate against an identically configured first parent.
- This deliberately leaves compiled-closure serialization fidelity opt-in.
  It is a policy compromise, not an achieved zero-cost implementation, and
  should be explicit before changing the default or test configurations.
- The alternative needs startup coordination: an owned serialized-input API
  and validated raw bytecode spans/reference handles, avoiding eagerly built
  replay vectors and most cold allocations. The current borrowed-slice API
  cannot keep input spans alive after return. That work crosses the rdb/input
  ownership boundary; it has not been implemented or measured here.
- Reproduction artifacts: CI run 41038742565 / job 124010650025, base build
  run 41031760764 at 5bff0caf, artifact hashes recorded in the retained
  binaries.json. Run scripts/perf-paired.py inside robigo-oracle:4.5.0 with
  ROBIGO_NATIVES pointing to the downloaded candidate/natives on both sides;
  leave ROBIGO_RDBFLAT unset. Local profiles and commands are retained under
  .robigo/perf100/ci2. No fixture output, mask or ratchet was removed.


## 2026-10-30: approved opt-in bytecode fidelity

`bytecode-fidelity` is a Cargo feature, off by default. Default builds use
main's expression-only GNU bytecode reader and interpreter fallback. They
compile out the Bytecode value variant, closure metadata field and collector
metadata owners, restoring the original layouts as well as import work.
No partial metadata preservation is retained in default builds without a
measured benefit.

**Default builds do not round-trip GNU R bytecode byte-identically.** They
retain and execute the source expression, but discard GNU instructions,
constant-pool layout and shared-cell replay metadata. There is no retained
BCODESXP owner for attributes: the expression-only reader can attach them
to the source expression instead. Re-serializing an imported compiled closure writes its expression body in
place of BCODESXP; serializing an imported standalone BCODESXP writes the
expression. The divergence concerns serialized bytes; execution continues
through the existing interpreter fallback. This is an explicit temporary
serialization conformance gap, not a zero-cost fidelity implementation.

Enable `cargo build --release -p robigo-cli --features bytecode-fidelity`
to preserve those bytes. The feature retains the unchanged oracle fixtures
and ratchet markers. `.requires` sidecars select the feature build for the
two fidelity scripts; default CI runs all other scripts. A dedicated trixie
CI job runs the metadata/encoding tests and the two scripts in all three
tiers (6/6 comparisons). Unknown feature requirements fail loudly.

The split is necessary because CI at cc2bb64 measured fannkuchredux +0.58%,
nbody_naive_2 +0.91%, profiler-shared +0.59%. The retained metadata changes
allocator behavior even when never inspected; the rejected lazy-container
experiments did not remove that cost. Shipping that tax for opt-in
serialization use was unacceptable.

Retire the flag after coordinating an owned serialized-input API with the
startup track: validate and retain raw bytecode spans and reference handles
without constructing Rust replay vectors, then decode only when forced or
re-serialized. Prove byte-identical native/XDR round trips and <=0.5% on
every affected benchmark with both exact binaries, caches disabled and the
same native libraries, before restoring fidelity to default builds.
Round 10 mkCode/bcClose work remains behind this same feature.


- Initial default trixie release verification: runtime 57 unit + 3 integration,
  C API 9 tests, CLI 9 conformance/selection tests all pass. This includes
  the explicit default expression/serialization-divergence test. All 32,230
  markers remain. These results precede merging main ef493176 (#116).
- Merge ef493176 intact, including physical reclamation, encoding marks,
  methods invalidation and all new markers. Resolve only the reachability
  Cell/OnceCell import conflict and initialize the guarded metadata field in
  core's new test closure. No legacy lists are restored. Measurements and
  final verification must use this merged tree.


### Matched default-feature result after merging main

Main advanced once more to a142ca65 (#117's JIT invalidation fix). Merge it
unchanged and freeze that commit for both sides. Build base a142ca65 and
candidate 7fad78f5 in robigo-oracle:4.5.0 with Rust 1.99.0 and the same private
Cargo target, sequentially. Run the unchanged scripts/perf-paired.py against
the two saved exact binaries: two samples each, caches disabled, isolated
executable directories, the same downloaded CI natives on both sides, and
GNU R output checks. This replaces the earlier unmatched local evidence.

| Benchmark | Main instructions | Default feature instructions | Change |
| --- | ---: | ---: | ---: |
| rbench/awf/Bounce | 620,888,075 | 620,855,834 | -0.0052% |
| rbench/shootout/fannkuchredux | 571,465,189 | 571,424,643 | -0.0071% |
| rbench/shootout/nbody_2 | 869,290,969 | 869,273,305 | -0.0020% |
| rbench/shootout/nbody_3 | 969,065,278 | 969,030,640 | -0.0036% |
| rbench/shootout/nbody_naive_2 | 1,260,971,232 | 1,260,964,169 | -0.0006% |
| rbench/simple/bytecodes | 232,710,173 | 232,687,949 | -0.0096% |
| rbench/simple/profiler-shared | 5,775,348,497 | 5,775,329,756 | -0.0003% |
| robigo/fib | 962,270,514 | 962,265,718 | -0.0005% |

All eight are flat within 0.01%; do not claim these small differences as
speedups. The unchanged harness reports 0 slower, 0 broken at 0.5%. The old
CI tax (0.58%, 0.91%, 0.59%) and this new local result have different base
commits/platforms; CI remains the final judge, not a claim that CI is green.
Base SHA256: 971004b1bc458d0cfc65b5529e4fbb63d5f5e4b2ff35485ddf5df02d5a012fe9.
Candidate SHA256: 7bf62d960db6bad46533d86a4691c059a262464b89ea32d1064279e739cec7cb.
Profiles, samples and environment manifest: .robigo/perf100/ci2/flag-default
and flag-manifest.json.

Merged default CLI verification: all 9 conformance/selection/tier tests
pass, all 281 active script ratchets hold (282/283 scripts match the oracle;
the sole mismatch is the known unratcheted compiled-isvector-context.R).
All 32,238 marker files remain. Add a nonempty-selection assertion so a
mistyped feature fixture filter cannot silently turn the CI job into 0/0.
Feature-enabled full release tests are compiling for this checkpoint; the
PR remains draft pending their result and the complete CI gate.


### Feature-enabled release verification

- In the same capped trixie container, cargo test --release for runtime,
  C API and CLI with bytecode-fidelity passes: runtime 64 unit tests
  (including core's physical reclamation tests and bytecode ownership),
  six metadata/encoding tests, two lazy-load tests; C API 9 integration
  tests; CLI 9 conformance/feature-selection/tier tests.
- Run the exact dedicated-CI fixture commands in interpreter, VM without
  JIT and VM with JIT: 2/2 each, **6/6** total. Oracle transcripts and every
  marker are unchanged. The entire feature-enabled script suite also holds
  its ratchets and agrees across all three execution tiers.
- Even the worst candidate sample against the best base sample in the
  eight benchmark pairs is only +0.0152% (fannkuchredux). The table above
  uses the official harness's minimum-of-two rule, not an average.
- cargo fmt --all -- --check and git diff --check pass. No default metadata
  partial optimization is claimed. The PR stays draft until the complete
  CI gate is green; coordinator merge precedes the fresh-main round 10
  mkCode/bcClose work behind bytecode-fidelity.
