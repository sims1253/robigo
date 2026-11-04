# perf-simple-loops: calling native trees from the VM (round 3)

- Why: with loops in closed form (#324), emptyFor and scalarFor were left
  at 9.6e-8 s per `execute()` against Ř's 6.9e-8 / 7.2e-8: all of it the
  call path of the harness's `for (.j in seq_len(.k)) .body()`, ~2180
  instructions per call (callgrind, 2e5 against 4e5 calls).
- Where: the VM's top-level loop flushed `.j` to the global environment
  before every call, because `.body`'s tree reads a global (`.n`): ~600
  instructions (`Vars::flush`, `set_var`, `plain_lookup`,
  `binding_flags`). The closure frame of values of any type (for a NULL
  result) initialized and released 10 `RVal` slots, 9 of them temporaries
  no op used: ~220.
- Changes (`jit/closure.rs`, `vm.rs`, `compile.rs`):
  - `Entry::frame_syms`: every name a tree looks up in its environments
    (cells, functions its call sites name, functions it guards). A
    pending assignment of the caller is flushed only if it is one of those.
    First version scanned the dirty slots per call: fasta_naive +2.2%,
    fastaredux_naive +2.8% (large caller frames). Now a bitmask of the
    caller's slots the tree reads, cached per call site by entry id
    (`CallSite::native_reads`; ids, not addresses, since `give_up` drops
    entries), and only for trees with cells (without cells, pending
    assignments were never consulted).
  - The frame size given to `robigo_d_init`/`robigo_d_release` is patched
    after code generation to the temporaries used (`Function::replace`).
- Result: 2177 -> 1753 instructions per call; peak (with #324) emptyFor
  9.6e-8 -> 8.6e-8 s, scalarFor 9.6e-8 -> 8.7e-8, bytecodes 1.22e-7 ->
  1.05e-7, emptyWhile 1.25e-7 -> 1.10e-7, scalarWhile 1.22e-7 -> 1.04e-7.
  Ř is still ahead on emptyFor/scalarFor (6.9e-8 / 7.2e-8). `perf.py
  compare` against #367: 6 faster (profiler-shared -4.1%, binarytrees
  -3.8%, spectralnorm_alt_3 -2.4%), 0 slower.
- Left: the rest is spread thin (cell lookup and clone of `.n` per call,
  memo validation, argument vectors, the VM's own loop). The structural
  fix is native loops calling native trees directly (other groups' #296,
  #313, #325).
- Verified: `tests/r/jit/native-call-pending.R` (pending assignments to a
  cell, to a called function, to a guarded `+`, to an unread loop variable;
  GNU R 4.5.0); conformance (except `linalg/svd`, `linalg/battery`: LAPACK
  digits on this VM, also on main); fastr slice of functions/sys/eval;
  sol-loop 2 rounds, approved.
