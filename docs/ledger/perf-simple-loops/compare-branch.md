# perf-simple-loops: branching on comparisons in native loops (round 4)

- Why: loops the closed form (#324) cannot remove run at the speed of
  their native code. `while (x < n) x <- x + 0.5` took ~3 cycles an
  iteration where its `fadd` chain needs ~2 (Zen 5). Native code made an
  R logical of every comparison (`select` for NA) and then tested it at
  the jump: two compares, a `cmov` and three branches per `while`/`if`
  test.
- Change (`jit.rs`, used by the loop and the closure tier):
  `fusable()` finds a `Rel`/`RelL` with scalar operands whose register is
  the condition of the very next `JumpIf`/`JumpIfNot`, that jump not being
  the target of any other native jump; `Gen::compare_branch` branches on
  the comparison. NA (unordered or an NA integer) sets the register to NA
  and leaves native code at the jump, whose error the VM raises.
- Two corrections, both found by measuring every program:
  - The 0/1 logical was still made for the register (it is live at later
    deopt points): `seta` and a repeated `vucomisd`, vectorWhile +5.9%.
    Now only if an op after the jump may read the register
    (`live_after`: a forward search over the rest of the code; at its end
    only the code's result register counts).
  - The jump's own block, unreachable, was filled with a jump to its
    deopt block: a second way in, so SSA passed every live variable to it
    through block parameters. robigo/mandelbrot +9%. Not generated now.
- Rejected: rotating loops (a copy of the test before the body, the body
  falling into the test at its end: one taken branch an iteration). No
  gain on the while loop, +60% on a `for` loop with an `if` (worse
  register allocation around the duplicated `ForStep`).
- Result (peak): `while (x < n) x <- x + 0.5` 3.5 -> 3.15 ms (-10%), a
  `for` loop with `if (i %% 3L == 0L)` 4.3 -> 3.5 ms (-19%),
  rbench vectorWhile -2%, robigo/closures -6%, robigo/fib -5%.
  Instruction counts: 1 faster (vectorWhile -2.5%), 0 slower.
- Verified: differential tests (NA conditions on doubles and integers,
  `if` and `while`, `!(a <= b)`, the closure tier) match GNU R 4.5.0 in all
  tiers; conformance; fastr comparison/control-flow slice; sol-loop 3
  rounds, approved.
