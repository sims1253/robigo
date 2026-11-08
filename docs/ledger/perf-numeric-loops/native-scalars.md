# Native complex scalars and bitwise calls on current main

Base: `2a5ff0b366bfe594e3a4e83df8fb144ce2345ff2` (PR #485).
Release binaries built sequentially with `bash scripts/build.sh` in the same
worktree. Host Callgrind; GNU R outputs and instruction counts from main CI
run `42389195288`, artifact `perf-main`. No full corpus or full performance
suite was run locally.

Complex loop variables and registers hold two f64 lanes. Constants, addition,
subtraction, multiplication, negation, absolute value and length are native.
The value buffer reserves both words and uses unaligned 128-bit loads/stores.
Entry checks reject attributed values and vectors. Exit and deoptimization
restore both parts; first-assignment flags retain their existing behavior.
NaN arithmetic results and NA integer operands leave before assignment, so the
VM preserves GNU R's NaN payloads and complex infinity recovery. Complex
arguments and captured values in whole-closure code remain boxed.

The six bitwise internals use the existing `CallBuiltin` instruction. Native
loop analysis recognizes these calls, including the bodies of base's guarded
bitwise closures. No bytecode compiler or VM dispatch opcode was added.
`relop.c` defines these operations in GNU R 4.5.0 (there is no `bitwise.c` in
that release). Integer coercion follows `coerce.c:IntegerFromReal`; conversions
that warn leave to the VM. NA operands and shifts outside 0..31 produce
NA_INTEGER, and right shift is unsigned. Traced primitives disable this path.

An int-or-double value can pass as a double only to a wrapper that returns the
result of one bitwise internal. A wrapper that returns its argument must keep
the tag. The first candidate exposed a mismatch between this type analysis
and the loop-specific call generator; the existing loop-bitwise and
loop-split-types fixtures caught the panic. The generator now applies the
same normalization. The new fixture also covers a wrapper that returns its
argument, bitwNot, rebinding, and complex restoration at a return.

## Verification

- Existing loop-complex and loop-bitwise recordings were regenerated with
  `robigo-oracle:4.5.0`; their recorded bytes did not change.
- New `tests/r/jit/loop-bitwise-not.R` recorded in the same container.
- Native-compilation unit test checks that complex and bitwise loops have
  a compiled region with no unconditional leaves, rather than merely passing
  by falling back to the VM.
- Runtime unit tests: 100 pass. The two existing failures,
  `builtins::complex::tests::c_library_oracle` and `c_library_trig_oracle`,
  reproduce on untouched base with identical actual/expected bits: complex
  sqrt and csin each differ by one ULP on this host. No builtin code changed.
- All three runtime integration tests pass.

## Remaining work

`ForSeqLength` inside a native region remains open. Moving its method checks
requires auditing method lookup against pending VM assignments and possible
promise forcing; this change leaves the VM's checks in place. Complex
powers/division, complex inlined arguments, and broader scalar coercions
also retain their current fallbacks.

The shipped awf fixture has size 14, below the 16-back-edge threshold, and
runs too few pixel-loop entries to reach the 1024-back-edge heat threshold.
It therefore does not measure native bitwise code. A separate measurement
runs the same pinned awf source with `print(execute(100L))`; both binaries'
output is compared with GNU R 4.5.0 in the container. The heat policy is
unchanged.

## Final release gate

All 14 selected programs match the GNU R oracle. `perf.py compare` reports
one faster, zero slower above 0.5%, zero broken. Full counts are in
[native-scalars.csv](native-scalars.csv).

- Scalar-complex Mandelbrot: 736,058,072 -> 281,868,693 instructions (-61.7056%).
- awf/Mandelbrot at size 100: 216,103,308 -> 98,610,134 instructions (-54.3689%).
  Both print `[1] 239`, matching the separately run GNU R 4.5.0 container.
- The standard size-14 awf benchmark stays within 0.5%; its outer loops
  never become hot enough to exercise the new lowering.

Commands for the standard slice and five regression controls:

```sh
python3 scripts/perf.py run --metric instr --runtime robigo \
  --oracle .robigo/perf/oracle/perf.json --jobs 2 \
  --filter 'mandelbrot|Mandelbrot|convolution|bitwise|robigo/(closures|na-entry-alternation)|rbench/shootout/nbody_naive$|rbench/simple/(scalarFor|vectorFor)$' \
  --out .robigo/perf/final486.json
python3 scripts/perf.py compare .robigo/perf/base486-all.json \
  .robigo/perf/final486.json --threshold 0.5
```

The base dataset combines two runs of the same release base binary: the
nine requested targets and the five controls. For the size-100 pair, append
`print(execute(100L))` to the pinned RBenchmarking awf source and run each
binary with `R_DEFAULT_PACKAGES=NULL valgrind --tool=callgrind`; compare
stdout with `Rscript --vanilla` in `robigo-oracle:4.5.0`.

Kept both changes. No compiler, builtin, environment, or interpreter source
files changed; the whole-closure JIT only keeps complex arguments/cells in
its existing boxed representation. No loop-heat policy changes.

Final validation on the copied release binary: all nine conformance harness
tests pass with `ROBIGO_CONF_FILTER=tests/r/jit`; all JIT ratchets hold and the
new fixture is ratcheted. `compiled_matches_interpreted` ignores that filter
and compares every recorded script across interpreter, VM, and VM+JIT; it
also passes. `cargo fmt --check`, both `scripts/lint.sh` passes,
`scripts/ratchet.py check`, and `git diff --check` pass. The largest instruction
increase in the 14-program gate is `nbody_naive`, +0.03075%.
