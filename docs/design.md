# robigo: design

robigo is an attempt at a faster, modern runtime for the R language, written in
Rust. This document records what we learned from earlier attempts (in R,
Python, Julia and recent AI-assisted compiler ports), the architecture that
follows from it, and the goalposts set by a design review (§5).

## 0. Goal and ground rules

**Make R as fast as possible.** Existing R scripts and libraries (CRAN
packages) must run unchanged; nothing from existing R runtimes (GNU R's C
internals, byte-code compiler, memory manager) has to be reused. Speed is
the objective, compatibility the constraint:

- **Results are bit-identical to GNU R by default.** Compiling loops,
  removing allocations and fusing elementwise vector operations do not
  change results. Reductions (`sum`, `mean`, `cumsum`, ...) stay
  sequential and accumulate in long double like R; reordered, SIMD or
  parallel reductions are allowed only in an opt-in fast mode.
- **Packages with compiled code are supported through R's C API**,
  implemented over robigo's own object model, so CRAN packages can be rebuilt
  against robigo's headers. Engine work comes first, but no design choice may
  block the C API (vector payloads stay at stable addresses; handles can
  be protected).
- **Work is prioritised by profiles of real workloads** (`benchmarks/`),
  with GNU R as the black-box semantic reference (`tests/r`).

## 1. What "better runtime for R" has to mean

R's value is its ecosystem: base R, the recommended packages and ~20k CRAN
packages, a large share of which contain C, C++ or Fortran. A runtime that
does not run that code is a different language. Every alternative R
implementation so far has run into this:

| Project | Approach | What happened |
|---|---|---|
| FastR (Oracle, Truffle/Graal) | AST interpreter + JIT on the JVM, emulated C API | Fast at peak, but warmup, memory and C-API emulation cost; maintenance mode since 2023 |
| Renjin (JVM) | Interpreter + GCC-bridge compiling C/Fortran to JVM bytecode | ~17% of CRAN passed all tests (2016); dormant since 2021 |
| pqR | Fork of GNU R 2.15 | Real wins (precise refcounts, deferred evaluation, helper threads); died of upstream drift |
| Riposte | Vector-trace JIT | 5–50× on vector code; research prototype |
| Ř / rir (PRL-PRG) | Bytecode + optimizing JIT *inside* GNU R, speculation + deoptimization | ~1.7× over GNU R on average, much more on loops; keeps the C API for free |
| Rho/CXXR | C++ refactor of GNU R | Abandoned: too much churn against upstream |
| TERR | Proprietary clean-room engine | Survives as a vendor product with partial C API |
| dgkf/R | Rust tree-walker | Language experiment, deliberately incompatible, ~1000× slower |
| webR | GNU R compiled to WASM | Works because it *is* GNU R |

Lessons:

1. **Compatibility first, then speed.** Peak speed did not save FastR; drift
   killed pqR and Rho. A new runtime must be measured continuously against
   GNU R's actual behaviour.
2. **Reuse R's own R code.** Base, stats and utils are mostly R code on top
   of ~760 `.Primitive`/`.Internal` functions (`src/main/names.c`).
   Reimplementing that layer and running the original R code on top is what
   got FastR and TERR furthest.
