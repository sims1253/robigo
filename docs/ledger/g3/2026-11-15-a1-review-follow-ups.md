## 2026-11-15: A1's follow-ups: the six-argument stub without its zeroing, `.Last`, the base subset in CI, and GNU R's protect-stack depth at a `.Call`

- What: the foreman's answers to `2026-11-14-a1-capi-cases.md`'s open
  questions, on #671: (c) measure removing the six-argument stub's zeroing
  on the `.Call` stub's evidence and remove it if clean; (d) `.Last` as GNU
  R runs it; (e) `scripts/base-subset.sh --check` in CI inside the 4.6.2
  oracle image; (a) the stack-imbalance warning exactly as GNU R prints it.
- Tree: branch `g3/a1-capi-cases` at `29ad3bde` (#666's `9cedbeaf`
  merged). Machine and binary as `2026-11-14-a1-capi-cases.md`; GNU R the
  4.6.2 oracle image.

### (c) The six-argument stub's zeroing: removed

`robigo_heap_call6_stub` is the native entry of C finalizers (the stress
script's), `R_ToplevelExec`'s and `R_UnwindProtect`'s bodies (E8's cases),
ALTREP methods and `cend` cleanups. Without its 1 KiB zeroing, the same
matrix as the `.Call` stub's (8 cases: the four capi cases, both probes,
weak-keys, the stress script; plain, gctorture with `GC_STATS=check`,
gctorture with the moving collector, the pin report): all 64 runs, with the zeroing and without, byte-identical to the 4.6.2 recordings, and the pin report the same 4 lines in both, all in `capi_pp_probe` or `capi_eb_probe`.
`cargo test --release -p robigo-heap` passes three runs in a row,
`six_argument_entry_frames_keep_nothing` included.

Instructions per `.Call` of a routine that makes one `R_ToplevelExec` of a
no-op (callgrind, N = 20,000 vs 2N): 2,355 with the zeroing, 2,264
without: **−91 per six-argument entry**, as for the `.Call` stub.

### (d) `.Last`

`R_dot_Last` (main.c) ported into robigo-rt's `repl.rs` and run where
`R_CleanUp` runs it: after a script that ran to its end (Rscript's `--file`
implies no save, so `runLast` holds), `.Last()` if `findVar(".Last",
R_GlobalEnv)` is a closure, then `.Last.sys()` from the base namespace
(only `R CMD BATCH`'s profile defines one); then the exit finalizers, then
`PrintWarnings`. An error in `.Last` is reported, "Execution halted", the
exit finalizers still run, status 1 (`check_session_exit`'s
`R_CleanUp(SA_NOSAVE, 1, 0)`); a script that halted runs no `.Last`. Four
`tests/rt/toplevel/dot-last*.R` cases, recorded on 4.6.2, pass and are
ratcheted.

### (e) The base subset in CI

A `base-subset` job (gated like `rt`) runs `scripts/base-subset.sh
--check` in the 4.6.2 oracle image (`select`'s `image_oracle_4_6_2`), with
seccomp off as `scripts/oracle-run.sh` has it. The script now runs the
`Rscript` on `PATH` when it is 4.6.2 and the oracle image through Docker
otherwise. Run that way locally (the oracle image, the checkout mounted,
the job's commands): the checked-in files are up to date, in 67 s.
`ci-check` allows the job's skip only when `rt`'s is allowed; actionlint
reports nothing new (the two shellcheck infos it prints are main's).

### (a) GNU R's protect-stack depth at a `.Call`: not done; a logical account

The warning is eval.c's `check_stack_balance`, `"stack imbalance in '%s',
%d then %d"` with `R_PPStackTop` before and after the builtin. robigo-rt
prints it with its own depths, because its protect stack holds only C's
protections; GNU R's holds its evaluator's too. GNU R 4.6.2 (the oracle
image, a routine that leaves one object protected):

| site | GNU R's warnings |
|---|---|
| `.Call("leak")` at top level | `'.Call', 2 then 3` |
| `invisible(.Call("leak"))` | `'.Call', 3 then 4`; `'invisible', 2 then 3` |
| `print(.Call("leak"))` | `'.Call', 8 then 9` |
| `x <- .Call("leak")` | `'.Call', 4 then 5`; `'<-', 2 then 3` |
| `f <- function() .Call("leak")`; `f()` three times | `'.Call', 5 then 6` once; the byte-compiled calls print nothing |
| `h(3)`, recursing through `if` | `'.Call', 11 then 12`; `'if', 5 then 6` |
| `for (i in 1:2) .Call("leak")` | `'for', 2 then 4` |
| `lapply(1, function(i) .Call("leak"))` | `'.Call', 15 then 16`; `'lapply', 4 then 5` |

The depth depends on every evaluator path above the call (the AST
evaluator's `eval`, `evalList`, `applyClosure`, `promiseArgs`; byte code
keeps its values elsewhere and checks no builtin), the leak makes every
enclosing builtin warn as well, and the same routine's `UNPROTECT` of one
too many is GNU R's error only when the whole stack runs out. Matching it
exactly is a logical account of `R_PPStackTop` through both evaluators,
like L7's for `gc()`, kept off the hot path. That is not small and not
A1's; a depth robigo guessed would break the porting contract's rule 6.
robigo-rt keeps eval.c's warning with its own depths after a `.Call`, and
"unprotect(): only N protected items" where C unprotects more than it
protected.
