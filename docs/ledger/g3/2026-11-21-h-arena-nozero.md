# H: arena segments that do not zero, and two boot-sized tables (2026-11-21)

- **The problem.** After #929, a symbol miss still spent about 128 of its 946 instructions zeroing new 64 KiB arena segments: 65k instructions a segment, 1.1% of a base boot. Two more tables grew during every boot:
  - the CHARSXP cache: 2,201 strings at boot, from 1,024 slots to 4,096, growing twice;
  - rtcore's symbol cells: ids below 2,048, from 1,024 cells, growing twice, for 74,583 instructions.
- **The change:**
  - a segment zeroes only its pseudo header (`DATA_OFF` bytes). The invariant is in `arena.rs`'s SAFETY comment: every byte a reader can reach is written before publication, under the arena's lock; the unwritten tail is unreachable; a forked child copies the memory while the lock is held;
  - the CHARSXP cache starts at, and prunes to no fewer than, 4,096 slots;
  - the symbol cells' first growth makes 2,048.

  Both sizes are constants with their measurements beside them.
- **Also fixed:** the heap tests' probe library kept its key and hook in process globals. Two tests on parallel libtest threads could race on them, the cause of `the_strong_library_keeps_what_its_frames_name`'s rare failure (0 in 4,200 looped runs here; seen once by E). They are `_Thread_local` now, and `the_probe_keeps_one_key_per_thread` fails on the old globals.
- **Measured:**
  - a miss: 946 → 817 instructions (`examples/install_cost.rs`, callgrind); a hit stays at 139;
  - boot (`--vanilla -e 1`): 17.25M → 16.93M instructions (−1.85%);
  - `install_named`: 1.77M → 1.46M.
- **Gates** on 5e687283e, main 2a5c420cb merged:
  - Miri on robigo-heap (`layout::`, `heap::bitmap_tests`, `heap::medium_tests`, `arena::`, including #929's cross-thread record test) and on robigo-rtcore passed;
  - lint passed;
  - rt-bars against main: 0 failures, e7-paste +0.01%;
  - the full `scripts/rt-ci.py` passed except its `ROBIGO_RETRY_TORTURE=1` conformance step. There `tests/rt/bytecode/cmpfun-ops.R` and `tests/rt/serialize/compile-pkgs.R` timed out at a machine load of 36–48. Main (2a5c420cb), built the same way, times out on both too.
- **Review:** sol-loop, 2 rounds, gpt-6.2-sol on high: APPROVE. Fixed:
  - S1: the probe test calls the routine with its C signature;
  - S2: the SAFETY comment says which bytes readers reach;
  - S3: the CHARSXP floor's cost note counts the collections' scans.
