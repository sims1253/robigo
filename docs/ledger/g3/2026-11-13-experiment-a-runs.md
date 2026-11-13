## 2026-11-13: experiment A's remaining runs (M0)

- why: `docs/g3.md` §15.1 left four runs before `robigo-heap` adopts pins
  (§3 step 3): the capi corpus and the package suites with natives built,
  under gctorture, with the pin report; a premature-finalization probe; an
  external-buffer probe; narrowing and zeroing measured as separate
  switches. §15.1's adoption rule was written before them: pins only if no
  capi-corpus or package-suite case that passes today fails under pins
  and the premature-finalization probe passes.
- **result: the rule as written fails**, on both of its conditions.
  1. The premature-finalization probe fails under pins on `Rf_cons`,
     `Rf_lang2` and `Rf_setAttrib`. Cause: today's exports do not root
     their arguments. §3 item 6 fixes all three (a diagnostic switch that
     roots them makes the probe pass under pins).
  2. One package-suite case that passes today fails under pins: cli's
     `test-progress-message` (3 of 3 standalone runs; 3 of the 5 suite
     runs with pins; never in the old order). Cause: processx holds a new
     connection unprotected across `install()`, and robigo's `install`
     allocates there (the core's symbol table is not the runtime's) where
     GNU R's finds the existing symbol. §3 item 6 does **not** fix it; an
     `install` that never collects does.

  Everything else that differs under pins also differs in the old order
  (torture crashes and timeouts), and the capi corpus has no other
  difference. With pins, §3 item 6 and an `install` that does not
  allocate for a symbol the runtime has, every case here passes (shown
  with diagnostic switches; numbers below). So §3 step 3 is reopened, and
  the proposal at the end adds those two conditions.

### Tree, binaries, commands

- Tree: branch `track/g3-expa-runs`. The runs used its first commit on
  main `25bad708`: the `.Call` and collector switches below in
  `crates/robigo-core` (`shim.c`, `gc.rs`, `gc/diag.rs`), the two probes
  in `tests/corpora/capi`, and `scripts/package-suite.py` passing
  `ROBIGO_GC*`, `ROBIGO_CALL65_*`, `ROBIGO_CAPI_*` and `R_GCTORTURE*` into its
  container.
  The follow-ups (per-failure mechanism, the root-args and install-nogc
  switches, `library+offset` in the report) used the branch's later
  commit, host and trixie builds alike. The branch was then rebased on a
  newer main; no file the runs touch changed in between.
- Host binary (capi corpus, probes, callgrind): `scripts/build.sh`
  (release), gcc 16.2.1, glibc 2.44; natives `scripts/build-natives.sh
  tools parallel utils grDevices graphics grid splines` (stats does not
  link here); `R_HOME=~/.cache/robigo-cran/rhome`.
- Trixie binary (package suite): `cargo build --release --locked -p
  robigo-cli` in `ghcr.io/sims1253/robigo-ci:rust-067c755acfb94a9c` (the
  CI image, glibc 2.41); natives `scripts/build-natives.sh stats splines
  tools parallel utils grDevices graphics grid` in the same image; its
  home made by `scripts/robigo-home.sh` in `robigo-oracle:4.5.0`; no
  rdbflat.
- capi corpus: `env $CONFIG ROBIGO_GC_PIN_REPORT=<file> scripts/corpus.py
  run capi --jobs 12`. Package suite: `env $CONFIG
  ROBIGO_GC_PIN_REPORT=<file> ROBIGO=<trixie> ROBIGO_NATIVES=<trixie>
  ROBIGO_PKG_JOBS=6 scripts/package-suite.py run --ratchet-only --out
  <file>`. `--ratchet-only` runs exactly the blocks that pass today.
