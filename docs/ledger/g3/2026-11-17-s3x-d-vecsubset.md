## 2026-11-17: S3x-d, byte code's VECSUBSET fast path

- What: the larger part of trycatch's error branch, the foreman's S3x-d
  (d). It follows S3x-d's builtins (#806) because it touches `vm.rs`, and
  is measured on top of AC1 (#803) and C1's MATH1 paths (#812).
- **VECSUBSET's fast path** (eval.c:VECSUBSET_PTR, DO_FAST_VECELT,
  FAST_VECELT_OK, bcStackIndex). `x[i]` and `x[[i]]` in byte code always
  went to `do_subset_dflt` / `do_subset2_dflt`, which boxed the index.
  GNU R reads the element in the byte-code loop. Now `vm::vec_subset_fast`
  does the same from `dflt_subset`:
  - one unnamed index (cmp.R compiles every other form to MATSUBSET,
    SUBSET_N or DFLTSUBSET);
  - `bcStackIndex`: an unboxed or plain scalar integer (not NA) or
    double (not NaN, above 0, at most `R_XLEN_T_MAX`); anything else is
    -1;
  - for `[`, `FAST_VECELT_OK`: no attributes, or only `dim`;
  - REAL, INT and LGL go into the register unboxed, CPLX and RAW are a
    new scalar, and a list's element is the element itself for `[[` and a
    list of one for `[`;
  - out of range, or any other type, goes to the default, which raises
    the error.
- **ALTREP vectors take the default**, at the foreman's request: no
  ALTREP method runs from the byte-code loop. GNU R 4.6.2 differs here.
  `FAST_VECELT_OK` checks only the attributes, and `DO_FAST_VECELT` reads
  an ALTREP vector through `REAL_ELT` / `INTEGER_ELT`, its class's `Elt`
  method. The values are the same. Reading AC1's own compact sequences
  in the loop is queued for AC2. An index that is ALTREP is -1 in both,
  since `IS_SCALAR` reads the scalar bit that only `allocVector3` sets.
- **Out of `vm::run`**, as the foreman asked. `dflt_subset` is
  `#[cold] #[inline(never)]`, and the fast path is its own
  `#[inline(never)]` function.
- **Two pins.** `store_scalar` and `set_cell_value_raw` are now
  `#[inline(always)]`: the VM's local stores and SETVAR. Without them,
  LLVM moves `store_scalar` out of `vm::run` once `dflt_subset` leaves
  it: +19 instructions per call of `plain` and of `env-fn`, measured on
  this branch with the pins removed.
- Case: `tests/rt/subset/vecsubset-fast`, recorded on GNU R 4.6.2 and
  ratcheted. It covers:
  - each type with `[` and `[[`;
  - integer, double and boxed indices;
  - names and `dim`;
  - NA, 0, negative, logical and out-of-range indices, with their errors;
  - an ALTREP `1:10`;
  - character vectors;
  - copies.
- Measurements: callgrind instructions per iteration
  (`scripts/callgrind-diff.py`, `ROBIGO_COMPILE=1`, 2N against N). The
  WSL2 workstation, Ryzen 9 5900X, rustc 1.100.0. Release `robigo-rt` at
  `64ba18bb` against main `2a7f3be6` with #806 merged, which is main once
  #806 lands.

  | row | before | after | change | `vm::run` itself |
  |---|---:|---:|---:|---:|
  | trycatch-err | 177,512 | 174,068 | -1.94% | -838 |
  | trycatch | 115,685 | 113,529 | -1.86% | -495 |
  | plain | 1,901 | 1,894 | -0.37% | -8 |
  | env-fn | 1,948 | 1,929 | -0.98% | -19 |
  | superassign | 4,909 | 4,895 | -0.29% | -14 |
  | s3 | 18,894 | 18,886 | -0.04% | -8 |
  | `s + x[2L]` in a loop | 1,362 | 963 | -29.3% | -146 |

  - In the error branch, four of the six `do_subset2_dflt` calls are now
    byte code's, as in GNU R: `do_subset2_dflt` -1,433, `get1index`
    -574, the boxing of indices -200, malloc and free -489.
  - The last row is `x <- c(1.5, 2.5, 3.5)` read by `[` in a loop. The
    gate's rows use `[[` only. Sol's review found that the first version
    took `[`'s fast path only for an object with no attribute field at
    all: `Ref::attrib()` gives `R_NilValue` for a vector without
    attributes. So `[` always went to the default, now through one more
    call: 1,477 instructions there, +8.4%. `FAST_VECELT_OK` now accepts
    `ATTRIB(vec) == R_NilValue`, as GNU R's does.
  - `scripts/perf.py rt-bars` against the same base reports 0 failures:
    - plain -0.36%, defaults -0.40%, named -0.42%, env-fn -1.00%,
      superassign -0.30%, binarytrees -0.23%;
    - s3 +0.00%, fib +0.02%, the e7 rows -0.04%.
- Proof: see the PR.
