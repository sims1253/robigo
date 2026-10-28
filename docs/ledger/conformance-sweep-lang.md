# conformance-sweep-lang

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
