## 2026-11-21: scripts/rt-rdb-compare.sh, robigo-rt's databases against GNU R's (L4)

- What: the base build's lazy-load databases (scripts/rt-base-build.sh's
  OUT) compared with the oracle image's, run by GNU R 4.6.2 in that image
  (`scripts/rt-rdb-compare.sh [OUT] [PKG/NAME ...]`, the work in
  `scripts/rt-rdb-compare.R`). Not in CI. Per database:
  - the `.rdx` maps: the compression, and the variables' and references'
    keys;
  - each record by GNU R's key, decompressed as `lazyLoadDBfetch` does
    (R_decompress1/2/3), byte for byte;
  - each differing record unserialized on both sides (a reference to
    another record kept as its key) and classed: the same value,
    byte-code only, build paths only (our source and install trees mapped
    to GNU R's in-place build tree), closures' environments only
    (`scripts/lazyload-ck.R`'s `nz`), different, or unreadable by GNU R
    (counted by the reason).
- First run: main a8e08fdd9 + #860 (9e051ff73), the stand-ins of
  `~/.cache/robigo-research/l/shim-main-2026-11-21.R` (split, Sys.time,
  iconv with an INTERIM sub = "byte"):
  - **tools/sysdata**: 3 of 3 records the same bytes.
  - **tools/tools**: the maps agree (915 variables, 39 references).
    Records: 94 the same bytes, 853 the same value, 2 build paths only,
    4 closures' environments only, 1 different. The 853, and the
    remaining one (`env::1`, the namespace's info), come from one
    difference: the namespace reference holds `c("tools", <R.version's 13
    strings>)` where GNU R's holds `c("tools", "4.6.2")`. tools' mkdesc
    stops in `as.POSIXlt` (B), so `Meta/package.rds` is missing when
    mklazycomp runs. `loadNamespace` then never sets `version`, and
    `makeNamespace(name, version = version)` finds base's `version`
    (`R.version`). GNU R does the same without that file. Expected to
    become the same bytes once mkdesc runs.
  - **base/base**: 29 of 1189 records the same bytes; 1148 records are
    promises (GNU R refuses them: "cannot return a promise"). robigo-rt's
    INTERIM(L5) wide loader binds base lazily, so base's variables are
    still unforced when makebasedb.R writes them. GNU R's build sources
    base in full first. Also: 10 extra variables (robigo-rt's `.C_`/`.F_`
    bindings in base: `.C_R_addTaskCallback`, `.F_dqrcf`, ...), 397
    references against GNU R's 10, `.popath` (the translations path), and
    the three stand-ins (`Sys.time`, `iconv`, `split.default`). This is
    L5's work: base's boot as GNU R does it (the base source file at
    build time, base.rdb after).
