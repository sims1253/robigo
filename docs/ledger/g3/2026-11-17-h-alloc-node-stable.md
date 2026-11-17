## 2026-11-17: `Core::alloc_node` is one fast path, not three crate copies

- Why (L, AC1's ledger, "For H"): `alloc_node` was `#[inline]`, and the
  release binary (one codegen unit per crate, thin LTO) held three
  out-of-line copies of it: robigo-heap's, robigo-rtcore's and
  robigo-rt's. LLVM shaped each copy from the set of types its crate
  passed, so a change in what a crate allocates moved the instruction count
  of unrelated rows. Examples: AC1's +24 instructions per s3 iteration with
  the same 11 calls and the same source, and S3x-a's +59 from one
  `alloc_node(EXTPTRSXP, ..)` call in rtcore. On main `f7378ac3` all three
  copies were the generic one: 477 bytes, pushing 7 registers on every
  call, because the slow paths' calls shared the body.
- What (`crates/robigo-heap/src/gc.rs`, `heap.rs`):
  - `alloc_node` is `#[inline(never)]`: one body, compiled in robigo-heap
    only, whose machine code no calling crate can change.
  - The body is `alloc_object`'s fast path with no call: the trigger test,
    the class's current block (`Heap::alloc_fast`: its free list, then its
    bump region), the header, the count, the fields. Any other case goes to
    `#[cold]` `alloc_node_slow`, which is the old body.
  - `Heap::alloc_fast` is `Heap::alloc`'s inline fast path, split out and
    shared. It returns `Option<NonNull<u8>>`, and the bump slot is derived
    from the non-null block, so the bump path has no null test.
  - The result: 380 bytes, 3 register pushes (the fast path's own register
    pressure), one symbol in the binary next to `alloc_node_slow`.
- Measured (this box, release, `scripts/perf.py rt-bars --main`): main
  `25bc512e` (sha256 456b757d…) against this branch rebased on it
  (4241b105…), each built in its own target from its own checkout. 0
  failures. s3 −0.45% (18,816.5 against 18,902.2 instructions per
  iteration). Every other row is within ±0.04% (fib −0.00%, binarytrees
  −0.01%, e7-paste −0.04%). Peak RSS is within ±1%.
- Verified: the full `scripts/rt-ci.py` (every mode, the injected
  failures); Miri on robigo-rtcore and the heap's layout and bitmap tests;
  `scripts/lint.sh`.
- Review: sol-loop, 1 round with gpt-6.1-sol on high, approved with no
  findings.
