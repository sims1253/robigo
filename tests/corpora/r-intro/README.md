# `r-intro`: the code of "An Introduction to R"

"An Introduction to R" (`doc/manual/R-intro.texi` in the GNU R 4.5.0
sources) is the manual new users read, and its examples are the kind of
code an everyday R script is made of: vectors and factors, arrays,
data frames, distributions, models, graphics, packages.

The corpus turns the manual's runnable examples into whole scripts — one
per chapter, plus the appendix A sample session — and records GNU R's
output for each. Cases run in `echo` mode, like the manual's transcripts.

## How the cases are built

`scripts/import-corpus-scripts.py r-intro` reads the `.texi` source:

- **`> ` transcripts.** The manual shows examples with R's prompt; the
  commands are the `> ` lines. Continuation lines in the manual are *not*
  marked with R's `+ ` prompt (they are aligned), so a command continues
  onto the following lines as long as brackets or quotes are open. The
  lines after it are the printed output and are dropped — GNU R prints them
  again when the case is recorded.
- **Bare code blocks** (no prompt) are code only if they parse as R; the
  other bare blocks are printed output, shell sessions or syntax templates
  (`> x[i]` with `i` a placeholder), and are dropped.
- **Block-level filters** drop what a batch session cannot do: `scan()`,
  `read.table()` and other file readers (their data files do not ship with
  R), `library()`/`help()`/`example()`/`demo()`/`vignette()`, `source()`,
  `sink()`, `browser()`, `locator()`, device setup (`png()`, `pdf()`,
  `dev.off()`), `setwd()`, `system()`, `q()`, and anything that prints the
  time (`date()`, `Sys.time()`, `.Last` — `.Last` *does* run when R exits).
- **Execution pruning.** Examples build on earlier sections and chapters;
  a batch session stops at the first error, so an example whose objects come
  from a section that was left out would hide everything after it. The
  importer runs each chapter under GNU R 4.5.0 with a marker before every
  example block and drops the block the session dies in, until the chapter
  runs through. Every case therefore runs to completion in GNU R (status 0),
  and what remains is the runnable part of the manual.
- A chapter with fewer than two runnable statements gets no case: today that
  is chapter 9 (Grouping, loops and conditional execution) and appendix
  "Invoking R", whose examples are all syntax templates or command-line
  options.

The preamble of every case is `options(keep.source = TRUE, useFancyQuotes =
FALSE, warn = 1)` and `set.seed(123)`, as GNU R's own `tests/demos.R` does.

## Determinism

All 13 cases are byte-identical between two GNU R 4.5.0 runs (no masked
lines, no `unstable` cases).

## Importing

```sh
# in the R 4.5.0 oracle image: the parse check and the pruning run Rscript
scripts/import-corpus-scripts.py r-intro     # needs $ROBIGO_RSRC
scripts/corpus.py record r-intro             # needs GNU R 4.5.0
```

## Licence

The manual and its examples are part of GNU R 4.5.0: GPL-2 or later.
