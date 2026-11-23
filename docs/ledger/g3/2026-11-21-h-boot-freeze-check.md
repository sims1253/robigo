## 2026-11-21: the boot freeze's check period, its leak rule, and its frozen-garbage cases (amends 2026-11-21-h-boot-freeze-bar.md)

This changes #948's bar, `2026-11-21-h-boot-freeze-bar.md`, in three places: (c), (d), and (d)'s leak rule. The foreman signed it on #959 before the runs it governs.

### (c) The check runs at every N-th collection

`ROBIGO_GC_FREEZE_CHECK=N` checks the freeze's invariant at every N-th collection (1 means every collection, unset means never). A value that is not a number panics, so a typo cannot turn the check off unnoticed.

- **Why:** at every collection under gctorture, the check traces the heap twice per allocation. `tests/rt/bytecode/jit.R` took 536 s with the check and 97 s without, past the 600 s case timeout. Seven ratcheted cases timed out in E3.
- **CI's E3** (the conformance under gctorture) checks every 16th collection. `jit.R` takes 124 s there, 1.27× the unchecked run.
- **The crates' gctorture tests step** still checks every collection.
- **The nightly** (`rt-freeze-check`, `scripts/rt-ci.py --freeze-check`) runs the conformance under gctorture with the check at every collection. The cases that need longer are named in `FREEZE_CHECK_SLOW` with their measured times. The every-collection check is moved to the nightly, not lost.

**Why 16 still finds a missed barrier.** A missed barrier stores an ordinary pointer into a frozen object without logging it. That leaves a state, not an event: an unlogged frozen object that refers to an ordinary one. The state lasts until the field is overwritten or the object is unfrozen. Any later checked collection sees it in the direct per-edge check, which names both objects, or in the comparison with the independent trace.

