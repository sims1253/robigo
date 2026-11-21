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
