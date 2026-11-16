## 2026-11-16: T2, the observation modes' identity runs and E6's regression cases

- What: m1-plan T2 (docs/g3.md §15.3 E5 and E6's three regression cases,
  §16's `ROBIGO_NO_INPLACE=1` and `ROBIGO_MATERIALIZE=1`). Base: `main` at
  `85ba683c`.
  - **`ROBIGO_NO_INPLACE=1`** (robigo-rtcore `Cx::no_inplace`, read at
    install by robigo-rt): every `MAYBE_SHARED`/`MAYBE_REFERENCED` decision
    that lets an object be written in place says "copy" (`Cx::shared`,
    `Cx::referenced`). Ported to them: `make_unique` (so `unique_mut`,
    `unique_list` and `edit_pair`), `grow_in_place`, subassign's
    `make_local` and `clear_vector`, the attribute setters' `writable`
    tests, `[[`'s recursive getter, complex assignment's copies in the AST
    and compiled paths (`assign.rs`, `bcassign.rs`), the `for` loop's
    variable reuse and `environment<-`'s copy. Checks on a value being
    stored (`FIXUP_RHS`) and promise reference bookkeeping are not
    in-place decisions and are unchanged.
  - **Owned writes.** The first mode run found two places that write
    through `unique_*` where GNU R writes with no decision, into an object
    the code made its own earlier and kept where the write must show:
    `lapply`'s index (`INTEGER(ind)[0] = i + 1`, seen through `i`'s
    binding) and the recursive `[[<-` walk (`SET_VECTOR_ELT(xup, off, x)`,
    seen through the parent list). Under the mode the write went to a copy
    in a local root, so `X[[i]]` read 0 and `nest[[c("a", "b", "c")]] <- 2`
    was lost. New in rtcore: `Cx::owned_mut` and `Cx::owned_list`, GNU R's
    bare write (they assert the object is not shared, the condition under
    which production wrote in place too). The two sites use them.
  - **Mode runs in CI** (`scripts/rt-ci.py`): the RT conformance under each
    mode, and the rt crates' tests under `ROBIGO_NO_INPLACE=1` (the
    existing `ROBIGO_MATERIALIZE=1` step runs robigo-rt's). A mode run
    compares with the production recordings, refuses
    `ROBIGO_CONF_UPDATE`, and leaves out the cases whose `.requires` names
    the identity the mode gives up: new conformance features
    `inplace-identity` and `promise-identity` (robigo-cli's column always
    has both).
  - **getBindingType** (`tests/rt/args/binding-type-matrix.R`): a constant,
    a symbol, a call and a defaulted argument, before and after the first
    read, with the caller and the callee each as GNU R's AST (a closure's
    first call) and as byte code (past the threshold: a small top-level
    closure compiles at its second call), and a supplied default. The
    older `binding-types.R` covers more kinds but stops at
    `makeActiveBinding`, which the M1 subset lacks.
  - **E6 1, package ALTREP Length.** rtapi: `R_make_alt*_class` (all seven
    types), GNU R's class registry (`RegisterClass`), `R_new_altrep`, the
    40 method setters, `R_altrep_inherits`, `ALTREP`, `ALTREP_CLASS`,
    `R_altrep_data1/2` and their setters. rtcore `altrep`: GNU R's method
    table layouts and `Cx::altrep_length` (`ALTREP_LENGTH`), which runs the
    C method as a native entry from a hidden record, or raises GNU R's "no
    ALTREP Length method defined [class: c, pkg: p]". `length()` and
    `seq_along()` call it where and as often as GNU R does: `do_length`
    calls `xlength(x)` then `length(x)`, so the method runs twice and the
    second answer is the result. Before this, `length()` of a package
    ALTREP object returned 1 (`xlength`'s `unwrap_or(1)`).
  - **E6 2, in-place writes into BODY and FORMALS.** rtapi:
    `R_ClosureFormals`, `R_ClosureBody`, `R_ClosureEnv`, `R_ClosureExpr`.
    The tracking (§9.3, "tracked writes invalidate dependent code before
    its next use") is two header bits and two floors:
    - robigo-heap's `extra::AST_SOURCE` marks the cells of an expression
      compiled for GNU R's AST (robigo-rt's `Eval` profile), and
      `FORMALS_SOURCE` the cells of a compiled closure's formals. `code::build`
      marks them (`Cx::mark_code_source`); `duplicate` does not copy them.
    - The code stamp counter moved from robigo-rt's state to the isolate
      (`Cx::next_code_stamp`). A write through `set_car`/`set_cdr`/`set_tag`
      or the C API's setters (`robigo_rtapi_store`) into a marked cell
      raises the matching floor to the next stamp. Code below a floor is
      stale at its next use (`code::is_code`, `is_closure_code`): AST code
      after a write into its source, any closure's code after a write into
      any formals. Byte code is not made stale by a write into its source
      expression, because GNU R's byte code is an object of its own.
  - **E6 3, writes while compiled code is suspended.** R code called back
    from C (`rt_callback`) rebinds the function the suspended closure calls
    next, assigns and removes its locals, reclasses the object it dispatches
    on next, replaces its own default before the promise is forced (the
    promise keeps the old one, the next call promises the new), and writes
    the default call in place (the promise evaluates the written call). Each
    runs three times, as the AST and as byte code.
- Decisions, each where g3.md or the brief left a choice:
  - **Identity runs pin production in the Rust tests that read addresses.**
    `setters_write_in_place_when_gnu_r_does` (`tests/attrib.rs`) and
    `growth_reallocates_as_gnu_r_does` (`tests/subset.rs`) are E5's
    "separate production-mode expectations". They set the mode off whatever
    the environment says and check the mode's side next to it: every setter
    step copies, every growth step moves (200 and 1000 moves), with the
    same values.
  - **One invalidation rule, not per-closure bookkeeping.** Floors make
    every code object of a kind stale at once. Writes from C into compiled
    calls are rare, so recompiling everything once is cheaper than a
    reverse index from cells to code. `tests/code_source.rs` checks that
    ordinary evaluation (complex assignment, `substitute`, `on.exit`,
    recursion, `switch`, `...`, loops, in each profile and under the JIT)
    never raises a floor. Robigo's own writes never make code stale.
  - **GNU R's two behaviours are kept apart.** The oracle runs a write into
    an AST body at the next call, keeps running byte code after a write
    into the expression `R_ClosureExpr` returns (`body(f)` shows the
    write), and reads formals at every call. Hence the two kinds of floor.
  - **Running-body writes are excluded** (`tests/rt/capi/suspended-body-write.R`,
    `.requires: suspended-deopt`, a new conformance feature no build has).
    A callback that writes a later statement of the AST closure suspended
    in it: GNU R's `do_begin` reads the next cell live, so GNU R runs the
    new statement. Robigo-rt's suspended activation runs its old code, and
    the next call runs the new. Matching GNU R needs the §9.3 deopt of a
    suspended activation into code compiled from the rest of the live list,
    which nothing designs yet. The case and its 4.6.2 recording are in the
    tree, waiting for it.
  - **The default Length method is GNU R's error**, ported, not a "not
    implemented" refusal: its message is GNU R's. A negative length from a
    method is refused as not implemented (GNU R does not check it).
- Excluded cases, explicitly:
  - under `ROBIGO_NO_INPLACE=1`: `tests/rt/capi/inplace-identity.R`
    (`inplace-identity`: C reads objects' addresses before and after R's
    own writes; production matches GNU R, the mode copies, the values are
    the same);
  - in every run: `tests/rt/capi/suspended-body-write.R` (`suspended-deopt`);
  - under `ROBIGO_MATERIALIZE=1`: none (no case observes argument
    PROMSXPs from the start; `promise-identity` is defined for the cases
    that will).
- Proof (WSL2 workstation, release, private target `~/.cache/cargo-target-t2`):
  - RT conformance: production 232/237, `ROBIGO_NO_INPLACE=1` 231/236 (the
    identity case left out), `ROBIGO_MATERIALIZE=1` 232/237. The 5 failures
    are main's, in every mode (`ls`, `lockBinding`, `makeActiveBinding`
    are outside the M1 subset). Before the owned writes the
    `ROBIGO_NO_INPLACE=1` run failed 18 more cases.
  - New cases, recorded on GNU R 4.6.2 and ratcheted:
    `args/binding-type-matrix.R`, `capi/inplace-identity.R`,
    `capi/altrep-length.R`, `capi/closure-mutation.R`,
    `capi/callback-mutation.R`. Each also passes in both modes (but the
    identity case under `ROBIGO_NO_INPLACE=1`), and the ALTREP case under
    `ROBIGO_GCTORTURE=1` and `ROBIGO_GC_MOVE=1`.
  - Sensitivity: with `code_is_current` always true, `closure-mutation.R`
    fails on the AST body write, the default call written in place and the
    renamed formal.
  - The rt crates' tests pass under `ROBIGO_NO_INPLACE=1`, with the two
    identity tests pinned; `scripts/rt-ci.py` passes all steps.
- Cost: callgrind instructions per iteration, `scripts/idiom-vs-r.py --rows
  benchmarks/rt/calls-rows.toml --no-r`, main `85ba683c` against this
  branch (the floor check adds one load and compare to each closure-code
  lookup):

  COST_TABLE
- Also: rtcore's `alloc_vector_raw` documented its plain data as zeroed;
  the heap leaves it uninitialized (as GNU R's `allocVector` does). The doc
  now says so. The ALTREP class table is zeroed where it is made.
