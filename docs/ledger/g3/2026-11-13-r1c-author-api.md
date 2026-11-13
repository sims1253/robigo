## 2026-11-13: R1c, the builtin-author API after E6's review

- What: the five ranked changes of E6's ergonomics review
  (`2026-11-13-e6-ergonomics-review.md`, Sol, "pass with changes") in
  `robigo-rtcore` and `robigo-rtbuiltins`, proved by porting B1's three
  modules that motivated them. Stacked on B1 (#648), N1a (#645) and R1a
  (#642); R1b (#654) merged in for the next item.
- 1. **`cx.build_with`** (`outcome.rs`). The closure returns
  `Built::Done(result, meta)` or `Built::Fail(Diag)`. `meta` is plain data
  (its type is fixed outside the closure's higher-ranked `&'g Cx`, so it
  cannot carry a `Ref` out of the read interval) and comes from the
  attempt that succeeded, so a retried closure needs no reset; a `Diag`
  (GNU R's `error()` or `errorcall(call, ...)`, plain data) is raised once
  the interval has ended, and nothing is written. Warnings are signalled
  from `meta` after the build, in GNU R's order. `cx.build` stays, as the
  result-only case on top of `build_with`, so N1a's, B1's and R1b's other
  callers are unchanged. `cx.raise(diag)` raises a `Diag`.
  - paste: the `too_long` flag and the R_NilValue sentinel became
    `Built::Fail(Diag::error("result would exceed 2^31-1 bytes"))`.
  - arith: the `naflag`/`lost` flags reset at every attempt became the
    attempt's `(naflag, lost)`.
  - nchar: the staging `Vec<i32>` of counts became the result itself,
    filled in place, with the first uncountable element's error as a
    `Diag`; GNU R's `do_nchar` also allocates before its loop.
- 2. **Rooted-result helpers** (`root.rs`): `s.capture(cx, |cx, out| ...)`
  makes an `Out` in the scope, runs the callee, checks it wrote the slot and
  returns it as a root (paste's `as.character` call and its evaluation are
  two captures); `current.replace_from(cx, next)` replaces a root's object
  in one `&mut Cx` step, never empty in between (the guide's callback loop
  moves each result out of a per-element scope with it).
- 3. **Unwritten results**: `Out` is `#[must_use]`, `Root::is_written`
  tells an unwritten slot from one holding R_NilValue, `capture` panics on
  an unwritten one, and `Builtin::invoke` (the dispatch boundary in
  `robigo-rtbuiltins`, which B1's tests now call instead of `(b.fun)(..)`)
  panics when a builtin returns `Ok` without writing its result. All of
  B1's builtins pass through it.
- 4. **Registry**: `op!("+", PLUSOP, arith::do_arith, ...)` writes a shared
  function's names.c code once and instantiates `do_arith::<PLUSOP>` from
  it, so the function's specialization and the row's `code` cannot
  disagree; `row` checks the eval column's digits at compile time (X in
  0..=2, Y and Z in 0..=1). `R_Visible` is the isolate's
  (`cx.visible()`, `cx.set_visible()`): `invoke` sets it from the column
  before the call (a special always, a builtin unless X = 2) and after it
  (unless X = 2), as `eval.c` does, and before every `.Internal` as
  `names.c:do_internal` does; a `Visibility::Custom` builtin sets it
  itself, which the variant's docs say.
- 5. **Admission** (`handle.rs`): `Ref::std_vector()` is `Some(StdVec)` for
  NULL and GNU R's `isVector` types in standard storage, whose `len()` is a
  `usize`; `None` for a package ALTREP object and for other types. The
  ports read lengths through it: in paste a package ALTREP list or
  character element now raises "not implemented yet" where its `None`
  length was read as 0 (and a character element then panicked at
  `Strings::of(..).expect("checked before")`); in arith the operands are
  admitted after `do_arith`'s ALTREP branch and type check. The crate docs
  gained the author guide (check, build, signal; a scalar result, a
  warning loop, a callback loop with bounded scopes, `error` against
  `error_call(None)` and `error_call(Some(call))`, `?Sized` for generic
  adapters over the unsized `Cx`), as running doctests.
- **R1b's review, deferred here** (OpenCodeReview's 8 findings on #654).
  An EXTDATA release ran exactly once only if no panic unwound out of a
  collection: a heap dropped mid-sweep lost the calls of the block being
  walked (its start bits were cleared before the walk), and a call queued
  but still armed in its slot would have run twice. Now a call is spent
  once taken (`take_extdata_release` writes `None` back, for the sweep and
  for `Drop for Core` alike), and the sweep keeps a block's start bits
  until its walk is done; a unit test panics at each death of a sweep,
  before the call is taken and after it is queued, and counts each release
  once (it fails with either change undone). EXTDATA's words are read
  through `OFF_EXTDATA_PTR`/`_OWNER`/`_RELEASE` (lib.rs, beside the
  layout) everywhere; the sweep frees chunks with `Chunk::layout()`.
  `alloc_extdata`'s `# Safety` says what a release may do at a safe point
  (the embedder's rule) and at the heap's drop (nothing with the heap),
  `Drop for Heap`'s doc no longer says no release runs, the Miri guard is a
  `cfg!(miri)` branch (no unreachable code), `fill` is `fill_cx`, the
  large-request test says why it skips under `ROBIGO_GC_STATS=check`, and
  the compile-fail guard also fails on a `.stderr` without its case.
- Proof (WSL2 workstation, Rust 1.100.0): B1's tests through `invoke`,
  their GNU R 4.6.2 oracle cases among them (`arithmetic_matches_gnu_r`,
  `paste_matches_gnu_r`, `nchar_matches_gnu_r`, ...); `tests/author.rs` (7
  tests: `build_with`'s data and diagnostics, `capture` and its refusal of
  an unwritten result, `replace_from`, the admission, `R_Visible`); the
  registry's unit tests (visibility per eval column, the unwritten-result
  panic, an out-of-range eval column, one row per code); the guide's
  doctest; `scripts/rt-ci.py`; clippy.
- Review (sol-loop, gpt-6.1-sol): round 1 found `invoke` leaving
  `R_Visible` alone before an X = 2 `.Internal` (GNU R's `do_internal` sets
  it before every internal; fixed, with an entry-visibility test that fails
  under the old rule), and two guide examples that taught what the review
  removes (a `sqrt` that unwrapped a double after admitting any vector, an
  `lapply` that read a failed admission as length 0); both now admit
  explicitly. Round 2: approved.
