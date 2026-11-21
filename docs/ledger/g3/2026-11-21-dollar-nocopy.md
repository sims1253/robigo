# `$` reads a list's names in place (2026-11-21)

X/P1, the item after X9-2 in the queue that came out of the cmp.R profile
(`~/.cache/robigo-research/x-cmpR-profile/README.md`, "`$`, `[[`, `[`").

## What was slow

`subset3_dflt`, `$`'s default method (crates/robigo-rtbuiltins/src/subset.rs),
copied every name of the list it scanned, twice:

```rust
to_native(text(nm), locale).map(|b| b.into_owned()).unwrap_or(nm.bytes().to_vec())
```

`unwrap_or`'s argument is evaluated, and so allocated, before the
translation's result is looked at. Then `into_owned` copies the borrowed
native bytes. The input name was copied the same way. GNU R's `pstrmatch`
(subset.c:1179) reads each name in place: `translateChar` returns the
CHARSXP's own bytes when no translation is needed.

## The change

- Each name is now matched through `to_native`'s `Cow`, borrowed when it is
  already native. The fallback on a translation error, the raw bytes, is
  kept as it was.
- The input name is copied once (`unwrap_or_else`). It must outlive the
  `&mut Cx` uses that follow.

Behaviour is unchanged. One divergence was found on the way and is left
for its owner, because it is not this PR's: GNU R's `translateChar` errors
on a "bytes"-encoded name, where robigo-rt falls back to the raw bytes.
`$`'s scan past such a name and `as.pairlist` of one show it. The case and
GNU R 4.6.2's output are in `~/.cache/robigo-research/g3-foreman/delegates/X-dollar-bytes.R`.

## Measured

Against main `19886a17`, both binaries built from their exact trees into
empty targets. The sha256 are in `data/2026-11-21-dollar-nocopy/jit.txt`.
perf-rt ran twice per binary:

| row | main instr | this | ÷ |
|---|---:|---:|---:|
| plain | 1,898.0 | 1,899.0 | 1.0005 |
| defaults | 1,982.8 | 1,982.9 | 1.0000 |
| named | 1,841.2 | 1,840.9 | 0.9999 |
| env-fn | 1,936.1 | 1,936.3 | 1.0001 |
| superassign (`a$dep(…)`) | 4,905.0 | 4,589.6 | 0.9357 |
| s3 (`s$side`) | 18,969.4 | 18,663.5 | 0.9839 |
| fib | 10,711,743.1 | 10,711,682.3 | 1.0000 |
| binarytrees | 3,499,325.6 | 3,499,317.2 | 1.0000 |
| trycatch | 113,485.2 | 113,487.1 | 1.0000 |
| trycatch-err | 173,832.4 | 173,834.7 | 1.0000 |
| trycatch-ok | 51,053.9 | 51,054.1 | 1.0000 |
| geomean | | | 0.9926 |

Per cmp.R compile (callgrind, N = 800 minus N = 400):
- `jit-scale_by`: 9,920,092 → 9,711,088 (0.9789);
- `jit-safe`: 14,748,764 → 14,478,700 (0.9817);
- `malloc` calls: 3,917 → 1,970 per compile.

**Checks at `4512843f`:**
- debug `cargo test -p robigo-rt -p robigo-rtbuiltins`: 174 pass;
- `cargo fmt --check`, `scripts/test-rt-ci.py`, the full `scripts/rt-ci.py`
  and `scripts/lint.sh` pass. The conformance step is 642/651; the 9
  failures are unratcheted cases that fail the same way on main
  (`bytecode/*`, `jit/serialize-write*`, `serialize/compile-pkgs`,
  `alloc/link-lifetime`);
- rtcore is untouched, so Miri does not apply.

Sol (gpt-6.1-sol, high), round 1: approved, no findings. It found no other
per-name copies left in `subset3_dflt`.
