# The package suite

The most used CRAN packages' own test suites, on robigo, against GNU R. See
`docs/verification.md` for the metric; `scripts/package-suite.py` is the
tool.

| file | what | written by |
|---|---|---|
| `selection.tsv` | the packages, with the figures that selected them; `framework`, `filter` and `env` mirror each package's own test entry point | `select`, plus hand edits of those three columns |
| `rproject.toml`, `rv.lock` | the selection and its Suggests, pinned to one PPM snapshot | `lock` |
| `sources.tsv` | the selection's source tarballs, with checksums (rv's binaries carry no tests and `rv.lock` no checksums) | `lock` |
| `expected/<pkg>.jsonl.gz` | GNU R's outcome of every test file and block, recorded three times | `record`, which runs GNU R only |
| `passing/<pkg>/<file>/<id>.pass` | the ratchet: one empty marker per block that matches | `run --update` |
| `runner.R` | runs one test file in one process and streams events | |
| `canary/` | a package with one test per outcome, to check the harness on robigo | |
| `Dockerfile`, `apt.txt` | `robigo-oracle:4.5.0` and the system libraries the pinned packages link against | |

A package enters the selection above 900,000 downloads in 30 days and 500
packages using it (distinct packages naming it under Depends, Imports or
LinkingTo, the R Observatory's "Used by"), and leaves only below 810,000 or
450, so a package at the line does not flap. The list is regenerated from
CRAN's index at the snapshot and cranlogs, because the Observatory's paged
listing refuses scripts and its data release is a 770 MB download. The
selection of snapshot 2026-11-01 was checked against that release: the same
20 packages.

## Moving to a new snapshot

New package versions bring new tests, so a snapshot is an epoch of the
metric: it changes only on purpose, in its own pull request.

```sh
scripts/package-suite.py select --snapshot DATE
scripts/package-suite.py lock --snapshot DATE
scripts/package-suite.py record
scripts/package-suite.py run --update
```

Run the old and the new snapshot on the same robigo commit and put both
numbers in the pull request, so that the step on the chart is explained.
Delete `passing/` before the `--update`: its block ids are hashes of the
old tests. This is the one case where markers go away.
