# robigo-core and robigo-capi (goalpost G2)

The new object model, its memory manager, and GNU R 4.5's C API exported
over it as a `libR.so`. Spec: `docs/design.md` §3.1, §5.3 (F1–F6), §5.5
and G2 in §5.6; `docs/architecture.md` "The core object model". The
memory manager was decided by the bake-off (`experiments/memory`,
`docs/ledger/memory.md`): a non-moving mark-region heap with
conservative stack scanning and reference counts kept only for
copy-on-modify. This document fixes the concrete layout and the
boundaries; the ledger (`docs/ledger/core.md`) records what was measured.

**The contract (lead's decision, 2026-10-28): source compatibility.**
Max's existing R scripts must run on robigo unchanged, and robigo must be
the fastest R; recompiling packages from source for robigo is acceptable.
So robigo does *not* load GNU R's pre-compiled binaries (neither GNU R's
own base-package `.so`s nor installed CRAN binaries). It is compatible
with GNU R's C API at the *source* level: `R.h`, `Rinternals.h`,
`Rinlinedfuns.h`, the `R_ext/` headers, and the `Defn.h`/`USE_RINTERNALS`
level that GNU R's own base-package sources use. Packages are reinstalled
for robigo from source, the way users reinstall after an R major-version
upgrade, through GNU R's own install code (§8).

**Exit gate (G2):** inside robigo, `install.packages("fastmap", repos =
<CRAN or a local source repository>)` builds `fastmap` from source against
robigo and its tests pass; then `digest`, a small cpp11 package and a small
Rcpp package (C++ exceptions and `R_UnwindProtect`); and GNU R's
base-package natives built from the R 4.5.0 sources against robigo's
headers, `stats` first (it unblocks `deriv`, splines, `cov`/`cor`, `optim`
and most dataset examples). Binary repositories (PPM, r-universe) are out
of scope; a robigo binary cache can come later.
**Metrics, in order:** (a) `capi` corpus cases passing
(`scripts/corpus.py run capi`, its cases compiled against robigo's
headers); (b) weighted coverage of `tests/capi/symbols.tsv` by
*implemented* exports (stubs that raise "not implemented" do not count;
`crates/robigo-capi/coverage.py`); (c) the gate.

## 1. What the C API fixes, and what is ours

Package sources reach R objects through the API's functions, macros and
inline functions and a few global variables:

- **functions** (`INTEGER`, `STRING_ELT`, `Rf_allocVector`, `ATTRIB`,
  `TYPEOF`, `LENGTH`, ...), exported by `libR.so`;
- **data symbols**: `SEXP` variables (`R_NilValue`, `R_GlobalEnv`,
  `R_NamesSymbol`, `R_NaString`, `R_BlankString`, ...), numbers
  (`R_NaInt`, `R_NaReal`, `R_NaN`, `R_PosInf`, `R_NegInf`), and a few
  whose type is fixed by the headers: `R_PPStack`/`R_PPStackTop`/
  `R_PPStackSize` (`SEXP*`, `int`, `int`: the protect stack is a C array
  of `SEXP`, as in GNU R), `R_interrupts_pending`, `R_Interactive`,
  `R_CStackLimit`, `R_isForkedChild`, `R_TempDir`, ...;
- **struct layouts in headers**: `R_CallMethodDef` & co (registration),
  `Rcomplex` (`{double r, i}`), `R_altrep_class_t` (`{SEXP ptr}`),
  `R_outpstream_st` (serialization), `DevDesc`, and for base-package
  sources `RCNTXT` — all sized by robigo's headers.

**The layout behind a `SEXP` is robigo's**, chosen for speed (§2), and
robigo's headers map the macros onto it.

