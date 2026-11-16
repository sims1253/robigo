## 2026-11-16: E2 decision: `fib` is unmet in M1, owed to the native tier (G8)

- **Bar:** `docs/g3.md` §15.3, E2: "fib, binarytrees: ≤ today's robigo
  (JIT off) in instructions and estimated cycles". For `fib` that is today's
  robigo's 1,008 instructions per call (`2026-11-16-p1-gaps.md`), 8,428,736
  per iteration of the row (`../g3.md`, "M1 baselines").
- **Evidence:** `2026-11-16-p1-fib-entry.md` (#722, X/P1).
  - After its two steps, an inline `fib` call costs 1,317 instructions
    (1.31x).
  - The cuts still visible in the profile add up to about 1,147 (1.14x),
    not 1.0x.
  - **The cause of the rest:** robigo-rt forces a nested call's arguments
    lazily in the callee, as GNU R does. Each force makes GNU R's
    evaluation counts (`early_force`). That costs about 245 instructions a
    call. Today's robigo evaluates such arguments eagerly at the call site.
  - Eager evaluation would be sound only where it is proven unobservable:
    the same counts at the same points, and no warning or error moved.
  - The entry recommends recording `fib` as unmet at about 1.3x, owed to
    the native tier.
- **Today:** `fib` 1.305x today's robigo (10,996,510 instructions per
  iteration), on main `bac15367` with `scripts/perf.py rt-bars`, on this
  machine (AMD Ryzen 9 5900X). CI's `perf-rt` on the same commit measured
  1.305x too (run 43879794229).
- **Decision (Max, as lead, 2026-11-16):**
  - E2's `fib` is recorded as unmet in M1, at about 1.3x today's robigo.
  - It is owed to the native tier for hot pure closures (G8).
  - It is not an accepted regression. `fib` never met its bar, so no
    ceiling is set: `benchmarks/rt/m1-bars.toml` keeps it `met = false`,
    and the `perf-rt` gate still fails a PR that makes it more than 0.5%
    slower than main.
- **What follows:**
  - **E2 in M1:** binarytrees meets its bars; `fib` does not. The M1 exit
    evidence (`2026-11-16-m1-exit-evidence.md`, E2) records this
    decision.
  - **The cheap cuts:** the three cuts of #722's entry are worth taking only
    if they cost little elsewhere (about 13% of `fib`). Owner: P1.
  - **The bar itself:** owner G8.
