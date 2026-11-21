## 2026-11-21: the compiler installed and loaded at the end of the start (L5)

On GNU R's boot (`ROBIGO_RT_BOOT=stage0`/`rdb`); M1's loader is unchanged.
Contains #933's head (bef8b1644) and #925's (6d33c8b93, `Sys.time`,
`Sys.info`, `Sys.glob`).

- **The tree** (scripts/rt-home-db.sh) is GNU R's R target's steps that
  install base and the compiler, in its order, each run by the robigo-rt
  that boots from the tree with its Makefile's R_EXE (the boot the tree's
  base: the source until base.rdb is written, then the database):
  translations-all (the catalogues, no R: the Makefile has no R_EXE),
  base's and tools' mkdesc2, tools' description installed by R
  (`R_ENABLE_JIT=0 TZ=UTC`), compiler's mkdesc and mklazycomp
  (`tools:::makeLazyLoading("compiler")`, `_R_COMPILE_PKGS_=1`,
  `R_ENABLE_JIT=0`), base's database rule, base's mkdesc, then vdesc and
  vns (Meta/package.rds, Meta/nsInfo.rds). Not run: tools-mklazycomp
  (tools stays source) and the bibliographies and dictionaries (utils).
  36 s here.
- The build runs with an empty HOME and no `R_LIBS*`, as the oracle
  image's build: the system profile's `.libPaths()` (whose environment
  base.rdb holds) added this machine's `~/R/x86_64-pc-linux-gnu-library`.
- The tree records where it was built (`.rt-home-db-built-at`), and
  `scripts/rt-rdb-compare.sh` takes the tree and maps that path to GNU R's
  build tree (E's `path` finding): a database records its build path in
  GNU R too (`.Library`, a namespace's `path`).
- **INTERIM(B, datetime.c)**: the three steps that write `Built:`
  (`format(Sys.time(), ..., tz = "UTC", usetz = TRUE)`) first read
  `scripts/rt-home-db-interim.R`, E's UTC stand-ins for
  `as.POSIXlt.POSIXct` and `format.POSIXlt`; the steps that write
  compiler.rdb and base.rdb do not. The build stops, saying to delete the
  file, once the robigo-rt it runs implements `.Internal(as.POSIXlt())`.
- **The JIT** (crates/robigo-rt/src/jit.rs): off while R starts
  (`at_startup`: `R_jit_enabled`, `R_compile_pkgs`, `R_disable_bytecode`
  0, Defn.h's "has to be 0 during R startup"; the strategy and minimum
  score as R_CheckJIT's first call reads them, at base's first closure
  call with the JIT off). `R_init_jit_enabled` (`init_jit_enabled`) is
  setup_Rmainloop's last step (`base::init_jit`, its own SETJMP; an error
  that check_session_exit lets pass is `R_Suicide("unable to initialize
  the JIT\n")`, status 2): `.ArgsEnv` forced, `R_ENABLE_JIT` read and,
  when not 0, the compiler's namespace loaded and `checkCompilerOptions`
  called; then `_R_COMPILE_PKGS_` and `R_DISABLE_BYTECODE`.
- **The compiler's namespace** comes from base's `loadNamespace` on GNU
  R's boot (lazy-loaded from compiler.rdb); `ns::load_compiler` (from
  source) is M1's only, kept for C1.