*Audit (2026-10-28), why the headers matter:* `Defn.h` defines
`USE_RINTERNALS` (Defn.h:49-51), and 39 C files of GNU R's base packages
include it — stats (cov, deriv, model, optim, optimize, splines), graphics
(base, graphics, par, plot, plot3d), grDevices (13 files: devices, colors,
devPS, ...), methods (methods_list_dispatch, utils,
do_substitute_direct), utils (io, size, stubs), tools (install, Rmd5,
Rsha256, gramRd, ...), parallel (fork), tcltk. Those compile the
accessors as direct field accesses: counted over their sources, `CDR` 579,
`REAL` 578, `INTEGER` 482, `CHAR` 210, `CADR` 198, `LENGTH` 153, `TYPEOF`
121, `CADDR` 94, `LOGICAL` 48, `XLENGTH` 21, `TAG` 16, `RAW` 15,
`COMPLEX` 10, `PRINTNAME` 8, and single uses of `CLOENV`, `FORMALS`,
`BODY`, `ATTRIB`, `ENCLOS`, `SYMVALUE`, `INTERNAL`, `DDVAL`, `ENC_KNOWN`,
`IS_CACHED`, `ENSURE_NAMEDMAX`, `RTRACE`/`SET_RTRACE`, `SET_OBJECT`,
`SET_TYPEOF`, `EXTPTR_TAG`/`EXTPTR_PROT`. The installed `stats.so` imports
`TYPEOF`, `CAR`, `INTEGER`, ... as functions only from its other files.
Loading GNU R's own binaries would have pinned GNU R's `SEXPREC` (a
32-byte header with two generation-list words); under source
compatibility robigo instead builds those sources against its own
`Defn.h`, whose macros address robigo's layout. CRAN packages cannot
define `USE_RINTERNALS` since R 4.3 (the census confirms they import the
accessors as functions), so they need nothing beyond the public headers.

## 2. Object layout

All objects are 16-byte aligned, 16-byte granules, allocated by the heap
(§3). Every object starts with a 16-byte header:

```text
+0  u8   type     GNU R's SEXPTYPE number (NILSXP 0 ... OBJSXP 25)
+1  u8   flags    OBJECT 0x01 · ALTREP 0x02 · EXTDATA 0x04 · IMMORTAL 0x08
                  · MARK_A 0x10 · MARK_B 0x20 · HOST 0x40 · NO_TRACKREFS 0x80
                  (GNU R's `spare`: TRACKREFS is `!spare`)
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

**Header-level inlining.** robigo's `Rinternals.h` may define the hot
accessors inline (`TYPEOF` is one byte load; `INTEGER`/`REAL`/`DATAPTR`
test the ALTREP|EXTDATA byte and otherwise return `x + 32`; `LENGTH`
reads `+16` under the same test) where that is measured faster and stays
correct; the exported functions remain for code that takes their
address. `Defn.h` for base-package sources defines the `USE_RINTERNALS`
macros (`CAR`, `CDR`, `TAG`, `PRINTNAME`, `ENC_KNOWN`, `RTRACE`,
`SET_TYPEOF`, ...) against the same layout.

## 3. Memory: the mark-region heap

The `mm-gc` prototype becomes `robigo_core::heap`, unchanged in policy:
64 KiB blocks in 1 MiB chunks, 44 size classes up to 2 KiB, a dedicated
chunk per larger object, free lists rebuilt by the sweeper, fully empty
blocks pooled, dead chunks returned to the OS, a 2 MiB collection floor,
growth to 1.5x the live bytes, alternating mark colors and skip-clean
sweeps with per-block immortal counts. Every slot is carved at its size
class (the bake-off's allocator cut bump-allocated slots to the request
size, so reusing a dead slot for a larger request of the same class
overran its neighbour; fixed and tested over every request-size pair).
Objects never move, so `REAL(x)`
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

**The rooting rule.** A `Sexp` survives a collection only if it is (1) in
a local or register of the R thread's machine stack (conservative scan;
interior and one-past-the-end pointers count), (2) on the protect stack
or the precious set, (3) reachable from a traced object, or (4) in a
container registered as a root (`Core::root_fns`). A `Sexp` stored
anywhere else — a Rust `Vec`, a `HashMap`, a VM register file or JIT
cache in heap memory, a C global — is invisible and will be freed.
Every off-stack container of `Sexp`s is therefore either a traced R
object (a VECSXP), protected for its lifetime, or registered with a root
callback. `ROBIGO_GCTORTURE=1` collects on every allocation and poisons
freed slots, so a violation fails at once instead of rarely; CI runs the
core and capi tests (and later the capi corpus) under it.

Symbols and the immortal constants are never marked (block immortal
counts), but their fields can change (a symbol's value, attributes put
on a shared constant), so every collection traces the children of the
non-CHARSXP immortals. The CHARSXP cache is weak: it is pruned after marking, before
any memory is reused. Finalizers (`R_RegisterCFinalizer[Ex]`,
`R_RegisterFinalizer[Ex]`) and weak references follow memory.c's
`WEAKREFSXP` protocol: values and finalizers of references with live
keys are marked to a fixed point; then (CheckFinalizers) every reference
whose key died is ready to finalize, with or without a finalizer, and
every reference keeps key, value and finalizer alive for this round.
Finalizers run after the collection, at safe points (after a native call
returns, `gc()`), outside the collector; R finalizers' errors are
reported, not propagated. `onexit = TRUE`
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

### 4.1 Encodings in the runtime (next step, proposal)

The runtime's strings are `RStr = Option<Rc<str>>`: valid UTF-8, no
encoding mark. GNU R's CHARSXPs carry `LATIN1`/`UTF8`/`BYTES`/`ASCII`
and may hold bytes that are not UTF-8. This blocks fastmap's encoding
tests, 17 iconv cases (conformance-strings ledger) and `Encoding<-`.
The proposal:

- **Representation**: a newtype `RString(Rc<str>)` whose first byte is
  a structural mark (native-valid, UTF-8, latin1, bytes,
  native-invalid), and whose content is valid UTF-8 always. Latin-1
  text is stored decoded; bytes and invalid native strings are mapped
  byte to U+0000..U+00FF. The original bytes are therefore a function
  of (mark, content), and `Deref<Target = str>` gives the content to
  the existing text code. The value stays 16 bytes (fat pointer); no
  side table.
- **Encoding-aware operations** read the mark:
  - `Encoding`/`Encoding<-`, `enc2utf8`/`enc2native`, `iconv`,
    `charToRaw`, `nchar(type = "bytes")`;
  - printing of bytes (`\xe7`), `useBytes`, `validUTF8`;
  - serialize (GNU R's LATIN1/BYTES levels with the original bytes)
    and readLines/scan `encoding=`;
  - the bridge: CHARSXP marks map both ways; `translateCharUTF8` is the
    content.

  Everything else keeps seeing UTF-8 text, as GNU R in a UTF-8 locale
  translates first.
- **Change plan**: about 120 construction sites (`Some(Rc::from(..))`,
  `rstr(..)`) across 46 runtime/capi files become `RString`
  constructors, mechanically, in one PR coordinated with the tracks
  editing those files.

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
- `R_UnwindProtect(fun, data, cleanfun, cleandata, cont)` runs `fun` in a
  context; on a jump it stores the pending exit in `cont`, pops the
  context, calls `cleanfun(cleandata, TRUE)` directly (cpp11 and Rcpp
  leave it by longjmp to their own setjmp and call `R_ContinueUnwind`
  later) and otherwise continues the unwind itself: like GNU R's, it
  never returns after a jump. A context also saves the host's handler and
  restart stacks and restores them when a jump reaches it
  (`R_restore_globals`).
- *Open (council review):* longjmp skips Rust frames, which is defined
  only for frames without pending destructors. Today `libR.so` keeps the
  rule by construction (messages are formatted into stack buffers and
  owned values dropped before jumping; no locks or `RefCell` guards are
  held across calls that can jump; host callbacks return tokens and the
  jump happens in `libR.so`). The stronger form the council proposed —
  inner functions return `Result`, and only a thin exported wrapper (or a
  C trampoline) jumps — is the planned refactor before G3 puts the
  evaluator itself behind these calls.
- Warnings can jump too (`options(warn = 2)`, a calling handler that
  invokes a restart): the warning callback returns a token like errors.
- *C++ exceptions*: an exception escaping a package's callback is fatal in
  GNU R too (it unwinds into C frames without unwind tables and
  terminates); robigo's callback types stay `extern "C"`, so it aborts
  deterministically instead of unwinding through Rust. Rcpp and cpp11
  catch their exceptions and convert them through `R_UnwindProtect`,
  which is the path that must work (gate).
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
`DT_SONAME libR.so`, installed in robigo's `R_HOME/lib` next to the
binary (`scripts/build.sh`; CI copies it with the binary). The first
`dyn.load()` of a package object opens it with `RTLD_GLOBAL`; package
objects link against it (`-lR` from robigo's Makeconf) and bind to the
loaded copy by soname. `R_init_<pkg>` runs for routine registration
(`R_registerRoutines`, `R_useDynamicSymbols`, `R_forceSymbols`,
`R_RegisterCCallable`/`R_GetCCallable`); unregistered symbols are found
with `dlsym`, as Rdynload.c does. GNU R's own base packages keep their
Rust ports until their sources are built against robigo (§8), one by
one, measured against the corpora.

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
| logical/integer/double/complex/raw vector | **zero-copy**: an EXTDATA vector whose data pointer is the `Vec`'s buffer and whose owner word holds an `Rc` clone (released by the sweeper; `ALTREP(x)` stays false). C writes through `INTEGER(x)` are visible to the caller, as in GNU R. Attributes are converted. |
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

## 8. Headers and the package build path

No new package manager: GNU R's install path is R code robigo runs —
`install.packages()` → `R CMD INSTALL` = `tools:::.install_packages()` →
`R CMD SHLIB` → `make` with `R_HOME/etc/Makeconf` (pak, renv and remotes
all end there). robigo provides what that path expects:

- **`R_HOME` layout**: `bin/` (the `robigo` binary, an `R` front end),
  `lib/libR.so`, `include/` (robigo's headers), `etc/Makeconf`
  (`R_INCLUDE_DIR` → robigo's headers, `LIBR = -L$(R_HOME)/lib -lR`,
  compilers and flags as GNU R 4.5.0's), `library/` (base packages; their
  R code and, once built, their `libs/*.so`), `share/make/shlib.mk` and
  friends from GNU R. `R.home()`/`R_HOME` point there.
- **`robigo CMD INSTALL|SHLIB|...`**, mirroring GNU R's `bin/R` dispatch:
  `CMD SHLIB` and `CMD INSTALL` run `tools:::.SHLIB()` /
  `tools:::.install_packages()` in robigo.
- **Headers**: GNU R 4.5.0's public headers (GPL, vendored) with the
  layout-dependent parts replaced: `Rinternals.h`'s inline accessors (if
  any, §2), `Rinlinedfuns.h`, and a `Defn.h` subset for base-package
  sources (`USE_RINTERNALS` macros, `RCNTXT`, internal entry points those
  sources call). A package binary built against GNU R's stock public
  headers still works against `libR.so` (functions only); the capi corpus
  is built against robigo's headers.
- **Base packages' natives from GNU R's sources, by default** (lead,
  2026-10-28). `scripts/build-natives.sh [PKG...]`:
  - configures the R 4.5.0 sources once (`.robigo/natives/build`; the
    sources are downloaded and checked against their SHA-256 when
    missing);
  - compiles a package's `SOURCES_C`/`SOURCES_F` (from its
    `Makefile.in`) with GNU R's Makeconf flags (`portsrc.f` with
    `SAFE_FFLAGS`, as GNU R) and `-DTESTING_WRITE_BARRIER`, so GNU R's own
    Defn.h calls the exported accessor functions instead of reading
    fields;
  - links `.robigo/natives/<pkg>/<pkg>.so` with a `robigo_base_native`
    marker symbol.

  `dyn.load` of a base package (one GNU R's build registers routines
  for) loads that build when it exists: `$ROBIGO_NATIVES`, else
  `natives/` beside the binary's directory. It keeps GNU R's path, so
  `getLoadedDLLs()`, `R.home()` and tracebacks look as in GNU R. Without
  one, the Rust ports serve the package. Measured for stats (ledger):
  - fastr 30,576 vs 30,311 cases and gnur-examples 571 vs 545, with 0
    regressions;
  - vectorised d/p/q functions ~7% slower than the ports (the vector
    copies of the bridge; GNU R is in between);
  - the ports stay until G3 removes that copy, then go, measured.
- The remaining base packages follow stats: `utils`, `tools`,
  `grDevices`, `graphics`, `grid`, `splines`, `parallel`. `methods` is
  G6's (§13).

## 9. Order of work and parallel tracks

1. Core heap + objects + strings + symbols, unit-tested (allocation, GC
   under PROTECT and conservative roots, CHARSXP interning and encodings,
   unwind through C), with instruction counts per operation. *Done.*
2. `libR.so` with the top of `symbols.tsv` (registration, allocation,
   accessors, attributes, errors), the bridge, `.Call`/`.External`/`.C`;
   hill-climb the `capi` corpus case by case.
3. The `R_HOME` layout, headers, Makeconf and `robigo CMD SHLIB|INSTALL`;
   switch the capi corpus build to robigo's headers. *Done*
   (`scripts/robigo-home.sh`; the capi corpus builds with `robigo CMD
   SHLIB`).
4. The gate: `install.packages("fastmap")` from source inside robigo and
   its tests; `digest`; a small cpp11 and a small Rcpp package; then
   `stats` from source, the other base packages after it. *Measured
   (ledger 2026-10-28):*
   - installed from CRAN sources inside robigo, with their own tests:
     fastmap 302/341 expectations (the rest is latin1), digest 325/325,
     timechange (cpp11) 967/967;
   - Rcpp compiles but cannot load: it imports methods, whose natives
     need the evaluator's S4 hooks (`R_set_standardGeneric_ptr`,
     `R_set_quick_method_check`, `do_set_prim_method`, `R_execMethod`,
     ...: G6);
   - `tools.so` from source runs `R CMD INSTALL`; `stats.so` from
     source loads; `src/appl` is next.

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

## 10. Parallelism and isolates (lead's requirement, 2026-10-28)

robigo must have at least GNU R's parallel story and should beat it.
GNU R's frontier is process-based: `parallel` (fork/`mclapply`, PSOCK),
`future`, `mirai` (persistent daemons over nanonext/NNG) and `mori`
(shared-memory R objects: POSIX shm + ALTREP, lazy access in other
processes, ALTREP serialization hooks so a shared object serializes as a
reference).

**Parity.** `nanonext`, `mirai`, `mori` and `future` build from source and
pass their tests on robigo; they join the G4 package ratchet after
fastmap/digest/cpp11/Rcpp. That needs ALTREP class registration with the
serialization hooks (robigo has no ALTREP serialization today:
`docs/ledger/startup.md`) and fork safety.

**Decision A.1, fork-safe collection: side mark bitmaps** (implemented:
a collection in a forked child copies 17 MB of a 229 MB heap instead of
226 MB; ledger). `mclapply`
forks a live heap; a collector that writes mark bits into object headers
dirties every copy-on-write page of the live set in every child. Marks
therefore move out of the header into per-block side bitmaps (next to
the start bitmap in the 576-byte block header area; a bit in the chunk
header for large objects), and the collector writes no other header
state of old objects (immortal and host bits are written once, at
creation; the sweep writes only dead slots' free-list links). The
header's MARK_A/MARK_B bits are retired when this lands (with the
store()/barrier work, (e)); skip-clean sweeps keep working by comparing
per-block mark counts, and a block's bitmap is cleared only when it is
walked. Mutator refcount writes remain (they are GNU R's too).

**Decision A.2, isolates: no process-global heap or interpreter.** One
*isolate* = one heap (`Core`), one interpreter, one set of API globals,
bound to one OS thread; R semantics (no shared mutable state between
workers) hold. A later track runs mirai-style workers as threads in one
process, each an isolate, sharing immutable data zero-copy without
serialization. So:

- all mutable state lives in an isolate record reached through one
  thread-local pointer; `libR.so` must not add process-global mutable
  state (the current `STATE` static becomes that record);
- exported data that differ per isolate (`R_GlobalEnv`, `R_BaseEnv`,
  `R_EmptyEnv`, `R_BaseNamespace`, `R_NamespaceRegistry`, `R_PPStack*`,
  `R_Visible`, `R_interrupts_*`, `R_CStack*`, `R_Srcref`, ...) are, in
  robigo's headers, macros over the isolate accessor (packages are
  recompiled, so this is allowed);
- truly immutable singletons (`R_NilValue`, `R_NaString`,
  `R_BlankString`, `R_TrueValue`/`R_FalseValue`/`R_LogicalNAValue`,
  `R_UnboundValue`, `R_MissingArg`, and the symbols) live in a shared,
  immortal, read-only segment created once per process; the symbol table
  is the one shared mutable structure (interning under a lock, symbols
  immutable once created; a symbol's value cell, used only by `base`,
  moves into the isolate);
- a *frozen* segment: objects made deeply immutable can be moved (copied
  once) into a shared read-only segment that every isolate may reference
  and that mori-style shm can map into other processes;
- per isolate, the council's rule stands: one owning thread, enforced in
  `collect()`;
- packages with process-global C state (C statics holding `SEXP`s or
  mutable data) cannot run in two isolates at once. A package opts in to
  isolates (a DESCRIPTION field, e.g. `Robigo-Isolates: yes`, or a
  registry robigo ships for audited packages); a worker that loads a
  package without the opt-in falls back to a process worker. `R_init_`
  runs once per isolate that loads the package.

**Measurements** (§9, (d)): per-task overhead of `mirai_map`, `mclapply`
on a large live heap (RSS growth from copy-on-write after a collection
in the child), and sharing a 1 GB vector with workers — each vs GNU R.

## 11. GC lessons to evaluate by measurement (lead, 2026-10-28)

1. Sticky mark bits (JavaScriptCore's Riptide, Julia): non-moving
   generational and concurrent collection without copying — the upgrade
   path behind the store()/barrier hook ((e)).
2. Ruby RGenGC's write-barrier-unprotected objects: packages write
   pointers through `DATAPTR`/`STRING_PTR_RO`-cast/`VECTOR_PTR` without a
   barrier; objects whose raw pointer was handed out are marked
   unprotected and always rescanned, so a generational collector stays
   correct.
3. Parallel marking on spare cores (Go, ZGC) for big heaps (12 cores
   here).
4. MMTk (Immix, StickyImmix, GenImmix, LXR; ported to Julia, Ruby, V8):
   a build-vs-adopt measurement on the bake-off and GC-at-scale workloads;
   robigo keeps its own heap only if it is at least as fast.
5. PyPy's cpyext (two heaps and a translation layer) confirms one heap.

## 12. Council review, 2026-10-28

GPT-6 Astra, Opus 5.5 and Fable 5.1 reviewed this document @ e3eeac8;
the synthesis and the reviews are in `docs/core-council.md`. The
direction was endorsed unanimously; the base-natives layout problem
they found is resolved by the lead's source-compatibility decision (§1,
§8). The 25 items of the synthesis are requirements. Status (updated as
they land; evidence in `docs/ledger/core.md`):

| # | item | status |
|---|---|---|
| 1 | base-package natives rebuilt from R 4.5.0 sources against robigo's headers + `Defn.h`; ~25 non-API internals | planned (§8) |
| 2 | immortals traced (symbols, mutable constants) | done |
| 3 | full context record (PPStackTop, vmax, gc_inhibit, red zone, R_Visible, depth, handlers, interrupts) | done (`errors::Ctx`), including the host's handler and restart stacks (found by the caught-error benchmark, ledger) |
| 4 | R_UnwindProtect: pop before cleanfun, token owns the condition; cpp11 + Rcpp in the gate | done: continues the unwind after cleanfun as GNU R does; the continuation owns a pending exit (HOST proxy) when cleanfun leaves by longjmp; packages in the gate |
| 5 | every export may jump (warnings, allocation, PROTECT overflow) | done (warnings return flow tokens and jump; tested with warn=2) |
| 6 | rooting rule + gctorture mode in CI | done (rule in §3; `ROBIGO_GCTORTURE`; CI step) |
| 7 | core bound to the evaluator thread; R_CStack*; C stack check | done (collect aborts off-thread; R_CheckStack ported) |
| 8 | one barrier-capable store() for heap stores; reserved header state | first step: every API store goes through `fix_refcnt` → `Core::write_barrier` (immortal roots today); generational use planned |
| 9 | C microbenchmarks vs GNU R's libR; GC-at-scale workloads | microbenchmarks done (`crates/robigo-capi/bench`, ledger); GC-at-scale planned |
| 10 | flat open-addressing CHARSXP table; ATTRIB(CHARSXP) = R_NilValue | done |
| 11 | the bridge is a thin scaffold with listed limits | §6, bridge.rs header |
| 12 | allocation failure is an R error with GNU R's messages | done for vectors beyond the small classes (collect, retry, then "cannot allocate vector of size ..."); small-object exhaustion still aborts |
| 13 | medium-object pages, LARGE bit, consistent accounting | medium classes up to 64,960 bytes share blocks (ledger); accounting review open |
| 14 | CE_NATIVE distinct from UTF-8; translateCharUTF8 rejects bytes; locale exports | planned (e) |
| 15 | runtime inside libR.so behind a thin `robigo` executable; version script, -Bsymbolic-functions, RTLD_NOW | executable exports exactly the API (dynamic list) for now; thin-executable shape needs ci.yml to ship libR.so (PR) |
| 16 | weak refs: classify before retaining; queued refs rooted, drained at safe points; release callbacks outside collect | done |
| 17 | no longjmp across Rust frames: C trampolines, C-unwind, catch_unwind | partly: C trampolines for every call into package code (`.Call`/`.External`/`.C`, R_init, finalizers, R_UnwindProtect/ExecWithCleanup/ToplevelExec bodies and cleanups); outward errors still jump from the exporting Rust frame (plain frames by construction) until the Result refactor; catch_unwind goes into the host callbacks with the bridge; see §5 on C-unwind |
| 18 | allocator: slots at class size; request-pair test; re-measure the bake-off | done (re-measure: memory track) |
| 19 | conservative scan resolves w-1 at granule boundaries | done (+ resolve checks the slot end) |
| 20 | 48-byte nodes with a 4th word; env body fixes only ENCLOS | planned (e) |
| 21 | port TRACKREFS exactly (no car refcount for BUILTIN/SPECIAL) | done |
| 22 | foreign/read-only buffers as an ALTREP class; no zero-copy EXTDATA bridge args | bridge copies arguments in (EXTDATA unused); ALTREP class planned |
| 23 | console output through C stdio; signal ownership | planned |
| 24 | G3-facing safe accessor layer spec (borrowed reads) | planned |
| 25 | start G3 once the accessor layer and the fastmap gate exist | noted |

## 13. What GNU R's methods.so needs from the core (handoff to G6)

GNU R's methods package natives (src/library/methods/src) build with
`scripts/build-natives.sh methods` and load. `.onLoad` then calls
`C_R_initMethodDispatch`, which installs methods' dispatch into the
evaluator. Its imports beyond robigo's implemented API are these 22
non-API entry points (all stubs in `crates/robigo-capi/src/stubs.rs`
today, raising "not implemented yet"). Each has GNU R's definition as
the spec:

| entry point | GNU R | what it does for methods |
|---|---|---|
| `R_set_standardGeneric_ptr` | objects.c | installs methods' `R_dispatchGeneric` as the evaluator's `standardGeneric` |
| `R_set_quick_method_check` | objects.c | installs `R_quick_method_check` (fast dispatch for primitives) |
| `R_primitive_methods`, `R_primitive_generic`, `R_set_prim_method`, `do_set_prim_method` | objects.c | the per-primitive method tables (`prim_methods`, `prim_generics`, `prim_mlist`): S4 methods on `[`, `+`, `length`, ... |
| `R_deferred_default_method` | objects.c | the marker for "call the internal default" |
| `R_execMethod` | eval.c | runs a method closure in a new frame that copies the generic's arguments (`.defined`, `.target`, `.Generic` bindings) |
| `R_do_slot`, `R_do_slot_assign`, `R_has_slot` | attrib.c | slots as attributes, `.Data`, `.S3Class`, the `\001NULL\001` pseudo-null |
| `R_do_new_object`, `R_do_MAKE_CLASS` | objects.c | `new()`'s C side (`prototype`, the S4 bit) and `getClass` |
| `R_data_class` | attrib.c | `class(x)` including implicit classes |
| `R_findVarLocInFrame`, `R_GetVarLocMISSING` | envir.c | binding cells: methods reads whether a formal is missing |
| `R_withCallingErrorHandler` | errors.c | a calling handler around a C callback |
| `R_subassign3_dflt` | subassign.c | the default of `$<-` |
| `DDVAL` | memory.c | `..1`-style symbols (a node bit) |
| `Rf_substitute`, `Rf_EncodeChar`, `getPRIMNAME` | coerce.c, printutils.c, names.c | helpers |

The evaluator side, beyond these functions:

- the **S4 bit**. robigo-core reserves GNU R's (`gp::S4`, bit 4) and the
  C API's `IS_S4_OBJECT`/`SET_S4_OBJECT` use it. The runtime's `Value`
  has none: `isS4()` is always FALSE and `setS4Object` is the identity,
  so the bridge drops the bit. A runtime flag (and OBJSXP values) is
  the first G6 change the core needs, carried across the bridge both
  ways.
- **dispatch hooks**. GNU R's `eval` calls `R_standardGeneric_ptr` for
  `standardGeneric()` and checks `R_has_methods(op)` before a
  primitive's internal code (`DispatchOrEval`, `DispatchGroup`,
  `DispatchAnyOrEval`). robigo's `try_dispatch`/`try_dispatch_group`
  are the places; the tables above are their data.

How the core answers G6: requests go in docs/ledger/core.md (a "G6
asks" entry). The C API functions above are robigo-capi's to implement
once the runtime has the S4 bit and the hooks; G6 owns the evaluator
side and the methods package.
