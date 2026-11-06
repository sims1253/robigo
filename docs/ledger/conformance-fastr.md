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

## 2026-11-02: round 11 - matrix column labels use the escaped width
- Topic: `builtins/TestBuiltin_aperm` (2 cases)
- Diagnosis: the two "logical" aperm cases print a matrix whose dimnames contain `\Sexpr`. R's `_COMPUTE_W2_` (printarray.c) measures a column name with `strwidth()` = `Rstrwid`, which counts a backslash as two because `EncodeString` prints it doubled; robigo measured the raw string, so every column under such a label was one character too narrow.
- Changes:
  - `crates/robigo-runtime/src/print.rs`: `clabw` now measures `escape_rstring(s, None)` — the same escaped width the label is printed with.
  - `tests/r/matrix/dimname-escape.R` (recorded in `robigo-oracle:4.5.0`) pins column labels, row labels, quotes vs backslashes, and the logical/numeric/string matrix paths.
- Result: `TestBuiltin_aperm` 43/44 (+2 cases, 0 regressions); third remaining case is the complex-perm warning of round 12.
- Status: kept (PR #261).

## 2026-11-02: round 12 - terms() accepts a NULL response; aperm's complex perm warns

- Topic: `library/stats/TestUpdateForm` (4 cases), `builtins/TestBuiltin_aperm` (1 case)
- Diagnosis:
  - `update.formula(x ~ y, NULL ~ .)` failed in `terms.formula` with "invalid term in model formula": the parser puts R_NilValue in the language for a literal `NULL`, and R's `InstallVar` accepts it because `Rf_isLanguage(R_NilValue)` is true (verified against libR in `robigo-oracle:4.5.0`). robigo's `install_var` rejected `Value::Null`.
  - `aperm(a, c(1+1i, 3+3i, 2+2i))` was missing GNU R's `imaginary parts discarded in coercion` warning: `do_aperm` uses `coerceVector(perm, INTSXP)` (array.c:1734), whose warnings fire; robigo used quiet coercion.
- Changes:
  - `crates/robigo-runtime/src/native/stats/model.rs`: `install_var` accepts `Value::Null` (comment explains the isLanguage quirk); `match_var` already handled Null both ways.
  - `crates/robigo-runtime/src/builtins/array.rs`: `do_aperm` takes the interp and coerces `perm` through `interp.coerce`, which emits R's coercion warnings against the calling closure.
  - `tests/r/packages/stats-terms.R` + `tests/r/io/matrix.R` extended and re-recorded in the oracle: NULL response installs a "NULL" variable (`res[[2]] <- NULL` then drops the lhs in `simplify`), 0/1/TRUE responses stay, 2 and "a" still error; the complex perm warns and permutes by real parts.
- Result: `TestUpdateForm` 4/4, `TestBuiltin_aperm` 44/44 counting PR #261's print fix (+5 fixed here; the two logical aperm cases need #261); 10 more markers absorbed (deriv 7, TestStats 3) that pass on main where CI ships natives. 0 regressions.
## 2026-11-02: round 13 - runtime installs reject the empty name

- Topic: zero-length variable names (4 cases: `TestSimpleAssignment` 2, `TestSimpleParsing` 1, `TestParser` 1)
- Diagnosis: the parser keeps `'' <- 123` as a string lhs (robigo already parses it like GNU R), but every runtime site that *installs* a name goes through `installTrChar` (`names.c:install`): the empty string errors "attempt to use zero-length variable name" and names over MAXIDSIZE (10000 bytes) error "variable names are limited to 10000 bytes". robigo interned the empty symbol silently. (`switch(x, ""=v77)` was already correct: it errors at parse time.)
- Changes:
  - `crates/robigo-runtime/src/sym.rs`: `Sym::install_tr` ports the two guards (plain `error()`, reported against the context call).
  - `crates/robigo-runtime/src/builtins/lang.rs`: `assign_target` — a string lhs installs through `install_tr` (`eval.c:do_set`).
  - `crates/robigo-runtime/src/builtins/coerce.rs`: `as.symbol`/`as.name` of "" errors (`coerceToSymbol`).
  - `crates/robigo-runtime/src/builtins/envir.rs`: `assign`, `makeActiveBinding` and `delayedAssign` install their names through `install_tr` (verified against the oracle: all three error for "" and 10001-byte names).
  - `tests/r/lang/empty-name.R` (recorded in `robigo-oracle:4.5.0`) pins all five sites, the 10000/10001 boundary, and that `parse` itself still accepts the empty lhs.
- Result: 4 cases pass, 0 regressions (`TestSimpleAssignment|TestSimpleParsing|TestParser` slice green; attach/env2list/ls/postoenv fail for unrelated argument-validation reasons).
## 2026-11-02: round 14 - tracemem reports copies of traced objects

- Topic: `builtins/TestBuiltin_tracemem` (4 cases)
- Diagnosis: `tracemem()` itself, `untracemem()` and the address strings existed, but the copy report (`memory.c:memtrace_report`) was missing, so `x[[1]] <- 42` on a vector shared with a traced binding printed nothing. The corpus masks every tracemem line wholesale (addresses vary per run), so what must match is the line structure: one `tracemem[0x... -> 0x...]: ` line per replacement-copy of a traced object, an empty context for top-level statements (R prints the byte-code trace buffer), and `tracemem[<previous> -> ...]` from `retracemem`. Plain assignment (`x <- v`) never copies, so it never reports — robigo's Rc sharing already matches.
- Changes:
  - `crates/robigo-runtime/src/builtins/debug.rs`: a `TRACING` flag (`R_current_trace_state`), maintained by tracemem/untracemem/retracemem; `make_mut_reported` = `Rc::make_mut` that reports `tracemem[0x{:x} -> 0x{:x}]: ` for a traced source (the `#[cold] #[inline(never)]` path); `retracemem(x, previous)` prints its `[<previous> -> new]` line.
  - `crates/robigo-runtime/src/builtins/subassign.rs`: `assign_values` (the element loops of `[<-`/`[[<-`) routes through the reporting make_mut. The flag check sits once at the function entry — the traced variant is a `#[cold]` duplicate and the plain body is unchanged, keeping the hot arms' codegen identical (perf below).
  - `tests/r/debug/tracemem-report.R` (recorded in `robigo-oracle:4.5.0`): six sink-captured scenarios with the addresses normalized (`0xADDR`): two copies, untracemem stopping reports, the list path, retracemem's bracketed source, `[<-`, and a no-copy control; plus the four error arms.
- Perf (instruction counts, `scripts/perf.py --metric instr`): with the check inlined in every arm fannkuch read +0.37% — but against a stale base; re-measured against the proper control (main + #264, the tree CI will diff), the entry-branch restructure is free: binarytrees +0.001%, closures -0.001%, fannkuch -0.000%, nbody -0.006%. Not reported (kept out of this round): copies in recursive `[[<-` paths (`elt_mut`), `$<-`, `names<-`/attribute writes (`value.rs` `attrs_mut` — hot path), and R's trace-bit inheritance by duplicates — none covered by the corpus; noted for a later round.
- Status: kept (PR pending).

## 2026-11-03: round 15 - envir/diag argument validation and coercion warnings

- Topic: `TestBuiltin_attach` (2), `TestBuiltin_env2list` (2), `TestBuiltin_ls` (1), `TestBuiltin_postoenv` (1), `TestBuiltin_diag` (2)
- Diagnosis: five internal argument paths diverged from GNU R's C:
  - `do_attach` (envir.c:2537): `pos` goes through `asInteger` (so a string `pos` warns `NAs introduced by coercion` before the error) and `name` through `isValidStringF`; both errors are plain `error()` (the attach closure's call), while robigo reported the `.Internal`'s own call and skipped the warning and the name check.
  - `R_lsInternal3`: a non-environment is a plain `Error: invalid 'envir' argument` (no call), and `all.names`/`sorted` go through `asLogical` with NA as FALSE, not strict validation.
  - `do_env2list`: a non-environment errors `argument must be an environment` (robigo accepted a list); `all.names`/`sorted` asLogical like ls (robigo rejected `'string'`).
  - `do_pos2env`: `coerceVector(x, INTSXP)` warns before the per-position validation.
  - `do_diag`: the default arm is `coerceVector(x, REALSXP)`, so a string `x` warns `NAs introduced by coercion` (robigo coerced quietly).
- Changes: `builtins/namespace.rs` (`do_attach`), `builtins/envir.rs` (`do_ls`, `do_env2list`, `do_pos_to_env`), `builtins/array.rs` (`do_diag`); `tests/r/env/attach-args.R` (recorded in `robigo-oracle:4.5.0`) pins all five.
- Result: +8 fastr cases (54/54 slice, 0 regressions; the ls/env/attach neighborhood 242/242); `tests/r/env` conformance 9/9.
- Status: kept (PR pending).

## 2026-11-03: round 16 - is.* predicates and time limits

- Topic: `TestBuiltin_isinteger` (1), `TestBuiltin_isSingle` (1), `TestBuiltin_setTimeLimit` (1), plus markers absorbed for `TestBuiltin_isarray`/`TestBuiltin_ismatrix`/`TestBuiltin_pcre_config` (3, passing on main).
- Diagnosis:
  - `is.integer(factor(...))` returned TRUE: `do_is`'s INTSXP arm is `TYPEOF == INTSXP && !inherits("factor")` (coerce.c:2057).
  - `is.single()` with zero arguments reported the constant "single" error instead of the arity error: `checkArity` runs first.
  - `setTimeLimit`/`setSessionTimeLimit` had no internals. Ported `do_setTimeLimit` and `do_setSessionTimeLimit` (sysutils.c:2837/2867): finite positive limits are kept, anything else disables, `transient` restores, session limits are absolute against proc.time. The limits are stored (sysutils.c's `cpuLimitValue`/`elapsedLimitValue`/`cpuLimit2`/`elapsedLimit2`); the evaluator does not poll them yet (R_CheckTimeLimits), which no corpus case observes.
- Changes: `builtins/coerce.rs` (`is_integer`, `is_single`), `builtins/sysutils.rs` (the two internals); `tests/r/basics/is-predicates.R` (recorded in `robigo-oracle:4.5.0`).
- Result: +3 fixed, +3 absorbed, 92/92 slice, 0 regressions.
- Status: kept (PR pending).

## 2026-11-03: deferred — in-place hand-off for complex assignment (blocks #272)

`a$x[[1]] <- v` with a uniquely-held container: GNU R mutates the element in place (the replacement path uses `lazy_duplicate`, which only ENSURE_NAMEDMAXes — duplicate.c:171 — and never copies or reports), while robigo's general complex-assign path extracts `a$x` into `*tmp*` keeping the container's reference, so `Rc::make_mut` duplicates (and #272's report correctly fires, regressing 5 `TestSimpleLists` cases that the invisible copy used to pass). Fix plan: hand the replacement its object uniquely when the container is uniquely held — take the element out of the container (AGENTS.md §5) for the `$`/`[[` getter levels, restore on error, write back through the existing setter chain. Also a perf win (one copy per complex assignment).
## 2026-11-03: round 17 - in-place hand-off for complex assignment

- Topic: the deferred fix from round 14 (unblocks #272's 4 `TestBuiltin_tracemem` cases and the 5 `TestSimpleLists` cases they regressed).
- Diagnosis: GNU R hands a replacement the element of a container without copying (the replacement path's `lazy_duplicate` only ENSURE_NAMEDMAXes, duplicate.c:171), so `a$x[[1]] <- v` mutates `a$x` in place. robigo's general complex-assign path extracted `a$x` into `*tmp*` keeping the container's reference, so `Rc::make_mut` duplicated the element on every nested assignment — invisible until tracemem reported it.
- Changes (`builtins/lang.rs`): `complex_assign_inplace` takes the target's binding out (`take_local`) and walks the getter chain (`$`/`[[` builtins over classless list containers, slot found by allocation identity, `same_allocation`), taking each element out of its uniquely-held container (`sole_allocation`); the innermost builtin replacement then mutates in place and the chain is healed back into the binding. Aliases of the container keep the old value (the spine copy `Rc::make_mut` makes is GNU R's own `$<-` duplicate); anything unsupported (classed values, other getters, special setters, superassignment, active bindings) falls back to the general path with the binding untouched. `complex_assign_inner` runs it before taking `base_val`'s clone (the general path re-acquires on fallback).
- Verification: `tests/r/lang/complex-assign-inplace.R` (recorded in `robigo-oracle:4.5.0`) — four sink-captured tracemem scenarios print no report, alias isolation, a three-level chain, name-keyed `[[`, `[[<-.foo` dispatch, and error restore — robigo is byte-identical to GNU R on all of them. fastr slices (`TestSimpleLists|TestSimpleAssignment|TestSimple*|classassign|extract_replace|dqr|TestStackBuiltins`): 0 regressions; TestSimpleLists all pass.
- Perf (callgrind instr, vs a fresh origin/main control): binarytrees +0.012%, closures +0.064%, fannkuch +0.004%, nbody +0.016%.
- Status: kept (PR pending).

## 2026-11-03: round 17 addendum - the VM tier's register cache blocks uniqueness

(An earlier shape of the change double-evaluated the target for every complex assignment, which CI's perf gate measured as +0.775% on `rbench/shootout/nbody_naive_2` — its hot loop is a `body_v[, i] <<- ...` superassignment. The attempt now runs before any acquisition, so the other paths pay nothing.)

Debugging the stacked tracemem round showed the take-out works at the interpreter level but the element still has count 3 when the replacement runs in the VM tier: `Op::SetDollar`/`SetIndex` code keeps **unboxed register caches** of values it evaluated (`tracemem(b$x)`'s argument and the previous assignment's result sit in `regs` for the life of the code object), so `sole_allocation` fails and the path falls back to the general (copying) path. The interpreter-level path engages only where no register cached the element; GNU R's refcounting has no such holders (its registers PROTECT rather than REFCNT).

Completing the fix for the corpus needs one of:
- vm.rs register-lifetime work: release (or re-box) element caches around complex assignments — perf-critical (the #252 lesson applies; measure fannkuch/nbody/binarytrees/closures);
- or an object-model change (elements reachable through shared storage with interior mutability) — a DESIGN DECISION.

Until then #272 stays parked: its 5 `TestSimpleLists` regressions are exactly this copy, now understood and instrumented.

### Round 14 addendum (2026-11-06): the 5 TestSimpleLists regressions

Rebased on main (#298's take-out path is merged). The copy that made
`l$x[2:9] <- 42` report was not the VM's registers any more: the in-place
path of complex assignment (lang.rs `complex_assign_inplace`) built the
setter's call object from `argv.clone()`, so the call held the element
and the replacement duplicated it, in every tier. The call now names the
object `*tmp*`, as fast_replace's and GNU R's do; `l$x[i] <- v`,
`l$x[[i]] <- v`, `l[["x"]][i] <- v` and `l$a$x[i] <- v` modify in place
(no report), a list shared with another binding still copies (report).
Growing a shared vector reports too (enlarge_vector, R's EnlargeVector
duplicate). The plain and traced element loops come from one macro.
New `tests/r/debug/tracemem-inplace.R` (GNU R 4.5.0 recording). fastr
TestBuiltin_tracemem + TestSimpleLists: 89/89.
