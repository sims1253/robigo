# methods (G6: methods/S4)

Metric: fastr S4 cases and S4-related builtins, gnur-examples `^(methods|stats4)/`,
`library(methods)` + Rcpp loading, then the corpora with methods as a default
package; all byte-identical to GNU R 4.5.0, no regressions.

Slice used throughout ("S4 slice"):
`scripts/corpus.py run fastr --filter 'S4|TestBuiltin_(setS4Object|isS4|slot|attr|standardGeneric|isVirtualClass|asS4|new|setClass|setGeneric|setMethod|validObject|methods|getClass|is\b|inherits|existsMethod|getGeneric|showMethods|selectMethod|removeMethod|isGeneric|unclass|oldClass|class)'`

## 2026-10-30: round 8 — native write-back and VM slot handoff

- Branch `track/methods-slot-bridge`, PR #101. Paired measurements
  below use d2ce510; rebased onto 1b4670a after #94/#96/#97 landed.
  #71 is untouched; no vm.rs, value.rs, or serialization edits.
- Port basis: memory.c:SET_STRING_ELT/SET_VECTOR_ELT mutate the existing
  SEXP; attrib.c:installAttrib and R_do_slot_assign do likewise.
  duplicate.c:duplicate1 recursively copies children for deep duplication,
  while shallow duplication shares them. The core duplication already
  does that; the bridge lost the shared child's write on return.
- Fix: synchronize converted children as well as arguments, write back
  lists, strings and attributes, and preserve existing child identities
  through a reverse SEXP map. Include OBJSXP in bridge identity/sharing.
- Fixture `tests/r/capi/write-back.R` builds a small public-function-ABI
  native library, avoiding dependence on a preinstalled Rcpp. It covers
  list aliases, character writes, matrix attributes, S4 slots, and
  deep/shallow duplication. Recorded with Debian GNU R 4.5.0-3's copied
  libR and private ICU 76, never the host's 4.6.1 wrapper.
- Guarded VM handoff: compile literal-symbol `x@slot` under the existing
  primitive-binding guard. Read/force x once, then require methods enabled
  and an S4 object. For ordinary names, use do_slot (including pseudo-NULL,
  missing-slot and .S3Class behavior); defer .Data to the slow path because
  data_part can evaluate R. Non-S4 objects must fall back to do_AT for S3
  dispatch and exact errors. Guard misses must not evaluate x twice; pass
  the evaluated value in a forced promise to the primitive fallback.
  Restrict the first fast path to present ordinary slots; missing-slot
  errors should retain do_AT's context. Object forcing can run R: invalidate
  VM variable caches afterwards, but keep the primitive selected before
  forcing for this call (do not look up a newly rebound @). Preserve
  visibility. Existing entry guards account for rebinding @. No cached attribute offset without checking the
  actual attribute layout: same-class objects can have different layouts.
- Method-table lookup caching remains a separate experiment: cache against
  both table identity and environment mutation version, including changes
  in inherited tables and method bindings. Cache metadata, never a method
  result across setMethod/removeMethod without invalidation. C API
  attribute writes can mutate metadata without rebinding an environment
  variable, so an environment version alone cannot guard such metadata.
- Rcpp classification: listof 4/5 (list elements), matrix 30/31/34
  (dimnames), S4 2/3 (slot attributes), vector 116/120/135 (character
  element writes): ten bridge bugs. misc 51 is connection message sinking:
  plain message() under sink(type="message") also bypasses the sink,
  with no C/Rcpp involved; hand off builtins/io.rs:cat_output/conn_write
  to its owner. xptr 7/8 also fail in the same GNU R shim. No failing
  comparison requires exposing undocumented GNU R layout internals.
- Baseline full existing conformance ratchet: 241 scripts x 3 tiers all
  pass. New native write-back fixture fails in all three tiers. C API
  corpus 19/21 (objects and strings fail); S4 corpus 644/662. All result
  sets saved before candidate runs, so compare identities too.
- Rejected measurement setup: moving a saved robigo executable to /tmp
  discovers /tmp/natives (unrelated experiments) and fails to load methods
  with undefined DDVAL. Save binaries under the worktree .robigo/bin.
  Discarded those Rcpp/instruction runs. Oracle measurements use the
  direct R executable with copied libR + ICU, not bin/R's host wrapper.
- Native message sink reproduction: direct message("direct") and a
  C Rf_eval(message("native"), R_BaseEnv), while a textConnection is the
  message sink, both print to stderr and leave the connection empty.
  This is independent of the SEXP write-back bug; no io.rs edits here.
- Rcpp baseline: 1,610 PASS / 13 FAIL, sugar 493/493, no PROTECT-stack
  imbalance warnings. The diagnostic runner printed sys.calls on failures
  and exited with a parser error after the Matrix file; reran that file
  with the original shim: exit 0, same 80 assertion records byte-for-byte.
  Use the original shim for the candidate.
- Candidate conformance: **242/242 scripts in each of AST/VM/JIT**;
  fixture added to passing.txt (minimal shared ratchet edit). S4
  **644/662 -> 644/662**, C API **19/21 -> 19/21**, methods/stats4
  examples **45/50**, all zero regressions. Saved result identities
  confirm no previously passing S4/C API case was lost.

| workload | robigo before | robigo after | GNU R, same host | before/R | after/R |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic | 11,284.436 | 11,288.298 | 12,193.281 | 0.925 | 0.926 |
| slot | 1,366.447 | 1,372.986 | 656.124 | 2.083 | 2.093 |
| replace | 39,153.106 | 39,179.670 | 38,692.237 | 1.012 | 1.013 |
| new | 353,364.510 | 353,522.362 | 315,224.912 | 1.121 | 1.121 |
| valid | 179,077.188 | 178,996.576 | 163,136.676 | 1.098 | 1.097 |

- Paired callgrind, 2N minus N iterations after identical setup, as in
  round 7; no S4 hot-path edits. All changes are below the 0.5% gate
  (slot read +0.479%, generic +0.034%, replacement +0.068%, new +0.045%,
  validation -0.045%). Rank remains slot read, new, validObject,
  replacement, generic. Slot fast path is a handoff, not a claimed win.

- Full candidate Rcpp: **1,610 pass / 13 fail -> 1,620 pass / 3 fail**,
  exactly ten FAIL->PASS records and no PASS->FAIL or missing records.
  Sugar remains **493/493**, all files complete with exit 0 and no
  stack-imbalance warnings. Fixes are the ten assertions classified above.
- Corrected XPtr classification: both failures were child R processes
  reporting "there is no package called 'Rcpp'". Exporting
  `R_LIBS=/tmp/methods7/lib` makes XPtr **8/8 on baseline and candidate**.
  Replacing those two harness failures gives **1,612/11 ->
  1,622/1**; the sole remaining runtime failure is message sinking.
  Also reran the S4 file freshly on the copied GNU R 4.5.0 executable:
  **20/20**, matching the candidate; the old full-oracle S4 failures
  are not reproducible in isolation and are not a spec to emulate.
