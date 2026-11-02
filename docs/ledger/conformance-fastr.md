# conformance-fastr

## 2026-10-31: baseline and failure groups

- Baseline: 31,206 of 31,374 pass on main at `f523d79a`, 168 failing cases.
- Failure groups:
  - 10 parser/TestParser (empty symbol names)
  - 9 library/base/TestSharedCluster (parallel makeCluster)
  - 8 builtins/TestBuiltin_qr (LAPACK=TRUE)
  - 7 S4/TestS4
  - 7 builtins/TestBuiltin_deriv
  - 6 builtins/TestBuiltin_zzfile (gzfile/xzfile append and read)
  - 5 builtins/TestBuiltin_copyDFattr
  - 5 library/base/TestSimpleAssignment (empty symbol assignment)
  - 4 builtins/TestBuiltin_vector (error messages for invalid mode/length)
  - 4 builtins/TestBuiltin_adist
  - 4 builtins/TestBuiltin_mget
  - 4 builtins/TestBuiltin_tracemem
  - 4 library/stats/TestUpdateForm
  - 3 builtins/TestBuiltin_parentenvassign (defunct NULL env and error calls)
  - 3 builtins/TestBuiltin_aperm
  - 3 functions/TestFunctions
  - 3 library/stats/TestStats
  - 28 remaining groups with 1-2 cases each (40 cases total)

## 2026-10-31: round 1 - builtins/TestBuiltin_vector

- Topic: `builtins/TestBuiltin_vector`
- Changes: `crates/robigo-runtime/src/builtins/bind.rs`: align `do_vector` with GNU R `do_makevector` (`src/main/builtin.c:786`). Validate length first using `as_vec_size`, check `length == 1`, coerce mode argument to string vector, check `mode.len() == 1`, validate against allowed modes (rejecting `any`), and report errors matching GNU R without internal call attribution.
- Result: 31,210 of 31,374 pass (+4 cases, 0 regressions). All 28/28 cases in `builtins/TestBuiltin_vector` now pass.
- Status: PR #158 (merged).

## 2026-10-31: round 3 - builtins/TestBuiltin_inherits

- Topic: `builtins/TestBuiltin_inherits`
- Changes: `crates/robigo-runtime/src/builtins/attrib.rs`: align `do_inherits` with GNU R `src/main/objects.c:1014-1035` (`inherits3`) and `src/main/coerce.c:1812` (`asRbool`). Strictly check `which` is a length 1 logical vector, erroring with `Flow::error("'which' must be a length 1 logical vector")`, check `NA_LOGICAL` in `asRbool` coercion erroring with `Flow::error("NA in coercion to boolean")`, and error on invalid `what` with `Flow::error("'what' must be a character vector or an object with a nameOfClass() method")` without call attribution.
- Result: 31,215 of 31,374 pass (+2 cases, 0 regressions). All 58/58 cases in `builtins/TestBuiltin_inherits` now pass.
- Status: kept.

## 2026-11-01: round 2 - REPL parse-error reporting

- Topic: `parser/TestParser`, `library/base` (parse errors in echo mode)
- Diagnosis: the echo runner printed every parse error as `parseError(R_NilValue, 0)` (`msg in "<context>"`). GNU R only takes that path for grammar errors (`yyerror`); conditions from `raiseParseError`/`raiseLexError` carry their own `(<input>:<line>:<col>)` suffix, and a plain `error()` from a parser action (`names.c:install`) has neither position nor context.
- Changes: `crates/robigo-cli/src/echo.rs`: `parse_error_message` dispatches on `ParseError::style` (`Grammar` keeps the context lines, `Condition` uses `condition_message(buf, "<input>")`, `RError` is the bare message).
- Result: 31,224 of 31,374 pass on this branch (+14 cases, 0 regressions): 9 of 10 `parser/TestParser`, 4 `TestSimpleAssignment`, `TestSimpleParsing` octal and `TestSimpleVectors` cases.
- Status: kept.

## 2026-11-01: round 6 - parse-error call attribution and the console source name

- Topic: `functions/TestFunctions` (`f(a,a)`, `f(''=123)`), `parser/TestParser`
- Changes:
  - `crates/robigo-cli/src/echo.rs`: `parse_error_line` prints the whole `Error...` line and uses `conditions::format_error_message` when a `Condition` parse error carries a call, so `1 |> f(_)` reports `Error in f("_") : \n  pipe placeholder can only be used as a named argument (<input>:1:6)` like `verrorcall_dflt` (R: errors.c). Unit tests cover the `Condition`, `RError` and call-attribution branches.
  - `crates/robigo-syntax/src/lib.rs`: `CONSOLE_SRCNAME = "<input>"`, used by `dump.rs`, `builtins/parsing.rs` and the REPL instead of three literals.
- Result: 31,231 of 31,374 pass on this branch (+2 cases, 0 regressions). `library/utils/TestUtils/{7e10,868c}` fail only locally: they need the prebuilt native routines CI ships via `ROBIGO_NATIVES`.
- Status: kept.

## 2026-11-01: round 5 - SET_ENCLOS cycle check and R_IsImportsEnv

- Topic: `builtins/TestBuiltin_parentenvassign` (3 cases: `parent.env(baseenv()) <- ...`, imports environments)
- Changes:
  - `crates/robigo-runtime/src/env.rs`: `Env::parent_cycle` ports the parent-chain walk in `SET_ENCLOS` (`src/main/memory.c:4667`), so reparenting an environment below one of its descendants errors with `cycles in parent chains are not allowed` (GNU R reaches this for `parent.env(baseenv()) <- globalenv()`, because the base *package* environment is not a namespace).
  - `Env::is_imports_env` ports `R_IsImportsEnv` (`src/main/builtin.c:384`), including the `LENGTH(name) == 1` gate.
  - `crates/robigo-runtime/src/builtins/envir.rs`: `do_set_parent_env` uses the helper and the cycle check.
- Result: 31,231 of 31,374 pass on this branch (+3 cases, 0 regressions); `parentenvassign` 11/11, `tests/r/basics` unchanged.
- Status: kept.
