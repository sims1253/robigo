# numerics ledger

Software 80-bit long double everywhere, its speed, and the exact/fast
numeric-policy seam. Metric: (1) bit agreement with x87 over property
tests, operations covered; (2) instructions per operation; (3) policy
kernels implemented and benchmarked.

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
