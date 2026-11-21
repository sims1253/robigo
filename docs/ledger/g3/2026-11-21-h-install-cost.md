# H: symbol interning at half the cost (2026-11-21)

- **The problem.** X's profile of the base boot (#922, `--vanilla -e 1`) put `Core::install_named` at 946 instructions a call, against GNU R's 317, 5.06M in all.
- **Measured in isolation** (`crates/robigo-heap/examples/install_cost.rs`, callgrind toggle-collect, release; 1,024 existing and 1,024 new names, base-shaped):

  | | before | after |
  |---|---:|---:|
  | hit | 193 | 139 |
  | miss | 2,289 | 946 |

  A hit never allocated. The cost was in misses (1,970 of the boot's 5,347 calls).
- **Where a miss went** before the change: about 520 instructions on two boxed key copies (malloc, copy, free); about 340 on both tables growing by rehashing; about 345 on the PRINTNAME's `mk_char`; about 290 on rtcore's linear special-name compare; the rest on a second hash and probe and on the arena's lock.
- **The change:**
  - one hash per install, passed to the heap's table, the arena, the name record and the PRINTNAME;
  - `symtab::SymTable`: open addressing over symbols, keyed by the arena's name records, so no key copies (the safety argument is in its module doc);
  - both tables sized at 4,096 slots (a boot makes 1,970 symbols; 32 KiB per heap);
  - `is_spec_name`: a first-byte/length table before the compare, exact by construction and tested against the linear compare.
- **Unchanged:** per-isolate PRINTNAMEs, the arena lock and its place in the fork order, and the contended two-isolate test.
- **What a miss's 946 instructions are now** (callgrind, by part, approximate):
  - the PRINTNAME CHARSXP, its allocation and cache insert: about 210;
  - zeroing new 64 KiB arena segments (`alloc_zeroed`): about 128;
  - hashing (the one `hash_bytes`, plus GNU R's pjw hash for the symbol's HASHVALUE): about 120;
  - the arena's mutex: about 48;
  - the rest (probes, inserts, the symbol's and the record's writes, the ASCII check): about 440.

  The next lead: segments that do not zero, since every byte of an object is written before it is published.
- **Boot** (main's binary, `--vanilla -e 1`, callgrind): 19.75M → 17.25M instructions (−12.6%); `install_named` 4.08M → 1.77M.
- **Gates** on d0a297cae:
  - the full `scripts/rt-ci.py` passed, except the builtin differential, which lost its `/tmp/robigo-a4` tree to another delegate's run (the foreman's notice); rerun alone, it passed;
  - Miri on robigo-heap (`layout::`, `heap::bitmap_tests`, `heap::medium_tests`, `arena::`, including the new cross-thread record test) and on robigo-rtcore passed;
  - lint passed.
- **Perf:** rt-bars against main 386a0358f, 0 failures. e7-paste 95,708 → 95,717.
- **Review:** sol-loop, 2 rounds, gpt-6.2-sol on high: APPROVE. Fixed:
  - S1: the hash word read as the u64 it is written as;
  - S2: the cross-thread test synchronized only by the arena's lock;
  - S3: a helper folded;
  - S4: the layout assertion's width.
