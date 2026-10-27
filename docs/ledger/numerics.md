# numerics ledger

Software 80-bit long double everywhere, its speed, and the exact/fast
numeric-policy seam. Metric: (1) bit agreement with x87 over property
tests, operations covered; (2) instructions per operation; (3) policy
kernels implemented and benchmarked.

## 2026-10-28: runtime policy kernels, inherited WIP reviewed

- why: the runtime had no fast-mode reduction kernels or common policy API.
- what: `policy::Accumulator` selects `LDouble` or `f64` statically after
  one entry-point match. Exact delegates to the existing kernels without
  changing them. Fast reductions use four lanes; prefix scans are sequential.
  The trait preserves accumulator precision across chunks and columns.
- inherited WIP: kept the four-lane reduction idea. Rejected its global
  atomic because interpreters must be independent, and its `add_each` API
  because an f64 buffer rounds exact state between columns. Replaced its
  benchmark: the 16-element add_each input was divided by 8192, setup was
  included in instruction counts, and mean/prod/dev/cumprod were missing.
- result: **0 -> 9** policy operations measured. All eight existing slice
  kernels and corrected mean have fast implementations. Kept. Release
  numerics tests: 36 passed, including 4096 generated vectors with mixed
  exponents, cancellation and large offsets, all prefixes, and row sums.
  Bounds include conditioning and exclude intermediate overflow/underflow;
  range-limit tests explicitly cover differences outside those conditions.
- measurement: x86_64, Rust 1.99, 8192 elements, 300 iterations under
  callgrind, matched empty-operation process/setup counts subtracted.
  Wall time is the minimum of five runs of 10000 iterations, ns/element,
  on the shared VM; instruction counts are the gate. Disassembly contains
  `addpd`, `subpd`, and `mulpd` for the fast loops. Reproduce with
  `scripts/ldouble-callgrind.sh` after building the examples under the
  shared build lock. `ldouble_baseline` is the independent direct-kernel
  reference measured before implementation; its instruction costs match
  the policy exact kernels within 0.05%.

| kernel | exact ins/element | fast ins/element | speedup | exact ns/element | fast ns/element | speedup |
|---|---:|---:|---:|---:|---:|---:|
| sum | 4.002 | 1.192 | 3.36x | 1.654 | 0.229 | 7.24x |
| prod | 4.002 | 1.192 | 3.36x | 1.769 | 0.253 | 6.98x |
| mean | 10.012 | 3.323 | 3.01x | 3.589 | 0.476 | 7.54x |
| cumsum | 6.005 | 2.753 | 2.18x | 1.540 | 1.078 | 1.43x |
| cumprod | 6.005 | 2.753 | 2.18x | 1.669 | 1.049 | 1.59x |
| dev | 6.003 | 1.693 | 3.55x | 1.665 | 0.228 | 7.29x |
| sqdev | 7.003 | 2.629 | 2.66x | 1.837 | 0.232 | 7.91x |
| cross | 10.006 | 3.381 | 2.96x | 1.726 | 0.231 | 7.47x |
| add_each | 7.003 | 2.754 | 2.54x | 3.043 | 0.253 | 12.04x |

## 2026-10-27: soft powl — glibc 2.41's x86_64 powl, ported

- why: `LDouble::powl` called glibc's assembly; off x86_64 (aarch64,
  wasm32) there was no long-double `powl` at all, and R's `format.c`
  divides by `powl(10, kp)` for |kp| > 22 — the exact bits matter for
  printing.
- what: `ldouble/soft/pow.rs` ports glibc 2.41 operation by operation:
  `sysdeps/x86_64/fpu/e_powl.S` (special cases, exact binary powering
  for integer |y| < 4 incl. the asm's |acc|/base·|base| multiply
  sequence, |y| saturation to ±2^-79/2^78 with the asm's actual
  polarity, odd-integer tests via the exact fprem y mod 2^63) and
  `sysdeps/x86/fpu/powl_helper.c` (frexpl split, the −log(k/16) table
  with exact 64-bit encodings, the w_hi series with acc_split, base-2
  conversion, y split at 32 bits, roundl half-away) — verified
  intermediate-by-intermediate against the compiled glibc helper.
  `f2xm1` and `fyl2xp1` are replaced by correctly-rounded software
  (Taylor/Mercator series on a 128-bit-significand core with guard/
  sticky rounding), since the hardware microcode is *not* correctly
  rounded (measured vs `__float128`: 1.6–3.2% of results off by 1 ulp,
  uniformly across all exponents, the error a smooth deterministic
  curve — a rounded polynomial in the microcode; bit-exact replication
  is open).
- result: soft vs glibc over 251,165 cases (powl(10, kp) kp ∈ [−340,
  340], a specials matrix, 250k random pairs incl. denormals, huge/tiny
  y, integers ≥ 2^63): **0 differ by more than 1 ulp; 275 (0.109%)
  differ in the last bit** — exactly the cases where glibc's own
  f2xm1 microcode error survives the +1.0 absorption (frac ∈ [−0.5,
  −0.415]). powl(10, kp): 679/681 bit-exact (2 one-ulp). Specials and
  integer |y| ≤ 3: bit-exact. x86_64 keeps calling glibc (imp::powl),
  so CI conformance is unchanged. Cross-builds green:
  aarch64-unknown-linux-gnu, wasm32-unknown-unknown. kept.
- debugging trail (for the next agent): four bugs found by tracing
  against the instrumented C: `ld`'s biased exponent off by 63 (pow2
  cancelled it), `mul128`'s mid carry counted as 1 instead of 2^64,
  `fyl2xp1`'s even-term sign for negative x (`s || k%2==0`, not xor),
  and `roundl`'s |x|<1 branch. The instrumented-glibc-helper harness
  is in /tmp/opencode/helper_dbg.c (regenerate from glibc sources if
  lost).

## 2026-10-27: software long double core, verified against the FPU

- why: exact-mode numerics silently fell back to `f64` off x86_64, so
  `sum`, `cumsum`, `var`, ... produced different bits than GNU R.
- what: `ldouble/soft.rs`, a software x87-semantics 80-bit type (add,
  sub, mul, div, sqrt, rint, cmp, from_f64/from_i64/to_f64, the loop
  kernels); non-x86_64 `imp` uses it. Property tests on x86_64 compare
  every operation against the x87 asm path bit for bit (1M random
  inputs per generator + edge pairs + alignment sweep + NaN rules;
  `ROBIGO_LDPROP_N` tunable).
- result: before — no software path (off-x86_64 results simply wrong);
  after — 29/29 tests pass, 0 mismatches over ~2·10⁷ comparisons
  (6.8 s release). Mutation check: flipping the round-to-even tie in
  `pack` fails `arithmetic_matches_x87` immediately, so the oracle
  wiring is live. kept.
- verification of the inherited WIP: the previous agent's branch was
  unverified; re-reviewed `soft.rs` (NaN selection, denormal single
  rounding, the `de == 65` add shortcut), rebased on current `main`,
  mutation-tested, then rewritten into the two commits above.

## Open

- bit-exact `f2xm1`/`fyl2xp1`: reverse-engineer the x87 microcode
  polynomial (its error curve is smooth; sample with the /tmp/opencode
  probes) to close the last 0.109% divergence of soft powl.
- callgrind measurement of the software ops; make them fast.
- numeric-policy seam (exact/fast) for the ldouble kernels + benchmark.
