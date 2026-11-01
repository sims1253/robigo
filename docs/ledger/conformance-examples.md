# conformance-examples: help-page examples and pure-R CRAN packages

Metric: passing cases in `gnur-examples` (examples of every help page in GNU R 4.5.0's base and recommended packages) and `cran-purer` (pure-R CRAN packages: R6, withr, assertthat, proto, R.methodsS3).

Baseline on main (`f523d79a`):
- `gnur-examples`: 1,046 / 1,153 passing (90.7%)
- `cran-purer`: 36 / 41 passing (87.8%)

---

## 2026-10-31: pcre_config, nargs out-of-context, OutDec print formatting

- **why**: Help-page examples in base R and withr failed on missing internal, incorrect default for out-of-context primitive, and unhandled option formatting:
  - `gnur-examples: base/pcre_config`: GNU R's `pcre_config()` delegates to `.Internal(pcre_config())` in `grep.c:do_pcre_config`. It was completely missing in robigo, aborting with `there is no .Internal function 'pcre_config'`.
  - `gnur-examples: base/nargs`: In `lang.rs:do_nargs`, when called outside a function context (`frame_of_env` returns None), robigo returned `0`. GNU R's `util.c:do_nargs` initializes `nargs = NA_INTEGER;` and only updates it inside a closure context, returning `NA_integer_`.
  - `cran-purer: withr/man/with_options`: Tested decimal separator `options(OutDec = ",")`. Robigo initialized the option but ignored it during vector, matrix, and complex printing in `print.rs` and `format.rs`.
- **what**:
  - `builtins/grep.rs`: registered and implemented `.Internal(pcre_config())` returning a 4-element logical vector `c("UTF-8" = TRUE, "Unicode properties" = TRUE, "JIT" = TRUE, "stack" = FALSE)` as GNU R's `do_pcre_config` in `src/main/grep.c`.
  - `builtins/lang.rs`: `do_nargs` returns `NA_INTEGER` (`i32::MIN`) when called outside a function frame.
  - `format.rs`: added `out_dec: String` (default `"."`) to `PrintParams`, and added `encode_real_dec` and `encode_complex_dec` substituting `.` with `out_dec` when `out_dec != "."`.
  - `print.rs`: wired `Interp::print_params` to read the `OutDec` option into `pp.out_dec`, and passed it through to `encode_real_dec` / `encode_complex_dec` across vector and matrix printers.
- **tests**:
  - `gnur-examples`: `base/pcre_config` (passed, ratcheted)
  - `gnur-examples`: `base/nargs` (passed, ratcheted)
  - `cran-purer`: `withr/man/with_options` (passed, ratcheted)
  - Verified zero regressions across `cran-purer` (37/41 pass) and all 43 `robigo-runtime` unit tests pass.
- **result**:
  - `gnur-examples`: 1,046/1,153 -> 1,048/1,153 (+2 cases)
  - `cran-purer`: 36/41 -> 37/41 (+1 case)

---

## 2026-11-01: round 2 — `.Internal(file.show())` and `.Internal(file.symlink())`

- **why**: `gnur-examples` help pages failing on two missing internals from
  `platform.c`:
  - `file.show` (used directly by `base/file.show`; `utils/promptPackage` and
    `utils/write.table` call it on the files they write) aborted with
    `there is no .Internal function 'file.show'`.
  - `file.symlink` (`base/files`) aborted with
    `there is no .Internal function 'file.symlink'`.
- **what**:
  - `builtins/sysutils.rs`: ported `do_fileshow`
    (R: `platform.c:do_fileshow` + `sys-std.c:Rstd_ShowFiles`): argument
    checking in C's order (`asBool2(delete.file)` first, then the string
    checks), `isValidStringF` semantics for `pager` (NA or empty first
    element ⇒ `""`, not an error), temp file via `R_tmpnam(NULL, ...)` (empty
    prefix), per-file `\n\n` headers, `Cannot open file '%s': %s` message,
    then `R_system("'pager' < 'tmp'")`; status 127 warns
    `warningcall(R_NilValue, "error in running command")` (robigo:
    `warningcall(&Value::Null, ..)`). Visible vs invisible per `names.c`:
    `file.show` eval col `111` (invisible), `file.symlink` col `11`
    (visible — the wrapper's return value must auto-print).
  - `do_filesymlink` (R: `platform.c:do_filesymlink`): recycling
    `from`/`to`, `nothing to link` when `from` is empty, `logical(0)` when
    `to` is empty, NA ⇒ FALSE, warning
    `cannot symlink '%s' to '%s', reason '%s'` on failure.
- **environment note** (host ≠ oracle): the corpus was recorded in the
  `robigo-oracle:4.5.0` docker (Debian trixie, `r-base-core 4.5.0-3`,
  R_HOME=/usr/lib/R, R_DOC_DIR=/usr/share/R/doc, PAGER=/usr/bin/pager→more).
  CI's corpora job runs the same container, so marked cases behave like the
  oracle there. On this host the system `/usr/share/R` is 4.6.1's, so ~20
  path-printing cases (Rhome, search, srcfile, getLoadedDLLs, ...) fail
  locally regardless of code; and `file.show`'s group needs
  `PAGER=more R_DOC_DIR=$R_HOME/doc` locally (files md5-identical to the
  oracle's; `more` is the same program the oracle resolves to).
- **tests**:
  - `gnur-examples`: `base/file.show`, `base/files`, `utils/promptPackage`,
    `utils/write.table` (passed under oracle-aligned env, ratcheted).
  - `gnur-examples`: 9 more cases now pass and were ratcheted —
    `base/maxCol`, `datasets/{AirPassengers,Nile,UKDriverDeaths,nottem}`,
    `stats/{arima,arima.sim,arima0,predict.arima}` — fallout of merged #158
    (`do_vector` alignment), unmarked by that PR; all pass under the plain
    `R_HOME`-only env.
  - `cran-purer`: 36/41 locally, 0 regressions (#160's withr case is on that
    branch, not main).
  - `cargo test -p robigo-runtime --lib`: 81 pass; 2 failures are
    `builtins::complex::tests::c_library_*_oracle` (bit-level libm drift,
    complex.rs untouched here, another agent's active topic).
- **result**: `gnur-examples` 1,046/1,153 → 1,059/1,153 marked (+13);
  CI is ground truth for the ~20 host-env-sensitive cases.

## 2026-11-01: round 2 addendum — memory caps

Conductor rule after an OOM incident: every corpus/test command now runs
under `systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0`,
corpus runs with `--jobs 2`. Applied from the first full-corpus run in this
round onward.
