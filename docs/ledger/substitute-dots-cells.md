# substitute dots cells

## 2026-10-30: split fix and remove common deserialization storage

- Rebased the two runtime commits from track/substitute-dots-cells onto
  d81d29c. Dropped four inherited methods/corpus commits from this PR.
- Baseline reproduces the serialized LISTSXP (02) versus GNU R LANGSXP (06)
  mismatch for unbound dots. Debian GNU R 4.5.0-3 recorded the fixture,
  including tags, empty/missing dots and arguments beyond index 64.
- Candidate removes the unused per-node attribute value from PNode. Only
  the head attribute is consumed; it already lives in head_attr. The cell
  type fits within a smaller node than main, with no Lang layout change.
- Replace the 64-bit mask with indices allocated only for exceptional
  cells. Ordinary substitute calls do no bit operations or registry calls.
  Keep weak references for exceptional calls to prevent address reuse;
  serialization consults the registry, evaluation does not.
- Spec: coerce.c:substitute/substituteList/do_substitute and
  serialize.c:ReadItem_Iterative. Reviewed eval.c:R_forceAndCall and
  make_applyClosure_env: their promise/missing-argument behavior does not
  reconstruct the distinction between quoted and substituted dots cells.
- Initial split CI: build, gnur-tests, gnur-demos, r-intro, capi and
  cran-purer passed. Candidate measurements and verification pending.

### Rejected smaller node experiment

- CI candidate 15149bf passes all corpora, unit tests and 750/750 conformance
  ratchet executions locally using its CI executable. Both S4 generator
  serialization cases match freshly recorded GNU R output byte for byte.
- Exact d81d29c/candidate CI binaries, same host, same native libraries,
  flat cache disabled: fib +0.055%, bytecodes +0.222%, S4 generic +0.632%,
  new -0.058%, slot read +0.840%, slot write +0.791%, valid +0.606%.
  Rejected: exceeds the requested 0.1% limit.
- Slot-read callgrind attribution: +1,217,622 instructions in the additional
  cell-index collection pass, +1,687,660 in allocator reallocation. The
  smaller PNode also changes in-place Vec conversion behavior.
- Next experiment restores the original PNode layout and collects rare
  cell indices during streaming, eliminating the extra pass and preserving
  the ordinary reader's allocation behavior. Measurements pending.
