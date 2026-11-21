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

Main 88a0ec56b against 7be989e96, this branch's head with main merged in.
Both were built into empty targets:

- `robigo-rt-main-88a0ec56`, sha256 `8d715a5f2c44f14355cfdf7d08d5d6d3574d9d796aae1cbff0b8e516c2ed0785`;
- `robigo-rt-dc-7be989e9`, sha256 `c485195873145bf3d6871ca0c25e403ba766e29c1119819af3d916f2c6606d64`.

The boot is `--vanilla -e 1` under callgrind:

| boot | main | this | change |
|---|---:|---:|---:|
| `ROBIGO_RT_BOOT=rdb` | 57,553,753 | 56,529,450 | −1,024,303 (−1.8%) |
| interim (M1) loader | 16,682,702 | 16,681,678 | −1,024 (no record is inflated) |

Under rdb, `Decompress::new` went from 75 calls and 1,112,704 instructions
to 1 call and 14,741. The inflating itself (`decompress_vec`, 78 calls)
went from 4.91M to 5.09M: about +2.3k a record, inside zlib-rs's inflate,
after the reset. That is less than a sixth of the 14.8k a record saved.

The thread-local version (d7fd971f0) measured −1,173,344 on the same main,
148,583 fewer.
- About 12k of the difference is in `do_lazyload_dbfetch`, where
  `decompress1` and the layer state's two lookups per record are inlined:
  about 160 a record.
- About 130k is inside glibc's `malloc`, `free` and an unnamed internal
  routine (offsets 0xbbdc0, 0xba740). The allocator takes other paths with
  the inflater's allocations made at another point. That is heap layout,
  not code.

**Resident memory, read from `/proc/PID/status` after the boot.**
- Anonymous memory: +4 KiB (rdb) and +8 KiB (interim), the inflater kept
  alive.
- File-backed memory, the binary's pages: within ±10 KiB under rdb and
  +532 KiB under the interim boot, which never makes the inflater. That is
  code layout, and it varies from build to build; the thread-local
  version's was +384 KiB.
- Peak RSS (`wait4`, median of 9): +176 KiB (rdb) and +392 KiB (interim).

**perf-rt (`scripts/rt-bars.sh`, median of two runs each).** Every row is
within ±0.05% instructions under both boots. `superassign` is 4,590.2 on
both binaries under the default boot and 4,578.5 on both under rdb. The
thread-local version had moved it by +0.19% under rdb, all of it in glibc's
alignment-dependent `__memmove_avx_unaligned_erms`; that shift is gone.

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
