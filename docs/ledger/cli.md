# cli: R's command line and transcripts

Metric: passes in `gnur-tests` (echo) and `gnur-examples` (echo).

## 2026-10-27: `R -f` as a console with echo (resumed track)

- why: robigo had no `-f`; the echo corpora ran 0/59 and 0/1153.
- what: ported R's non-interactive console (`R: unix/sys-std.c:Rstd_ReadConsole`,
  `main/main.c:Rf_ReplIteration`, `R_ReplConsole`, `check_session_exit`,
  `end_Rmainloop`, `source.c:parseError`/`getParseContext`) as
  `crates/robigo-cli/src/echo.rs`, plus R's two-pass option processing
  (`CommandLineArgs.c:R_common_command_line`, `unix/system.c:Rf_initialize_R`)
  in `main.rs`. The resumed WIP (`feat/cli-file-transcripts`) was reviewed,
  rewritten into this commit; its `robigo-syntax` changes (a `parse_console`
  entry point, `1.5L` warning choice, `1;;2` file-parse fix) were dropped —
  the parser track owns that crate — and replaced by cli-side equivalents
  (first-token `;` check, `ends_in_comment` scan).
- probes: 21 hand cases (prompts, `+` continuations, comments, `;` chunking,
  `;`-in-comment chunking, parse-error context formats, warnings deferred and
  immediate, `options(error=)` continuation dropping the rest of the line,
  unexpected end of input, missing file/`-f` argument/unknown option, exit
  statuses) all byte-identical with GNU R 4.5.0 (stdout, stderr, status).
- result: gnur-tests 0/59 -> 16/59, gnur-examples 0/1153 -> 439/1153;
  conformance tests/r 157/157 hold; kept.

### Notes for other tracks

- The parser track should own a `parse_console` (R's `R_Parse1Buffer` with
  `gend = 0`): today the cli crate re-derives the status from
  `parse_program_with_warnings` and a first-token `;` check plus an
  `ends_in_comment` scan (R: gram.y `SkipComment` setting `EndOfFile = 2`
  promotes every status to `PARSE_INCOMPLETE`).
- robigo-syntax bugs found from the console path (file mode too):
  - `1;;2` parses as two expressions; R errors "unexpected ';'" at the
    second (grammar `prog` has no leading-`;` rule).
  - `1.5L` warns "contains decimal", but `1e999L`/`1.0e400L` should say
    "non-integer value %s qualified with L"; the pick is `seendot &&
    !seenexp` (R: gram.y `NumericValue`).
- `R -e` and stdin consoles print the R banner first unless `-q`; robigo
  prints none (no corpus coverage yet).
- The final newline of `end_Rmainloop` is `Rprintf("\n")`, which a `sink()`
  open at end-of-file would capture; robigo's writes it straight to stdout.

### Failures outside the cli track (ranked; input for the other tracks)

Remaining after this change: gnur-tests 43/59 fail, gnur-examples 714/1153
fail. Classified by re-running each failing case and diffing stdout, stderr
and status against the recorded GNU R (first differing stream):

1. **Lazy-loaded datasets** (~70 cases): `Error: lazy-load database is
   corrupt` on `USJudgeRatings`, `mtcars`, `iris`, `women`, `stackloss`,
   `airquality`, `warpbreaks`, ... — startup track.
2. **Packages fail to attach** (~110): `package or namespace load failed
   for 'grid'` (59 grid cases), `'methods'` (36 methods cases + more),
   `Loading required package: survival` differences, `base/isS4` — startup
   (default packages) and methods (G6).
3. **Graphics device internals** (~80): `.External2(C_par, args)`,
   `.External(C_devholdflush, ...)`, `.External2(C_plot_new)`,
   `.Call(C_gray/C_col2rgb/C_doTabExpand/C_spline_basis, ...)` — native
   routines that come from GNU R's shared libraries at G2; not to be ported.
4. **Missing `.Internal`s / connections** (~40): `file.info`, `file.access`,
   `list.files`, `dump`, `readDCF`, `readBin`/`writeBin`/`writeChar`,
   `memCompress`, `gzcon`, `saveToConn`, `pretty`, `La_svd`/`La_qr`/`La_rs`,
   `eSoftVersion`, `pcre_config`, `curlVersion`, `bcVersion`, `Sys.umask`,
   `Sys.readlink`, `Sys.localeconv` — conformance-* and startup tracks.
5. **Complex arithmetic** (~8): `sqrt(0+1i)`, `exp((0+1i) * x)`,
   "unimplemented complex function" — numerics.
6. **Print/format differences** (~15): column-name truncation
   (`cos(pi * x...` vs `cos(pi * x)`), `<bytecode: 0x...>` lines missing
   when printing functions, `trace:` output, `dput` hex floats,
   `$bitmapType` in `str(options())` — conformance-print.
7. **Warning message texts/timing** (~15): `dnbinom` NaNs, `qbeta`,
   integer-overflow NAs, BLAS/LAPACK warnings (gnur-tests `lapack`,
   `print-tests`, `reg-BLAS`) — numerics + conformance-*.
8. **Distributions natives** (~6): `C_psignrank`, `C_pnt`, `d-p-q-r-tests`
   strict tolerance failures — conformance-distributions.
9. **Parser** (1): gnur-tests `simple-true` — `1e-3L` warning choice
   (see the parser note above).
10. **Conditions/S4 semantics** (~15): caught-error escapes to top level
    (`base/source` halts where GNU R's withCallingHandlers continues),
    duplicate-handler warnings, `setClass` failures, inherited-method
    dispatch — conformance-lang / methods.
11. **Environment-bound** (unfixable by robigo): `commandArgs()` prints the
    argv0 of the recorded run (`/usr/lib/R/bin/exec/R`); `Sys.getenv()`
    lists the recording machine's variables; dates/times recorded.

### 2026-10-27 (later commits)

- **Unterminated chunk**: a line longer than `CONSOLE_BUFFER_SIZE` is cut
  mid-line; robigo evaluated the partial statement (`x <- 1 + 1 + ...`
  computed from the first 4095 bytes). R's grammar needs the `;`/newline
  terminator (`prog` rules), so the poll is PARSE_EOF/incomplete. Fixed by
  requiring the terminator in `parse_status`; a 6 KB single-line file now
  byte-matches GNU R. Kept.
- **`-e` is Rscript's**: Rscript execs R with `--no-echo` (R:
  unix/Rscript.c), so `robigo -e` runs without prompts/echo; verified
  against `Rscript --vanilla -e` for values, errors, and incomplete input.
  Kept.
- **tests/r/cli/**: seven conformance scripts pin the console semantics
  (chunking, lone `;`, comments cut by the chunk loop, `.Last.value`,
  unexpected end of input, error-option continuation, lexer warnings);
  recorded from GNU R, all pass, ratchet 157 -> 165.

