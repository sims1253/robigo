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
