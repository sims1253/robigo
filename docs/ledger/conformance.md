# conformance: orphaned data cases

## 2026-10-30: repair the complex replacement performance regression

- CI run 41097174768, job 124210107182 reports mandelbrot_noout_naive
  +29.14% at 4144a0e4 against its merge first parent a32c5623. The prior
  scalar assignment/fib screen missed this complex-matrix ownership bug.
- Reproduce with the downloaded exact CI executables in Debian trixie,
  pinned GNU R 4.5.0-3, identical candidate native libraries, isolated
  executable directories, caches disabled and the unchanged paired harness.
  Two-sample minima: 667,535,419 -> 862,127,190 (+29.15%).
- Exact CI profile attribution: total +194,644,018 instructions in sample 1.
  memcpy rises 7,100,903 -> 200,918,637 (+193,817,734). Its caller is
  Rc<Vector<Complex>>::make_mut: 229 copies, 193,818,043 instructions.
  Numeric kernel instruction counts are unchanged. Filling C[, y] after
  its first complex column triggers one full-matrix copy per next column.
- Cause: the restoration guard added in this PR uses Value::ptr_eq, whose
  match omits complex, logical, raw, expression and pairlist values. It
  consequently retains an unchanged binding as an error-restoration owner.
  That extra reference forces copy-on-modify on each replacement.
- Candidate: compare the omitted Rc identities locally in fast_replace,
  releasing an unchanged binding before replacement. Preserve a binding
  only when index evaluation actually changes it. Leave shared value.rs,
  evaluator/context capture and GNU R ordering unchanged. Add an evaluator
  test checking buffer reuse across repeated complex/logical/raw/expression
  replacements. Existing assignment side-effect oracle fixtures remain.
- Fresh base and original-head source archives build sequentially in one
  capped container, each with a separate private Cargo target directory.
  Fresh-build reproduction and final corrected measurements are pending.
- Corrected screening profile: 667,440,293 instructions; memcpy exclusive
  7,101,350, back to the base's 7,100,903. The 229 full-matrix copies are
  gone. All 12 fixed cases and 286 scripts in all three execution tiers
  (858 comparisons) pass with the corrected Debian binary.
- Reject the first fresh-base run as environment-invalid: liblzma-sys had
  bundled its static library before pkg-config was installed. Unlike CI,
  that executable lacked DT_NEEDED liblzma and loading utils failed on
  lzma_crc64. Rebuild that private dependency with the system pkg-config;
  do not count its failed run as a performance result. Both comparison
  executables must load the same system libraries and candidate natives.



### Verified repair

- Fresh base a32c5623 and original head 4144a0e4, isolated private Cargo
  targets, same completed Debian build image and identical system liblzma:
  **667,516,685 -> 862,136,244 (+29.16%)** on the flagged benchmark.
- Final fixed binary compared against fresh base with unchanged
  scripts/perf-paired.py, caches disabled, isolated executable directories,
  identical CI native libraries, same drivers and parameters, and the
  minimum of two samples per executable. Every output matches GNU R.

| Benchmark | Base instructions | Corrected instructions | Change |
| --- | ---: | ---: | ---: |
| rbench/shootout/mandelbrot_noout_naive | 667,522,766 | 667,527,886 | +0.0008% |
| rbench/shootout/mandelbrot_noout | 567,656,487 | 567,640,029 | -0.0029% |
| rbench/shootout/mandelbrot_naive_ascii | 853,934,789 | 853,951,926 | +0.0020% |
| rbench/shootout/mandelbrot_ascii | 483,040,836 | 483,102,211 | +0.0127% |
| robigo/mandelbrot | 254,848,310 | 254,855,181 | +0.0027% |

- **0 slower above 0.5%, 0 broken.** Largest increase +0.0127%; flagged
  mandelbrot_noout_naive is +0.0008%, effectively neutral. Keep the repair.
- Pinned Debian tests, no exclusions: 64 unit tests and all three runtime
  integration tests pass, including the new buffer reuse test and both
  host-sensitive complex/libm comparisons. All 12 fixed corpus cases pass.
  All 286 recorded scripts match in interpreter, VM and JIT (858/858).
- Full Debian data slice: 8,424/8,426, zero failed markers. Only the two
  recorded GNU R crashes remain. The host's 8,423 includes its known
  locale failure; this extra Debian pass is environmental, not a new fix.
- Formatting and marker integrity pass; no recording, mask or marker was
  changed. This repair touches only builtins/lang.rs and this ledger.
- Reproduction artifacts under .robigo/perf123: CI job log and profiles,
  exact binary hashes, fresh build logs, source hash, raw paired samples,
  drivers, output checks and profiles. base-target and head-target are
  separate and private. The corrected source's SHA256 matches the source
  used for the pinned tests. Shared cargo target directories are not used.

## 2026-10-30: revived data-filter ownership

- Start from a32c5623. Claim the 12 actionable cases left by the finished
  language and print tracks. Keep changes to the evaluator's top-level print
  call, replacement assignment, bind dispatch and character coercion.
- Baseline: all 12 actionable orphaned cases fail. Reproduce each with
  `scripts/corpus.py show fastr ID --robigo .robigo/orphaned/base`, with
  R_HOME=/home/m0hawk/.local/opt/R-4.5.0. The full 8,426-case filter from
  conformance-sweep-data.md is running against a saved baseline executable.
- The brief reverses the unused-argument mismatch. The recordings contain
  `unused argument (1)`; baseline robigo prints `(v)`. GNU R's
  eval.c:promiseArgs repromises dots, match.c:matchArgs_NR removes one layer,
  and deparse.c:deparse2buff evaluates the remaining forced promise.
