# robigo-core and robigo-capi (goalpost G2)

The new object model, its memory manager, and GNU R 4.5's C API exported
over it as a `libR.so`. Spec: `docs/design.md` §3.1, §5.3 (F1–F6), §5.5
and G2 in §5.6; `docs/architecture.md` "The core object model". The
memory manager was decided by the bake-off (`experiments/memory`,
`docs/ledger/memory.md`): a non-moving mark-region heap with
conservative stack scanning and reference counts kept only for
copy-on-modify. This document fixes the concrete layout and the
boundaries; the ledger (`docs/ledger/core.md`) records what was measured.

**Exit gate (G2):** the installed `fastmap` and `digest` binaries (built
by GNU R 4.5.0) load unmodified into robigo and pass their tests.
**Metrics, in order:** (a) `capi` corpus cases passing
(`scripts/corpus.py run capi`); (b) weighted coverage of
`tests/capi/symbols.tsv` by *implemented* exports (stubs that raise
"not implemented" do not count; `crates/robigo-capi/coverage.py`);
(c) the gate packages.

## 1. What the ABI fixes, and what is ours

A package compiled against GNU R 4.5's headers without `USE_RINTERNALS`
reaches R objects only through:

- **functions** (`INTEGER`, `STRING_ELT`, `Rf_allocVector`, `ATTRIB`,
  `TYPEOF`, `LENGTH`, ...): every accessor is a call, so the object layout
  is ours;
- **data symbols**: `SEXP` variables (`R_NilValue`, `R_GlobalEnv`,
  `R_NamesSymbol`, `R_NaString`, `R_BlankString`, ...), numbers
  (`R_NaInt`, `R_NaReal`, `R_NaN`, `R_PosInf`, `R_NegInf`), and a few
  whose *layout* is fixed: `R_PPStack`/`R_PPStackTop`/`R_PPStackSize`
  (`SEXP*`, `int`, `int`: the protect stack is a C array of `SEXP`, as in
  GNU R), `R_interrupts_pending`, `R_Interactive`, `R_CStackLimit`,
  `R_isForkedChild`, `R_TempDir`, ...;
- **struct layouts in headers**: `R_CallMethodDef` & co (registration),
  `Rcomplex` (`{double r, i}`), `R_altrep_class_t` (`{SEXP ptr}`),
  `DllInfo` (opaque), `R_outpstream_st` (serialization), `DevDesc`.

So: a `SEXP` is one pointer to a robigo heap object; the C ABI is exactly
GNU R 4.5's exported symbol set (`symbols.tsv` ranks it by use); the
layout behind the pointer is free. Rust code inside robigo sees the same
pointer as `Sexp` (a `Copy` newtype, no handle table: design §5.5).

## 2. Object layout

All objects are 16-byte aligned, 16-byte granules, allocated by the heap
(§3). Every object starts with a 16-byte header:

```text
+0  u8   type     GNU R's SEXPTYPE number (NILSXP 0 ... OBJSXP 25)
+1  u8   flags    OBJECT 0x01 · ALTREP 0x02 · EXTDATA 0x04 · IMMORTAL 0x08
                  · MARK_A 0x10 · MARK_B 0x20 · HOST 0x40 · TRACKREFS 0x80
+2  u16  gp       GNU R's sxpinfo.gp, bit for bit (LEVELS): encodings
                  BYTES 1<<1, LATIN1 1<<2, UTF8 1<<3, S4 1<<4, CACHED 1<<5,
                  ASCII 1<<6; MISSING 0..3, DDVAL 0, ACTIVE_BINDING 1<<15,
                  BINDING_LOCK 1<<14, GROWABLE 1<<5, ...
+4  u16  refcnt   copy-on-modify count (GNU R's REFCNT; saturates)
+6  u16  extra    debug/trace/step bits, NOJIT; spare
+8  SEXP attrib   pairlist of attributes, R_NilValue when none
                  (CHARSXP: u32 hash + u32 spare instead)
```