- Message-sink handoff: in builtins/io.rs, `conn_write` treats terminal
  stderr directly, bypassing `Interp::write_err`'s error_con diversion.
  Reproduce with `out <- character(); con <- textConnection("out", "w",
  local=TRUE); sink(con, type="message"); message("direct");
  sink(type="message"); close(con); print(out)`: out is empty on robigo,
  `"direct"` on GNU R. Fix belongs to the connections owner.
- Remaining bridge work beyond these assertions: immediate visibility of
  C writes during C->R callbacks, `.External` write-back, and aliasing of
  newly created nested C objects across returns. These need separate
  fixtures; do not claim the transitional bridge is a shared object heap.

- Rebased verification on 1b4670a: **248/248 scripts in AST/VM/JIT**,
  S4 **644/662**, C API **19/21**, methods/stats4 examples **45/50**,
  zero regressions. `cargo test --release -p robigo-capi` passes all
  unit/integration/doc tests under the build lock. The first unlocked
  invocation passed both integration tests, then rustdoc lost its runtime
  .rlib to shared-target turnover; rejected that failed invocation and
  reran build + tests within one lock. Formatting and whitespace pass.
- Rebased focused Rcpp (listof, matrix, misc, modref, S4, sugar, vector,
  XPtr) with R_LIBS exported: **819 pass / 1 fail**, all eight files exit 0;
  sugar **493/493**, S4 **20/20**, XPtr **8/8**. The only failure is
  misc 51, the independently reproduced message sink. No stack imbalance.
