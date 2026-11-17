## 2026-11-17: S3x-d, the trycatch row against the stock rival

- What: S3x-d's first step, a measurement with no code change. The bar
  is trycatch at or below 1.0x GNU R's instructions (g3.md §15.3, "M3
  adds `tryCatch`"). The foreman asked for the gap to the stock rival
  before anything touches the call path, so that X/P1 could be lined up.
- Method:
  - The rival is `scripts/gnur-stock.sh`'s build of GNU R 4.6.2, made in
    the oracle image `oracle-4.6.2-49a8ee77bb6c46db` (static libR,
    `gcc -std=gnu23`, `-g -O2`), as CI's perf-rt job makes it.
  - Both sides ran in that image, so they share one CPU and one glibc
    (2.41): `scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml
    --cycles`, N against 2N, as rt-bars measures.
  - CPU signature `1be0f69f7d2b7252` (Ryzen 9 5900X).
  - robigo-rt is main's binary at `a9c4fda3`, sha256 `8c2f0fbd…`;
    S3x-a's binary measures the same to +0.05%.
- **The row is 1.38x.** robigo-rt takes 116,758 instructions per
  iteration and GNU R 84,911. Meeting the bar needs 31.8k fewer, about
  27%.
- **The error branch holds most of the gap.** The row alternates a value
  and an error. The new `conditions` rows split it into its branches and its
  neighbours. They are reported, and no bar cites them:

  | row | robigo-rt | GNU R | ratio |
  |---|---:|---:|---:|
  | `trycatch` (the bar's row) | 116,758 | 84,911 | 1.38x |
  | `trycatch-ok`: no error | 51,633 | 53,424 | 0.97x |
  | `trycatch-err`: every call raises | 179,760 | 126,672 | 1.42x |
  | `trycatch-bare`: the call without tryCatch | 1,227 | 2,150 | 0.57x |
  | `withcalling-ok`: a calling handler, no condition | 20,942 | 15,339 | 1.37x |
  | `try-err`: try(silent = TRUE) of an error | 510,182 | 484,131 | 1.05x |

  Without an error, tryCatch is already below the bar (1.8k less than
  GNU R). An error costs 53.1k more than GNU R's. Half of each, the
  row's mix, is 25.6k of the row's 31.8k.
  - The other 6.2k is not in either branch alone. GNU R's mixed row costs
    5.1k less than the mean of its two branches (84,911 against 90,048);
    robigo-rt's costs 1.1k more (116,758 against 115,697). Nothing here
    says why. X9 and X7 should measure the mixed row as well as the
    branches.
- **Where an error's instructions go** (callgrind self cost per error,
  `trycatch-err`):
  - The condition system is not the gap. GNU R's errors.c and context.c
    cost 2.5k per error, and so do robigo-rt's conditions and records.
  - The rest is the R code that `stop()`, `.makeMessage`, tryCatch's
    `tryCatchOne`/`doTryCatch` and the handler's call run. Each line
    sums the self costs of the functions that do that work (the data's
    tables); GNU R's builtins are its "other builtins", attrib.c and
    arithmetic:

    | | robigo-rt | GNU R |
    |---|---:|---:|
    | making and cleaning frames | 18.8k | 4.7k |
    | defining and looking up variables | 16.4k | 12.7k |
    | argument matching and frame arguments | 21.0k | 14.6k |
    | builtins on `stop()`'s path | 21.9k | about 11k |
    | the compile of a call made at run time | 4.2k inclusive | none |
    | allocation and collection (robigo-heap, malloc; memory.c) | 21.1k | 34.0k |
    | the evaluator (VM, bcEval) | 34.0k | 23.5k + inlined code |

  - The calls per error match GNU R's, but each call costs more:
    - closure calls: 15, plus 3 inlined, against 16;
    - `define_var`: 22 against `Rf_defineVar`'s 22, at 6,699 against
      1,610 instructions in all;
    - matched calls: 15 against 16, with `args::apply_with` at 16,772
      against `Rf_matchArgs_NR`'s 6,690.
  - robigo-rt allocates 96 objects (6,840 bytes) per error: 11 ENVSXP,
    20 LISTSXP, 15 LGLSXP, 13 VECSXP, 9 STRSXP, 9 INTSXP, 5 PROMSXP,
    5 DOTSXP, 4 CLOSXP, 3 RAWSXP and 2 LANGSXP.
- **D11's evidence.** An error compiles one call that is made at run time
  (`Comp::root` once, 4.2k inclusive). `eval::expr_code` takes 6.5k
  inclusive over its 8 calls per error, most of which find the cached
  code. GNU R evaluates such a call once as an AST. Whether run-once code
  should go to a minimal `eval` is D11's question (X7, the bar in
  `2026-11-17-m2m3-bars.md`); S3x changes no JIT policy. The first report
  to the foreman gave "up to about 12k" for this. That was an inclusive
  cost counted twice under recursion.
- **Who takes what** (the foreman, 2026-11-17):
  - S3x-d: `stop()`'s builtins on the error path: `do_subset2_dflt`,
    `do_makelist`, paste, `c`, `attributes<-`, `names`. These are
    port-faithful speedups. Each is measured on its own row and on
    `trycatch-err`, and no other perf-rt row may get worse.
  - X/P1, package X9: the frames, the variables and the argument
    matching, against GNU R per error:
    - frames: 14.1k more;
    - variables: 3.7k more in all. `define_var` alone is 5.1k more, and
      the lookups are 1.4k less;
    - argument matching: 6.4k more.

    The foreman's assignment quoted "frames and `define_var` (about 14k)"
    and "argument matching (about 6.6k)" from the first report. Those
    figures had two errors: the frames alone are 14.1k, and the argument
    matching is 6.4k. The raw callgrind files, the row scripts and
    the binaries for both sides are in
    `~/.cache/robigo-research/x9-trycatch/` (its README.md).
  - X7 (D11): the compile on a fresh call's first eval.
- Data (`data/2026-11-17-s3x-d-trycatch-gap/`):
  - `rows.json`: the table above, with the binaries and the environment;
  - `calls-per-error-{robigo,gnur}.txt`: calls, self and inclusive cost
    per error, the top 120 functions;
  - `identity.txt`: the image, valgrind, the CPU and both binaries.
