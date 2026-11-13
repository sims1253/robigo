# libR's C API, as packages use it

One census per R version:

| file | R | used by |
|---|---|---|
| `symbols.tsv` | 4.5.0 | the frozen old runtime: `crates/robigo-capi` (stubs, coverage), `scripts/capi-census.py coverage` |
| `symbols-4.6.tsv` | 4.6.2 | G3 (`docs/g3.md` §8, `docs/ledger/g3.md`) |

`scripts/capi-census.py scan` writes the census of the R on `PATH`: every
libR symbol imported (`nm -D --undefined-only`) by the shared libraries
of the packages on that R's `.libPaths()` (`$(R RHOME)/library`,
`R_LIBS`, `R_LIBS_USER`, `R_LIBS_SITE`; the first copy of a package
wins), intersected with what `libR.so` exports. GNU R's own packages are
those of priority base or recommended. Columns:

| column | meaning |
|---|---|
| `symbol` | the exported name packages link against |
| `kind` | `function` or `data` (from `nm`'s type letter) |
| `n_pkgs` | how many packages import it — the weight |
| `gnur_pkgs` | which of GNU R's own packages import it |
| `header` | the header(s) under `$(R RHOME)`'s include directory declaring it (empty: none do — packages declare them `extern` themselves) |
| `api` | `api`/`eapi`/`emb` per that R's `tools:::funAPI()` (its lists in "Writing R Extensions"); empty: not documented API |

`scripts/capi-census.py coverage [--census 4.6]` builds the `capi` corpus
and reports the weighted share of a census its cases exercise (see
`tests/corpora/capi/`). `scripts/capi-census.py symbols [--census 4.6]
REGEX...` greps a census. `--census` defaults to 4.5.

## How `symbols-4.6.tsv` was made (2026-11-13)

- GNU R 4.6.2 from `R-4.6.2.tar.gz` (sha256
  `c297040f6392b1dac6e5c0eb398422cfdaedf540ae2205edb9ad0d10cb10ee00`),
  configured with `scripts/build-oracle.sh`'s flags plus
  `--enable-R-shlib`, and without its `--with-x=no --without-tcltk
  --without-cairo`, so that GNU R's own `tcltk.so` and grDevices'
  `cairo.so` exist as in the 4.5 census: installed to
  `~/.cache/robigo-research/R-4.6.2-census`. Its `libR.so` exports the
  same symbols as the oracle-style build in
  `~/.cache/robigo-research/R-4.6.2`, plus `R_setX11Routines`.
- The recommended packages from the same tarball
  (`src/library/Recommended/*.tgz`, in `R_PKGS_RECOMMENDED` order),
  installed with the oracle-style R into
  `~/.cache/robigo-research/R-4.6.2-recommended`.
- The CRAN packages in the workstation's `~/R/library`, built for R 4.6.1
  (the same API as 4.6.2; `nm` reads them, nothing loads them). Its
  `MASS`, `Matrix` and `lattice` are shadowed by the recommended ones.

```sh
PATH=~/.cache/robigo-research/R-4.6.2-census/bin:$PATH \
R_LIBS=~/.cache/robigo-research/R-4.6.2-recommended R_LIBS_USER=~/R/library \
  scripts/capi-census.py scan
```

`symbols.tsv` came from a GNU R 4.5.0 install with the recommended
packages and 79 CRAN shared libraries (2026-10-27, on a machine that no
longer has that library). Its exports and API column match the 4.5.0
oracle image's `libR.so` and `tools:::funAPI()`. `docs/ledger/g3.md`
(2026-11-13) has the diff between the two censuses.
