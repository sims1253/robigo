# A byte-code promise of a plain symbol, read with no activation (step 1b, 2026-11-23)

X/P1. Under the rdb boot, base's code is byte code, and a compiled caller
passing a variable on makes a byte-code promise of a plain symbol. Its
translation is two instructions, `[GetVar a=R_FIRST fl=F_VIS e=<sym>
f=<slot>, Ret b=R_FIRST]`: measured with a temporary probe in `code::build`.
Forcing it ran a full `vm::exec`:
- the value-stack check;
- a register frame;
- the depth step, bcEval's entry poll and the entry safepoint;
- byte code marked active;
- the dispatch loop, the read and the return.

All of that for one variable read.

## What changed

- **The mark.** `code::build` sets the header's spare byte, now
  `single_read`, on exactly that shape (`Bc` profile, the read's only flag
  `F_VIS`, the return's none, its visibility untouched).
  `code::single_read` reads it from the header directly, not through
  `View`, which `vm::run` reads.
- **The read.** At the two places byte code is entered for a promise
  (`eval_in`'s BCODESXP arm and `bc_eval_inline`), marked code goes to
  `eval::run_single_read`. It does what `vm::exec` does for it, in the same
  order:
  - the depth step, which errors at the limit as `exec` does;
  - `bc_tick`, then `safepoint(Entry)`;
  - byte code active (`bc_activation(true)`), inside whatever `eval_in`
    saved (S1's save/restore, #983);
  - the read's visibility;
  - GETVAR's fast read: the frame slot by the compiled index through the
    code's shape. An unboxed scalar is boxed as `Ret` boxes it;
  - otherwise the slow path at the read's own depth (`entry + 1 + d`): a
    formal not read yet is forced in place, else the folded `get_var_in`;
  - the depth back.
- **The fold** (#986's acceptance items):
  - `get_var` and `get_var_keepmiss` are one body, `get_var_in::<KEEPMISS,
    P>`, over a `ReadPlace`: an activation's registers (`ActPlace`), or a
    rooted environment and an out slot (`OutPlace`, the single read's);
  - one depth predicate, robigo-rtcore's `Cx::force_depth_overflows`;
  - `inline_force_overflow`'s VALUE arm is gone: only a delayed slot reaches
    it, since `force_arg` hands a value slot's promise to `force_promise`,
    which checks for itself;
  - passing the read's kind into the force was measured and not done (the
    ledger of #986 has why).

## Measured

Release builds of main df437f6bf and this branch, each in an empty target.

**`vm::run` is byte-identical:** 0x7a65 bytes both, 0 differing lines after
normalising addresses, symbols and comments. `get_var` and
`get_var_keepmiss` keep their signatures and are still called out of
line.

Rows, callgrind instructions per iteration, main → this branch:

| row | interim boot | rdb boot | rdb, `ROBIGO_RT_PROFILE=bc` |
|---|---:|---:|---:|
| trycatch | −1.0 (0.00%) | **−2,400 (−2.14%)** | **−2,537 (−2.26%)** |
| lapply | +101 (+0.09%) | **−399 (−0.37%)** | −165 (−0.16%) |
| binarytrees | +0.7 | −10 | +7 |
| fib | −3 | +1 | −45 |
| s3, plain, named, defaults | ±0.3 | ±0.2 | ±0.3 |
| d0-paste, d0-paste0 | 0 | 0 | 0 |
| guard-subset-dbl, -matrix | ±0.1 | ±0.2 | ±0.1 |

Re-taken on main 5d961809e, the boot heap's freeze on by default (#992),
rdb boot, this branch rebased onto it:
- trycatch −2,369 (−2.13%), lapply −185 (−0.17%);
- every other row ±0.3, the guard rows ±0.0;
- `vm::run` byte-identical (0x7a75 bytes both).

The gain is where base's byte code forces promises of plain symbols:
tryCatch's chain of closures (`tryCatch`, `tryCatchList`, `tryCatchOne`,
each passing `expr` on) and lapply's. Under the interim boot base is
evaluated from source and no such promise is made: no effect, within
±101. d0-paste does not move. An earlier reading of a +572 per-promise
cost there, under C1's probe, does not reproduce on today's code.

## The case

`tests/rt/jit/single-read-promise.R`, recorded on GNU R 4.6.2. It covers:
- a value and a compiled loop's unboxed scalar;
- visibility;
- a missing argument;
- an active binding read through the promise;
- a promise whose own expression errors;
- an argument forced once through two reads;
- a deep chain of single reads, and the evaluation-depth limit.

Under the rdb boot, `run_single_read` ran about 1,000 times in it, and the
output is GNU R's. rt-ci runs it there ("conformance under the rdb boot").
Under the default boot it does not reach the new path.

## Rooting, found by Sol

The first version passed the code object to `run_single_read` as a raw
pointer and read it after `bc_tick` and the entry safepoint, either of
which can collect. Under a moving collector the code had moved by then.
The code is now passed rooted and bound only after both, as `vm::run`
reads its code register after them.

The case, under the rdb boot with `ROBIGO_GC_MOVE=young:16 R_GCTORTURE=1`
(a collection at every safepoint, objects moved; the freeze on too, on
5d961809e):
- the first version dies with a bus error (exit 135);
- this one prints GNU R's output.

rt-ci's GC_MOVE step runs the conformance under the default boot, which
never reaches this path, so the gate alone would not have caught it. The
run above was by hand.
