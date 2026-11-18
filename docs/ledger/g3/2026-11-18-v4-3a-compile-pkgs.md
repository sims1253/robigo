## 2026-11-18: V4 step 3a: R_compile_pkgs compiles closures as serialize writes them (draft)

GNU R's package build writes each closure's byte code into the lazy-load
database: under `_R_COMPILE_PKGS_=1` (lazycomp.mk, tools' Makefile),
`WriteItem` compiles every closure it writes (serialize.c:1040) by
`R_cmpfun1` (eval.c:1822), which calls the compiler package's
`tryCmpfun`. robigo-rt wrote no byte code there: A1's probe found 0 of
139 compiler functions and 0 of 850 tools functions compiled. L4's bar
(the databases byte-identical to the oracle's) cannot pass without it.
This is the build-time half of V4 step 3, and it does not wait for L5:
the compiler namespace only has to load in the build's environment.

### What landed

- `jit::cmpfun1`, a port of `R_cmpfun1`: `compiler:::tryCmpfun(fun)`
  evaluated in base. `R_Visible` is kept. When the result's body is not
  byte code, it runs a full collection (`R_gc()`, with its pending
  finalizers), as GNU R does. It reads no NOJIT or MAYBEJIT bits: only
  the JIT's `R_cmpfun` sets them.
- `WriteItem`'s check, first in the walk as in GNU R. The closure is
  compiled when:
  - `R_compile_pkgs` is set;
  - its body is not already a BCODESXP;
  - `R_disable_bytecode` is off;
  - it is not an S4 object of class `refMethodDef` or
    `defaultBindingFunction`.

  `R_compile_pkgs` is off while the compiled closure is written, so a
  closure in its environment is written as it is. It is set to TRUE after
  the write, and an error leaves it off, as GNU R leaves it.
- The compile is one of the writer's effect points (R code runs and may
  collect). Its result is kept, and a walk that starts over after a
  collection takes the kept result. `serialize()` sets `R_compile_pkgs`
  back to its value at the start of each walk, so a restart inside a
  compiled closure's subtree starts as the first walk did.
- serialize/mod.rs's module comment no longer lists the compile on write
  as not implemented. The stale comment in write.rs's walk is gone.

### The fixture

`tests/rt/serialize/compile-pkgs.R` (recorded on 4.6.2) serializes, under
`compilePKGS(TRUE)`, in versions 2 and 3:
- a plain closure;
- a closure whose environment holds another closure (that one is written
  uncompiled);
- a loop, and `...`;
- a list of closures;
- an already compiled closure;
- a closure the compiler declines (a `browser()` call);
- after `compilePKGS(FALSE)`, the same closure, uncompiled.

It also reads one back and runs it. robigo-rt's output equals GNU R's,
every byte, when `.AutoloadEnv` is stood in. It is not ratcheted:
- robigo-rt cannot load the compiler namespace from source on main. cmp.R's
  top-level `local()` calls `get()` on every name in base, and
  `.AutoloadEnv` is still an unrun base binding (L, INTERIM(L5)).
- The stand-in needs `unlockBinding`, which is #831 (B).
- `bytecode/cmpfun.R` and the other `bytecode/*` cases fail for the same
  reason.

### The probe bar

A1's probe (`makeLazyLoading("compiler")`, then tools' `makeLazyLoad.R` and
`makeLazyLoading("tools")`) was run with this step on A1's local probe
branch, with its stand-ins. It ran in a private copy of the kit, under
the build's environment from compiler's and tools' Makefile.in
(`_R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1 R_ENABLE_JIT=0 TZ=UTC
LC_ALL=C`). GNU R 4.6.2 read each database back, and the records were
compared with the oracle's databases, decompressed, after the serialize
header.

| database | functions compiled | variables identical | environments identical |
|---|---:|---:|---:|
| compiler | 139 of 139 (was 0) | 288 of 289 | 6 of 8 |
| tools | 850 of 850 (was 0) | 887 of 915 | 38 of 39 |

Every difference, by cause:

1. **Nine primitives have the wrong type in robigo-rt.** `@`, `@<-`,
   `Exec`, `Tailcall`, `call`, `declare`, `round`, `signif` and `~` are
   SPECIALSXP in GNU R and BUILTINSXP in robigo-rt. cmp.R's default inline
   handlers are chosen by `typeof()` (`cmpSpecial` or `cmpBuiltin`), so
   robigo-rt's cmp.R inlines a call that GNU R compiles as CALLSPECIAL.
   - compiler: `bcprof` (`round`) and `inlineHandlers` (the handler
     table itself).
   - tools, 12 closures:
     - `@`: `.check_code_usage_in_package`, `.get_S4_generics`,
       `.get_ref_classes`, `codocClasses`, `get_S4_generics_with_methods`,
       `undoc`;
     - `call`: `.check_package_code_assign_to_globalenv`,
       `as.alist.symbol`;
     - `~`: `checkObjAPI`;
     - `round`: `.check_packages`, `.runPackageTests`, `report_timeout`.
2. **The probe's `seq.int` stand-in**, a closure where GNU R has a
   builtin: 9 tools closures that call it, and `inlineHandlers`. These go
   when the stand-in does.
3. **The native pipe.** gram.c's `xxpipe` makes `lhs |> f(args)` as
   `lcons(fun, lcons(lhs, args))`, so the call's second cell is a
   LANGSXP. robigo-rt's parser makes a LISTSXP. This affects
   `rdfragment2text` and `rdfragment2toc`, whose code is otherwise
   identical.
4. **Where and how the build ran**, not the compile:
   - the namespace info's `path` (both databases);
   - five tools values made from the build tree: `.R_top_srcdir_default`,
     `.R_top_srcdir_file_path`, `system_ABI`, `so_symbol_names_table` and
     `charset_to_Unicode` (L4's build step).
5. **Not compared yet: the header and the compression.** robigo-rt writes
   the native encoding as UTF-8 under `LC_ALL=C`, because `l10n_info()`
   says UTF-8 is TRUE there. GNU R writes `ANSI_X3.4-1968`. So no
   compressed record is byte-identical yet, and whether robigo-rt's zlib
   output equals GNU R's for the same payload is still open.

With the JIT on (the probe's own setting), robigo-rt's JIT also left
`NOJIT` on closures that the build had called (tools' vignette engine
registry). GNU R's build runs with `R_ENABLE_JIT=0`, and with it those
records are identical.

### Verification

To do before filing: full `scripts/rt-ci.py`, perf-rt, lint, Sol.
