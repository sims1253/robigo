# packages ledger

The most used CRAN packages' own test suites on robigo, scored against GNU
R 4.5.0 (`scripts/package-suite.py`, `docs/verification.md` § Top CRAN
packages). Metric: test blocks whose outcome matches GNU R's, and packages
fully green.

## 2026-11-03: the harness and the baseline

- Selection at PPM snapshot 2026-11-01: 20 packages over 900,000
  downloads in 30 days and 500 packages using them. These are CRAN's
  index plus cranlogs, checked against the R Observatory's data release
  of 2026-11-02, which gives the same 20. curl is at the line (502).
  knitr (490) and RColorBrewer (467) are just outside.
- Tree: 221 packages, the 20 plus their Suggests and dependencies, pinned
  in `rv.lock`. rv installs them from PPM's trixie binaries in 90 s. Both
  GNU R and robigo load those binaries.
- Council, round 1 (Opus 5.5, GPT-6.1 Sol, Gemini 4 Argon; Muse Wonder
  could not be seated, its provider is disabled in T3):
  - Real testthat, not the shim.
  - One process per test file, streaming events.
  - Score test blocks against GNU R; a canary.
  - `NOT_CRAN=true` and no network.
  - Nightly, not gating merges.
  - Examples and vignettes left out.
  - Sol wanted the source-built tier to be the headline. The other two put
    the binary tier first with source installs beside it, and that is
    what shipped.
- What the council's checks of the first spike found to be harness
  artifacts:
  - A global variable of the runner broke a tidyselect test.
  - Serializing results through jsonlite (S4 dispatch) and through
    `as.data.frame.testthat_results` lost runs that had finished.
  - Rcpp's tests need `RunAllRcppTests=yes`.
  - sf and friends needed their system libraries in the image.
  - With all of these fixed, GNU R records clean: 37 of about 7,100
    blocks fail under GNU R itself (snapshot backtraces), 62 are fully
    skipped, and 7 differ between the three recordings.
- `NOT_CRAN=true` roughly triples cli's covered blocks: a CRAN-style run
  skips every snapshot test.
- Baseline on main 732a179b (#265 fixed testthat's cloned namespaces;
  without it the canary fails, 8 of 9):
  - 5,222 of 8,617 scored blocks match GNU R (60.6%).
  - 0 of 20 packages are green; canary 9 of 9; 42 of 42 packages of the
    dependency tree load.

| package | matching | share | files |
|---|---|---|---|
| httr | 59 / 59 | 100.0% | done 20 (not green: GNU R skips 34 network tests, under the 80% floor) |
| Rcpp | 1,628 / 1,629 | 99.9% | done 53 |
| stringr | 211 / 212 | 99.5% | done 30 |
| glue | 110 / 111 | 99.1% | done 8 |
| scales | 215 / 218 | 98.6% | done 39 |
| htmltools | 105 / 112 | 93.8% | done 10, timeout 1 |
| magrittr | 21 / 24 | 87.5% | done 9 |
| R6 | 63 / 73 | 86.3% | done 12 |
| tibble | 233 / 290 | 80.3% | done 27, oom 1, timeout 4 |
| tidyselect | 152 / 219 | 69.4% | done 10, timeout 6 |
| RcppEigen | 86 / 131 | 65.6% | done 5, setup 2 |
| rlang | 910 / 1,425 | 63.9% | done 40, timeout 19, crash 2 |
| cli | 425 / 689 | 61.7% | done 52, timeout 20 |
| dplyr | 571 / 1,341 | 42.6% | done 38, timeout 27, budget 13, crash 1 |
| purrr | 188 / 452 | 41.6% | done 42, timeout 8, abort 3 |
| jsonlite | 20 / 56 | 35.7% | done 30 |
| tidyr | 177 / 692 | 25.6% | done 15, timeout 14, crash 2 |
| lifecycle | 8 / 38 | 21.1% | done 8, timeout 1 |
| ggplot2 | 40 / 846 | 4.7% | done 4, oom 119 |
| curl | 0 / 0 | | setup 22: its helper needs a live HTTP test server |

