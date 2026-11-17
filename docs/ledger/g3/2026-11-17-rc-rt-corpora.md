## 2026-11-17: RC, robigo-rt's corpus column and what blocks its failing cases

The lead adopted the early-wide-boot proposal (BW and RC; the foreman's
review, 2026-11-17). RC gives robigo-rt a column in the test corpora,
against GNU R 4.6.2's recordings, with ratchets of its own, and ranks what
blocks the failing cases. This entry is RC's first step, the parts that do
not wait for BW or for robigo-rt's `-f` front end.

### What changes

- **`scripts/corpus.py run/show --runtime rt`** runs robigo-rt (`--robigo`,
  else `$ROBIGO_RT_BIN`, else the release build) against the recordings of
  its own pin (`rt` in `scripts/oracle-versions`).
  - It has its own ratchet, `tests/corpora/<corpus>/passing-rt/`.
  - Each case gets `ROBIGO_RSRC` from `scripts/r-src.sh`, because cases run
    with `HOME` set to their own directory.
  - **It refuses, saying why,** what robigo-rt cannot run yet:
    - corpora in echo mode, since robigo-rt has no `-f` front end;
    - corpora with a build step (capi, cran-purer);
    - `--tier`, which selects the old runtime's execution tiers.
- **The burn-down.** Each failing case records the first missing thing its
  stderr names (`blocked`):
  - a primitive or `.Internal` stub (`crates/robigo-rt/src/base.rs`);
  - another "robigo: ... is not implemented yet";
  - a function base does not bind ("unbound").

  The run prints them ranked by the cases each blocks, and the results JSON
  keeps the whole table.
- **Score and absorb:**
  - `corpus.py score` prints robigo-rt's column next to the old runtime's;
  - `corpus.py absorb` files each runtime's passes in its own ratchet,
    against its own pin. Before this, rt results run against their own pin
    carried no `r_version`, so absorb would have put them into the old
    runtime's `passing/`.
- **`scripts/ratchet.py check`** covers the `passing-rt/` ratchets.
- **CI: `rt-corpora`.** On every rt pull request it runs fastr on robigo-rt
  (the binary `perf-rt-build` builds) against `passing-rt/`:
  - a pull request runs one quarter of fastr, the quarter rotating with its
    number;
  - a push to main runs all of it, about two minutes;
  - `Check` needs the job, so it gates through the ratchet only.
  - The foreman asked for a nightly run. A run on every push to main costs
    about as little and catches a regression at the merge that caused it.

### fastr's column

robigo-rt at main `fb429442` (release, `-p robigo-rtapi`; sha256
`237f49bbcaff34b859b1ee92645bc80ee1d4c9ff6e2129008a20ca90d487f95a`, GNU
build ID `410ea73950da65f1e0cf41df2e18c3ea18402032`) passes **18,812 of
fastr's 31,374 cases** (60.0%) against GNU R 4.6.2's recording.
- **How it runs:** in 65 s on 6 jobs. Two runs give identical pass sets, and
  the 18,812 are ratcheted in `tests/corpora/fastr/passing-rt/`.
- **Why it works before BW:**
  - fastr's configured recording (`expected-methods.jsonl.gz`) already
    exists for 4.6.2;
  - robigo-rt loads only base, whatever `R_DEFAULT_PACKAGES` says. A case
    that needs nothing else can match GNU R byte for byte. One that needs
    stats or utils fails, as it should.
- **The markers stay.** Markers are never removed, so BW and L5 must keep
  every one of them. A marker is a byte-exact match with GNU R 4.6.2, so a
  later failure is a real regression.

### What blocks the failing cases

Of the 12,562 failing cases, 10,478 name a missing thing, 461 distinct ones:
- 7,592 are unbound names, which BW will bind;
- 1,295 are primitive stubs and 177 `.Internal` stubs;
- 1,414 are other "not implemented yet" paths.

The other 2,084 differ in output without naming one. The top 50, by cases
blocked (`.robigo/results/fastr.rt.json` of the run above):

| # | cases | first missing thing |
|---:|---:|---|
| 1 | 1,299 | `unbound toString` |
| 2 | 1,239 | `primitive ceiling` |
| 3 | 473 | not implemented: printing a matrix (printMatrix) |
| 4 | 381 | `unbound set.seed` |
| 5 | 360 | `unbound intersect` |
| 6 | 324 | not implemented: options(keep.source = ) |
| 7 | 203 | `unbound deriv` |
| 8 | 199 | `unbound crossprod` |
| 9 | 147 | `unbound array` |
| 10 | 136 | not implemented: '[<-' with matrix or array subscripts |
| 11 | 134 | not implemented: loadNamespace |
| 12 | 118 | `unbound log` |
| 13 | 109 | `unbound data.frame` |
| 14 | 103 | `unbound cbind` |
| 15 | 92 | `unbound %*%` |
| 16 | 84 | `unbound expression` |
| 17 | 82 | `unbound rbind` |
| 18 | 79 | `unbound qbeta` |
| 19 | 69 | `unbound pbeta` |
| 20 | 64 | `unbound round` |
| 21 | 64 | `unbound setClass` |
| 22 | 64 | `unbound textConnection` |
| 23 | 58 | `unbound parse` |
| 24 | 57 | `unbound type.convert` |
| 25 | 54 | not implemented: '[' with array subscripts |
| 26 | 54 | `unbound unserialize` |
| 27 | 53 | `unbound range` |
| 28 | 49 | not implemented: printing an array-like list |
| 29 | 47 | `unbound dbeta` |
| 30 | 47 | `unbound substr<-` |
| 31 | 46 | not implemented: printing an array (printArray) |
| 32 | 46 | `unbound strsplit` |
| 33 | 44 | `unbound file` |
| 34 | 44 | `unbound qgamma` |
| 35 | 44 | `unbound qhyper` |
| 36 | 44 | `unbound source` |
| 37 | 43 | `primitive call` |
| 38 | 43 | `unbound qbinom` |
| 39 | 42 | `.Internal sub` |
| 40 | 42 | `unbound qnbinom` |
| 41 | 42 | `unbound sapply` |
| 42 | 41 | `unbound .External2` |
| 43 | 40 | `unbound seq.int` |
| 44 | 39 | `.Internal grep` |
| 45 | 39 | `unbound phyper` |
| 46 | 39 | `unbound rev` |
| 47 | 38 | `.Internal regexpr` |
| 48 | 38 | `unbound rawShift` |
| 49 | 37 | `unbound double` |
| 50 | 37 | `unbound model.frame` |

