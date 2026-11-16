## 2026-11-16: P1 step 9, E2 `fib` fix 1: a qualifying body's instruction groups are decoded once into fused steps

The foreman's second item: `fib`'s fixes 1-3 from `2026-11-16-p1-gaps.md`,
each measured as its own step. This is fix 1, "decode a qualifying body
once". Step 6's conditions apply (`2026-11-16-p1-inline.md`).

- Base: `g3/p1-superassign` at `2591f576` (#704's code). Change: branch
  `g3/p1-fib-decode`, measured at `d41d3fa4` (`robigo-rt`).
- Where the instructions went. A line profile of `fib(18)` on the base
  put about 190 instructions per call into loading and unpacking
  instructions (`View::at`). Another 110 went to the dispatch `match`, 31
  to flag tests and 50 to register traffic, over about 10 instructions per
  call, plus each argument's own code.
- What changes (`crates/robigo-rt/src/inline.rs`):
  - **Decoded once.** At its verdict, qualifying code is decoded
    (`decode`) into a table of fused steps, each standing for a group of
    instructions:
    - `CMP_BR`: `GetVar` of a formal, `LdScalar`, `Relop`, `BrFalse`;
    - `CALL_FUN`: `GetFun`, then `Call` of the closure;
    - `RET_ARG`: `GetVar` of a formal, `Jmp` to its `Ret`;
    - `ARITH_RET`: `Arith`, then its `Ret`.

    Only instructions without evaluation-point flags are fused, and only
    where no jump lands inside a group. `fib`'s taken path is then four
    steps where it was ten instructions, and its base case two where it
    was seven.
  - **The same semantics.** A step runs what its instructions run. The
    formal's read is `slot_read`, which forces as `GetVar` does. Where a
    step's precondition fails (an operand that is no plain number), the
    instructions run one by one, or the run gives up where they would.
  - **Kept on the code.** The table is the code object's attribute word:
    a one-cell pairlist whose CAR is a RAWSXP, with one byte per
    instruction and then the steps. The collector traces and moves it
    with the code. It is allocated without collecting, and nothing is
    kept if the allocation is refused. A copy for other formals
    (`code::reshape`) gets no table and a fresh verdict.
  - **What it costs elsewhere.** A table costs every instruction one
    byte's test, so it is kept only where a compare-and-branch or a call
    is fused, and the loop without a table is a second copy that tests
    nothing. The first version cost the plain-call rows 1.5-3.5%, as the
    larger loop changed what the compiler inlined. Forcing `str_eq`'s
    helper and `Frame::formal` inline, and keeping the generic `GetVar`'s
    own copy of the read, brought them back within 0.5%.
- Admission at `d41d3fa4`: `scripts/test-rt-ci.py` and `scripts/rt-ci.py`
  pass every step (conformance 257/262, main's 5 unratcheted failures).
  `cargo +nightly-2026-11-02 miri test -p robigo-rtcore`, `scripts/lint.sh`
  and `cargo fmt --check` pass too.
- Commands as in step 2 (`rt-bars.sh` twice, `s3` three more times,
  `rt-bars-compare.py --a superassign ... --b fibdecode ...`).
- Raw JSON: `data/2026-11-16-p1fibdecode-d41d3fa4/run{1,2}-*.json` and
  `run{3,4,5}-s3.json`, with the base's runs as `base-run*`.

Instructions and estimated cycles per iteration (medians of two runs;
`s3` of five):

| row | base instr | fused instr | ÷ | base est. cycles | fused est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,235.1 | 2,229.0 | 0.9973 | 2,238.2 | 2,232.2 | 0.9973 |
| defaults | 2,279.6 | 2,271.7 | 0.9965 | 2,282.6 | 2,274.9 | 0.9966 |
| named | 1,980.7 | 1,985.8 | 1.0026 | 1,983.2 | 1,988.2 | 1.0026 |
| env-fn | 2,592.1 | 2,600.1 | 1.0031 | 2,595.6 | 2,602.7 | 1.0027 |
| superassign | 5,155.3 | 5,180.5 | 1.0049 | 5,260.1 | 5,286.2 | 1.0050 |
| s3 (median of five) | 32,141.3 | 32,133.9 | 0.9998 | 37,677.1 | 37,569.7 | 0.9971 |
| fib | 17,899,577.7 | 15,009,580.6 | 0.8385 | 18,038,099.7 | 15,156,733.1 | 0.8403 |
| binarytrees | 6,221,448.6 | 6,221,440.0 | 1.0000 | 6,632,683.3 | 6,710,063.6 | 1.0117 |

The geomean is 0.9787 in instructions and 0.9801 in estimated cycles.
Allocations are unchanged on every row. No row spreads more than 0.006% in
instructions or 0.36% in estimated cycles over its runs.

`binarytrees` runs the same instructions but 1.2% more estimated cycles,
more than its spread. None of its closures runs inline, so the cause is
the binary's code layout in callgrind's cache model, not work.

E2 after this step: `fib` 1,795 instructions per call against today's
robigo's 1,008 (1.78x; bar 1.0x, still unmet). Fixes 2 and 3 are next.

Checks of step 6's conditions:
- **Condition 1.** `tests/inline.rs` gains a test taking the steps'
  fallbacks: a complex comparand, NA in the branch, an integer overflow
  returned, unforced arguments. It passes with inlining on and off, and
  under gctorture.
- **Condition 2.** `tests/rt/inline/fused.R`, recorded on GNU R 4.6.2:
  the same fallbacks, with their errors' and warnings' messages and calls.
- **Condition 3.** As admitted above.

Found on the way: robigo-rt cannot order strings under the runner's
collation (`"a" < 2L` errors that ICU collation is not implemented), and
its base has no `print` method for condition objects. The case uses a
complex comparand, and prints conditions' message and call.

Review: Sol approved `d41d3fa4` with no findings. It ran the inline tests
normally, under gctorture and under the moving collector, and the inline
cases. Its own scripts against GNU R 4.6.2 covered tables warmed under the
moving collector, operand fallbacks, NA, complex values and integer
overflow.
