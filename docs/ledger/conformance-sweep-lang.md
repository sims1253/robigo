# conformance-sweep-lang

## 2026-10-28: follow-up delivery verification

- Final candidate 049e4ed reconfirms 2,093/2,187 in the requested FastR filter, zero regressions. Combined names/tracing slice is 258/272, nine gains overall. Base examples remain 257/381 and GNU R scripts remain 18/59; related handler cases preserve their previous pass/fail status while conditions advances to the filesystem dependency.
- Runtime tests: 23 unit + two integration pass. Language conformance: 31/31; vm_guards_hold and compiled_matches_interpreted both pass. Full conformance: 174/175, solely the pre-existing timezone mismatch with byte-identical main/candidate output. Formatting and whitespace checks pass.
- Three verified cycles delivered: names-label attributes, memory-tracing validation, and persistent global calling handlers. No remaining named group can become fully passing without the ownership dependencies below. Stop this batch with PR #30 open; do not merge. CI's timezone failure needs the datetime/runner owner, not a weakened expectation.

## 2026-10-28: global calling handler registration

- Port errors.c:do_addGlobHands into the existing condition internals. Reject registration with local handlers on the stack, install the new global chain, and update every saved context to prevent restoration of the old chain.
- First experiment installed the chain in closure contexts but eval_toplevel reset it before the next expression. Rejected that incomplete approach. Keep a separate top-level HandlerStack on Interp and restore it at each top-level boundary, matching R_ToplevelContext->handlerstack.
- global-handlers.R changes from an immediate failed assertion to a byte-identical match with Docker GNU R 4.5.0. It covers persistence, class selection, multiple handlers, local-handler precedence, removal, duplicates, closure attributes/environments, and rejected registration with local handlers. Kept.
- GNU R conditions now completes all global handler assertions and reaches its source-reference section, where file.info is unimplemented outside this track. Overall GNU R test passes remain 18/59. The affected base/conditions example remains passing; reg-tests-1d retains its pre-existing formals warning failure.
- Full conformance: 174/175, only datetime/zones.R failing. Three new scripts are recorded in the conformance ratchet. Dataframe instructions after all changes: 617,176,220 -> 617,223,145 (+0.0076%), inside the 0.5% gate.
- Shared changes: one Interp field, initialization, and top-level restoration; move .addGlobHands registration from misc.rs into builtins/conditions.rs. No VM/compiler/JIT or parser changes.

## 2026-10-28: CI timezone blocker and remaining ownership

- PR #30's build failed solely at tests/r/datetime/zones.R line 2040, matching the known local baseline failure. Saved origin/main bf985045 binary and candidate produce byte-identical stdout (589,363 bytes), stderr, and successful exit status on that script. Do not change the oracle, ratchet, CI, or datetime implementation from this track. The datetime owner must reconcile the runner tzdata/oracle difference before CI can be green.
- Requested FastR filter: 2,085/2,187 -> 2,093/2,187, eight gains, zero regressions. There is one additional namesassign gain outside the primary filter, nine FastR ratchet additions total. Base examples remain 257/381 with the same 15 baseline host failures; GNU R tests remain 18/59.
- Next ownership handoffs: VM/value-model owners for trace flags and duplication hooks; parser/CLI owner for literal <<<NEWLINE>>> diagnostics; package-native owner for methods initialization; filesystem owner for file.info; printing/core owners for language head tags and compact row-name printing. After these dependencies, resume parse-data/source-reference examples and the remaining global-handler regression script.

## 2026-10-28: memory tracing validation

