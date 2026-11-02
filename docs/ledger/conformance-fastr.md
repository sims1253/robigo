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

- Topic: `parser/TestParser` (`as.symbol("")`), `library/base/TestSimpleAssignment` (`"" <- 123`)
- Changes: `crates/robigo-runtime/src/builtins/coerce.rs` (`as.vector(x, "symbol")` / `as.symbol`) and `crates/robigo-runtime/src/builtins/lang.rs` (`<-`, `=`, `<<-` with a string left-hand side) now raise `attempt to use zero-length variable name` like `install()` (`src/main/names.c:1269`); R's `do_set` installs a string LHS (`src/main/eval.c:3564`) and `coerceToSymbol` installs the first element of an atomic vector.
- Result: 31,227 of 31,374 pass on this branch (+3 cases, 0 regressions).
- Status: kept (PR pending).

## 2026-11-01: round 4 - mget argument validation

- Topic: `builtins/TestBuiltin_mget`
- Changes: `crates/robigo-runtime/src/builtins/envir.rs`: port `do_mget` (`src/main/envir.c:2154`) faithfully — no `simple_as_environment` for `envir` ("second argument must be an environment"), vectorized `mode` (length `length(x)` or 1), `ifnotfound` coerced to a list (environments: "environments cannot be coerced to other types"; closures fail as "cannot coerce type 'closure' to vector of type 'list'") with the "wrong length for 'ifnotfound' argument" gate, `inherits` via `asLogical`, and a function `ifnotfound` called in the caller's environment (`gfind`).
- Round 2 of the port: `str2mode` runs before `gfind`'s `install`, so a bogus mode wins over a zero-length name, and an NA mode is the string `"NA"` (`invalid 'mode' argument 'NA'`), not `any`. `str2mode` validation per element (`invalid 'mode' argument 'bogus'`, only when an element reaches it, so `mget(character(0), mode = "bogus")` is a named empty list); the NA-name arm of R's check is dead code (`isNull(STRING_ELT(x, i))` never fires, an NA name is looked up as `"NA"`), and an empty name after the first errors in `install` as `attempt to use zero-length variable name`; only an empty *first* name is `invalid name in position 1`. `tests/r/env/mget.R` pins all of it.
- Result: 31,231 of 31,374 pass on this branch (+4 cases, 0 regressions); `TestBuiltin_mget` 22/22.
- Status: kept.

## 2026-11-01: round 7 - active bindings and methods native routines

