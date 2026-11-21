# One zlib inflater for the lazy-load database's records (2026-11-21)

X/P1, item #2 of the startup plan for the rdb boot
(`~/.cache/robigo-research/x-boot/PLAN.md`). The interpreter that reads
the code is L's.

## What changed

`serialize::decompress1` (GNU R's `R_decompress1`, connections.c) made a
new `flate2::Decompress` for every record it inflated. A new one costs its
window's allocation and set-up: about 14.9k instructions a record, from H's
split of the boot profile. The rdb boot inflates 75 records.

The inflater is now kept in a thread-local `Cell` and reset for each
record.

- **Why a thread-local, not a field.** The first version put it in a new
  `RtState` field. That reordered the struct and moved `superassign` by
  +8.5 instructions (+0.19%), the RtState layout trap. A thread-local
  leaves `RtState` alone. The inflater holds no R object, so it does not
  need to belong to the isolate.
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

Main 88a0ec56b against d7fd971f0 (main merged in). Both were built into
empty targets:

- `robigo-rt-main-88a0ec56`, sha256 `8d715a5f2c44f14355cfdf7d08d5d6d3574d9d796aae1cbff0b8e516c2ed0785`;
- `robigo-rt-dc-d7fd971f`, sha256 `1d5d7eae9733da24abdf91b65d9c7335e3d122a26c04e7626332adc5622fae11`.

The boot is `--vanilla -e 1` under callgrind:

| boot | main | this | change |
|---|---:|---:|---:|
| `ROBIGO_RT_BOOT=rdb` | 57,554,211 | 56,380,867 | −1,173,344 (−2.0%) |
| interim (M1) loader | 16,682,780 | 16,685,155 | +2,375 (no record is inflated) |

Under rdb, `Decompress::new` went from 75 calls and 1,112,704 instructions
to 1 call and 14,741. The inflating itself (`decompress_vec`, 78 calls)
went from 4.91M to 5.09M: about +2.3k a record, inside zlib-rs's inflate,
after the reset. That is less than a sixth of the 14.8k a record saved.

**Resident memory, read from `/proc/PID/status` after the boot.**
Anonymous memory is +24 KiB under both boots: the inflater kept alive.
File-backed memory, the binary's pages, is +384 KiB (interim) and +180 KiB
(rdb). That is code layout: the binary is 10 KB larger, and the interim
boot never makes the inflater. Peak RSS (`wait4`, median of 9) moves by the
same amounts.

**perf-rt (`scripts/rt-bars.sh`, median of two runs each).**
- Default boot: every row is within ±0.02% instructions.
- rdb boot: every row is within ±0.01%, except `superassign` +0.19%
  (4,578.6 → 4,587.5). All of that row's change is in glibc's
  `__memmove_avx_unaligned_erms` (offset 0x1b1220, 19 → 28 instructions per
  iteration), whose count depends on buffer alignment. The inflater kept
  alive shifts malloc's addresses. The row is address-sensitive under the
  rdb boot, and the change is not code.

## For later: seek and tell on gz connections

`GzReader` (connections.rs) inflates ahead into a block buffer (#937).
When `seek` and `tell` on gz connections are ported:
- a seek drops the buffer (`obuf`, `opos`, `oend`, `ahead`, `failed`);
- `tell` subtracts the bytes not yet read from it (`oend − opos`) from what
  the inflater has produced.

The same note is on the struct.

## Next

H measured the inflating itself at about 57k instructions a record in
zlib-rs, against about 39k in GNU R's libdeflate. A one-shot,
whole-buffer decoder is the next question, costed with its dependency and
licence. No new dependency is added before the foreman decides.
