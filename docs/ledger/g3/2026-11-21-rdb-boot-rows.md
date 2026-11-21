# The rdb boot's extra cost on `s3` and `e7-match`, found and fixed (2026-11-21)

X/P1, for L's #942 (`g3/l5-compiler-rdb`), which boots base from its
lazy-load database (`ROBIGO_RT_BOOT=rdb`). There `s3` measured +5.25% and
`e7-match` +5.17% against the interim loader. The callgrind files and
scripts are in `~/.cache/robigo-research/x-boot/rows/` (`FINDINGS.md`).

## Method

- One binary for both boots, so the difference is the boot and nothing else.
- Before: `robigo-rt-cl-686217ab`, #942's head 686217ab4, sha256
  `150fa2a92febed3b5731ba67b8d589437c53d29d205a43528da37a1801f1f06e`.
- After: `robigo-rt-sh-77cd8f01`, 686217ab4 with this change, sha256
  `f60f36e4feb9bc834eac6c4193906cf5a9a5184811971bf040b0a405e4e73ece`.
- Each was built from its exact tree into an empty target.
- Each row ran as the harness runs it (`g <- function() { for (i in 1:N)
  { BODY }; NULL }; g()`), at N and 2N under callgrind. A figure is
  instructions per iteration, (2N − N) / N, compared function by function.

| row | m1 before | rdb before | m1 after | rdb after |
|---|---|---|---|---|
| `s3` | 16,947 | 17,783 | 16,857 | 16,925 |
| `e7-match` | 91,941 | 96,514 | 91,961 | 91,922 |
| `e7-unique` | 117,862 | 118,546 | 117,853 | 117,981 |
| `guard-vec-arith` | 88,185 | 89,571 | 88,186 | 89,573 |

## `e7-match`: base's `match` was not inlined under rdb

- The JIT compiles with robigo's port of cmp.R (`compile_closure_bc`). Its
  `is.simpleInternal` (`cenv::simple_internal`) read the closure's body slot
  and required a call.
- The database installs base's closures with byte code there. So the
  inline of `match` (a safe base internal) was refused, and every iteration
  made a real closure call: one closure frame and one `vm::run` more.
- cmp.R reads `body(def)`, which is `R_ClosureExpr`, the byte code's
  source, so GNU R does inline it. `compiler::cmpfun` inlined it under both
  boots.
- The fix reads that source. `tests/simple_internal.rs` judges closures
  with byte-code bodies against their language-bodied twins. No R-level case
  shows the change: an error from the inlined call reports the same call as
  one from the closure.
- The other closure-body reads in the compiler port were searched (`.cdr()`
  and `CLOSXP` in `compile.rs`, `cenv.rs`, `compile/*.rs`). They are
  `source_body` and the BCODESXP branches of `compile_closure_bc`, of the
  gap walk and of the crosscheck, and all handle byte code. Of the
  functions the compiler finds by name, only `simple_internal` read a body:
  the constant fold checks only the type.

## `s3`: `topenv`'s shape cache was full before the row ran

- `topenv` runs three times per iteration under both boots. It cost 431
  instructions per call under rdb against 173 under the interim loader.
- `passed_by_shape` remembers 16 frame shapes and stops adding when full.
  The rdb boot fills it (16 entries against 5), so the row's shapes never
  got in, and each call searched 16 globals and then read the shape.
- A cap of 64 was worse (19,130 per iteration), because each miss is
  longer.
- Eviction is not possible: releasing a global takes `&mut Cx`, and
  `topenv` has `&Cx`. A cache that does not root its shapes is unsound
  under the moving collector.
- The fix: a shape of up to 8 symbols is read directly, which costs less
  than the search.
- What is left: a cache that stops adding when full keeps the first long
  shapes a session meets, and a long-running session meets far more than a
  boot. Every long shape met after it filled is searched for and read at
  every call.
- The complete fix is a flag set on a shape when it is made. It needs a bit
  in robigo-heap and is proposed to H (`delegates/H.md` §23).

## What remains, and whose it is

- `s3`, +68 per iteration: S1's method and dispatch memos (`memo_get` +45,
  `search_methods` +32, `same_dispatch` +14). They are 4-way
  set-associative, with the set picked by symbol id. Under rdb the symbols
  have other ids and the boot leaves more entries behind, so a hit scans
  more of a full set.
- `e7-unique`, +128: the same memos (`memo_get` +57, `method_shape` +34,
  `quick_search` +30). Before this change it was +684; the rest was
  `topenv`.
- `guard-vec-arith`, +1,387: the collector.
  - One collection falls between N and 2N. It costs 8.67M under rdb against
    3.02M, and `trace` 4.05M against 0.37M.
  - A collection marks the whole heap, and the rdb boot leaves more live
    objects to mark (all of base's byte code and the compiler).
  - This is the collector's cost of the larger heap (H, A1), not a dispatch
    or compiler cost.
