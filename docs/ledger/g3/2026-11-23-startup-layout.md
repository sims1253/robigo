# The startup gate over the heap's layout: 15 shifts, the mean (2026-11-23)

X/P1. The startup gate (#978) compared one start of each binary, one layout
each, and one layout decided verdicts. Checks failed tonight by layout
alone, with no cost in the PR:
- #979 twice, #984, B's tempdir gate: the interim start's instructions;
- L's saveload gate: interim instructions, and rdb-jit's anonymous peak
  +112 KiB;
- #988: rdb-jit's anonymous peak +80 KiB, part of it real;
- #992: rdb-jit's anonymous peak +128 KiB against a +68 KiB limit, where H's
  mean over 15 path lengths was about +19 KiB;
- #991, A1's runner PR, whose only runtime change formats one error line:
  rdb-nocache's anonymous peak +136 KiB against a +68 KiB limit, with every
  instruction row within ±0.03%. That is the cleanest evidence: no cost at
  all, and the Check failed.

**The standing rule:** tables built at a start are pre-sized. A start
whose count steps by ~80k instructions is realloc/memcpy until shown
otherwise.

**H's finding** (CI's own two binaries of run 45102076475, in the oracle
image, rt-startup.py's environment, only the length of R_HOME's path
changed, 8 to 78 characters): each binary's interim start is bimodal, in
steps of ~80k instructions (main 15,305k–15,391k, #984 15,329k–15,428k), at
lengths that differ per binary. CI's paths put the PR on its upper step and
main on its lower: +0.54%, a fail. The mechanism:
- `Wide::parse` (robigo-rt/src/base.rs) grows its Vec and HashMap by
  doubling, and each realloc either extends in place or copies ~60k
  instructions of memcpy, by what lies above it;
- C1's RtState 824 → 832 bytes (+0.68% on #979) is the same lever;
- #984 pre-sized Wide (interim −3.6%), but steps of ~150k remained at some
  lengths on both binaries: 1% of the interim start, against a 0.5% limit.

What still grows by realloc at a start, after #984 (callgrind with
`--separate-callers`, realloc's inclusive cost by the function that grew):
- interim:
  - Wide's initialisation, 521k (3.5% of the start);
  - `define_builtins`, 105k;
  - `base::load`'s `lib.join(f)` per base file, 102k;
  - `Cenv::find_locals`, 107k;
- rdb-jit, the compiler's tables: `Comp::emit` 437k, `Comp::pass` 308k,
  `Comp::translate` 281k, `Comp::k` 231k, `bctrans::merge` 132k.

The foreman routed each one to its owner. An owner reruns the list with
`scripts/rt-startup.py --robigo-rt BIN --trees --reallocs` (each start
under callgrind with `--separate-callers=3`, at the first layout shift:
realloc's inclusive cost by the function that grew the buffer, the top
five, as shares of the start). On main 92d933c4b:
- interim (14.66M): Wide's initialisation (a `Vec<&str>` collected from
  `split`, 224k, 1.53%; the OnceLock's own, 216k, 1.47%),
  `Cenv::find_locals` 116k, `define_builtins` 106k, `Path::_join` 80k;
- rdb-jit (81.42M): `Comp::emit` 162k, the translation cache's
  `tcache::format::entry` 149k, `set_option` 90k, `define_builtins` 85k,
  `Comp::k` 44k.

## The shifts

- **The home's path length** (ROBIGO_RT_CHECKOUT for robigo-rt, R_HOME for
  GNU R) moves where the early allocations fall. Main 26b7252f and #986's
  head, every length from 35 to 139 characters: one start's count spans
  0.84% (interim), 0.71% (rdb-jit) and 1.15% (rdb-nojit) of the start,
  over many levels, not two.
- **The binary's own size** moves where its heap starts, which no path
  length reaches. A no-op build of main (4 KiB more data, which nothing
  reads) was −0.67% on the interim start over 15 lengths, lower at every
  one of them. The difference is Wide's remaining reallocs again: realloc's
  own copying, −200k.
- **glibc malloc's `top_pad`** (`GLIBC_TUNABLES=glibc.malloc.top_pad`, the
  extra each heap extension takes) moves where the heap's top lies. Over
  15 values at one length, main against the no-op build was −0.18%.
- **The two paired:** shift *k* boots from a home 35 + 7*k characters long
  with top_pad 128 KiB + 4 KiB × *k*, for *k* = 0..14. Main against the
  no-op build: −0.07%.

