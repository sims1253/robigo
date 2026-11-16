## 2026-11-16: the rtapi/capi deferral sweep (#666, #671, #682 and PPACCT's open items)

- What: the foreman's sweep of review items deferred from A1 (#666, #671),
  T1 (#682) and PPACCT (#692), each ported from GNU R 4.6.2's C where
  behaviour is involved, or found done already.
- Tree: branch `g3/sweep-exact` on main `36798309` (#692 merged). Binary:
  `cargo build --release --locked -p robigo-rtapi` (`robigo-rt`), private
  target directory. Machine: AMD Ryzen 9 5900X (24 threads), WSL2 Linux
  6.18, rustc 1.100.0, valgrind 3.27.1. GNU R: the 4.6.2 oracle image for
  every recording.

### Exactness

1. **checkArity** (#666 r4). robigo-rt had ten hand-written copies of
   util.c's `Rf_checkArityCall` (their comments named errors.c). They are
   one port now, `prims::check_arity` and `prims::arity_message`: an
   `.Internal`'s error is `error()`'s, against the context's call; a
   primitive's `errorcall`'s, against its own. Two copies were not GNU R's:
   conditions.rs's `.Internal`s (`stop`, `warning`, `.addCondHands`, ...)
   reported against the inner call (`errorcall`), where GNU R names the
   context (`doTryCatch(return(expr), ...)` inside `tryCatch`); and
   `do_maxVSize` checked an arity GNU R does not check (`asReal(CAR(args))`,
   the rest ignored: `.Internal(mem.maxVSize(Inf, 2))` is `Inf`).
   `tests/rt/builtins/check-arity.R`, recorded on 4.6.2 and ratcheted,
   covers both and the shared message for `.Internal`s, primitives and
   compiled specials (`gctorture`'s among them; `internal-arity.R` from
   #682 already had its text).
2. **A leak in a resumed loop iteration** (PPACCT's open gap: "no case
   reaches a leak there yet", `2026-11-16-ppacct.md`). Closed by
   `tests/rt/capi/ppacct-loop-resume.R`, recorded on 4.6.2 and ratcheted:
   a callback rewrites, through `support/ppleak.c`'s `pp_set_path`, a
   suspended loop's `while` condition (do_while reads `CAR(args)` at each
   iteration), a statement of a `for` body and of a `repeat` body (the
   body object the loop read when it started). The case exercises
   `Op::LoopResume` and its offsets, shown by breaking them: with its
   kind 0 instead of 3 the case fails, and with `for`'s five more removed
   its second iteration reports `'.Call', 17 then 18` where GNU R reports
   `22 then 23`.
3. **The `.Call` stub's narrowing and zeroing** (A1's follow-up). The
   zeroing is gone from both x86-64 stubs already (`2026-11-14-a1-capi-cases.md`
   for `.Call`, `2026-11-15-a1-review-follow-ups.md` (c) for the six-argument
   one), each measured there. The narrowing makes a difference: with the
   stub bypassed (`ROBIGO_CALL65_PLAIN=1`) and the test's own skip removed,
   `native_call_frames_keep_nothing` fails ("a stale word in the frames of
   a native call of 65 arguments pinned an object"); with it, it passes.
   It costs nothing: a no-op `.Call` is 1,674 instructions with the stub
   and 1,683 without (`call-noop`, `2026-11-16-ppacct/ppacct-rows.toml`).
   It stays. The 1 KiB zeroing of the non-x86-64 path (no stub there)
   cannot be measured on this machine and stays as it was.

### Hygiene

4. **`robigo_heap_try_call65`'s contract** (#671): its declaration
   documented it already; it now says that `nargs` is `0..=65` (a `.Call`
   of no argument is one) and the call in `Cx::call_native65` asserts that
   in debug builds. `6..=65` is the slot count the stub passes, not the
   argument count, so it is not what is asserted.
5. **#682 r2**: done in #692 (`f31d5b03`): the support object's cache key
   holds the compiler's arguments; native.rs's message names
   `NATIVE_MAX_ARGS`.
6. **#666 r3**:
   - Rinternals.h's `scripts/rt-capi-build.sh` exists (A1 added it).
   - The capi tests' support objects are content-keyed in the target
     directory, which checkouts share: a build now removes the other
     keys' objects no build has written for a day (an object another
     checkout uses stays; one it loses is rebuilt).
   - Test-C thread-local state: `Session::new` drains the log (#682), and
     each test runs on a fresh thread.
   - Unused `rt_*` helpers: none left (#682); every `rt_*` routine of
     `capi_test.c` is called by a test.
   - `malloc` is checked for NULL (#682).
   - `robigo_cglobals_layout` is an exact-size array and C's length is
     asserted (#682).
   - rt-ci.py's stale possessive ("robigo-rtapi's
     crates/robigo-rtapi/tests/conformance.rs") reworded.
7. **`.Platform$pkgType`** (#666 r4): GNU R 4.6.2 reads
   `R_PLATFORM_PKGTYPE` first and falls back to the configure value
   `PLATFORM_PKGTYPE`, which a unix build leaves unset (`"source"`,
   platform.c). The doc now says both.
8. **robigo-rtapi's symbols**: its 39 C API shortcuts are R1d's `RSym`
   entries since #682 (`CAPI_SYMBOLS`); the three `install`s of `class` and
   `names` left in dotcode.rs and memory.rs now read the pre-interned
   symbols.

### Proof

- Cases: `check-arity.R` and `ppacct-loop-resume.R`, recorded on 4.6.2 and
  ratcheted.
- `cargo test --release -p robigo-rtapi -p robigo-heap -p robigo-rt`;
  `scripts/lint.sh` clean.
