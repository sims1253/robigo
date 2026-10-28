# conformance-sweep-lang

## 2026-10-28: baseline and failure grouping

- Baseline: FastR requested filter 2,036/2,187, zero regressions; GNU R tests 18/59, zero regressions; base examples 252/381 with 15 pre-existing host failures. Saved results in `/tmp/sweep-{baseline,gnur-baseline,examples-baseline}.json`.
- Existing unrecorded passes: 43 FastR, two GNU R tests, 36 base examples. Keep these separate from implementation gains.
- Failure groups by first observed cause: class extraction strips class-vector attributes (12); dots element validation/error context (11, including a zero-argument panic); default warning visibility (12); attribute replacement (11 attrassign, five attributesassign, one attributes), including missing tsp validation, lost names-vector attributes, and language head tags; compact row-name printing (seven unclass, outside scope); parse/source metadata and diagnostics (12 parse, two SimpleParsing, parser changes outside scope); debugging/tracing placeholders (23 interactive, eight trace, seven tracemem, currently registered in unowned misc.rs); conditions (11); miscellaneous (nine); remaining language/environment/function/system/numerics failures (21).
- Known NextMethod missing supplied arguments also blocks `base/print`. Inspect `match.c:patchArgsByActuals` and preserve supplied order rather than rebuilding arguments in formal order.
- Base example host failures include installation paths, platform output, and missing R installation files. Never remove their ratchet entries; compare final results against this saved baseline.

## 2026-10-28: return explicit class vectors intact

- Port: `attrib.c:R_data_class` returns a nonempty explicit class vector without rebuilding it. Preserve its names and package attributes.
- Attribute slice: 262/300 -> 274/300, 12 gains, zero regressions. Recorded 12 gains plus 16 previously unrecorded baseline passes with `--update`. Kept.
- No changes to interp.rs or other shared evaluator paths.

## 2026-10-28: dots lookup and NextMethod supplied slots

- Port: `envir.c:do_dotsElt` / `ddfind` validates arity, index type/length, argument tags, and reports context errors for nonpositive/out-of-range indices. Register as an evaluated builtin per names.c (201), removing the zero-argument panic.
- Port: `match.c:patchArgsByActuals` / `patchArgument` patches supplied slots in original order. Retain explicit missing arguments and promise current local values, including exact and partial named matches.
- Dots/function slice: 342/359 -> 353/359, 11 gains and zero regressions. `base/print`: fail -> pass, including table subsetting with an explicit missing row index. Kept.
- Follow-up under verification: nested missing checks distinguish defaults from absent arguments per `envir.c:R_isMissing`; attribute and removal validation follow attrib.c/envir.c. `.dfltWarn` needs one minimal shared registration change in builtins/conditions.rs to match names.c eval=11.

## 2026-10-28: attribute validation and condition visibility

- Ports: `attrib.c:tspgets` validates numeric triples, positive frequency, row count and endpoint consistency; `do_attributesgets` rejects an empty first attribute name; `envir.c:do_remove` uses context errors and returns immediately for NULL names; `names.c` makes `.dfltWarn` visible.
- Final candidate run: 2,084/2,187, with one regression in cyclic defaults. The first nested-missing experiment removed the default flag but retained an arbitrary depth cutoff. Rejected that cutoff and replaced it with promise-identity cycle detection from `envir.c:R_isMissing`; final rerun pending.
- Base examples: 252/381 -> 257/381, five gains (`aperm`, `dots`, `marginSums`, `print`, `proportions`), no newly failing cases. GNU R tests: 18/59 unchanged. Recorded baseline passes separately through --update as well as gains.
- Related rep/names/S3/tsp slice: 347/363, zero ratchet regressions. Runtime tests: 23 unit + two integration pass. Full conformance: 171/172; only known datetime/zones.R host difference. Compiled guard and interpreted/compiled comparison tests pass. Formatting passes.
- Previous pushed commit 5a0383d passes all CI jobs, including four FastR shards and instruction-count performance checks. Final validation fixes are being verified separately.
