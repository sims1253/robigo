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
- **Rscript's `--default-packages=`**: consumed and exported by the
  Rscript-style invocation (positional file or `-e`), left as an unknown
  option warned about in `R` mode (`-f`/console), as in R.
- **BOM/multi-byte parse errors**: a parse-error end offset inside a
  multi-byte character panicked the console; R's `R_ParseContext` ring
  works in bytes, so the context is sliced as bytes (lossy). No corpus
  case has a BOM; kept for robustness.
- **fastr guard**: the full local run was killed externally (SIGTERM at
  ~37 min); shard 1/16 (1,980 cases, ~14.5k corpus-wide contain `;`, so
  the chunking path is well covered) ran clean: 1785/1980, 0
  regressions, 1 new pass (`library/base/TestSimpleParsing/87fcf921a5cdcda3`,
  `1234.0L` — the per-statement lexer warnings). CI runs the whole corpus
  on the PR.


### More parser-track notes

- A parse error whose token span lands inside a multi-byte character
  (UTF-8 BOM at file start) produces a different context in robigo
  (`"\\uFFFD"` vs R's whole character) — the token span should end at the
  character boundary, and R's lexer rejects invalid multi-byte input with
  `invalid multibyte character in parser` (robigo's lossy conversion
  accepts it; n1–n4 probes in the ledger's spirit: invalid UTF-8, NUL
  bytes — no panics, transcripts differ).



## 2026-10-31: shutdown order and TLS destruction (issue #145)

- why: `library(mirai)` printed a spurious `getClasses`/`substring` error at
  exit, and `library(mirai); m <- mirai(1+1)` aborted with
  `cannot access a Thread Local Storage value during or after destruction`.
- actual causes (verified against the R 4.5.0 sources and GNU R):
  1. robigo runs the installed GNU R's base library; this machine has R
     4.6.1, whose `substring`/`substring<-` pass `NULL` for an omitted
     `last` (4.5 used `1000000L`). `character.c:do_substr` rejected the
     `NULL` stop, so loading `cli` (mirai's `.onLoad` dependency) failed.
     `cli`'s `.onLoad` has already registered an exit finalizer; the failed
     load left it queued, and at exit it re-reported the failure.
  2. Rust destroys thread-locals in reverse registration order. The C API
     bridge's `PROXIED`/`STORED`/`SEEN` caches hold runtime values whose
     `Drop` touched `WRAPPED`, the isolate (`ISOLATE`) or the search epoch
     (`SEARCH_EPOCH`); at process exit those were already destroyed, and
     std aborts on a TLS `AccessError` in a destructor.
  3. `Rstd_CleanUp` runs `.Last`, then `R_RunExitFinalizers`, then
     `PrintWarnings`; robigo skipped `.Last`, printed "Execution halted"
     after the finalizers, and dropped warnings raised by either.
- what: `substr`/`substr<-` read a `NULL` stop as "to the end" (matching
  R 4.6's base code); the bridge's destructors use `try_with` (`CoreRef`,
  `isolate`, `Frame::drop`'s epoch bump, the finalizer-runner guard);
  `Interp::run_dot_last` ports `R_dot_Last`; `finish` and `do_quit` run
  `.Last` (honouring `runLast`), report "Execution halted" before the
  finalizers, and print warnings afterwards.
- oracle: `Rscript --vanilla` shows `.Last` -> finalizers -> warnings;
  an error in `.Last` is reported, finalizers still run, status 1; a
  script error skips `.Last` but runs the finalizers; `q("no", 3, FALSE)`
  skips `.Last` and exits 3.
- result: `library(mirai); cat("ok\n")` exits 0 with no extra error;
  `mirai(1+1)` no longer aborts (it still exits 1 on nanonext's separate
  `use of NULL environment is defunct` native-call failure). Six new
  `tests/r/cli/exit-*.R` / `unload-methods.R` scripts and
  `tests/r/strings/substring-default-last.R` pass; strings 25/25 and cli
  17/17 conformance. `cargo test -p robigo-capi --lib` covers the TLS
  destructor with a subprocess (it aborts without the fix).

### 2026-10-31 (follow-up): interactive `.Last` errors return to the REPL

- finding: the first fix reported `.Last` errors as `Execution halted` and
  exited from the interactive `repl()` and `q()` paths. GNU R 4.5's
  `main.c:R_dot_Last` lets the error kick back to the REPL instead.
- oracle (controlled PTY, `robigo-oracle:4.5.0`): in an interactive
  session, `.Last <- function() stop("last failed")` then `q()` prints
  `Error in .Last() : last failed`, stays at the prompt, runs no exit
  finalizer and does not print `Execution halted`; the rest of the input
  line is dropped. The same happens at EOF (the prompt returns and the
  session stays alive). `q(runLast = FALSE)` afterwards runs the
  finalizers and exits with the requested status. A conditional nested
  `q()` inside `.Last` runs `.Last` twice, so no re-entry guard is added.
- what: `Interp.interactive` (`R_Interactive`, also what `interactive()`
  now reports) distinguishes the REPL. `run_dot_last` returns its error
  unreported and restores the caller's frames; `do_quit` returns that
  error interactively (top-level reports it and drops the line) and keeps
  the `Execution halted`/status-1 path non-interactively. Interactive EOF
  calls `finish_interactive`, which returns to the prompt on a `.Last`
  error and otherwise runs the exit finalizers.
- tests: `crates/robigo-cli/tests/interactive.rs` covers `q()` returning
  to the REPL, no finalizer on the aborted quit, `interactive()` true,
  and EOF running `.Last` then finalizers. The EOF-error PTY transcript
  above is the manual proof; the existing non-interactive
  `tests/r/cli/exit-last-error.R` still passes.

### 2026-10-31 (review follow-up): TLS branch coverage and vectorized substr

- TLS test: `PROXIED` is registered before `WRAPPED`, so `WRAPPED` (a
  `RefCell<HashMap>`, the only destructor-bearing thread-local in this
  path) is already destroyed when `PROXIED` drops its values. Reverting
  `WRAPPED.try_with` to `with` makes the subprocess abort (SIGABRT), which
  is the branch the test guards. Reverting `try_isolate` or the
  `SEARCH_EPOCH` `try_with` does not abort: both are plain `Cell`s, which
  register no TLS destructor; those guards are defensive, and the test
  cannot reach a destroyed state for them. The test still drops a
  search-path frame, so `Frame::drop` runs during TLS teardown.
- substr: `substr`/`substr<-` with a `NULL` stop now have unit tests for
  vectorized `start` and recycled `start`/`value` (R 4.6.1 oracle:
  `substr(c("abcdef","ghijkl","mnopqr"), c(2,3,1), NULL)`, etc.).
  `tests/r/strings/substring-default-last.R` adds vectorized/recycled
  omitted-`last` cases; its output matches GNU R 4.5.0 byte-for-byte and
  the conformance case passes.

### 2026-10-31 (Sol review round 2): top-level abort and startup `interactive()`

- S1: restoring the caller's frames let `on.exit(return(invisible(NULL)))`
  swallow the interactive `.Last` error, so the rest of the line ran.
  GNU R's `R_dot_Last` error runs under the top-level contexts and skips
  the caller's `on.exit`. Added `Interp::abort_to_toplevel`: `do_quit`
  sets it on an interactive `.Last` error, `end_context` skips the
  caller's `on.exit` while unwinding, and `eval_toplevel` clears it. The
  regression (`.Last <- stop(...); f <- function() { on.exit(return());
  q() }; f(); cat("AFTER")`) now drops the line; it printed `AFTER`
  without the flag.
- S2: `interactive()` in `.First`/profiles was false because the flag was
  set after startup. R_Interactive is known from the tty before
  base/profile startup; `Interp.interactive` is now set there. A profile
  `.First <- function() cat(interactive())` prints TRUE, matching GNU R
  4.5.
- `crates/robigo-cli/tests/interactive.rs` now has five cases, including
  the `on.exit` swallow regression and `.First`; all pass. cli
  conformance 17/17.
