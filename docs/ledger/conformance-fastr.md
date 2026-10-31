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
- Status: kept.
