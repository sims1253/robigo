## 2026-11-18: V4 step 3a: R_compile_pkgs compiles closures as serialize writes them

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

With the JIT on (the probe's default), the build's records also differed
in closures that the build had called, in tools' vignette engine
registry: they carried `NOJIT`. GNU R's build runs with `R_ENABLE_JIT=0`,
and with it those records are identical. This is not a robigo-rt
difference. With the JIT on, GNU R sets the same bits after one call, on
the oracle and on robigo-rt alike: `NOJIT` (32) on a small closure and on
one in a local environment, `MAYBEJIT` (64) on a loop.

Owners of the causes outside this step, as the foreman assigned them:
- cause 1: E (the audit of every names.c primitive's `typeof()`; `@`,
  `@<-`, `~`, `Exec`, `Tailcall`, `declare`) and B (`call`, `round`,
  `signif`);
- cause 2: the trial's `seq.int` PR;
- cause 3: E;
- cause 4: L4's build step;
- the header's native encoding: L (the start-up locale).

Compression is not part of the bar: L4 compares records decompressed.

**The count to beat next time:**
- compiler: 288 of 289 variables and 6 of 8 environments identical after
  the header;
- tools: 887 of 915 variables and 38 of 39 environments.

Once causes 1 and 3 and the `seq.int` stand-in are fixed, the only
differences left should be cause 4's.

### How to run it again

The kit and the scripts are in `~/.cache/robigo-research/c1/probe-3a/`
(README.md). It is a private copy of A1's kit with two changes:
- `setup.sh` takes a prebuilt binary (`PROBE_PREBUILT=BIN`);
- `run.sh` sets `R_HOME` to the kit's tree.

The binary is this step's commits cherry-picked onto A1's local
`g3/a1-a5-probe`, built in release.

```
step2-run.sh WORKTREE BIN   # makeLazyLoading("compiler"), plain, then compiled
step3-run.sh WORKTREE BIN   # tools' makeLazyLoad.R + makeLazyLoading("tools"), compiled
# both run under R_ENABLE_JIT=0 TZ=UTC LC_ALL=C (the Makefile.in's R_EXE);
# the compiled runs add _R_COMPILE_PKGS_=1 R_COMPILER_SUPPRESS_ALL=1
docker run --rm -u "$(id -u):$(id -g)" -v "$K:$K" -e LC_ALL=C.UTF-8 robigo-oracle:4.6.2 \
  Rscript $K/z2.R $K/compiler-R-compiled/compiler $K/ora/compiler   # tools likewise
```

`z2.R` decompresses each record and compares the bytes after the
serialize header. The other scripts locate a difference: `why.R` (the
first differing op or constant), `use.R` (the primitives a closure
calls), `env*.R` and `nf.R` (environment and data records).

### Review

Sol (codex `gpt-6.1-sol`, high), round 1 on the whole step: APPROVE, no
findings. The round checked the restart replay, that a closure inside a
compiled closure's subtree is not compiled, the rooting around the
effect point, R_Visible, `R_compile_pkgs` as NA_LOGICAL (C reads it as
true), and the flag's restore.

### Verification

On main `c3a685b3e`:
- perf-rt (`scripts/perf.py rt-bars --main`, against main's binary): flat,
  every row within +0.03% (named +0.03%, guard-subset-matrix +0.03%), peak
  RSS within 2%.
- `scripts/rt-ci.py` stopped at conformance, 552/560. The 8 failing are:
  - the 5 `bytecode/*` cases and this step's `serialize/compile-pkgs.R`,
    all unratcheted, which wait for the compiler namespace;
  - `dynload/library-dynam-tools.R`, unratcheted;
  - `datetime/strptime.R`, ratcheted. It prints today's date, recorded on
    the 17th; #841 fixes it.

- The full `scripts/rt-ci.py`, on a local merge with #841 (not pushed):
  exit 0. Conformance 553/560; the 7 failing are the unratcheted ones
  above. Every mode passes:
  - `NO_INPLACE`, `MATERIALIZE`;
  - gctorture, and `GC_MOVE` with gctorture;
  - `FRAME_QUARANTINE`, `GC_PIN_REPORT`;
  - the stress script, overshoot, the small vector limit;
  - the byte-code crosscheck;
  - injected allocation failures (2800 runs, 0 differences);
  - retry torture.
- Miri: not run. The step changes neither robigo-rtcore nor robigo-heap.
- `cargo fmt`, `scripts/lint.sh`: clean.