Causes, most blocks first:

1. Files over their time limit (1,267 blocks). robigo is slow on these
   suites' failure paths: the same file is 10-40× GNU R's time, and a
   failing block costs seconds where a passing one costs milliseconds.
   This is not evaluation speed, which is close to GNU R on the passing
   blocks.
2. Files over 3 GB (832 blocks, ggplot2 and tibble). The cap is right:
   - One ggplot2 test file peaks at 0.14 GB under GNU R. Under robigo it
     passed 5 GB in 17 s.
   - robigo is already at 1 GB when a ggplot2 test starts.
   - Each of rlang's per-frame C calls in `trace_back()` (one
     `rlang::abort()` inside a test) adds about 100 MB, and `gc()` adds
     more instead of freeing any.
   - This is bridge memory: what the bridge holds, and when it lets go.
     It needs its own design round.
3. `.x[[i]]`: "attempt to select less than one element" (382 blocks,
   purrr, tidyr, dplyr). purrr's map writes its index into an argument
   before every callback. robigo copied arguments to C and back only when
   the call returned. See the next entry.
4. Crashes (94), a `vm.rs` index-out-of-bounds panic (21, purrr), and
   `R_check_class_etc` not implemented (45, RcppEigen through Matrix).
5. S4 on jsonlite's generics (34): "no applicable method for `@`
   applied to an object of class nonstandardGenericFunction".
   testthat's `env_clone()` of the namespace passes every closure through
   C, and the bridge's proxy of a closure lacked the S4 bit; reading the
   proxy back cleared it on the generic itself. Fixed by
   `track/packages-capi-s4-bit` (capi case `s4bit`): jsonlite 56 / 56,
   green.
6. Found while bisecting, not yet counted:
   - ggplot2's `+` fails on robigo with "Incompatible methods
     (Ops.S7_object, +.gg)"; GNU R 4.5's `chooseOpsMethod` resolves it.
   - withr's global `defer()` fails with "could not find function
     deferred_run".
   - In rlang, 96 blocks are missing from files that did finish: their
     control flow differs from GNU R's.

## 2026-11-03: zero-copy atomic arguments (council round 2)

- purrr's `i` reaches the C loop through `cleancall_call`: it goes in a
  pairlist, then back into R in a call that C builds. The bridge's
  `from_sexp` copied it on the way back, so C wrote into a copy of a copy.
  dplyr writes the group id into a vector it finds in the data mask with
  `env_get`.
  - A first fix synced vectors that C binds with `Rf_defineVar` at each
    callback. It is correct for that shape and gave purrr 187 → 191 and
    tidyr 174 → 179, because purrr does not use that shape. Not shipped.
- Council (Opus, Sol, Gemini; Muse disabled):
  - Sol and Gemini: track the writable pointers C takes, and sync those.
  - Opus: zero-copy EXTDATA arguments, the design docs/core.md §6
    describes.
  - Pointer tracking loses its point because rlang and vctrs read
    through the writable `INTEGER()` and `REAL()` (`rlang/vec.h`): it
    would track nearly everything and compare whole columns at every
    callback.
  - Max chose zero-copy, which reverses council item 22.

## 2026-11-04: on more machines (boat.dev VMs)

- The same main binary scores 5,729 of 8,617 blocks on an 8-vCPU VM, against
  5,222 on the 2-core dev machine. Time limits scale with GNU R's times
  recorded on the dev machine, so the faster machine loses fewer files.
  Decision: a score is compared only with scores from the same kind of machine
  (docs/verification.md). The nightly always uses one runner type.
