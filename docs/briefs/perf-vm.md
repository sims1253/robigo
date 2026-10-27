## Your track: `perf-vm` (the VM and the JIT)

robigo should be faster than GNU R on every benchmark. `scripts/perf.py` runs robigo's own suite (`benchmarks/suite`) and Ř's RBenchmarking (Are We Fast Yet, the shootout, micro-benchmarks, real programs; `scripts/perf.py fetch` first) against GNU R, by callgrind instructions (the CI gate) and wall time. Wrong output does not count as fast.

Known gaps (`docs/design.md` §4.2): recursive closure calls (fib, binarytrees) cost ~10-30% more than GNU R's: each call allocates an environment, a promise per argument, the argument vector and a boxed result, and entering the VM has a fixed cost. Candidates: forced promises built by the caller for arguments that cannot fail or run code, unboxed results for compiled callers, cheaper frames, VM-to-VM calls without Rust recursion, a whole-closure JIT (`docs/design.md` §5.6, G8).

1. Measure: `scripts/perf.py run --metric instr` (both runtimes) and record the table in your ledger. Note benchmarks that fail or print wrong output (`wrong`): those are bugs to report (or fix, if in your scope).
2. Hill-climb the geometric mean of robigo/GNU R instructions over correct programs, biggest losers first; verify every change with `scripts/perf.py compare before.json after.json` (no benchmark more than 0.5% slower) and the conformance tests (`cargo test --release -p robigo-cli --test conformance` checks the interpreter, the VM and the VM with JIT against each other).
3. Profile with `scripts/callgrind-diff.py` and `scripts/idiom-vs-r.py`.

You own `crates/robigo-runtime/src/{vm,compile,jit}.rs`, plus minimal changes to `interp.rs`/`env.rs`/`value.rs` where a calling-convention change needs them (called out in the PR). Metric: instructions geomean robigo/GNU R, number of benchmarks faster than GNU R, and wall time on an idle machine.