The homes are symlinks to the binary's trees in a fixed-length scratch
directory under /tmp, so both binaries get the same absolute lengths
wherever their trees are unpacked: CI's paths drop out.

Every measured run also gets a fixed environment, nothing of the caller's:
- PATH, HOME=/tmp and the starts' variables;
- ROBIGO_RSRC, the pinned GNU R sources the interim boot loads base's R
  code from, through a fixed-length symlink.

L found the cause of the +112 KiB: the anonymous peak followed the
environment's size. The rdb-jit start, anonymous peak in KiB with a padding
variable added:

| padding (bytes) | L's branch | main 71922a235 |
|---:|---:|---:|
| 0 | 6,848 | 6,820 |
| 100 | 6,828 | 6,932 |
| 1,000 | 6,828 | 6,932 |
| 4,000 | 6,832 | 6,936 |
| 16,000 | 6,844 | 6,948 |
| 64,000 | 6,892 | 6,996 |
| 130,000 | 6,956 | 7,060 |

A 104 KiB step flipped between the two binaries at a few bytes of
environment, so the anonymous peak was exact for one layout and decided by
layout between binaries, as the instruction count was.

The environment padding of #978 (ENV_SHIFTS, 0/100/1,000 bytes) goes. It
moves the initial stack, not the heap. Over the 15 shifts, a 1,000-byte
variable moves the mean by −274 instructions (interim) and +4,142 (rdb-jit),
about 0.005%.

## The statistics

- **Instructions: the mean of the 15 shifts.** The levels are many, so a
  median could still jump between two of them; a mean cannot.
- **The anonymous peak: the mean of the 15 shifts, one run at each.**
  - Each layout's peak is exact: five invocations at one shift gave one
    value, for interim (1,860 KiB) and rdb-jit (6,744–6,748 KiB).
  - Layouts differ by pages: over the 15 shifts the readings spread in
    4 KiB steps over 24 KiB (interim), 64 KiB (rdb-jit) and 232 KiB
    (rdb-nocache).
  - Between main and the no-op build, the minimum moved by up to +72 KiB
    and the median by up to −60 KiB; the mean moved by at most 14 KiB.
  - The mean is the expected peak over layouts: the honest figure.
- **Peak RSS:** unchanged, the median of the runs at the home itself; it
  is gated loosely.

## The floor

Main 92d933c4b, all four starts. "Two tree paths" is the same binary with
its trees built twice into different directories; "no-op" is the 4 KiB
build.

| start | instructions, two tree paths | instructions, no-op | anon mean, two tree paths | anon mean, no-op |
|---|---:|---:|---:|---:|
| interim | +0.002% | −0.073% | 0 | +1 KiB |
| rdb-jit | +0.001% | +0.011% | 0 | −3 KiB |
| rdb-nojit | +0.001% | −0.001% | 0 | −6 KiB |
| rdb-nocache | +0.015% | −0.061% | −5 KiB | −14 KiB |

Before, one layout per binary:
- instructions: up to ±0.8% on a no-op change (the earlier probe's single
  lengths);
- anonymous peak: +35 KiB for rdb-jit on the no-op build over lengths
  alone; up to +128 KiB in CI.

"Two tree paths" is not exactly 0: RandomState-keyed maps at a start
(`prims::RtState::by_sym` among them) hash differently per run, ±518
instructions on one start. The rdb-nocache start also translates its code
afresh in each tree.

## The cost

