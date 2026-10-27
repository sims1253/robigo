# robigo: workstreams

robigo is built by many agents in parallel, each hill-climbing one part
against a metric. This file defines the parts (tracks), what each owns,
how it is measured, and the protocol every agent follows. Read
`AGENTS.md`, `docs/architecture.md` and `docs/verification.md` first.

## Protocol

1. **Start** from `main` in your own worktree, on a branch
   `track/<track>-<topic>`. Read your track below and its ledger
   (`docs/ledger/<track>.md`; create it if missing).
2. **Measure first.** Record your metric before changing anything.
3. **Loop:** pick the most valuable failing case or slowest benchmark in
   your scope; find the GNU R C code that defines it; change; build
   (`scripts/build.sh`); verify. Keep the change only if your metric
   improves and nothing regresses: every ratchet holds, `cargo test` of
   the crates you touched passes, `scripts/perf.py compare` shows no
   slowdown above 0.5% where you touched hot code. Otherwise revert.
4. **Log** every experiment in your ledger, kept or not: what, why, the
   measured result. Rejected ideas are results too.
5. **Ship** small: one topic per pull request, commits that explain why,
   the metric before and after in the description. Run `cargo fmt`. CI
   must be green. Merge conflicts are yours to resolve by rebasing.
6. **Stay in scope.** Edit only paths your track owns. Shared files
   (`crates/robigo-runtime/src/{value,interp,env,error}.rs`,
   `Cargo.toml`, `AGENTS.md`, `docs/architecture.md`) take minimal changes,
   called out in the PR.
7. **Never** remove ratchet entries, weaken a test or the oracle, edit
   `rlib/`, or hand-edit recorded GNU R output (re-record with
   `scripts/corpus.py record` / `scripts/regen-expected.sh`).
8. **The machine has 2 cores.** One build at a time (`scripts/build.sh`
   waits for the lock). Run corpora with `--filter`; CI runs them whole.
   Leave no background processes behind.
9. **Report** at the end: PR links, metric before and after, what is
   still open.

## Tracks

Status: **open** (start now), **later** (after the named goalpost,
`docs/design.md` §5.6).

### cli: R's command line and transcripts — open

- **Goal:** robigo runs files the way `R` does: `-f FILE` with R's echo
  transcript (prompts `> `/`+ `, echoed source, `--quiet`, `--no-save`,
  `--args`), `R_DEFAULT_PACKAGES` handling, exit statuses.
- **Owns:** `crates/robigo-cli/src/`; top-level evaluation in
  `crates/robigo-runtime/src/interp.rs` (`run_source`) as needed.
- **Metric:** passes in `gnur-tests` and `gnur-examples` (echo mode; 0 at
  the start).

### startup: default packages and lazy loading — open

- **Goal:** start with GNU R's default packages like GNU R: `datasets`
  lazy-loads (`head(mtcars)` says "lazy-load database is corrupt" today),
  a missing package warns once (it warns twice), `options()` and
  `search()` match.
- **Owns:** `crates/robigo-runtime/src/{startup,bootstrap,serialize}.rs`,
  `builtins/io.rs` (connections).
- **Metric:** `fastr` passes, especially `TestBuiltin_(serialize|unserialize)`
  and `TestConnections` (~380 failing at the start).

### numerics: robigo-numerics — open

- **Goal:** exact numerics everywhere and a fast mode. (1) Software 80-bit
  long double with results identical to the x87 code, so exact mode does
  not depend on x86_64. (2) The numeric-policy seam (exact / fast) for
  reductions and long-double sites. (3) nmath accuracy against GNU R.
- **Owns:** `crates/robigo-numerics/src/{ldouble,arith}.rs` and new
  modules for the software long double and the policy (the d/p/q/r files
  belong to conformance-distributions).
- **Metric:** bit-identical results of the software long double against
  x87 over randomized and edge-case inputs (property tests, millions of
  cases); d/p/q/r functions bit-identical to GNU R's over parameter grids
  (an oracle recorded from GNU R, committed as test data).

### parser: robigo-syntax — open

- **Goal:** parse everything GNU R parses, identically, fast.
- **Owns:** `crates/robigo-syntax/`, `tests/parse/`, `scripts/oracle-parse*.R`.
- **Metric:** files whose parse tree matches GNU R's `parse()` over a
  large corpus: R's own sources, every case in `tests/corpora`, and the R
  code of all installed CRAN packages; then parse throughput (MB/s).

### capi: the C API corpus — open