- Port debug.c:do_tracemem, do_untracemem and do_retracemem validation, including check1arg tags, function/environment/NULL rejection, previous-argument matching, and diagnostic order. Move these three registrations/functions from shared misc.rs into owned builtins/debug.rs; register the module in shared builtins/mod.rs.
- Names/tracing slice: 255/272 -> 258/272 after the names fix; compared with original main, 249/272 -> 258/272. The tracing group itself improves 1/8 -> 4/8, three gains, zero regressions. Kept.
- Both focused scripts match Docker GNU R 4.5.0. Runtime tests pass (23 unit, two integration). Full conformance is 173/174, with only the known datetime/zones.R host mismatch; both compiled checks pass. New scripts added through ROBIGO_CONF_UPDATE=1.
- Dataframe instruction count: 617,176,220 -> 617,164,203 (-0.002%); perf.py compare reports no slowdown above 0.5% or broken answers.
- Valid memory tracing requests report an explicit unimplemented-feature error. Duplicate reporting needs allocation/copy hooks in value.rs and replacement builtins outside this track. No fabricated object identities or trace output.
- Basic function tracing also needs coordination with the VM owner: vm.rs can inline pure closure calls before interp.call_closure, so an evaluator-only trace hook would silently miss calls. Tracer-expression cases additionally need methods native initialization. No partial tracing engine was added.
- GNU R tests triage: conditions stops at globalCallingHandlers signaling (the existing .addGlobHands placeholder in misc.rs); later source-reference checks need file.info and parser source metadata. reg-tests-1d needs formals warning behavior in funs.rs, reg-tests-1e needs env.profile object hash state, method-dispatch needs S4 class handling. Other first failures are primarily unimplemented package natives, external tools, or printing/strings.

## 2026-10-28: follow-up baseline and names attribute coercion

- Branch starts at origin/main bf985045, after PR #27 merged. Rebuilt baseline: FastR 2,085/2,187, zero regressions; GNU R tests 18/59, zero regressions; base examples 257/381, with the same 15 host failures. Saved binary and results under `.robigo/lang-debug/`.
- Reclassified debugging: nearly all InteractiveDebug failures contain literal <<<NEWLINE>>> and fail before debugging starts; the mismatch is parser/CLI context formatting. Trace cases with tracer expressions need methods native initialization. The eleven getParseData parse cases need methods initialization, and the twelfth parse case fails at file.info. Runtime parse-builtin edits do not unblock these cases.
- Names port: attrib.c:namesgets preserves the coerced labels' attributes; builtin.c:xlengthgets keeps only names when extending a short label vector. Atomic labels use coerceVector rather than factor dispatch. Pairlist labels follow isVectorizable in Rinlinedfuns.h.
- Names/attribute slice: 248/264 -> 254/264, six gains, zero regressions. Five gains belong to the primary filter; namesassign/99c388871ae85598 is an additional gain outside it. Kept.
- names-attribute-values.R matches Docker GNU R 4.5.0 byte for byte. Crate tests and final corpus verification are in progress.

## 2026-10-28: final verified language metric

- FastR requested filter: **2,036/2,187 -> 2,085/2,187**, 49 implementation gains, zero regressions. All 26 missing() cases pass, including forwarded defaults and cyclic defaults. The initial depth-cutoff experiment was rejected; promise-identity cycle detection restores the existing cyclic-default pass.
- Ratchet additions: 92 FastR entries, comprising 49 gains and 43 pre-existing unrecorded passes; 41 base examples, comprising five gains and 36 baseline passes; two pre-existing GNU R regression-script passes. No entries removed.
- Base examples: **252/381 -> 257/381**, no newly failing cases. GNU R tests: **18/59 -> 18/59**, zero regressions. The same 15 host-dependent base example failures remain.
- Shared change: only the `.dfltWarn` visibility registration in builtins/conditions.rs. No edits to interp.rs, VM/JIT/compiler, parser, value model, printing, coercion, or vendored R code.
- Reuse s::TSP in attribute validation to avoid interning on each attribute assignment.
- Final reruns: runtime 23 unit + two integration pass; conformance 171/172 with only datetime/zones.R, both compiled checks pass; all three corpus metrics above reconfirmed after the cycle fix. cargo fmt --check and git diff --check pass. Final release build succeeds.

### Remaining work

- Debugging/tracing: 38 cases (23 InteractiveDebug, eight Trace, seven tracemem); implementations are currently placeholders in unowned builtins/misc.rs. Move them into the owned debug module with a minimal registration change before porting debug.c.
- Parser/source behavior: 12 parse, two SimpleParsing, and eleven ConditionHandling cases. The latter contain literal <<<NEWLINE>>> and differ in unexpected-input context. Hand diagnostics to the parser owner; port source metadata in language builtins where feasible.
- Attributes: eight attrassign cases need names-vector attribute preservation or language head tags; two classassign cases and seven unclass cases need printing/object-model work. Preserve names-vector attributes in attrib.c:namesgets without changing coercion-owned paths; coordinate head-tag representation with the core owner.
- Other remaining cases: nine MiscBuiltins, five functions, two rmultinom, two system timeouts, and one each environmentassign, exists, gettext and switch. The numeric/system/startup/S4 cases require other owners. Inspect switch and function failures individually before the next language batch.

