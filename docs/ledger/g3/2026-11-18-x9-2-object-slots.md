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

`tests/rt/capi/unbound-store.R` (Sol round 1): a native `.Call` can
return `R_UnboundValue`. The direct store leaves it to `define_var`, whose
error "attempt to bind a variable to R_UnboundValue" GNU R raises too.
It is recorded on 4.6.2 for a new binding and an existing one, and
ratcheted. GNU R's byte-code SETVAR into an existing cell
(`SET_BINDING_VALUE`, eval.c:2736) has no such check, where robigo-rt's
`define_var` always checks. That difference predates X9-2, is unchanged by
it, and is a low.

**Checks at `b1f31e6a`:**
- debug `cargo test -p robigo-rt -p robigo-rtcore`: 313 pass;
- `cargo fmt --check`, `scripts/test-rt-ci.py`, the full `scripts/rt-ci.py`
  and `scripts/lint.sh` pass. The conformance step is 590/596; the 6
  failures are main's unratcheted cases (five `bytecode/*` and
  `dynload/library-dynam-tools`);
- Miri on rtcore: 143 tests pass.

Sol (gpt-6.1-sol, high): round 1 found S1 (`R_UnboundValue`), which is
fixed; round 2 approved.

## Re-measured on main `2f16dc4c` (2026-11-21), and the store out of `vm::run`

The 31-PR batch moved main under this branch. Merged with it, the inline
object store cost the call rows +0.10 to +0.12% (about 2 instructions, all in
`vm::run`, which inlines `set_var`). `vm.rs` now calls
`frame_store_object_raw` through `#[inline(never)] store_object_slot`. It
costs a call only on an object store, and about 280 instructions are saved
per store.

Measured against the same tree without X9-2, both binaries built from
their exact trees into empty targets: base `a336aa64` (5d plus main
`2f16dc4c`), against `2db831df`. perf-rt ran twice per binary:

| row | ÷ |
|---|---:|
| plain | 0.9995 |
| defaults | 0.9993 |
| named | 0.9997 |
| env-fn | 0.9994 |
| superassign | 1.0001 |
| s3 | 0.9999 |
| fib | 1.0000 |
| binarytrees | 0.9998 |
| trycatch | 0.9689 |
| trycatch-err | 0.9703 |
| trycatch-ok | 0.9627 |
| geomean | 0.9908 |

Per cmp.R compile: `jit-scale_by` 9,909,566 → 9,767,938 (0.9857) and
`jit-safe` 14,746,459 → 14,490,305 (0.9826).

Then main `5e5a27d2` (#886, which brings 5d onto main) was merged. The
branch's diff against main is X9-2's two files alone.