- Configurations, each also run as the baseline under the same torture:

  | name | environment |
  |---|---|
  | baseline | `R_GCTORTURE=<gap>` |
  | pin | `+ ROBIGO_GC_PIN_ONLY=1` |
  | pin-nonarrow | `+ ROBIGO_GC_PIN_ONLY=1 ROBIGO_CALL65_NO_NARROW=1` |
  | pin-nozero | `+ ROBIGO_GC_PIN_ONLY=1 ROBIGO_CALL65_NO_ZERO=1` |
  | pin-plain | `+ ROBIGO_GC_PIN_ONLY=1 ROBIGO_CALL65_PLAIN=1` |

  Torture is robigo's `R_GCTORTURE` (`memory.c:init_gctorture`; a
  collection every `gap` core-heap allocations, which happen at the C
  boundary only). The capi corpus ran at gap 1 (GNU R's `gctorture(TRUE)`).
  The package suite could not: at gap 1 rlang ran 3,633 s against GNU
  R's 196 s with 30 of its 58 files over the budget, and ggplot2 the same,
  so it ran at gap 100. Calibration on rlang's ratcheted blocks: no torture
  1,322/1,326 in 26 s; gap 1000 1,321 in 34 s; gap 100 1,240, two files
  crashing, 73 s. Those crashes come from the torture itself, not from
  pins.

### The switches (diagnostic, off by default, frozen-core exception for M0)

- `ROBIGO_CALL65_NO_NARROW=1`: the stub leaves the scan's limit where it
  is (it still zeroes). `ROBIGO_CALL65_NO_ZERO=1`: the stub skips the
  1 KiB zeroing (it still narrows). `ROBIGO_CALL65_PLAIN=1`: both, by
  calling the routine directly (as before). All three, and the
  diagnostics' call stack, go through `robigo_try_call65_exp`, a frame of
  its own; the default `robigo_try_call65` is main's before 4338227e
  except for one thread-local mode read.
- `ROBIGO_CAPI_ROOT_ARGS=1` (capi): `Rf_cons`, `Rf_lcons` and
  `Rf_setAttrib` protect their arguments for the whole call, an emulation
  of §3 item 6 for the exports the probe found unrooted.
  `ROBIGO_GC_INSTALL_NOGC=1` (core): making a new symbol never collects, an
  emulation of GNU R's one symbol table, where `install` of a name R
  already has allocates nothing. Both are off by default; neither touches
  a fast path (`Rf_cons` gains one cached test). The root-args runs below
  used the protect stack; the committed switch uses the precious set,
  which cannot fail, so these exports still never jump (docs/core.md §5,
  `tests/no_inner_jumps.rs`). Both are precise roots; the probe gives the
  same result with either.
- `cargo test --release -p robigo-core -p robigo-capi`: 65/65 by default
  and with each of `NO_ZERO`, `PIN_ONLY` + `NO_NARROW`, `PIN_ONLY` +
  `PLAIN`, `PIN_ONLY` + the report, `PIN_ONLY` + root-args +
  install-nogc, and `ROBIGO_GCTORTURE=1` + root-args. `NO_NARROW` alone
  fails `the_api`'s `native_call_frames_keep_nothing` ("a stale word in
  the frames of a native call kept an object alive"). That test plants
  dead pointers in the bridge frames that narrowing leaves out, so it fails
  without narrowing in the old order.
- `ROBIGO_GC_PIN_REPORT=1` (stderr) or `=<path>` (appended): every weak
  reference whose key is unmarked on the precise marks (after the
  weak-reference fixed point) and that a C stack word reaches, with that
  word, its address and the routine whose frames hold it (`dladdr`:
  library and symbol, or `library+offset`). Once per weak reference, key and
  routine. Under pin-only that object is finalized while pinned
  (`mode=pin-only`). Otherwise the word keeps it alive (`mode=baseline`).
  The marks a collection leaves are the plain path's: marking is
  monotone, so the fixed point taken before the scan changes nothing.
- `ROBIGO_GC_STRONG_DLL=<name>` (with pin-only): the words in the frames
  of that library's routines are scanned before weak references are
  decided.
- Attribution: while either diagnostic is set, the shim keeps a stack of
  running `.Call`/`.External`/`.C` routines with the highest address
  their frames (and, unnarrowed, the bridge frames below the scan's
  limit) can occupy. A word belongs to the innermost routine above it.
  Words outside any routine (callbacks such as ALTREP methods, libR's own
  entries) print `dll=? routine=?`.
- Default path: per no-op `.Call` 11,820 instructions against main's
  11,823 (the stub's 172 identical; the trampoline 71 against 74,
  because main's `PLAIN` test moved off the default path). A
  collection gains one `Option` test.

### Cost: narrowing and zeroing separately

Instructions per no-op 3-argument `.Call` (callgrind, 40,000 minus 20,000
calls of `.Call("noop3", 1L, 2L, 3L)` in a closure's loop, host binary;
`~/.cache/robigo-scratch/g3-expa/cg.sh`):

| configuration | per call | vs default |
|---|---:|---:|
| main `25bad708` | 11,823 | |
| default | 11,820 | 0 |
| pin (`ROBIGO_GC_PIN_ONLY=1`) | 11,820 | 0 |
| no-zero | 11,772 | −48 |
| no-narrow | 11,851 | +31 |
| plain | 11,742 | −78 |
| pin + plain | 11,742 | −78 |
| pin report on | 11,880 | +60 |

The switch path itself costs about 34 instructions (a second frame and
the mode test), so per-function counts separate the two parts. The stub
is 172 instructions with zeroing and 81 without, so **zeroing costs 91**.
Narrowing is 2 in the stub (the limit's store) and 10 in the trampoline
(test and restore): **narrowing costs about 12**. On this benchmark the
two are 0.8% and 0.1% of a no-op `.Call`. (The council measured 7,631
with another script, and −120 for the plain call in the trampoline's own
frame; here plain is −78 after the switch path's 34, so about −112.)

### The probes (capi corpus cases, GNU R 4.5.0's output recorded)

- `pin_premature`: an external pointer held only by a C local, passed to
  an allocating API function under `gctorture(TRUE)`, and checked for
  finalization right after (protected while pending finalizers run, so
  only a decision inside the call counts). Seven functions:
  `R_RegisterCFinalizerEx`, `Rf_defineVar` into an environment made in R
  (the process's first write from C into one), `Rf_cons`, `Rf_lang2`,
  `Rf_setAttrib`, `R_MakeWeakRef`, `Rf_defineVar` into `R_NewEnv`. GNU R:
  FALSE for all seven. robigo, old order: FALSE for all seven.
  **Pin-only: TRUE for `Rf_cons`, `Rf_lang2` and `Rf_setAttrib`**, so
  these finalized their argument during the call. Identical in all four
  pin configurations. `ROBIGO_GC_STRONG_DLL=pin_premature` restores
  FALSE. (GNU R's `R_MakeExternalPtr` does not protect its `tag` or
  `prot` while allocating, `memory.c:R_MakeExternalPtr`, so the probe
  does not use it.)
- `pin_extbuf`: an external pointer held only by a C local, whose
  finalizer frees a `malloc` buffer another C local still names; `R_gc()`
  then `R_RunPendingFinalizers()`. GNU R: `freed_while_named TRUE`,
  `pointer_cleared TRUE`. GNU R frees it during the call, which is the
  resource-safety limit GNU R itself has. robigo, old order: FALSE
  FALSE (the stack word keeps it). **Pin-only: TRUE TRUE, as GNU R.**
  Identical in all pin configurations.
- Narrowing and zeroing leave both probes unchanged: the outputs of both
  are byte-identical across `NO_NARROW`, `NO_ZERO` and `PLAIN`, with and
  without pin-only (one output pair for the old order, one for
  pin-only).
- The pin report names exactly the probe's pointers: 7 hits in the old
  order, 6 under pin-only, all in `capi_pp_probe` or `capi_eb_probe`.

### capi corpus (84 cases, gctorture gap 1)

| configuration | pass | failing |
|---|---:|---|
| no torture | 80 | `cached_ptr_across_calls`, `default_args`, `identity-unboxed` (all three unmarked today), `pin_extbuf` |
| baseline | 78 | the three, `pin_extbuf`, `lookup_static_torture`, `nested_write_back` |
| pin | 78 | the three, `pin_premature`, `lookup_static_torture`, `nested_write_back` |
| pin-nonarrow | 78 | as pin |
| pin-nozero | 78 | as pin |
| pin-plain | 78 | as pin |

`lookup_static_torture` (prints 1 where GNU R prints 0) and
`nested_write_back` (FALSE for TRUE) fail under torture without pins too,
deterministically (three runs each, both orders): torture finds them on
today's tree, not pins. Apart from `pin_premature`, no case that passes in
the torture baseline fails under pins. The pin report hits only the two
probes. These runs used the
first, six-function version of `pin_premature`; the final seven-function
probe was re-run in every configuration (above).

### Package suite (ratcheted blocks, 20 packages, gctorture gap 100)

Blocks matching GNU R, of the 7,567 ratcheted (`--ratchet-only`), and
what differs from the baselines. Two baseline runs differ from each
other in eight ggplot2 blocks (`test-guide-axis`, `test-guides`,
`test-theme`: files near their time limit under torture), so a block
counts as a pin effect only if it differs from both baselines and the
difference repeats.

| run | match | files not done |
|---|---:|---|
| baseline | 7,533 | ggplot2 3 timeout, cli 1 oom, magrittr 1 crash, curl 1 crash |
| baseline-r2 | 7,525 | as baseline |
| pin | 7,489 | + rlang `test-c-api` crash |
| pin-r2 | 7,486 | + rlang `test-c-api` crash |
| pin-nonarrow | 7,494 | rlang `test-c-api` crash; magrittr oom instead of crash; cli all done |
| pin-nozero | 7,487 | + rlang `test-c-api` crash, cli `test-ansi-make` crash |
| pin-plain | 7,445 | + rlang `test-c-api` crash and `test-trace` oom, ggplot2 `test-labels` crash |

Every difference, followed up with the file alone through the suite's
runner (`tests/packages/runner.R`) in `robigo-oracle:4.5.0`, both orders,
two or three runs each, and where that disagreed, the package alone
through `package-suite.py run PKG` three times per order:

| difference | pin effect? | evidence |
|---|---|---|
| cli `test-progress-message` "cli_progress_message error" (1 block) | **yes** | pin 3/3 fail, baseline 3/3 pass; `ROBIGO_GC_STRONG_DLL=processx` 2/2 pass; see processx below |
| rlang `test-c-api` crash (42 blocks) | no | rlang alone: baseline crashes it 3/3, pin 3/3; standalone both orders crash at "collisions are handled" 5/5 |
| rlang `test-trace` crash/oom (pin-plain, 38 blocks) | no | standalone baseline crashed it once in two runs (signal 11), pin-plain once (killed); rlang alone: one pin run of three, no baseline run |
| ggplot2 `test-labels` crash (pin-plain) | no | standalone 64/64 in both orders, twice each |
| cli `test-ansi-make` crash (pin-nozero) | no | standalone 48/48 in both orders, three times each |
| ggplot2 `test-guide-axis`, `test-theme`, `test-guides` blocks | no | differ between the two baselines; standalone `test-guide-axis` 47/47 in both orders; `test-theme` over 900 s in both |
| cli `test-ansi-combine` (5 blocks won, magrittr crash↔oom) | no | memory and time limits under torture, both directions |

So exactly one ratcheted case fails because of pins. Narrowing and
zeroing change nothing beyond run-to-run noise. The processx case failed
in pin, pin-r2 and pin-nozero, and passed in pin-nonarrow and pin-plain,
whose reports show no processx hit: no collection fell in processx's
window in those runs.

### Pin report hits, investigated

Hits per package-suite run (`dll:routine`, deduplicated per process):

| run | hits |
|---|---|
| baseline | processx `processx_exec` 1; sf `_sf_CPL_transform` 3; units `_units_R_ut_parse` 3, `_units_ud_convertible` 1; curl (static routine) 2; unattributed 1 |
| baseline-r2 | processx 1; sf 5; units 4; curl 2 |
| pin | processx 2; sf 4; units 2 |
| pin-r2 | processx 2; sf 5; units 7 |
| pin-nonarrow | sf 6; units 4; unattributed 3 |
| pin-nozero | processx 2; sf 3; units 3; unattributed 1 |
| pin-plain | sf 4; units 9; rlang (static routine) 1 |

- **processx `processx_exec`: a real missing `PROTECT`, made observable
  by robigo's `install`.** `processx__create_connection`
  (`src/unix/connection.c`) takes the connection's external pointer
  (finalizer `processx__connection_xfinalizer`, which closes the
  descriptor) from `processx_c_connection_create`, which unprotects it,
  and calls `defineVar(install(membername), res, private)`. GNU R's
  `install("stdout_pipe")` finds the symbol (processx's R code has it) and
  allocates nothing, so GNU R never collects in the window. robigo's
  `install` interns the name in the core's own symbol table on first use,
  which allocates, and under pin-only the connection is finalized during
  `processx_exec`. Reproduction (`process$new` 40 times under
  `R_GCTORTURE=1`): GNU R 40/40; old order 40/40; pin-only 39/40;
  `ROBIGO_GC_STRONG_DLL=processx` 40/40; `ROBIGO_CAPI_ROOT_ARGS=1` 39/40;
  `ROBIGO_GC_INSTALL_NOGC=1` 40/40. cli's case the same way: pin 7/8
  blocks, +root-args 7/8, +install-nogc 8/8, +both 8/8. A C-level
  reproduction (`install` of a name then `defineVar` into an R
  environment, the pointer unprotected): the first call of a process
  finalizes it under pin-only, and a second call (name now interned)
  does not.
- **sf (`_sf_CPL_transform`, `_sf_CPL_geos_op`, `_sf_CPL_geos_op2`,
  `_sf_CPL_get_bbox`, `_sf_sfc_unique_sfg_dims_and_types`) and units
  (`_units_R_ut_parse`, `_units_ud_convertible`): stale words of dead
  Rcpp temporaries, no outcome change.** ggplot2's `test-coord_sf` (46/46)
  and `test-geom-sf` (39/39), the files that call them, match in the old
  order, pin-only, and pin-only with `ROBIGO_GC_STRONG_DLL=sf` or
  `=units`. units 1.0-1's `ud_convertible` makes two `xut_unit` locals
  (Rcpp `XPtr`s finalized by `ut_free`); Rcpp releases them when the
  function returns, and the generated wrapper then allocates (`wrap`,
  `Rcpp_precious_preserve`) while their addresses are still in the
  frames. They are garbage, and pin-only finalizes them when GNU R would.
- **curl (a static routine; this binary printed no offset, the later one
  prints `curl+0x...`)**: in the old order only. curl's `test-gc` passes
  in all configurations (6/6, also with `STRONG_DLL=curl`). `test-multi`
  crashes under torture in every configuration, the baseline included.
- **rlang (a static routine, pin-plain, once)** and the **unattributed
  hits** (`dll=?`: words in frames outside any `.Call`, such as ALTREP
  callbacks or libR's entries): no outcome attributable to them; they
  occur in the runs of the sf/units files, whose outcomes do not change.

`ROBIGO_GC_PIN_REPORT` therefore found the one real case, and all its
other hits were stale words. It cannot tell the two apart by itself; the
`STRONG_DLL` comparison and the package's source did.

### Proposal: revise §3 step 3

What the runs show, per failure:

| failure | mechanism | §3 item 6 fixes it? | old order |
|---|---|---|---|
| probe: `Rf_cons` | `alloc_node` can collect with `car`/`cdr` only in C words | yes (`ROBIGO_CAPI_ROOT_ARGS=1`: FALSE) | passes |
| probe: `Rf_lang2` | `cons_with` protects `s`, then `Rf_list1(t)` conses `t` unrooted | yes | passes |
| probe: `Rf_setAttrib` | `set_attrib` unprotects `vec` before `sym("comment")`, which interns a core symbol the first time it is asked (an allocation GNU R does not make) | yes; an `install` that never collects fixes it too | passes |
| cli `test-progress-message` | processx's unprotected connection across `install()`, which allocates in robigo only | **no** | passes |
| `pin_extbuf` (documents) | GNU R frees a buffer a C local names | pins match GNU R | differs from GNU R |

With both conditions emulated (`ROBIGO_GC_PIN_ONLY=1
ROBIGO_CAPI_ROOT_ARGS=1 ROBIGO_GC_INSTALL_NOGC=1`), both probes match GNU
R, and cli's case and the processx reproduction pass. The root-args
switch covers only the three exports the probe exercises
(`Rf_cons`, `Rf_lcons`, `Rf_setAttrib`). It shows the mechanism; it is
not item 6.

Proposed text for §3 step 3 (the lead decides; `docs/g3.md` is not
changed by this entry beyond §15.1's account of the runs):

> 3. C segments conservatively, as pins (decided by experiment A, §15.1,
> and its M0 runs), **on two conditions that pins depend on**: (a) §3
> item 6, every C API export that may allocate roots its `SEXP` arguments
> precisely for its whole duration; (b) **allocation parity**: an export
> allocates (and so can collect) only where GNU R's does. `install` and
> `installChar` of a name the isolate already interned, attribute and
> symbol lookups, and any other path GNU R serves without allocating, may
> not allocate in robigo either; robigo-specific caches are filled
> outside a C caller's window or with collection deferred. Under pins a
> package's missing `PROTECT` is then exposed exactly where GNU R exposes
> it (processx's connection in `install`, which GNU R never collects in),
> no earlier. Gates for `robigo-rtapi`: `pin_premature` with every export
> it covers, a case for `install` of an existing name between an
> unprotected allocation and its use, and cli's `test-progress-message`
> under gctorture, with the pin report. `ROBIGO_GC_STRONG_DLL=<name>`
> stays a diagnostic only ...

If (b) cannot be held for some export, the fallback for the frames of a
running `.Call` is the old order with the stub (narrowing and zeroing):
it passed every case here, and on today's tree it cost 103 instructions
per `.Call` (12 narrowing, 91 zeroing). It finalizes stale-word garbage
late (`pin_extbuf` differs from GNU R; the sf/units temporaries live
until their frames go). Narrowing and zeroing: under pins both probes are
unchanged without them, so they can go when pins land. In the old order
they are what keeps the stale-word misses away (§15.1's 30/0/0), so
they stay as long as that order does.
