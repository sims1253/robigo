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
- **No stand-ins**: B's #935 (`as.POSIXlt`) and #947 (`format.POSIXlt`)
  give the three steps that write `Built:` what they need, so the tree
  is built with nothing standing in. (The INTERIM stand-ins this PR first
  carried, with their self-expiring check, are deleted with them.)
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
  R_init_jit_enabled; ours inside `load_compiler_namespace`; with main
  347d462c3, after #930's cheaper translation; before it, 69.0M with the
  translation 34.5M):

  | part | robigo-rt | GNU R |
  |---|---:|---:|
  | the load | 55.1M | 34.3M |
  | records fetched (lazyLoadDBfetch) | 25.2M (128) | 28.4M (129) |
  | – unserialize | 21.6M (133) | 26.7M (133) |
  | – inflate | 7.2M (9,445 calls) | 3.8M (R_decompress1; 9,309 inflate calls) |
  | closures translated at first call | 21.4M (155, 138k each) | none (byte code run as read) |
  | the rest (loadNamespace's R code, the hooks) | ~8.6M | ~5.9M |

  The start: JIT on 121.4M, 15.8 MiB (GNU R 88.9M, 20.7 MiB); JIT off
  64.9M, 15.0 MiB (GNU R 54.3M). Peak RSS the median of 5.
- `volume`'s compiler-loading cases stay out of the torture steps: the
  conformance suite runs M1's loader, which still makes the compiler from
  source, so their cost is the 599-601 s of #933. They move back when the
  suite runs on GNU R's boot (the interim loader's deletion).
- perf rows, the rdb boot against M1's loader (`scripts/perf.py
  rt-bars`, callgrind instructions per iteration, the same binary run
  under each, at the gated head; the M1 loader as the base):

  | row | M1 loader | rdb boot | change |
  |---|---:|---:|---:|
  | binarytrees | 3,499,269.0 | 3,497,740.1 | -0.04% |
  | d0-attr | 1,975.2 | 1,975.3 | +0.01% |
  | d0-attributes | 2,244.8 | 2,244.7 | -0.00% |
  | d0-names | 4,744.4 | 4,744.3 | -0.00% |
  | d0-paste | 13,875.4 | 13,819.5 | -0.40% |
  | d0-paste0 | 12,797.0 | 12,740.8 | -0.44% |
  | d0-unlist | 11,064.9 | 11,081.7 | +0.15% |
  | defaults | 1,981.9 | 1,982.0 | +0.00% |
  | e7-match | 91,764.9 | 96,513.1 | +5.17% |
  | e7-paste | 95,707.9 | 95,579.7 | -0.13% |
  | e7-unique | 117,847.6 | 118,550.0 | +0.60% |
  | env-fn | 1,934.9 | 1,935.4 | +0.02% |
  | fib | 10,711,747.5 | 10,711,334.4 | -0.00% |
  | grow-dbl | 5,535.2 | 5,535.2 | +0.00% |
  | guard-dollar-assign | 3,852.1 | 3,852.1 | -0.00% |
  | guard-dots | 8,688.4 | 8,682.2 | -0.07% |
  | guard-repeat | 8,257.9 | 8,257.5 | -0.00% |
  | guard-subassign-dbl | 4,452.1 | 4,452.3 | +0.00% |
  | guard-subset-dbl | 1,733.3 | 1,733.2 | -0.00% |
  | guard-subset-matrix | 3,190.0 | 3,190.1 | +0.00% |
  | guard-vec-arith | 88,185.8 | 89,287.7 | +1.25% |
  | guard-vec-sum | 19,874.6 | 19,874.8 | +0.00% |
  | guard-while | 8,126.2 | 8,125.8 | -0.00% |
  | named | 1,839.4 | 1,839.9 | +0.03% |
  | plain | 1,897.7 | 1,897.9 | +0.01% |
  | s3 | 16,383.6 | 17,244.3 | +5.25% |
  | scalars | 6,044.0 | 6,044.7 | +0.01% |
  | superassign | 4,599.3 | 4,579.1 | -0.44% |

  | program | M1 loader peak RSS KiB | rdb boot | change |
  |---|---:|---:|---:|
  | benchmarks/rt/mem/churn.R | 80,256 | 86,164 | +7.4% |
  | benchmarks/rt/mem/listgrow.R | 46,780 | 52,296 | +11.8% |
  | benchmarks/suite/binarytrees.R | 26,388 | 28,620 | +8.5% |

  Above 0.5%: `s3` (+5.25%), `e7-match` (+5.17%), `guard-vec-arith`
  (+1.25%), `e7-unique` (+0.60%); peak RSS +7-12% (the compiler loaded
  at start). Not analysed here: the bar for deleting the interim loader
  (at or below it, like for like) is that PR's; X and C1 have the
  breakdowns above.
- **What built a tree** (the 33 must not recur unexplained): robigo-rt
  carries the commit it was built from (robigo-rtapi's build.rs, the
  marker `robigo-rt-commit:<sha>`, read by `scripts/rt-builder.sh`; an edit
  not committed shows in the sha256 only). `scripts/rt-home-db.sh` and
  `scripts/rt-base-build.sh` take `--robigo-rt BIN` only (no default
  target's binary) and record it in the tree (`.rt-home-db-builder`,
  `OUT/builder`: path, sha256, GNU build ID, commit);
  `scripts/rt-rdb-compare.sh` prints it in its report and refuses a tree
  that does not record it.
- #933's five late script lows (deferred here): rt-home-db.sh holds a
  shared lock on rt-home.sh's tree while it reads it (a rebuild takes it
  exclusively) and refuses rt-home.sh's own tree as its DIR (the same lock
  file would deadlock); rt-home.sh's path normalization cannot leave it
  empty; Debian's `R_LIBS_SITE` line is one string, written and checked;
  `Parser::error_frontier`'s doc names the private `codegen` without a
  link.
- **L4: GNU R's own build, run by robigo-rt, no stand-ins** (with B's
  #935 and #947; `scripts/rt-base-build.sh --robigo-rt BIN`, whose R now
  boots GNU R's way, `ROBIGO_RT_BOOT=stage0`, not M1's loader): all ten
  steps of src/library/Makefile.in's R target run to the end (profile,
  translations, base, tools and compiler `all`, base and tools
  `mklazycomp`, base `mkdesc`, vdesc, vns), 51 s. Its databases against
  the oracle's (`scripts/rt-rdb-compare.sh`, which now maps both spellings
  of the walk's tree, its build tree and the install link to it):

  | database | records | the same bytes | the same but for build paths | other |
  |---|---:|---:|---:|---:|
  | base.rdb | 1189 | 1186 | 3 | 0 |
  | compiler.rdb | 297 | 296 | 1 | 0 |
  | tools.rdb | 954 | 951 | 3 | 0 |
  | tools' sysdata.rdb | 3 | 3 | 0 | 0 |

  The database tree (`scripts/rt-home-db.sh`, 23 s): base.rdb 1186 + 3,
  compiler.rdb 296 + 1, the same classes.
