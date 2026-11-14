## 2026-11-14: A1's capi cases on robigo-rt, recorded on 4.6.2, and the `.Call` stub without its zeroing

- What: the R-level half of A1 (m1-plan §B A1; docs/g3.md §15.3 E6):
  robigo-rt's variants of the capi corpus's four cases, experiment A's two
  probes as L8's gate cases, weak-keys-collections and experiment A's
  stress script, recorded on GNU R 4.6.2 and ratcheted; then §15.1's
  separate measurements of the `.Call` stub's narrowing and zeroing, on
  which the zeroing goes, and a stub that passes only the stack arguments a
  call has. The exports, entries and Rust-level proofs are
  `2026-11-14-a1-rtapi.md`'s (PR #666).
- Tree: branch `g3/a1-capi-cases` on `g3/a1-rtapi` `f78f04be`. Machine:
  AMD Ryzen 9 5900X (24 threads), WSL2 Linux 6.18, gcc 16.2.1, glibc 2.44,
  rustc 1.100.0. Binary: `cargo build --release -p robigo-rtapi`
  (`robigo-rt`, private target directory as the foreman allows). GNU R:
  the 4.6.2 oracle image for every recording and the base subset.

### The cases (`tests/rt/capi`, `tests/rt/stress`)

Each loads its C from `RT_CAPI_LIBS` (`tests/rt/capi/support/*.c`, checked
in): `scripts/rt-capi-build.sh DIR` builds it for robigo-rt (`cc -fPIC
-shared`, GNU R 4.6.2's public headers with `crates/robigo-rtapi/include`
in front, no `R CMD SHLIB`), `--gnur DIR` for the GNU R on `PATH` (its `R
CMD config --cppflags`). robigo-rtapi's conformance test builds and sets
robigo-rt's; `scripts/regen-expected.sh` builds and sets GNU R's.

