# The compile census: what compiling calls costs, and where (2026-11-23)

E, at the foreman's request after X's finding (#988).

**Why.** robigo-rt compiles a call at its first evaluation (`eval::expr_code`):
there is no AST interpreter. GNU R's `eval()` of a call walks the AST and
compiles only closures (by call count) and top-level loops. So code that runs
once pays a compile GNU R never makes. Nothing in perf-rt showed this, because
each row runs its body inside one closure, compiled once.

## The census (main b84740012)

- **Method:** a measurement build counted `expr_code`'s compiles and cache
  hits per code object. Its miss path ran out of line, so callgrind could
  price it inclusive. Each row ran as N against 2N under callgrind, on both
  boots. The patch and the data are in E's research cache, not in the tree.
- **Zero compiles per iteration** under both boots:
  - every perf-rt bar and guard row;
  - e7, d2s, d0;
  - `eval(quote())`, `Reduce`, `Recall`, a `tryCatch` that does not error.

Compiles per iteration, and their share of the row's instructions:

| workload | compiles | share |
|---|---:|---:|
| a script's top-level line (`x_i <- i + 1`) | 1 | 53–55% |
| `do.call(f, list(i, 1))` | 1 | 16% |
| `runonce` (`eval(parse(text = "f <- ...; f(2)"))`) | 2 | 15% |
| `Map(f, 1:5, 1:5)` | 4 | 14% |
| `eval(parse(text = "i + 1"))` | 1 | 13% |
| `lapply(1:10, f)` | 1 | 5% |
| a `tryCatch` that errors | 1 | 3% |
| `sapply(1:10, f)` | 1 | 3% |
| an rdb start (`--vanilla -e 1`) | 253 (no JIT) / 339 (JIT) | 3.2% / 2.1% |

## Why a compile costs 3–11k

The phases, per compile (callgrind inclusive under the miss path):

| phase | top-level assignment | `do.call`'s call | `eval(parse())` |
|---|---:|---:|---:|
| whole compile | 10.6k | 2.9k | 7.5k |
| allocator traffic (fresh Vecs and a map per compile) | 4.5k (43%) | 0.7k (24%) | 3.7k (49%) |
| guarded inlining of a builtin | 2.3k | 0 | 1.8k |
| `code::build` | 1.4k | 1.0k | 1.2k |
| `Verified::new` | 0.6k | 0.2k | 0.4k |

The allocator traffic is what #988 removes (the compile's buffers kept in a
per-isolate pool), for every compile, cached or not.

## Against GNU R 4.6.2

Per iteration, rdb boot, robigo-rt against the local R-4.6.2-build (main
b84740012, before #988):

| workload | ratio |
|---|---:|
| top-level lines | 0.99x |
| try-err | 0.96x |
| start, `R_ENABLE_JIT=0` | 0.91x |
| start, JIT on | 1.06x |
| runonce | 1.26x |
| sapply | 1.27x |
| trycatch-err | 1.30x |
| do.call | 1.32x |
| Map | 1.46x |
| eval-parse | 1.58x |
| lapply | 1.90x |

## The decisions (the foreman, 2026-11-23)

- **No second evaluator.** D11's mini-eval is not built.
- **In order, each its own PR:**
  1. scratch buffers for every compile (#988);
  2. `do.call` through `apply::eval_call`, which compiles nothing. A probe
     build matched GNU R on `sys.call()`, `substitute()`, `match.call()`,
     `parent.frame()` and nine other probes: 23.7k → 19.3k per iteration;
  3. a one-shot tier for code the caller declares run-once: no guarded
     inlining, and no verification of the compiler's own output, through
     C1's constructor. Code with loops gets the full compile.
- **The apply family's call reuse is B's.**
- **D11's startup clause:** the start's compile share is 2–4%, so it rests
  on C1's translation cache (#979), not on the evaluator.

The run-once rows in `benchmarks/rt/calls-rows.toml`'s `cold` group carry
these workloads: `toplevel-lines`, `do-call`, `eval-parse` and `map`, beside
`runonce` and `trycatch-err`.
