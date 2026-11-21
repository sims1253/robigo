# One zlib inflater for the lazy-load database's records (2026-11-21)

X/P1, item #2 of the startup plan for the rdb boot
(`~/.cache/robigo-research/x-boot/PLAN.md`). The interpreter that reads
the code is L's.

## What changed

`serialize::decompress1` (GNU R's `R_decompress1`, connections.c) made a
new `flate2::Decompress` for every record it inflated. A new one costs its
window's allocation and set-up: about 14.9k instructions a record, from H's
split of the boot profile. The rdb boot inflates 75 records.

The inflater is now kept in a `Cell` in an isolate layer state of its own
(`RdbInflater`, installed with `Cx::set_ext` on the first record) and reset
for each record.

- **Why a layer state of its own, not an `RtState` field.** The first
  version put it in a new `RtState` field. That reordered the struct and
  moved `superassign` by +8.5 instructions (+0.19%), the RtState layout
  trap. A second version used a thread-local, which g3.md §11 forbids (all
  mutable runtime state is the isolate's; `tests/isolate_state.rs` checks
  it). A layer state of its own leaves `RtState`'s layout alone. Layer
  states are found by a search that stops at the first match, and
  `RtState` is installed before any record is read, so `prims::state`
  costs the same.
- **Why two uses cannot overlap.** Between taking the inflater out of the
  cell and putting it back, only `reset` and `decompress_vec` run. No R
  code runs, no R object is allocated, no safepoint is reached, and the
  warning comes after the put-back. A nested use would find the cell empty
  and make its own inflater, so no borrow can panic.
- **The fetches R can nest.** A record's restore hook can fetch another
  record while the first is being unserialized. That happens after the
  first record's inflate is finished. `tests/rt/serialize/lazyload-nested.R`
  does it, then fetches again and once more after both, and matches GNU R
  4.6.2.

## Numbers

Main 1f8cc8fbb against d0b75cd4c, this branch with that main merged in.
Both were built into empty targets:

- `robigo-rt-main-1f8cc8fb`, sha256 `7a98e2dd117075af137375a042676d93f9e72dc4b3bf28e8573a67e204df94b6`;
- `robigo-rt-dc-d0b75cd4`, sha256 `5d61cf6a30ae0afd746fa9ded9a20d4088f9fb23bb831ab7c9f08dda4262ba59`.

The boot is `--vanilla -e 1` under callgrind:

| boot | main | this | change |
|---|---:|---:|---:|
| `ROBIGO_RT_BOOT=rdb` | 57,423,056 | 56,326,637 | −1,096,419 (−1.9%) |
| interim (M1) loader | 17,096,350 | 17,104,273 | +7,923 (+0.05%; no record is inflated) |

- **rdb:** `Decompress::new` went from 75 calls and 1,111,162 instructions
  to 1 call and 14,788. The inflating itself (`decompress_vec`, 78 calls)
  went from 4.92M to 5.09M: about +2.3k a record, inside zlib-rs's inflate,
  after the reset. That is less than a sixth of the 14.8k a record saved.
- **Interim loader:** it inflates no record and never reaches
  `decompress1`. What moves there is in glibc's unnamed internal routines:
  heap and allocator layout, not code.
- **Variation across measurements.** The same change measured −1.02M to
  −1.28M on earlier mains (88a0ec56b, dd648bb78), and the interim boot from
  −1k to +47k. Every difference was in glibc's `malloc`, `free` and memmove.
  Against 7be989e96, about 12k of a difference was the layer state's two
  lookups a record, which are inlined in `do_lazyload_dbfetch`.

**Resident memory, read from `/proc/PID/status` after the boot.**
- Anonymous memory: −8 KiB (interim) and +16 KiB (rdb). The rdb boot
  keeps the inflater alive; the interim boot never makes it.
- File-backed memory, the binary's pages: +72 KiB (interim) and +64 KiB
  (rdb). That is code layout, which varies from build to build: earlier
  builds of this change measured +180 to +532 KiB.
- Peak RSS (`wait4`, median of 9): +32 KiB (interim) and −64 KiB (rdb).

**perf-rt (`scripts/rt-bars.sh`, median of two runs each).** Every row is
within ±0.05% instructions under both boots, `superassign` included
(4,590.1 → 4,589.7 under the default boot, 4,578.4 → 4,578.6 under rdb).
The thread-local version had moved `superassign` by +0.19% under rdb, all
of it in glibc's alignment-dependent `__memmove_avx_unaligned_erms`; that
shift is gone.

## For later: seek and tell on gz connections

`GzReader` (connections.rs) inflates ahead into a block buffer (#937).
When `seek` and `tell` on gz connections are ported:
- a seek drops the buffer (`obuf`, `opos`, `oend`, `ahead`, `failed`);
- `tell` is a position counted across members and transparent reads
  (gzio.h's `s->out`, which `R_gztell` returns), less the bytes not yet
  read from the buffer (`oend − opos`). It is not the inflater's
  `total_out`, which a new member resets and a transparent read never
  moves.

The same note is on the struct.

## Next

H measured the inflating itself at about 57k instructions a record in
zlib-rs, against about 39k in GNU R's libdeflate. A one-shot,
whole-buffer decoder is the next question, costed with its dependency and
licence. No new dependency is added before the foreman decides.
