# Ledger: profile-guided optimization of the release binary

All numbers are callgrind instruction counts on the 85 benchmarks of
`scripts/perf.py`, against a `cargo build --release` of the same commit
(c4e71bed), compared with GNU R 4.5.0's library installed. Output was
identical in every case.

| training | geomean | worst | slower by >0.5% |
|---|---|---|---|
| half the benchmarks, the other half measured (01ca4547) | 0.870 | +6.7% | 2 of 42 |
| vendored base only, as on a machine without GNU R | 0.957 | +14.8% | 7 |
| all benchmarks with GNU R's library | 0.856 | +4.0% | 1 |

Kept the third. The second, the simple design, is why training moved to the
oracle's container: most of a short R program is loading packages, and a
profile that never saw it optimizes the wrong code.

Costs: two more builds. Locally 514 s against 164 s and 2.5 GB peak rustc
memory against 2.0 GB; the job runs on nightly only, so no pull request pays.

Rejected without a build: PGO in the pull request gate, because a profile
that moves with every commit would swamp the 0.5% threshold.

Measured and rejected (wall time, 74 benchmarks, best of 12 runs):
`panic = "abort"`, 3.7% faster but within its own noise, and it would remove
`bridge.rs`'s guard that turns a panic in a C callback into an R error;
`-Ctarget-cpu=x86-64-v3`, no gain overall, and one benchmark 2.7 times slower.
