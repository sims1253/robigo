# conformance: orphaned data cases

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
