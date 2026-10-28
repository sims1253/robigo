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
