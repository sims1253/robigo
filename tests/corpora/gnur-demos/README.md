# `gnur-demos`: GNU R 4.5.0's demos

Each case is one of the `demo()`s shipped with GNU R 4.5.0's base
packages (`src/library/*/demo/*.R` in the R sources), run the way
`demo(topic, package = pkg, ask = FALSE, echo = TRUE)` runs it — the
transcript `R CMD check` produces, since GNU R's own `tests/demos.R` calls
`demo()` with `echo = TRUE` in echo mode too.

A case file is the demo source with a short preamble:

```r
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)  # as tests/demos.R
set.seed(123)                                                   # demos draw random numbers
cat("\n\n\tdemo(<topic>)\n\t---- ~~~~~~~\n", sep = "")          # the demo() banner
```

The corpus runs in `echo` mode, so R's prompt echoes each top-level
expression, exactly what `source(echo = TRUE)` prints inside `demo()`.
Cases are seeded individually (GNU R's `tests/demos.R` seeds once for the
whole run) so that a case does not depend on the demos that sort before it.

## What is included, and why

All 20 demos in the R sources; 16 run as cases:

| package | demos |
|---|---|
| base | `error.catching`, `is.things`, `recursion`, `scoping` |
| graphics | `graphics`, `Hershey`, `image`, `Japanese`, `persp`, `plotmath` |
| grDevices | `colors`, `hclColors` |
| stats | `glm.vr`, `lm.glm`, `nlm`, `smooth` |

- `tcltk`'s four demos (`tkcanvas`, `tkdensity`, `tkfaq`, `tkttest`) are
  **not** included: they open Tk windows and need a display, which a corpus
  case (a subprocess with no X server) has not.
- `Hershey`, `Japanese`, `plotmath`, `lm.glm` and `nlm` are the demos GNU R
  leaves out of `tests/demos.R` (they live in `tests/demos2.R`, whose output
  `make check` does not diff) because they print a great deal of text and
  glyph tables. They are deterministic here and are included: they exercise
  `str()`, plotting math and modelling code that scripts do use.
- Graphics go to the default device (`pdf()` in a batch session, written
  into the case's temporary directory and thrown away with it); no device
  is opened by the harness.

## Determinism

`scripts/corpus.py record gnur-demos` runs every case twice; all 16 are
byte-identical between runs (no masked lines, no `unstable` cases). The only
nondeterminism in the demos is `rnorm()`/`runif()`, which the `set.seed(123)`
preamble pins.

## Importing

```sh
scripts/import-corpus-scripts.py gnur-demos   # needs $ROBIGO_RSRC (the R sources)
scripts/corpus.py record gnur-demos           # needs GNU R 4.5.0
```

## Licence

The demo scripts are part of GNU R 4.5.0: GPL-2 or later.
