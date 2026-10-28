# Pure-R CRAN programs

Help-page examples, demos and standalone R tests from five pinned CRAN
source packages. `packages.txt` records versions, SHA-256 hashes and
licences. Original source archives, including copyright and licence files,
are in `src/`. No download is needed to run the corpus in CI.

| Package | Version | Licence | Required dependencies besides R |
|---|---|---|---|
| R6 | 2.6.1 | MIT, see archive LICENSE | none |
| withr | 3.0.3 | MIT, see archive LICENSE | graphics, grDevices |
| assertthat | 0.2.1 | GPL-3 | tools |
| proto | 1.0.0 | GPL-2 | none |
| R.methodsS3 | 1.8.2 | LGPL-2.1-or-later | utils |

Each source DESCRIPTION declares `NeedsCompilation: no`. Required
Imports/Depends contain only R and bundled packages, with no external
compiled dependencies. Optional Suggests are not installed. The runtime's
base-package native routines remain necessary for graphics examples.

`build.sh` verifies archive hashes and installs the packages with GNU R
4.5.0 into the ignored `support/lib`, without bytecode. The corpus runner
links that library into each case's isolated directory. Cases prepend
`.libPaths("lib")`, attach their package, and set seed 1.

Regenerate cases with `python3 scripts/import-corpus-scripts.py cran-purer`
in the GNU R 4.5.0 oracle container. Help examples use `tools::Rd2ex`, with
`dontrun` and `donttest` blocks commented. Testthat entry points are excluded
because testthat requires compiled dependencies. The proto gWidgets demo
requires a display; withr's database example requires DBI and a driver.

Seven help pages are excluded for nondeterministic or machine-specific
output: assertthat's `assertions-file`, and withr's `with_dir`,
`with_libpaths`, `with_tempfile`, `with_timezone`, `withr`, and
`with_makevars`. The latter compiles C code and prints compiler flags.
The others print random temporary paths, library paths, or the current time. Exclusions are
explicit in `CRAN_SKIP`, rather than masking recorded output.

Record with `python3 scripts/corpus.py record cran-purer --all --repeat 2
--jobs 4` inside `robigo-oracle:4.5.0`. Run with `python3 scripts/corpus.py
run cran-purer --jobs 4 --keep-output`. See
`docs/ledger/corpus-scripts.md` for results and failure ownership.

The initial robigo passing list contains 31 of 41 cases. The corpus manifest
pins PATH to `/usr/local/bin:/usr/bin` for the `with_path` examples.