`OBJECT` is GNU R's `obj` bit (has a class attribute), kept in sync by
`setAttrib`/`classgets` like GNU R. `S4` lives in `gp` exactly where GNU R
keeps it, so `LEVELS`/`SETLEVELS` round-trip.

| kind | types | body after the header |
|---|---|---|
| vector | LGL INT REAL CPLX STR VEC EXPR RAW (and WEAKREF) | +16 `length` i64 · +24 `truelength` i64 · +32 payload |
| CHARSXP | CHAR | as a vector of bytes: +16 length · +24 truelength (package scratch, e.g. data.table) · +32 bytes, NUL-terminated |
| node | LIST LANG DOT PROM CLO ENV SYM EXTPTR BCODE OBJ(S4) | +16 car · +24 cdr · +32 tag (GNU R's three node fields under its accessor names: `PRINTNAME/SYMVALUE/INTERNAL`, `FORMALS/BODY/CLOENV`, `PRVALUE/PRCODE/PRENV`, `EXTPTR_PTR/PROT/TAG`, `FRAME/ENCLOS/HASHTAB`) |
| ALTREP | any vector type with flag ALTREP | +16 data1 · +24 data2 · +32 class (a RAWSXP holding the method table, as GNU R) |
| external data | vector with flag EXTDATA | +16 length · +24 truelength · +32 data pointer · +40 owner word · +48 release fn |
| host proxy | ENV CLO PROM BUILTIN SPECIAL with flag HOST | +16 host handle · +24 release fn · +32 spare (§6) |

- **Payload access** (`DATAPTR`, `INTEGER`, `REAL`, ...): flags clear →
  `x + 32`; EXTDATA → the stored pointer; ALTREP → the class's `Dataptr`
  method (materializing). One header-byte test on the fast path, the
  same shape as GNU R's `ALTREP(x) ? ALTVEC_DATAPTR(x) : STDVEC_DATAPTR(x)`.
  EXTDATA is not ALTREP: `ALTREP(x)` is false, so packages that branch on
  it see an ordinary vector.
- **Storage variants** (F2): *owned* (inline payload), *external*
  (EXTDATA: a buffer owned by someone else, released by the sweeper —
  Arrow/NumPy/mmap buffers, and the bridge's zero-copy arguments, §6),
  *compact* and other *ALTREP* classes (compact intseq/realseq, wrappers,
  deferred strings are built-in classes; packages define their own with
  `R_make_alt*_class`).
- **Scalars and small vectors** need no special case: a length-1 INTSXP
  is 48 bytes (GNU R: 56). Immortal shared constants: `R_NilValue`,
  `R_TrueValue`/`R_FalseValue`/`R_LogicalNAValue`, `NA_STRING`,
  `R_BlankString`, `R_BlankScalarString`, `R_MissingArg`,
  `R_UnboundValue`, `R_RestartToken`, every symbol.
- **Long vectors**: `length` is `R_xlen_t` from the start.

## 3. Memory: the mark-region heap

The `mm-gc` prototype becomes `robigo_core::heap`, unchanged in policy:
64 KiB blocks in 1 MiB chunks, 44 size classes up to 2 KiB, a dedicated
chunk per larger object, free lists rebuilt by the sweeper, fully empty
blocks pooled, dead chunks returned to the OS, a 2 MiB collection floor,
growth to 1.5x the live bytes, alternating mark colors and skip-clean
sweeps with per-block immortal counts. Objects never move, so `REAL(x)`
held by C stays valid across collections.

**Roots** (marked precisely, then traced):

1. the protect stack `R_PPStack[0 .. R_PPStackTop]` (GNU R's own layout,
   exported; `PROTECT`/`UNPROTECT`/`PROTECT_WITH_INDEX`/`REPROTECT`
   index it exactly as memory.c does, including the
   "protect(): protection stack overflow" and `R_signal_reprotect_error`
   errors);
2. the precious list (`R_PreserveObject`/`R_ReleaseObject`,
   `R_PreserveInMSet`), a counted hash set;
3. the global `SEXP` variables of the API and the host-proxy table (§6);
4. **the machine stack, conservatively**: every aligned word from the
   current stack pointer up to the top of the thread's stack
   (`pthread_getattr_np`), after spilling callee-saved registers into a
   `jmp_buf` on the stack. Interior pointers resolve to their object in
   O(1) via the per-block start bitmap, so `double *p = REAL(x) + i`
   alone keeps `x` alive. This is what makes unprotected-but-live C
   locals (common package bugs that GNU R tolerates by luck) safe.

Symbols and the immortal constants are never marked (block immortal
counts). The CHARSXP cache is weak: it is pruned after marking, before
any memory is reused. Finalizers (`R_RegisterCFinalizer[Ex]`,
`R_RegisterFinalizer[Ex]`) and weak references follow memory.c's
`WEAKREFSXP` protocol: after marking, weak references whose key is
unmarked have their finalizer scheduled (key/value/finalizer then marked
so they survive this collection), and finalizers run after the sweep,
outside the collector, under `R_ToplevelExec`. `onexit = TRUE`
finalizers run at exit.

**Copy-on-modify counts** (`refcnt`) are GNU R's `REFCNT` protocol since
4.0: `SETCAR`/`SET_VECTOR_ELT`/`SET_ATTRIB`/`SET_STRING_ELT`/`defineVar`
increment the new value and decrement the old; locals and the protect
stack do not count. `NAMED(x)` is `REFCNT(x)`, `MAYBE_SHARED(x)` is
`REFCNT(x) > 1`, `MARK_NOT_MUTABLE` saturates. When the sweeper frees an
object it does not decrement its children (GNU R does not either: a dead
parent's count contribution only makes a survivor look more shared,
which is conservative).

**Allocation fast path:** size-class index, free-list pop or bump,
header store (two 8-byte writes), start bit; the collection check is one
compare. `allocVector` of a fresh vector does not zero numeric payloads
(GNU R does not), `STRSXP` is filled with `R_BlankString` and `VECSXP`/
`EXPRSXP` with `R_NilValue` as GNU R does. Instruction counts per
allocation and per collection are recorded in the ledger
(`cargo bench`-free: callgrind on the unit-test workloads).

## 4. Strings: the CHARSXP cache and encodings

`mkCharLenCE(bytes, len, enc)` interns `(bytes, encoding)`:

- the cache is an open-addressing table of weak `SEXP`s keyed by a 32-bit
  hash of the bytes (stored in the CHARSXP's header word +8), compared by
  length, bytes and the encoding bits, so `mkCharCE("é", CE_UTF8)` and a
  Latin-1 `"é"` are different objects, as in GNU R;
- the encoding bits are GNU R's: ASCII is set for all-ASCII strings (and
  then no other encoding bit is kept: `mkCharLenCE` drops UTF8/LATIN1 for
  ASCII input), `CE_BYTES` sets BYTES, embedded NULs are the error
  "embedded nul in string: '...'" verbatim;
- `NA_STRING` is a distinct immortal CHARSXP `"NA"` (not in the cache);
  `R_BlankString` is the cached `""`, immortal;
- `translateChar`/`translateCharUTF8`/`Rf_reEnc` work on the C.UTF-8
  locale robigo implements (design §6): native = UTF-8; Latin-1 is
  converted, bytes are escaped as GNU R does (`<xx>`);
- pointer equality of CHARSXPs means equal strings with equal encodings,
  and `TRUELENGTH` is free scratch for packages (data.table relies on
  both).

Symbols are an immortal interned table keyed by the PRINTNAME's CHARSXP
pointer; `install(name)` is one hash probe after `mkChar`.

## 5. Non-local exits across C and Rust

GNU R unwinds with `longjmp`; package code is compiled for that and does
not run destructors (Rcpp and cpp11 convert C++ exceptions themselves
and use `R_UnwindProtect` to get control back). robigo keeps GNU R's
model at the C boundary:

- A **context stack** of `jmp_buf`s lives in a tiny C shim
  (`robigo-capi/src/shim.c`; Rust cannot call `setjmp`). Every entry from
  robigo into C code (the bridge's `.Call`), every `R_ToplevelExec`,
  `R_tryCatch`, `R_UnwindProtect`, `R_ExecWithCleanup` pushes one.
- `Rf_error`/`Rf_errorcall` (C-variadic, defined in Rust: stable since
  Rust 1.9x) format the message with `vsnprintf` into a fixed buffer,
  record the pending condition (call + message, or the host's `Flow`),
  and `longjmp` to the innermost context. Frames between a context and
  the jump must be plain (no pending `Drop`): every exported function that
  can jump finishes its Rust work, drops its owned values, and only then
  calls the jump in tail position.
- **Callbacks into robigo** (`Rf_eval`, `R_forceAndCall`, `findFun`
  through active bindings, ...) return `Result<Sexp, Flow>` from the host;
  an `Err` is stored as the pending condition and jumped with exactly as
  `Rf_error` would, so an R error inside `Rf_eval` called from C unwinds
  the C frames like GNU R and reappears as the same `Flow` (error,
  `return`/`break`/restart, interrupt) at the `.Call` boundary.
- `R_UnwindProtect(fun, data, cleanfun, cleandata, cont)` pushes a
  context, runs `fun`; on a jump it calls `cleanfun(cleandata, TRUE)` and
  continues the jump with `R_ContinueUnwind(cont)`, the token holding the
  pending condition. `R_MakeUnwindCont` allocates the token (a node).
- Rust panics never cross into C (`extern "C"` aborts on unwind); C++
  exceptions never cross into robigo (packages catch their own).
- Warnings (`Rf_warning`, `Rf_warningcall`) go to the host's warning
  machinery directly (no jump); `R_CheckUserInterrupt` polls the host's
  interrupt flag and jumps with an interrupt condition.

## 6. The bridge: the current runtime calls compiled code (before G3)

Until G3 rebuilds the evaluator on the core, robigo's `Value`
(`Rc`-based, `crates/robigo-runtime/src/value.rs`) and core `SEXP`s live
side by side; the bridge converts at the `.Call`/`.External`/`.C`
boundary.

**Loading.** `robigo-capi` is a `cdylib` named `libR.so` with
`DT_SONAME libR.so`, installed next to the `robigo` binary
(`scripts/build.sh`; CI copies it with the binary). The first real
`dyn.load()` opens it with `RTLD_NOW | RTLD_GLOBAL`; package objects
whose `DT_NEEDED` says `libR.so` then bind to it (glibc matches already
loaded objects by soname), so binaries built by `R CMD SHLIB`/`R CMD
INSTALL` against GNU R load unmodified. Package `.so`s are opened with
`RTLD_LAZY | RTLD_LOCAL` (GNU R's default `now = TRUE` is honoured once
every census symbol is exported), and `R_init_<pkg>` runs for routine
registration (`R_registerRoutines`, `R_useDynamicSymbols`,
`R_forceSymbols`, `R_RegisterCCallable`/`R_GetCCallable`). Unregistered
symbols are found with `dlsym`, as Rdynload.c does. GNU R's own base
packages (`stats`, `utils`, `grDevices`, ...) keep their current Rust
ports until their `.so`s are switched over one by one (measured against
the corpora).

The runtime does not link `robigo-core`: it resolves the C API it needs
from the loaded `libR.so` with `dlsym` (the same symbols packages use,
plus a few `robigo_*` entry points), so there is exactly one heap and
one set of API globals in the process. `libR.so` in turn gets a table of
host callbacks (`robigo_capi_init`) for what lives in the runtime:
evaluation, environments, conditions, printing, options, parsing,
coercion and printing helpers it has not ported yet.

**Value → SEXP** (arguments):

| robigo value | core object |
|---|---|
| `NULL` | `R_NilValue` |
| logical/integer/double/complex/raw vector | **zero-copy**: an EXTDATA vector whose data pointer is the `Vec`'s buffer and whose owner word holds an `Rc` clone (released by the sweeper). C writes through `INTEGER(x)` are visible to the caller, as in GNU R. Attributes are converted. |
| character vector | a STRSXP of cached CHARSXPs (`Rc<str>` is UTF-8; non-ASCII strings are marked UTF-8) |
| list / expression | converted element-wise |
| symbol | the interned core symbol |
| call / pairlist | cons cells, tags as symbols |
| environment, closure, promise, builtin | a HOST proxy holding the `Value`; one proxy per robigo object (identity: `rho == R_GlobalEnv` works), kept in an identity table that the collector treats as weak |
| external pointer created by C | the original core object (robigo's `ExtPtr` wraps it, §below) |

**SEXP → Value** (results, `Rf_eval` arguments): the inverse. HOST
proxies and EXTDATA vectors return the robigo object they wrap (identity
kept); other core vectors are copied into `Vector`s; EXTPTRSXP,
WEAKREFSXP, S4 objects and anything without a robigo equivalent become a
robigo `ExtPtr` whose payload keeps the core object preserved (released
when robigo drops it), so passing it back to C yields the same `SEXP`
(`fastmap` keeps its hash map in such a pointer).

Environments are always robigo environments in the bridge: `R_NewEnv`,
`defineVar`, `findVar`, `R_existsVarInFrame`, active bindings, locking,
`R_lsInternal` call the host. Closures created in C (`R_mkClosure`) are
created by the host.

Known differences until G3 (documented, measured by the corpus):
`SET_STRING_ELT`/`SET_VECTOR_ELT` on an *argument* do not reach the
caller's object; `NAMED`/`MAYBE_SHARED` on arguments report "shared"
(always safe for C code: it copies).

## 7. Crates and boundaries

- `crates/robigo-core` (rlib, no `unsafe` outside `heap`/`object`
  modules' audited accessors): `heap` (blocks, size classes, mark, sweep,
  conservative scan), `object` (header, types, typed views
  `Sexp::as_slice::<f64>()`), `strings` (CHARSXP cache, encodings),
  `symbols`, `altrep`, `weak` (weak refs and finalizers). No R semantics
  beyond memory.c/`Rinlinedfuns.h`-level constructors and accessors.
- `crates/robigo-capi` (`cdylib` `libR.so` + `rlib` for tests): one module
  per GNU R source file whose API it exports (`memory.rs`, `attrib.rs`,
  `envir.rs`, `errors.rs`, `rdynload.rs`, `altrep.rs`, `arithmetic.rs`
  for `R_NaReal` & co., `nmath.rs` re-exporting `robigo-numerics`, ...),
  `shim.c` for `setjmp`, and `host.rs` (the callback table). Unimplemented
  census symbols are exported as stubs that raise
  "robigo: C API function 'X' is not implemented yet" (porting contract
  §6), never as guesses.
- `crates/robigo-runtime/src/builtins/dotcode.rs` + `native/` (bridge
  side): `dlopen`, registration tables, conversion, host callbacks.

## 8. Order of work and parallel tracks

1. Core heap + objects + strings + symbols, unit-tested (allocation, GC
   under PROTECT and conservative roots, CHARSXP interning and encodings,
   unwind through C), with instruction counts per operation.
2. `libR.so` with the top of `symbols.tsv` (registration, allocation,
   accessors, attributes, errors), the bridge, `.Call`/`.External`/`.C`.
3. Hill-climb the `capi` corpus case by case; then `fastmap`, `digest`.

Once the API exists, the work splits along `symbols.tsv`'s areas, each a
track with the capi corpus slice and the census coverage as its metric:
**capi-objects** (attributes, coercion, duplication, `Rf_*` predicates),
**capi-envs-eval** (environments, evaluation, conditions, `R_tryCatch`),
**capi-altrep** (ALTREP classes, compact sequences, external storage),
**capi-nmath-appl** (Rmath, LINPACK/`appl`, BLAS/LAPACK re-exports),
**capi-graphics** (`GE*`/`DevDesc`, grDevices.so/graphics.so/grid.so), and
**base-natives** (switching `stats.so`, `utils.so`, `tools.so`,
`grDevices.so` from Rust ports to GNU R's own shared objects, corpus by
corpus). G3 (the runtime on the core) then removes the bridge.
