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
