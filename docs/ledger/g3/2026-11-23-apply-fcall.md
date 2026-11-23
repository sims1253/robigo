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
- Which calls are kept: only those evaluated in a frame of a closure of the
  base namespace (base's own `lapply` and `vapply`).
  - Under the `Bc` profile (`ROBIGO_RT_PROFILE`), the code compiled on the
    call takes cmp.R's decisions from that environment, and they are the
    same there for every invocation.
  - Any other `.Internal(lapply(...))` builds its call afresh, as before
    (tests/rt/apply/lapply-kept-call.R: another closure's `Y`/`G`, and a
    frame that rebinds `[[`).
- **A deliberate difference from GNU R:** `sys.call()` inside FUN returns
  the same call object in every invocation, where GNU R returns a new one
  each time.
  - The two are equal in value, so `identical()` cannot tell them apart.
  - The kept call and its `X[[i]]` are not mutable (`MARK_NOT_MUTABLE`), so
    a change to a retrieved call copies it first and the next invocation's
    call is intact. The case modifies `sys.call()`'s result (`[[<-` on the
    call and on its `X[[i]]`), then calls lapply again.
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
  | lapply | 109,343 | 103,758 | 59,244 | 1.75x |
  | vapply | 114,332 | 108,682 | 60,904 | 1.78x |
  | sapply | 201,432 | 187,252 | 126,541 | 1.48x |
  | mapply | 294,776 | 295,291 | 167,064 | 1.77x |
  | Map | 244,112 | 243,498 | 123,536 | 1.97x |

  Compiles per invocation (calls of `code::build`, N against 2N): lapply,
  vapply and sapply go from 1 to 0; mapply stays at 4.
- What is left of lapply's 44.5k extra instructions per call: forcing the
  `X[[i]]` promise (+22k), the closure call through `force_and_call`'s
  general path (+15.8k), and the loop (+3.3k). These are the next steps.
