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
