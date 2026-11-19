## 2026-11-18: scripts/rt-base-build.sh, GNU R's library build with robigo-rt as R (L4)

- What: `scripts/rt-base-build.sh` runs `src/library/Makefile.in`'s R target
  for profile, translations, base, tools and compiler, with robigo-rt as
  the R the Makefiles call, and reports what stops each step. The steps are
  the packages' `all`, base's and tools' `mklazycomp`, base's `mkdesc`, and
  the two `.vinstall_*_as_RDS` commands. The package Makefiles and
  `share/make/*.mk` run unedited.
  - The build tree gets a minimal `Makeconf` (only the variables the R
    target reads, as configure sets them on the oracle image), and the
    Makefiles and DESCRIPTIONs from their `.in` files.
  - `bin/R` execs a copy of robigo-rt in the install layout, so the build
    tree is its R_HOME with or without #836. tools.so comes from
    `scripts/rt-home.sh`.
  - The R target stops before the bibliographies and dictionaries, which
    need utils.
- `--shim FILE` reads R stand-ins before each step's script. They live in
  `scripts/rt-base-build-shim.R`, each naming its owner, and probe how far
  the build gets past internals others are porting. They are never part of
  robigo-rt.
- Not in CI yet (the queue is deep): run by hand with a binary of the
  stack of interest, `--robigo-rt BIN`.
- The stop list, the merge's binary at the time of the PR: in the PR text.
