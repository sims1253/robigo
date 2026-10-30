# substitute dots cells

## 2026-10-30: cold continuation meets the 0.1% target, kept

- Why: preserve GNU R's LANGSXP argument cells without enlarging Lang or
  PNode or adding bookkeeping to ordinary substitute calls.
- Result: all seven targeted instruction comparisons pass at 0.1%.
  Maximum increase +0.08474%, versus the supplied original regression
  of 0.68–1.05%. Generic repeat: 588,080,911 (+0.04794%).
- Setup: exact CI executables for main 805e5fb and code commit 2f50ec7,
  same host, R_HOME pointing to Debian GNU R 4.5.0-3, same native libraries,
  ROBIGO_RDBFLAT=0. scripts/perf.py compare --threshold 0.1 passes.

| workload | main 805e5fb | cold continuation | change |
| --- | ---: | ---: | ---: |
| rbench/simple/bytecodes | 232,943,804 | 232,911,773 | -0.01375% |
| robigo/fib | 960,428,420 | 960,342,871 | -0.00891% |
| robigo/s4-generic | 587,799,124 | 588,297,226 | +0.08474% |
| robigo/s4-new | 629,214,041 | 629,333,365 | +0.01896% |
| robigo/s4-slot-read | 461,588,447 | 461,785,816 | +0.04276% |
| robigo/s4-slot-write | 487,524,843 | 487,698,220 | +0.03556% |
| robigo/s4-valid | 627,582,879 | 627,674,291 | +0.01457% |

- Ordinary PNode layout and conversion remain identical to main. The
  reader transfers to read_call_tail at the first exceptional cell;
  tracking and registry insertion happen in that cold continuation.
  substitute allocates indices only when unbound dots are encountered.
  The 64-cell limit is removed. Attach head attributes before registering
  a weak reference, preserving attributed calls across deserialization.
- Correctness: conformance ratchet 750/753 -> 753/753 executions across
  AST/VM/JIT. The expanded oracle fixture covers tagged, bound, empty,
  missing and unbound dots, arguments beyond index 64, and head attributes.
  Recorded with scripts/regen-expected.sh in robigo-oracle:4.5.0.
  Both original S4 class-generator snippets (4a71e01f0301e4a5 and
  bfd38dfb85177afa) match fresh GNU R output byte for byte with methods
  attached, including empty stderr.
- CI build 41003616546 passes: release build, workspace tests, parser
  oracle, all conformance tiers, gctorture, and parse corpus ratchet.
  GNU R tests, demos, r-intro, C API and cran-purer jobs passed when
  inspected; remaining full corpora/performance jobs were still running.
- Local runtime unit attempt: 44/46 pass. The two unchanged complex
  C-library oracle tests differ by one bit on the host libc (sqrt/csin).
  complex.rs is identical to main; Debian CI's full tests pass. No tests
  were skipped or weakened. Local release builds completed for earlier
  experiments; the final exact-source executable came from CI. Canceled
  only the redundant local flock waiter after verification, never a compiler.
- GNU R duplicate.c:duplicate1/duplicate_list normalizes duplicated call
  tails to LISTSXP. Losing registry metadata on a Lang copy is intentional:
  attribute assignment was checked separately against GNU R and matches.
  Deserialization does not duplicate, so its cell types must survive.
- Scope: builtins/lang.rs, serialize.rs, the fixture and this ledger.
  Shared edit: one added tests/r/passing.txt entry. No corpus records,
  rlib sources, or other tracks' runtime code changed.
- Open: finish the final CI run and coordinator review/merge. Do not merge
  this PR from the track. The full CI performance gate covers benchmarks
  beyond the seven measured locally.

## 2026-10-30: Rejected marker state in the common reader

- Marker candidate d27070d passes 753/753 ratchet executions. CI build,
  unit tests and conformance pass. Exact 805e5fb/candidate binaries,
  same natives, cache off: fib +0.0025%, bytecodes +0.0183%, S4 generic
  +0.1376%, new +0.1117%, slot read +0.1484%, slot write +0.1394%,
  valid +0.1085%. Rejected at the requested 0.1% limit.
- Profile: only ONE exceptional deserialized call. The residual is
  +612,734 instructions in read_item_flags0 (inlined common reader),
  versus only 80 in exceptional_cells and 682 in set_lang_cells.
- Next experiment restores the baseline reader and transfers to a cold
  continuation at the first LANGSXP tail. All side-vector state, cell
  tracking and special call construction live exclusively in that
  continuation. No recursion per exceptional cell. Attach attributes
  before registry insertion to preserve the weak reference.


## 2026-10-30: Rejected unconditional side-vector state

- Local d81d29c versus the streaming reader: fib 958,598,789 -> 958,628,116;
  bytecodes 231,035,544 -> 231,139,073; S4 generic 585,150,263 -> 585,978,244;
  new 622,732,066 -> 623,288,207; slot read 459,461,548 -> 460,165,835;
  slot write 485,423,431 -> 486,144,190; valid 625,465,013 -> 626,120,300.
  Rejected: S4 overhead remains 0.10–0.15%, above 0.1%.
- Expanded oracle fixture and both original class-generator cases match.
  FastR substitute/serialize slice remains 259/262, with the same existing
  failure identities as baseline. Case 6260e145e42c04ad timed out under host load.
- Next experiment uses the unused tail attribute slot in PNode as a rare
  LANGSXP marker. Only the head attribute is retained by the existing
  reader. Collect indices on the exceptional path; ordinary reads retain
  only a boolean and perform no side-vector initialization or drop.


## 2026-10-30: Rejected smaller node experiment

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


## 2026-10-30: initial split and scope

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
