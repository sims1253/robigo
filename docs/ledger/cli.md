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

### Failures outside the cli track (ranked, from gnur-examples `^base/`)

To be re-ranked as the runtime tracks progress; entries below were sampled
from the 714 failing cases after this change.
