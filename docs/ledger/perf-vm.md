# perf-vm: the VM and the JIT

Metric: `scripts/perf.py run --metric instr`, geometric mean of robigo/GNU R
instructions over programs whose output matches GNU R's; the number of
benchmarks faster than GNU R. Gate: `scripts/perf.py compare` (no benchmark
more than 0.5% slower) and `cargo test --release -p robigo-cli --test
conformance` (interpreter, VM and VM+JIT against each other).

## Setup notes (this machine)

The host has GNU R 4.6.1 and glibc 2.44. Both runtimes are measured in a
`debian:trixie` container with `r-base-core=4.5.0-3`, valgrind 3.24 and
python3 (as CI), robigo built in the same container (the host binary needs
glibc 2.44's `sinh`/`cosh`, and would use a different libm). The local
baseline reproduces CI's `perf-main` artifact of `main` 7bb554c within 0.1%
(geomean 0.730 locally, 0.729 on CI).

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

Recorded with GNU R 4.5.0; not yet in `tests/r/passing.txt` (robigo
differs in cases 3, 4 and 6).

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