All four starts of one binary, `--runs 9`, trees given, on four cores
(`taskset -c 0-3`, as GitHub's runners), at load 34–46:
- the old harness (3 environment shifts): 22 s;
- this one (15 layout shifts): 56 s.

That is +34 s per robigo-rt binary. CI measures two, plus GNU R's two
starts, on slower cores: an estimated +2–3.5 minutes in perf-rt, which the
PR's own run reports. 31 shifts 3 apart (floor about ±0.1%) would roughly
double the shift part again, past the ~3 minutes allowed, so 15 it is.

## The harness fix that came with it

memory() waited with `waitpid(-1, __WALL)`, from #987, so two measurements
running at once in one process took each other's ptrace events and hung.
The gate runs its memory runs one at a time, and CI never met it; a probe
running them side by side did. memory() now waits on its start's own
process group only, which the start has to itself (setpgid), with a test
of four runs side by side.

## The anonymous peak on the runner: a coarse tripwire (2026-11-23, later)

Under the 15 shifts, two unrelated PRs' binaries (#991's, which formats one
error line, and #994's) read the same anonymous peaks against main
e4a9fab22's, to the KiB: +5 / −2 / +19 / +40 on interim / rdb-jit /
rdb-nojit / rdb-nocache. #993 against main 370b96aa0 read 0 / +9 / −23 /
−9. The limit was +max(1%, 64 KiB).

#991's runner binaries (CI's perf-rt-bins), measured inside CI's image
(robigo-oracle:4.6.2, glibc 2.41), the 15 shifts, each on its own trees:

| start | PR − main, in CI's image | on the runner | locally, glibc 2.44 |
|---|---:|---:|---:|
| interim | +3 | +5 | |
| rdb-jit | −3 | −2 | |
| rdb-nojit | +15 | +19 | +13 |
| rdb-nocache | +56 | +40 | +8 |

- **It is glibc 2.41's malloc, not the runner or the trees:** a binary on
  the other's trees reads the same for rdb-nocache, and pinning
  `glibc.malloc.mmap_threshold` changes nothing.
- **Split by mapping at the peak's reading:**
  - the binary's own pages (relocated RELRO, `.got`, `.data`, `.bss`) are
    180 KiB in both binaries, every start, every layout;
  - robigo-heap's 1 MiB chunk regions are identical to the KiB;
  - the whole difference is glibc's `[heap]` and its anonymous mmaps for
    large allocations: the runtime's own Rust allocations. At one layout
    the PR's binary reads 44 KiB *less*, all of it `[heap]`.
- **Why no layout lever helps:** physical pages at the peak follow malloc's
  history: the order and sizes of what the runtime mallocs and frees, which
  any code change perturbs. The shifts move where the heap lies, not that.

So the anonymous-peak limit is now **+max(2%, 128 KiB)**, a coarse tripwire
for gross regressions only. Instructions and peak RSS are unchanged. It is
to be replaced by a logical peak that no placement enters: the peak of
malloc'd bytes, from a tool that counts requests, and robigo-heap's own
bytes in use.

## The memory clause: a logical peak (2026-11-23, later)

The anonymous-peak tripwire above is replaced. Memory is now judged as
**the peak of malloc'd bytes**: valgrind's massif, its peak snapshot's
useful heap (`--peak-inaccuracy=0.0`), bytes as the program requested them.
No page, alignment or placement enters it.

- **robigo-heap's chunks are inside the figure:** its 1 MiB chunks come
  from Rust's allocator (`std::alloc::alloc`, 64 KiB aligned), so they are
  malloc requests that massif counts. Adding robigo-heap's own chunk count
  to it would count them twice.
- **The limit:** +max(0.5%, 16 KiB) against main's binary, one run per start
  at the first layout shift. Instructions and peak RSS are unchanged. The
  anonymous peak (physical pages) is reported, and fails nothing.

On #991's runner binaries (PR f3cc6cee0, main e4a9fab22, each on its own
trees):

| start | peak over the 15 shifts | PR − main, at every shift | repeated, one shift | s/run |
|---|---|---|---|---|
| rdb-nocache | 9,947,143–9,947,919 B | +2 bytes | identical, 3 runs | 2.2 |
| interim | 3,456,959–3,457,831 B | 0 bytes | | 1.0 |

Against the same pair's anonymous peak, +56 KiB on rdb-nocache in CI's
image, the logical figure's floor is the code's real allocations.

How to read the figure:
- **A pre-sized table shows as a memory gain, correctly.** Valgrind's
  realloc always copies, so while a buffer grows, the old and the new one
  count together for a moment. That is the same on both sides and
  deterministic: the requested-bytes view.
- **Why one fixed shift:** the figure moves across the 15 shifts only by
  the home path's own strings, about 800 bytes over paths 35–133
  characters long. One shift, the same for both binaries, is enough, and
  it keeps the cost to one run.
- **GNU R's malloc peak** (its allocator goes through malloc too) is
  reported beside it: 14.6 MB with the JIT, 11.3 MB without.
- **robigo-heap's own peak line,** once a binary prints it under
  `ROBIGO_GC_STATS=1`: `robigo gc: peak bytes in use N (objects A, block
  overhead B, large L); peak chunk bytes mapped M`. N is reported beside
  the clause, finer-grained than the chunk-sized steps in massif's figure.
  A binary without the line shows "-".
- **The cost:** one massif run (1–2 s) and one stats run per start per
  binary: about +20 s for the four starts of two binaries, and +6 s for GNU
  R's two.