- Harness fixes found there, all on this branch:
  - `--out` outside the checkout and the cache is mounted.
  - A robigo from another checkout gets its R_HOME's headers mounted too;
    without them, every test that compiles C++ at run time failed.
  - `ROBIGO_PKG_JOBS` caps the workers.
  - `ROBIGO_PKG_KEEP_LOGS` keeps each worker's output.
- tidyr's `test-pivot-wide.R` segfaults on main every time (#167).
- Issues filed from the suite: #302 to #306 and #310. #303's fix is zero-copy
  bridge arguments (local branch `track/bridge-zero-copy`): net +275 blocks,
  dplyr +245, but it reaches #167's crash sooner in tidyr, so it waits on
  #167.

## 2026-11-04: a local httpbin for curl; httr stays under the floor

- curl's tests run against a local httpbin: python3-httpbin 0.10.2, pinned
  in `apt.txt`, started on each worker's loopback before R
  (`tests/packages/README.md`, Test servers). curl's helper probes
  `http://httpbin.org` whatever `HTTPBIN_TEST_SERVER` says, because
  `Sys.getenv` forces its default. So the server answers as httpbin.org, which
  the oracle maps to 127.0.0.1 (`--add-host`; a mount for each worker's own
  hosts file is refused by Docker's AppArmor profile).
- GNU R, three recordings, all identical: curl 80 blocks, 73 scored (91%),
  22 files done. The 7 unscored blocks name hosts of the internet
  (letsencrypt.org, köln.de, hetzner) or need Windows.
- robigo on main bf7332ee (4-vCPU VM): curl 50 / 73, files done 22.
  - `R_new_custom_connection` is a stub (12 blocks): #343.
  - C finalizers of handles made inside `test_that()` do not run on `gc()`
    (9 blocks): #344. test-multi's "Data callback" fails there too, with
    ``could not find function "{"`` (1 block).
- httr: GNU R records the same 96 blocks as before (59 scored, 61%), and
  robigo matches all 59. A test server cannot lift httr over the 80% floor:
  - 33 of the 37 unscored blocks skip unconditionally in httr's own
    source: `skip_httpbin()` is `skip("httpbin is not reliable")`, plus
    one `skip("Currently broken")`.
  - One more is `skip_on_ci()`, and the remaining three name hosts of the
    internet (google.com, firefox.com, hb.cran.dev, api.figshare.com).
  - Whether skips the authors always take belong in the floor's
    denominator is a question about the metric, left open.

## 2026-11-04: skips of the package's own leave the floor (httr green)

- Coordinator's decision, option (A): `tests/packages/own-skips.tsv` lists,
  for each package and version, the skip reasons its own code always takes,
  each with the source line that makes it unconditional. A block GNU R
  records as skipped, steadily and only for listed reasons, leaves the 80%
  floor's denominator; robigo must still skip it the same way. Environment
  skips (`skip_on_ci()`, missing Suggests, hosts of the internet) are not
  listed.
- httr 1.4.9: `skip("httpbin is not reliable")` (helper.R:1, `skip_httpbin()`)
  and `skip("Currently broken")` (test-header.r:17), 33 of 96 blocks. The
  floor becomes 59 of 63 (94%). On main dbe74cda (4-vCPU VM): httr 59 / 59,
  files done 20, nothing diverged, **green**. curl unchanged at 50 / 73.

## 2026-11-05: round 1 of the campaign, 9 of 20 green

Main 47ccdf61 on one 8-vCPU boat VM, 4 jobs (`scripts/package-suite.py run`,
all 20 packages, canary 9 / 9): **7,342 of 8,693 blocks match GNU R (84.5%),
9 of 20 packages green.** The baseline was 5,222 and 0 (on the dev machine;
scores depend on the machine, see 2026-11-04).

| package | matching | files |
|---|---|---|
| Rcpp | 1,629 / 1,629, green | done 53 |
| RcppEigen | 131 / 131, green | done 7 |
| jsonlite | 56 / 56, green | done 30 |
| lifecycle | 38 / 38, green | done 9 |
| magrittr | 24 / 24, green | done 9 |
| scales | 218 / 218, green | done 39 |
| stringr | 212 / 212, green | done 30 |
| tibble | 290 / 290, green | done 32 |
| httr | 59 / 59, green | done 20 |
| htmltools | 111 / 112 | done 11 |
| glue | 112 / 114 | done 8 |
| tidyselect | 217 / 219 | done 16 |
| cli | 677 / 689 | done 72 |
| R6 | 66 / 73 | done 12 |
| ggplot2 | 809 / 846 | done 119, timeout 3, oom 1 |
| rlang | 1,191 / 1,425 | done 59, crash 1, timeout 1 |
| curl | 50 / 73 | done 22 |
| dplyr | 943 / 1,341 | done 79 |
| purrr | 251 / 452 | done 51, timeout 2 |
| tidyr | 258 / 692 | done 31 |

The merged root causes, each with a test recorded with GNU R and its
package-suite effect measured on a VM:

| PR | cause | effect |
|---|---|---|
| #339 | `.External2` was always visible (names.c eval 201) | magrittr green |
| #345 | a C write into a shared `0L` changed every `0L` | purrr's VM panics gone |
| #348 | internal generics did not find S4 methods; `lengths` port | RcppEigen green |
| #355 | the bridge dropped closures' S4 bit | jsonlite green |
| #356 | the simple-.Internal memo kept callers alive | R6 finalizers |
| #358 | C-bound envs/functions read as active bindings | lifecycle, stringr |
| #359 | `on.exit` inside `eval()` never ran | lifecycle, tidyselect |
| #360 | `stderr()` ignored `sink(type = "message")` | Rcpp green |
| #361, #392, #253 | `R_Busy`; `...` through C; `adist` | scales green |
| #363 | S3 dispatch context of primitives hidden from `sys.*()` | tibble |
| #346, #369 | a local httpbin; own skips leave the floor | curl scorable, httr green |
| #372 | every `.Call` replaced every proxied closure's attributes | ggplot2 39 → 236, no oom |
| #386 | every `.Call` walked every pinned proxy (council round 5) | 72 → 5 timed-out files |
| #395 | GNU R's cairo module, built from its sources | glue (`png()`, `svg()`) |
| #402 | GNU R's collection trigger (memory.c) | rlang, cli, dplyr faster |
| #411, #412 | perf drift from #359 and #348 | gate restored |

Found while measuring: ggplot2's `+` (the "Incompatible methods" entry
above) and withr's global `defer()` work on main; both entries are stale.