- Against the oracle (tests/boot_db.rs): `loadedNamespaces()` at start is
  `compiler base` (`base` with `R_ENABLE_JIT=0`), the compiler's closures
  byte code, `enableJIT(-1)` 3 (1, 0 as set), `compilePKGS` TRUE with
  `_R_COMPILE_PKGS_=1`, `enableJIT(-1)` 0 in a profile. The system profile
  runs to its end (#925): the boot's stderr is empty.
- Waiting on C1's #931 (V4 step 3): a closure the JIT compiled is not
  byte code to `bodyCode` yet (M1 the same). Then the test adds GNU R's
  cases, kept in its comment, among them the strategy's default read
  with the JIT off: `R_ENABLE_JIT=1` compiles a small top-level closure
  at its second call (TOP_SMALL_MAYBE, not NO_SMALL).
- **The databases against the oracle's** (scripts/rt-rdb-compare.sh on
  the tree): base.rdb 1186 of 1189 records the same bytes, the other 3
  the same but for build paths (`.Library`, `.popath` now that
  library/translations is there, env::6's `.lib.loc`); compiler.rdb 296
  of 297 the same bytes, the other the same but for the build path (the
  namespace's `path`). None differs in byte code.
- **The 34 byte-code-only records of #922's run** (the council's item):
  - `Sys.timezone`: a fidelity failure in what the compiler calls on
    robigo-rt. names.rs listed names.c's `#ifdef Win32` rows, so
    `is.builtin.internal(as.name("tzone_name"))` was TRUE and cmp.R
    inlined `.Internal(tzone_name())` (code 1137 words against 1135,
    first differing at word 101). Fixed in #933.
  - The other 33 do not reproduce. With #922's commit (b523ae31d, the
    ledger's "main 459478a2f + #860 + #916"): its rt-home-db.sh (three
    runs) and the Makefile walk (`rt-base-build.sh` stage 0 with the
    stand-ins), each compared by every version of rt-rdb-compare.R
    (dd54b5732, 452831557, 30f4aa4f1, e945cb733) against the same oracle
    image (built 02:37, before the 06:53 run), give 1 byte-code-only record
    (`Sys.timezone`) and the rest the same bytes. The run's keys were not
    kept. Most likely it read another binary: rt-home-db.sh's default is
    cargo's release binary, in a target directory other builds write.
    rt-rdb-compare.R now names each byte-code-only record and where its
    code first differs (`bytecode: KEY: ...`), so the next run says.
- **The compiler's load at start** (callgrind, `--vanilla -e 1`,
  `R_DEFAULT_PACKAGES=NULL`; GNU R in the oracle, collected inside
  R_init_jit_enabled; ours inside `load_compiler_namespace`):

  | part | robigo-rt | GNU R |
  |---|---:|---:|
  | the load | 69.0M | 34.3M |
  | records fetched (lazyLoadDBfetch) | 25.5M (128) | 28.4M (129) |
  | – unserialize | 22.0M (133) | 26.7M (133) |
  | – inflate | 7.2M (9,445 calls) | 3.8M (R_decompress1; 9,309 inflate calls) |
  | closures translated at first call | 34.5M (155, 223k each) | none (byte code run as read) |
  | the rest (loadNamespace's R code, the hooks) | ~9.0M | ~5.9M |

  The start: JIT on 145.4M, 15.9 MiB (GNU R 88.9M, 20.7 MiB); JIT off
  74.4M, 13.7 MiB (GNU R 54.3M).
- `volume`'s compiler-loading cases stay out of the torture steps: the
  conformance suite runs M1's loader, which still makes the compiler from
  source, so their cost is the 599-601 s of #933. They move back when the
  suite runs on GNU R's boot (the interim loader's deletion).
- perf rows, the rdb boot against M1's loader (`scripts/perf.py rt-bars`, callgrind instructions per iteration, the
  same binary run under each; the M1 loader as the base):

  | row | M1 loader | rdb boot | change |
  |---|---:|---:|---:|
  | binarytrees | 3,499,264.1 | 3,497,589.5 | -0.05% |
  | d0-attr | 1,975.9 | 1,978.9 | +0.15% |
  | d0-attributes | 2,245.2 | 2,244.4 | -0.03% |
  | d0-names | 4,798.7 | 4,801.0 | +0.05% |
  | d0-paste | 13,877.3 | 13,820.7 | -0.41% |
  | d0-paste0 | 12,798.6 | 12,754.6 | -0.34% |
  | d0-unlist | 11,064.3 | 11,086.6 | +0.20% |
  | defaults | 1,981.7 | 1,976.8 | -0.25% |
  | e7-match | 91,804.7 | 96,434.1 | +5.04% |
  | e7-paste | 95,708.2 | 94,527.8 | -1.23% |
  | e7-unique | 117,847.8 | 118,074.2 | +0.19% |
  | env-fn | 1,935.3 | 1,938.0 | +0.14% |
  | fib | 10,711,701.3 | 10,715,232.2 | +0.03% |
  | grow-dbl | 5,535.2 | 5,530.3 | -0.09% |
  | guard-dollar-assign | 3,852.1 | 3,848.7 | -0.09% |
  | guard-dots | 8,688.3 | 8,676.4 | -0.14% |
  | guard-repeat | 8,258.0 | 8,260.9 | +0.03% |
  | guard-subassign-dbl | 4,452.1 | 4,438.5 | -0.31% |
  | guard-subset-dbl | 1,733.8 | 1,714.9 | -1.09% |
  | guard-subset-matrix | 3,189.9 | 3,203.4 | +0.42% |
  | guard-vec-arith | 88,182.1 | 89,308.5 | +1.28% |
  | guard-vec-sum | 19,875.1 | 19,880.4 | +0.03% |
  | guard-while | 8,126.5 | 8,120.5 | -0.07% |
  | named | 1,840.2 | 1,840.3 | +0.00% |
  | plain | 1,897.5 | 1,879.9 | -0.93% |
  | s3 | 16,394.8 | 17,257.9 | +5.26% |
  | scalars | 6,044.7 | 6,056.5 | +0.20% |
  | superassign | 4,589.8 | 4,573.8 | -0.35% |

  | program | M1 loader peak RSS KiB | rdb boot | change |
  |---|---:|---:|---:|
  | benchmarks/rt/mem/churn.R | 80,040 | 86,228 | +7.7% |
  | benchmarks/rt/mem/listgrow.R | 46,648 | 52,548 | +12.6% |
  | benchmarks/suite/binarytrees.R | 26,364 | 28,760 | +9.1% |

  Above 0.5%: `s3` (+5.26%), `e7-match` (+5.04%), `guard-vec-arith`
  (+1.28%); peak RSS +8-13% (the compiler loaded at start, which M1's
  loader does at its first use). Not analysed here: the bar for
  deleting the interim loader (at or below it, like for like) is that
  PR's; X and C1 have the breakdowns above.