## 2026-10-28: attribute validation and condition visibility

- Ports: `attrib.c:tspgets` validates numeric triples, positive frequency, row count and endpoint consistency; `do_attributesgets` rejects an empty first attribute name; `envir.c:do_remove` uses context errors and returns immediately for NULL names; `names.c` makes `.dfltWarn` visible.
- Final candidate run: 2,084/2,187, with one regression in cyclic defaults. The first nested-missing experiment removed the default flag but retained an arbitrary depth cutoff. Rejected that cutoff and replaced it with promise-identity cycle detection from `envir.c:R_isMissing`; final rerun pending.
- Base examples: 252/381 -> 257/381, five gains (`aperm`, `dots`, `marginSums`, `print`, `proportions`), no newly failing cases. GNU R tests: 18/59 unchanged. Recorded baseline passes separately through --update as well as gains.
- Related rep/names/S3/tsp slice: 347/363, zero ratchet regressions. Runtime tests: 23 unit + two integration pass. Full conformance: 171/172; only known datetime/zones.R host difference. Compiled guard and interpreted/compiled comparison tests pass. Formatting passes.
- Previous pushed commit 5a0383d passes all CI jobs, including four FastR shards and instruction-count performance checks. Final validation fixes are being verified separately.

## 2026-10-28: dots lookup and NextMethod supplied slots

- Port: `envir.c:do_dotsElt` / `ddfind` validates arity, index type/length, argument tags, and reports context errors for nonpositive/out-of-range indices. Register as an evaluated builtin per names.c (201), removing the zero-argument panic.
- Port: `match.c:patchArgsByActuals` / `patchArgument` patches supplied slots in original order. Retain explicit missing arguments and promise current local values, including exact and partial named matches.
- Dots/function slice: 343/359 -> 353/359, ten gains and zero regressions. The eleven additions to passing.txt include the pre-existing unrecorded functions/TestFunctions/c1681fdbc9589a91 pass. `base/print`: fail -> pass, including table subsetting with an explicit missing row index. Kept.
- Follow-up under verification: nested missing checks distinguish defaults from absent arguments per `envir.c:R_isMissing`; attribute and removal validation follow attrib.c/envir.c. `.dfltWarn` needs one minimal shared registration change in builtins/conditions.rs to match names.c eval=11.

## 2026-10-28: return explicit class vectors intact

- Port: `attrib.c:R_data_class` returns a nonempty explicit class vector without rebuilding it. Preserve its names and package attributes.
- Attribute slice: 262/300 -> 274/300, 12 gains, zero regressions. Recorded 12 gains plus 16 previously unrecorded baseline passes with `--update`. Kept.
- No changes to interp.rs or other shared evaluator paths.

## 2026-10-28: baseline and failure grouping

- Baseline: FastR requested filter 2,036/2,187, zero regressions; GNU R tests 18/59, zero regressions; base examples 252/381 with 15 pre-existing host failures. Final results and preserved baseline slices are in `.robigo/lang-sweep/`.
- Existing unrecorded passes: 43 FastR, two GNU R tests, 36 base examples. Keep these separate from implementation gains.
- Failure groups by first observed cause: class extraction strips class-vector attributes (12); dots element validation/error context (11, including a zero-argument panic); default warning visibility (12); attribute replacement (11 attrassign, five attributesassign, one attributes), including missing tsp validation, lost names-vector attributes, and language head tags; compact row-name printing (seven unclass, outside scope); parse/source metadata and diagnostics (12 parse, two SimpleParsing, parser changes outside scope); debugging/tracing placeholders (23 interactive, eight trace, seven tracemem, currently registered in unowned misc.rs); conditions (11); miscellaneous (nine); remaining language/environment/function/system/numerics failures (21).
- Known NextMethod missing supplied arguments also blocks `base/print`. Inspect `match.c:patchArgsByActuals` and preserve supplied order rather than rebuilding arguments in formal order.
- Base example host failures include installation paths, platform output, and missing R installation files. Never remove their ratchet entries; compare final results against this saved baseline.
