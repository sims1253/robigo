# The byte-code translator's maps on a fast hasher (2026-11-21)

X/P1, item #3 of the start-up plan for the rdb boot (L's #922;
`~/.cache/robigo-research/x-boot/PLAN.md`). The code is C1's
(`crates/robigo-rt/src/compile.rs`, `compile/bctrans.rs`).

## What was slow

robigo-rt translates GNU R byte code into its own IR at a closure's first
call (`args::prepare` → `compile::compile_closure_bc` → `translate_code`).
The boot from base.rdb calls 60 such closures, and translating them cost
10.99M of its 66.84M instructions: about **183k per closure**. GNU R runs its
byte code directly and pays nothing here. About 5.2M of the 10.99M, roughly
70k per closure, was SipHash: the standard library's hasher, in the
translator's `HashMap`s, whose keys are byte-code positions, IR indices and
the constant pool's object addresses (`Comp::pass`, `Comp::k`,
`translate_ins`, `finish`).

## The change

`compile.rs` defines `PosHasher`, rustc-hash 2's multiply
(`0xf1357aea2e62a9c5`) with its final `rotate_left(26)`, and `PosMap`/`PosSet`
on it. The translator's maps and sets use them. The keys are `usize`s the
translator makes itself, never chosen by R code, so the hasher does not need
SipHash's resistance to attack. Iteration order can change, but std's
`RandomState` already changes it in every process, so nothing may depend on
it; Sol found nothing that does (the one accumulated list is sorted before
use).

## Measured

**On the rdb boot**, with the patch applied to #922's head `2ca69586` in a
local commit:

| | before | after |
|---|---:|---:|
| boot (`ROBIGO_RT_BOOT=rdb --vanilla -e 1`) | 66,841,525 | 61,589,667 (−7.9%) |
| `translate_code` (60 closures) | 10,987,499 | 6,652,465 |
| per closure translated | ~183k | ~111k |
| SipHash in the translator | 5.23M | 0.21M (other callers) |

For V4 step 3's JIT-cost bar (C1): translating a closure at its first call
costs about 111k instructions after this change. Before, it was about 183k:
about 70k of hashing and about 113k for the rest (decoding, the passes'
work, emitting).

**perf-rt** against main `386a0358`, twice per binary:
- every call row is within ±0.01%;
- `trycatch` −0.83% and `trycatch-err` −1.07% (cmp.R's closures translated
  in the error path);
- `fib`, `binarytrees` and `superassign` 0.00%; geomean 0.9982;
- per cmp.R compile, `jit-scale_by` −0.29% and `jit-safe` −0.24%.

Raw JSON in `data/2026-11-21-bctrans-hash/`.

**Checks at `79736736`:**
- debug `cargo test -p robigo-rt`: 142 pass;
- `cargo fmt --check`, `scripts/test-rt-ci.py` and `scripts/lint.sh` pass;
- the full `scripts/rt-ci.py` passes all but one step. Under
  `ROBIGO_RETRY_TORTURE=1`, four ratcheted cases timed out locally, under a
  load average of 45-50: `bytecode/cmpfun-ops`, `bytecode/cmpfun`,
  `bytecode/fidelity` and `serialize/compile-pkgs`. These are the known
  timeouts on main (S1 measured `cmpfun-ops` at 599 s of its 600 s and
  `compile-pkgs` at 601 s alone), which L moves out of that step. A rerun of
  that shard alone timed out only on `compile-pkgs`. CI's run of the shard
  is the proof.

Sol (gpt-6.1-sol, high), round 1: approved, no findings.
