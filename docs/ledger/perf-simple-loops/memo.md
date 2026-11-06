# perf-simple-loops: validating a native tree's memo (round 6)

- Why: after #390 and #400, emptyFor/scalarFor cost ~1480 instructions a
  call (7.15e-8 s; Ř 6.9e-8 / 7.2e-8). ~200 of them were the memo check
  of `closure::run`: upgrade a weak reference per closure of the tree into
  a SmallVec, fold `chain_version` over their environments, drop it.
- For a tree whose closures are all defined on the search path, the
  search epoch says everything (the code already relied on that for trees
  without cells), the weak references keep the closures' addresses from
  being reused (so the root key is the same closure), and the version of
  a search-path environment is 0. Change: `Valid::envs` keeps those
  environments; a run uses them directly (`Insts::Envs`).
- First version cloned every closure's environment on the other path too:
  spectralnorm_alt_3 +1.5%. `Insts` is an enum now (the closures, or the
  kept environments).
- Result: emptyFor/scalarFor peak 7.15e-8 -> 6.87e-8 s with #390 and
  #400 (Ř 6.9e-8 / 7.2e-8). `perf.py compare` against #390:
  profiler-shared -3.3%, 0 slower.
- Verified: conformance (tests/r/jit/native-closures*, native-call-pending
  rebind callees and cells), fastr functions/sys/eval/environment slice;
  sol-loop 2 rounds, approved.