**How to read it:**
- The unbound names are base closures outside robigo-rt's binding set
  until BW.
- The stubs and the "not implemented" paths order B5's ports by cases
  unblocked, and they cross-check D0c's count.
- A case counts only once, under the first thing it hit, so the counts are
  lower bounds of what each port unblocks.

### The other corpora

- **Echo mode (gnur-tests, gnur-examples, gnur-demos, r-intro).** Their
  4.6.2 recordings exist, but they run as `R -f FILE`: `R_ReplConsole` and
  `Rf_ReplIteration` over the file, with the echoing console reader
  (`src/unix/sys-std.c:994-1001`).
  - RC's next step adds `-f`, `--quiet` and `--no-save` to robigo-rt's CLI.
    It also adds that reader's prompt and echoed-line rules to robigo-rt's
    console loop, through L2a's `repl_eval_toplevel`.
  - It is based on main once L2a merges.
- **cran-purer** has a build step (its packages' library), and robigo-rt
  loads no packages before M5.
- **capi cannot be recorded on 4.6.2 yet.** 34 of its 77 C sources do not
  compile against 4.6.2's headers, because they use entry points 4.6 no
  longer declares. `build.sh` compiles every source under `set -e`, so the
  build stops at the first failure.
  - The recording and the rewrite of those 34 sources for 4.6's API move
    to M5. M5 starts from this list: each source and its first missing
    entry point, as `R CMD SHLIB` in the 4.6.2 oracle image reports it
    (gcc 14.2).
  - By entry point: `Rf_findVarInFrame` (13); `ATTRIB` (4); `Rf_allocSExp` (3); `Rf_findVar` (3); `PRVALUE` (2); `R_nchar` (2); `Rf_allocVector3` (1); `OBJECT` (1); `SET_S4_OBJECT` (1); `SET_TYPEOF` (1); `IS_S4_OBJECT` (1); `SETLENGTH` (1); `LENGTH_EX` (1).

| source (`tests/corpora/capi/support/`) | first missing entry point |
|---|---|
| `alloc.c` | `Rf_allocVector3` |
| `altrep_lazy_string.c` | `ATTRIB` |
| `attr_sync.c` | `OBJECT` |
| `attributes.c` | `ATTRIB` |
| `c_env_bindings.c` | `Rf_findVarInFrame` |
| `closure.c` | `Rf_allocSExp` |
| `const_args.c` | `PRVALUE` |
| `cpromise.c` | `Rf_allocSExp` |
| `cpromise_altrep.c` | `Rf_allocSExp` |
| `default_args.c` | `PRVALUE` |
| `dots.c` | `Rf_findVarInFrame` |
| `dots_write.c` | `Rf_findVarInFrame` |
| `encodings.c` | `R_nchar` |
| `envattr_eval.c` | `SET_S4_OBJECT` |
| `envir.c` | `Rf_findVar` |
| `identity_vm.c` | `Rf_findVar` |
| `lookup_identity.c` | `Rf_findVarInFrame` |
| `lookup_loop.c` | `Rf_findVarInFrame` |
| `lookup_poke.c` | `Rf_findVarInFrame` |
| `lookup_static.c` | `Rf_findVarInFrame` |
| `mask_cached_ptr.c` | `Rf_findVarInFrame` |
| `missing_marker.c` | `Rf_findVar` |
| `objects.c` | `Rf_findVarInFrame` |
| `pairlist.c` | `SET_TYPEOF` |
| `promise.c` | `Rf_findVarInFrame` |
| `proxy_attr_write.c` | `ATTRIB` |
| `proxy_statics.c` | `ATTRIB` |
| `s4bit.c` | `IS_S4_OBJECT` |
| `statics.c` | `Rf_findVarInFrame` |
| `strings.c` | `R_nchar` |
| `symbol_globals.c` | `Rf_findVarInFrame` |
| `twin_raw_writes.c` | `SETLENGTH` |
| `vectors.c` | `LENGTH_EX` |
| `zerocopy.c` | `Rf_findVarInFrame` |

- **Not recorded, as the foreman decided:**
  - the package suites (M5);
  - fastr's older methods-free `expected.jsonl.gz`, which no configuration
    reads.