**What 16 can miss:** the field is overwritten within the 16 collections between two checks. If the ordinary object was reachable another way during that window, nothing was wrong at any collection, and no check sees it. If it was reachable only through the frozen field, an unchecked collection frees it while the frozen field still points to it. The every-collection runs (the crates' tests step, the nightly) catch both.

### (d) The leak rule, in its true form

The bar said a replaced base value leaks at most the frozen heap's size. That is true of the frozen objects themselves. It is not true of what they hold:

- **A dead logged frozen object retains its ordinary children as of its death.** These are the values its fields held when its last reference was cut. They are retained for the rest of the session, because the log is a root.
- **The bound is the number of logged frozen objects, not their size.** One logged hash table of a detached environment can hold a gigabyte of user data.
- **The frozen garbage itself** is never freed (bounded by the frozen heap, 1.7–1.9 MB under the rdb boot).

### What R sees: four cases (GNU R 4.6.2, run on #959's head)

| case | result | why |
|---|---|---|
| `freeze-garbage-options.R`: a boot option's cell (`dvipscmd`) given a finalizable environment, then removed with `options(dvipscmd = NULL)` | matches | robigo's `set_option` (`rtbuiltins/options.rs`) rebuilds `.Options` as a new list; no frozen cell is written |
| `capi/freeze-garbage-weak.R`: the same, with a weak reference keyed on the environment | matches | as above |
| `freeze-garbage-frame.R`: an existing binding in base's S3 methods table (unlocked, frozen) given a finalizable environment, then removed with `rm`, or reassigned | matches | the removal clears the cell (`chain::remove`, ported from `RemoveFromList`: CAR unbound, CDR nil); the reassignment overwrites it |
| `freeze-garbage-detach.R`: Autoloads, attached by the system profile and so frozen, given a binding to a finalizable environment, then `.AutoloadEnv` rebound and Autoloads detached | **differs**: GNU R finalizes at `gc()`, robigo-rt does not | the new binding cell is ordinary; Autoloads' hash table is frozen and logged, so it stays a root after the detach |

- **(d) is an INTERIM(H) deviation.** Its owner is H's next PR, the frozen-reachability pass, before "freeze at force". Until then the case has no ratchet marker.
- **The pass:** when a store into a frozen object, a rebinding of a freeze root, or a `set_enclos` reaching the frozen set has happened since the last pass, the next collection traces the frozen subgraph from the freeze roots. Every frozen object it does not reach becomes ordinary and is unlogged.
- **(a) and (b) match only because `set_option` rebuilds the list.** A faithful port of `SetOption`'s in-place `SETCAR` (options.c) would write into frozen cells, and the removed cell would keep its value, so (a) and (b) would differ too. `set_option` must not be ported in place before the pass lands.

### On main after #942: the compiler is frozen under the rdb boot (measured on the merge, 5074dcb23)

#942 runs `R_init_jit_enabled` at the end of startup under the rdb boot, and the compiler's namespace loads there when the JIT is on. The freeze runs after it, as #948's (a) places it: after `setup_Rmainloop`'s last step. So under the rdb boot the frozen set now holds the compiler. `loadedNamespaces()` at the first expression is `compiler base`. The interim loader, which the conformance and the nightly boot by default, is unchanged: `base`, and the same 5,289 frozen objects.

Each number below is beside its value before #942. Collector totals are callgrind instructions in `Core::collect_needing`, `R_DEFAULT_PACKAGES=NULL`, release binaries of main 894f1896a and of the merge.

| | before #942 | after #942 |
|---|---|---|
| **bar (e): `young.R`'s collector total under the rdb boot** (limit: within 10% of the interim loader's 12.3M, so at most 13.5M) | 13.25M, 6 collections: **holds** (+7.7%) | **16.55M, 6 collections: does not hold** (+34.6%). Main's rdb boot: 29.27M |
| of which the full collection at the freeze point (an empty script) | not measured separately | 5.04M under the rdb boot, 0.76M under the interim loader |
| `young.R`'s five collections after the freeze | – | 11.51M under the rdb boot, against 10.42M under the interim loader and 12.35M on main's |
| **bar (e): an explicit `gc()` under the rdb boot** (limit: within 10% of the interim loader's 0.59M) | `gccost.R` (21 `gc()`s): 7.34M in all | 10.47M in all. After the freeze point's collection: about 0.26M per `gc()`, so **holds**. Main's rdb boot: 74.8M |
| `young.R` + 100k live calls, rdb (reported) | 258.3M (main 266.6M) | 297.0M (main 317.6M) |
| `young.R` + 300k live calls, rdb (reported) | 920.9M (main 935.7M) | 924.6M (main 948.7M) |
| **frozen heap per isolate, rdb boot, JIT on** (premise 6's leak bound) | 20,836 objects. Premise 6 put it at 1.7–1.9 MB, by premise 1's method (live bytes with and without the roots) | **42,395 objects, 2.48 MB**, by the class sizes of the frozen objects themselves. The same method with the JIT off gives 20,504 objects and 1.16 MB, the pre-#942 set |
| frozen heap, interim loader | 5,289 objects, about 0.27 MB | 5,289 objects, 0.26 MB |
| **frozen share at the freeze point** (premise 1) | 93.9% (rdb: live bytes no longer reachable when the freeze roots are dropped) | **99.70%** of the mortal live bytes (rdb, JIT on). 99.37% with the JIT off, and 99.74% under the interim loader. This is a different method: frozen bytes over the live bytes that were not already immortal, at the freeze point |
| the nightly's every-collection check on its slow cases | `FREEZE_CHECK_SLOW`'s times | **unchanged.** The nightly's conformance boots the interim loader, whose frozen set did not change |

- **(e)'s `young.R` limit is missed, and it is reported, not re-tuned (the foreman's rule).** The miss is the one full collection made at the freeze point, which now traces the compiler's namespace once: 5.04M of the 16.55M. The five collections after it cost 11.51M, which is within 10% of the interim loader's 12.3M. A collection's cost after the boot is what the bar was written to measure. The total is what it signed, so the total is what this entry reports against.
- **The crates' gctorture step** checks every collection. Measured on the merge on the workstation (two runs each): 38 s and 35 s without the check, 48 s and 43 s with it, about 1.27×. That is about 0.12 min on CI's 0.45-minute step. So `modes` is at about 7.1 of its 9 minutes (`scripts/rt-ci.py`'s `BUDGETS`).
- **Carried into the pass PR, retaken on its head and not here:**
  - The unfreeze count and the cliff were measured with the compiler outside the frozen set. With it frozen, part of H's L5 condition is due now for the compiler's namespace: anything in the conformance or in the pinned packages that puts a finalizer or a weak reference on it, or on what only it reaches.
  - The dirty-event counts for forced promises now include the compiler's lazy-load promises.

### The freeze is lazy and off by default (the foreman's decision, 2026-11-22)

**Why.** The eager freeze above (one full collection, then the walk, at the end of every boot) cost every start, and no gate measured that. Main 9b812ab47 against 380ab42ce, `--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`:

| | main | the eager freeze | its cost | peak RSS |
|---|---|---|---|---|
| interim loader | 15.41M | 23.62M (+53%) | 8.12M | 9.7 → 10.1 MB |
| rdb boot, JIT on | 100.71M | 166.59M (+65%) | 66.12M | 15.4 → 20.7 MB |
| rdb boot, JIT off | 53.21M | 85.38M (+60%) | 32.47M | 13.3 → 16.0 MB |

Main makes no collection on that script, so the freeze point's collection replaced nothing. Under the rdb boot with the JIT on, the 66.12M splits into:
- the referrer thaw at the freeze's end (`thaw_with_referrers`, Sol S3): 39.43M, 60%;
- sorting the frozen list for `is_frozen_at`: 6.05M, 9%;
- the collection: 5.04M, 8%;
- the walk that sets the flags, and the logging: about 15.6M, 24%.

**What changed.**
- **`ROBIGO_GC_FREEZE=1` turns the freeze on.** Unset, empty or `0`, the default, it is off, and none of it runs. `ROBIGO_GC_FREEZE_CHECK` is read only with the freeze on. The default flips in the frozen-reachability pass PR, when the four clauses of (e) below hold with the pass in, measured on its head, together with case (d) ratcheted and the pass's own bar.
  - **Not on the performance clauses alone (the foreman, 2026-11-22):** `tests/rt/memory/freeze-garbage-detach.R` differs from GNU R 4.6.2 with the freeze on. That is an observable difference, fine behind the switch. The default cannot flip while the case differs, unless the pass PR makes it match or Max signs the divergence.
- **"The freeze point" is now two moments:**
  - **The boot boundary,** at the end of the boot, where the eager freeze ran (after `R_init_jit_enabled`). It only marks the blocks that hold objects as boot blocks: O(blocks), no object visited. Each records its bump position and sets its free list aside, so an object allocated afterwards is carved above that position or in another block. Large chunks are listed by base.
  - **The freeze,** inside the first collection after the boundary, before it marks. It freezes every boot object reachable from the freeze roots. An object allocated after the boundary is not frozen: it stays ordinary, and a frozen object that points to it is logged. Then the boot blocks' free lists are given back, before that collection's sweep adds to them.
- **Why the boundary exists:** without it, the first collection would also freeze what the session stored under the freeze roots before then. `options(x = <1e8 doubles>)` followed by `options(x = NULL)` would keep the 800 MB for the session (until the pass). With it, the frozen set is "boot-allocated, and still reachable from the freeze roots at the first collection", so boot objects that died before that collection are collected as usual.
- **A first boundary that retired the allocation blocks** (every class starting a fresh block) cost +5% of the interim loader's peak RSS at start. Keeping the blocks in use, with the bump mark and the free lists set aside, removed it.
- **The leak rule, (d) and the `SetOption` warning above** apply to the mode (`ROBIGO_GC_FREEZE=1`), not to the default.
- **The four frozen-garbage cases** now begin with `invisible(gc())`, so that with the freeze on, what follows is stored into frozen objects. GNU R 4.6.2's output is unchanged (re-recorded: only the `.Rsha` files changed).
  - `freeze-garbage-detach.R` matches with the freeze off and differs with it on. It stays unratcheted, because the freeze-on steps would fail on a marker.
  - Without the initial `gc()`, the lazy freeze would run at the case's own `gc()`. The freeze's referrer thaw then makes Autoloads ordinary (it reaches the finalizer's key through the new binding), so the case passed without testing anything.
- **CI runs the switch on** in:
  - the freeze's own tests (they turn it on: `set_freeze_enabled`);
  - the crates' gctorture step (check at every collection);
  - E3 (check at every 16th);
  - the nightly's every-collection run;
  - one conformance step under each boot with the freeze on and its check off: the mode users would turn on. Those two steps add about 0.3 min to `modes` (CI's plain conformance step takes 0.15 min).
- **`ROBIGO_GC_STATS`'s report** names the frozen set's objects and bytes.

**(e) amended, signed by the foreman:**

> **Cost, judged with the freeze on.** `--vanilla -e 1`'s total instructions and peak RSS are within 1% of main's under the interim loader and under the rdb boot with the JIT on and off. With `young.R` under the rdb boot, the collector's cost per collection after the first is within 10% of the interim loader's per collection on main. The first collection, the one that freezes, costs at most twice main's first collection under the same boot. `young.R`'s collector total, the first collection included, is reported beside main's.

**The startup clause's memory, as the foreman decided (2026-11-22):** it is judged on anonymous memory (peak `RssAnon`), within 1% of main's. Total peak RSS (`VmHWM`) is reported beside it, and may not grow by more than 5%.
- **Why:** the file-backed part of the peak is the binary's pages that fault-around makes resident. It moves with code layout and size (+0.4 MB here for 45.8 KB of code), is shared page cache, and says nothing about what a session allocates. The 5% keeps code bloat visible.
- **Scope:** the startup rows only (X's perf-rt startup rows). perf-rt's 2% peak-RSS rule on the N/2N rows stays as it is.

Measured on this PR's head (4112b9af4) against main b0c49e869, `--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`, the median of 61 interleaved runs, with the interquartile range:

| | anonymous (`RssAnon`), freeze off | freeze on | total peak (`VmHWM`), freeze off | freeze on |
|---|---|---|---|---|
| interim loader (main 1,964 kB [1,960–1,984]) | 1,984 kB [1,984–1,988] (+1.0%) | 1,984 kB (+1.0%) | 9,864 against 10,172 kB (−3.0%) | 9,844 kB (−3.2%) |
| rdb, JIT on (main 6,840 kB) | 6,840 kB (+0.0%) | 6,848 kB (+0.1%) | 15,832 against 15,980 kB (−0.9%) | 15,836 kB (−0.9%) |
| rdb, JIT off (main 4,648 kB) | 4,652 kB (+0.1%) | 4,652 kB (+0.1%) | 13,620 against 13,780 kB (−1.2%) | 13,600 kB (−1.3%) |

- **The interim loader's anonymous memory is +20 kB (five pages), at the 1% limit,** with the freeze off and on alike, so it is not the freeze's work. Where it is not:
  - malloc: DHAT counts the same allocations, +654 bytes in all and +271 at the peak;
  - the stack: `VmStk` is the same 132 kB;
  - `Core`'s size: main with 112 bytes of padding in `Core` reads 1,960 kB;
  - the writable sections: the same sizes, placed so that they span at most one more page.
- **Address randomisation moves a single run by about ±20 kB** (main alone: 1,952–1,988 kB over 25 runs).
- **Under the rdb boot** anonymous memory is within 0.1%, and total peak RSS is below main's under every boot.

**Measured on the lazy freeze** (this PR's last commit; release binaries of it and of main 9b812ab47; callgrind instructions; peak RSS the median of 21 runs, interleaved):

| clause | interim loader | rdb, JIT on | rdb, JIT off |
|---|---|---|---|
| `--vanilla -e 1`, freeze off, against main | +0.17% (15.43M / 15.40M); RSS +0.3% | −0.16% (100.72M / 100.88M); RSS −1.1% | −0.07% (53.17M / 53.21M); RSS −1.4% |
| `--vanilla -e 1`, freeze on, against main: **holds** | +0.19% (15.43M); RSS +0.8% | −0.12% (100.76M); RSS −1.2% | −0.05% (53.18M); RSS −1.4% |
| per collection after the first (`young.R` after a freezing `gc()`), against the interim loader's 2.45M on main: **holds** | 2.07M | **2.30M (−6%)** | 2.08M |
| the first collection (`invisible(gc())` alone), against main's: **does not hold** (at most 2×) | 8.11M against 0.78M (10.4×) | **63.55M against 5.06M (12.6×)** | 31.22M against 2.78M (11.2×) |
| `young.R`'s collector total, reported | 18.45M (main 12.35M) | 74.61M (main 29.23M) | 41.22M (main 19.09M) |
| frozen set | 5,288 objects, 270 KB | 42,395 objects, 2.48 MB | 20,504 objects, 1.16 MB |

- **The freeze off is not instruction-identical to main.** It is within 0.2% either way, the branch's other code included. With the freeze off, the startup runs none of the freeze's functions: one more environment variable read, `ROBIGO_GC_FREEZE`, and a call that returns.
- **The first-collection clause fails, as the foreman expected:** that collection pays the referrer thaw and the sort, which the pass PR removes by design. If it still fails with both gone, the walk's cost and folding the walk into that collection's marking go to the foreman before any build.

**The default users get (the freeze off), against main b0c49e869** (this PR's head; `--vanilla -e 1`, `R_DEFAULT_PACKAGES=NULL`):

| | interim loader | rdb, JIT on | rdb, JIT off |
|---|---|---|---|
| instructions, main → #959 (no extra variable set) | 15,530,764 → 15,550,565 (+0.13%) | 95,330,830 → 95,230,938 (−0.10%) | 50,760,669 → 50,948,155 (+0.37%) |
| the same with one more environment variable set (median of 8 lengths) | 15,528,810 → 15,549,745 (+0.13%) | 95,175,350 → 95,362,040 (+0.20%) | 50,846,203 → 50,742,056 (−0.20%) |
| anonymous memory (median of 15) | 1.91 → 1.94 MB (+1.2%) | 6.69 → 6.68 MB (−0.1%) | 4.53 → 4.54 MB (+0.2%) |
| peak RSS (`VmHWM`, median of 15) | 9.69 → 10.10 MB | 15.43 → 15.66 MB | 13.27 → 13.49 MB |
| file-backed pages | 7.78 → 8.17 MB | 8.75 → 8.98 MB | 8.73 → 8.95 MB |

- **What the freeze itself costs, switched off:** 19–112 instructions in its own functions (`freeze_boot_heap` returns at once), and about 500–1,080 for the one more `getenv` (`ROBIGO_GC_FREEZE`). A run is deterministic to about ±1k.
- **The rest is layout, not work:**
  - Under the rdb boot, the sign of the difference flips between the two rows with one environment variable added: the count depends on the environment's layout by about ±0.2–0.4%.
  - Under the interim loader, the +20k is glibc's `realloc` (Vec growth) copying more: the same 2,276 calls, but about 16k more instructions in the copy beneath it. Whether `realloc` grows in place depends on the malloc arena's layout, which the branch's slightly different allocations shift.
  - A first version of the lazy freeze handed the heap its roots through a wrapper type (`GcRoots`); that cost 55–152k instructions of codegen. `Roots` now carries its freeze roots itself (`#[trace(freeze = "trace_freeze_roots")]`, a form the derive gained for this).
- **Peak RSS:** the anonymous memory, the heap and every other data, is unchanged within noise. The peak's growth is file-backed: the binary's code pages a start touches. `.text` is 45.8 KB larger (the freeze and its check, compiled in with the switch off), which moves code across pages and readahead windows. On main alone that varies by about 0.3 MB from run to run.

### Note of 2026-11-24: the check in two groups, and E3's full checks (g3/h-check-cost)

**Why the check changed:** c-stack-overflow.R under E3 (S1's profile, quoted): "c-stack-overflow.R under E3 (gctorture, freeze, check at 16), main 96764d7fd: 670.8 G instructions for four C stack overflows at 2,975 lapply levels each, 110k forced collections. The split is work 0.1%, forced collections 71.8%, freeze check 28.0% (rdb boot: 50.3% and 49.6%). Each overflow costs collections (~9 per level) × trace cost (~200 ns per level of live frames), quadratic in depth. GNU R 4.6.2 under gctorture(TRUE) runs the script in 144 s at 1,751 levels and is ~8x slower per level; robigo-rt's case runs long only because it goes 1.7x deeper. B: the kept lapply call (#999) takes the case from 78.4 to 47.9 s locally (−39%), i.e. the compile per recursion level." (S1)

The comparison with the independent trace traced every root, the stack's included, with a hash set at every checked collection; I2 and I3 kept hash sets of the 42k frozen objects under rdb.

**The check is now two groups** (`ROBIGO_GC_FREEZE_CHECK=N[,M]`, M default 1, so every existing use keeps every part at every check):
- **The direct loop**, at every N-th collection: every frozen object's ordinary children are marked after marking. Its defect, a missed barrier, is transient: the next sweep frees the child, so the rate is the chance of catching a one-off miss. It stays at 16 in E3.
- **The full check, the persistent-state parts**, at the first collection with a frozen set (the freezing collection, checked whatever the period) and then at every M-th checked collection:
  - I2 (every frozen object reachable from the freeze roots): its defect, a missed note, lasts until the next pass, and passes are rare (0 in steady state, 0.006 per collection in the conformance). **The one exception:** a pass that runs between the bad write and the next full check thaws the evidence.
  - I3 (FROZEN set exactly on the frozen list): a stray FROZEN bit is never removed; a duplicate lasts until the next pass.
  - the comparison with the independent trace: its defect, a child marked but never traced, is systematic, so a run needs it at all, not often. Its cost grows with the live set.
- **E3 runs `16,16`**: the direct loop at every 16th collection, a full check at each process's freezing collection and every 16th check after (every 256th collection). It keeps all four parts. rt-ci counts the full checks (a tally line per full check in the report) and fails the step if there were none.
- I2 and I3 keep their sets in the pass's mark bits, which are clear between pass steps; the check clears every bit it sets. The I2 walk and the direct loop visit edges directly, without collecting them in vectors.

**Measured on c-stack-overflow.R**, `perf stat -e instructions:u`, E3's exact environment, before = main 6a6a923de, after = this change; rdb with a tree from scripts/rt-home-db.sh (42,422 frozen), m1 the default boot (5,205):

| boot | build | check | G instructions | the check's G | its share |
|---|---|---|---:|---:|---:|
| m1 | before | none | 483.0 | | |
| m1 | before | 16 | 670.3 | 187.3 | 27.9% |
| m1 | after | 16,16 | 500.6 | 17.6 | 3.5% |
| rdb | before | none | 453.0 | | |
| rdb | before | 16 | 897.4 | 444.4 | 49.5% |
| rdb | after | 16,16 | 493.0 | 40.0 | 8.1% |

- One comparison costs 39.5M instructions under rdb and 23.7M under m1 at this case's depth. The direct loop costs about 2.5M per check under rdb. It is O(frozen) by nature: only reading every frozen object's fields finds a write that bypassed the barrier.
- **The foreman's target** of under 5% under rdb was dropped. It stood in for the flip's criterion on this case (at most 800 G under rdb), which 493 G meets. A list of frozen objects with pointer fields was not built: it would be a second list to keep in step with the frozen one.

**Mutations:**
- A remembered-set entry removed: the direct loop names it at a checked collection that is not a full one.
- A stray FROZEN bit, and a frozen edge cut without a note, planted after a process's first full check: I3 and I2 name them at the next full check, 16 checks later, and none of the 15 before. These three are kept tests.
- **Marked, not traced** (a temporary source mutation: a logged object's ordinary children marked and never pushed): named by the full check and at period 1, and not by the direct loop.
  - Under rdb in E3's configuration, the freezing collection's full check names it.
  - Under m1 the log holds no such child at the freeze. The defect appears after a later store, and the process fails first with a wrong result (37 of 40 random rt cases fail, none named). E3 fails on it without naming it.

**So E3 does not name every marking defect.** It fails on this one under m1 (37 of 40 random rt cases, none named by the check). Detection is the step's job; naming the defect takes a rerun at period 1. **An E3 failure with an unexplained error under the freeze is rerun with `ROBIGO_GC_FREEZE_CHECK=1` before anything else** (docs/verification.md). There is no "first K checks" rule. Under rdb, the default after L's flip, the freezing collection's full check names it.
