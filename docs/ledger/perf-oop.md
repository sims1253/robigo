# perf-oop: closure- and object-heavy programs

Scope: the programs where robigo lost at peak to FastR and Ř (and on some
to GNU R's steady state): `awf/Bounce`, `Bounce_nonames`,
`Bounce_nonames_simple`, `Storage`, `shootout/binarytrees_2`,
`binarytrees_naive`. Metric: `scripts/rivals.py` peak wall time against
GNU R, Ř and FastR; gate: `scripts/perf.py compare` of instruction counts
(no benchmark more than 0.5% slower) and conformance in all tiers.

Measuring on these VMs: the per-iteration numbers below come from
`perf stat -e cycles:u` of one process running the program body K times
minus a process running it 0 times (best of 3; this VM's user clock is
about 4.3 GHz). They track rivals.py's peak within ~10%. rivals.py itself
gives this VM's rival numbers, about 1.5x slower than the scoreboard
VM's (`~/oopwork/rivals-here.json`):

| benchmark | GNU R | Ř | FastR | robigo (main bf7332ee) |
|---|---|---|---|---|
| Bounce | 0.0437 | 0.0315 | 0.177 | 0.0335 |
| Bounce_nonames | 0.0445 | 0.0154 | 0.0117 | 0.0288 |
| Bounce_nonames_simple | 0.0318 | 0.00831 | 0.00788 | 0.0250 |
| Storage | 0.046 | 0.0239 | 0.00409 | 0.0355 |
| binarytrees_2 | 0.0191 | 0.015 | 0.0103 | 0.0119 |
| binarytrees_naive | 0.0184 | 0.0135 | 0.00831 | 0.0129 |

## 2026-11-04: element assignment in native closures, lapply into native code (Bounce_nonames 2.7x)

- Why: `bounce()` in `Bounce_nonames` (`ball[1] <- ball[1] + ball[3]`, `abs`,
  `list(ball, bounced)`) could not be compiled: `x[i] <- v` was not a native
  op in call trees (only in loops, which own their vectors), nor `abs()` of a
  value of any type. And `lapply(balls, ball)` called `ball()` through the
  AST interpreter, evaluating `FUN(X[[i]], ...)` with a promise for every
  element; its fast path ran only pure inline closures, and took any closure
  not analysed yet for one, so it declined `ball()` at every element.
- Change (`jit/closure.rs`, `jit.rs`'s `types_ok`, `builtins/apply.rs`):
  - `Op::SetIndex` on a variable of any type: `h_d_set_index` is the VM's own
    fast path (`vm::set_element`, `Rc::make_mut` copies a shared vector first)
    and leaves native code wherever the VM would make the call (append past
    the end + 1, coercion, `NULL`, classed objects, odd indices). A formal so
    modified enters as a value of any type, and so does one passed on to such
    a formal (`dyn_formals`, a fixed point over the tree); it gets its own
    copy at entry, as assigned formals already did. In a tree `x[i] <- v`
    makes `x` a local, never a cell. A typed vector view is never written
    (views may be shared): `SetIndex` on one leaves native code.
  - `Op::Math1` of a value of any type (`h_d_math1`, `vm::math1_fast`).
  - `lapply`/`sapply`/`vapply` call a closure of one formal that is not pure
    through `vm::call_native_rval` (counted and given up like a call site's
    `native_site_call`), so `FUN(X[[i]])` runs as native code when it can.
    The VM's frames are flushed: the builtin's own call was a real call.
- Rejected on the way: the root's per-formal flags as a `Vec<bool>` read in
  the native entry: `profiler-shared` +0.60% (bounds checks, inlined into
  `native_site_call`); a bit mask: +0.09%.
- Result (cycles per program body, this VM): Bounce_nonames 29.2 -> 10.9 ms,
  Bounce_nonames_simple 21.0 -> 8.1 ms (FastR 11.7 / 7.9, Ř 15.4 / 8.3).
  Instructions, startup included: Bounce_nonames -36.1%,
  Bounce_nonames_simple -30.6%, `robigo/apply` -27.9%.
- Open: what is left in `bounce()` is the helpers for values of any type
  (`h_d_index` 13%, `h_d_copy` 8%, `index_value` 7%): every `ball[1]` clones
  and drops. A native, owned double vector (copied once at entry, then
  loads and stores) would remove them.
- Verified: `tests/r/jit/native-closures-setindex.R` (the bounce step through
  lapply with a `<<-` counter, the caller's vectors unchanged, names kept,
  appending, coercion, out-of-range and odd indices, a `[<-` method, lists
  with `NULL` and list values, a local built in place, `sqrt` warning, an
  error part way through lapply, `sys.call()` from FUN), recorded with GNU R
  4.5.0, byte-identical in all tiers.
- Kept.

## 2026-11-04: nested arithmetic as call arguments (binarytrees 4x)

- Why: binarytrees' `tree()` was compiled to native code (round 20) but never
  ran there: `tree(2L * item - 1L, depth - 1L)` has an argument that is
  arithmetic of arithmetic, and inline/native call arguments (`InlineArg`)
  were a leaf, one operator over two leaves, or an element read. The site
  left native code, so every run was abandoned and after 128 misses the
  body went back to the VM for good. `check()`, whose arguments are `t[[2]]`,
  ran at 46 cycles per node; `tree()` at 3,400.
- Change: `NativeArg` (compile.rs), a call argument that is arithmetic whose
  operands are leaves or arithmetic again, up to three operators deep, or
  unary minus (`-i`, as `Op::Neg`: `-0` stays `-0`, `-NA` stays NA, `-TRUE` is
  `-1L`). A call site whose arguments need it gets `CallSite::native_args`
  (`inline_args` stays `None`), which only `jit::closure::call_tree` reads:
  inside a native tree such an argument is `CallArg::Nested`, typed and
  generated recursively, each operator's function guarded at entry like the
  bodies' and every operator by its scalar fast path, leaving native code
  where that gives up (overflow, NA where a warning is due, a value that is
  not a plain scalar). The VM makes these calls as before, with promises.
- Rejected on the way, by the gate:
  - `InlineArg::Nested`, evaluated by the VM too (its inline path, its native
    entry, `caller_promargs`): `robigo/closures` +1.08% and
    `spectralnorm_naive` +0.74% without running any of it. With the new arm
    `site_arg` was in a recursive cycle and LLVM stopped inlining
    `run_inline_with` into `inline_site_call` (+37 instructions per inline
    call). A separate evaluator for nested operands plus `#[inline(always)]`
    on the runner and its `read` got `closures` to +0.31%, but then
    `robigo/apply` +0.77%, `lapply` +0.51%; with the VM declining nested
    arguments (an arm returning `None`) `profiler-shared` was still +0.67%
    (`native_site_call`, +17 instructions per call). A separate type leaves
    the VM's code as it was: worst +0.03% on all four.
  - `convolution_v` stays at +0.8%: all of it is mimalloc zeroing large
    blocks (`_mi_page_malloc_zero`, 160 instead of 161 calls), the heap's
    layout after compiling base code with one more field per call site, not
    work.
- Result (cycles per program body, this VM): binarytrees_naive 9.80 -> 2.49 ms,
  binarytrees_2 9.43 -> 3.00 ms (FastR 8.31 / 10.3, Ř 13.5 / 15.0). Instructions,
  startup included: binarytrees_2 -25.7%, binarytrees_naive -25.8%.
- Verified: `tests/r/jit/nested-arith-args.R` (trees, nested and unary
  arguments in pure closures, signed zeros, NA and logicals under unary minus,
  integer overflow inside a nested argument, never-forced arguments that would
  warn or fail, errors from forced ones, `%%`/`%/%`, operators rebound after
  compilation), recorded with GNU R 4.5.0, byte-identical in all tiers.
- Kept.
