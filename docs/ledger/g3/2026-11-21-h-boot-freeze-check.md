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
