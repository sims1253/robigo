## 2026-11-17: RC, robigo-rt's `-f` front end, so the echo-mode corpora get rt columns

RC's second step (the first is `2026-11-17-rc-rt-corpora.md`).

Four corpora run in echo mode, as `R --vanilla --quiet --no-save -f FILE`:
gnur-tests, gnur-examples, gnur-demos and r-intro. Their recordings are R's
transcripts: each line the console reads, after its prompt, then what
evaluating it prints. robigo-rt had only Rscript's front end, so
`corpus.py --runtime rt` refused all four. It now runs them.

### What changes

- **The console loop** (`robigo_rt::repl::run_console`) gains `echo`, which
  is `!R_NoEcho`. It stays the one loop for Rscript and `R -f`. The
  statements still go through `base::repl_eval_toplevel`, as before.
  - **Reading** ports `Rstd_ReadConsole`'s non-interactive branch
    (sys-std.c:994-1050):
    - with echo, the prompt is written before each read, the end of the
      input included, and the line is written after it;
    - a read is what `fgets` returns: through the newline, at most 4,095
      bytes (`CONSOLE_BUFFER_SIZE - 1`), cut at a NUL by `strlen`;
    - CRLF becomes LF;
    - the input's last line gets a newline if it lacks one.
  - **The prompt** is `R_PromptString`'s: `getOption("prompt")` for a new
    statement and `getOption("continue")` inside one. As in
    `Rf_ReplIteration`, PARSE_INCOMPLETE and PARSE_EOF ask for a
    continuation, and the other statuses for a new statement.
  - **A port fix:** PARSE_EOF ended the loop. It now reads on, as GNU R
    does (PR#15941). It arises when a read ends in blanks without a
    newline, at a long line's 4,095-byte cut or a NUL.
- **The binary** (`robigo-rtapi`'s `main.rs`):
  - **`-f FILE` and `--file=FILE`** (`--file=-` reads stdin). As R's front
    end does (R.sh.in), `-f` followed by nothing, or by an argument
    starting with `-`, is "ERROR: option '-f' requires a filename
    argument", exit 1.
  - **The options:** `--quiet`, `--silent` and `-q` set `R_Quiet`;
    `--no-echo`, `--slave` and `-s` set both `R_Quiet` and `R_NoEcho`.
    `--no-save` is accepted, and `-f` implies it.
  - **End of input:** `end_Rmainloop`'s newline is written before `.Last`.
  - **`--args`** ends the options after `-f`. Any other argument after `-f`
    is "ARGUMENT '…' __ignored__", as in R.
  - **The start-up errors** go to stdout, where GNU R writes them this
    early:
    - "cannot use -e with -f or --file";
    - "cannot open file". Rscript's case was on stderr before, which was
      wrong.
  - **Not implemented, so refused:** `--save` (saving the workspace at the
    end) and R's start-up greeting (`-f` without `--quiet`). Each says so
    and exits 2.
- **`scripts/corpus.py`** no longer refuses echo mode for rt. Corpora with
  a build step stay refused.
- **CI:** the `rt-corpora` job runs all four echo corpora on every rt pull
  request, beside fastr's quarter. They are small.
- **The deferred rtapi nit.** `entry!` asserts the export's
  integer-register budget: `lo` goes in the integer register after the
  arguments, so an export takes at most five integer-class arguments. A
  sixth used to fail as a macro that matched no rule. It now fails with
  "robigo-rtapi: NAME has 6 integer-class arguments: the machine entry
  passes `lo` after them, in one of six registers" (tried with a temporary
  `[6]` export).

### The rt columns

The four corpora against GNU R 4.6.2's recordings:

| corpus | passes | what most failing cases hit first |
|---|---:|---|
| gnur-tests | 4/58 | `pdf` unbound (8), the `proc.time` primitive (5), `.Internal(set.seed)` (4), `options(digits = )` (3) |
| gnur-examples | 0/1,153 | `.Internal(RNGkind)` (all of them: every example starts with `RNGkind`) |
| gnur-demos | 0/16 | `options(keep.source = )` (all of them) |
| r-intro | 0/13 | `options(keep.source = )` (all of them) |

gnur-tests' four passes are ratcheted in `passing-rt/`: `any-all`,
`eval-fns`, `reg-win` and `timezone`. Each is a whole GNU R transcript,
byte for byte. The failures are base's, not the front end's: in every
failing case of the four corpora, robigo-rt's transcript up to its last
prompt line is the start of GNU R's.

### Proof

- **The slice as transcripts.** robigo-rt's ratcheted conformance slice has
  456 scripts (tests/rt and tests/r). Each was run as `--vanilla --quiet
  --no-save -f FILE`, once by GNU R 4.6.2 in the oracle image and once by
  robigo-rt, in the conformance environment.
  - All 456 transcripts are the same, stdout, stderr and status, with
    addresses masked as the corpora mask them.
  - That is 25,226 lines: 9,741 prompt lines and 2,671 continuation lines.
  - 125 of the runs end in an error.
- **The reader's edges.** `crates/robigo-rtapi/tests/cli.rs` holds 16
  cases recorded with GNU R 4.6.2:
  - statements split by `;`, continued statements, comments, blank lines;
  - a last line without a newline, CRLF, a line over 4,095 bytes, a NUL;
  - `.Last` after the final newline, and an error that halts;
  - `--no-echo`, `--file=FILE`, `--file=-`, `-f -`, `-e` with `-f`, an
    extra argument, `--args`, a missing file under R and under Rscript.

  Each matches GNU R's stdout, stderr and status. A second test checks the
  refusal of the greeting.
- **The old runtime** was not needed as a cross-check: GNU R's own
  transcripts decided every case.
