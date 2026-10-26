# rr: design

rr is an attempt at a faster, modern runtime for the R language, written in
Rust. This document records what we learned from earlier attempts (in R,
Python, Julia and recent AI-assisted compiler ports), the architecture that
follows from it, and the roadmap.

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
share a method, which rr adopts:

- **An oracle and a harness before breadth.** ts-rust's first attempt
  plateaued because breadth came before an authoritative harness. rr pins
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
harness, several of which rr follows or plans to:

- **Every tier is gated against the oracle, not just the fast one.** blaze
  runs its whole harness under both Cranelift and LLVM; rr runs every
  conformance script under the interpreter and the bytecode VM and requires
  identical output (`compiled_matches_interpreted`).
- **Tier up from sampling, not from counters everywhere.** blaze samples
  where the program is every 2 ms and recompiles functions with ≥2% of the
  samples at LLVM -O3 with hot callees inlined, then again with the branch
  counts that tier collected (PGO). An R JIT can choose closures the same
  way, keeping the baseline cheap.
- **Count refusals.** Functions a tier declines are counted with reasons;
  for rr, the share of operations that fall back to the AST interpreter
  (`Eval`) is the analogous metric.
- **Reduced regressions and fuzzing.** Minimal reproductions of bugs found
  in real code are kept and run one by one ("breakers"); generated programs
  are compared between implementations. For rr: random scalar/vector loop
  programs, compared between GNU R, the interpreter and the VM.
- **Run the reference's own test suite with counted exclusions** (rustc's
  `tests/ui` for blaze; GNU R's `tests/*.R` with `.Rout.save` for rr).

## 3. Architecture

```
rr-syntax   lexer + Pratt parser reproducing gram.y (precedence, newline and
            `else` rules, `->`, `|>` and placeholder rewrites)
rr-runtime  object model, evaluator, condition system, builtins, printing
  rlib/     GNU R 4.5.0's base-package R sources, evaluated at startup
rr-cli      `rr` (Rscript-like runner, REPL) and the conformance runner
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
visibility rules as names.c. At startup rr parses and evaluates the 161
files of GNU R's base package (≈22k lines, ~50 ms in a release build);
`RR_BOOT_VERBOSE=1` lists top-level expressions that still fail because an
internal is missing. That list, plus the conformance suite, drives what to
implement next.

Formatting and printing are ports of format.c (`formatReal`'s
fixed/scientific decision), printutils.c, printvector.c, printarray.c,
print.c and deparse.c, because R users read output and tests compare it.
One deliberate deviation: GNU R scales by powers of ten in 80-bit
`long double`; rr uses Rust's exact decimal conversion, which agrees except
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
exports them (`ry dump-facts`, versioned JSON keyed by source spans). rr
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

`benchmarks/run.py` times rr against `Rscript` (GNU R 4.5.0 with its
bytecode JIT) on a 2-vCPU QEMU VM; best of three wall times in seconds,
including ~0.03 s startup for both:

| script | GNU R | rr | rr VM | what it stresses |
|---|---|---|---|---|
| fib | 0.11 | 0.21 | 0.20 | closure calls, promises |
| loop | 0.12 | 1.08 | 0.24 | scalar arithmetic in a top-level `for` |
| loopf | 0.12 | 1.13 | 0.23 | the same inside a function |
| vecassign | 0.09 | 0.62 | 0.10 | `x[i] <- v` in a loop |
| strings | 0.19 | 0.15 | 0.15 | `paste0`, `gsub`, `unique`, `nchar` |
| apply | 0.18 | 0.26 | 0.27 | `lapply`/`vapply` with closures, `%*%` |
| sortbench | 0.60 | 0.43 | 0.43 | `order`, `sort` on 10^6 doubles and strings |

Ledger:

- **Done:** function lookup cache keyed by a search-path epoch; unboxed
  scalar registers; in-place scalar updates; variables cached in VM slots
  and written back before anything that can observe the frame (1.6× on
  scalar loops); `while`/`for` loops with the test at the bottom and no
  visibility bookkeeping for discarded values; `x[i]`, `x[[i]]` and
  `x[i] <- v` with a variable `x` and a constant or variable index read
  and update plain vectors directly (7× on `vecassign`), falling back to
  the interpreter for objects, attributes that matter, type changes and
  out-of-range indices.
- **Measured, not yet fixed:** each VM instruction costs 5–10 ns on this
  machine (two indirect branches: dispatch and `RVal` cloning), so a scalar
  loop iteration of 6–7 instructions is ~2× GNU R's. Next: fuse
  `GetVar`/constant operands into arithmetic and comparison instructions
  and fold guard checks into the instructions they protect.
- **Not compiled yet:** closure calls still allocate an environment and
  promises per call (`fib` is ~1.8× slower); matrix indexing `m[i, j]`
  and computed indices `x[i + 1]` go through the interpreter.

## 5. Roadmap

1. Base language and base library (in progress): finish the internals that
   base R needs; grow the conformance suite; then run GNU R's own
   `tests/*.R` with their `.Rout.save` files.
2. Default packages: load `utils`, `stats`, `methods` namespaces from R
   sources (namespace loading, S4), `datasets` via `unserialize`.
3. Performance tiers 2–3 with a benchmark suite (`benchmarks/`) compared
   against GNU R with its bytecode JIT enabled.
4. C API: `Rinternals.h` implemented directly over rr's object layout
   (handles for SEXP, PROTECT as a root stack, ALTREP hooks), starting with
   the API surface used by the most popular CRAN packages.
5. ry integration: consume `dump-facts` for guarded specialization;
   contribute the extra facts runtimes need.

## 6. Decisions to revisit

- **License.** rr ports GNU R's C sources and vendors base R's R code, so it
  is GPL-2.0-or-later like R. A permissively licensed runtime would have to
  be a clean-room design and could not reuse base R's R code.
- **Locale.** rr implements the C.UTF-8 locale (byte-order collation,
  English messages); the oracle runs with `LC_ALL=C.UTF-8`.
- **Strings** are always valid UTF-8 (`Rc<str>`); R's Latin-1/bytes
  encodings are not supported yet.