- Topic: `S4/TestS4` (6 cases), `library/stats/TestExternal_*` (3 cases)
- Changes:
  - `crates/robigo-runtime/src/builtins/envir.rs`: `do_make_active_binding` ports `R_MakeActiveBinding` (`src/main/envir.c:3460`) — `error()` (context call, not the internal's own), the `symbol already has a regular binding` / `cannot change active binding if binding is locked` / `cannot add bindings to a locked environment` gates in R's order. `do_binding_is_active` ports `R_BindingIsActive` (`envir.c:3518`), also with `error()`, so `.Internal(bindingIsActive(...))` reports `Error: no binding ...` and a closure call keeps its own call.
  - `crates/robigo-runtime/src/s4.rs`: `slot_name`/`do_slot_assign` install the name like `installTrChar` and reject `""` with `attempt to use zero-length variable name` (`src/main/attrib.c:R_do_slot`).
  - `crates/robigo-runtime/src/native/methods.rs`: `R_getClassFromCache` rejects `""` the same way, and `R_set_method_dispatch` uses `asBool` (`coerce.c:1837`: `asLogical2(x, 1, NULL)` — an atomic vector longer than one errors with `'length = N' in coercion to 'logical(1)'`, then NA errors with `NA in coercion to boolean`; `NULL` is NA). `tests/r/s4/set-dispatch.R` pins all three errors (no fastr case covers the length arm).
- Result: 31,240 of 31,374 pass on this branch (+9 cases, 0 regressions). `library/tools/TestExternal_tabExpand` and `library/utils/TestUtils` cases only fail locally: they need the prebuilt native routines CI ships via `ROBIGO_NATIVES`.
- Status: kept (PR pending).

## 2026-11-02: round 8 - file() opens gzipped files through its gz connection

- Topic: `builtins/TestBuiltin_zzfile` (2 of 6 cases)
- Diagnosis: `do_file` rejected every compressed file in read mode. GNU R's `do_file` (`src/main/connections.c:6076-6095`) runs `comp_type_from_file` and, for `open` empty/`r`/`rt`, builds the `gzfile`/`bzfile`/`xzfile`/`zstdfile` connection instead — only the class attribute stays `file`, and an empty `open` becomes `"rt"` (`newgzfile(url, strlen(open) ? open : "rt", 0)`).
- Changes:
  - `crates/robigo-runtime/src/builtins/io.rs`: `compressed_file_type` ports `comp_type_from_memory` with `with_zlib = FALSE` (what `comp_type_from_file` passes), including the ten-byte bzip2 check (`BZh` + block size + the BCD pi / sqrt(pi) block magic, R's PR#18768), the LZMA and `]\0\0\200\0` xz variants, lzop's `\x89LZO` and zstd. `do_file` opens a detected gzip file as a `ConnKind::Gz` connection with class `"file"`, mode `"rt"` when `open` is empty, otherwise the caller's mode; lzop reports R's own unsupported message rather than a robigo stub.
  - `canseek` stays FALSE for the gz connection (robigo's `gzfile` cannot seek; R's can, a pre-existing divergence, not widened here).
  - `tests/r/io/compressed-gzip.R` pins the class, the lazy open, `file(f, "rt")` and the `raw = TRUE` / `"rb"` passthrough paths.
  - `tests/r/io/compressed-bzip2-xz.R` is the gap test for the bzip2/xz branches (GNU R 4.5.0 output recorded, no marker): robigo still errors there, because `bzfile`/`xzfile` would need new compression crates.
- Result: 31,242 of 31,374 pass on this branch (+2 cases, 0 regressions); the remaining four `TestBuiltin_zzfile` cases are the bzip2/xz pair, tracked by the gap test.
- Status: kept.

## 2026-11-02: round 9 - qr_coef_cmplx (complex qr.coef)

- Topic: `builtins/TestBuiltin_qr` (8 cases: 7 were already passing on main and only needed markers, the 8th needed the internal)
- Diagnosis: `qr_coef_cmplx` had no implementation, so `.Internal(qr_coef_cmplx(qr, y))` failed with "there is no .Internal function". `qr.coef` takes that branch whenever `is.complex(qr$qr)` (`rlib/base/R/qr.R:75`), before the `useLAPACK` test.
- Changes:
  - `crates/robigo-runtime/src/builtins/lapack.rs`: `do_qr_coef_cmplx` ports `qr_coef_cmplx` (`src/modules/lapack/Lapack.c:806`) — `zunmqr("L", "C", ...)` then `ztrtrs("U", "N", "N", ...)`, with the workspace size read from the real part of the work query as R does. `zunmqr`/`ztrtrs` are resolved with the rest of the system LAPACK routines.
  - `tests/r/linalg/qr-coef-cmplx.R` pins the complex branch on a LAPACK factor (both `qr` and `qraux` made complex, the shape `La_qr_cmplx` produces), the coerced numeric RHS and the `'b' must be a complex matrix` arm.
- Divergence not reproduced: R passes `COMPLEX(tau)` to `zunmqr` without coercing, so a *real* `qraux` (the LINPACK factor) is reinterpreted as pairs of doubles and read past its end. robigo coerces the real `qraux` to complex, which is the only defined reading. The complex branch is reached for real factors only through that undefined path.
- Gap: `qr()` of a complex matrix still needs `La_qr_cmplx` (zgeqp3); recorded in `tests/r/linalg/qr-cmplx.R` (GNU R 4.5.0 output, deliberately unmarked).
- Result: 31,259 of 31,374 pass on this branch (+8 cases, 0 regressions); `TestBuiltin_qr` is 18/18.
- Status: kept (PR pending).

## 2026-11-02: round 10 - pairlists print their attributes

- Topic: `builtins/TestBuiltin_asvector` (4 cases)
- Diagnosis: `print` rebuilt a pairlist as a plain list to get the `$tag` headers and kept only the derived names, dropping every other attribute. GNU R's `PrintValueRec` (`src/main/print.c:909`) calls `printList` and then falls through to `printAttributes`, which hides only the structural attributes (names for a non-array, dim/dimnames for a list).
- Changes:
  - `crates/robigo-runtime/src/print.rs`: the LISTSXP arm now carries the pairlist's attributes (minus the names entry, which is derived from the tags) onto the list it prints.
  - `tests/r/lang/pairlist-print.R` pins the named and unnamed pairlist cases, both `as.vector` modes, the `parse()` expression, and that a class attribute is still printed after the elements.
- Result: +4 cases over main (31,263 of 31,374 by the ratchet), 0 regressions; the print/pairlist/aslist slice of the fastr corpus is 416/416.
- Status: kept (PR pending).
