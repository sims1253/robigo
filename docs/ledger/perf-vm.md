# perf-vm: the VM and the JIT

Metric: `scripts/perf.py run --metric instr`, geometric mean of robigo/GNU R
instructions over programs whose output matches GNU R's; the number of
benchmarks faster than GNU R. Gate: `scripts/perf.py compare` (no benchmark
more than 0.5% slower) and `cargo test --release -p robigo-cli --test
conformance` (interpreter, VM and VM+JIT against each other).

## 2026-10-30: round 8, CSV connection formatting

- Fresh branch from main d2ce510, rebased before filing onto f5dae40
  (#96/#97). Saved baseline executable at 9b9bab5. Recover the round 7
  literal-%s candidate from the stash after measuring this baseline. The authorized
  shared edits are capi connections.rs and main/robigo_sys.c.
- Baseline: standard CSV 947,271,725 instructions, native utils CSV
  1,950,190,065. Both use the same immutable executable and R_HOME.
  Standard and native conformance each pass 244/244 scripts in all tiers;
  both focused FastR slices pass 180/181 with zero regressions.
- Add a Debian R 4.5.0-3 recording for CSV file bytes and parsing,
  missing values, UTF-8, percent directives in data, quoted fields,
  255/256/257-byte writes, row names and CRLF. The delivered round 7
  binary matches the recording. Current-main measurement is complete; candidate verification follows.
- Core's ledger leaves environment finalizers and cycle reachability in
  flight. Promise::forced still owns a boxed Value retained by both frame
  bindings and Context.promargs. Leave value.rs unchanged. A VM-only
  shortcut cannot remove that box while preserving observable promises.
- Sweep-data's internal-call handoff is lapply/245e62b: is.vector must be
  recognized before X is forced, following GETINTLBUILTIN/CALLBUILTIN.
  Keep this as a separate follow-up after the CSV batch.


## 2026-10-30: setter fix rebased onto the byte window

- Rebased #94 onto **df3d535** (#84). The conditions.rs conflict keeps
  main's RString class representation and the condition/default-call split.
  Verified every main conformance entry remains, with five additions.
- Post-rebase TestSimpleVectors: **1934/1937**, **zero regressions**.
  All 27 reported failures pass again; the other 1910 classifications
  match the original failing #94 snapshot. New main alone is 1932/1937.
- All **246/246** conformance scripts match GNU R in AST, VM and JIT.
  Debian runtime tests: **45/45**, including the unchanged 14-case setter
  oracle, the JIT transition, and all 9440 arithmetic comparisons. The
  6144-case payload fixture passes in the full conformance replay.
- Matched private Debian builds of df3d535 and the rebased candidate:
  seven-benchmark instruction geomean **0.661500 -> 0.661468** GNU R;
  paired gate: zero slower above 0.5%, zero broken. Host runs use the
  supplied R_HOME. Formatting and source whitespace checks pass.
- The four old-base local shard runs were stopped after #84 merged;
  they are not verification of the rebased tree. Full shards and the
  complete performance gate must run in real CI after the rebased push.
- Artifacts: `.robigo/r7-fix/`, including immutable matched binaries,
  vector classifications, conformance results, and instruction profiles.

## 2026-10-30: round 7 follow-up, condition calls versus default errors

- Reproduced all **27** #94 TestSimpleVectors regressions: **1907/1937**,
  zero status differences. Read every affected recorded stderr; messages
  are unchanged. Twenty-two failures need the sentinel in default output;
  five need the evaluated RHS on a small closure's first invocation.
- R: errors.c:R_vmakeErrorCondition resolves R_CurrentExpression through
  getCurrentCall() for the condition. R_signalErrorConditionEx instead
  passes the original marker to errorcall_dflt for uncaught output. Direct
  assignments and closures exhibit this distinction too; it is not limited
  to anonymous eval/parse. Resolve only the condition's call while the
  assignment context is active, retaining the raw call for default output.
- R: eval.c:R_CheckJIT also distinguishes eager robigo VM execution from
  GNU R compilation. Use the existing closure JitState at the cold setter
  boundary: small first-call setters show the RHS value; compiled setters
  show its expression. A new GNU R oracle records first and second calls.
- Kept: **1934/1937**, all 27 restored, the other **1910** classifications
  unchanged. All **241** conformance scripts match in all three tiers,
  including new direct/eval default-output and handler fixtures. The original
  **14/14** oracle data is unchanged; its unit test now observes condition
  calls through a handler and independently asserts the uncaught marker.
- Debian runtime tests: **40/40**, including the JIT transition and 9,440
  arithmetic comparisons. The 6,144-case payload conformance fixture passes.
  Paired seven-benchmark gate against saved main: zero slowdowns above 0.5%,
  zero broken cases. Formatting/source whitespace checks pass.
- Four full FastR shards were started, then stopped when #84 merged and
  required a new base. See the post-rebase verification above. No oracle
  recording or existing ratchet was weakened; four entries were added.

## 2026-10-30: round 7, setter baseline and native CSV split

- Baseline: main f668f60, saved binary `.robigo/r7/main`. A new Debian
  GNU R 4.5.0-3 oracle explicitly compares interpreted and compiled
  setters. Main matches **8/14** cases. Four compiled `[<-` cases lose
  the RHS expression; two `[[<-` cases expose an unresolved
  `<current-expression>` marker. The candidate resolves both in the
  assignment helper before handlers run. Final oracle result: **14/14**.
- Rejected intermediate: changing only the helper fixes 11/14 cases;
  missing and effectful indices still fall back to interpreted assignment.
  A general setter opcode fixes all 14, but compiling its RHS separately
  regresses a 2,000-update matrix-row probe from 72,853,738 to 73,338,443
  instructions (+0.665%). Delegate RHS evaluation and setter lookup to the
  existing interpreter/helper instead. Final probe: **72,491,277**
  instructions (-0.498%), identical output. Kept.
- Paired seven-benchmark instruction geomean: **0.663447 -> 0.663464** GNU R,
  all seven faster than GNU R, no slowdown above 0.5%. This focused set is
  not the round 6 full-suite metric. Both binaries match GNU R 4.5.0 on
  stdout, stderr and status for all seven programs. Artifacts: `.robigo/r7/`.
- Conformance: **236 -> 237** oracle passes in every tier, zero differences
  between AST, VM and JIT; the new setter-context fixture also checks the
  error class after crossing the native threshold. Ratchet adds one line.
  Focused FastR subset/matrix/assignment slice: **507/512 -> 507/512**,
  zero regressions. Formatting and source whitespace checks pass.
- Host runtime unit tests: 37/39 pass, including the new setter test and
  all 9,440 VM arithmetic oracle cases. The two unchanged complex C-library
  fixtures differ by one bit with host glibc 2.44. The final Debian trixie
  runtime unit run passes **39/39**, including both C-library fixtures and
  the setter oracle. No arithmetic code changed. The full conformance run also
  replays the 6,144-case muladd payload fixture.
- Shared changes: minimal diagnostic-source/current-expression handling
  in builtins/lang.rs and one tests/r/passing.txt entry. General setters
  resolve and dispatch their replacement function at runtime, so they do
  not need the scalar fast path's replacement-function guard.
- Reanalyzed the saved round 6 native CSV callgrind profile, total
  1,933,306,497 instructions. Inclusive totals (nested, do not add):
  writetable 1,043,869,788 (53.99%), Rconn_printf 713,041,782 (36.88%),
  m_vfprintf 697,441,782 (36.08%), EncodeElement2 307,843,439 (15.92%),
  Rf_EncodeElement0 283,673,719 (14.67%), typeconvert 66,451,525 (3.44%).
  utils io.c:writetable emits each cell, separator and newline through
  Rconn_printf("%s", ...). capi connections.rs:m_vfprintf calls vasprintf,
  copies to a Vec, frees the allocation, and forwards the bytes.
  Next experiment for core/startup: a literal-%s/%s%s bridge path that
  avoids formatting and allocation, verified against connection and CSV
  corpora. No edits to their files in this batch.
- Unboxed promises still require shared Value/Promise storage changes:
  both environment bindings and Context.promargs retain the promise.
  A VM-only shortcut cannot remove the scalar allocation while preserving
  escaping promises, parent.frame mutation, substitute/missing and the
  methods context hooks. Agree on storage with core before implementation.
- Rejected oracle launch: the host R wrapper ignores R_HOME, and the
  copied R 4.5 executable loads the host 4.6 libR without its Debian
  dependencies. Discarded those GNU R counts. Use robigo-oracle:4.5.0
  for GNU R measurements; host robigo still uses the supplied R_HOME.

## 2026-10-30: native CSV cost and complete benchmark stream checks

- Controlled CSV probe with the saved #60 CI binary and its flat cache:
  matching CI native libraries cost **1,933,306,497 instructions**;
  selecting existing Rust ports with an empty native directory costs
  **912,341,802**. Benchmark stdout matches byte for byte. This reproduces
  CI's 1,933,679,323 count within 0.02%. Do not attribute the CI/local CSV
  gap to VM code generation without matching native/cache configuration.
- The native CSV profile attributes 18,029,457 self instructions (0.933%)
  to VM source, 285,637,878 (14.775%) to scan/io, 99,477,531 (5.145%) to
  C API source and 151,517,896 (7.837%) to core source. These are exclusive
  costs, not an attribution of the entire native/port difference.
  Native utils io.c:writetable/typeconvert and C API formatting are
  concrete places to split the next profile. Coordinate with startup/core;
  compare a verified Rust override or reduce bridge/formatting costs only
  after complete formatting and CSV conformance, rather than disabling
  native code globally.
- Rejected profiling setup: mixing the local CLI-only binary with CI's
  native utils fails to load utils.so (undefined symbol lzma_crc64).
  The matching CI binary/native artifacts load correctly. Use matching
  artifacts; this failed setup does not measure CSV performance.
- #90 CI run 40966972376 is also green on code head 2884282. Against
  its own main base dbf6754, the official stdout/status geomean is
  0.456983 -> 0.454530 GNU R; the 76-case complete-stream subset is
  **0.452108 -> 0.449655**, 73 -> 74 faster. All stream-match flags are
  identical to main/#60. Gate after excluding the known false FIXED flag:
  6 faster, 0 slower above 0.5%, 0 broken, 0 fixed.
- Actual CI artifacts compared with GNU R across all benchmark streams:
  main e25e6cc and #60 both match stdout/status on 77/78, but all three
  streams on **76/78**. spectralnorm_math is the known mismatch;
  flexclust_no_s4 returns status 1 in both GNU R and robigo but its error
  call differs. The discrepancy is unchanged by #60 and is present in
  robigo's AST and VM. perf.py discards stderr, so its official "correct"
  count includes that case. Do not claim 77 fully byte-identical programs.
- On the 76 common fully matching cases, CI instruction geomean is
  **0.452063 -> 0.449943** GNU R, 73 -> 74 faster. The broader official
  stdout/status metric is recorded in the recovery section below.
- Reduced error-call probe:

```r
f <- function(m) { for (i in 1:2) m[2, ] <- m[1, ]; m }
f(matrix(0, 1, 1))
```

  GNU R with R_ENABLE_JIT=3 reports value = m[1, ]; GNU R with JIT=0 and
  robigo's AST/VM report value = 0. A top-level assignment reports 0 in
  both implementations and is not a reproduction. Next: port compiled
  setter-call source preservation from eval.c, with a focused fixture,
  while preserving interpreted calls. Coordinate shared interpreter changes.
  CLI's performance runner should also retain/compare stderr; keep this
  existing difference explicit in performance reports.
- Artifacts: .robigo/r6/ci-csv*, ci-ports-csv*, ci-{main-,}outputs*,
  flex-warning*, flex-ast*, replacement-call-check*. No runtime change is
  kept from these investigations; this is the next-track handoff.

## 2026-10-29: inline closure buffers skip reservation calls

- Baseline: recovered #60, df9b0dc, current thin-LTO profile.
- run_inline_with unconditionally calls SmallVec::reserve_exact twice,
  even when its fresh stack buffers already fit the registers and slots.
  Gate these calls on inline capacity, as run_pre already does.
- A VM loop with native JIT disabled, f(x) = x + 1, measured by 20,000
  minus 10,000 iterations, costs **1,223 -> 1,160 instructions/iteration**
  (-5.1%). Each reserve helper used 28 instructions per iteration; both
  disappear. No allocator request is removed: these buffers already fit.
- Focused suite: spectralnorm_naive **1,002,483,620 -> 989,534,349**
  instructions (-1.292%). fib, binarytrees and profiler-microbenchmark
  change by at most +0.009%, within startup variation.
- Kept: full local suite geomean **0.487338 -> 0.487026** GNU R,
  77/77 faster before and after. Gate: 3 faster, 0 slower beyond 0.5%,
  0 broken, 0 fixed. lapply -1.95%, apply -1.66%, spectralnorm_naive -1.30%.
  Independent outputs remain 77/78, with spectralnorm_math unchanged.
- cargo test --release -p robigo-runtime -p robigo-cli passes: 38 runtime
  (including all 9,440 arithmetic oracle cases), 7 CLI, two integration
  tests and conformance in all three tiers. Formatting/source diff checks
  pass. CI run 40966972376 on 2884282 passes build, all ten corpus shards
  and performance. PR #90 is open for review, stacked on the green #60.
  Main advanced to dbf6754 during verification; CI waited for that main
  run's perf baseline before comparing the test merge, rather than using
  stale numbers.
  Artifacts: .robigo/r6/inline-reserve*, inline-calls-{base,after}.txt.

## 2026-10-29: round 6 follow-up, recursive argument allocations

- After #60, the paired suite has no correct benchmark slower than GNU R
  in instruction count (77/77). The next measured target is allocation in
  recursive calls, rather than the stale slower-benchmark table.
- `fib(21)` minus `fib(20)`, divided by 13,530 additional calls, costs
  3,083 instructions/call: VM body 640, closure_call 576.5, run_pre 159,
  end_context 152, plain_lookup 145.5, direct_call 108, argument binding 82.
- The old alloc-count.py's __rust_alloc counters undercount under thin LTO
  (403 requests reported for fib). A private copy counting mimalloc's
  malloc/zeroed/aligned entry points measures 1,273,305 allocation requests
  for fib(25), and 485,850 for profiler-microbenchmark. It excludes realloc
  requests and does not change the shared script.
- Fib's main allocation callers: Value::real 487,656, Env::with_frame
  252,408, closure_call 244,874. Startup is included in these totals.
- A startup-canceling allocation delta, fib(21) minus fib(20), gives
  54,121 requests / 13,530 additional calls = 4.000074 requests/call:
  exactly two in Value::real, one in Env::with_frame, and approximately
  one in closure_call, with one residual request across the whole delta.
- Boundary: Promise::forced stores a boxed Value; bind_positional installs
  that promise into an Env and Context.promargs retains it. Removing the
  scalar box or promise requires coordinating value.rs/interp.rs with the
  core owner, preserving expression identity, missing/substitute,
  parent-frame mutation, escaped promises, and the S4 context hooks.
  Environment reuse also requires env.rs ownership and escaping-frame
  semantics. Do not change those shared files during core round 4.
- GNU R contracts for that work: eval.c:forcePromise stores PRVALUE,
  applies ENSURE_NAMEDMAX, and clears PRENV after successful evaluation;
  make_applyClosure_env/R_execClosure define the binding and context
  lifetimes. Delaying a scalar box must still commit the forced value
  before any observable parent-frame mutation or introspection.
- Next: agree on unboxed forced-promise storage with core, then measure the
  same per-call instruction/allocator deltas and rerun introspection,
  promise, closure-context and full conformance tests before a separate PR.
  The boxed-scalar/promise allocation change needs core coordination.

## 2026-10-29: round 6, recover #60 on the current profile

- Rebased draft #60 onto main `e25e6cc`, retaining both sides' conformance
  entries and the scalar payload fixes from #86.
- The recovered kernel's plain Rust arithmetic allowed LLVM to commute
  competing NaNs. Select SSE destinations independently for multiply and
  add, following `arithmetic.c:real_binary` and the builtin port's shapes.
  Use packed operations with eight-element chunks and scalar tails.
- Extend the exact-oracle payload fixture from 768 to 6,144 comparisons:
  all eight scalar/vector combinations at lengths 2, 8 and 9. Main and
  the packed candidate pass. Expected output recorded with Debian R 4.5.0-3.
- Rejected intermediate scalar SSE kernel: `vecarith` 4,353,108,825 ->
  4,443,092,973 instructions (+2.07%), despite passing the payload fixture.
- Reuse `cadd`/`csub` for complex scalars. A new 99-case byte comparison
  also catches the recovered multiplication's stale operand order:
  `complex_binary` calls `cmul(y, x)`. Correct the scalar path accordingly.
- The packed candidate passes all crate tests (38 runtime, 7 CLI, two
  integration tests) and conformance in all three tiers, but initially
  regresses vector mandelbrot +0.526%/+0.539%. Inlining `math1_slow`
  improves the scalar benchmark but leaves vectors +0.565%/+0.570%.
- Reject non-double operands before calling `mul_add_next`, avoiding its
  cold failed call for every complex-vector multiplication. Paired family
  check now passes: ASCII +0.420%, noout +0.446%, naive ASCII -29.593%.
  Local full-suite remeasurement passes: geomean 0.489543 -> 0.487323,
  76/77 -> 77/77 faster, 2 faster, 0 slower above 0.5%, 0 broken.
- CI run 40959483877 passes build and all corpus shards, but rejects noout
  at just over +0.50%. Keep the PR draft and improve the vector path.
- Route complex vectors directly to the builtin before checking scalar
  conversions or double-kernel eligibility. The family recheck improves
  ASCII to +0.343%, noout to +0.351%; naive ASCII remains -29.099%.
  All 6,144 vector payload and 99 complex scalar comparisons pass.
- Final local full-suite result: **0.489543 -> 0.487338** GNU R;
  **76/77 -> 77/77** stdout/status-matching programs faster. Naive ASCII
  **1,210,079,365 -> 857,964,515** instructions (-29.098%). Vector ASCII
  +0.348%, noout +0.351%. Gate: 2 faster, 0 slower above 0.5%, 0 broken.
- Final `cargo test --release -p robigo-runtime -p robigo-cli` passes:
  38 runtime tests including all 9,440 arithmetic oracle cases, 7 CLI,
  two integration tests, all three conformance/guard checks. Formatting
  and source diff checks pass (verbatim oracle .Rout whitespace excluded).
  Independent benchmark outputs remain 77/78 before and after, with the
  known spectralnorm_math mismatch excluded. CI run 40962356010 on df9b0dc passes build, all ten corpus shards,
  and performance. #60 is Ready for review. CI paired geomean is
  0.456939 -> 0.454816 GNU R, 73/77 -> 74/77 faster; naive ASCII -29.821%,
  vector ASCII +0.363%, noout +0.389%, no gate regressions. CI still has
  flexclust, flexclust_no_s4 and csv slower than GNU R (unlike this host).
  Local focused FastR arithmetic/operators: 1,057/1,058 before and after,
  zero regressions.
  Artifacts: `.robigo/r6/`; immutable binaries, exact Debian trixie libraries,
  private build target `~/.cache/robigo-trixie-target`.

## 2026-10-29: round 5, scalar payloads and rejected muladd recovery

- Baseline: current main `74e8c73`, Debian trixie GNU R 4.5.0-3.
  Replaying the scalar portion of the recorded arithmetic oracle through
  closures reproduces **16/1,888** bit mismatches. All involve scalar
  integer/double `+` or `*` with competing NA/NaN payloads.
- Change: follow `arithmetic.c:do_arith` and the existing builtin port:
  mixed integer scalars use the right operand as the SSE destination for
  addition/multiplication. Logical operands retain double semantics.
  Extend the unchanged 9,440-case GNU R oracle through compiled VM
  expressions and eligible simple bodies. **All 9,440 cases pass**;
  independent closure replay improves **16/1,888 -> 0/1,888** mismatches.
- Local paired full-suite instruction geomean: **0.503263 -> 0.503515**
  GNU R, **76/77 -> 76/77** faster. Gate: **0 faster, 0 slower above
  0.5%, 0 broken, 0 fixed**. Keep for correctness, not a speed claim.
  The largest increase is `nbody_naive` +0.402%. Saved immutable binaries,
  independent output checks and main-only remeasurement of the two cases
  overlapping the candidate binary copy prevent baseline contamination.
- Verification: `cargo test --release -p robigo-runtime -p robigo-cli`
  passes (37 runtime, 7 CLI, 2 integration tests, all three conformance /
  guard checks). `cargo fmt --check` and `git diff --check` pass.
  The ratcheted `jit/muladd-payloads.R` fixture, recorded with the exact
  oracle, compares 768 competing-payload cases against unfused primitives.
- CI run `40934630158`, code head `657fd24`: build, all ten corpus shards,
  and performance pass. CI also reports no slowdowns beyond 0.5%; its
  reported fixed `spectralnorm_math` reflects a missing correctness flag,
  not a demonstrated fix. Independent local output checks remain 77/78.
  Absorbing its FastR artifacts adds 311 passing entries already enabled
  by main's native-package work; do not attribute those to this VM fix.
- Busy-host single-run wall-time geomean is 0.538x GNU R, 73/77 faster.
  Repeat on an idle host. `nbody_naive_2` now baselines at 0.988x GNU R
  instructions; profile from this baseline rather than the old 1.07x.
- Rejected: recovered #60's VM patch on current main, with the requested
  `Scan::raw` and `io::conn_getc` inline attributes. A new exact-oracle
  probe over 768 vector multiply/add/subtract cases with competing NaN
  payloads finds **0 mismatches on main, 102 with #60**. Revert the recovery
  and leave #60 draft. Its plain Rust kernel predates #74's payload rules.
  Next: use the shape-dependent destination rules from `arith_exact.rs`
  for both multiplication and addition, prevent LLVM commutation, and
  add byte-level competing-payload coverage before measuring performance.
  The old test's single NaN per element cannot establish this property.
- Local artifacts: `.robigo/round5/`; builds and measurements use the
  track's private `~/.cache/robigo-trixie-target` in `robigo-build:4.5.0`.

## 2026-10-29: round 4, matrix CI diagnosis and current-profile verification

- #80 run `40906985766` failed before comparing performance: downloading
  `perf-main` from run `40899570595` returned HTTP 503, "Egress is over
  the account limit." No perf artifact was uploaded after that failure.
  The build and all ten corpus shards passed. The baseline artifact is
  downloadable now. This failure is not evidence of a measured regression.
- Reapplied #80 on main `098bd95`, including #81's thin LTO profile and
  #70's base-package natives. Paired measurements use the same newly built stats, splines,
  tools, and parallel natives and Debian GNU R 4.5.0-3.
- Re-recording `tests/r/jit/matrix-elements.R` with the exact oracle
  reproduces every recorded byte. The independent full-suite output check
  matches GNU R on 77/78 programs. `spectralnorm_math` is still the known
  local mismatch; exclude it from performance claims.
- Busy-host single-run wall-time geomean: 0.565x GNU R over the 77 correct
  programs. This is provisional, not an idle-host comparison.
- Setup: `robigo-oracle:4.5.0` lacks zlib headers and gfortran required by
  current main. Use `robigo-build:4.5.0` and the track's private target
  `~/.cache/robigo-trixie-target`. No source workaround was needed.

- Current-profile CI run `40914475266` passed build, all ten corpus shards,
  and perf against main `098bd95` (baseline run `40912854891`). On the
  common 77 correct programs, instruction geomean **0.512146 -> 0.504341**
  GNU R, faster than GNU R **76 -> 76**. `matrixFor` **723,808,375 ->
  288,698,956** instructions (**-60.11%**). Gate: **10 faster, 0 slower
  above 0.5%, 0 broken**. The CI report's "FIXED spectralnorm_math" comes
  from the candidate's absent output flag; the independent local output
  check still differs, so it is not counted as fixed here.
- Fresh local full-suite comparison confirms CI: geomean **0.511560 ->
  0.503785**, **76/77 -> 76/77** faster than GNU R. `matrixFor`
  **723,270,029 -> 288,292,670** instructions (**-60.14%**).
  `scripts/perf.py compare`: **10 faster, 0 slower above 0.5%, 0 broken,
  0 fixed**. Both runtimes were measured; candidate correctness flags
  come from the independent byte-for-byte output check because a
  robigo-only instruction run does not retain stdout in its JSON.
- Keep the matrix patch on the current profile. The old CI failure was an
  artifact-download failure; no additional matrix code change was needed.
- Remaining batch: rebase #60 on current main, add the requested
  `Scan::raw`/`io::conn_getc` inline attributes, and verify independently.
  Then fix the scalar payload cases below, profile `nbody_naive_2`, and
  repeat wall-time checks when the host is idle.

### Follow-up: VM arithmetic oracle

The complete 9,440-case recorded arithmetic oracle was replayed through
VM closures using the saved main and matrix binaries. Both produce exactly
the same output: 16 divergences from GNU R, all scalar mixed integer/double
`+` or `*` with `NA_integer_` and another NaN payload in either operand
order. All vector/recycling shapes match. `vm::arith_fast` uses `real_op`
with the double/double left-destination convention; R's mixed scalar fast
path uses the right destination (`arith_exact::x_is_dest(true, 1, 1)`).
Next: preserve that operand order for mixed integer scalars, add a VM-level
oracle test, and measure it independently. No arithmetic code changed here.

Release verification passed: `cargo test --release -p robigo-runtime
-p robigo-cli` (34 runtime tests, 7 CLI tests, all 3 conformance/guard/tier
checks, 2 documentation tests). `cargo fmt --check` and `git diff --check`
pass. Local artifacts are under `.robigo/perf-round4/`.

## 2026-10-29: native matrix elements (round 3, PR #80)

- baseline: main `7ff8a140`, Debian GNU R 4.5.0-3, existing release
  profile. Local full baseline: instruction geomean 0.5536, 74/77 correct
  programs faster than GNU R. CI paired baseline: 0.5541, also 74/77.
- change: dimension-only integer/double matrices have native views with
  validated storage bounds. `Index2`/`SetIndex2` check each dimension and
  use column-major offsets. Existing ownership handles aliases; unsupported
  indices and values exit to the VM before accessing storage. Changes to
  `vm.rs` only set up the native matrix views and types.
- rejected first candidate: matrixFor -57.62%, but spectralnorm_alt_3
  +0.65%. Paired profiling found VM execution cheaper by 7.35 million
  instructions; Cranelift compilation cost outweighed that saving on its
  short matrix initialization loop.
- correction: disable Cranelift's development IR verifier in release
  builds, retaining it in debug builds. Its default is useful during
  compiler development but charges production executions for internal IR
  checks at multiple compilation stages.
- final paired CI result (run 40905386550, base run 40899570595): geomean
  **0.5541 -> 0.5461**, faster than GNU R **74 -> 74** on the same 77
  correct programs. `scripts/perf.py compare`: **10 faster, 0 slower**
  beyond 0.5%, 0 broken. matrixFor **762,737,326 -> 319,981,245**
  instructions (**-58.05%**, 0.9966x -> 0.4181x GNU R).
  spectralnorm_alt_3 +0.225%, inside the gate; robigo/nbody -7.95%,
  robigo/spectralnorm -3.77%, robigo/mandelbrot -3.06%.
- isolated matrix write: **759 -> 39 instructions/element**, GNU R 608;
  `callgrind-diff.py` confirms 40 instructions/element, nearly all native.
- verification: `cargo test --release -p robigo-runtime -p robigo-cli`
  passes, including conformance in interpreter, VM and JIT; fmt/diff checks
  pass. New ratcheted fixture recorded with Debian R 4.5.0-3 covers integer
  and double storage, reads/writes, aliases, fractional/invalid indices,
  row bounds with an otherwise valid linear offset, deoptimization after
  mutations, accumulators, single-element matrices, and changed dimensions
  on subsequent compiled entries.
- final full output check: 77/78 programs match GNU R, no runtime errors.
  `spectralnorm_math` still differs locally, as in the baseline; do not
  count CI's missing output flag as evidence this PR fixed it.
- wall time on the busy shared host: final geomean 0.491x GNU R. This is
  provisional; repeat `scripts/perf.py run --metric time --runtime both`
  when the host is idle. The duplicate final local instruction sweep was
  stopped after downloading CI's completed paired full-suite results.
  Local and CI artifacts are in `/tmp/robigo-vm3/` (base-all.json,
  matrix2-time.json, ci-base/perf.json, ci-final/perf.json).
- next: align VM scalar NA/NaN payloads with the arith_exact oracle;
  profile nbody_naive_2. #60 remains draft at c2ed404: main still lacks
  the release-profile change. Once it lands, rebase #60, add the requested
  Scan::raw/conn_getc inlines, and rerun the fair performance gate.

## Setup notes (this machine)

The host has GNU R 4.6.1 and glibc 2.44. Both runtimes are measured in a
`debian:trixie` container with `r-base-core=4.5.0-3`, valgrind 3.24 and
python3 (as CI), robigo built in the same container (the host binary needs
glibc 2.44's `sinh`/`cosh`, and would use a different libm). The local
baseline reproduces CI's `perf-main` artifact of `main` 7bb554c within 0.1%
(geomean 0.730 locally, 0.729 on CI).

## 2026-10-29: wrong output: `x[[i]]` of an unmaterialized `a:b` (PR #65)

- `fannkuchredux` and `fannkuchredux_naive` failed in compiled code only:
  `if (k <- perm1[[1L]])`: "the condition has length > 1". `perm1 <- 1:n`
  leaves an `RVal::Range` in the slot, and `vm::element` (the `x[[i]]` /
  `x[i]` fast path) treated every unboxed register value as a scalar, so
  `x[[1]]` returned the whole range: silently wrong values elsewhere
  (`x <- 1:5; c(x[[1]], x[[5]])` gave ten values). A range now yields its
  i-th integer; out of range takes the slow path.
- tests: `tests/r/subset/range-elements.R` (recorded, ratcheted; main
  fails it).
- `scripts/perf.py compare`: a program that failed or printed a different
  result in the base run is not counted, so its base count measured a
  different run (here one that stopped at the error): it is now listed as
  FIXED instead of compared (the fix was reported +128% SLOWER).
- result: every other benchmark within +-0.05% of main 7345297 and 0.00%
  of db8cced; both fannkuch benchmarks now count, both faster than GNU R
  (0.895x and 0.639x).
- the other wrong one, `spectralnorm_math`, needed `.Internal(La_rs)`
  (symmetric eigen, outside the VM); main has it now (CI lists it FIXED).

## 2026-10-29: next: `simple/matrixFor` (1.02x) natively

- `x[[i, j]] <- 5` in a double loop costs ~830 instructions per element in
  the VM (`run_with` 400, `get_attr` for `dim` 65, `set_element` 51,
  `matrix_offset` 36, `own_var` 31); native code rejects the loop
  (`ROBIGO_JIT_TRACE`: "unsupported at ...: SetIndex2"): jit.rs has no
  matrix type (`jit_type` takes attribute-free vectors only).
- plan: a matrix type (double/integer, a `dim` attribute and nothing else;
  nrow taken at loop entry, as nothing native can change it), `Index2` and
  `SetIndex2` with in-range scalar indices (offset `(i-1) + (j-1)*nrow`),
  everything else deoptimizing as `SetIndex` does. Expected ~50
  instructions per element.

## 2026-10-28: `for (j in seq(length = n))` as a native range

- why: `real/convolution_slow` (1.18x GNU R) runs its inner loop over
  `seq(length = ny)`, a call of the `seq` closure returning an ordinary
  vector, so native code rejected the loop and the VM ran it at ~2,000
  instructions per iteration.
- what: `for (v in seq(<prefix of length.out> = n))`, with `n` a variable or
  a numeric constant, compiles to `Op::ForSeqLength` (guarded on `seq`). When
  `n` reads without forcing as a scalar, `seq_length_range` reproduces
  `seq.default` for a single `length.out` argument (`One`): logical and
  integer as they are (`is.logint`), doubles `ceiling`ed; then finite,
  non-negative (and here within integer range), `integer()` for 0, else
  `seq_len(n)`. It only does so after checking, as `UseMethod` would (after
  a flush), that no `seq.integer`/`seq.numeric`/`seq.double`/`seq.logical`
  method is visible from the call or registered and that `seq.default` is
  base's. Otherwise the call is evaluated by the interpreter: errors,
  warnings, a lazy argument and dispatch are GNU R's.
- tests: `tests/r/basics/for-seq-length.R` (integer, double, logical and
  zero lengths, the loop variable after an empty loop, `length.out`/`len`
  spellings, the non-negative error for -1/NA/Inf/NaN, the "first element
  used" warning, a character length, user `seq.numeric`/`seq.integer`
  methods, a local `seq.default`, a local `seq`, a missing and a lazy
  argument, nested loops). JIT on and off print GNU R's output.
- result against main 414ae44 (1 faster, none slower):
  `convolution_slow` -80.6% (1.18 -> **0.23**); geomean 0.597 -> 0.584,
  faster than GNU R on 67 -> 68; kept.

## 2026-10-28 (second agent): baseline main 7d8c589, container

geomean **0.587** over 75 correct programs, faster on 68 (`mandelbrot_ascii`
now prints the right output and is counted). Wrong output: `fannkuchredux`,
`fannkuchredux_naive`, `spectralnorm_math`. Biggest losers:
`mandelbrot_noout` 1.61 (1.17 in the first baseline: a regression from
another track, to bisect), `mandelbrot_ascii` 1.38, `mandelbrot_naive_ascii`
1.22, `mandelbrot_noout_naive` 1.20, `convolution_slow` 1.18,
`nbody_naive_2` 1.06.

Tooling of this agent (private target dir, container builds):
`~/.cache/robigo-perfvm2/{build,measure,cg,fndiff}.sh`, `delta.py`,
`geo.py`, `dump.py` (writes a benchmark's program to a file).

## 2026-10-28: GNU R's JIT compile moment and its binding of base language functions (PR #51)

- why: GNU R compiles a closure at its 1st or 2nd call, and compiled code
  keeps the base `+`, `<`, `if`, `[<-`, ... it found at that moment
  (`tests/r/jit/base-rebinding.R`, recorded earlier: robigo differed in
  cases 3, 4, 6). Coordinator decision: compiled behaviour is GNU R's
  observable behaviour.
- what (`vm/rjit.rs`, new; `Closure::jit`, a `Cell<u64>`):
  - per closure, GNU R's JIT state: unseen, `MAYBEJIT`, `NOJIT`, compiled
    with its frozen set (bit per entry of `FROZEN_FUNS`: the
    `languageFuns` that have a base inline or setter handler; `%%`, `%/%`,
    `%*%`, `~`, `@` have none and are called as found), or namespace code
    (ignored: compiled at install, and lookups find base anyway).
  - `jit_check` at each call of an unsettled closure
    (`exec_closure_ex`): `R_CheckJIT` with strategy
    `STRATEGY_TOP_SMALL_MAYBE`, `JIT_score` (including its quirk: `if`
    scores its condition and consequent only; a nested `function`'s body
    counts), then `R_cmpfun`: `mayCallBrowser`, the JIT cache by body hash
    with `jit_env_match` (same top-level environment, every local variable
    known; a mismatch means `NOJIT`), `identical(.., 16)`.
  - the frozen set at compile time: not a formal or local
    (`findLocalsList`, with its shadowing of `quote`/`expression`/`local`),
    and the first frame binding the name (any value, as `exists`) is base
    and not below the top-level environment (`findCenvVar`,
    `getInlineInfo`).
  - closures created by compiled code are compiled with their creator's
    set minus their own locals (`cmpfun` compiles nested functions;
    `do_function` -> `jit_inherit`).
  - the effect: `Interp::eval_call`, finding a closure for a language
    function's name, calls base's instead when the frame belongs to a
    compiled closure that froze it (not for calls with `...` or empty
    arguments, which GNU R compiles as calls). Every failed VM guard falls
    back to the interpreter, so VM and VM+JIT agree with it.
  - inline call paths count calls too: VM and `lapply` inlining only for
    settled closures; native code assumes the loop calls an inlined closure
    at least twice.
- tests: `tests/r/jit/base-rebinding.R` and `-2.R` (the `if` score quirk,
  a closure defined in a call with a loop, closures made by compiled code,
  a body assigning `<`, a cache entry with other formals, `assign()` into a
  compiled frame, operators in promises and in a closure made in `lapply`'s
  argument, a score >= 50 without a loop), recorded with GNU R 4.5.0; all
  tiers print GNU R's output; both in the ratchet.
- known gaps (rare; all tiers alike): a language function rebound to a
  *builtin* (`` `*` <- `+` ``) is still called in compiled frames (the
  interpreter hook checks closures only: checking every builtin call
  costs); a top-level loop does not freeze bindings for its own duration;
  promises of a compiled closure forced after it returned, `eval(quote(..))`
  in compiled frames and default arguments are treated as compiled; the
  JIT cache never evicts (GNU R's has 1024 slots by address hash).
- perf, measured against main 32c6002 (container; main: geomean 0.573,
  faster on 71):
  - first version: worst +0.38%, from classifying each closure object at
    its first call (`topenv` and a walk of the body). Now an ancestor walk
    (global vs base namespace), the score and hash memoized per body,
    closures made by namespace code marked directly.
  - **interning moves everything.** The first versions interned ~10 names
    at startup (the frozen-function table, `findLocals`' special names).
    Every later symbol's id shifted, and with it the collisions of the
    frames' 64-bit masks (`env::sym_bit` = id & 63): unrelated lookups
    took the linear scan (`call_site_fun_fast` +16M on `profiler-shared`,
    `Scan::scanchar` +18M on `flexclust`, `Frame::position` costs moving
    between functions). This looked like codegen noise of +-0.5%. rjit
    now never interns: names are classified by text, memoized per symbol
    id, and looked up with `Sym::lookup` (new, sym.rs). After that the
    large programs moved by +0.01%.
  - a memo bug (class 255 + 1 wrapped to "unknown") re-classified the
    function name of every interpreted closure call (+0.58% on `Storage`).
  - result (none faster or slower than 0.5%): worst `mandelbrot_noout`
    +0.32%, `Storage` +0.29%, `nbody` +0.23%, startup +0.12% (the
    per-call state check, the interpreter hook, compiling user closures);
    geomean 0.5734 -> 0.5740, faster on 71.

## 2026-10-29: guards in namespace code (lead from the methods ledger)

- why: the methods agent found `vm::guard_ok_slow` at 13.6% of an S4
  `new()`. Frames of namespace closures (frame -> namespace -> imports ->
  base namespace -> global) are neither on nor directly below the search
  path, so (1) each guard walked the chain and, finding `+` etc. bound in
  the base namespace, called `resolves_to_base` with no memo; (2)
  `fresh_nested_ok` never pre-checked their guards; (3) their frames were
  not `stable`, so every flush invalidated every guard mark.
- what: where the environments past the frame's parent up to the search
  path are all locked (namespaces, imports, the base namespace: they cannot
  gain bindings), the outcome depends only on the parent's function
  bindings and the frame's own: memoized by the parent
  (`GuardMemo::outer`), the closure's guards pre-checked once per
  (epoch, environment, version, formals) as for closures below the search
  path, and the frame `stable` (also the pure-closure inliner's per-epoch
  check). Not noticed: a function rebound in an imports environment
  without a search-path change (GNU R's namespace code is compiled with
  base inlined anyway).
- tests: `tests/r/jit/locked-env-guards.R` (a locked chain, a binding
  replaced after `unlockBinding`, a frame binding `length`, namespace
  closures), recorded, ratcheted; three-tier conformance incl. the S4
  scripts.
- result: an S4 loop (2000 `new("P", x = i)`, 20000 calls of a one-method
  generic; `R_DEFAULT_PACKAGES=utils,stats,methods`) 2,788M -> 2,553M
  instructions (-8.4%). Suite against main 7345297: none faster or slower
  than 0.5% (no benchmark uses methods; an S4 benchmark in `benchmarks/`
  would track it).
## 2026-10-28: `a * b + c` in one pass; complex scalars without the builtins (draft PR, blocked on the build profile)

- why: `x * y + z` on double vectors made two passes (4.5 instructions per
  element over both); `shootout/mandelbrot_naive_ascii` (1.18x GNU R) runs
  `z <- z * z + c` and `abs(z)` on complex scalars, each a builtin call with
  a flush, an argument vector and attribute checks.
- what:
  - no new opcode (scalar `a * b + c` loops must stay native): the vector
    slow path of `*` peeks at the next op; if it is `+`/`-` of the product
    with an operand already at hand (register, constant, variable read
    without running code), and all three are attribute-free doubles of one
    length (or scalars), one loop computes `x[i] * y[i]` and then adds,
    in R's operand order, never fused (`vm/slow.rs`, 32 monomorphized
    kernels, 2.9 instructions per element), into an unshared temporary's
    buffer when there is one. Double arithmetic signals nothing.
  - complex scalars (with double/integer/logical): `complex_binary`'s
    functions (`cmul`/`cdiv`/`cpow`) directly; `abs` of a complex scalar is
    `hypot`.
  - all slow paths of `Arith`/`ArithL`/`RelL`/`Math1` are one out-of-line
    call after the flush (`vm/slow.rs`), so the dispatch loop does not grow.
- rejected, measured at codegen-units=1: complex scalars before the flush
  (mandelbrot_naive_ascii -38% instead of -28%, but `fannkuch` +0.6%,
  `binarytrees` +0.7%: code in `run_with`); the first layout with the
  checks inside `run_with` (`fannkuch` +1.5%).
- tests: `tests/r/vec/mul-add.R` (the four forms, recycling, NA/NaN one per
  element, signed zeros, the no-FMA case `(1+2^-27)^2 - (1+2^-26)`,
  attributes, integers, mismatched lengths, zero length, unchanged
  operands, loops, S3 `Ops`, a local `+`),
  `tests/r/math/complex-scalars-compiled.R` (all operators, mixed types,
  Inf/NaN/NA, overflow, signed zeros, abs, a mandelbrot loop, `%%` errors);
  both ratcheted; all tiers print GNU R's output. fastr slices: two
  complex-arithmetic cases newly pass, none fail.
- **blocked by codegen units.** Against main 32c6002 as CI builds it (16
  codegen units): `mandelbrot_naive_ascii` -28.3%, `vecarith` -0.8%, but
  `flexclust` +10.6% and `csv` +4.2% (`Scan::raw`/`conn_getc` no longer
  inlined into `scanchar`: unit boundaries moved) and `fib` +0.75%,
  `listWhile` +1% (inlining inside vm.rs's unit), none of it work this code
  does. With `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1` for both: geomean
  0.5696 -> 0.5678, faster on 72; `mandelbrot_naive_ascii` -28.5%, `nbody`
  -0.9%, `vecarith` -0.8%; worst `mandelbrot_ascii`/`mandelbrot_noout`
  +0.55% (the out-of-line slow path for complex vectors), then
  `nbody_naive` +0.41%.
- codegen-units=1 for main itself (full suite, 32c6002): geomean 0.5734 ->
  0.5696; `spectralnorm_alt_2` -13.3%, `profiler-shared` -2.4%, most -1..-2%;
  but `flexclust` +11.8%, `flexclust_no_s4` +10.5%, `csv` +4.7% (the same
  scan functions out of line: `#[inline]` on `Scan::raw` and
  `io::conn_getc` is the fix), `spectralnorm_naive` +2.0%, `vecops` +1.6%.
  Recommendation for the build-profile track: codegen-units=1 plus those
  attributes; then rebase this PR.

## 2026-10-28: profile: `real/convolution_slow` (1.18x), not changed yet

- the inner loop (`for (j in seq(length = ny)) { ij <- i + j - 1; z[[ij]]
  <- z[[ij]] + xi * y[[j]] }`) runs in the VM at ~2,000 instructions per
  iteration. Native code rejects it (`ROBIGO_JIT_TRACE`: "register 4 has no
  single type at ForStep"), because `seq(length = ny)` is a call of the `seq`
  closure returning an ordinary vector, not a range.
- next: compile `seq(length = n)` / `seq(length.out = n)` (a single
  argument) as `seq_len` where GNU R's result is `seq_len(length.out)`:
  `seq` is base's, S3 dispatch on the argument would reach `seq.default`
  (no `seq.integer`/`seq.numeric`/`seq.double` method visible), and
  `seq.default`'s checks hold (length 1, finite, non-negative; `ceiling` for
  doubles, the "first element used" warning for longer vectors). Anything
  else calls `seq`.

## 2026-10-28: compile-time binding: GNU R's behaviour recorded (`tests/r/jit/base-rebinding.R`)

Recorded with GNU R 4.5.0; robigo differed in cases 3, 4 and 6 (ported:
see the entry above).

| case | GNU R | robigo (main 7d8c589) |
|---|---|---|
| 1. small top-level closure, `<` rebound before its first call | user `<` every call | same |
| 2. rebound after its first call | user `<` every call (compiled at the 2nd call, which sees the user `<`) | same |
| 3. rebound after its second call (compiled) | user `<` never called | calls it |
| 4. closure with a loop (compiled at the 1st call), rebound after | never called | calls it |
| 5. closure created in a call, rebound after its 1st call | user `<` every call | same |
| 6. second closure with the same body as a compiled one | user `<` on its 1st call only (then the cached compiled code) | every call |
| 7. `+` bound in the closure's own environment | local `+` every call | same |
| 8. `length` rebound (not a language function) | user `length` (guarded inline) | same |

So the difference is confined to code GNU R has compiled. Operators and the
other `languageFuns` that resolved to base at compile time are inlined
without a guard. The port:
- per compiled body, count calls as `R_CheckJIT` does: compile at the 1st
  call when `JIT_score(body) >= MIN_JIT_SCORE` (50) or for loops, else
  `MAYBEJIT` and compile at the 2nd; port `JIT_score` and the default
  strategy from `eval.c`;
- at that moment, snapshot which `languageFuns` among the code's guard
  symbols resolve to base from the closure's environment (`getInlineInfo`,
  optimize 2), and freeze those guards: `Guard`/`ArithL`/`RelL` checks and the
  pure-closure inliner treat them as holding from then on;
- emulate the JIT cache: a body identical to one already compiled starts
  compiled, from the 2nd call (case 6);
- other base functions stay guarded (`BASEGUARD`), as robigo does now.

## 2026-10-28: `x[[i]] <<- v` / `x[i] <<- v` in the VM

- why: `shootout/nbody_naive` (1.32x GNU R) updates velocity and position
  vectors element by element with `body_vx[[i]] <<- body_vx[[i]] - dx *
  body_mass[[j]] * mag`: each one ran the interpreter's complex
  superassignment (`complex_assign_pre`, `eval`, `dispatch_or_eval_sp`,
  `subset2_dflt`), 1.28M instructions per `advance()` call.
- what:
  - `x[[i]] <<- expr` / `x[i] <<- expr` (index as in `index_form`) with a
    right-hand side the VM evaluates without calls (`is_simple`: arithmetic
    and comparisons of variables, constants and `x[i]`/`x[[i]]` reads)
    compiles to `Op::SuperSetIndex`, guarded on `<<-` and `[[<-`/`[<-`.
  - The binding `x <<- ` would assign (first enclosing frame binding `x`,
    else global) is updated in place by `set_element` when it is a plain
    binding of a non-object vector of a compatible type with an in-range
    index (or the append position), as `x[i] <- v` is locally; anything
    else is the complex superassignment after a flush, unchanged.
  - The current frame's cached copy of `x` is dropped first (it would make
    the vector look shared, so `make_mut` would copy it on every store).
  - `sym <<- expr` uses `is_simple` too.
- tests: `tests/r/basics/super-assign-index.R` (the binding used and a copy
  that must not change, growth, type changes, lists, names, a locked
  binding, a missing global, an S3 `[[<-` method, the expression's value,
  out-of-range and NA indices).
- result against main 414ae44 (2 faster, none slower): `nbody_naive`
  -61.8% (1.32 -> **0.50**; 1.28M -> 0.35M instructions per `advance()`),
  `queens` -19.5% (0.77 -> 0.62); geomean 0.597 -> 0.587, faster than GNU
  R on 67 -> 68; kept.
- carries over to G3: element superassignment in place under the same
  conditions as local element assignment.

## 2026-10-28: `a * b + c` on long double vectors: measured, no temporary to remove

- probe: `r <- a * b + c` on 10^6-element plain double vectors, in a loop
  (callgrind and allocator calls, 20 vs 10 evaluations).
- allocations: ~4.6 allocator calls per evaluation, i.e. the result
  vector's two allocations plus small change. The `+ c` step already writes
  into the unshared `a * b` temporary (`real_binary_inplace`), so there is
  no extra temporary.
- instructions: 5.5M per evaluation, half in `real_binary` (`a * b`), half
  in `real_binary_inplace` (`+ c`): two passes over the data.
- next (a new op, not started): a fused kernel for `x * y + z` (and
  `x + y * z`, `x * y - z`) on plain double vectors of equal length, in one
  pass computing `(x[i] * y[i]) + z[i]` as two separately rounded
  operations. A hardware FMA rounds differently and must not be used. It
  applies only where R's result is that elementwise: no attributes, equal
  lengths (recycling and the length warning stay on the builtins), and both
  operators guarded to base. Double arithmetic signals no warnings, and
  NA/NaN propagate elementwise as in the two passes.

## 2026-10-28: VM-to-VM calls: the non-recursive frames measured, the flattened call kept (PR #50)

- the S4 hook contract (docs/ledger/methods.md "Hook contract"): a path
  that runs a closure body pushes a `Context` exactly as `exec_closure`
  does and pops it with `end_context`. Both variants below do that, through
  helpers factored out of interp.rs (`enter_call`/`leave_call`,
  `call_sysparent`, `push_closure_context`, `bind_positional`), which the
  general path uses too.
- tried: VM frames without Rust recursion, as planned below. `Op::Call` of
  a compiled closure with positional arguments pushed a frame in the
  running `run_with`; every `return` there became "finish this frame"
  (callee value to the caller's register, errors unwound frame by frame
  with each caller's `break`/`next`); registers, slots and guard marks
  from a pool. Correct (conformance, all tiers), but per call 3,278
  instructions against 3,253 on main: the frame switch (saving and
  restoring 13 locals, ~140) and the storage handling (~175) cost what the
  layers did, and making `code`/`env`/`regs`/`vars`/`checked` reassignable
  in `run_with` spilled registers all over the dispatch loop (`fannkuch`
  +4.4%, `profiler-microbenchmark` +1.5%, all on lines without line info).
  Rejected; kept as a local tag `pv2-nonrecursive-frames`.
- kept: the same call setup as one function, `vm::direct_call`, with Rust
  recursion into `run_closure` as before: `enter_call`, `bind_positional`,
  the context, `run_closure`, a `return()` to the frame, `end_context`,
  `leave_call`, for a compiled body with positional untagged arguments
  (`direct_code`); everything else keeps `call_closure_ex`. It skips
  `call_closure_ex`/`apply_closure_ex`/`exec_closure_ex`'s general cases
  (argument matching, dispatch variables, the interpreter fallback).
  fib: 3,253 -> 3,160 instructions per call.
- remaining per call in fib (3,160): body 669, argument promises and the
  eager argument's box (`closure_call`) 582, `run_pre` 170, `direct_call`
  153, `end_context` 151, variable lookups 144, the caller-side `n - 1`
  guard memo missing per frame (`guard_ok_slow`) 68, frame allocation and
  release ~250. GNU R: 2,923.
- rebased on main 7345297 (#51 merged): `direct_call` calls `jit_check` after the context push, as `exec_closure_ex` does. Result against 7345297: 14 faster, none slower (`profiler-shared` -2.3%, `fib` -1.9%, `robigo/binarytrees` -1.6%, `spectralnorm_alt_3` -1.4%, `profiler-microbenchmark` -1.1%); geomean 0.5762 -> 0.5743, faster on 70 -> 71; three-tier conformance with both rebinding scripts passes.
- first result against main a6bd46e (default profile, container): 20 faster, none slower than 0.5% (worst `spectralnorm_alt_2` +0.42%); `profiler-shared` -2.7%, `fib` -2.2%, `robigo/binarytrees` -1.9%, `spectralnorm_alt_3` -1.6%, `profiler-microbenchmark` -1.4%, the `binarytrees`/`fasta` families -0.5..-1.0%, `Storage` -0.9%; geomean 0.5777 -> 0.5754, faster on 70 -> 71. (main a6bd46e itself has `flexclust` at 5.24G instructions: the scan functions are out of line there now too, see the codegen-units note.)

## 2026-10-28: plan: VM-to-VM calls without Rust recursion (measured, not started)

- per-call cost of `fib` on main 414ae44: 3,265 instructions (GNU R
  2,923). By function (`callgrind-diff.py`, `fib(21)` - `fib(20)`):

  | part | instructions |
  |---|---|
  | interpreting the body (`run_with`) | ~670 |
  | argument promises, caller-forced argument box, guard check (`closure_call` with `caller_promargs`) | ~520 |
  | `call_closure_ex` + `apply_closure_ex` + `exec_closure_ex` + `run_closure` | ~400 |
  | `run_pre` + register/slot drops | ~260 |
  | `end_context` | ~150 |
  | environment allocation and release (`with_frame`, `push_new`, drop) | ~145 |

- the design: `Op::Call` of a closure whose body is compiled, with
  positional arguments matching its formals one to one (the common case;
  everything else keeps the `call_closure` path), pushes a VM frame inside
  the running `run_with` instead of recursing:
  - the context (`Interp::frames`) is pushed as `exec_closure` does, and
    popped by `end_context`, so `sys.call`, `parent.frame`, `on.exit`,
    handlers and the S4 hooks see the same stack;
  - registers and slots move from the Rust stack to a VM-owned arena (one
    `Vec<RVal>` and `Vec<SlotVal>` grown and reused, frames as offsets), so
    a call allocates neither;
  - the per-run state of `run_with` (`pc`, `generation`, `token`,
    `recheck`, `loops`, `checked`, `Vars`) becomes a frame record saved and
    restored at call and return;
  - `Return` of the callee pops the frame and writes the unboxed result to
    the caller's destination register (no `unboxed_ret` hand-off);
  - errors unwind the VM frames (popping contexts and running `on.exit`
    in order) before propagating, as the Rust recursion does today;
  - native loops (`jit_enter`) and the interpreter's own calls keep
    recursing; depth limits (`MAX_DEPTH`, `check_stack`) count VM frames.
- expected gain: the ~400 instructions of call layers and much of `run_pre`
  and `end_context`'s fixed costs; a frame push in the same loop is tens of
  instructions. Prerequisite: the S4 agent's `exec_closure`/`end_context`
  hooks, which a VM frame push has to call too, so this follows their
  interp.rs changes.
- single guard check per call: measured at ~15 instructions (the callee's
  repeated key compare in `fresh_guards_ok`); folded into the frame push
  above rather than done separately.

## 2026-10-28: `x[[i]] <- v` in native loops

- why: `simple/vectorFor`/`vectorWhile` (`x[[i]] <- i` in a loop, 1.08x
  GNU R) never ran natively: the JIT accepted `SetIndex` only for `[`
  (`supported`: `two` rejected).
- what: native code's `SetIndex` handles attribute-free double and
  integer vectors with an in-range scalar index and a double/integer/
  logical value. On exactly those, `x[[i]] <- v` does what `x[i] <- v`
  does, and anything else (out of range, other types, lists, a zero or NA
  index) deoptimizes to the VM op as for `[`. So `[[` is accepted too.
- tests: `tests/r/subset/assign-elt2-loops.R`: loops inside functions,
  long enough to be compiled (`ROBIGO_JIT_TRACE` shows them compiled, one
  continuing in the VM after a deopt), with the edge cases after
  compilation (growth past the end, a double into an integer vector, NA,
  names, a list, an index of 0, logical values). JIT on and off print
  GNU R's output.
- result against the previous commit (5 faster, none slower):
  `vectorWhile` -80.6%, `vectorFor` -77.3%, `spectralnorm` -59.9% (its
  `eval_A_col[[j + 1]] <- eval_A(i, j)` loop now compiles with the inlined
  call), `fastaredux*` -0.9%. Ratios: `vectorFor` 1.08 -> **0.25**,
  `vectorWhile` 1.07 -> **0.21**, `spectralnorm` 0.54 -> **0.22**; geomean
  0.630 -> 0.597, faster than GNU R on 65 -> 67; kept.
- carries over to G3: the rule (a `[[<-` on an attribute-free atomic
  vector with an in-range scalar index is `[<-`).

## 2026-10-28: `sym <<- expr` in the VM, scalars updated in place

- why: `simple/superWhile` (`while (x < n) x <<- x + 1` on a free variable,
  1.00x GNU R) cost 3,510 instructions per iteration: `<<-` ran in the
  interpreter (`do_super_assign` evaluating `x + 1` through `eval`,
  `call_builtin`, `do_arith`, then `super_assign` -> `define`), boxing a
  new double each time (two allocations). GNU R's bytecode updates a
  scalar binding in place.
- what: `x <<- expr` with a symbol target and a pure right-hand side (no
  calls: `is_pure`) compiles to the right-hand side in a register and
  `Op::SuperSet`, guarded on `<<-`.
  - It finds the binding as `Interp::super_assign` does: the first
    enclosing frame of the frame binding the symbol, else the global
    environment.
  - A plain binding of a non-function value is overwritten without running
    code (`set_plain_in_place`): an unshared scalar of the same type in
    place, anything else replaced.
  - Everything else (locked or active bindings, promises, function values,
    a new global binding) goes to `super_assign` after a flush, unchanged.
  - The current frame's cached copy of the symbol is dropped.
- rejected, measured: compiling every `<<-` (with calls on the right):
  `nbody` +1.5% (vector right-hand sides with `as.vector(... %*% ...)`
  cost more through the VM's flushes and lookups than through the
  interpreter); still +0.75% with the plain-binding store, +0.25% with
  pure right-hand sides only.
- tests: `tests/r/basics/super-assign-loops.R` (the binding found, copies
  that must not change, type changes, nested closures, the global
  fallback, a locked binding's error, an active binding, visibility, the
  expression's value, a function value).
- result against main 1696902 (11 faster, none slower):

  | benchmark | instructions | x GNU R | allocations |
  |---|---|---|---|
  | `superWhile` | -56.6% | 1.00 -> **0.43** | 1.76M -> 0.39M |
  | `fasta` | -23.9% | 0.97 -> **0.74** | 1.18M -> 0.73M |
  | `fastaredux` | -29.1% | 0.96 -> **0.68** | 1.21M -> 0.64M |
  | `fasta_naive` | -22.3% | | 1.04M -> 0.61M |
  | `Storage` | -4.5% | 0.89 -> 0.85 | 1.07M -> 0.98M |

  Also `fasta_2/3` -22/-19%, `fasta_naive_2` -17%, `fastaredux_naive`
  -26%, `Bounce_nonames*` -1.5%. geomean 0.653 -> 0.630, faster than GNU R
  on 64 -> 65; kept.
- carries over to G3: compiling `<<-` with in-place scalar stores (G2's
  binding cells make the in-place store GNU R's `BNDCELL` case), and
  compiling it only where the right-hand side does not call out.
- noted: fastr `functions/TestS3Dispatch/0cd270304a9a4dfd` fails on main
  1696902 too (not this branch).

## 2026-10-28: argument vectors reused; scalar results returned unboxed to compiled callers

- why: per call, fib allocated the argument vector (1 allocation), the
  boxed result at `Return` (2: `Rc<Vector>` + `Vec`) and the boxed
  caller-forced argument (2), plus the promise and the environment.
- what:
  1. `Interp::arg_pool`: `end_context` clears the finished call's argument
     vector and keeps it (up to 64); `promise_args_with` and
     `caller_promargs` take one (`Interp::arg_vec`). Empty argument lists
     allocate nothing and skip the pool.
  2. Unboxed results: `Op::Call` calls closures through `closure_call` ->
     `Interp::call_closure_ex(.., unboxed = true)` -> `apply_closure_ex`
     -> `exec_closure_ex` -> `vm::run_closure(.., unboxed)`. The request
     travels as a parameter (a global flag could be taken by a condition
     handler run before the callee starts). The callee's `Return`/end of
     body (`body_result`) leaves a double/integer/logical scalar in
     `Interp::unboxed_ret` and returns a placeholder, only when the frame
     has no `on.exit` code: then nothing runs between the return and the
     caller taking the value. The existing `call_closure`/`apply_closure`/
     `exec_closure` are wrappers passing `false`.
  3. To keep the dispatch loop's code from getting worse (each step
     measured by instruction and by source line against main): the flag
     lives in `Vars` rather than as a `run_with` parameter; the slow paths
     of `SetIndex`/`SetIndex2`/`SetDollar` (complex assignment) moved to the
     cold `set_index_slow`; the closure arm of `Op::Call` to
     `closure_call`; `body_result` inlined.
- rejected, measured: a `run_with` parameter for the flag (+12
  instructions per iteration of flushing loops), the flag in `Context`
  (`fannkuch` +1.1%), the result slot as a thread-local instead of an
  `Interp` field (no change in the loop, more per call), `body_result` out
  of line (`profiler-microbenchmark` +0.73%).
- tests: `tests/r/basics/unboxed-returns.R` (invisible results,
  `on.exit`, `return()`, warnings handled mid-loop, `tryCatch` around a
  failing callee, recursion, named/attributed results, `sys.function`,
  `Recall`).
- result against main 963b3cf (13 faster, none slower):

  | benchmark | instructions | x GNU R | allocations |
  |---|---|---|---|
  | `fib` | -5.0% | 1.03 -> **0.98** | 2.09M -> 1.35M (-35%) |
  | `robigo/binarytrees` | -1.5% | 1.01 -> **1.00** | 1.95M -> 1.42M (-27%) |
  | `fasta` | -17.8% | 1.18 -> **0.97** | 1.71M -> 1.18M (-31%) |
  | `fastaredux` | -21.4% | 1.22 -> **0.96** | 1.83M -> 1.21M (-34%) |
  | `spectralnorm_alt_3` | -38.8% | 1.35 -> **0.82** | 3.08M -> 0.84M (-73%) |
  | `profiler-shared` | -3.6% | | 9.40M -> 4.89M (-48%) |

  geomean 0.665 -> 0.653, faster than GNU R on 57 -> 64; kept.
- carries over to G3: the calling convention (an unboxed scalar result
  for a compiled caller that asked, requested by parameter and handed over
  only where nothing can run in between) and argument-vector reuse. With
  G2's scalars the boxes get cheaper, but not free.

## 2026-10-28: conformance item: GNU R binds base functions when its JIT compiles a closure

- finding (from caller-forced-promises tests): after `g <- function(n) if (n < 2) ...` has
  run a few times, a later global `` `<` <- function(a, b) ... `` is ignored by `g` in GNU R;
  robigo's guards re-check and call the user's `<`. Coordinator decision: compiled behaviour is
  GNU R's observable behaviour (JIT on by default), so robigo must match it.
- the rules to port (R 4.5.0 sources):
  - `compiler/R/cmp.R`: `getInlineInfo` (optimize >= 2 inlines `languageFuns`, incl. the
    operators and `if`/`{`/`for`, when `findCenvVar` finds them as ftype "global"; guarded
    inlining via `BASEGUARD` for other base functions), `tryInline`/`getInlineHandler(name,
    package)` (handlers per package, so a definition found in `.GlobalEnv` is not inlined),
    `findCenvVar`/`isBaseVar`/`findLocVar` (compile-time environment: locals from formals and
    assignments in the body, then the closure environment's frames).
  - `main/eval.c`: `R_CheckJIT` (strategy `STRATEGY_TOP_SMALL_MAYBE` etc.: small top-level
    closures, `JIT_score < MIN_JIT_SCORE` = 50, are marked MAYBEJIT on their first call and
    compiled on the second; loops are compiled with `LOOP_JIT_SCORE`), `R_jit_enabled` level 3,
    and the JIT cache (compiled code reused by body, which is why a second closure with the
    same body as an already compiled one behaves as compiled from its first call:
    `g2 <- function(n) if (n < 2) ...` defined after the global `<` printed "global <" once).
- plan: port the decision procedure (when a closure is compiled; which calls are inlined
  without guard, guarded or looked up) into compile.rs, and make the VM's guards follow it:
  unguarded inlines keep base semantics after a rebinding, guarded ones (`BASEGUARD`) fall
  back to the call, as now. Conformance scripts: operators redefined before the first call,
  between the first and second call, after compilation, inside the closure's environment, and
  for two closures sharing a body.
- status: not started; after the fib/binarytrees call-cost work (not cheap: the compile-time
  environment analysis, MAYBEJIT and the body-keyed cache all affect the observable result).

## 2026-10-28: allocation counts (baseline for the allocation work)

- tool: `scripts/alloc-count.py` counts calls of `__rust_alloc`,
  `__rust_alloc_zeroed` and `__rust_realloc` per benchmark from callgrind's
  call graph, by calling function. Startup with `utils`/`stats` is ~0.55M
  allocations.
- finding (layout): a boxed double/integer scalar (`Value::real`/`int`) is
  **two** allocations, the `Rc<Vector>` and the `Vec` buffer inside it. The
  new core's inline vector payload (G2) halves every remaining box.
- `fib` (35,421 calls of `fib(21)`; 143k `Value::real` allocations) makes
  ~7 allocations per call: 2 boxed doubles (2 each: the result at `Return`,
  boxed in `run_with` because closures return `Value`, and the caller-forced
  argument), the forced promise, the environment (`Env::with_frame`), the
  `promargs` vector.
- next, by measured count:
  1. unboxed results between compiled callers and callees (the `Return`
     box). Needs a VM-to-VM call path that returns `RVal` past
     `call_closure`/`exec_closure`/`end_context` (interp.rs, shared file:
     a calling-convention change for its own PR);
  2. `promargs` without a heap vector (inline storage or a pool);
  3. boxed scalars in `profiler-shared` (`promise_args_with` 3.5M, `real`
     2.0M, `with_frame` 1.5M), `superWhile` and `nbody_naive`
     (`real`, `call_builtin`, `complex_assign_pre`);
  4. elementwise temporaries (`a * b + c`) only where bit-identical with
     warnings/NA in the same order.
- carries over to G3: the measurement and the attack list. The
  two-allocation box is today's layout and disappears with G2.

Largest allocators (total allocator calls, including startup):

| benchmark | allocations | top allocating function |
|---|---|---|
| rbench/simple/profiler-shared | 9.56M | promise_args_with |
| rbench/real/flexclust_no_s4 | 5.74M | finish_grow |
| rbench/real/flexclust | 5.40M | finish_grow |
| rbench/shootout/nbody_naive | 3.77M | real |
| rbench/shootout/nbody_naive_2 | 3.32M | finish_grow |
| rbench/shootout/spectralnorm_alt_3 | 3.24M | promise_args_with |
| rbench/shootout/mandelbrot_naive_ascii | 2.67M | arith_binary |
| rbench/shootout/nbody_3 | 2.49M | finish_grow |
| robigo/fib | 2.25M | real |
| rbench/shootout/nbody_2 | 2.23M | finish_grow |
| robigo/binarytrees | 2.10M | real |
| rbench/shootout/fasta_2 | 2.08M | finish_grow |

## For G3: which techniques carry over to the new core

The VM will be rebuilt on `robigo-core` (G2, PR #28). Most of what this
track measured depends on R's semantics and on which work can be skipped,
not on today's `Value`/`Env` layout:

- **Carry over (layout-independent):**
  - *Call-site function lookup through local frames* (`call_site_fun_fast`):
    `findFun` semantics, cached per call site, slow path for anything that
    could force or signal.
  - *Pure-closure inlining* with leaf and single-arithmetic arguments
    (`inline_args`, `run_inline_with`): evaluate arguments eagerly only
    through effect-free fast paths, and fall back to a real call before
    anything observable happens.
  - *Caller-forced promises* (`Code::strict` + `caller_promargs`): the
    strict-prefix analysis and the conditions under which eager evaluation
    is unobservable (a formal forced before any op that may run code, all
    guards known to hold, arguments readable without forcing). With the new
    core's promises it maps onto GNU R's own structure (PRVALUE set, PRENV
    NULL).
  - *Index promises forced in R's order* (`R_DispatchOrEvalSP` +
    `R_isMissing`), holding `x`'s value from before the forcing.
  - *Guard memoization keyed on (search epoch, environment id, function
    binding version)*, pre-checked once per closure environment, and only
    from a closure's second call for environments that live for one call.
    The new core's per-environment version counters (architecture.md) are
    exactly what this needs.
  - *Static per-site rejection memos* (`no_eager`): once a call site and a
    callee definition rule an optimization out, never retry it.
  - `%*%` row vector as dot products (bit-identical to `dgemv("T")`).
- **Specific to today's layout (redo, don't port):**
  - the `SmallVec` register/slot buffers and their reserve/drop costs;
  - the in-place `set_var`/`own_var` binding tricks, which depend on
    `Frame`'s `SmallVec<Binding>` and on `Rc::get_mut`;
  - moving cold paths out of `run_with` to tame LLVM's register
    allocation. The lesson carries over (keep the dispatch loop small;
    measure codegen effects by instruction against the base), the code
    does not.
- **Method:** compare against main by function (`BYFN=1 cmp.py`), by
  source line against main's source (`linediff.sh`), and by instruction
  (`asm.py`, callgrind `--dump-instr=yes`). Per-step gates hide cumulative
  drift, so re-measure against main before each PR.

## 2026-10-28: caller-forced promises for strict formals

- why: `fib` (1.12x GNU R; 3,767 instructions per call against GNU R's
  2,923) spent ~450 per call forcing `n` in the callee: the promise of
  `n - 1`, its bytecode, the caller's environment, a boxed result.
- what: `Code::strict` lists the variables a body reads, in order, before
  its first op that may run R code (guards, constants and reads first; an
  `ArithL`/`RelL` reads its operands and ends the prefix). At a call site
  whose arguments are leaves or single arithmetic ops (`site.eager`), the
  caller walks that list in order. For each formal it can supply without
  running code, it builds a promise already forced (`Promise::forced`,
  expression kept, environment cleared, as forcing leaves it): a constant,
  a variable readable without forcing, or an arithmetic op through its
  scalar fast path with its guard memoized. This requires the callee's
  guards to be known to hold (`fresh_guards_ok`; for nested closures only
  an existing `fresh_nested` memo), so no rebound `if`/`<` can run before
  the force. The walk stops at the first entry it cannot supply.
  `no_eager` memoizes static rejections per (site, callee formals).
- profile-driven fixes before keeping it: `profiler-microbenchmark` +3.1%
  (zero-argument calls paid ~128 instructions to reject: the guard check
  and the argument array came before the cheap tests; then 38 for the
  out-of-line call, fixed by the compile-time `site.eager` flag), and
  `profiler-shared` +1.1% (1.5M calls rejected for static reasons at ~43
  each; fixed by `no_eager`).
- tests: `tests/r/basics/caller-forced-promises.R` (`substitute`,
  `missing`, a callee that rebinds the caller's variable before forcing,
  a local rebinding of `<` in the caller, integer overflow and errors in
  arguments, `sys.call`/`match.call`, closures capturing the argument,
  NA).
- result: `fib` 3,767 -> ~3,500 instructions per call, -5.3% (1.12 ->
  1.06), `robigo/binarytrees` -2.3% (1.06 -> 1.03), `queens` -1.1%,
  `shootout/binarytrees` -0.7%; none slower (worst
  `profiler-shared` +0.14%); kept.
- findings, not fixed:
  - GNU R binds base functions when its JIT compiles a closure:
    after `g <- function(n) if (n < 2) ...` has been compiled, a later
    global `` `<` <- function(a, b) ... `` is ignored by `g` (and how
    often it is called before depends on when the JIT compiled `g`).
    robigo's guards re-check and call the user's `<`. A conformance gap
    of the VM to decide on: emulate compile-time binding, or keep R's
    AST semantics.
  - fastr `library/base/TestSimpleArithmetic/6f36cc66d662530c`
    (`as.complex(NA) %*% NaN` prints `NA` instead of `NaN+NaNi`) fails on
    main bf98504 too, so it is not from this branch.
- remaining per-call cost in `fib` (by function): `run_with` ~680,
  `caller_promargs` ~300 (mostly boxing the value and allocating the
  forced promise), `run_pre` ~165, `apply_closure`/`exec_closure`/
  `end_context` ~390, malloc/free ~260. Next: `promargs` without a heap
  `Vec` (SmallVec or a pool: `Context::promargs`, interp.rs, plus 4 uses
  in builtins/lang.rs), one guard pre-check per call instead of two
  (caller and `run_closure`), VM-to-VM calls without `call_closure`'s
  Rust recursion.

## 2026-10-28: tzdata 2026e on CI (`zones.R`), for the record

- #31's first CI run failed `tests/r/datetime/zones.R`. Debian
  `trixie-updates` moved tzdata from 2026c to 2026e between main's run
  (02:27 UTC) and #31's (02:38 UTC). Under 2026e, GNU R 4.5.0 prints
  different Europe/Dublin rows (1925-09-27 11:00 UTC is hour 11, not 12),
  and robigo (main, #31, #31 merged into main) printed exactly what GNU R
  prints. PR #33 made the test hermetic (vendored zoneinfo).
  `tests/r/datetime/zones-arith.R` pins the test's arithmetic in UTC.
- local setup: my image now has tzdata 2026e
  (`robigo-oracle:4.5.0`; 2026c kept as `robigo-oracle:4.5.0-2026c`).

## 2026-10-28: `u %*% M` for a row vector as dot products (builtins/array.rs, with permission)

- why: `shootout/spectralnorm_alt` (1.87x GNU R) is 93% `matprod_real` on
  a row vector, whose column loop ran one element per step (~40
  instructions per multiply-add).
- what: for `nrx == 1` (and `ncy != 1`) R calls `dgemv("T")`
  (R: array.c:matprod): each `z[k]` is `0 + sum(Y(j,k) * X(j))` over
  ascending `j`. The same products in the same order, now one dot product
  per column. Bit-identical by construction.
- tests: `tests/r/math/matprod-rowvec.R` (`%a` of the results, signed
  zeros, NA/Inf inputs, overflow, cancellation).
- result: `spectralnorm_alt` 2.31G -> 1.03G instructions (1.87 -> 0.83);
  `robigo/matrix` +0.1%; kept.

## 2026-10-28: no drift against main: startup and run_with code size

- why: each step above passed the gate against the previous step, but
  against main 12 startup-dominated benchmarks (~286M instructions, mostly
  startup) had drifted to +0.50..+0.58%, and `fannkuch`/`binarytrees`/`fib`
  to +0.4%: too close to CI's 0.5% gate, which compares with main.
- profile (by function, `BYFN=1 cmp.py`; by source line against main's
  source, `linediff.sh`; by instruction, `asm.py`):
  1. startup: the once-per-call guard check for closures defined in a call
     frame ran on the first call of each such closure; at startup most of
     them run once, so checking every guard up front (`resolves_to_base`,
     `search_fun`) cost ~830k. Now it runs from the second call with the
     same key (`fresh_seen`).
  2. `run_closure`: the 4-field key (with the environment's function
     version) cost every search-path closure call ~6 instructions (a larger
     frame, one more load and compare). Search-path closures keep the
     3-field key; closures defined in a call frame have their own cell
     (`fresh_nested`, behind an out-of-line check).
  3. `Compiler::leaf` ran twice per argument: `leaf_args` is derived from
     `inline_args` now.
  4. `Op::Index`: `match (fast, i, held)` moved an `Option<RVal>` through
     the fast path (20.8M vs 7.2M on `fannkuch`). Restructuring it inline
     made other ops' code worse (register allocation in `run_with`: +12.8M);
     the general cases of `Index`/`Index2` (index promises, `[`/`[[` calls)
     are now `#[cold]` out-of-line functions after one flush, which made
     `run_with` smaller: `fannkuch` -0.9%, `convolution_slow` -1.9%.
- remaining against main: `fib` +0.32% (its `fib(n - 1)` call sites now
  look the callee up and find it not inlinable, ~10 instructions per call;
  main skipped that because `n - 1` was not a leaf).
- result: against main 28 faster, none slower (worst `fib` +0.32%);
  geomean 0.683 -> 0.682, faster on 55; kept.

## 2026-10-28: pure closures inlined with arithmetic arguments; guards once per call for nested helpers

- why: `shootout/spectralnorm_naive` (1.52x GNU R) calls
  `eval_A(i - 1, j)`, a pure helper defined in the enclosing function, in
  its inner loop: 11.2k instructions per iteration (GNU R ~6.5k). Only
  calls whose arguments were all constants or variables ran inline, and
  the callee checked each of its guards per call (`run_closure`
  pre-checked them only for closures defined on the search path).
- what:
  1. Call sites keep `inline_args`: leaves, or one arithmetic operation on
     leaves (`i - 1`), evaluated before the inlined body only through its
     scalar fast path (which cannot warn or fail) and with its operator's
     guard memoized per site; otherwise the real call with promises runs.
     (`leaf_args`, which the JIT uses, are unchanged.)
  2. `run_closure` checks all guards once per (search epoch, closure
     environment, its function-binding version, formals) also for closures
     whose environment lies directly below the search path.
  3. `set_var`: a shared scalar binding of the same type (the loop variable
     a promise still references) is overwritten in place too, instead of
     falling through to `Interp::define`.
  4. The inline call moved out of `run_with` into `inline_site_call`, and
     `run_inline_with` takes the arguments through a callback that fills the
     slots directly (no intermediate `SmallVec`, no clone per argument);
     `lapply`'s `call_inline` uses it as well.
  5. `run_pre` reserves the register/slot `SmallVec`s only when they exceed
     the inline capacity.
- rejected, measured: treating frames two levels below the search path as
  `stable` (token with the parent's function version): -0.9k per
  iteration on `spectralnorm_naive` before (1), but +36 instructions per
  iteration of every flushing loop (`vectorFor` +5%, `fannkuch` +3.8%);
  after (1) its benefit was gone. Dropping registers/slots by hand in
  `run_pre`: worse (+1% on `fib`). Keeping the inline call inside
  `run_with`: +16 instructions per iteration on `vectorFor` from register
  pressure in `run_with`.
- profile notes (`callgrind --dump-instr=yes`, callers via
  `callgrind_annotate --tree=caller`): the remaining `fib` +0.17% is
  `SmallVec::drop` of the registers and slots emitted out of line in
  `run_pre` (36 + 19 instructions per call) where it was inlined before.
- tests: `tests/r/basics/inline-call-args.R` (overflow warnings from
  arguments and from the body, a caller-local `-`, `%%`/`%/%` by zero, NA,
  unused arguments not forced, logical operands).
- result: `spectralnorm_naive` -58.5% (1.52 -> 0.62; 11.2k -> 4.0k per
  iteration), `profiler-microbenchmark` -5.6%, `mandelbrot_naive_ascii`
  -2.4%, `apply` -2.2%, `fasta_naive` -2.1%, `lapply` -2.0%,
  `spectralnorm_alt_3` -1.9%, 11 more -0.5..-1.6%; none slower; geomean
  0.693 -> 0.683, faster on 53 -> 55; kept.

## Notes for other tracks

- everyone measuring performance: interning a new symbol early (a
  `Sym::new` in startup code, a thread-local table of names) shifts the id
  of every later symbol, so frames' 64-bit membership masks
  (`env::sym_bit`, id & 63) collide differently and unrelated benchmarks
  move by up to +-0.5% (seen: `flexclust` +0.39% all in `Scan::scanchar`,
  `profiler-shared` +0.38% in `call_site_fun_fast`). Compare per function
  before blaming codegen, and prefer `Sym::lookup` for names that need not
  exist.
- conformance-values-operators (`builtins/arith.rs`): when both operands
  of a double operation are NaN, GNU R (gcc, x86-64) keeps the first
  operand's payload (`NA_real_ * NaN` is `NA`, `NaN + NA` is `NaN`);
  robigo's kernels are plain Rust `a op b`, whose operand order LLVM may
  swap, so `a * b + c` prints `NaN` where GNU R prints `NA` (e.g. `f <-
  function(a, b, c) a * b + c; f(c(NA, NaN), c(NaN, NA), 1)` in GNU R:
  `NA NaN`).

- conformance-vectors (`builtins/array.rs`): `matprod_real` for a row vector
  (`nrx == 1`, `u %*% M`) runs its inner loop one element per column
  step, ~40 instructions per multiply-add; it is 93% of
  `shootout/spectralnorm_alt` (1.87x GNU R). A dot-product loop per output
  element (ascending `j` from 0.0, as the reference `dgemm` sums) gives the
  same bits.

## 2026-10-28: call sites find functions bound in local frames without the slow path

- why: in `shootout/spectralnorm` (1.82x GNU R) the inner loop calls
  `eval_A`, a pure helper defined in the enclosing function. Call sites
  looked functions up quickly only on the search path:
  `call_site_fun_fast` gave up at the first local frame that might bind
  the name, so the call was never inlined and went through promises, a
  frame and a flush (17.6k instructions per iteration).
- what: a local frame that binds the name to a closure, builtin or forced
  promise of one returns it (as `findFun` would; non-functions are
  skipped); unforced promises, active bindings and missing arguments still
  take the slow path.
- tests: `tests/r/basics/local-fun-calls.R` (helpers, shadowing by a
  non-function, function arguments forced lazily, redefinition in the
  loop, missing and active bindings, local recursion, warnings).
- result: 17.6k -> 3.1k instructions per inner iteration;
  `spectralnorm` -69.1% (1.82 -> 0.56), `profiler-microbenchmark` -3.7%,
  `spectralnorm_alt_3` -1.8%, `spectralnorm_naive` -1.1%, `profiler-rsa`
  -0.6%, none slower; geomean 0.705 -> 0.693, faster on 52 -> 53; kept.

## 2026-10-28: `x[[i, j]]` in the VM; index promises forced in place; cheaper slot write-backs

- why: `simple/matrixFor` (`x[[i, j]] <- 5` in a double loop) cost 5.45x
  GNU R: only `x[i, j]` had VM ops, so `[[` with two indices ran the
  interpreter's complex assignment (~6.4k instructions per element; GNU R
  ~600).
- what:
  1. `Index2`/`SetIndex2` take `[[` as well (`two`; `x[[i, j]] <- v` stores
     list elements as `[[<-` does).
  2. That alone made `spectralnorm_alt_3` 13.6% slower: its
     `eval_A <- function(i, j) eval_A_cache[[i, j]]` reaches `Index2` with
     `i`, `j` unforced promises, so the fast path always failed and paid for
     a fallback. `Index`/`Index2` now force index promises themselves when
     `x` is not an object, in R's order (x, then the indices: eval.c
     `R_DispatchOrEvalSP` + `evalListKeepMissing`), only when
     `R_isMissing` would say "not missing" (promise of a non-symbol, or of
     a symbol bound to a plain value in the promise's frame); `x`'s value is
     held from before the forcing, and a failed fast path calls `[`/`[[` with
     the held values rather than re-evaluating the call (a promise may
     rebind `x`).
  3. Still +3.9%: each `m[[i, j]] <- f(i, j)` iteration moved `m` into a VM
     slot (`own_var`: 3 frame lookups) and the call's flush wrote it back
     (`set_var` -> `Interp::define`: 2 more lookups, ~375 instructions).
     `own_var` now takes a plain binding in one lookup and `set_var`
     overwrites any plain non-function binding in place.
- tests: `tests/r/subset/matrix-elt2.R`, `tests/r/subset/index-promises.R`
  (order of evaluation, promises rebinding `x`, missing indices, dispatch
  with unforced promises, defaults), recorded with GNU R 4.5.0; conformance
  (all tiers) passes; fastr subsetting/functions slices unchanged
  (3138/3225 before and after).
- result: geomean 0.730 -> 0.705, faster than GNU R on 50 -> 52; 16 benchmarks faster,
  none slower: `matrixFor` -81.2% (5.45 -> 1.03), `nbody_naive` -35.1%
  (2.05 -> 1.33), `fastaredux_naive` -13.0%, `fasta_naive` -10.8%,
  `fasta_naive_2` -8.9%, `spectralnorm`/`spectralnorm_naive` -2.7%,
  `fastaredux` -2.4%, `mandelbrot_naive_ascii` -2.2%, `fasta*` -1.4..-1.7%,
  `vectorFor` -1.5%, `profiler-shared` -1.4%, `spectralnorm_alt_3` -1.4%,
  `Storage` -1.0%; kept.

## 2026-10-28: baseline (main 10d0a81)

instructions: robigo/GNU R geometric mean 0.730 over 74 correct programs; faster on 50
not counted (4): rbench/shootout/fannkuchredux, rbench/shootout/fannkuchredux_naive, rbench/shootout/mandelbrot_ascii, rbench/shootout/spectralnorm_math

Biggest losers: `simple/matrixFor` 5.45 (`x[[i, j]] <- 5` is not a VM
op), `nbody_naive` 2.05, the `spectralnorm` family 1.4-1.9, `fasta*`
1.1-1.3, `mandelbrot_naive_ascii` 1.30, `convolution_slow` 1.20, `fib`
1.11, `vectorFor`/`vectorWhile` 1.1, `binarytrees` 1.06.

Wrong output (bugs outside the VM, to look at): `fannkuchredux`,
`fannkuchredux_naive`, `mandelbrot_ascii`, `spectralnorm_math`.

<details><summary>full table</summary>

| benchmark | robigo instr | GNU R instr | ratio | |
|---|---|---|---|---|
| rbench/awf/Bounce | 707,305,990 | 825,271,903 | 0.86 |  |
| rbench/awf/Bounce_nonames | 679,454,447 | 883,548,558 | 0.77 |  |
| rbench/awf/Bounce_nonames_simple | 582,515,962 | 798,980,289 | 0.73 |  |
| rbench/awf/Mandelbrot | 295,906,518 | 497,760,722 | 0.59 |  |
| rbench/awf/Storage | 698,893,887 | 748,363,444 | 0.93 |  |
| rbench/real/convolution | 402,275,920 | 1,561,159,022 | 0.26 |  |
| rbench/real/convolution_slow | 2,166,599,480 | 1,799,232,018 | 1.20 |  |
| rbench/real/convolution_v | 729,414,303 | 733,969,146 | 0.99 |  |
| rbench/real/flexclust | 5,314,580,281 | 5,183,386,714 | 1.03 |  |
| rbench/real/flexclust_no_s4 | 5,764,701,927 | 5,796,739,300 | 0.99 |  |
| rbench/real/volcano | 286,146,047 | 531,117,525 | 0.54 |  |
| rbench/shootout/binarytrees | 503,998,184 | 687,255,881 | 0.73 |  |
| rbench/shootout/binarytrees_2 | 498,394,185 | 668,650,263 | 0.75 |  |
| rbench/shootout/binarytrees_naive | 519,418,949 | 664,387,524 | 0.78 |  |
| rbench/shootout/fannkuchredux | 285,911,305 | 730,170,002 | 0.39 | wrong output |
| rbench/shootout/fannkuchredux_naive | 285,923,859 | 651,918,956 | 0.44 | wrong output |
| rbench/shootout/fasta | 1,101,779,263 | 901,149,839 | 1.22 |  |
| rbench/shootout/fasta_2 | 1,232,021,218 | 967,608,768 | 1.27 |  |
| rbench/shootout/fasta_3 | 1,086,604,603 | 948,379,632 | 1.15 |  |
| rbench/shootout/fasta_naive | 1,076,580,567 | 961,751,152 | 1.12 |  |
| rbench/shootout/fasta_naive_2 | 924,936,200 | 952,761,439 | 0.97 |  |
| rbench/shootout/fastaredux | 1,210,266,893 | 960,732,802 | 1.26 |  |
| rbench/shootout/fastaredux_naive | 1,306,106,717 | 1,142,622,908 | 1.14 |  |
| rbench/shootout/knucleotide | 286,479,389 | 466,297,855 | 0.61 |  |
| rbench/shootout/knucleotide_brute | 286,742,503 | 463,861,957 | 0.62 |  |
| rbench/shootout/knucleotide_brute_2 | 286,515,075 | 463,693,649 | 0.62 |  |
| rbench/shootout/knucleotide_brute_3 | 286,530,760 | 463,764,366 | 0.62 |  |
| rbench/shootout/mandelbrot_ascii | 797,971,912 | 754,110,227 | 1.06 | wrong output |
| rbench/shootout/mandelbrot_naive_ascii | 1,451,992,303 | 1,116,959,646 | 1.30 |  |
| rbench/shootout/mandelbrot_noout | 995,174,926 | 847,875,611 | 1.17 |  |
| rbench/shootout/mandelbrot_noout_naive | 875,022,364 | 815,759,885 | 1.07 |  |
| rbench/shootout/nbody | 861,891,630 | 1,020,744,613 | 0.84 |  |
| rbench/shootout/nbody_2 | 976,928,304 | 1,039,116,024 | 0.94 |  |
| rbench/shootout/nbody_3 | 1,085,631,461 | 1,120,873,749 | 0.97 |  |
| rbench/shootout/nbody_naive | 2,776,987,488 | 1,355,419,139 | 2.05 |  |
| rbench/shootout/nbody_naive_2 | 1,386,608,936 | 1,290,535,487 | 1.07 |  |
| rbench/shootout/pidigits | 484,250,786 | 1,517,557,283 | 0.32 |  |
| rbench/shootout/regexdna | 286,037,950 | 426,175,535 | 0.67 |  |
| rbench/shootout/reversecomplement | 286,201,864 | 409,242,964 | 0.70 |  |
| rbench/shootout/reversecomplement_2 | 286,220,852 | 411,614,504 | 0.70 |  |
| rbench/shootout/reversecomplement_naive | 286,242,062 | 420,178,082 | 0.68 |  |
| rbench/shootout/spectralnorm | 2,697,507,200 | 1,443,529,040 | 1.87 |  |
| rbench/shootout/spectralnorm_alt | 2,307,836,945 | 1,235,707,101 | 1.87 |  |
| rbench/shootout/spectralnorm_alt_2 | 999,052,302 | 1,653,754,958 | 0.60 |  |
| rbench/shootout/spectralnorm_alt_3 | 2,333,916,053 | 1,631,301,809 | 1.43 |  |
| rbench/shootout/spectralnorm_math | 372,368,497 | 851,076,734 | 0.44 | wrong output |
| rbench/shootout/spectralnorm_naive | 2,717,441,503 | 1,740,930,666 | 1.56 |  |
| rbench/simple/bytecodes | 303,077,343 | 1,421,897,592 | 0.21 |  |
| rbench/simple/emptyFor | 371,559,341 | 1,354,110,092 | 0.27 |  |
| rbench/simple/emptyWhile | 321,173,773 | 1,581,987,531 | 0.20 |  |
| rbench/simple/lapply | 391,544,168 | 796,516,514 | 0.49 |  |
| rbench/simple/lapplyDots | 796,861,777 | 794,005,790 | 1.00 |  |
| rbench/simple/listFor | 378,475,666 | 949,971,642 | 0.40 |  |
| rbench/simple/listWhile | 489,045,062 | 1,247,892,405 | 0.39 |  |
| rbench/simple/matrixFor | 4,172,266,913 | 765,082,091 | 5.45 |  |
| rbench/simple/profiler-microbenchmark | 881,044,075 | 1,174,933,303 | 0.75 |  |
| rbench/simple/profiler-rsa | 575,551,002 | 1,104,080,689 | 0.52 |  |
| rbench/simple/profiler-shared | 6,620,471,296 | 6,935,667,565 | 0.95 |  |
| rbench/simple/scalarFor | 307,405,882 | 1,205,440,767 | 0.26 |  |
| rbench/simple/scalarWhile | 310,290,680 | 1,650,537,118 | 0.19 |  |
| rbench/simple/superWhile | 1,334,951,603 | 1,321,688,523 | 1.01 |  |
| rbench/simple/vectorFor | 1,524,401,065 | 1,377,027,569 | 1.11 |  |
| rbench/simple/vectorWhile | 1,653,874,388 | 1,516,542,769 | 1.09 |  |
| robigo/apply | 947,051,802 | 2,009,338,036 | 0.47 |  |
| robigo/binarytrees | 1,458,896,699 | 1,378,708,855 | 1.06 |  |
| robigo/csv | 1,006,606,203 | 1,542,426,146 | 0.65 |  |
| robigo/dataframe | 734,737,128 | 1,552,322,662 | 0.47 |  |
| robigo/fannkuch | 1,327,026,361 | 2,015,929,684 | 0.66 |  |
| robigo/fib | 1,202,226,293 | 1,079,788,250 | 1.11 |  |
| robigo/lm | 761,923,212 | 935,747,244 | 0.81 |  |
| robigo/mandelbrot | 336,450,739 | 983,589,471 | 0.34 |  |
| robigo/matrix | 756,837,353 | 1,410,574,442 | 0.54 |  |
| robigo/nbody | 488,373,385 | 2,632,988,973 | 0.19 |  |
| robigo/queens | 413,018,594 | 507,937,172 | 0.81 |  |
| robigo/spectralnorm | 354,145,940 | 2,641,067,067 | 0.13 |  |
| robigo/strings | 876,116,603 | 1,589,076,332 | 0.55 |  |
| robigo/vecarith | 4,464,965,026 | 5,409,460,545 | 0.83 |  |
| robigo/vecops | 2,539,798,129 | 4,015,961,152 | 0.63 |  |

</details>
