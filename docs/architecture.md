# robigo: architecture

The target architecture, what exists today, and the boundaries along which
work is split. `docs/design.md` records how we got here (lessons from
other R implementations, the council review); `docs/verification.md` how
everything is checked; `docs/workstreams.md` who works on what.

## Principles

1. **GNU R 4.5.0 is the oracle.** Same output, errors, warnings and
   numbers, byte for byte. Its C sources are the spec.
2. **Anything that can be verified is solved.** Every part of the system
   has a metric that a machine checks (`docs/verification.md`). Work
   without a metric starts by building one.
3. **Performance wins; effort is free.** Effort, churn and backwards
   compatibility are never arguments. If a design is faster for more work,
   it wins. When two designs are close, build both and measure.
4. **One owner per area.** Crates and directories have owners
   (`docs/workstreams.md`), so parallel work does not collide.

## The shape of the system

```text
                       robigo-cli (binary, REPL, -f/-e, harness)
                                     │
        ┌────────────────────────────┼─────────────────────────────┐
        │                     robigo-runtime                        │
        │  evaluator · contexts · conditions · S3/S4 dispatch       │
        │  builtins (.Primitive/.Internal) · VM · Cranelift JIT     │
        └──────┬──────────────────┬────────────────┬───────────────┘
               │                  │                │
        robigo-capi          robigo-core       robigo-syntax
        libR.so ABI    ───►  objects, memory,   lexer, parser
        (dlopen, Rf_*,       strings, symbols,
         PROTECT, unwind)    envs, ALTREP
               │                  │
               │            robigo-numerics      robigo-text (planned)
               │            nmath, long double,  number formatting,
               │            numeric policy       sprintf, regex, encodings
               ▼
   package .so files: GNU R's own (stats, utils, methods, grDevices,
   graphics, tools) and CRAN binaries, loaded unmodified
```

Dependencies point down only. `robigo-numerics`, `robigo-text` and
`robigo-syntax` know nothing about R objects: they work on `f64`, `&[u8]`,
`&str` and plain Rust types, so they can be developed, tested and
optimized on their own.

| crate | status | contents |
|---|---|---|
| `robigo-syntax` | exists | lexer and parser reproducing gram.y; checked against GNU R's `parse()` |
| `robigo-numerics` | exists | nmath (d/p/q/r, special functions, RNG), x87 long double, `%%`/`%/%` |
| `robigo-runtime` | exists, to be rebuilt on `robigo-core` | object model (today: `value.rs`), evaluator, builtins, VM, JIT |
| `robigo-cli` | exists | the `robigo` binary, REPL, conformance tests |
| `robigo-core` | planned (G2) | the object model and memory manager below |
| `robigo-capi` | planned (G2) | `libR.so`: GNU R 4.5's C API over `robigo-core` |
| `robigo-text` | planned | formatting and string algorithms, from `robigo-runtime` |

## Native code: robigo is a `libR.so`

Package binaries built for GNU R 4.5 reach R objects only through
exported functions and a few global pointers (`INTEGER`, `STRING_ELT`,
`Rf_allocVector`, `R_NilValue`, ...): `fastmap.so` imports 27 such
symbols, `Rcpp.so` 73, `vctrs.so` 153, and GNU R's own `stats.so` 293.
So robigo exports those symbols from a `libR.so` over its own objects and
loads the binaries users already have, unmodified:

- **CRAN packages** load from their installed binaries (goal: any package
  GNU R can load).
- **GNU R's own base packages' C and Fortran code** (`stats.so`,
  `utils.so`, `methods.so`, `grDevices.so`, `graphics.so`, `tools.so`)
  loads the same way, so `optim`, `fft`, `loess`, LAPACK calls, S4 dispatch
  helpers and graphics are GNU R's own code, bit-identical by construction,
  instead of ports. For a robigo that runs without GNU R installed, the
  same C sources are compiled against robigo's headers.
- **Rust ports of native routines are overrides**, kept only where they
  are faster and produce identical results (checked by the corpora and
  `scripts/perf.py`).
- What must be ported is libR itself: `src/main` (evaluator, builtins,
  printing, ...), `src/nmath` (done, `robigo-numerics`), `src/appl`
  (LINPACK and friends: ported or compiled), and the API surface in
  `src/include/R_ext`.

