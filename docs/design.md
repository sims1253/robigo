# robigo: design

robigo is an attempt at a faster, modern runtime for the R language, written in
Rust. This document records what we learned from earlier attempts (in R,
Python, Julia and recent AI-assisted compiler ports), the architecture that
follows from it, and the roadmap.

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

Recent large ports (Cloudflare/VoidZero's TypeScript→Rust `tsrs`,
pingdotgg/ts-rust, vercel-labs/tsc-ts, aardappel/goose, t3dotgg/lust-lang)
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
  copy-on-modify anyway) and add a cycle collector over environments and
  closures.

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
One deliberate deviation: GNU R scales by powers of ten in 80-bit
`long double`; robigo uses Rust's exact decimal conversion, which agrees except
in pathological double-rounding cases.

## 4. Performance plan

Measured first, optimised second (a "measured and rejected" ledger like
tsrs's keeps us honest). Planned tiers:

1. **AST interpreter** (now). Correctness reference for all reflective
   features.
2. **Interpreter fast paths:** unboxed scalar values; caching of function
   lookups for base functions (`+`, `[`, `if`) validated by a global
   "base binding changed" epoch; skipping promise creation for constants
   (done) and for arguments to builtins; arena-allocated frames for
   closures whose environment never escapes.
3. **Specializing bytecode.** Compile closures to a register bytecode;
   quicken instructions to type-specialized forms (`add_real_scalar`,
   `index_int_vector`) with inline caches and guards; deoptimize to the AST
   interpreter on reflection (`sys.function`, `environment()`,
   `eval(parent.frame())`), as Ř does.
4. **JIT** (Cranelift) for hot specialized code, with contextual dispatch.
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
| spectralnorm | 0.49 | 0.09 | 0.19 | a tiny closure called in a double loop (JIT, inlined) |
| nbody | 0.42 | 0.11 | 0.27 | scalar arithmetic on short vectors (JIT) |
| mandelbrot | 0.18 | 0.09 | 0.51 | scalar `while` loops (JIT) |
| apply | 0.39 | 0.21 | 0.53 | `lapply`/`vapply`/`sapply` with closures |
| strings | 0.37 | 0.21 | 0.57 | `paste0`, `gsub`, `sprintf`, `nchar` |
| fannkuch | 0.37 | 0.23 | 0.60 | integer vectors, `x[a:b] <- x[b:a]` |
| dataframe | 0.26 | 0.17 | 0.66 | `data.frame` building, `aggregate`, `merge` |
| matrix | 0.21 | 0.14 | 0.70 | `%*%`, `solve`, `apply` over rows |
| vecarith | 1.03 | 0.77 | 0.75 | elementwise arithmetic, `sum`, `mean` on 10^6 |
| vecops | 0.71 | 0.58 | 0.81 | `x[x > 0]`, `ifelse`, `cumsum`, `which`, `order` |
| queens | 0.13 | 0.11 | 0.86 | recursion with vectors |
| lm | 0.23 | 0.20 | 0.87 | `lm`, `summary.lm` |
| csv | 0.30 | 0.28 | 0.95 | `write.csv`, `read.csv` |
| fib | 0.22 | 0.27 | 1.23 | closure calls, promises |
| binarytrees | 0.26 | 0.34 | 1.31 | recursion, list allocation, `[[` |

Most times include ~0.07 s of startup (both runtimes load `utils` and
`stats`); the JIT'd loops run in a few milliseconds.

`scripts/callgrind-diff.py WORK.R BASE.R UNITS` measures with callgrind:
instruction counts are exact, so a change can be judged without
wall-clock noise (the VM shares two cores with other work). A fib call costs GNU R ~2,900
instructions and robigo ~4,300.

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
- **Next:** typical base-R functions (`match.arg`, `stopifnot`,
  `tryCatch`, `format`, S3 dispatch) are ~2x GNU R: every closure call
  costs an environment, a context, a promise per argument and a VM run
  (~2-4k instructions), and each VM op ~30-70 instructions. Candidates: a
  cheaper calling convention (frames without heap environments until
  observed, promise elision for arguments forced on entry), and native
  code for whole function bodies, not only loops.

## 5. Roadmap

1. Base language and base library (in progress): finish the internals that
   base R needs; grow the conformance suite; then run GNU R's own
   `tests/*.R` with their `.Rout.save` files.
2. Default packages: load `utils`, `stats`, `methods` namespaces from R
   sources (namespace loading, S4), `datasets` via `unserialize`.
3. Performance tiers 2–3 with a benchmark suite (`benchmarks/`) compared
   against GNU R with its bytecode JIT enabled.
4. C API: `Rinternals.h` implemented directly over robigo's object layout
   (handles for SEXP, PROTECT as a root stack, ALTREP hooks), starting with
   the API surface used by the most popular CRAN packages.
5. ry integration: consume `dump-facts` for guarded specialization;
   contribute the extra facts runtimes need.

## 6. Decisions to revisit

- **License.** robigo ports GNU R's C sources and vendors base R's R code, so it
  is GPL-2.0-or-later like R. A permissively licensed runtime would have to
  be a clean-room design and could not reuse base R's R code.
- **Locale.** robigo implements the C.UTF-8 locale (byte-order collation,
  English messages); the oracle runs with `LC_ALL=C.UTF-8`.
- **Strings** are always valid UTF-8 (`Rc<str>`); R's Latin-1/bytes
  encodings are not supported yet.
