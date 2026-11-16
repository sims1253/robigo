## 2026-11-16: M1 exit evidence: E1–E9 measured on main `2671f453`

The evidence index for the council's M1 exit review (`docs/g3.md` §15.3).
For each criterion it gives the bar as §15.3 words it, its status, today's
measurement on main, earlier entries where they differ, what proves it in
CI, and the gaps. Two lists close it: the "not implemented yet" stop-gaps
robigo-rt still has, and the ledger's deferred follow-ups. No code
changed.

- **Tree:** main `2671f453` (#716, after #710 and #714). Every number
  below was measured today on this tree. Exceptions are marked: GNU R
  4.6.2's counts, and older entries cited for what they measured.
- **Binary:** `cargo build --release --locked -p robigo-rtapi`
  (`robigo-rt`), sha256 `41b7e38f4d341e33…`.
- **Machine:** AMD Ryzen 9 5900X (12 cores), 47 GiB, WSL2 Linux 6.18,
  rustc 1.100.0, valgrind 3.25.1. The baselines (`../g3.md`, "M1
  baselines", line 405) name valgrind 3.27.1. GNU R's counts reproduce
  them to the instruction anyway.
- **GNU R 4.6.2:** the local build `~/.cache/robigo-research/R-4.6.2-build`,
  the same binary the baselines used. The baselines called it
  provisional, to be re-run on the oracle image (`../g3.md:419-421`). No
  entry did that re-run; P1's runs use the same build (`p1-base.md:11`).
  Today's two runs give the baselines' GNU R counts on every row, within
  0.001%. Recordings use the oracle image.
- **Commands:**
  - instructions and estimated cycles: `scripts/rt-bars.sh`, twice, with
    `--rows` for `trycatch`, `e7-*` and `d2s-*`;
  - GNU R: `scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml
    --cycles` with `RSCRIPT_BIN` set to the build above, twice;
  - the rest: `scripts/rt-ci.py` (every step), `scripts/rt-stress.sh
    --runs 30`, `scripts/rss.py --runs 5`, and `scripts/rivals.py run
    --filter 'calls$' --impl robigo,gnur,rir,fastr --runs 5`.

  Every two runs agree within 0.01% in instructions.
- **Raw data:** `data/2026-11-16-m1-exit-2671f453/`. In `gnur-run*.json`,
  the robigo-rt columns are main `1878a8d5`'s (the tree before #716); only
  their GNU R columns are cited.

### Summary

| | criterion | status | today |
|---|---|---|---|
| E1 | calls.R rows | not met | s3 1.17x GNU R (bar 1.0x); the other five M1 rows 0.37-0.71x; one process 0.283 s against Ř 0.904 s and FastR 6.64 s |
| E2 | fib, binarytrees | not met | fib 1.43x today's robigo (bar 1.0x); binarytrees 0.93x, RSS 26,228 KiB against 32,156 and 43,588 |
| E3 | determinism | met | stress 30 of 30 per profile; weak-keys passes; gctorture and the moving collector clean |
| E4 | overshoot, peak RSS | met on the rt slice | 608 runs, none over the bound, largest 0.238 of it; RSS 0.54-0.82x GNU R |
| E5 | torture identity | met | both modes match production, but for the cases the modes exclude; `getBindingType` matrix passes |
| E6 | the API | partly met | modules, reviews, capi cases and regression cases done; the hook table's cost never measured on a real `lapply` or `[` |
| E7 | strings | met (accepted regression) | match 0.87x, unique 0.71x; paste 1.94x, accepted by the lead |
| E8 | unwinding | met (Eval and Bc) | the three cases pass in Rust, in both profiles and across a C frame |
| E9 | allocation failure | met on the rt slice | conformance under `R_MAX_VSIZE=16M` 283/288 (the known five); 400 injected failures, no difference |

The five failing cases are the same in every mode and have no markers:
- `args/binding-types.R`
- `env/binding-errors.R`
- `env/closure-frames.R`
- `env/hashed-env.R`
- `env/unhashed-env.R`

### What CI proves, and what only hand runs prove

- **CI:** the `rt` job runs on every pull request except docs-only ones
  (`.github/workflows/ci.yml:129-136`, `scripts/ci-select.py:184-187`).
  Its steps are `scripts/rt-ci.py:127-180`. It covers:
  - E3, with 3 stress runs per profile, not 30;
  - E4's overshoot;
  - E5, E8's Rust tests and E9.
- **Hand runs only:** CI's `perf` job measures the old robigo alone, and
  is skipped on changes to rt crates only (`ci.yml:678-680`). No workflow
  runs `rt-bars.sh`, `idiom-vs-r.py`, `rss.py` or `rivals.py`. So these
  have no CI gate and are measured by hand only, as here:
  - E1, E2 and E7;
  - every peak-RSS bar;
  - E3's 30-run count;
  - E6's hook cost.

### E1: `calls.R`

> `calls.R` (one process, JIT off), M1 rows: plain, defaults, named,
> `e$f`, `<<-`, S3 ≤ 1.0x GNU R's instructions, the plain-call idioms
> ≤ 0.6x; wall time below Ř's and FastR's one-process. M3 adds `tryCatch`.

**Not met:** s3 is 1.17x.

| row | robigo-rt instr | GNU R 4.6.2 instr | ratio | bar | |
|---|---:|---:|---:|---|---|
| plain | 2,014 | 4,472 | 0.45x | ≤ 0.6x | met |
| defaults | 2,079 | 5,643 | 0.37x | ≤ 0.6x | met |
| named | 1,937 | 4,433 | 0.44x | ≤ 0.6x | met |
| env-fn (`e$f`) | 2,579 | 3,619 | 0.71x | ≤ 1.0x (below) | met |
| superassign (`<<-`) | 5,029 | 7,480 | 0.67x | ≤ 1.0x | met |
| s3 | 21,708 | 18,497 | 1.17x | ≤ 1.0x | not met |
| trycatch (M3's row) | 124,258 | 88,471 | 1.40x | M3 | — |

- **s3:** main's 1.17x agrees with `s1-s3-row-2.md:71-75` (21,695.1 at
  `a0286dd1`). Two fixes:
  - #717 (P1: the call path) measured 1.135x on its branch. It merged
    after this run, as `90f3d400`, so today's main is below 1.17x; this
    entry's numbers stay `2671f453`'s.
  - #718 (S1: dispatch from slot frames) is open. It measured 1.10x on
    its branch, and 1.096x by callgrind-diff.

  Both touch `args.rs`; the two together are unmeasured. The gap to
  1.0x is split between S1's dispatch and X/P1's call path.
- **env-fn's bar:** §15.3 names `e$f` among the rows at ≤ 1.0x. It never
  lists the plain-call idioms, and neither does §13. Two places put
  env-fn at 0.6x anyway:
  - the bars entry, signed off by the foreman (`../g3.md:191`: "PLAIN:
    `plain`, `defaults`, `named`, `env-fn` (§13's plain-call idioms)"),
    with G1's figure of ≤ 2,171 (`../g3.md:463`);
  - the rows file, which classes the row `plain-call`
    (`benchmarks/rt/calls-rows.toml:67`).

  P1's entries judge it at 1.0x (`p1-base.md`, `p1-gaps.md:5,44`). By
  §15.3's text the bar is 1.0x, so env-fn is met and P1's entries are
  right. The text does not settle it fully: plain, defaults and named are
  also listed under 1.0x and are plain-call idioms all the same.
  - **Proposed fix:** a one-line change of env-fn's `class` to `m1` in
    the rows file.
  - **And a dated section:** the bars protocol fixes a cited bar until a
    dated section replaces it (`../g3.md:156-160`), so the PLAIN list
    needs a dated section that drops env-fn, before the change lands.
    Both are for the foreman.
- **Wall time:** first measured for robigo-rt today: `rivals.py`, the
  fastest of 5 fresh processes. Each runtime prints the same answer.

  | robigo-rt | GNU R 4.5.0 (image) | Ř | FastR |
  |---:|---:|---:|---:|
  | 0.283 s | 0.244 s | 0.904 s | 6.64 s |

  Below both rivals, so met. Caveats:
  - The run is indicative. The machine was shared, the binary is not the
    PGO build, and §13's rule asks for 10 interleaved runs and medians.
  - The warmed peak could not be measured: `rivals.py`'s peak harness
    times itself with `proc.time()`, which robigo-rt's base does not
    have.
- **Earlier numbers:** `p1-bt-check.md:55-64` (`a37f0d19`) had env-fn at
  2,597.0; today it is 2,579, after #714 and #716.
- **Proof:** hand runs only.
- **Owners:** S1 and X/P1 for s3; the foreman for env-fn's bar.

### E2: fib, binarytrees

> fib, binarytrees: ≤ today's robigo (JIT off) in instructions and
> estimated cycles; binarytrees peak RSS ≤ today's and ≤ GNU R's.

**Not met:** fib.

| row | robigo-rt | today's robigo (G1) | ratio | |
|---|---:|---:|---:|---|
| fib instructions | 12,056,540 | 8,428,736 | 1.43x | not met |
| fib est. cycles | 12,164,107 | 8,465,081 | 1.44x | not met |
| binarytrees instructions | 3,828,778 | 4,138,805 | 0.93x | met |
| binarytrees est. cycles | 3,985,259 | 4,388,485 | 0.91x | met |
| binarytrees peak RSS | 26,228 KiB | 32,156 KiB (GNU R 43,588) | 0.82x (0.60x) | met |

- **fib** agrees with `p1-fib-typed.md:64,77-78` (1.43x). The next step
  is the native tier (fix 4, G8; `p1-fib-typed.md:79-80`). Owner: P1 and
  G8.
- **binarytrees peak RSS:** `rss.py --runs 5`, GNU R in the same sitting.
  Earlier measurements: `t1-collection.md:58-65` had 28,904 KiB;
  `d5-recycle.md:104-106` had 23,040 KiB at `060c20af`, without GNU R in
  that run. Today's 26,228 KiB is 14% above D5's figure and still under
  both bars. No entry explains the rise since `060c20af`.
- **Two references for "today's robigo":** G1's binarytrees figure of
  4,138,805 (`../g3.md:451`) and P1's 4,110,191 (`p1-base.md:31`) differ
  by 0.7%. Both bars are met either way.
- **Proof:** hand runs only.

### E3: determinism

> the stress script misses nothing in 30 ASLR runs per tier;
> `weak-keys-collections.R` passes; gctorture and `ROBIGO_GC_MOVE=1`
> clean.

**Met.**
- **Stress script:** `rt-stress.sh --runs 30` with ASLR 2. Eval 30 of 30
  and Bc 30 of 30 runs missed nothing (`data/…/stress30.txt`). That is
  the second 30-run check, after `t1-collection.md:15-32` (`c076b56a`).
- **`tests/rt/capi/weak-keys-collections.R`:** passed in all seven
  conformance steps of today's rt-ci.
- **gctorture and the moving collector:** both conformance steps pass
  282/287: production's 283/288, less `sort/radix-volume.R`, which the
  two gctorture steps skip by its `.requires`. The crate tests under
  gctorture, and under gctorture with `ROBIGO_GC_MOVE=1`, pass.
- **CI on every PR:** "conformance under gctorture (E3)", "conformance
  under ROBIGO_GC_MOVE with gctorture (E3)", and the crate-test steps for
  both. The stress script runs 3 times per profile there, not 30.
- **Gaps:** two cases skip the collector modes:
  - `sort/radix-volume.R` skips both gctorture steps by design (its
    7,200 orders), and `radix-random.R` covers its paths there;
  - the five failing cases have no markers, so they are excluded
    everywhere.

### E4: overshoot

> bytes allocated between `GC_DUE` being set and the collection that
> serves it never exceed max(32 MiB, 10% of the live heap) on the slices
> M1 runs; peak RSS ≤ GNU R's on the memory benchmarks. Extended to every
> corpus and package suite as coverage grows.

**Met on the slice M1 runs.**
- **Overshoot:** `rt-overshoot.py`, in rt-ci. 304 programs in both
  profiles, 608 runs, none over the bound. The largest is 0.2384 of the
  bound: `benchmarks/rt/mem/churn.R`, 8,000,032 bytes against 33,554,432
  (`data/…/overshoot.md`). `t1-collection.md:36-57` had 251 programs and
  the same largest ratio.
- **Peak RSS** (`rss.py --runs 5`, GNU R in the same sitting):

  | program | robigo-rt KiB | GNU R KiB | ratio |
  |---|---:|---:|---:|
  | `benchmarks/suite/binarytrees.R` | 26,228 | 43,588 | 0.60x |
  | `benchmarks/rt/mem/listgrow.R` | 45,428 | 55,484 | 0.82x |
  | `benchmarks/rt/mem/churn.R` | 73,892 | 135,960 | 0.54x |

  T1's numbers (`t1-collection.md:58-65`) were 28,904, 44,520 and 72,640.
- **CI on every PR:** "overshoot on every slice (E4,
  scripts/rt-overshoot.py)". Peak RSS: hand runs only.
- **Gaps:**
  - The extension to every corpus and package suite has not started:
    robigo-rt runs no corpus yet.
  - GNU R is the local build.

### E5: torture identity

> under `ROBIGO_NO_INPLACE=1` and `ROBIGO_MATERIALIZE=1`, semantic values
> and effects are identical on the prototype cases where the debug
> transformation is unobservable. […] And `getBindingType` matches the
> GNU R oracle before and after the first read for a constant, a symbol,
> a call and a defaulted argument, on a closure's first call and after
> GNU R's JIT threshold.

**Met.**
- **Both modes:** `ROBIGO_NO_INPLACE=1` 282/287 and `ROBIGO_MATERIALIZE=1`
  283/288 against production's 283/288. The one difference is
  `capi/inplace-identity.R`, which `inplace-identity.requires` keeps out
  of the in-place mode. The crate tests pass under both modes.
- **Production-mode expectations:**
  - `crates/robigo-rt/tests/attrib.rs:28` and `tests/subset.rs:236`;
  - `tests/rt/capi/inplace-identity.R`.

  See `t2-observation.md:96-139`.
- **`getBindingType`:** `tests/rt/args/binding-type-matrix.R` passed in
  all seven conformance steps. It covers a constant, a symbol, a call and
  a defaulted argument, before and after the first read, on the AST's
  first call and in byte code from the second (`t2-observation.md:39-45`).
  The older `binding-types.R` is one of the five failing cases: it needs
  `makeActiveBinding`, which robigo-rt does not have.
- **CI on every PR:** the two "conformance under …" steps and the
  crate-test steps under each mode. Owner: T2.

### E6: the API

> three builtin modules (arithmetic, `paste`/`nchar`,
> `assign`/`get`/`exists`) with no `unsafe`, reviewed for ergonomics by a
> council member who did not write them; the capi cases `eval`,
> `pairlist`, `unwind`, `extptr` pass over the minimal `robigo-rtapi`;
> the hook table's indirect-call cost (§10) recorded on `lapply` and `[`
> dispatch; regression cases pass for a package ALTREP Length callback
> that allocates and re-enters R, in-place mutation of BODY/FORMALS
> children, and mutation during a callback while compiled code is
> suspended.

**Partly met:** the hook cost is missing.
- **Modules:** `arith.rs` with `relop.rs` and `logic.rs`; `paste.rs` and
  `character.rs`; `envir.rs` (`b1-builtins.md:14-19`). The crate is
  `#![forbid(unsafe_code)]` (`crates/robigo-rtbuiltins/src/lib.rs:24`),
  and `unsafe` occurs in its source only in comments.
- **Reviews:**
  - Sol: "pass with changes" (`e6-ergonomics-review.md:3-5`), done in
    R1c;
  - Gemini: "pass with changes" (`e6-review-gemini.md:3-8`), done in R1d;
  - Muse's review failed with an access error
    (`b1-builtins.md:185-189`).
- **capi cases:** `eval`, `pairlist`, `unwind` and `extptr` passed in
  every conformance step today (`a1-capi-cases.md:26-31`). Two differ
  from the corpus: the rt variant of `pairlist` leaves out
  `capi_car_as_call`, and `extptr` uses `UNPROTECT(6)`.
- **Regression cases:** these pass in every conformance step today:
  - `capi/altrep-length.R`
  - `capi/closure-mutation.R`
  - `capi/callback-mutation.R`, with `capi/suspended-body-write.R`
    (`t2-observation.md:46-94`, `t2b-suspended-deopt.md:56-75`)
- **Hook cost: gap.** It was recorded only with stand-ins, before any
  evaluator existed: at `ae6318b3`, `lapply` 4,039.56 against 3,959.56
  and `[` dispatch 216 against 206 (`b1-builtins.md:151-170`). The entry
  says "V1 re-runs the four rows with the VM's". No entry has done that.
  Owner: V1.

### E7: strings

> `paste`/`match`/`unique` loops ≤ today's instructions, or the
> regression recorded in `docs/ledger/g3.md` with its cause and the
> lead's acceptance.

**Met:** `match` and `unique` pass, and `paste` passes through the
acceptance clause.

| row | robigo-rt | today's robigo (G1) | ratio |
|---|---:|---:|---:|
| e7-paste | 96,790 | 49,821 | 1.94x |
| e7-match | 99,237 | 114,413 | 0.87x |
| e7-unique | 129,476 | 181,394 | 0.71x |

- **The record:** `2026-11-15-e7-seq.md:67-80`. It gives the cause:
  interning each result string costs about 426 instructions
  (`Cx::mk_char`), and `paste_all`'s per-piece work about 600 per
  element. Its acceptance line reads "Accepted by the lead 2026-11-16;
  fix in P1, interning decided by D10." The ledger directory continues
  `docs/ledger/g3.md` (`README.md`), so the place matches E7's words.
- **Gaps in the record:**
  - The acceptance is dated 2026-11-16 inside an entry dated 2026-11-15;
    the foreman recalls 2026-11-15.
  - It names no ceiling. It was accepted at 2.52x, with
    `p1-e7-paste.md:3-7` giving 125,690 instructions; P1 brought that to
    1.92x (`p1-e7-paste.md:63-76`).
  - Today's 96,790 is 2.2% above `ppacct.md:105-110`'s 94,701.4 (main
    `2c6075a5`). No entry measures what raised it.
- **Proof:** hand runs only. `rt-bars.sh` measures the e7 rows only when
  `--rows` asks for them.
- **Owners:** P1 for paste; D10 for interning, at M3.

### E8: unwinding

> `on.exit` that catches its own error during an unwind returns the outer
> condition; `R_UnwindProtect` cleanup that catches an error resumes the
> original continuation; a C context's `cend` runs before the jump; all
> in every tier and across a C frame.

**Met for the two tiers M1 has, Eval and Bc.** Each passed in today's
rt-ci, in the release step and under gctorture, the moving collector and
the pin report:
- `on.exit`:
  - `crates/robigo-rtcore/tests/contexts.rs:126`;
  - across a C frame: `crates/robigo-rtapi/tests/capi.rs:112`.
- `R_UnwindProtect`:
  - `robigo-rtcore/tests/native.rs:321`;
  - across a C frame: `robigo-rtapi/tests/capi.rs:124`.
- `cend`:
  - `robigo-rtcore/tests/native.rs:251`;
  - across a C frame: `robigo-rtapi/tests/capi.rs:135`.

`capi.rs` runs both profiles (`PROFILES`, line 27), with its C in
`tests/support/capi_test.c`.

- **Gaps:**
  - The expected results are Rust assertions; no GNU R-recorded R case
    exists for E8.
  - "Every tier" is Eval and Bc: M1 has no native tier, and no CI step
    runs these with inlining off.

### E9: allocation failure

> the M1 slices pass under a small `R_MAX_VSIZE`, and an injected
> allocation failure after a visible effect neither loses nor repeats the
> effect.

**Met on the rt slice.**
- **Small limit:** under `R_VSIZE=4M R_MAX_VSIZE=16M`, conformance is
  283/288, the five known failures. `tests/rt/alloc/vector-limit.R` has
  GNU R's messages verbatim.
- **Injected failures:** `rt-fail-alloc.py --points 400` on
  `tests/rt/alloc/effects.R`, 400 runs, no difference.
  `t1-collection.md:76-102` covered more: every allocation of that case,
  and 6,917 runs over 188 cases.
- **The leak this found:** this step found a real one. Dead slot frames
  kept their vector cells in the heap's count, so long loops reached
  the limit. Fixed in #710 (`2026-11-16-sort.md`).
- **CI on every PR:** "conformance under a small vector limit (E9)" and
  "injected allocation failures (E9, scripts/rt-fail-alloc.py)".
- **Gap:** "the M1 slices" is the rt slice only, since robigo-rt runs no
  corpus yet.

### Stop-gaps: "not implemented yet" in robigo-rt

robigo-rt raises "robigo: X is not implemented yet" where it knows it
differs from GNU R. Each item below is a `not_yet` call or a stub, with
its path and line on `2671f453`. The paths are shortened: `bi` is
`crates/robigo-rtbuiltins/src`, `rt` is `crates/robigo-rt/src`, `core` is
`crates/robigo-rtcore/src`, `api` is `crates/robigo-rtapi/src`.
- **The loader's stubs** (`rt/base.rs:276-317`):
  - **What they are:** every name of `proto/base-bound.txt` without an
    implementation of its kind is bound to a stub that raises "the
    primitive 'NAME'" or ".Internal(NAME())" is not implemented yet.
  - **How many:** 11 in the shipped binary, 3 primitives and 8 internals.
    The primitives are `call`, `ceiling` and `exp`. The internals are
    `grep`, `iconv`, `pipe`, `rapply`, `rawToChar`, `regexpr`, `sub` and
    `which.max`.
  - **Checked:** `call`, `ceiling`, `exp`, `grep`, `which.max` and
    `rawToChar` raise these messages on this binary.
  - **Names outside the bound** give "could not find function" instead:
    `round`, `withVisible`, `makeActiveBinding`, `R.home`, `proc.time`.
- **Size limits:**
  - a function frame of more than 65,536 variables
    (`core/env/slotapi.rs:282`);
  - an environment of more than 65,536 variables past its frame's shape
    (`core/env/mod.rs:1505`);
  - more than 65,535 formals or call arguments (`rt/args.rs:811,813`).
- **Environments:**
  - user-defined database environments and S4 environments
    (`core/env/mod.rs:370-371`);
  - `SET_FRAME` on a closure frame (`core/env/slotapi.rs:438`);
  - `eval()` in an S4 environment or an untagged pairlist
    (`rt/eval.rs:644,674`);
  - `as.environment` of several positions, of S4, of a list
    (`bi/envir.rs:989,1025,1026`);
  - a frame number as `envir` (`bi/envir.rs:274`).
- **Collation:** both come from `bi/text.rs:104-109`:
  - text not in FCD form. ICU collates it unnormalized and ICU4X only
    normalized. This includes the Tibetan vowels U+0F73, U+0F75 and
    U+0F81.
  - `R_ICU_LOCALE` set to anything but C.
- **Sort:**
  - radixsort grouping with `na.last = NA` (`bi/sort/radix.rs:196`);
  - a `decreasing` that is not logical (`bi/sort/radix.rs:178`);
  - `.Internal(rank())` of more elements than the vector has
    (`bi/sort.rs:799`; GNU R reads past the vector there).
- **Package ALTREP:** writing, duplicating or reading in place a package
  ALTREP object is refused in about 60 places, among them:
  - arithmetic, `paste`, subsetting and sort (`bi/arith.rs:42` and
    others);
  - `for` over an ALTREP sequence (`rt/vm.rs:1441`);
  - the C API's `DATAPTR`, `*_ELT` and `SET_*` (`api/api.c:226-353`).
- **Options:**
  - 11 options cannot be set to a value other than NULL: `width`,
    `digits`, `deparse.cutoff`, `expressions`, `nwarnings`, `max.print`,
    `keep.source`, `echo`, `verbose`, `warnPartialMatchDollar` and
    `warnPartialMatchAttr` (`bi/options.rs:337-349,418`).
  - 13 more lack their checks and C state, among them `scipen`, `OutDec`,
    `prompt` and the `PCRE_*` options (`bi/options.rs:692`).
- **Deparse:** S4 objects, unforced promises, srcrefs, `S_compatible`,
  `hexNumeric` and `digits17` (`rt/deparse.rs:588-1414`).
- **Printing:** matrices, arrays, array-like lists and pairlists,
  primitives and `row.names` (`rt/print.rs:231-710`).
- **S4:** S3 dispatch on an S4 object, S4 dispatch and group dispatch
  (`rt/dispatch.rs:180,2467,2588,2820`). Also two different `Ops`
  methods, except the `difftime` pairs (`:2620`).
- **Subscripts:**
  - matrix and array subscripts by dimnames (`bi/subset.rs:456`,
    `bi/subassign.rs:681`);
  - `[[<-` on a pairlist (`bi/subassign.rs:968`).
- **Namespaces:** `loadNamespace`, and every namespace but base
  (`rt/ns.rs:68,71`), so no package and no `stats` (`rank(ties =
  "random")`).
- **Evaluator:** byte-code objects (`BCODESXP`, marked M3;
  `rt/eval.rs:117`); internal restarts (`rt/conditions.rs:839`);
  `gc(verbose = TRUE)` (`rt/memory.rs:32`); `tracemem`
  (`core/edit.rs:157`).
- **C API:**
  - `R_init_<pkg>` routine registration (`api/dotcode.rs:176`);
  - `.Call` of a native symbol reference (`api/dotcode.rs:323`);
  - `R_signal_protect_error` (`api/errors.rs:105`);
  - some `SETCAR`/`SETCDR`/`SET_TAG` cases (`api/api.c:410-436`).

  One more is an assert, a panic rather than an R error:
  `core/context.rs:1692`, `on.exit` code in a context that C pushed.

### Deferred follow-ups a council reader will ask about

Still open:
- **Performance:**
  - s3 at 1.17x (#717 merged after this run; #718 open; owners S1 and
    X/P1);
  - fib at 1.43x (the native tier, G8; `p1-fib-typed.md:77-81`);
  - paste at 1.94x, accepted, with D10 at M3 (`p1-e7-paste.md`).
- **Baselines and bars:**
  - the provisional GNU R 4.6.2 baseline was never re-run on the oracle
    image (`../g3.md:419-421`);
  - env-fn's bar (E1 above);
  - the E6 hook cost (owner V1);
  - the E1 wall-time rule of §13 (10 interleaved runs, PGO build) has
    never been applied.
- **Collation** (`2026-11-16-collation.md`):
  - non-FCD text and `R_ICU_LOCALE` (`:189`);
  - a Latin fast path (`:166-168`);
  - an `icu_collator` debug assertion that the dev profile turns off,
    not reported upstream; that needs Max's approval (`:172-179`).
- **Memory and frames:**
  - D4's extra REALSXP from early values (`p1-reruns.md:146-149`);
  - the 3 LISTSXP of `eval(quote(v <- 1))`, never explained
    (`d2s-runs.md:100-105`);
  - deopt only at statement boundaries (`t2b-suspended-deopt.md:50-53`).
  - The binarytrees RSS rise since `060c20af` (E2 above).
- **Compiler** (`v2b-cmp.md:89-97`):
  - not ported: the `::`/`:::` handler, DOTCALL, `cmpIs`, `cmpPrim1/2`;
  - promise code is kept per expression, not per promise (M3's
    BCODESXP).

  A closure with very many formals recompiles on every call
  (`p1-inline.md:171-175`).
- **The base subset** lacks `round`, `sys.calls`, `withVisible`,
  `debug`, `body`, `enc2native`, `makeActiveBinding` and `proc.time`
  (`p1-unboxed.md:90-93`, `t2-observation.md:45`). Several cases and
  tools work around them; the rivals' warmed-peak number above is one.
- **C API:**
  - `on.exit` in a C-pushed context; `R_run_onexits(NULL)` at
    `R_CleanUp` (`c2-contexts.md:187-190`);
  - protections on paths no case reaches yet (`ppacct.md:160-163`);
  - over-unprotecting stays robigo's error (the foreman's interim
    decision, `ppacct.md:157-159`);
  - no AArch64 stub (`h3-debug-gc.md:161-163`);
  - 34 of the 75 capi C sources do not compile against 4.6.2's headers
    (`oracle-462.md:70-87`).
- **Oracle:** 4.6.2's behaviour changes still need a minimized
  reproduction and a NEWS entry (`oracle-462.md:57-63`).
- **Undecided:**
  - `R_HOME` and `R.home()` (`startup-options.md:61-64`);
  - the §8 TRUELENGTH shared-bit rule, "for the lead"
    (`../g3.md:139-143`).
- **Builtins** not ported (`b2b-internals.md:80-87`, `b3-print.md:53-57`,
  `s1-dispatch.md:96-102`, `v1b-vm.md:90-99`): listed under the
  stop-gaps above.
- **Sort** (`2026-11-16-sort.md:207-217`):
  - `rank(ties = "random")` needs `stats`;
  - `round()` is missing;
  - an error reached through `NextMethod` to a special names argument
    values where GNU R shows expressions.

Done in a later entry, so not open:
- collation's missing sort internals (`sort.md`) and its short options
  list (`startup-options.md`);
- the `.Call` stub's zeroing (removed in `a1-capi-cases.md`);
- PPACCT's resumed-loop gap (`rtapi-sweep.md:28-38`);
- D5's cause and the D2–D5 decisions (`d5-709.md`, `p1-reruns.md`);
- the suspended-body deopt (`t2b-suspended-deopt.md`);
- superassign and binarytrees under their bars (`p1-superassign.md:91`,
  `p1-bt-check.md:75-77`);
- the E6 reviews (`e6-ergonomics-review.md`, `e6-review-gemini.md`);
- D9 (`d9-decision.md`);
- weak-keys in the rt slice (`a1-capi-cases.md:5`).
