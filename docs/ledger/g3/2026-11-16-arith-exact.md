## 2026-11-16: scalar arithmetic: GNU R's NaN or NA per path, and the lost-modulus warning

- What: the foreman's package, two exactness gaps Sol found on #714
  (`2026-11-16-p1-fib-typed.md`, "Found on the way"). Both occurred with
  inlining off too.
- Tree: branch `g3/arith-exact`, begun on #714's head `fc1da8e4`, then
  merged with main `1f1dbd81` (#714). Binary: `cargo build --release
  --locked -p robigo-rtapi` (`robigo-rt`). Machine: AMD Ryzen 9 5900X (24
  threads), WSL2 Linux 6.18. GNU R: the 4.6.2 oracle image.

### 1. Which of NA and NaN survives

On x86-64, a double `+` or `*` of two NaNs keeps the destination
register's payload, and gcc picks the destination per C function.
`kernel::arith::x_is_dest` records GNU R's choices. With a scalar double
against a scalar integer, the choice depends on the path:
- **The AST** (top level, closures GNU R never compiles, and the
  compiler's constant folding, which evaluates as `eval` does) goes
  through `do_arith`'s scalar block. It makes y the destination, so
  `NaN + NA_integer_` is NA.
- **Byte code** (compiled closures, their compiled callees, top-level
  loops): an NA integer leaves `Arith2`'s fast path for `cmp_arith2`.
  That calls `R_binary`, not `do_arith`, and its mixed-type loop makes x
  the destination, so the same sum is NaN.
- `-`, `/`, `^`, `%%` and `%/%` agree on both paths.

robigo-rt used the AST's choice in compiled code too.
- **The fix:** `ops::arith2` takes its code's profile (`bc`). The VM's
  `Arith` passes `v.profile == Profile::Bc`. The early arguments, the
  inliner's `pure` and its argument shapes pass `true`, since they run
  only byte code. The inliner's body loop and fused steps pass their
  code's profile.
- **NOJIT callees:** robigo-rt inlines callees that GNU R never compiles.
  GNU R runs those in the AST, so they keep the AST's choice. The probe
  covers that case.

### 2. "probable complete loss of accuracy in modulus"

`myfmod` warns when the quotient is finite and past 1/LDBL_EPSILON, once
per element. It runs for `%%` on doubles and for `R_pow`'s `myfmod(y, 2.)`
in `(-Inf)^y`, which the byte code's fast `^` reaches too.
- **What already worked:** the builtin's `real_binary` counted those
  elements and warned, so vectors warned.
- **The gap:** every scalar went through `ops::arith2`, which computed the
  value without the check. That covered top-level code (Eval-profile
  `Arith`), compiled code, the early arguments and the inliner.
- **The fix:** `kernel::arith::real_op_lost` returns the value and the
  loss, in one match on the operator. `real_op` and `real_binary` are now
  built on it. `arith2` returns "slow" for a loss, as it does for an
  integer overflow. The VM then takes the builtin's path, which warns;
  the early arguments and the inliner give up, and the real call warns.
- **The reported call:** each warning names the call GNU R names: the
  closure's call in compiled code (`R_getBCInterpreterExpression` skips
  builtins), the context's call in the AST, and none at top level.

### Proof

- **Cases:** `tests/rt/arith/na-nan-order.R` and `mod-accuracy.R` are
  recorded on 4.6.2 and ratcheted.
  - `na-nan-order.R` covers the seven operators against eleven pairs of
    NA, NaN, logical NA and short vectors. Each is run through `do.call`,
    a NOJIT closure and a compiled closure. A compiled caller runs
    compiled and NOJIT callees. It also covers folded constants and a
    top-level loop.
  - `mod-accuracy.R` covers the warning's boundary (`1e19 %% 1` warns,
    `1e18 %% 1` does not; likewise `(-Inf)^2e19` and `(-Inf)^1e18`), the
    integer and logical operands, `%/%`, vectors with one warning per
    element, each context with the call it names, and the top level's
    deferred warnings.
  - Both fail on #714's binary and pass on this one, with
    `ROBIGO_NO_INLINE=1` and with inlining on.
- **The JIT off:** with `R_ENABLE_JIT=0`, GNU R 4.6.2 and robigo-rt print
  both cases byte for byte the same, with inlining on and off. This run is
  not ratcheted:
  - the conformance runner sets no environment per case;
  - robigo-rt reads its JIT level once (`jit::Config`) and has no
    `.Internal(enableJIT())`.

  The cases' AST sections, which are what the JIT off gives, are
  ratcheted. A runtime `enableJIT` is a follow-up for the JIT module.
- **Cost:** measured with `scripts/rt-bars.sh` twice on main `1f1dbd81`
  and twice on `72d67ec0`, compared with `scripts/rt-bars-compare.py`.
  These are instructions and estimated cycles per iteration, as medians
  of two runs. No row spreads more than 0.003% in instructions over its
  runs, and allocations are unchanged on every row.

| row | main instr | arith instr | ÷ | main est. cycles | arith est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,028.4 | 2,018.4 | 0.9951 | 2,038.9 | 2,028.9 | 0.9951 |
| defaults | 2,091.1 | 2,087.1 | 0.9981 | 2,101.8 | 2,097.7 | 0.9981 |
| named | 1,945.8 | 1,942.8 | 0.9985 | 1,956.0 | 1,953.1 | 0.9985 |
| env-fn | 2,579.2 | 2,583.1 | 1.0015 | 2,581.5 | 2,585.6 | 1.0016 |
| superassign | 5,075.0 | 5,056.3 | 0.9963 | 5,180.9 | 5,235.7 | 1.0106 |
| s3 | 21,695.2 | 21,700.2 | 1.0002 | 24,855.4 | 24,697.2 | 0.9936 |
| fib | 12,072,805.9 | 12,090,851.3 | 1.0015 | 12,191,294.7 | 12,214,608.6 | 1.0019 |
| binarytrees | 6,232,831.9 | 6,234,874.5 | 1.0003 | 6,526,871.5 | 6,517,410.4 | 0.9986 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 0.9989 | | | 0.9997 |

  - **What it took:** the first version, with `real_op_lost`'s loss checks
    inline at every site, cost `plain` +0.70% in instructions. The checks
    for `%%` and `^` now live in two small out-of-line functions, and the
    `+ - * /` arms are as small as before.
  - **`superassign`'s estimated cycles (+1.06%):** its work went down:
    instructions 4,984 to 4,965 per iteration, data reads 1,603 to 1,549.
    Its L1 data read misses went up from 5.2 to 25 per iteration (10
    cycles each in the estimate), in the VM's reads of code objects. These
    misses depend on where data sits:
    - padding the environment by 2,500 bytes alone moves `main`'s own
      misses from 5.2 to 1.2 per iteration (0.8% of its cycles);
    - this branch's two builds, which differ only in where the `%%` and
      `^` checks sit, gave +1.53% and +1.06%;
    - the branch stayed 12 to 21 misses per iteration worse than `main`
      at all four paddings tried.

    So the cache placement changed, not the work, but these runs do not
    show it as noise. This is for the foreman to weigh; instructions are
    within the limit on all three rows.
- **CI scripts:** `scripts/rt-ci.py` passes every step on `72d67ec0`:
  - **Conformance:** 267/272 in every mode, and 266/271 under
    `ROBIGO_NO_INPLACE=1`. The five failures predate this branch and have
    no markers: `args/binding-types`, `env/binding-errors`,
    `env/closure-frames`, `env/hashed-env` and `env/unhashed-env`.
  - **Other steps:** the release, gctorture, moving-collector,
    pin-report and frame-quarantine tests; the stress script; the
    overshoot of every slice; and E9.
- **Review:** Sol (gpt-6.1-sol, high) approved round 1 with no findings.

Raw JSON: `data/2026-11-16-arith-72d67ec0/` (`main-run{1,2}-*.json` and
`run{1,2}-*.json`).