| case | from | robigo-rt against 4.6.2 |
|---|---|---|
| `eval` | capi corpus, unchanged R | pass |
| `pairlist` | capi corpus without `capi_car_as_call` (foreman answer 2: no `SET_TYPEOF` in 4.6's API) | pass |
| `unwind` | capi corpus, unchanged R | pass |
| `extptr` | capi corpus with `UNPROTECT(6)` for its six protections (PR #666's open question: GNU R's own protect-stack depth below a `.Call` turns the corpus's `UNPROTECT(7)` into a warning) | pass |
| `pin_premature` | experiment A's probe (L8's gate: `Rf_cons`, `Rf_lang2`, `Rf_setAttrib` and four more under `gctorture(TRUE)`) | pass: all seven FALSE |
| `pin_extbuf` | experiment A's probe | pass: TRUE TRUE |
| `weak-keys-collections` | `tests/r/capi`'s case with its C checked in (foreman answer 3) | pass |
| `stress/finalizer-depths` | experiment A's stress script: a finalizer-carrying pointer made at R depth a, a collection at depth b (C churn, or `gc()`), a, b in 0..24 | pass: 0 of 625 missed, both kinds |

The stress script's churn is 200 allocations, not 20: with 20, GNU R
4.6.2 itself missed 471 of 625 in a run (20 small allocations seldom
trigger a collection), so the expected output would record GNU R's
allocation schedule. With 200, GNU R and robigo-rt both miss none, in about 2 s.

What the cases needed beyond PR #666:

- **The base subset, regenerated** with them as inputs
  (`scripts/base-subset.R`): 19 more closures bound (`Sys.getenv`,
  `local`, `reg.finalizer`, `gctorture`, `sort`, `order`, ...), 6
  primitives and 10 internals. The tracer kept every top-level closure it
  instrumented in its own `done` environment, so a closure whose
  environment is a weak key stayed reachable after the script removed it:
  weak-keys' traced run lacked "key-value cycle finalized" and the
  regeneration stopped. It now forgets closures no longer bound. (The
  regeneration also lists main's newer s3 cases, `method-memo` and
  `usemethod-object`, which the checked-in files predated.)
- **`R_RunExitFinalizers`** at the end of a session, as `R_CleanUp` runs
  them (after "Execution halted" when a script fails): weak-keys registers
  an `onexit = TRUE` finalizer. `.Last` is still not run: robigo-rt has no
  `R_dot_Last` yet.

### Narrowing and zeroing, measured separately (§15.1)

Configurations of the same tree: *base* (the stub narrows and zeroes the
1 KiB below its saved registers), *no zeroing* (the zeroing loop removed,
narrowing kept), *plain* (`ROBIGO_CALL65_PLAIN=1`: no stub, so neither).
Each ran the eight cases above plain, under `ROBIGO_GCTORTURE=1
ROBIGO_GC_STATS=check`, under `ROBIGO_GCTORTURE=1 ROBIGO_GC_MOVE=1` and
under `ROBIGO_GC_PIN_REPORT`: 96 runs, every one byte-identical to the
4.6.2 recording. The pin report has the same 4 lines in each
configuration, all naming `capi_pp_probe` or `capi_eb_probe` (the probes'
deliberately unprotected pointers). `cargo test --release -p robigo-heap`
passes without the zeroing, `native_call_frames_keep_nothing` included:
its planted words lie where the stub writes the stack arguments, so it
tests the narrowing.

Instructions per no-op `.Call("rt_noop3", 1L, 2L, 3L)` (callgrind, N =
20,000 vs 2N in a closure's loop, `scripts/idiom-vs-r.py ... --robigo
robigo-rt --no-r`):

| configuration | per call | vs base |
|---|---:|---:|
| base | 2,221 | |
| no zeroing | 2,130 | −91 |
| plain (no narrowing, no zeroing, no register clearing) | 2,104 | −117 |
| **no zeroing, arity-aware stub (this PR)** | **2,102** | **−119** |
| GNU R 4.6.2 (PR #666's measurement) | 3,789 | |

Decisions:

- **The zeroing goes** from the x86-64 `.Call` stub. Under pins (§3 step
  3) a stale word keeps memory, and never decides a weak reference or a
  finalizer, which is what the probes, the stress script, weak-keys and
  the pin report observe; none changed.
- **The narrowing stays.** Removing it changes neither probe either, but
  it is what keeps the trampoline's frame (its `jmp_buf`) and the Rust
  caller's callee-saved registers out of the segment: §3's "No Rust frame
  is ever scanned" is a rule, not a measurement
  (`a_rust_callers_registers_reach_no_segment`,
  `native_call_frames_keep_nothing`). It and the register clearing cost
  at most 26 instructions (no zeroing against plain).
- **The stub passes `nargs`** (PR #666's open question): `nargs - 6` stack
  arguments, plus one zero word when that count is even (six saved words
  below the return address need an odd count for 16-byte alignment), not
  all 59. The segment's top is the first word above them, as before. Per
  call the stub falls from 94 to 42 instructions and the trampoline gains
  9 (its seventh argument and the clamp); the whole call, −28.
  `stack_parameters_of_a_routine_are_roots` now also calls routines of 7
  and 8 arguments with exactly that many (a segment top one word low fails
  it), and `native_call_frames_keep_nothing` runs at 65, 6 and 8.

Left as they were: the six-argument stub (`call6`: ALTREP methods,
`R_ToplevelExec` and `R_UnwindProtect` bodies, C finalizers, `cend`)
still zeroes its 1 KiB, and so does the non-x86-64 path (no stub there;
H3's AArch64 follow-up). The same argument applies to both, but nothing
here measured them on their own.

### Proofs

- `ROBIGO_CONF_FILTER=tests/rt/capi` and `tests/rt/stress`: 8 of 8 pass and
  are ratcheted (`tests/rt/passing-rt/{capi,stress}`).
- The 96-run matrix above.
- `cargo test --release -p robigo-heap -p robigo-rtcore -p robigo-rtapi`;
  `scripts/rt-ci.py`, every step; clippy (`scripts/lint.sh`).

### Open questions for the foreman

- The six-argument stub's zeroing: remove on the same evidence (91
  instructions per finalizer, `R_ToplevelExec`, ALTREP method call)?
- `.Last` at exit: run it once robigo-rt has `R_dot_Last`, or raise "not
  implemented" when a script defines one? Neither the cases nor M1's
  scripts define one.
