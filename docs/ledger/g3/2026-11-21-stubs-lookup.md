# The not-implemented stubs: one lookup set, not a scan per name (2026-11-21)

X/P1, item #5 of the startup plan (`~/.cache/robigo-research/x-boot/PLAN.md`).
The code is L's (`base.rs`).

## What changed

At startup, `base::stubs_for` binds every primitive and internal that
names.c lists and robigo-rt does not define to a "not implemented yet"
builtin. Two callers use it:
- `stage0`, the rdb boot, over names.c's whole table;
- `bind_stubs_wide`, the interim loader, over the wide base.

For each name it asked whether robigo-rt defines it by scanning all of
`RtState::defs`, about 1,500 definitions. With about 1,500 names that is
about 2.4M instructions per boot.

The defined `(name, internal)` pairs are now collected once into a set.
The set uses the translator's `PosHasher` (rustc-hash's multiply-rotate,
from #930), not SipHash. The keys are names.c's compiled-in names, never a
value R code chooses, so there is nothing for a slow, keyed hash to defend
against. With SipHash the set alone cost 290k of the 0.93M left.

## Numbers

Main 3062db07b against ddd2f290a. Both were built into empty targets:

- `robigo-rt-main-3062db07`, sha256 `6e12d01a369a8dcfe1e7f0ba851a607528e6062e95134940e770078a220a951a`;
- `robigo-rt-st-ddd2f290`, sha256 `5d753bbc02e965cebe2d127540ee26e3f5eff01292e21a5ce69d4f25b85a5129`.

The boot is `--vanilla -e 1` under callgrind. The rdb boot reads one
base-database tree, written by main's binary, for both binaries.

| boot | main | this | change | `stubs_for` |
|---|---:|---:|---:|---:|
| interim (M1) loader | 17,105,941 | 15,412,094 | −1,693,847 (−9.9%) | 2,355,665 → 701,211 |
| `ROBIGO_RT_BOOT=rdb` | 57,429,866 | 56,006,834 | −1,423,032 (−2.5%) | 2,399,737 → 750,642 |

What is left in `stubs_for`:
- about 0.40M is `define_builtins` registering the stubs;
- about 0.11M is `internal_eval`'s binary search for the stubs' `eval`
  values;
- the rest is the set itself.

**Resident memory, read from `/proc/PID/status` after the boot.** Anonymous
memory is −8 KiB (interim) and 0 (rdb). The set is dropped when
`stubs_for` returns.

**perf-rt (`scripts/rt-bars.sh`, median of two runs each).**
- rdb boot: every row is within ±0.01% instructions.
- Default boot: every row is within ±0.03%, except `s3` +15.7 (+0.10%).
  - `stubs_for` runs only at boot.
  - Under the default boot, s3's script outside the harness measures the
    other way, −7.9 a call. All of that is glibc's alignment-dependent
    memmove (`__memmove_avx_unaligned_erms`, offset 0x1b1220), and no
    robigo function moves.
  - So `s3` is address-sensitive under that boot, and the change is not
    code.
