## 2026-11-23: torture after the start, and the rdb boot under gctorture

Two prerequisites for making the rdb boot the default: the torture steps
torture each case but not its start, and the start keeps a step of its own.
All measurements were taken on main 92d933c4b (#979's translation cache in),
with this PR's binary, on the 24-CPU workstation at load 19-50.

**The start, once** (the start-torture step's script: six lazy-loaded fetches
and a loop the JIT compiles):

| | M1's loader | rdb boot |
|---|---|---|
| plain | 0.0 s | 0.0 s |
| `ROBIGO_GC_MOVE=1` + gctorture from the heap's creation (the start-torture step) | 0.1 s | 8.3 s (12.7 before #979) |
| the same, held until after the start | 0.1 s | 0.4 s |

- **What the rdb start does** (`-e 'invisible(1)'`, GC stats): 73,411
  allocations. Tortured, that is 7,233 collections and 174,307,592 objects
  moved. Held, it is 2 collections. M1's loader: 9,121 allocations, 76
  collections tortured, 2 held.
- **`R_VSIZE=4M R_MAX_VSIZE=16M`:** the rdb start fits, so the small-vector
  step needs no change.

**The gctorture conformance step under rdb:**

| run | pass | wall | seconds of the other 748 cases |
|---|---|---|---|
| start tortured | 740/750 | 10.33 min | 6,030 |
| torture held (`ROBIGO_TORTURE_AFTER_START=1`) | 740/750 | 10.63 min | 4,271 (−29%) |

The two 600 s timeouts below set the wall time of both runs.

**Eight of the ten failures are not new.** They fail in both runs and on main
too (run 45108072544, in every conformance step, M1's loader included),
which leaves two:

- `alloc/link-lifetime`, `deparse/base-promise-op`
- `jit/base-guard-symbol-value`, `jit/bc-expression-replacement`,
  `jit/serialize-write-shadow`
- `jit/translate-warning-asname`, `-frames`, `-warn2`

**Two ratcheted cases time out under rdb with gctorture, held or not.** The
cause is the cost of each collection, and the boot heap's freeze removes it:

| case | M1, held | rdb, freeze off, held | rdb, `ROBIGO_GC_FREEZE=1`, held | rdb, #992 (632ae6a9d, freeze on by default) |
|---|---|---|---|---|
| `tests/r/lang/tailcall-frames.R` | 105.0 s | 928.6 s | 56.5 s | 71.0 s |
| collections | | 1,104,756 | 1,104,756 | 1,109,987 |
| `tests/r/memory/finalizers-self-ref-envs.R` | 289.2 s | > 1,800 s (killed) | 122.8 s | 122.0 s |
| collections | | (killed) | 2,409,436 | 2,414,667 |

- The collection count is the same in every configuration, so the cost is
  per collection. With the freeze off, each forced collection traces the
  boot heap's ~42,500 objects. The freeze takes them out: 42,494 objects,
  2.6 MB.
- With the freeze, both cases are faster under rdb than under M1's loader
  today.
- No allowance is needed and nothing in the cases needs fixing. The flip
  depends on #992, which it already did.
- After #992 merges, the gctorture step's minutes are to be taken again
  under both boots, since the freeze shortens the interim boot's too, and
  the shard's budget set from them.

**The start-torture step catches what the held mode misses.** Each mutation
was made once and reverted, never committed, and each was run under the rdb
boot.

- **Mutation 1, in `lazyLoadDBfetch`:** the unserialized value is kept as a
  raw address across a safepoint, and that copy is returned. Plain passes.
  Held fails, because the script's own `cat` goes through the same fetch.
  The start-torture step segfaults (exit 139) at the start's first fetches.
- **Mutation 2, at the top of `after_profile`:** this runs once in the start
  and no script re-enters it. A preserved string's raw address is kept
  across `.OptRequireMethods`. Plain passes and held passes. The
  start-torture step fails with exit 101, the address reading the poison
  type (219). M1's loader never runs `after_profile`, so all three pass
  there.
- My first form of mutation 2 used an explicit `cx.collect()`, which broke
  even the plain run: a forced collection ignores the start's GC-inhibited
  stretches. A safepoint collects only when one is due.

**The heap's rules for the hold** (H's review of #998):
- `ROBIGO_GC_MOVE`'s bookkeeping runs from the heap's creation, and only its
  moves wait: `SEEN_BY_C` is marked in `hand_to_c`, and the freeze stays off
  under any move mode.
- The young:N count starts at the arming.
- gctorture's counters are held only if gctorture is on at the hold.
- A held gctorture gives way to any setting the start makes itself, whether
  `gctorture(FALSE)` or another gap. An NA gap sets nothing and keeps the
  hold.

**H's note, unchanged by this PR.** Under young:N the OLD bit is FROZEN's
bit. So a store into an OLD object goes through `store_slow` and may note
DIRTY. That is harmless, because the frozen list is empty under the mover.
