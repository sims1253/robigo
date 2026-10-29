# conformance-sweep-data

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