3. **The C API decides package compatibility.** Projects that emulated it
   (FastR, Renjin, PyPy's cpyext on the Python side) paid in speed and
   correctness. The way out is to keep object layouts compatible so the C
   API can be implemented directly, and to target only R Core's (shrinking)
   official API, which hides `DATAPTR`, `FRAME` etc. since 2025–26.
4. **Where speed comes from** (in order of payoff per effort):
   - cheap wins in the interpreter: no allocation for scalars, cached
     variable and function lookup, avoiding promises for already-evaluated
     arguments (laziness is rarely relied on: 88% of promises are forced
     in the callee, OOPSLA'19), in-place update of unshared vectors;
   - a *specializing* interpreter (CPython 3.11's PEP 659: ~1.25× without
     a JIT; ORBIT: 3.5× on R loops) — type-specialized instructions with
     inline caches, validated by global epoch counters (Julia's "world
     age", CPython's dict versions);
   - fusing vector expressions (Riposte, pqR task merging, ALTREP-style
     lazy vectors);
   - only then a JIT with speculation and deoptimization (Ř's "contextual
     dispatch": compile per call-context such as "argument is an
     attribute-free scalar double", deoptimize on reflection).

## 2. How we build it: lessons from AI-assisted ports

Recent large ports (the TypeScript 7 rewrites in Rust, oxc-project/tsrs
(type checker) and pingdotgg/ts-rust (compiler), vercel-labs/tsc-ts,
aardappel/goose, t3dotgg/lust-lang)
share a method, which robigo adopts:

- **An oracle and a harness before breadth.** ts-rust's first attempt
  plateaued because breadth came before an authoritative harness. robigo pins
  GNU R 4.5.0 as the oracle and compares byte-for-byte:
  - `tests/parse`: parse trees against GNU R's `parse()`
    (`scripts/oracle-parse.R`); also run over all 834 `.R` files of R's own
    `src/library` (833 match; the remaining file uses Latin-1 byte strings).
  - `tests/r`: conformance scripts whose stdout, stderr and exit status are
    recorded from `Rscript` (`scripts/regen-expected.sh`).
- **A ratchet.** `tests/r/passing.txt` lists scripts that must keep
  passing; a regression fails the build, new passes are added explicitly.
- **The C source is the spec.** Builtins are ports of the C functions they
  replace, organised one module per C file, with verbatim messages
  ("improvements are bugs").
- **One writer per file** when work is parallelised; stub modules are
  pre-registered so parallel workers never edit the same file.
- **Vertical slices with gates**, not "percent ported": parser → printing →
  evaluator → vectors → S3/conditions → base library → packages.

[blaze](https://github.com/indexable-inc/blaze) (per-function Rust
compilation and live patching) adds lessons for the execution tiers and the
harness, several of which robigo follows or plans to:

- **Every tier is gated against the oracle, not just the fast one.** blaze
  runs its whole harness under both Cranelift and LLVM; robigo runs every
  conformance script under the interpreter and the bytecode VM and requires
  identical output (`compiled_matches_interpreted`).
- **Tier up from sampling, not from counters everywhere.** blaze samples
  where the program is every 2 ms and recompiles functions with ≥2% of the
  samples at LLVM -O3 with hot callees inlined, then again with the branch
  counts that tier collected (PGO). An R JIT can choose closures the same
  way, keeping the baseline cheap.
- **Count refusals.** Functions a tier declines are counted with reasons;
  for robigo, the share of operations that fall back to the AST interpreter
  (`Eval`) is the analogous metric.
- **Reduced regressions and fuzzing.** Minimal reproductions of bugs found
  in real code are kept and run one by one ("breakers"); generated programs
  are compared between implementations. For robigo: random scalar/vector loop
  programs, compared between GNU R, the interpreter and the VM.
- **Run the reference's own test suite with counted exclusions** (rustc's
  `tests/ui` for blaze; GNU R's `tests/*.R` with `.Rout.save` for robigo).

## 3. Architecture

```
robigo-syntax   lexer + Pratt parser reproducing gram.y (precedence, newline and
            `else` rules, `->`, `|>` and placeholder rewrites)
robigo-runtime  object model, evaluator, condition system, builtins, printing
  rlib/     GNU R 4.5.0's base-package R sources, evaluated at startup
robigo-cli      `robigo` (Rscript-like runner, REPL) and the conformance runner
```

### 3.1 Object model and memory

- `Value` is a 16-byte enum: `Null`, `Sym`, one variant per vector type
  holding `Rc<Vector<T>>`, `Lang`, `Pairlist`, `Closure`, `Builtin`
  (`&'static`), `Env`, `Promise`, `Dots`, `ExtPtr`.
- **Vector payloads use GNU R's layouts**: logical and integer are `i32`
  with `NA = INT_MIN`; doubles are `f64` with NA as the NaN payload 1954;
  complex is `{re, im}`. This keeps a future C API zero-copy for
  `INTEGER()`, `REAL()`, `LOGICAL()`.
- **Copy-on-modify is reference counting.** Cloning a `Value` bumps a
  count; mutation goes through `Rc::make_mut`, which copies only when the
  object is shared — exactly R's semantics since R 4.0 switched to
  reference counting. Replacement functions take their object by value,
  and the evaluator moves a variable's value out of its binding before
  calling a builtin replacement function, so `x[i] <- v` in a loop updates
  in place.
- **Environments** are `Rc` reference objects with small vector frames that
  switch to a hash index when large. Symbols are interned `u32`s.
- **Not yet solved:** cycles. A closure stored in the environment it
  captures (`f <- function() { g <- function() 1; g }`) leaks. The plan is
  CPython's answer: keep reference counts (R needs them for
  copy-on-modify anyway) and add a cycle collector over all container
  objects (§5, F5).
- The council review (§5) found that this object model cannot carry R's C
  API as it is; §5.3 lists what changes.

### 3.2 Evaluation

A direct port of GNU R's eval.c semantics on R language objects:
promises with forcing state, R's three-pass argument matching (exact,
partial, positional), contexts (`sys.call`, `parent.frame`, `on.exit`,
generic dispatch frames), S3 dispatch (`UseMethod`, `NextMethod`, internal
generics, group generics `Ops`/`Math`/`Summary`), and complex assignment
(`names(x)[2] <- v`) via `*tmp*` as in `applydefine`.

Non-local exits use `Result<_, Flow>` instead of `longjmp`. All of R's
jumps map onto four variants: `Return(value, env)` (also used by exiting
condition handlers and restarts — R's `findcontext(CTXT_FUNCTION, env)`),
`Break(env)`, `Next(env)` and `Error`. Conditions are signaled where they
are raised (calling handlers see the full stack), by porting errors.c's
handler and restart stacks; `tryCatch`, `withCallingHandlers`, `try`,
`warning`, `suppressWarnings` are base R's own R code on top.

### 3.3 The builtin layer and the base library

`builtins/` implements `.Primitive`s and `.Internal`s with the same
primitive/internal split, argument evaluation (builtin vs special) and
visibility rules as names.c. At startup robigo parses and evaluates the 161
files of GNU R's base package (≈22k lines, ~50 ms in a release build);
`ROBIGO_BOOT_VERBOSE=1` lists top-level expressions that still fail because an
internal is missing. That list, plus the conformance suite, drives what to
implement next.

Formatting and printing are ports of format.c (`formatReal`'s
fixed/scientific decision), printutils.c, printvector.c, printarray.c,
print.c and deparse.c, because R users read output and tests compare it.
`scientific()` scales by powers of ten in 80-bit `long double` as GNU R
does on x86_64 (`nmath/ldouble.rs`).

## 4. Performance plan

Measured first, optimised second (a "measured and rejected" ledger like
tsrs's keeps us honest). Planned tiers:

1. **AST interpreter.** Bootstrap, `eval` of arbitrary objects, and
   whatever the compiler hands back (`Op::Eval`); `ROBIGO_COMPILE=0` runs
   everything here.
2. **Interpreter fast paths:** unboxed scalar values; caching of function
   lookups for base functions (`+`, `[`, `if`) validated by a global
   "base binding changed" epoch; skipping promise creation for constants
   (done) and for arguments to builtins; arena-allocated frames for
   closures whose environment never escapes.
3. **Specializing bytecode** (the default). Compile closures to a register bytecode;
   quicken instructions to type-specialized forms (`add_real_scalar`,
   `index_int_vector`) with inline caches and guards; deoptimize to the AST
   interpreter on reflection (`sys.function`, `environment()`,
   `eval(parent.frame())`), as Ř does.
4. **JIT** (Cranelift) for hot specialized code, with contextual dispatch
   (hot loops today).
5. **Vector fusion** for chains of vectorised arithmetic, exposed through
   ALTREP-like lazy vectors so it composes with the C API.

### 4.1 Types from ry

[ry](https://github.com/sims1253/ry) statically infers types for R code and
exports them (`ry dump-facts`, versioned JSON keyed by source spans). robigo
parses with exact source spans, so facts can be joined to call sites. A
type checker cannot be trusted blindly in R (redefinable `+`, `eval`,
`assign`, active bindings), so facts become **guarded hints**:

- pre-select specialized versions at function entry (one guard per call
  instead of per operation), à la contextual dispatch;
- elide promises for parameters a function always forces;
- pick unboxed representations for loop variables and locals proven to be
  attribute-free scalars (`for (i in seq_len(n))`).

The empirical work on typing R (Turcotte, Goel, Křikava, Vitek: OOPSLA'20;
and the newer PRL-PRG `typr` project) says which facts pay off:
**attribute-free / not an object** (gates all dispatch), **scalar of an
exact mode** (`dbl1`, not "coercible to double"), **already evaluated**,
and **NA-free** (mostly for integer/logical). Today ry's facts carry mode,
length, class and data-frame columns but no NA-ness or attribute
information beyond class. Adding `na: free|maybe`, `attrs:
none|names|dim|unknown` and exact-vs-coercible mode to `dump-facts` would
make them directly usable by a runtime. Prior type-annotation efforts for R
(typed, rtype, interface, contractr, Vapour, TypR) were either runtime
checkers or transpiled languages; only the PRL-PRG line and ry infer types
for unmodified R code, which is what a runtime needs.

### 4.2 Where performance stands

`benchmarks/run.py` times robigo against `Rscript` (GNU R 4.5.0 with its
bytecode JIT, `R_DEFAULT_PACKAGES=utils,stats`) on a 2-vCPU QEMU VM; best of
three wall times in seconds, startup included (GNU R 0.086 s, robigo
0.073 s with these packages). Ratios below 1 mean robigo is faster.

| script | GNU R | robigo | ratio | what it stresses |
|---|---|---|---|---|
| spectralnorm | 0.42 | 0.08 | 0.19 | a tiny closure called in a double loop (JIT, inlined) |
| nbody | 0.40 | 0.10 | 0.26 | scalar arithmetic on short vectors (JIT) |
| apply | 0.40 | 0.19 | 0.48 | `lapply`/`vapply`/`sapply` with closures |
| mandelbrot | 0.18 | 0.09 | 0.49 | scalar `while` loops (JIT) |
| strings | 0.34 | 0.19 | 0.56 | `paste0`, `gsub`, `sprintf`, `nchar` |
| dataframe | 0.25 | 0.15 | 0.61 | `data.frame` building, `aggregate`, `merge` |
| fannkuch | 0.37 | 0.23 | 0.63 | integer vectors, `x[a:b] <- x[b:a]` |
| matrix | 0.20 | 0.13 | 0.66 | `%*%`, `solve`, `apply` over rows |
| vecarith | 0.95 | 0.72 | 0.76 | elementwise arithmetic, `sum`, `mean` on 10^6 |
| queens | 0.13 | 0.10 | 0.77 | recursion with vectors |
| lm | 0.22 | 0.18 | 0.80 | `lm`, `summary.lm` |
| vecops | 0.65 | 0.52 | 0.80 | `x[x > 0]`, `ifelse`, `cumsum`, `which`, `order` |
| csv | 0.30 | 0.26 | 0.86 | `write.csv`, `read.csv` |
| fib | 0.21 | 0.24 | 1.14 | closure calls, promises |
| binarytrees | 0.27 | 0.30 | 1.14 | recursion, list allocation, `[[` |

Most times include ~0.07 s of startup (both runtimes load `utils` and
`stats`); the JIT'd loops run in a few milliseconds.

`scripts/idiom-vs-r.py 'SETUP' 'BODY' N` compares the instructions per
iteration of an idiom with GNU R's. `scripts/callgrind-diff.py WORK.R BASE.R UNITS` measures with callgrind:
instruction counts are exact, so a change can be judged without
wall-clock noise (the VM shares two cores with other work). A fib call costs GNU R ~2,900
instructions and robigo ~3,700. Per call of typical base-R code (callgrind,
VM, `R_DEFAULT_PACKAGES=NULL`):

| idiom | GNU R | robigo |
|---|---|---|
| `match.arg(type)` with a choice given | 68.1k | 67.8k |
| `match.arg(type)` with the default | 43.1k | 57.3k |
| `stopifnot(is.numeric(x), length(x) == 1)` | 25.7k | 35.7k |
| `tryCatch(i, error = function(e) NULL)` | 42.3k | 48.2k |
| S3 dispatch (`UseMethod`, `s$side^2`) | 16.6k | 17.6k |
| `seq(1, 10, by = 2)` | 106k | 131k |
| `vapply(xs, function(v) ..., 0)`, 20 elements | 118k | 48k |
| `m[i, j] <- v` in a loop | 3.3k | 1.8k |
| `l$a <- l$a + 1` | 1.5k | 1.4k |
| `r[i] <- i` growing `r` | 2.0k | 0.5k |

Ledger:

- **Done (calls):** `Op::Call` calls closures directly; argument
  promises carry bytecode (as GNU R's compiler does); the function is
  memoized per call site; guards memoize per environment and per
  search-path environment and are skipped when they held since the last
  point where R code could run; environments keep up to four bindings
  inline (one allocation); closures cache their bytecode; VM registers live
  on the Rust stack; a mimalloc allocator without the aligned path.
- **Done (loops):** unboxed scalar registers; variables cached in slots;
  `x[i]`/`x[i] <- v` and `x[a:b]`/`x[a:b] <- v` on plain vectors in place;
  `a:b` as an unmaterialized range; arithmetic and comparisons with
  constant/variable operands read in place and guards folded in.
- **Done (vectors):** shape-specialized kernels for arithmetic and
  comparisons that LLVM vectorizes, reusing unshared operands; `sum`,
  `mean`, `cumsum`, `prod` in x87 assembly (bit-identical long double
  accumulation); LSD radix `order`.
- **Done (startup):** 320M -> 166M instructions: byte-level lexer, no token
  cloning in the parser, Fx-hashed symbol table, lazy-DFA regexes with
  ASCII-restricted classes for ASCII text, a cache of compiled patterns.
- **Done (inlining):** calls of pure closures (arithmetic, comparisons,
  `if`, no assignments or calls) with constant or variable arguments run
  their body without a call, giving up before any slow path so warnings
  and errors come from a real call; single-operation promises (`n - 1`)
  are evaluated without a VM run.
- **Done (JIT, `src/jit.rs`):** loops whose ops all have scalar fast
  paths (arithmetic, comparisons, `&&`/`||`, `if`, `for` over ranges,
  double/integer vector element reads and writes, unboxed math, calls of
  pure closures without free variables) are compiled by Cranelift once
  they have iterated 16 times. Variables and registers are typed by a
  forward analysis (registers per op) and live in machine registers;
  guards are checked once on entry. Wherever a fast path would give up
  (overflow, NA condition, index out of range, NaN from a math function),
  native code writes its state back and the VM executes that op, so
  results, warnings and errors are the VM's. `ROBIGO_JIT=0` disables it,
  `ROBIGO_JIT_TRACE=1` reports compilations and exits.
- **Done (general code):** `lapply`/`vapply` run pure `FUN`s without a
  call per element; guards are revalidated only when function bindings
  change; cached scalars survive flushes until R code runs; `TRUE`,
  `FALSE`, `NA` and small integers are shared constants; `.Internal(f(..))`
  compiles to a direct builtin call.
- **Done (base-R functions):** registers are allocated as a stack and
  freed after each call expression (`stopifnot` needs 21 instead of 100),
  so registers, variable slots and guard state stay on the Rust stack;
  constant default arguments are bound without a promise and other
  defaults carry bytecode; formals are bound without frame lookups; frames
  keep a 64-bit mask of their symbols, so most failed lookups cost one
  test; `x[i]`/`x[[i]]` with any index compile to VM ops (dispatching
  with a promise for objects); one-argument builtins (`length`,
  `is.null`, `is.na`, `typeof`, ...) and scalar string `==` run without a
  call; primitives are found by hash and S3 method names built without
  `format!`.
- **Done (vector builtins, found by a sweep of ~80 base operations against
  GNU R):** `duplicated`/`unique`/`match` use open-addressing tables of
  element indices; integer arithmetic has shape-specialized kernels;
  `x[length(x) + 1] <- v` appends in place (4x fewer instructions than GNU
  R); `switch()` compiles to a jump table. Several of these were also
  conformance bugs, now exact ports: `scientific()` (long double scaling,
  with R's `tbl[]` of double literals and glibc's `powl`), `colSums` and
  friends (x87 long double instead of a double-double), `%%`/`%/%` on
  doubles (long double remainder, `c_eps = LDBL_EPSILON`, the "loss of
  accuracy" warning), and `switch()` on `NA`. `.Internal(merge)` was
  missing.
- **Done (VM coverage):** `m[i, j]`, `m[i, j] <- v`, `x$name`,
  `x$name <- v`, `missing()`, `switch()` and calls to special primitives
  run in the VM rather than falling back to the interpreter; guards of
  closures defined in calls share memos; a local helper function does not
  invalidate a body's guards. `--features op-stats` counts executed ops by
  kind.
- **Done (statistics):** `cov`/`cor`/`var` run their long double sums as
  x87 loops; `anyDuplicated` uses the index tables; logical-mask and
  matrix subsetting gather directly; recycled arithmetic avoids
  per-element division; strings of small integers (row names) are shared.
  `lm` on 20k rows takes 39.5M instructions (was 60.8M), `glm` 174M (was
  211M). Many stats natives are still missing (`optim`, `nlm`,
  `integrate`, splines, `fft`, `loess`, `nls`, `kmeans`, `hclust`, the
  LAPACK internals behind `svd`/`eigen`).
- **Done (VM by default, G1):** the VM runs closures and loops unless
  `ROBIGO_COMPILE=0`; startup costs the same either way (151M
  instructions, 274M with `utils` and `stats`). The share of VM ops that
  fall back to the interpreter (`Op::Eval`, counted by `--features
  op-stats`) is 0.01–0.7% on the benchmark programs: each runs ~800, the
  same in all 15, from loading `utils` and `stats`.
- **Next:** simple closure calls now cost what GNU R's do, but recursive
  ones (fib) still ~25% more: each allocates an environment, a promise
  per argument, the argument vector and a boxed result, and entering the
  VM costs ~85 instructions before the first op. Candidates: forced promises built by the caller for
  arguments that cannot fail or run code (`n - 1` on numbers), so the
  callee's force is a read; unboxed results for compiled callers.

## 5. Council review and goalposts

On 2026-10-26 six models reviewed the design read-only (one round) against
the project's goals: (1) an interactive REPL and scripts, (2) loading any
package GNU R can load, compiled code included, (3) better interop with
other languages, (4) the fastest R implementation, (5) two numeric modes,
exact (bit-identical to GNU R) and fast, (6) WebAssembly.

| Juror | Score |
|---|---|
| Fable 5.1 | 6/10 |
| GPT-6 Astra | 6.5/10 |
| Opus 5.5 | 6/10 |
| GLM-5.3 | 7/10 |
| Gemini 4 Argon | 7.5/10 |
| Kimi K3.1 Pro | 7/10 |

The verdict was unanimous in direction: the method and the execution
engine are right; the object model was built for a Rust interpreter, not
for R's C API, and has to change before more builtins depend on it.

### 5.1 Keep

- **The oracle.** Byte-for-byte conformance, the ratchet, every tier run
  against every script. GPT-6 Astra found the gate enforced only stdout;
  since then it fails on any stdout, stderr or exit-status difference,
  compared as bytes, under the interpreter, the VM and the VM with the JIT.
- **Porting GNU R's C sources** and running base R's R code unmodified,
  under GPL-2.0-or-later. A clean-room implementation would have to rewrite
  base, stats and utils too, and would lose the oracle. The rule the jurors
  added: *port behaviour and messages verbatim, not data structures.*
- **Rust.** Go's GC and cgo costs and the JVM's warmup and C-API emulation
  rule those out for a pointer-heavy C API; C and C++ bring GNU R's memory
  safety profile; Zig is young. Keep `unsafe` in one audited boundary.
- **The VM. It is not a crutch.** On WebAssembly, where an in-process JIT
  is not possible, it is the fast tier. A JIT needs a deoptimization target
  that implements R's reflection, and cold base-R code would pay compile
  latency. Tiers: AST interpreter (bootstrap, `eval` of arbitrary objects,
  deopt) → register VM by default → Cranelift for whole hot closures chosen
  by sampling, with contextual dispatch and per-op deopt to the VM.
- GNU R's payload layouts, `Result<_, Flow>`, guard epochs and the
  measured ledger.

### 5.2 Wrong today

Each item was reproduced against GNU R 4.5.0:

| Problem | Evidence |
|---|---|
| Closure and environment cycles leak | 200k calls of `f <- function(i) { g <- function() i; g }`: 32 → 120 MB (GNU R 39 MB); 200k `tryCatch` calls: 489 MB (GNU R 71 MB) |
| No compact sequences outside VM registers | `x <- 1:2e8; x[5L]`: 786 MB peak (GNU R 54 MB) |
| Strings must be valid UTF-8 | `rawToChar(as.raw(c(0x61, 0xff)))` is an error; GNU R returns a 2-byte string |
| No S4 | `library(methods)` fails in `R_initMethodDispatch` |
| `dyn.load` loads no code | `.Call` dispatches to hand-written Rust ports by name (`native/mod.rs`) |
| Exact numerics on x86_64 only | `nmath/ldouble.rs` falls back to `f64` elsewhere, silently |
| The fast engine is opt-in | the default AST interpreter is 5–15x slower than GNU R on loops. *Fixed (G1): the VM is the default.* |
| The interpreter's recursion limit | `f(4000)` (simple recursion) is "evaluation nested too deeply"; the VM and GNU R run it. *Fixed for compiled closures (G1): the VM nests 4,999 calls, GNU R 4,996, the interpreter 2,498, like GNU R's own AST evaluator.* |
| `reg.finalizer` does nothing, `gc()` returns `NA` | placeholders that guess |
| Not self-contained | `utils` and `stats` load from an installed GNU R (`R_HOME`) |
| The bytecode cache never evicts | `vm.rs` `CACHE` keeps every compiled expression |
| The REPL is a bare `read_line` loop | no editing, history or interrupts; a Rust panic ends the session |

### 5.3 Fundamental: redesign before more builtins depend on it

The cost of each grows with every builtin: at review time there were 1,233
direct `.data` accesses in 58 files, 169 `RStr::as_deref` sites and ~240
uses of `Pairlist::items` and `Lang::args`.

- **F1. Values have no object header.** `Value` is a 16-byte enum with the
  type in the tag; a `SEXP` is one pointer to an object that carries its
  type, flags (object, S4, encoding), attributes and length. A handle table
  in between is the cost FastR and PyPy's cpyext paid on every C call.
  Target: header'd heap objects, `Value` a thin pointer, typed Rust views
  (`RVec<f64>` derefs to a slice) for builtins.
- **F2. Vector storage is a `Vec<T>`.** It cannot hold a foreign buffer
  (Arrow, NumPy, mmap), an ALTREP object or a compact range, and C cannot
  write through `REAL(x)` while Rust holds a reference. Target: storage
  variants `Owned | Compact | Foreign(ptr, len, release) | Altrep` behind
  `data()`/`data_mut()`/`elt()`/`get_region()`, materializing on `DATAPTR`,
  with explicit write-access rules for C.
- **F3. Strings are not CHARSXPs.** `Option<Rc<str>>` is UTF-8 only, not
  NUL-terminated, not interned and 16 bytes per element. Packages compare
  `CHARSXP` pointers and use `TRUELENGTH` as hash scratch. Target: an
  interned cache of NUL-terminated byte strings with encoding bits (UTF-8,
  Latin-1, bytes, ASCII) and a cached hash; pointer-sized `STRSXP`
  elements.
- **F4. Pairlists and calls are vectors.** Rcpp, rlang and cpp11 walk and
  mutate cells with `CDR`/`SETCAR`/`SETCDR` and rely on cell identity.
  Target: cons cells; compiled call sites keep a flattened argument array.
- **F5. Reference counting without a cycle collector.** Target: keep the
  counts (they are R's copy-on-modify), add trial deletion over all
  container objects (environments, closures, promises, lists, pairlists,
  external pointers). Its root registry is the `PROTECT` stack and
  `R_PreserveObject`; finalizers and weak references build on it.
- **F6. No native ABI.** Target: `dlopen`, `Rinternals.h` implemented over
  F1–F5, and trampolines between `Flow` and C's non-local exits
  (`Rf_error`, `R_UnwindProtect`, C++ exceptions), nested C → R → C
  included.
- **F7. No numeric policy.** Exactness is spread over nine files of x87
  code with no seam, and the fast mode does not exist. Target: one module
  for every operation whose result depends on the mode (reductions,
  `cumsum`, `colSums`, `var`/`cov`, `%%`, `%*%`, `scientific()` scaling,
  the long-double nmath sites). *Exact* is x87 on x86_64 and software
  80-bit elsewhere, never a silent `f64` fallback; *fast* is `f64` with
  SIMD and reassociation. The mode is a runtime setting and part of the
  compiled-code cache key, not a Cargo feature. Exact mode is gated byte
  for byte, fast mode with a tolerance oracle (`all.equal`).
- **F8. Not portable or embeddable.** Cranelift and the x87 assembly are
  unconditional dependencies, so there is no wasm32 build; 25
  `thread_local!`s make one runtime per thread. Target: both behind
  features, and runtime state owned by the interpreter instance.

### 5.4 Hill-climbable

VM by default with the `Eval` share as a metric; closure-call cost
(caller-forced promises, unboxed returns, pooled frames and promises,
VM-to-VM calls without Rust recursion); a whole-closure JIT; base-R idioms
(`match.arg` with the default, `stopifnot`, `seq`, S3 dispatch
allocations); missing stats natives; the REPL; a startup snapshot; code
cache eviction.

### 5.5 Where the jurors disagreed

- **Handles or thin pointers.** Fable 5.1 and GLM-5.3 proposed a handle or
  node layer next to today's `Value`; Opus 5.5, Gemini 4 Argon and Kimi
  K3.1 Pro the value itself as a header'd object; GPT-6 Astra stable
  handles over a non-moving traced heap. **Decision:** thin pointers to
  header'd objects, because the C API is the long-term interface and must
  be direct. The accessor layer (G2) keeps this cheap to revisit, and the
  C API spike (G5) measures it.
- **Cycle collector or tracing heap.** GPT-6 Astra argued for tracing with
  separate sharing metadata. **Decision:** reference counts with trial
  deletion over all container objects; counts are R's own copy-on-modify
  semantics, and trial deletion finds every cycle among containers.
- **Global state.** GLM-5.3 and Gemini 4 Argon call `thread_local!` state
  fundamental; Opus 5.5 finds single-threaded values fine, as in GNU R.
  **Decision:** values stay single-threaded; global state moves into the
  instance (F8).
- **Order.** Fable 5.1: VM default first. GLM-5.3 and Kimi K3.1 Pro: a C API
  prototype first. Opus 5.5 and Gemini 4 Argon: accessors and storage
  first. GPT-6 Astra: the gate first (done). **Decision:** below.

### 5.6 Goalposts

In order; the conformance gate stays byte-identical in every tier
throughout, and the benchmark and idiom tables (§4.2) must not regress by
more than 5%.

| | Goalpost | Exit gate |
|---|---|---|
| G0 | Gate on stdout, stderr and exact status, as bytes, in all tiers | done |
| G1 | VM by default; the AST interpreter for bootstrap, `eval` and deopt; `Eval` share tracked | done |
| G2 | Accessor layer: `.data`, `RStr`, `items`, `args` behind methods; CI rejects new direct access | no field access outside `value.rs` |
| G3 | Object model (F1–F4): header'd thin objects with an S4 bit, storage variants with compact ranges as values, interned CHARSXPs, cons cells | `1:2e8` in constant memory; byte strings round-trip |
| G4 | Cycle collector and root registry (F5); real `reg.finalizer`, `gc()`, weak references | leak tests in the suite: bounded memory for closure-defining and `tryCatch` loops |
| G5 | C API spike (F6): `robigo-capi`, `dlopen`, `PROTECT`, unwind trampolines | three real packages load and pass their tests: a plain `.Call` package, an Rcpp package, rlang; packages passing becomes a ratchet |
| G6 | Numeric policy and portability (F7, F8) | wasm32 (JIT off) and aarch64 pass the suite byte for byte in CI; fast mode passes a tolerance suite |
| G7 | `methods`/S4, default packages without `R_HOME`, a startup snapshot | `library(methods)`, `setClass`, `new` work; robigo runs without GNU R installed |
| G8 | Whole-closure JIT by sampling; call cost | fib and binarytrees at or below GNU R; `match.arg`, `stopifnot`, `seq` ≤ 1.0x GNU R's instructions |
| G9 | REPL: line editing, history, interrupts; panics become R errors | an interactive session survives errors and Ctrl-C |

G1 comes first because it is small and every user sees the fast tier.
G2–G5 are one track: the object model exists to make the C API direct.
Until G2 lands, new builtins and stats ports wait: each one adds field
accesses to migrate. Numeric ports that accumulate in long double (LAPACK,
`optim`, `loess`, ...) wait for the seam in G6. Afterwards: GNU R's own
`tests/*.R`, ry facts for guarded specialization (§4.1), vector fusion
over ALTREP-style lazy vectors.

## 6. Decisions to revisit

- **License.** robigo ports GNU R's C sources and vendors base R's R code, so it
  is GPL-2.0-or-later like R. A permissively licensed runtime would have to
  be a clean-room design and could not reuse base R's R code.
- **Locale.** robigo implements the C.UTF-8 locale (byte-order collation,
  English messages); the oracle runs with `LC_ALL=C.UTF-8`.
- **Strings** are always valid UTF-8 (`Rc<str>`); R's Latin-1/bytes
  encodings are not supported yet. Interned CHARSXPs with encodings
  replace them (§5.3, F3).