- GNU R print.c:PrintObjectS3 constructs a call with the base print closure
  in its head, rather than the symbol print. errors.c:R_ConciseTraceback
  therefore reports <Anonymous>. Fix the call construction, not the names
  collected by the traceback formatter.
- GNU R eval.c:evalseq saves the assignment target before setter indices
  run. Preserve that value through fast_replace when an index assigns to
  the target. Release the binding's reference before replacement to retain
  copy-on-modify behavior when the index leaves the target alone.
- match.arg already runs GNU R's vendored R code. Its two failures are
  lower-level: coerce.c:coerceVectorList deparses raw list elements, and
  errors.c:getCurrentCall selects the atomic coercion builtin's context.
- Recording artifacts: cbind/61d7e05c8f316038 and rbind/e2d15825d89e98c7
  have recorded status -11. Both segfault again in robigo-oracle:4.5.0
  with the original code (classed numeric v, method bound to length).
  The recordings already mask the address line. corpus.py's masks cover
  stream lines, not exit status; masking cannot make these comparable.
  Keep recordings and markers unchanged. An oracle-format policy for
  expected crashes belongs to the corpus owner.
- order/add626733541c799 remains the documented host-locale artifact.

### First verified candidate

- Actionable orphaned cases: 0/12 -> 12/12. All 12 pass separately with
  ROBIGO_COMPILE=0, with ROBIGO_COMPILE=1/ROBIGO_JIT=0, and with the default
  JIT configuration. Add exactly their 12 corpus markers.
- All 285 default-build recorded scripts match stdout, stderr and exit
  status in each of interpreter, VM and JIT modes: 855/855 comparisons.
  Feature-required scripts are left to their owning feature build.
- The only shared-file change is interp.rs:PrintObjectS3 call construction.
  No changes to value.rs, env.rs, vm.rs, parser/codegen, S4 or rlib.
- Full-filter comparison, runtime tests and paired performance checks are
  still running. Do not treat this checkpoint as a completed CI result.

### Assignment error boundary

- First candidate full filter: 8,411 -> 8,423/8,426, exactly 12 gains and
  zero lost passes. Coercion slice holds 323/323 on both saved binaries.
- Extra boundary probe rejects the first candidate's error restoration:
  `x <- 1:5; try(x[{x <- 9L; c(NA, 1L)}] <- 1:2)` must leave x=9L.
  Saving the target fixes successful replacement, but restoring that saved
  target after failure erases the index's side effect. Preserve the current
  binding separately only when it differs from the saved target.
- Record tests/r/lang/assignment-index-effects.R with the pinned Docker
  oracle. It covers successful nested updates, failure after rebinding,
  failure after removing the binding, and a promised function argument.
  Baseline 0/1 scripts; final candidate verification is pending.
- First-candidate local screening: fib 889,086,614 -> 889,245,621
  (+0.0179%); vector assignment default 484,834,841 -> 484,764,517
  (-0.0145%); interpreter 3,422,350,113 -> 3,412,361,461 (-0.2919%).
  Output matches the pinned oracle; no slowdown above 0.5%. Re-measure
  after the error-restoration correction. CI is the performance judge.

- Corrected assignment fixture: 0/1 -> 1/1, and 3/3 execution tiers.
  All 286 default-build scripts now match the oracle in all three tiers
  (858/858). Add its conformance marker. Final full-filter, unit/integration
  and performance reruns are pending at this checkpoint.
- Initial runtime invocation: 61 unit tests pass; only the two documented
  host c_library_oracle/c_library_trig_oracle tests differ by one bit in
  complex sqrt/sin. Final follow-up excludes only these host-specific
  tests, without changing their source or CI selection.

### Final corrected result

- Full data filter: **8,411 -> 8,423/8,426**, exactly 12 gains and zero
  lost baseline passes. The three remaining IDs are the two reproduced
  GNU R crashes and order/add626733541c799. The order marker fails on both
  host binaries; no marker or oracle output was removed or masked.
- Default-build recorded scripts: **286/286 in each tier, 858/858** total.
  The new oracle-recorded assignment script passes all three tiers after
  regenerating its transcript with cat's explicit separator, avoiding
  trailing whitespace. Adjacent character/numeric coercions hold 323/323.
- Final local instruction screening, identical saved baseline/candidate
  inputs and exact output comparisons against the pinned R oracle:
  fib 889,086,614 -> 889,210,695 (+0.0140%);
  assignment default 484,834,841 -> 484,801,342 (-0.0069%);
  assignment interpreter 3,422,350,113 -> 3,425,897,176 (+0.1036%).
  No slowdown above 0.5%; these small differences are not speedup claims.
  CI's pinned-platform full paired performance gate remains authoritative.
- cargo fmt --all -- --check, git diff --check and scripts/ratchet.py check
  pass. Marker integrity covers 32,574 cases. Exactly 12 corpus markers
  and one conformance-script marker were added.
- Next: coordinator reviews and merges after CI. Corpus owner may define
  a format policy for expected oracle crashes; line masking currently
  cannot mask their -11 exit status. No remaining actionable case needs
  a runtime change in this track.

- Final runtime follow-up passes: 61 unit tests and all three integration
  tests (default bytecode import plus two installed RDB probes). Only the
  two documented host-libm comparisons are filtered locally. Command:
  CARGO_BUILD_JOBS=2 R_HOME=/home/m0hawk/.local/opt/R-4.5.0 cargo test --release
  -p robigo-runtime -- --skip c_library_oracle --skip c_library_trig_oracle.
  The first unfiltered invocation above records their actual failures.
