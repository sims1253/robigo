## 2026-11-16: P1 step 10, E2 `fib` fix 2: argument code of a simple shape is computed directly

Fix 2 of `fib`'s three (`2026-11-16-p1-gaps.md`): "memoize a site's
argument shape". Step 6's conditions apply (`2026-11-16-p1-inline.md`).

- Base: `g3/p1-fib-decode` at `d41d3fa4` (#707). Change: branch
  `g3/p1-fib-argshape`, measured at `3bde32ef` (`robigo-rt`).
- Where the instructions went. On the base, forcing `fib`'s argument
  `n - 1` cost 497 instructions per call (`Frame::eval`). That was
  `code::cached`'s checks on the argument's byte code, then that code run
  over `pure`, the argument interpreter: four instructions, each fetched,
  unpacked and dispatched, with its constants decoded.
- What changes (`crates/robigo-rt/src/inline.rs`):
  - **Decoded once.** At its first force, an argument's code is decoded
    into a shape (`ArgShape`). The shapes are:
    - a variable (`GetVar; Ret`);
    - a variable and a scalar under one arithmetic operator, in either
      order;
    - two variables under one operator.

    The shape is kept on that code object's attribute word, as #707's
    table is, allocated without collecting. Code of no shape is marked
    once and runs over `pure` as before.
  - **The same semantics.** A force through a shape makes `pure`'s reads
    (`Scope::read`, with the instruction's slot index and the code's
    shape), its arithmetic fast path and its give-ups. Neither shape nor
    interpreter touches counts or `R_Visible`. A shape keeps constant
    indices, not pointers, so a moving collection leaves it valid.
  - **One word, two kinds of data.** A call's 4th word can hold closure
    code (an `Eval` closure's code on its body), and a table and a shape
    share the attribute word. So each reader checks the length of what it
    finds: a shape is 24 bytes, while a table is its bytes, 8-aligned,
    then whole steps. No `Eval` code gets a table today, so this is a
    guard.
- Admission at `3bde32ef`: `scripts/test-rt-ci.py` and `scripts/rt-ci.py`
  pass every step (conformance 258/263, main's 5 unratcheted failures).
  `cargo +nightly-2026-11-02 miri test -p robigo-rtcore`, `scripts/lint.sh`
  and `cargo fmt --check` pass too.
- Commands as in step 2 (`rt-bars.sh` twice, `s3` three more times,
  `rt-bars-compare.py --a fused ... --b argshape ...`).
- Raw JSON: `data/2026-11-16-p1argshape-3bde32ef/run{1,2}-*.json` and
  `run{3,4,5}-s3.json`, with the base's runs as `base-run*`.

Instructions and estimated cycles per iteration (medians of two runs;
`s3` of five):

| row | base instr | shapes instr | ÷ | base est. cycles | shapes est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,229.0 | 2,114.3 | 0.9485 | 2,232.2 | 2,117.5 | 0.9486 |
| defaults | 2,271.7 | 2,155.0 | 0.9486 | 2,274.9 | 2,158.2 | 0.9487 |
| named | 1,985.8 | 1,999.8 | 1.0070 | 1,988.2 | 2,002.2 | 1.0070 |
| env-fn | 2,600.1 | 2,602.2 | 1.0008 | 2,602.7 | 2,604.8 | 1.0008 |
| superassign | 5,180.5 | 5,071.4 | 0.9789 | 5,286.2 | 5,176.6 | 0.9793 |
| s3 (median of five) | 32,133.9 | 32,133.8 | 1.0000 | 37,569.7 | 37,802.8 | 1.0062 |
| fib | 15,009,580.6 | 13,128,041.9 | 0.8746 | 15,156,733.1 | 13,273,682.4 | 0.8758 |
| binarytrees | 6,221,440.0 | 6,221,457.4 | 1.0000 | 6,710,063.6 | 6,721,241.7 | 1.0017 |

- The geomean is 0.9689 in instructions and 0.9701 in estimated cycles.
- `plain`, `defaults` and `superassign` gain too: their callers' argument
  expressions (`i * 0.25`, `i * 0.5`, `i %% 7`) are forced through shapes.
- `named`'s arguments are a constant and a symbol, which never reached
  `pure`; its +14 instructions are code layout.
- Allocations are unchanged on every row.
- No row spreads more than 0.006% in instructions or 0.1% in estimated
  cycles over its runs.
- `s3` runs the same instructions with 0.6% more estimated cycles, code
  layout in callgrind's cache model again.

E2 after this step: `fib` 1,570 instructions per call against today's
robigo's 1,008 (1.56x; bar 1.0x, still unmet).

Checks of step 6's conditions:
- **Condition 1.** `tests/inline.rs` gains each shape and its fallbacks,
  with inlining on and off, and under gctorture:
  - integer overflow;
  - NA, compared as `is.na` (NaN equals nothing);
  - a string operand;
  - division by zero.
- **Condition 2.** `tests/rt/inline/argshapes.R`, recorded on GNU R 4.6.2:
  the same, with the warnings' and errors' messages and calls.
- **Condition 3.** As admitted above.

Review: Sol approved `1616b05d` with no findings. It ran the inline tests
under each collector setting and the inline cases, and its own
operand-order and arithmetic cases against GNU R 4.6.2. The length guard
(`3bde32ef`) came after that review.
