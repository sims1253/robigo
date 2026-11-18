## 2026-11-18: the robigo-rt corpus ratchets brought up to main

- Why: fastr's `passing-rt/` stood at 22,137 of 31,374 while main passes
  more. For example `TestBuiltin_log` had 0 markers although `log` works
  since #802/#810. The foreman scores trial delegates by these counts, so
  they must be true.
- Cause: `corpus.py absorb` is manual. Main's push runs do run all of
  fastr on robigo-rt (`rt-corpora`, shard 1/1) and upload
  `.robigo/results/` as `corpus-rt-fastr-1of1`, but nothing absorbs it.
  Only PRs whose authors chose to ratchet (RC, BW, AC1) ever added
  markers. A PR's own run is one shard of four, and `--update` is not run
  on CI.
- What:
  - fastr's results on robigo-rt built from main e20eef0d1 (`corpus.py run
    fastr --runtime rt --jobs 12`: 24,154 of 31,373 pass, 0 regressions);
  - the last main push run that uploaded results, 44166202836 at
    b41fc4d8b (24,133 fastr passes), absorbed with the local run;
  - fastr `passing-rt/`: 22,137 → 24,155 (+2,018; `TestBuiltin_log` 0 → 56
    of 123). The newest main runs, 20 of them, were still queued for
    runners on 2026-11-18;
  - the echo-mode corpora's first rt markers from the same CI results:
    gnur-examples 162, gnur-tests 2 (6 in all). Each was checked on
    e20eef0d1's binary locally. `base/system.file` passed on CI but not
    here: its output depends on the library paths. It is left out, and so
    is the one local pass that CI did not have.
