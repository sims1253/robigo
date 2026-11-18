# X9-2: compiled local stores of objects go straight into the frame's slot (2026-11-18)

X/P1. From the cmp.R profile
(`~/.cache/robigo-research/x-cmpR-profile/README.md`) and E1/S1's trycatch
split: 20 of 22 local stores per caught error, and 642 per cmp.R compile,
store objects. `vm::set_var` sent only unboxed scalars into the closure
frame's slot (`frame_store_scalar_raw`); objects went through the generic
`Cx::define_var`, about 361 instructions each, against GNU R's 77-190.
P1 frames (#681, 072da236) limited the direct store to scalars as a scope
choice, with no recorded correctness reason.

## The change

- rtcore: `Cx::frame_store_object_raw`, the scalar store's object twin. It
  takes the slot by the same rule:
  - a slot frame of that shape, or one keying the slot by the same symbol;
  - a value, boxed or unboxed, not locked, assignment-pending or aliased;
  - or unbound in an unlocked frame.

  Anything else returns false and takes `define_var`. For a slot it takes,
  it does what `defineVar` does there (envir.c): the global cache flush,
  the special-symbol bit, the frame's version bump when the new value or
  the old one is function-like, a new binding's stamp (`frame::bind`, for
  `ls()` order), the counted and barriered store (the old object's count
  dropped), and `SET_MISSING(binding, 0)`.
- `vm.rs` `set_var` sends a register's object there, as it sends scalars.
  It adds no op, argument or caller.
- `frame::store_value`'s two reachable cases (VALUE; REAL/INT/LGL) are
  written out in the new store instead of called. A second caller made
  LLVM stop inlining `store_value` into `Loc::store`, which is `<<-`'s
  path: superassign rose +0.25% (+13 instructions) until then. This is the
  rule of `robigo-vm-run-codegen`: a function the hot path inlines gets no
  second caller.

## Measured

Measured on a local merge, because C1's 5d (`origin/g3/v4-5d-return`
`bdab1d6d`) did not contain #867:
- base: 5d plus main `24be2c15` (`05c31ee8`);
- this: the same plus X9-2 (`b8282e4f`).

Both binaries were built from their exact trees into empty targets, in
one worktree. The sha256 are in `data/2026-11-18-x9-2-object-slots/jit.txt`.

**perf-rt** (`scripts/rt-bars.sh … --rows '^(trycatch|trycatch-ok|trycatch-err)$'`,
twice per binary):

| row | base instr | this | ÷ |
|---|---:|---:|---:|
| plain | 1,898.7 | 1,899.8 | 1.0005 |
| defaults | 1,983.0 | 1,983.8 | 1.0004 |
| named | 1,841.2 | 1,842.1 | 1.0005 |
| env-fn | 1,936.7 | 1,937.6 | 1.0004 |
| superassign | 4,913.4 | 4,913.7 | 1.0001 |
| s3 | 18,986.5 | 18,987.5 | 1.0000 |
| fib | 10,711,681.6 | 10,711,796.4 | 1.0000 |
| binarytrees | 3,500,869.1 | 3,501,428.6 | 1.0002 |
| trycatch | 114,459.1 | 110,815.6 | 0.9682 |
| trycatch-err | 175,578.7 | 170,252.2 | 0.9697 |
| trycatch-ok | 51,064.8 | 49,105.9 | 0.9616 |
| geomean | | | 0.9909 |

The call rows pay about one instruction. Allocations are unchanged.

**cmp.R compiles** (callgrind, N = 800 minus N = 400, per compile):

| row | base | this | ÷ |
|---|---:|---:|---:|
| jit-scale_by | 10,351,793 | 10,205,131 | 0.9858 |
| jit-safe | 15,524,781 | 15,259,495 | 0.9829 |

**Per store** (callgrind on the trycatch row, N = 2000 minus N = 1000):
`define_var` calls fall from 14.5 to 1.5 per iteration, at about 361
instructions each. The iteration saves 3,644 instructions, about 280 per
store, so the new path costs about 80, within GNU R's 77-190.

## Correctness

`tests/rt/env/compiled-object-store.R` is recorded on 4.6.2 and
ratcheted. Each shape runs in the closure's first call and in later
(byte-code) ones:
- `ls(sorted = FALSE)` after new bindings and a rebinding (GNU R:
  `"c" "a" "b"`);
- a locked binding's error, with the value kept;
- a function-valued local rebound and called (the version bump);
- an S3 method reassigning its argument (an aliased slot, which keeps the
  long way);
- a reassigned formal with `missing()`;
- copy-on-modify of a shared value;
- a loop's growing local.

It matches GNU R in plain, gctorture, `GC_MOVE` with gctorture,
`NO_INPLACE`, `MATERIALIZE` and `FRAME_QUARANTINE`.

**Checks:** CHECKS
