## 2026-11-21: the tools' temporary directories, and the day's CI in numbers

**What leaked.** The shared workstation's `/tmp` held 28 GB of tool
leftovers at 21:00: 857 `a4-rt-*` (builtin-diff), 302 `rt-bars*`
(perf.py), 120 `robigo-rast*`, 41 conformance fakes, 38 `capibench*`. A1
removed 1,275 of them by hand (15 GB: older than six hours, nothing inside
newer, no process using them). The tools removed their directories on
success only (builtin-diff, after all rows), or never (perf.py's `rt-bars`
and `rt-gnur`, rast.py, the C API bench, the runner's fakes), and none on a
signal: no tool installed a SIGTERM handler, so a CI cancel, `timeout` or a
killed delegate left everything.

**Why one `a4-rt-*` is 1 GB.** One case's record:
``[<-`(a4cls, 1:1e5, long)`` returns about 10^8 elements, saved whole
(1,001,603,231 bytes; the other 4,400 records' median is 35 bytes), with two
GNU R twins in the oracle cache. The digest-instead-of-value PR is next.

**The fix** (`scripts/scratch.py`, two rules):
1. A tool removes its own directories on every exit path: a normal return,
   an exception, `sys.exit`, Ctrl-C, SIGTERM and SIGHUP. The two signals end
   the tool's children first, and wait while a directory is being made or
   removed. A directory that cannot be removed does not stop the others. A
   child made by fork removes none of its parent's.
2. At a tool's start, a sweep by age alone: that tool's prefix, this user's,
   the directory's own time more than 24 hours old. No gate runs for a day,
   so a live run is never a candidate. It deletes through a descriptor of
   the directory it judged, and re-checks the inode at the name before
   removing the entry.

What rule 2 gives up: a killed run's leftovers stay for up to a day instead
of until the next run. With the digest PR a leftover stops being a gigabyte,
so a day of them is small.

**How it got here.** The first design swept by "not in use", from the
outside: /proc, modification times, modes, then an flock lease. Seven Sol
rounds each found a real hole, every one since round 3 in the same place:
deciding at a given moment that a directory in a shared `/tmp` belongs to
nobody. That is a distributed protocol, and the task did not need it. The
foreman took the lease out. The exit-path findings (handlers installed at
import, signals deferred during creation and removal, the fork rule, a
failed removal not stopping the rest) stayed.

**The local gate's three profile trees.** `scripts/rt-ci.py` builds
`quick` (the crates' tests) and `release` (the binary, the conformance);
`scripts/lint.sh` is clippy, a check-only build in `dev`, which CI's lint
job also uses (its `cfg(debug_assertions)` code lints differently under
`quick`). On A1's target: `debug` 1.2 GB (clippy only), `quick` 2.6 GB,
`release` 3.4 GB. The 4-6 GB `debug` trees come from a plain `cargo test`,
which the rt gate does not run. So a delegate who gates rt work keeps
`quick` and `release`, plus a check-only `debug` for lint, with
`CARGO_INCREMENTAL=0`.

**#961's CI run (44955010369, e97077b8d), the first with per-case times.**
The cases over a third of their limit under gctorture (CI, a GitHub-hosted
runner, beside the workstation's numbers alone, A1/B/C1, 2026-11-21):

| case | plain gctorture (600 s) | young:16 (limit) | young:16 alone, workstation |
|---|---:|---:|---|
| `conditions/c-stack-overflow.R` | 297.2 s | 658.7 s (1,200) | 134-270 s |
| `jit/serialize-write.R` | 172.5 s | 211.5 s (1,200) | 503 s (B), 185 s (A1) |
| `memory/finalizers-self-ref-envs.R` | 112.7 s | 183.4 s (1,200) | 346 s (B), 267 s (A1) |
| `jit/lazy-promise-code.R` | 141.1 s | 180.9 s (1,200) | 347 s (A1), 266 s (C1) |
| `jit/active-binding-read.R` | 116.7 s | 142.5 s (1,200) | 302 s (B) |
| `jit/serialize-write-shadow.R` (not ratcheted) | 89.6 s, fails | 112.7 s (600), fails | 526 s (loaded gate) |
| `jit/symbol-encoding-compiled.R` | 82.6 s | 103.1 s (600) | |
| `capi/closure-mutation.R` | | 97.9 s (600) | 177 s (A1) |
| `lang/tailcall-frames.R` | 53.8 s | 95.3 s (600) | 152 s (A1) |
| `jit/shared-body.R` | 86.0 s | 92.6 s (600) | 200 s (A1) |
| `bytecode/jit.R` | 51.8 s | | 111 s (A1) |

CI's runner took less than the loaded workstation's alone times for most
cases, but not for `serialize-write.R` (211.5 s against A1's 185 s) or
`c-stack-overflow.R`. That case, at 658.7 s, is the only one over half its
limit, and in the plain step, at 297.2 s, it is just under. Its deep recursion is what varies
most between runs (about 360 s, about 600 s and over 600 s in three CI runs
earlier today), so it keeps its 1,200 s. In the `modes` shard the longest
case of every conformance step is `env/large-frames.R`, 5.6-6.3 s of 20.

**The day's CI in numbers.** This morning one `rt` job ran every rt step in
30-38 minutes against a 40-minute limit, and a PR run took about 200 runner
minutes. #961's run: 181 runner minutes over 39 jobs, 28.7 minutes wall,
and the longest rt shard (`rt-gctorture`) 19.3 minutes; the others: `rt`
8.4, `rt-feature-builds` 14.6, `rt-builtin-diff` 11.6, `rt-debug-assertions`
3.9. The critical path is now `rt-gctorture`, with `perf` (16.0) next.
