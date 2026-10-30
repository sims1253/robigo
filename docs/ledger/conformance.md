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