- Current rebased-head instruction ratios (the isolated bridge experiment
  is the before/after table above; #97's dispatch gain is inherited here):

| workload | rebased head | GNU R, same host | head/R |
| --- | ---: | ---: | ---: |
| generic | 10,893.902 | 12,193.281 | 0.893 |
| slot | 1,378.657 | 656.124 | 2.101 |
| replace | 39,126.714 | 38,692.237 | 1.011 |
| new | 352,891.758 | 315,224.912 | 1.119 |
| valid | 178,971.906 | 163,136.676 | 1.097 |

## 2026-10-30: round 7 Rcpp retest and S4 dispatch allocations

- Branch `track/methods-s4-fastpaths`; #71 is untouched. Measurements and
  local checks below use df3d535 plus this change. Rebased cleanly onto
  d2ce510 (#95 iconv) for delivery; Debian CI verifies the rebased head.
- Rcpp 1.1.2 downloaded from CRAN and installed from source with
  `R CMD INSTALL --preclean --install-tests` in `robigo-oracle:4.5.0`.
  Installation and both package load checks passed.
- Added five `benchmarks/suite/s4-*.R` workloads so the perf suite covers
  generic dispatch, slot reads, slot replacement, construction, and validation.
- GNU R 4.5.0-3 callgrind baseline, subtracting N from 2N iterations
  after identical class and method setup, instructions per iteration:
  generic 12,202.615; slot read 656.128; replacement 38,676.816;
  new 315,412.464; validObject 163,206.497.
  N is respectively 10,000, 10,000, 1,000, 500, and 1,000.
- Environment trap: the copied `bin/R` ignores R_HOME and launches the host
  R 4.6.1. Its direct executable also resolves the host libR unless
  LD_LIBRARY_PATH is set, after which Debian ICU dependencies are missing.
  Initial GNU R measurements and source installation use the oracle
  container; host robigo uses the specified R_HOME.
- Current-main S4 slice: 644/662, zero regressions.
- The same-host GNU R executable works with its copied libR plus private
  Debian ICU 76 libraries extracted from the oracle container. Set
  LD_LIBRARY_PATH to those two directories, keep R_HOME as specified,
  and disable DEBUGINFOD_URLS so callgrind does not wait for remote symbols.
  Paired counts on that executable are the comparison below.

| workload | robigo before | robigo after | GNU R, same host | before/R | after/R |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic | 11,284.008 | 10,890.087 | 12,193.279 | 0.925 | 0.893 |
| slot read (median of 3) | 1,374.391 | 1,378.448 | 656.128 | 2.095 | 2.101 |
| replacement | 39,163.031 | 39,212.513 | 38,692.233 | 1.012 | 1.013 |
| new | 353,574.020 | 353,097.786 | 314,889.252 | 1.123 | 1.121 |
| validObject | 179,021.625 | 179,069.409 | 163,059.674 | 1.098 | 1.098 |

- Generic dispatch improves **3.49%**. Other changes are below 0.5%.
  Slot reads initially varied by 1%; three additional paired runs gave
  base 1380.310/1362.857/1374.391 and candidate
  1378.113/1379.145/1378.448, median +0.30%. Kept.
- Reproduce with `scripts/s4-instructions.py --output before.json`, then
  the same command against the candidate. `--binary` selects a saved
  worktree binary; `--filter` selects a workload. Both runtimes use the
  same generated script and R_HOME. All counts exclude startup by subtraction.

- Rank by ratio: slot read, new, validObject, replacement, generic.
  Slot reads spend 212 instructions in VM run_with and 95 in Vars::flush,
  versus 71 in do_at and 34 in do_slot. Construction spends 46k in
  run_with, 10.7k in flush, and 8.2k in Value::clone. The next substantial
  slot improvement needs a guarded VM slot-read instruction, owned by
  perf-vm; value representation remains with core.
- Port follows R_loadMethod's ATTRIB walk and R_dispatchGeneric's
  VECTOR_ELT access: borrow the attribute and signature slices rather than
  cloning their vectors. Method table lookups and the loadMethod fallback
  remain live on every call. No shared runtime files changed.
- Verification: S4 slice **644/662 -> 644/662**, methods/stats4 examples
  **45/50 -> 45/50**, zero regressions. All **45** S4 conformance checks
  pass across interpreter, VM and JIT. Runtime unit tests: **41 pass / 2
  known host failures**, c_library_oracle and c_library_trig_oracle, the
  documented glibc 2.44 versus Debian 2.41 last-bit differences in unchanged
  complex.rs. Debian CI is the judge for these; the initial benchmark-only
  commit's CI build passed the full unit suite. cargo fmt --check and
  git diff --check pass.
- Rcpp 1.1.2, all test files with `scripts/rcpp-mini.R` expectations:
  GNU R **1,622 pass / 4 fail**, current main **1,610 pass / 13 fail**,
  no crashes. Sugar is **493/493** on both. The pass gap remains 12,
  as in round 4; total assertion counts differ from that run by two on
  both runtimes. Failures: listof 2, matrix 3, misc 1, S4 2, vector 3,
  xptr 2. GNU R also fails two S4 and two xptr assertions, at different
  S4 positions. The shim deliberately does not check warning/error
  patterns; these counts describe the same shim on both runtimes.
- Candidate Rcpp sugar/S4/modref: **517/517 assertion records identical**
  to current main, including sugar **493/493**. These cover the dispatch
  path changed here; the full run above used the saved main binary.
  The committed runner also passes a fresh function-helper smoke check,
  **22/22**, with Debian-compiled helpers.
- Rcpp toolchain trap: sourceCpp uses R.home("bin")/R, which was still
  the host wrapper. Helpers compiled with host R 4.6 headers imported
  R_getRegisteredNamespace and also produced parser-looking failures.
  Rejected those measurements. Compiling helpers with Debian R 4.5.0
  fixes the imports and the parser-looking failures without runtime edits.
  `scripts/rcpp-mini.R` accepts RCPP_TEST_R_BIN for a compatible compiler
  wrapper. A container wrapper must preserve CLINK_CPPFLAGS, PKG_CPPFLAGS,
  PKG_LIBS, PKG_CXXFLAGS, R_LIBS and USE_CXX* flags, mount /tmp at the same
  path, and retain the current directory. Do not pass --args to robigo's
  script invocation: it becomes a literal trailing argument.
- Open: the largest gap needs perf-vm's guarded slot-read instruction.
  Native helpers still have GNU R static caches to port per interpreter,
  notably attrib.c:check_slot_assign's checkAtAssignment closure; measure
  the S4State/Interp layout impact before retaining that change. Rcpp's
  remaining failures need C API bridge alias/write-back and vector work.

## 2026-10-29: round 4 — the reg-packages blank line; complex Math group; Rcpp sugar 493/493

Branch `track/methods-install-newline` (PR #87), from main 74e8c73.

- **The reg-packages blank line was `system(intern = TRUE)` swallowing the
  child's stderr.** The line is `R CMD build`'s closing `message("")`
  (tools' build.R, literally commented "blank line"): reg-packages runs
  build via `system(..., intern = TRUE, ignore.stderr = FALSE)`, and GNU
  R's do_system pipes stdout only (`R_popen(cmd, "r")`), leaving stderr
  inherited. robigo's `Command::output()` piped stderr as well and
  discarded it, so every intern child's stderr vanished. Fixed with
  `Stdio::inherit()` on stderr; `tests/r/cli/system-intern.R` (recorded
  against Debian R 4.5.0-3, seccomp unconfined) covers capture-passthrough,
  `ignore.stderr`, `system2` variants and the status attribute+warning;
  ratcheted. Verified end to end in the oracle container: robigo running
  `R CMD build` as a child now shows the blank line.
- **Re-measured Rcpp test_sugar (task b): the blocker was the complex
  Math group, not statics/env ordering.** Fresh Rcpp 1.1.2 built inside
  robigo against this worktree's home (tools.so from /tmp/m4/natives for
  `Rmd5`); the mini-runner reproduced 176 passes then
  "unimplemented complex function" aborting the file, at the
  `runit_complex` expression — cos/sin/tan/acos/asin/atan/cosh/sinh/
  tanh/asinh/atanh all lacked complex kernels (only log/exp/sqrt were
  wired).
- **Ported glibc 2.41's complex trig/hyperbolic kernels + R's wrappers**
  (`complex.rs`): s_ccos/s_csin/s_ctan/s_ccosh/s_csinh/s_ctanh, the
  k_casinh kernel, s_casinh/s_casin/s_cacos/s_catan, and complex.c's
  z_tan/z_acos/z_asin/z_atan plus z_acosh/z_asinh/z_atanh as the
  compiled `z_acos(z) * I` products (cmul/cadd — ri*0 propagates ri's
  NaN into the real part, which the recorded bits show). Wired into
  `math1_c_fn`; the NA/warning/attribute layer was already right.
  Verified: (1) `scripts/oracle-complex-trig.c` recorded in the
  robigo-oracle:4.5.0 container, 384,640 results compared bit-exactly
  by the new `c_library_trig_oracle` unit test (CI green; on this host
  1.5% of rows drift <=4 ULP — the glibc 2.44-vs-2.41 #47 category);
  (2) the same transcription compiled in C inside the container matches
  the container glibc's own functions over 18M inputs, 0 diffs;
  (3) `tests/r/math/complex-trig.R` recorded from Debian R 4.5.0-3
  (values at 15 digits, warnings, NA, unimplemented names verbatim,
  log10/log2/logb, names attribute) passes in all tiers on this host;
  (4) host GNU R 4.5.0 vs robigo: 135-value bit probe identical.
- **`Rf_revsort`** was a gen-stubs.py stub although sort.rs implements
  the heap sort as `revsort`; Utils.h renames revsort to Rf_revsort, so
  packages (Rcpp sugar sort) call the Rf_ name. Exported both names.
- **Rcpp results (mini tinytest runner, one process per file):**
  test_sugar 176 -> **493/493, identical PASS sequence to GNU R**;
  whole suite 1,101/13 (round 2, one crash then) -> **1,612 pass /
  12 fail**, no crashes (GNU R: 1,624/4). Remaining fails:
  listof 2, matrix 3, misc 1, s4 2, string 1, vector 3.
- Regression checks: S4 slice **644/662** (0 regressions), gnur-examples
  methods/stats4 **43/50** (0 regressions), conformance-values slice
  (incl. all complex cases) 10,496/10,501 (0 regressions); full
  conformance suite green; PR #87 CI: build (unit tests incl. the new
  bit oracles), 4 fastr shards + all other corpora, perf — all SUCCESS.
- Machine notes: glibc 2.41 sources for the ported templates live at
  `~/.cache/robigo-research/glibc/glibc-2.41/math/`; the oracle C program
  is in the repo (`scripts/oracle-complex-trig.c`), re-record inside
  robigo-oracle:4.5.0 only.
## 2026-10-29: round 3, default-package switch rebased and gc tables masked

- Started from main 098bd95 in this worktree and merged #71's existing
  branch. The compressed gnur-tests record merge preserves the original
  JSON lines: main changed only `r-strict-tst`, disjoint from #71's changes.
- Main re-baseline with #71's methods-aware records: S4 slice **643/662**,
  one lost ratchet case, `S4/TestR5/474b742f1785a587`. Main's release
  profile measures **11,685 instructions per S4 dispatch** (20,000 minus
  10,000 calls, divided by 10,000, using `scripts/callgrind-diff.py`).
  The older 13,192 number used a different release profile.
- Implemented the authorized `gc()` allocator-table mask in
  `scripts/corpus.py`. Both Ncells/Vcells rows and header padding are
  normalized without changing line counts or recorded masks. Three
  Python tests verify numeric/NA equivalence, the optional limit columns,
  and preservation of finalizer output and unrelated lines.
- The gc case still fails after masking: GNU R prints
  `[1] "finalizer: 456"`; robigo does not. `builtins/misc.rs`'s
  `do_reg_finalizer` validates its arguments then discards the callback.
  This needs runtime environment finalization and collection, owned by
  core/memory. Do not hide this output or drop the case from scoring.
- Rechecked `reg-packages` in Debian trixie with tools' native library:
  parseRd now works, stdout matches, and status is 0. Stderr still loses
  the blank line immediately before `* installing *source* package`.
  Its recorded temporary-name masks remain intact. Follow up in CLI's
  package-install transcript handling. The host alone is unsuitable for
  this test because its external R is 4.6.1 and its R share paths differ.
- Graphics natives were built from GNU R's sources locally. `reg-S4-examples`
  gets past devholdflush but stops at `graphics::C_plot_new`. The reason
  is outside S4: `native/mod.rs:is_robigo_build` builds LOAD tuples as
  `(p_offset, p_vaddr, p_filesz)` but consumes them as
  `(p_vaddr, p_offset, p_filesz)`. For graphics.so, the RW segment has
  offset 0x41450 and vaddr 0x42450; the DYNAMIC lookup fails. A Python
  reproduction finds graphics/splines false with today's mapping, true
  with the corrected mapping; the other five packages detect correctly.
  Core's next step is to swap the tuple's first two fields, add an ELF
  fixture with unequal offsets/addresses, and recheck this case. CI also
  needs grDevices/graphics/grid added to `build-natives.sh`'s package list.
- After preserving #71's serializer fix, the combined S4/serializer slice
  improves **643/664 -> 645/664** against the saved main binary. S4 alone
  stays **643/662**; both serialized-class-generator cases improve 0/2
  to 2/2. The remaining lost S4 case is still the finalizer above.
- Dispatch cost is **11,685 -> 11,690 instructions/call (+0.04%)**, below
  the 0.5% limit. With the full locally built native set, methods/stats4
  help examples pass **44/50 on both main and this branch**, with no lost
  passes. Added `stats4/update-methods` to the ratchet using corpus absorb.
  The earlier 43/50 run started before all native builds completed. Full fastr and
  gnur-examples measurements are left to CI, queued on the updated PR.
- Verification: release build, `cargo fmt --check`, three Python harness
  tests, and `R_HOME=~/.local/opt/R-4.5.0 ROBIGO_NATIVES=$PWD/.robigo/natives
  cargo test --release -p robigo-runtime -p robigo-cli -- --skip
  c_library_oracle` pass. Counts: 33 runtime unit tests, two lazy-load
  tests, seven CLI unit tests and all three conformance checks (including
  comparisons across execution tiers). The excluded complex-number C
  oracle is the known main/host glibc mismatch recorded in core's ledger.
- Rcpp sugar was not re-baselined successfully this round: the recovered
  mini-runner stops at `sourceCpp("cpp/sugar.cpp")` when run outside its
  tinytest directory. A fresh sourceCpp build needs this worktree's own
  robigo R_HOME and build setup. The previous 176/493 result remains
  historical, not a new measurement.
- Host-built binaries require glibc 2.44. For the Debian check only, mount
  host `/usr/lib` read-only as `/hostlib` and invoke the binary through
  `/hostlib/ld-linux-x86-64.so.2 --library-path /hostlib`; package builds
  and the R library stay Debian's 4.5.0-3.

## Hook contract (for the VM and other evaluator paths)

S4 lives in `crates/robigo-runtime/src/s4.rs` (evaluator side: GNU R's
objects.c/attrib.c/eval.c parts) and `native/methods.rs` (methods' natives).
What other evaluator paths must keep:

- **Closure frames.** `standardGeneric` finds its generic by walking
  `interp.frames` from the top for a non-builtin frame whose `function` has a
  `generic` attribute (R: `get_this_generic`), and `R_execMethod`
  (`Interp::exec_method`) takes `call`, `sysparent` and `promargs` of the
  innermost non-builtin frame (`current_frame()`), which must be the generic's.
  So any call path that runs a closure body must push a `Context` with the
  right `function`, `call`, `cloenv`, `sysparent` and `promargs` (as
  `exec_closure` does) and pop it with `end_context`. Inlining a closure
  without a frame is only safe if its body cannot reach `standardGeneric`,
  `sys.function`, `sys.call`, `parent.frame` or `missing()`.
- **Methods run through `exec_closure`.** `exec_method` builds the method's
  frame itself (bindings copied from the generic's frame, missingness kept as
  `BIND_MISSING_DEFAULT`, unforced default promises replaced by the method's
  own defaults) and then calls `Interp::exec_closure(call, method, promargs,
  newrho, sysparent)`; a VM frame push must accept a prebuilt environment.
- **Missingness.** Dispatch reads it with `s4::binding_missing(env, sym)`: a
  binding flagged `BIND_MISSING_DEFAULT` or holding the missing argument. A
  VM that binds arguments itself must set the same flag for defaulted formals.
- **Internal dispatch.** Primitives that dispatch must go through
  `try_dispatch`/`try_dispatch_group`/`dispatch_with_temp_context`: they try
  the S4 methods of the primitive (`R_possible_dispatch`) for S4 objects
  before S3. Fast paths may skip them only for values without a class
  attribute (`is_object()` false implies `is_s4()` false for everything but
  `asS4()` on a classless value, which has no methods anyway).
- **OBJSXP** is `Value::Obj(Rc<Vector<Value>>)` with empty `data` (see the
  perf entry below for why it shares List's payload type).
- **The S4 bit** is `Attrs.1` (`Value::is_s4`/`set_s4`). Code that rebuilds a
  value's attributes must carry it the way GNU R does: `copy_most_attrib`
  copies it, `set_attrs(box)` keeps whatever the box has, `clear_attrs`
  drops it (GNU R's `CLEAR_ATTRIB`).

### For perf-vm's in-VM frame push (#50): what the S4 code relies on

Answering the prerequisite noted in perf-vm.md ("the S4 agent's
exec_closure/end_context hooks"): there is no extra S4 call inside
`exec_closure_ex` or `end_context`; S4 only reads the context stack and the
argument bindings. A VM frame push is compatible if, while the callee runs:

1. `interp.frames.last()` is a `Context` with `builtin: false`,
   `function: Value::Closure(<the callee>)` (the same `Rc`, so its
   attributes, e.g. `generic`, are visible), `call` = the call as written,
   `cloenv` = the callee's frame, `sysparent` = the caller's environment
   (or the `generic` frame's sysparent, see 4), `promargs` = the promised
   arguments in call order (`R_execMethod` hands them to the method's frame);
2. formals bound from defaults carry `BIND_MISSING_DEFAULT` and unsupplied
   formals without default hold the missing argument (`binding_missing`);
   a default may be bound as a plain constant (exec_method accepts that);
3. `end_context` (on.exit, handler/restart restore) runs when the frame
   pops, also on errors and `Flow::Return` to the frame;
4. `Context.generic` keeps its meaning: the S3 dispatch code sets it on the
   generic's frame around the method call, and `apply_closure_ex` then
   takes the method's sysparent from that frame; a VM call path that
   computes sysparent itself must do the same;
5. methods are entered through `Interp::exec_closure(call, method,
   promargs, prebuilt_env, sysparent)` from `exec_method`; the VM need not
   handle that path (it is a native call), but `exec_closure` must keep
   accepting a prebuilt environment.

Calls that never push a frame (pure inline closures) stay correct as long
as their bodies cannot reach `standardGeneric`, `sys.*`, `parent.frame`,
`missing` or `callNextMethod`, as today.

## For perf-vm: namespace closures miss the guard memo (S4's biggest cost)

Measured 2026-10-29 on main 00328a0 (after #51), callgrind, 2e4 calls of
a one-method generic plus 2e3 `new("Sq", s = i)`: 2.51G instructions,
**`vm::guard_ok_slow` ~13% of them** (9.5% inclusive, plus its inlined
`Cell`/`mem` parts). The dispatch machinery itself (methods.rs:
`dispatch_generic` minus the R code it runs, `load_method`, `do_slot`) is
~1%; the rest is methods' own R code (`new` -> `initialize` ->
`validObject`, `getClass`) running in the VM.

Why: every closure of a package namespace runs in a frame three levels
below the search path: frame -> `namespace:methods` -> its imports env ->
`namespace:base` (`is_search`). `guard_ok_slow` memoizes only frames at
depth 1 (`env` itself) and depth 2 (`outer`), so for depth 3 it sets
`env: 0, outer: 0`, and each new frame (every call) walks the chain again
with `local_may_bind_fun` on each environment. In a namespace that is a
hash lookup in the frame's index: with ~1,000 bindings the 64-bit `mask`
is all ones and never short-circuits.

Proposal (vm.rs, perf-vm's file; not done here):
- Generalize `outer` to "the frame's parent `p` and the chain from `p` up
  to the search path": memoize `p.id()` and a checksum of the
  `(id, fn_version)` pairs of every environment from `p` to the search
  environment `cur`. A hit for a new frame `env` is: same epoch,
  `env.parent == p`, checksum still equal (walking 2-3 parent pointers and
  reading `fn_version`, no hash lookups), `!env.local_may_bind_fun(sym)`.
- Sound by the same argument as `outer`: `fn_version` changes whenever a
  function-like binding of that environment is created, changed or
  removed, and the search epoch covers everything from `cur` up.
- Namespaces and their imports are locked after loading, so in practice
  the checksum never changes and every guarded call in methods' code hits.
- Expected: most of the 13%, on any package code (stats, utils, Rcpp
  modules), not only S4. An S4 benchmark in `benchmarks/` (generic call,
  `new`, an S4 method for a primitive) would keep it tracked; the script
  above is in this entry.

## 2026-10-29: the zero-loss switch (lead: fix first, then switch)

Ratchet cases the switch would lose, and where each stands:

| case | cause | status |
|---|---|---|
| fastr `S4/TestR5/ef10c392` | reference class methods listed in `names(env)` order: GNU R's hash-chain order | fixed, #67 |
| fastr `TestBuiltin_serialize/4a71e01f`, `bfd38dfb` | class generators' bodies have a LANGSXP cell (`substitute()`'s unbound `...`) | fixed, #71 (a side registry: a field in `Lang` cost fib +0.68%) |
| fastr `S4/TestR5/474b742f` | `finalize()` on `gc()`, and the case prints `gc()`'s table with GNU R's own cell counts | **needs the lead**: no implementation reproduces `Ncells 340001`; robigo reports NA by design. Options: mask `gc()` tables in `corpus.py` like `proc.time()`, or exclude the case. Finalizers themselves need environment collection (core) |
| gnur-tests `reg-S4-examples` | with methods GNU R reaches `plot()` on an S4 object; robigo has no graphics device (`devholdflush`) | blocked on the graphics track |
| gnur-tests `reg-packages` | with methods GNU R runs the whole test (161 lines; without, it stopped early with status 1, and robigo matched that). robigo stops at `tools::checkRdContents()`: `tools`' native `parseRd` (the Rd grammar) is not implemented; one blank line after `R CMD INSTALL`'s output also differs | reproduced in a trixie container with r-base-core 4.5.0-3 and in #71's CI: needs a `parseRd` port (tools) |
| fastr `add62673`, `0bb20eaa` | recording/host artifacts | fixed on main (#58/#62) |

Recording: every case of the five corpora recorded twice in
`robigo-oracle:4.5.0`, with and without methods. A case takes the new
record only if the two differ after `corpus.py`'s normalization and every
recorded mask; otherwise it keeps main's record (which CI passes).
`record --mask-against-previous` across the switch would mask exactly the
lines methods changes. reg-large is not re-recorded here (memory).

Environment order (#67), what GNU R does and robigo now does:
- hashed environments (`new.env()`, global, namespaces, `attach()`,
  `list2env()` above 100 elements) list chain by chain. `R_HashSet`
  counts every new binding in `HASHPRI` (its empty-chain test runs after
  the search loop), so tables grow earlier than the load factor says;
- frame pairlists (closure frames, `eval()` with a list, `as.environment()`
  of a short list) list newest first after the initial frame;
- `serialize()` writes hashed environments as their `HASHTAB`.
Perf: the tables are built lazily, only when a hashed environment is
listed, serialized or loses a binding. The remaining cost is
startup: the ~37 `names(env)` calls of loading utils and stats build their
tables (~150 instructions per binding, +0.4% on the 280M-instruction
startup in a trixie container). Each layout of the frame code moved
LLVM's inlining of `Rc<EnvData>::drop_slow` and `local_may_bind_fun` by
±0.5% on fib: partitioning noise (the same code measured +0.06% on this
host's build). CI's perf job is the judge; `codegen-units = 1` would
remove the lottery (a project decision).

test_sugar crash (#75): Rcpp keeps `.rcpp_cache` (a VECSXP it stored with
`Rf_defineVar`) and `Rcpp::Function`s in statics. The bridge handed out a
new copy on each `findVarInFrame` and let proxies die when no C root held
them. Now stored vectors keep their core object and proxies stay pinned
while their runtime object lives. test_sugar runs to its end: 176/493,
then stops at complex `cos()` (robigo's complex `Math` has only
log/sqrt/exp: math track).

## 2026-10-28: fastr with methods attached, whole corpus (host)

- All 31,374 fastr cases re-recorded with methods in `default_packages`
  (oracle image, uid 1000, `--memory 6g`, `ulimit -v 3000000` per process,
  2 jobs, 8 chunks by the id's last hex digit; scratch worktree, nothing
  committed). robigo with methods attached: **30,668 / 31,374** on this
  host.
- Ratchet cases failing with methods (11 at first, after the fixes of
  this round 6, all understood):
  - fixed: `obj$x <- v` on an OBJSXP (error call), `rbind(<S4 OBJSXP>)`
    and `as.vector()` (data part or "no method for coercing this S4 class
    to a vector"), `cbind` cases (pass on re-run);
  - `TestR5/ef10c392`: reference class method names in hash-bucket order
    (`names(env)` of a hashed environment; env.rs);
  - `TestR5/474b742f`: reference class `finalize()` runs on `gc()`
    (robigo does not collect environments; memory/core);
  - `TestBuiltin_order/add626733541c799`: recording artifact, the oracle
    image here lacks the en_US.utf8 locale the original recording had;
  - `TestBuiltin_serialize/4a71e01f, bfd38dfb`: serializing a class
    generator (an S4 closure) writes one node type differently (LISTSXP
    for GNU R's LANGSXP) (serialize.rs);
  - `TestBuiltin_operators/0bb20eaa`: host complex NaN bits (CI judges).
- **For the switch** (corpora.toml + robigo's startup): re-record every
  corpus in exactly the original recording environment (CI's container,
  root, its locales; reg-large needs its RAM), then the ratchet cases
  above must pass or be decided by the maintainer: the scripts are the
  same, their oracle changes. A bare `robigo script.R` already attaches
  methods since #52's cli change.

## 2026-10-28: Rcpp's own test suite (in progress)

- `tinytest::test_package("Rcpp")` cannot run on robigo yet, for two
  reasons outside S4: tinytest opens `grDevices::pdf(nullfile())` per test
  file (grDevices' PDF native), and it reads each expression's srcref for
  line numbers (`env$lst - env$fst` is NA without source references:
  robigo's `parse(keep.source = TRUE)` keeps none).
- Measured instead with a minimal stand-in for tinytest's `expect_*`
  (`sys.source` per test file, `RunAllRcppTests=yes`), the same script on
  GNU R 4.5.0 (Rcpp 1.1.2 built there) and on robigo (Rcpp 1.1.2 built by
  `install.packages()` inside robigo), one process per file on robigo
  because a test file crashed it.
- Result (assertions): GNU R 1,624 passed / 4 failed; robigo 1,101 / 13
  before, then with the C API added for it (R 4.5's
  `R_ClosureEnv/Formals/Body/Expr` through the bridge; Rmath's
  `Rf_tetragamma`, `Rf_pentagamma`, `Rf_ftrunc`): test_function 9 -> 22/22,
  test_support 0 -> 5/5. test_sugar (493 assertions on GNU R) now loads
  its object but segfaults in it (being located). Smaller differences in
  test_date (3), test_listof (2), test_matrix (3), test_misc (1),
  test_string (1), test_vector (3), test_xptr.
- Still unexported Rmath functions (pointer arguments, need hand-written
  wrappers in robigo-capi): bessel_{i,j,k,y}_ex, dpsifn, lgammafn_sign,
  logspace_sum, pnorm_both, and dtukey/rtukey/rnbeta/rnf/rnt.

## 2026-10-28: the S4 cases lost with methods attached, re-checked

- The 12 fastr S4 cases (TestS4, TestR5) that failed only with methods
  attached, run against GNU R with methods attached: 11 now match. The
  remaining one prints a reference class generator; its "Class Methods"
  list is `names(def@refMethods)`, i.e. `ls(sorted = FALSE)` of a hashed
  environment, which GNU R returns in hash-bucket order (PJW hash of the
  name, table size and growth from envir.c's R_HashSizeCheck/R_HashResize).
  robigo's environments do not emulate that order (env.rs, not this
  track).
- Also open from that set: reference class `finalize` methods
  (`reg.finalizer` on environments with `gc()`; robigo does not collect
  environments), `asS4(NULL)` setting the S4 bit of the global NULL
  (bug-compatible, not emulated), `substituteDirect` cases (pending
  re-recording).

## 2026-10-28: S4 dispatch overhead per call

- callgrind (scripts/callgrind-diff.py, 2e4 - 1e4 calls of a one-method
  generic): 14,398 -> 13,192 instructions per call (-8.4%):
  - the calling handler around each signature argument's evaluation
    (`R_evalHandleError`) is skipped when the argument's binding is a
    value or a forced promise: evaluating it runs no code, so nothing can
    signal (the handler cost an Rc closure, an entry and a message string
    per argument per call);
  - the dispatch's fixed symbols (`.AllMTable`, `.SigArgs`, `.SigLength`,
    `target`, `defined`, `nextMethod`, `allMethods`) are interned once;
  - the signature label is built in one String (no Vec<String>, join).
- What remains of the dispatch: `find_var_in_frame` (~10 frame lookups per
  call: the generic's tables, `.target`/`.defined`/`.Method`), defining
  `.target`/`.defined`/`.nextMethod`/`.Method` in the generic's frame and
  copying them into the method's (GNU R does the same). The rest is the VM
  running the generic's and the method's frames.
- Wall time is too noisy on this shared machine to compare with GNU R
  (GNU R's own numbers moved 2-3x between runs); roughly 1.3-2.3x slower
  on S4-heavy loops (generic calls, `+` methods, `new()`).

## 2026-10-28: S4 deparse; recording fastr with methods (memory)

- Deparse of S4 objects ported (deparse.c's IS_S4_OBJECT branch); the
  parts that need R (getClassDef, R_do_slot of `.Data`, asS3) are computed
  by `Interp::deparse_prepared` before the interpreter-free deparser runs
  (thread-local table keyed by object identity, cleared afterwards).
  Printing a generic, dput/deparse of S4 objects: byte-identical
  (tests/r/s4/deparse.R). fastr S4 cases recorded with methods attached
  (recorded chunks): 115 -> 118/124.
- Recording fastr with methods: chunks 0, 1, 4-9 recorded; 2, 3, a-f not
  yet. Chunk a/b was OOM-killed under the 6 GiB container cap (docker
  events: exit 137), also with the stats distribution groups excluded.
  The cap did its job; the case is being found with a probe recording
  (`ulimit -v 3000000` per process, scratch copy, compared with the old
  expectations).

## 2026-10-28: reg-S4 (GNU R's S4 regression test) with methods attached

Climbing gnur-tests' reg-S4 (recorded with methods attached); each stop
fixed and checked against the oracle:

- line 285: `cbind(m)` with an S4 argument: bind.c:do_bind dispatches S3
  methods on `R_data_class2` and falls back to `methods:::cbind`/`rbind`
  when any argument is S4 (builtins/array.rs, minimal). tests/r/s4/bind.R.
- line 470: `setMethod("as.numeric", ...)` must also serve `as.double()`:
  base binds `as.numeric` to the `as.double` primitive, whose method tables
  hold both; robigo's builtin passes the name it was called by
  (`s4::prim_op` maps it).
- line 660: `2 == <S4 list>`: robigo's relop rejected every list. GNU R
  4.5.0 compares lists by coercing them to the comparison's type
  (`coerceVectorList`; non-character targets need `isVectorizable`, else
  "'list' object cannot be coerced to type 'double'"); expressions go the
  same way (the expression check is `#ifdef previous_R_versions`).
  builtins/arith.rs, minimal. tests/r/s4/groups.R.
- After these, reg-S4 runs to its end; what remains is `getSrcref()`
  (robigo returns NULL for any function parsed with keep.source, not
  only methods: parser/print tracks) and a timing line.
- Measurement trap: a robigo binary copied to `/tmp/x/robigo` loads base
  natives from `/tmp/natives` (`<exe>/../natives`), where another agent
  had built GNU R's methods.so (which fails: `DDVAL`). Copies used for
  measuring must live where `../natives` does not exist.

## 2026-10-28: Matrix from source: blocked on grid (not S4)

- `install.packages("Matrix")` inside robigo first builds its dependency
  lattice, whose lazy-loading step loads grid: `.Call(C_initGrid)` is not
  implemented (grid's natives: core's `scripts/build-natives.sh grid` or
  a port; graphics engine, not this track). Retry Matrix once grid
  loads; it is the S4 stress test after Rcpp (dgCMatrix & co).

## 2026-10-28: methods as a default package, per corpus (host, same binary)

Recorded with methods in `default_packages` (oracle image, uid 1000,
`--memory 6g`), robigo with the matching default packages:

| corpus | without methods | with methods | lost | gained |
|---|---|---|---|---|
| gnur-examples | 591 | 603 | file.access, system (recording artifacts: uid, seccomp) | 14 (some date-dependent) |
| gnur-tests (without reg-large) | 21 | 19 | reg-S4-examples (grDevices `devholdflush` once `plot()` is reached), reg-packages (the host's `R CMD` subprocesses use system R 4.6.1) | 0 |
| gnur-demos | 2 | 2 | | |
| r-intro | 4 | 4 | | |
| fastr | (recording) | | | |

The losses are cases that "passed" only because GNU R and robigo stopped
at the same early error without methods; with methods GNU R gets further.
None is an S4 failure. For the switch: record as root (as CI's container
runs), leave reg-large to CI, and decide with the maintainer how a case
whose recorded behaviour changes is treated in the ratchet (it is the
same script with a different oracle).

## 2026-10-28: S4 speed vs GNU R (measured, not yet improved)

- Wall time, robigo vs GNU R 4.5.0 (oracle image): 1e5 calls of a
  one-method generic 0.85 s vs 0.64 s; 2e4 `+` with an S4 method 5.84 s
  vs 4.44 s; 2e4 `new("P", x = i)` 16.25 s vs 9.32 s. robigo is
  1.3-1.7x slower.
- callgrind on `new()` (2000 - 1000 calls): 884k instructions per call.
  The dispatch machinery (`dispatch_generic` without the method,
  `load_method`, `do_slot`) is ~1%; the rest is the VM running methods'
  own R code (`new` -> `initialize` -> the default method, `validObject`,
  `getClass`). 13.6% is `vm::guard_ok_slow`: frames of namespace closures
  (parent = the methods namespace, not on the search path) miss the guard
  memo's fast paths, so every guarded call site rechecks. resolves_to_base
  itself is negligible.
- Next: make namespace-closure frames hit the guard memo (perf-vm,
  vm.rs), then re-measure; and an S4 benchmark in benchmarks/ (generic
  call, `new`, a primitive method) so perf CI tracks it.

## 2026-10-28: #52 merged; CI results; trace(); recording with methods

- #52 merged. CI, branch vs main's latest run: fastr 30320 -> 30369
  (+49), gnur-examples 559 -> 605 (+46), gnur-tests 20 -> 21 (+1), no case
  lost in any corpus; perf job green.
- `traceOnOff`/`debugOnOff` (debug.c) replaced placeholders that returned
  FALSE: `.doTrace` now runs, so `trace(f, quote(...))` and
  `trace("g", ..., signature = "numeric")` (methods' `.TraceWithMethods`)
  print GNU R's output (tests/r/s4/trace.R). Open: `trace("f")` without a
  tracer goes through `.primTrace` and the RTRACE bit, which needs the
  evaluator/VM call path to print `trace: f(1)` (VM owner).
- Methods as a default package, measurement in a scratch worktree
  (corpora.toml `default_packages` + methods, recorded in the oracle
  image): gnur-examples 591 -> 603 passing on this host with the same
  binary; the 2 "losses" are recording-environment artifacts (Docker uid
  for file.access, seccomp's "Function not implemented" for system()).
- **Memory incident:** recording gnur-tests with `--jobs 4` ran its
  `reg-large` case (8+ GB) next to three other recordings; the box (14 GB,
  shared) ran out and the core agent was OOM-killed. Now: `free -g` check
  before each recording, `docker run --memory 6g --memory-swap 6g`,
  `--jobs 1` for gnur-tests with `--filter '^(?!reg-large$)'`; reg-large
  is left to CI. fastr is recorded in 8 chunks (last hex digit of the id)
  because `record` writes only at the end.

## 2026-10-28: bare robigo starts with GNU R's default packages; startup cost

- `robigo script.R` without `R_DEFAULT_PACKAGES` forced it to NULL ("cannot
  load the default packages yet"); it now gets the profile's defaults,
  methods included, as GNU R does (crates/robigo-cli/src/main.rs, one
  block removed; the conformance runner now sets NULL itself, as
  regen-expected.sh records). Corpora, conformance and benchmarks set the
  variable explicitly and are unchanged.
- Startup, 5 runs each: robigo 230 ms without methods, 315 ms with;
  GNU R 4.5.0 (oracle image) 369 ms without, 672 ms with.
- Where methods' 162M startup instructions go (callgrind, `-e
  'invisible(1)'`, methods vs NULL): ~35M is zlib set-up, not inflating:
  `miniz_oxide::InflateState::new_boxed` and its zeroed allocation once
  per lazy-load fetch (273 fetches). One reused inflate state would remove
  it (builtins/serialize.rs, startup track). The rest is unserialize
  (`read_item`, bytecode `read_bc_lang`) and `..onLoad`'s R code.

## 2026-10-28: relation to core.md §13 (methods.so's entry points)

- core.md §13 lists the 22 non-API entry points GNU R's methods.so would
  import. Every one of them now exists in the runtime (`s4.rs`), and the
  S4 bit and OBJSXP cross the bridge both ways. The methods natives
  themselves are served by `native/methods.rs`; `scripts/build-natives.sh`
  builds only stats by default, so `dyn.load` keeps using the port unless
  someone builds methods.so explicitly.
- To make methods.so loadable later (a measurement, not a switch), the C
  side of those entry points would be thin host callbacks into `s4.rs`:
  `R_set_standardGeneric_ptr`/`R_set_quick_method_check` (store the C
  function pointers and call them from `Interp::standard_generic` /
  `possible_dispatch`), `R_execMethod`, the `prim_*` tables,
  `R_findVarLocInFrame`/`R_GetVarLocMISSING` (`s4::binding_missing`).
  Expected costs, from the bridge's design (core.md §6): argument copies on
  every dispatch and a lost in-place `R_set_slot` (which `initialize()`
  relies on), so the port stays the default until G3.
- Already in libR (robigo-capi `objects.rs`, for packages): `R_do_slot`,
  `R_do_slot_assign`, `R_has_slot`, `R_do_new_object`, `R_do_MAKE_CLASS`,
  `R_getClassDef`, `Rf_asS4`.

## 2026-10-28: Rcpp loads and runs; S4 environments; zero-cost OBJSXP; full corpus

- **Rcpp from source inside robigo.** With robigo's R_HOME
  (`ROBIGO_NATIVES=<dir with tools/tools.so> scripts/robigo-home.sh`,
  tools.so from core's `build-base-native.sh`), `utils::install.packages("Rcpp")`
  compiles Rcpp 1.1.2 against robigo's headers and passes R's load tests.
  `library(Rcpp)`, `evalCpp("1 + 1")`, `cppFunction()` (scalars, vectors,
  `List::create`), `Rcpp::stop()` caught by `tryCatch`, and an
  `RCPP_MODULE` class (`new(Acc, 1)`, `$add()`, `$get()`) all work.
  Needed: libR's S4 entry points (`crates/robigo-capi/src/objects.rs`:
  `R_do_slot`, `R_do_slot_assign`, `R_has_slot`, `R_do_new_object`,
  `R_do_MAKE_CLASS`, `R_getClassDef`, `Rf_asS4`), ported over core objects
  so that they modify in place as GNU R's do; S4 objects extending
  "environment" in `get`/`exists`/`assign`/`ls`/locking/`environment<-`/
  `parent.env`/`new.env`/`eval`/`[[`/`$`/`[[<-`/`$<-`/`is.environment`
  (GNU R's `simple_as_environment`, `R_getS4DataSlot`); `mget`,
  `list2env` and `delayedAssign` reject them as GNU R does.
- **R_execMethod with constant defaults.** robigo binds constant defaults
  without a promise; such a binding of a missing argument is the
  generic's default too and becomes the method's own (callNextMethod
  example). 
- **OBJSXP costs nothing.** With `Value::Obj(Rc<Obj>)` (its own payload
  type), `perf.py compare` against main showed fib +0.58% (> 0.5%): callgrind
  (scripts/callgrind-diff.py, fib(21) - fib(20)) put +29 instructions per
  call into drop glue that LLVM stopped inlining into `Rc<EnvData>::drop_slow`
  once `Value`'s drop glue had another distinct arm. `Obj` is now
  `Vector<Value>` (List's payload, `data` empty): the arms are identical
  and merge. Result: **0 of 78 benchmarks slower or faster than 0.5%**.
- **Full fastr corpus** (4 shards, host): 0 regressions against main's
  binary on this host. 13 complex NaN-payload cases (TestSimpleValues,
  TestSimpleArithmetic, one TestBuiltin_operators) and the
  `c_library_oracle` unit test fail for main and this branch alike on this
  host (#47 encodes gcc/glibc bits of CI's image). Of the 528 cases that
  pass beyond the ratchet, main passes 484: **+44 over main**, all S4.
- Slices now: S4 slice 608 -> 639/662; gnur-examples methods/stats4
  0 -> 41/50. Conformance: `tests/r/s4/{classes,dispatch,refclass,bits}.R`
  (recorded in the oracle image) pass and are in the ratchet.
- Open (outside this track, noted for their owners):
  - stats4 `mle`/`update-methods`: the stats `optim` native (Rust port
    missing; GNU R's stats.so built from source has it, core's ledger);
  - methods `setMethod`: grDevices `devholdflush`; `showMethods`:
    `lazyLoadDBflush` (unloadNamespace); `cbind2`: `trace()`;
    `GenericFunctions`/`implicitGeneric`: the `<bytecode>` line when
    printing base closures (corpus-scripts ledger);
    `Methods_for_Nongenerics`: a timestamp inside IGNORE_RDIFF (never
    byte-identical); `inheritedSlotNames`: Matrix not installed here;
    `refClass` (gnur-examples): GNU R's recording has codetools, the host
    R_HOME does not ("not checked for suspicious field assignments");
  - `f <- function(...) lapply(1, log, ...); f()` fails on main: `...`
    reaching a special through lapply (conformance-lang/vectors).
  - Rcpp's help pages: `RdTags(Rd) == "COMMENT"` errors during install
    (tools' Rd code on robigo).
- Next: methods as a default package (re-record every corpus with methods
  in `default_packages`, robigo's startup attaching it), Rcpp's own tests,
  `trace()` for S4 (`.primTrace`, methods' trace machinery).

## 2026-10-28: methods loads; setClass/new/setGeneric/setMethod/show

- why: `library(methods)` failed in `R_initMethodDispatch` (no port), and the
  runtime had no S4 bit, no OBJSXP, no `standardGeneric`, no slots.
- baseline (main 414ae44): S4 slice 608/662 (TestS4 84/102, setS4Object 1/21,
  asS4 0/1); gnur-examples methods/stats4 0/50 (all "package or namespace load
  failed for 'methods'").
- what:
  - S4 bit in `Attrs`, `Value::Obj` (OBJSXP); serialize reads/writes both
    (methods' lazy-load DB is full of S4 objects and S4-bit closures).
  - `s4.rs`: `standardGeneric` (+ `dispatchNonGeneric`), primitive-method
    tables, `R_possible_dispatch`, `R_execMethod`, `asS4`/`setS4Object`,
    `R_do_slot`/`R_do_slot_assign`/`R_has_slot`, `R_getS4DataSlot`,
    `R_data_class`, `S4_extends`/`.cache_class`/`.class2`,
    `R_do_new_object`, `PrintObjectS4`.
  - `native/methods.rs`: the 26 registered routines of methods.so
    (table dispatch `R_dispatchGeneric`, `R_quick_dispatch`, old MethodsList
    dispatch, `R_nextMethodCall`, `R_getGeneric`, `R_getClassFromCache`,
    `R_loadMethod`, ...).
  - `conditions.rs`: Rust calling error handlers
    (`R_withCallingErrorHandler`), with GNU R's handler call
    `h(simpleError(msg, call))` in the converted error.
  - `@`, `@<-` (+ `checkAtAssignment`), `isS4`, `class<-`/`oldClass<-`
    S4 warnings, `oldClass` `.S3Class`, `inherits` (R_data_class / S4
    extends), `attributes<-` keeps the bit, `copyMostAttrib` copies it,
    `[<-` embeds OBJSXP values (embedInVector), print of S4 objects without
    methods.
  - **R_set_slot in place.** GNU R's `R_set_slot` modifies its argument and
    methods' default `initialize()` relies on it (`"slot<-"(.Object, ...)`
    without assignment). robigo rebinds the caller's variable the `object`
    promise came from, if it still holds that very object.
- result: S4 slice 608 -> 636/662; gnur-examples methods/stats4 0 -> 38/50;
  a setClass/new/setGeneric/setMethod/show/contains="numeric" script is
  byte-identical to GNU R.
- **Why Rust ports, not methods.so built from source.** main serves every
  base package's natives from Rust ports (`native/`); GNU R's own `.so`s
  (core's build-base-native.sh, #37) are opt-in and need R sources and a C
  toolchain in CI (a maintainer decision, docs/ledger/core.md). For methods
  the bridge would also be wrong and slow: it copies arguments (every
  dispatch would copy every argument's slots), `R_set_slot`'s in-place
  modification cannot reach the caller through a copy, and dispatch calls
  back into the evaluator for every frame lookup. methods' C code is 1,500
  lines; the evaluator side (objects.c/attrib.c/eval.c), which is most of
  the work, is needed in Rust either way. Once the runtime is on core
  objects (G3), methods.so can be measured against the port.