- **Goal:** the measuring stick for `robigo-capi` (G2), built against GNU
  R before robigo can load native code. (1) A census of the libR symbols
  imported by GNU R's base packages and installed CRAN packages, ranked by
  how many packages use each (`tests/capi/symbols.tsv`, with a script that
  regenerates it). (2) A corpus of C test cases exercising those functions
  (allocation and `PROTECT`, vectors, strings and encodings, attributes,
  pairlists and calls, environments, evaluation from C, errors and
  `R_UnwindProtect`, external pointers and finalizers, ALTREP classes,
  routine registration), each compiled as a shared library and driven by
  an R script whose output GNU R records (`scripts/corpus.py`; add a build
  step if needed).
- **Owns:** `tests/capi/`, `tests/corpora/capi/`, `scripts/capi-*`.
- **Metric:** symbols covered (weighted by package count) by recorded
  cases.

### memory: the memory-manager bake-off — open

- **Goal:** decide `robigo-core`'s memory manager by measurement
  (`docs/design.md` §5.5). Two prototypes behind one Rust trait (objects
  with a header, vectors, cons cells, environments, closures):
  (a) reference counts with a trial-deletion cycle collector;
  (b) a non-moving mark-region heap with conservative stack scanning and
  counts only for copy-on-modify. Workloads modelled on R: binarytrees,
  one environment per closure call, list growth and in-place updates,
  string vectors, closure/environment cycles, `PROTECT`-heavy calls from C.
- **Owns:** `experiments/memory/` (its own Cargo workspace).
- **Metric:** instructions (callgrind), peak RSS, longest pause per
  workload; bounded memory on the cycle workload. Deliverable: the numbers
  and a recommendation in the ledger.

### conformance-*: base R semantics on the current runtime — open, one track per area

Hill-climb `fastr` failures in one area (baseline 2026-10-27: 28,341 of
31,374 pass; failing cases per area below). The semantics carry over when
the runtime moves to `robigo-core` (G3). Native routines of the `stats`,
`utils`, `graphics` and `grDevices` packages (`deriv`, splines, `cov`,
...) come from GNU R's own shared libraries once robigo exports
`libR.so` (G2): do not port them.

| track | owns | `fastr` filter | failing |
|---|---|---|---|
| conformance-values | `builtins/{arith,coerce,math}.rs`, `coerce.rs` | `TestSimpleValues|TestSimpleArithmetic|TestSimpleComparison|TestBuiltin_(operators|log|asvector|asraw|intToBits)` | ~690 |
| conformance-distributions | `crates/robigo-numerics/src/` except `ldouble.rs`/`arith.rs`, `builtins/random.rs` | `TestDistributions|TestRandGenerationFunctions` | ~460 |
| conformance-strings | `builtins/{character,paste,grep,sprintf,character_widths}.rs` | `TestBuiltin_(paste|substr|sprintf|grep|sub|regexpr|nchar|toupper|strsplit|chartr|format|trimws|startsWith|endsWith|encodeString|agrep|grepRaw|iconv)` | ~160 |
| conformance-vectors | `builtins/{subset,subassign,subscript,seq,bind,unique,sort,array}.rs` | `TestBuiltin_(seq|rep|order|sort|unique|duplicated|match|cbind|rbind|which|rev|array|matrix|dim|subset|c_|crossprod)|TestSimpleVectors|TestSimpleMatrix|TestSimpleArrays` | ~150 |
| conformance-lang | `builtins/{lang,envir,eval}.rs`, `dispatch.rs`, `conditions.rs` | `TestBuiltin_(do.call|match.arg|sys|environment|eval|quote|substitute|switch|tryCatch|stop|warning|UseMethod|get|exists|class)|functions/|TestEnvironments` | ~110 |
| conformance-print | `print.rs`, `format.rs`, `deparse.rs` | `TestBuiltin_(print|format|deparse|dput|cat|toString|printdefault)|TestSimplePrinting` | ~80 |

`serialize`/`unserialize` (~310 failing) and connections (~70) belong to
the startup track.

- **Metric:** `fastr` passes in the area's filter (`scripts/corpus.py run
  fastr --filter ...`), then the whole corpus on CI with no regressions.

### perf-vm: the VM and the JIT — open

- **Goal:** robigo faster than GNU R on every benchmark: closure calls
  (fib, binarytrees), base-R idioms, loops.
- **Owns:** `crates/robigo-runtime/src/{vm,compile,jit}.rs`.
- **Metric:** `scripts/perf.py` instruction counts, geometric mean
  robigo/GNU R over correct programs; no benchmark slower than before.

### core: robigo-core and robigo-capi — later (after the memory bake-off)

The new object model and `libR.so` (G2), then the runtime rebuilt on it
(G3). Designed by the lead; split into tracks once the core API exists.

### Later tracks

`methods`/S4 through `methods.so` (G6), numeric fast mode and wasm32
(G5), REPL (G7), whole-closure JIT (G8), packages (G4).
