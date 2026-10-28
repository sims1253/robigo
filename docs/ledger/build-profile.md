# Build profile: codegen-units and LTO (in progress)

Question (coordinator, 2026-10-28): which release profile gives the
fastest robigo, how much codegen-unit noise (the #46 near-gate
`#[inline]` cases) it removes, and what it costs in build and test
time. Measured on main b6fc7c8 (after #46). Each variant is a cargo
profile inheriting `release` (`[profile.pN]`, its own
`target/pN`), built in the `robigo-oracle:4.5.0` container with a
private target dir (`~/.cache/robigo-profile-target`), `-j 6`.
Instructions: `scripts/perf.py run --metric instr --runtime robigo`,
78 programs, in the container.

## Instructions (robigo, all 78 programs, against today's release)

| profile | geomean vs release | faster >0.5% | slower >0.5% | best | worst |
|---|---|---|---|---|---|
| release today (cgu 16, no LTO) | 1.0000 | 0 | 0 | +0.0% Bounce | +0.0% vecops |
| codegen-units = 1 | 0.9911 | 69 | 5 | -13.6% spectralnorm_alt_2 | +10.5% flexclust |
| lto = "thin" | 0.9729 | 77 | 0 | -5.3% strings | -0.2% vecarith |
| lto = "fat" | 0.9818 | 49 | 1 | -13.7% spectralnorm_alt_2 | +2.3% vecarith |
| codegen-units = 1 + lto = "fat" | 0.9632 | 74 | 3 | -15.2% spectralnorm_alt_2 | +9.4% flexclust |
| codegen-units = 1 + lto = "thin" | 0.9667 | 74 | 2 | -15.1% spectralnorm_alt_2 | +7.9% flexclust |

- thin LTO is the only variant that is faster everywhere (77 of 78
  faster by more than 0.5%, none slower; worst -0.2%).
- `codegen-units = 1` (alone or with LTO) wins the most on some
  programs (spectralnorm_alt_2 -15%) but loses on others: flexclust
  +7.9..+10.5%, plus 2-5 more slower than 0.5%: one partition does not
  suit every hot loop either; LLVM's inliner just decides differently.
- Combinations stack: cgu 1 + fat 0.963, cgu 1 + thin 0.967.

## Clean build time (robigo-cli, all dependencies, wall seconds)

Load average during the builds was 19-38 on 12 cores (other agents), so
these are rough.

| profile | clean build |
|---|---|
| release (p0) | lost (timer bug), rerun pending |
| cgu 1 (p1) | lost, rerun pending |
| thin LTO (p2) | 463 s (load 38); 460 s (load 21) for the second build |
| fat LTO (p3) | 639 s (load 29) |
| cgu 1 + fat (p4) | 626 s (load 20) |
| cgu 1 + thin (p5) | 448 s (load 20) |
| release, second build (variant tree) | 310 s (load 26) |

## Not measured yet

Paused on 2026-10-28 at the coordinator's request: the 14 GB machine was
under memory pressure (systemd-oomd killed another agent twice), and LTO
builds are among the most memory-hungry jobs.

- Noise: robigo built without #46's `#[inline]`s on `Frame::get`,
  `get_mut`, `real_op`, `int_to_real` (worktree `wt-noinl`); built
  for release (310 s) and thin LTO (460 s), not yet measured; fat and
  cgu-1 variants not built. Then vectorFor, vectorWhile, superWhile,
  convolution_slow, fib, nbody with and without the attributes per
  profile.
- Incremental rebuild after touching one runtime file, per profile.
- `cargo test` (conformance) time per profile.
- Peak memory of each build (relevant now: the machine is shared).