The ABI fixes one thing about the object model: a `SEXP` is one pointer
to an object whose type, attributes and length the API functions can
reach. Everything else about the layout is ours.

## The core object model (`robigo-core`, G2)

Every R value is a heap object that starts with a header; a `Sexp` is a
non-null pointer to it (8 bytes), the same value C code sees.

```text
header (16 bytes)   type u8 (GNU R's SEXPTYPE numbers) · flags u8 (object,
                    S4, ALTREP, immutable, GC bits) · gp u16 (GNU R's
                    LEVELS: encodings, missing, active/locked bindings)
                    · refcount u32 · attributes Sexp
vectors             + length · capacity (truelength), payload inline,
                    16-byte aligned; or a pointer for foreign/ALTREP storage
CHARSXP             interned (bytes, encoding) in a weak global cache;
                    NUL-terminated; cached hash; ASCII/UTF-8 bits
SYMSXP              interned, immortal; pointer equality
LISTSXP/LANGSXP     cons cells: car · cdr · tag
ENVSXP              frame (small inline / hashed), enclosure, binding
                    flags, version counters for the VM's guards
CLOSXP · PROMSXP    formals/body/env (+ compiled code) · value/expr/env
EXTPTRSXP · WEAKREFSXP · OBJSXP (S4) · BUILTINSXP/SPECIALSXP
```

- **Storage variants** for vectors: owned inline payload, compact
  sequences (`1:n`), foreign buffers (Arrow, NumPy, mmap: zero-copy
  interop), and ALTREP classes. `DATAPTR` materializes; element and region
  access does not have to.
- **Copy-on-modify** by reference counts, as in GNU R since 4.0.
- **Memory manager:** decided by measurement in G2 (`docs/design.md`
  §5.5): (a) reference counts with a trial-deletion cycle collector, or
  (b) a non-moving mark-region heap with conservative stack scanning,
  counts kept only for copy-on-modify. Both keep objects at fixed
  addresses (C holds pointers), make `PROTECT`/`R_PreserveObject` roots,
  and support finalizers and weak references.
- **Instance state:** symbol and string tables, epochs and caches belong
  to a runtime instance, not to `thread_local!`s, so several runtimes can
  live in one process (embedding). Values are single-threaded.

## Execution

- **AST interpreter:** bootstrap, `eval` of arbitrary language objects,
  and what the compiler hands back (`Op::Eval`).
- **Register VM (default):** closures and loops compiled to bytecode with
  guards against rebound functions, unboxed scalars, inline caches.
- **Native code:** Cranelift for hot loops today; whole closures chosen by
  sampling with contextual dispatch next; LLVM for the hottest code if it
  pays. Native code deoptimizes to the VM at op granularity, so results,
  warnings and errors are always the VM's. On wasm32 the VM is the top
  tier.
- **Non-local exits** are `Result<_, Flow>` inside robigo; at the C
  boundary, trampolines translate between `Flow` and `Rf_error`/
  `R_ContinueUnwind`/C++ exceptions.

## Numerics

One seam for every operation whose result depends on the numeric mode
(reductions, `cumsum`, `colSums`, `var`/`cov`, `%%`, `%*%`, formatting's
power-of-ten scaling, long-double nmath sites): *exact* (x87 long double
on x86_64, software 80-bit elsewhere; never a silent `f64` fallback) or
*fast* (`f64`, SIMD, reassociation). The mode is a runtime setting and
part of the compiled-code cache key. Lives in `robigo-numerics`.

## Text

Number formatting (`formatReal`, `scientific`, `encodeReal`), `sprintf`,
`formatC`, regular expressions (POSIX ERE via a DFA, PCRE-compatible
engine for `perl = TRUE`), encodings (UTF-8, Latin-1, bytes) and
collation. Pure functions over bytes and numbers, verified against GNU R
on generated inputs; moves to `robigo-text`.

## Interop and embedding

- C: the `libR.so` ABI, both directions (packages call robigo; hosts embed
  robigo like `libR`).
- Zero-copy buffers: foreign vector storage for Arrow, NumPy, Polars.
- A Rust embedding API over `robigo-core` with explicit ownership.
- WebAssembly: `wasm32` build without the JIT.
