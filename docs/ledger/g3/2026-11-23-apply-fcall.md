## 2026-11-23: B, base's lapply and vapply keep their FUN(X[[i]], ...) call

- What: `do_lapply` and `do_vapply` (crates/robigo-rtbuiltins/src/apply.rs)
  built the call `FUN(X[[i]], ...)` afresh on every invocation, as GNU R's
  builds `R_fcall`.
  - GNU R evaluates that call with the AST interpreter, which compiles
    nothing.
  - robigo-rt compiles a call at its first evaluation (`eval::expr_code`:
    `X[[i]]`'s code, when its promise is forced) and caches the code on the
    call. So every invocation paid one compile: about 5.5k instructions of
    `lapply(1:10, f)`, as X measured.
  - Now each isolate keeps one such call for base's lapply and one for
    vapply, rooted as globals in `BaseCalls`, so the compile happens once.
- Which calls are kept: a call is kept with the closure it was made for,
  only when that closure is base's own `lapply` or `vapply` (the base
  namespace's binding, a lazy-load promise's value), and the builtin runs in
  that closure's own frame (the innermost function context's environment
  is `rho`). It is reused only under the same two conditions.
  - Under the `Bc` profile (`ROBIGO_RT_PROFILE`), the code compiled on the
    call takes cmp.R's decisions from the closure's frame and body, a local
    `[[` for one. Those decisions are the same in every call of base's
    closure, and need not be the same in any other.
  - Sol's case: a closure of the base namespace with its own `[[` reused
    base's compiled `X[[i]]` and returned `11 12 13` where GNU R gives
    `-1 -2 -3`. It now builds its own call. The case is in
    tests/rt/apply/lapply-kept-call.R and passes under the `bc`, `eval` and
    default profiles.
  - Sol's second case: `.Internal(vapply())` evaluated in a promise of an
    argument, so `rho` is another closure's frame (with its own `[[`) while
    the innermost context is base's vapply. It builds its own call
    (tests/rt/capi/lapply-call-mutation.R, the probes).
  - Before each reuse the call's shape is checked again: `fun(x[[i]], ...)`
    with every cell's car, cdr and type, and no tag or attribute on any
    cell. A call that no longer has it is replaced. C code can write into
    the `X[[i]]` it reaches through `sys.call()`, as GNU R allows; GNU R's
    next invocation would not see that, and robigo-rt's does not either.
    tests/rt/capi/lapply-call-mutation.R covers an index set by `SETCADDR`,
    a tag set by `SET_TAG` and an attribute set by `setAttrib`.
  - Every other `.Internal(lapply())` or `.Internal(vapply())` builds its
    call afresh, as before.
- **A deliberate difference from GNU R:** `sys.call()` inside FUN copies the
  call's spine (`shallow_duplicate`), so its outer cells are new each time,
  but the `X[[i]]` inside it is the same object in every invocation. In GNU
  R it is a new one each time.
  - The two are equal in value, so `identical()` cannot tell them apart.
  - The kept call and its `X[[i]]` are not mutable (`MARK_NOT_MUTABLE`), so
    R code's change to a retrieved call copies it first, and the next
    invocation's call is intact. The case modifies `sys.call()`'s result
    (`[[<-` on the call and on its `X[[i]]`), then calls lapply again.
  - `substitute(v)`, `match.call()` and `sys.function()` in FUN are GNU R's.
  - A nested lapply (lapply inside FUN inside lapply) evaluates the same
    call recursively; each invocation has its own `i`.
- mapply is left building its call each time. The call holds FUN's value,
  the `MoreArgs` values and the arguments' names, so it differs between
  invocations. Its cost is 4 compiles per invocation for two arguments (two
  nested `[[` each).
- Measured: callgrind, instructions per iteration of
  benchmarks/rt/calls-rows.toml's rows (N against 2N). robigo-rt is main
  dc256de9f against this branch; GNU R is the local 4.6.2 build.

  | row | main | this branch | GNU R 4.6.2 | ratio after |
  |---|---:|---:|---:|---:|
  | lapply | 109,343 | 103,802 | 59,244 | 1.75x |
  | vapply | 114,332 | 108,728 | 60,904 | 1.79x |
  | sapply | 201,432 | 187,093 | 126,541 | 1.48x |
  | mapply | 294,776 | 295,291 | 167,064 | 1.77x |
  | Map | 244,112 | 243,497 | 123,536 | 1.97x |

  Compiles per invocation (calls of `code::build`, N against 2N): lapply,
  vapply and sapply go from 1 to 0; mapply stays at 4.
- What is left of lapply's 44.5k extra instructions per call: forcing the
  `X[[i]]` promise (+22k), the closure call through `force_and_call`'s
  general path (+15.8k), and the loop (+3.3k). These are the next steps.