What remains, by owner:
- purrr, tidyr, dplyr (most of the 1,351 missing blocks): C writes into
  `.Call` arguments seen during the call. Needs #398 (scalar identity,
  council round 3), then twins (#167, #387, #389, council round 5), then
  zero-copy. Measured on the zero-copy branch with #398: purrr 415, tidyr
  648, dplyr 1,058.
- R6: dead values in VM registers (#357, council round 4), PR #417:
  73 / 73.
- rlang, cli: twins (#389's 17 blocks) and causes under investigation.
- ggplot2: the 3 timeouts and 1 oom (proxy retention, under investigation).
- curl: `R_new_custom_connection` (#343, design pending) and finalizers in
  `test_that()` (#344).
- glue, htmltools: graphics devices after cairo (under investigation).
- tidyselect: cons-cell tails (#387).
- The council's third member (Gemini) could not be seated in rounds 4 and 5
  (provider usage limit); both rounds ran with Opus and Sol.

## 2026-11-05: round 2, 11 of 20 green

Full suite on main 9d9dff72, one VM, 4 jobs: 11 of 20 green, 7,365 of
8,693 blocks (round 1: 9, 7,342).

| package | round 1 | round 2 | cause of the change |
|---|---|---|---|
| glue | 112 | 114 / 114, green | #420, #421: natives configured without pango; cairo's FreeType exports |
| htmltools | 111 | 112 / 112, green | the same |
| R6 | 66 | 71 / 73 | #417 (dead registers); the rest is nondeterministic, below |
| curl | 50 | 59 / 73 | |
| cli | 677 | 679 / 689 | |
| rlang | 1,191 | 1,194 / 1,425 | |
| ggplot2 | 809 | 810 / 846 | |

R6's two blocks are test-finalizer's and differ run to run (two `show R6`
on the same VM fail different tests): sometimes an R6 object is
finalized one `gc()` late, in the next test. GNU R is deterministic here.
Under investigation.

Council round 7 (#422: package-defined ALTREP vectors and `inspect`;
Opus and Sol, Gemini's endpoint unavailable): a two-way ALTREP twin
(the runtime keeps a copy linked to the original C object; the class's
`Elt`, `Sum`, `Inspect`, ... are called where GNU R calls them, behind
one global flag), with every class call behind a protected C boundary.
cli's live `cli_timer_t` flag needs the per-read `Elt` call. Recorded on
#422.

What remains, by owner:
- purrr, tidyr, dplyr: unchanged; #398, then twins, then zero-copy.
- rlang: twins (#389, #167; ~100 blocks), a crash in test-c-api's
  `r_on_exit` (68), #387 (16), #197 srcrefs (8), #419 (7).
- cli: #422 ALTREP (5), #423 tail calls (1), #389 (3).
- curl: native custom connections (#343) and one mark over both heaps
  (#344), council round 6.
- ggplot2: 3 timeouts and 1 oom (proxy retention).
- tidyselect: #387.

## 2026-11-05: round 3 (preview), 12 of 20 green

A preview full run of main 88bef9bf with the eight PRs then open merged in
(#424, #425, #427, #428, #429, #430, #439, #440; branch
exp/integration-r3), one VM, 4 jobs: 12 of 20 green, 7,383 of 8,693
blocks. Against round 2: 18 blocks gained, none lost.

| package | round 2 | preview | from |
|---|---|---|---|
| R6 | 71 | 73 / 73, green | finalizer timing varies between runs (under investigation) |
| rlang | 1,194 | 1,202 | not attributed per PR |
| ggplot2 | 810 | 815 | #439 (`.C` NAOK, 1) and #440 (plotmath Greek in SVG, 2) checked per file; the rest not attributed |
| cli | 679 | 682 | not attributed per PR (#426, the REPL prompt, merged in between) |

Open PRs not in the preview, with the gains their authors measured on a
VM:

| PR | what | measured |
|---|---|---|
| #436 | twins (council round 5): one core object per looked-up runtime value; #387, #353 | rlang 1,194 → 1,311, dplyr 943 → 982, purrr +8 |
| #444, #445 | attributes and nested write-back only when C changed them | ggplot2 810 → 826, no timeouts, 2.3× faster |
| #435, #442 | native custom connections (#343); one mark over both heaps (#344) | curl 59 → 71, +2 |
| #453 | package ALTREP twins and `inspect` (council round 7) | cli 683 → 688 |
| #451 | constants promised where GNU R interprets (#419) | rlang +7, ggplot2 +5 |
| #452 | statement srcrefs, lazily (#197) | rlang +10 |
| #441 | GNU R's TRE for the patterns robigo's engine got wrong (#423) | cli +1 |
| #446 to #450 | void `.Call` results, C conditions through the R hooks, `suspendInterrupts`, call tags, the empty symbol | rlang about +50 |

Decisions this round:
- **#197:** lazy srcrefs, no gate exception.
- **#398:** GNU R's `R_expand_binding_value`, boxing once at the central binding accessor.
- **#302:** automatic cycle collection on runtime heap growth, run only at GNU R's pending-finalizer points.
- **#442:** C-held objects are not roots in `gc()`, as in GNU R.

The review bot's failures from its provider's rate limit (HTTP 429) are
not blocking; build, test, corpora and perf decide.

## 2026-11-07: dogfooding baseline and the smoke tier

Question: can robigo run a user script over the packages people
actually load, interactively, today? Measured three ways on this
machine (host library of 320 packages for R 4.5, dplyr 1.2.0 there;
pinned tree has 1.2.1):

- **Loads.** Without `ROBIGO_NATIVES`, `library(tibble)` and through
  it dplyr/tidyr/scales died in pillar's and scales' `.onLoad` on
  grDevices' `gray`/`hcl` routines, which had no Rust port. Ported
  `gray`, `rgb`, `hsv`, `hcl`, `RGB2hsv` (grDevices/src/colors.c)
  beside the existing `colors`/`col2rgb` ports; conformance
  `tests/r/packages/grdevices-colorfns.R` covers recycling, scaling
  and every error path, including GNU R's own `SET_STRING_ELT`
  bounds error for a too-long `gray()` alpha (recorded from the
  4.5.0 oracle; GNU R segfaults on a zero-length alpha, robigo
  errors instead). The data stack (tibble, dplyr, tidyr, purrr,
  stringr, rlang, vctrs, testthat, S7, withr) now attaches with no
  natives at all. ggplot2 still needs grid's native build.
- **Verbs.** dplyr mask expressions that are bare symbols work;
  any call (`x > 1`, `x * 2`, `identity(x)`) fails with
  "attempt to select less than one element in integerOneIndex".
  Inside a failing `mutate`, `dplyr:::current_group_id` reads 0
  where GNU R 4.5.0 (same library, oracle container) reads 1: the
  C++ mask loop caches the binding's `int*` (`DPLYR_MASK_SET_GROUP`,
  dplyr.h) and writes through it per group before evaluating the
  quosure; the write is not visible to the evaluation. Isolated
  C-API shapes all pass on robigo (defineVar+eval, C active
  bindings, C-made envs, holder-only args, kept SEXPs across calls,
  nested .Calls, eval through an R closure, primitive-headed
  binding bodies — repros of the session, in /tmp), so the gap is
  in how the real path combines them: R6 private holder, rlang-made
  info env, chops env with ALTREP lazy slices, eval through
  rlang's `eval_tidy`. Not fixed here; the bridge owner gets it as
  smoke case `tidyverse-verbs` (times out in the pinned tree, see
  below) plus this paragraph. Grouped/rowwise `mutate(z = x)` and
  `transmute` pass; `.data$x` fails like bare `x` in a call.
- **Errors.** An unhandled `rlang::abort("boom")` at top level
  hangs: no output until killed. With `options(expressions = 200)`
  it stops as "evaluation nested too deeply" instead of GNU R's
  message and backtrace — recursion in rlang's backtrace printing
  (the bayesim ledger's #232). This turns every rlang-shaped error
  in a script into a hang, which is why `tidyverse-verbs` times
  out rather than diffing. Smoke case `rlang-abort`.

The smoke tier (`scripts/package-suite.py smoke`,
`tests/packages/smoke/`): user scripts transcript-checked against
GNU R on the pinned library, on every PR (ci.yml `smoke`, minutes),
reported on the progress page under "Package scripts as users run
them". First run here (oracle, trixie binary of this branch):
6 of 8 scored cases pass and are marked (ggplot-build once grid's
native is built, lubridate-dates, purrr-map, stringr-glue,
testthat-file, tibble-print); `tidyverse-verbs` and `rlang-abort`
are pending timeouts; `own-dagriculture` is absent (not in the
pinned tree). Deliberately outside smoke: source installs (the
`install` tier), vignettes and examples (corpora), block-level
outcomes (the nightly suites), the REPL.

CI shape after this: PRs run smoke + conformance + corpora +
cran gate; the hours-long block suites stay nightly (shards are
already greedy-balanced on GNU R's recorded seconds). The
expensive remainder on PRs is `perf` (~30 min of the ~40 min
wall) — untouched here; see the session report's recommendation
to split it by change area.

## 2026-11-07 (later): the three blockers, fixed — smoke 8 of 8

- **Nested complex assignment into an unforced argument dropped
  every attribute** (the abort recursion's root):
  `complex_assign_inplace` took the target binding out
  (`take_local` leaves NULL), then bailed on an unforced promise
  with `p.value()?` without restoring it. The general path read
  NULL and assigned into a bare list. rlang's
  `cnd$rlang$internal$backtrace_on_error <- opt` inside
  `signal_abort` hit it on every unhandled error, dropping the
  condition class and recursing through `ffi_cnd_type`'s own abort
  until killed. Restoring the binding on that exit fixes
  conformance `tests/r/lang/nested-assign-attrs.R` (ratcheted)
  and smoke `rlang-abort`.
- **Cached DATAPTR writes after the first were invisible** (the
  group id): a twin is marked written when C takes its writable
  pointer, but dplyr's loop caches `INTEGER(group_id)` and writes
  through it per group with no new DATAPTR call. Small atomic copy
  twins (≤64 elements) that C took a writable pointer to are now
  compared and resynced at every flush (`SMALL_CACHED`), and the
  host-op seam (`op()`) flushes twins like `guarded()` already
  did, so an evaluation after a C write cannot miss it when no
  conversion of that twin intervenes.
- **Attribute-pairlist identity** (host rlang 1.1.7's
  `is_splice_box`): pairlist<->list coercion round-tripped through
  runtime values and rebuilt every element's attributes;
  `Rf_coerceVector` now does both directions in core
  (`Rf_PairToVectorList`/`Rf_VectorToPairList`), moving elements
  themselves as GNU R does. Escaped result elements are also
  pended — attributed ones twinned at once — so an element R hands
  back alone is still the object C made. Capi case
  `mask_cached_ptr` pins the loop and the identity (ratcheted).

Smoke is green: 8 of 8 scored cases match GNU R in the oracle,
all marked; `own-dagriculture` is still absent from the pinned
tree but matches GNU R on the host library. Host-library probes
after the fixes: the full tidyverse verb script is byte-identical,
`rlang::abort` prints its backtrace and exits 1 on rlang 1.1.7 and
1.3.0 alike, and dagriculture/bayesfam/huerd/bayesgrove all load.
dplyr's test-filter file still times out whole on this machine
(slow failure paths; blocks before the interruption match).

## 2026-11-08: own R packages (sims1253) as a separate suite group

Track `packages`, branch `track/packages-own-packages` (#505). Before: no
own-package recordings, no own markers, one of six own smoke cases marked.
The first lock had no testthat in the own closure, so every test file
stopped at setup: dagriculture's first file under GNU R gave status
`setup`, 0 blocks, "there is no package called 'testthat'".

- `tests/packages/own-selection.tsv` pins sims1253's ten R packages by
  commit. Six run (dagriculture, bayesfam, bayesgrove, bayesim, huerd,
  ciecam16). Four are excluded with their reason: bayeshear (imports brms
  and fits models), bever (Rust build, no tests), hegel-r (needs libhegel),
  hrvester (non-CRAN remote FITfileR and FIT fixtures).
- `lock --own` resolves the six plus testthat and their test Suggests (152
  packages, 3.5 min). `run --own` installs the canary into an empty cache
  on first use, fails hard when `sync_own` misses a package, and writes
  `results/own-packages.json` so a CRAN run after it keeps its own file.
- `scripts/history.py` shows the own group in a panel of its own. Green
  counts are out of the declared selection; a run whose canary failed or
  whose shards did not all report shows its table but no score and stays
  out of the trends. `scripts/test-package-suite.py` renders the panel
  from this branch's real result doc (`scripts/testdata/own-packages.json`).
- `.github/workflows/packages.yml` gains an `own` job: all six nightly,
  ciecam16 on a pull request, artifact `packages-own` (kind
  `own-packages`), which the history job ingests with `packages-*`.

Filters, from one unfiltered GNU R pass in the oracle (46 min in all):

- bayesfam: 3,679 s unfiltered. Seven files never finish under GNU R:
  family-contracts (7 s), inverse_burr (78 s), shifted_lognormal_uniform
  (52 s) and simplex (24 s) crash; test-helper, unitlindley and
  weibull-median run out of the 1,800 s budget. Fifteen more take 44 to
  892 s each (generalized_normal 892, gumbel-mean 433, gompertz 356,
  kumaraswamy 195, lomax 195, betaprime 189, logistic 184, cauchitnormal
  180, lognormal_natural 172, softplusnormal 165, shifted_inv_gaussian
  159, symlognormal 129, logitnormal 126, generalized_gamma 67,
  cloglognormal 44). The filter keeps the eight files that finish fast:
  33 s under GNU R.
- bayesgrove: 1,166 s unfiltered; backends-brms-integration (515 s) and
  properties (392 s) are 78% of it. Without them: 245 s.
- bayesim: 79 s unfiltered, so no filter (35 files).
- dagriculture, huerd, ciecam16: unfiltered.

Recorded (`record --own`, 3 GNU R runs each, 21 min) and run (`run --own
--update`, two jobs, 9.6 min) on main's CI binary at 37f9b847:

| package | files | blocks | scored | matching | robigo s | GNU R s | green |
|---|---|---|---|---|---|---|---|
| dagriculture | 12 | 370 | 370 | 370 | 232 | 211 | yes |
| bayesfam | 8 | 28 | 25 | 25 | 28 | 33 | yes |
| bayesgrove | 56 | 750 | 744 | 744 | 302 | 245 | yes |
| bayesim | 35 | 670 | 663 | 660 | 82 | 81 | no |
| huerd | 13 | 763 | 761 | 761 | 264 | 139 | yes |
| ciecam16 | 1 | 13 | 3 | 3 | 1 | 1 | no |

2,563 of 2,566 blocks match, 4 of 6 packages green, canary 9 of 9. The
2,563 matches are committed as `tests/packages/own-passing/` markers.

Causes:
- bayesim, 3 blocks in test-parquet-summary: robigo skipped them at
  `skip_if_not(requireNamespace("nanoparquet"))` because nanoparquet's
  shared object did not load ("undefined symbol: Rf_nlevels"; `nm` shows
  it is the only libR symbol nanoparquet needs that robigo lacked). Fixed:
  `Rf_nlevels` ported from Rinlinedfuns.h, capi case
  `tests/corpora/capi/cases/nlevels.R` recorded by GNU R and marked. With
  the fix, `test_file("test-parquet-summary.R")` under robigo on the
  oracle-built library passes all four tests, none skipped. The nightly
  scores it on main's next binary; the markers here are from the binary
  before the fix.
- ciecam16 is not green because only 3 of its 13 blocks are scorable:
  GNU R fails the other 10 too. The package defines S7 methods inside a
  `local()` block at build time and its `.onLoad` never calls
  `S7::methods_register()`, so `==` and `as.list()` on its classes fail
  under GNU R 4.5.0. A package bug, not a robigo gap.
- huerd takes 1.9 times GNU R's time (264 s against 139 s), bayesgrove
  1.2 times: speed, not output; no conformance cause.

Smoke: the earlier GNU R transcripts were recorded with the user's
personal library on `R_LIBS`. Re-recorded against the pinned libraries
only, each with an empty stderr and status 0. `own-ciecam16` now prints
results instead of the README's `==` and `as.list()` lines, which stop
under GNU R for the reason above. All six match robigo (CI binary) and are
marked; the four excluded packages stay absent.

Proof: `python3 scripts/test-package-suite.py` passes 82 tests;
`scripts/corpus.py run capi --filter '^nlevels$'` passes; the table above
is `run --own` against the recordings in `tests/packages/own-expected/`.
