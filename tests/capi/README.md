# libR's C API, as packages use it

`scripts/capi-census.py scan` writes `symbols.tsv`: every libR symbol
imported (`nm -D --undefined-only`) by the shared libraries of GNU R's
own packages (`$(R RHOME)/library/*/libs/*.so`) and of the installed
CRAN packages (`$R_LIBS_USER/*/libs/*.so`), intersected with what
`libR.so` exports. Columns:

| column | meaning |
|---|---|
| `symbol` | the exported name packages link against |
| `kind` | `function` or `data` (from `nm`'s type letter) |
| `n_pkgs` | how many packages import it — the weight |
| `gnur_pkgs` | which of GNU R's own packages import it |
| `header` | the header(s) under `$(R RHOME)`'s include directory declaring it (empty: none do — packages declare them `extern` themselves) |
| `api` | `api`/`eapi`/`emb` per `tools:::funAPI()` (R 4.5's lists in "Writing R Extensions"); empty: not documented API |

`scripts/capi-census.py coverage` builds the `capi` corpus and reports
the weighted share of the census its cases exercise (see
`tests/corpora/capi/`). `scripts/capi-census.py symbols REGEX...` greps
the census.
