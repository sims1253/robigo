# robigo-rt's function order at link, and what else a start's pages hold: shelved (2026-11-23)

X/P1, an experiment the foreman asked for after the startup rows (#978).
**Not landed:** the saving is real, but it decays within days of ordinary
merges, and keeping it needs a two-pass link that is not worth its build
complexity for clean shared pages. This records the numbers, so that the
lever is not rediscovered from scratch.

The script: `data/2026-11-23-order-file/rt-order.py`, kept here, not in
`scripts/`.

## What a start leaves resident

Main 0007ddf33, the three starts (`--vanilla -e 1`,
`R_DEFAULT_PACKAGES=NULL`). The binary's resident pages were read from
/proc/PID/smaps and pagemap at the ptrace exit stop, with randomization off.
The code a start runs comes from callgrind `--dump-instr=yes`, each
executed instruction mapped to its page and to its `nm` function.

| start | text resident | pages holding executed code | functions run (their summed size) |
|---|---:|---:|---:|
| interim | 4,264 KiB of 4,712 (90%) | 396 (1,584 KiB) | 577 (783 KiB) |
| rdb, JIT on | 4,648 KiB (99%) | 729 (2,916 KiB) | 1,305 (1,845 KiB) |

A start runs 0.8–1.8 MB of functions scattered over 400–730 pages, and the
kernel's fault-around maps nearly all the rest of `.text` as neighbours.

The read-only segment at the rdb JIT-on start's exit, resident:

| section | size | resident |
|---|---:|---:|
| `.rodata` | 903 KiB | 524 KiB |
| `.rela.dyn` | 206 KiB | 212 KiB |
| `.dynstr` + `.dynsym` | 219 KiB | 224 KiB |
| `.data.rel.ro` | 157 KiB | 160 KiB |
| `.gcc_except_table` | 121 KiB | 124 KiB |
| `.eh_frame` + `.eh_frame_hdr` | 362 KiB | 72 KiB |

- **A normal start does not unwind:** only 16–32 KiB of `.eh_frame`'s
  325 KiB is resident, which is fault-around neighbours.
- **`.dynsym`/`.dynstr`:** `-rdynamic` (robigo-rtapi's build script) exports
  every global symbol, 2,196 of them, of which 1,878 are Rust-mangled
  (`_R…`) and not the C API. A version script, or `--export-dynamic-symbol`
  patterns for the C API in place of `-rdynamic`, would shrink these
  tables by roughly 85%. Not tried.

## The order file

- **Method:**
  - LLD (23.1.5, rustc 1.100's default linker on x86_64-unknown-linux-gnu,
    also in CI's image) takes `--symbol-ordering-file`;
  - the file is the union of the three starts' executed functions, hottest
    first by summed instruction count: 1,355 mangled names, 1,935 KiB of
    code, led by vm::run, vm::exec, eval_in;
  - the build linked with
    `-C link-arg=-Wl,--symbol-ordering-file=… -Wl,--no-warn-symbol-ordering`;
  - LLD applied it: vm::run moved from 0x2e1420 to 0x1cacc0, `.text`'s
    first byte.
- **Result,** the same source and checkout, `rt-startup.py`, 9 runs,
  randomization off:

  | start | peak RSS main → ordered | text resident | anonymous peak | instructions |
  |---|---|---|---|---|
  | interim | 9,888 → 7,432 KiB (−24.8%) | 4,224 → 2,040 KiB | 1,880, = | within run-to-run noise |
  | rdb, JIT on | 15,508 → 12,844 KiB (−17.2%) | 4,628 → 2,040 KiB | 6,932, = | within noise |
  | rdb, JIT off | 13,300 → 10,632 KiB (−20.1%) | 4,628 → 2,040 KiB | 4,680, = | within noise |

  - **Run-to-run noise:** the same binary varies by up to 518 instructions
    between runs with randomization off. `define_builtins` keys
    `prims::RtState::by_sym` by name-string addresses, and that and other
    maps use `RandomState` (E has the list).
  - **perf-rt's 28 rows** (`perf.py rt-bars --cycles`): 0 failures,
    instructions ±0.04%, estimated cycles geomean −0.08%, from −0.96% to
    +0.42%.
- **Wall time and hardware counters,** at a load the box never left that
  night:
  - **hyperfine,** 200 runs per start, load 78–108 (inside a gate slot):
    the spread was ±25–70%, so no conclusion. The minimums agreed within
    noise: interim 8.6 against 8.7 ms, rdb JIT on 32.5 against 32.8 ms,
    JIT off 18.8 against 18.0 ms. hyperfine's own peak memory confirmed
    −23/−17/−20%.
  - **`perf stat`,** 100 runs per start, load 55–59:

    | start | cycles | iTLB misses | L1 i-cache misses | page faults |
    |---|---:|---:|---:|---:|
    | interim | −0.06% | +8.8% | +0.1% | 605 → 588 |
    | rdb, JIT on | −5.0% | +4.4% | +9.8% | 1,803 → 1,773 |
    | rdb, JIT off | +1.3% | +11.9% | +13.2% | 1,046 → 1,018 |

    A mixed result. Hottest-first does not keep callers beside their
    callees, so the instruction-side misses rose a little. Speed is neutral
    to mixed and unproven at that load; the memory saving is the result.
- **The caveat:** these are clean, file-backed pages, shared through the
  page cache.
  - The saving per additional robigo-rt process running at the same time is
    about 0.
  - What it saves is one process's resident set and its page faults at
    start.
  - Under memory pressure such pages are dropped first and re-read.
  - RSS overstates the memory cost of both robigo-rt and GNU R in the same
    way.

## Why it is shelved: the names decay

LLD matches names exactly. Two things change them:
- **ThinLTO's `.llvm.N` suffix on promoted local functions** is a hash of
  the module, which includes the line tables' absolute paths, so it changes
  with the checkout path. A file generated in one checkout covered only
  93–96.5% of a build of the same commit in another.
- **Ordinary code changes** inline functions away, monomorphize new ones,
  and change a crate's suffixes.

Measured on CI's own binaries (perf-rt-bins artifacts, which CI keeps for a
day), with the order file generated from CI's main 0007ddf33 binary:

| against CI's main | missing names | only the suffix | really gone or renamed |
|---|---:|---:|---:|
| 24f3fd36a, 2 h and 2 merges older | 119 of 1,355 (8.8%) | 64 | 55 |
| 40f4b6f71, 3.7 h older (before #974) | 195 (14.4%) | 62 | 133 |

**The saving falls off steeply with coverage:** each missing function is
scattered over `.text` and pulls its pages, and their fault-around
neighbours, back in. The PR's own binary at 93–96.5% coverage kept 44–75% of
the saving (interim −1.1 of −2.5 MB, rdb −2.0 of −2.7 MB).

- **(b) Generate from CI's binary, regenerate now and then.** It decays
  within days of merges; a weekly regeneration would not keep it.
- **Link twice, resolving suffix-stripped names** against the first link's
  `nm`: it recovers the suffix-only half and still decays at half the rate.
- **(c) Profile and relink at every build** (three callgrind starts with
  `--dump-instr`, about 1 min; a thin-LTO relink, about 50 s): 100% by
  construction, +2–3 min on every build. The release build would have to do
  the same.

**When it becomes worth doing:** when a release build pipeline runs the
binary anyway. The tree build already does: `rt-home-db.sh` runs it to write
base.rdb and C1's translation cache. A profile-and-relink stage belongs
there, not in every CI build.

## Packed relative relocations (not landed: Max's decision)

- `-Wl,-z,pack-relative-relocs` on the ordered build: `.rela.dyn` 206 KiB →
  4 KiB plus a 2.8 KiB `.relr.dyn`, and the binary is 204 KB smaller.
- The dynamic loader no longer applies about 8,800 relocations: −147k
  instructions on the rdb JIT-on start (−0.16%), −164k with the JIT off
  (−0.33%).
- Peak RSS is only 60–70 KiB lower than the ordered build alone. The
  anonymous peak moved −92/+56 KiB, not shown to be placement.
- It needs glibc ≥ 2.36 at run time, so Ubuntu 22.04 and RHEL 9 could not
  load the binary. The project's minimum supported system is undecided and
  is with Max.
