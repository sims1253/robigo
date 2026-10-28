# robigo's C headers

The headers packages compile against (`R_HOME/include` of robigo's home,
`scripts/robigo-home.sh`; docs/core.md §8). They are GNU R 4.5.0's
installed public headers (GPL-2 | GPL-3, like `rlib/`), copied from the
`r-base-core 4.5.0` install (`/usr/share/R/include`: the files of
`src/include` plus the configure-generated `Rconfig.h`, `Rmath.h` and
`Rversion.h` for x86_64 Linux).

GNU R 4.5's public API is functions only (no field access without
`USE_RINTERNALS`, which packages can no longer use), so nothing here is
layout dependent: a package built against these headers or GNU R's own
calls the same exported functions. robigo changes a header only where its
`libR` differs from GNU R's by design, and marks each change with
`robigo:`.
