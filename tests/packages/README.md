# The package suite

The most used CRAN packages' own test suites, on robigo, against GNU R. See
`docs/verification.md` for the metric; `scripts/package-suite.py` is the
tool.

| file | what | written by |
|---|---|---|
| `selection.tsv` | the packages, with the figures that selected them; `framework`, `filter` and `env` mirror each package's own test entry point, and `server` names the local test server its tests talk to | `select`, plus hand edits of those four columns |
| `rproject.toml`, `rv.lock` | the selection and its Suggests, pinned to one PPM snapshot | `lock` |
| `sources.tsv` | the selection's source tarballs, with checksums (rv's binaries carry no tests and `rv.lock` no checksums) | `lock` |
| `expected/<pkg>.jsonl.gz` | GNU R's outcome of every test file and block, recorded three times | `record`, which runs GNU R only |
| `passing/<pkg>/<file>/<id>.pass` | the ratchet: one empty marker per block that matches | `run --update` |
| `own-skips.tsv` | skip reasons a package's own code always takes, each citing its source line; such blocks leave the green floor's denominator | by hand |
| `smoke/cases/*.R` | user-level scripts, transcript-checked (`package-suite.py smoke`) | by hand |
| `smoke/expected/<case>.{stdout,stderr,status}` | GNU R's transcript of a smoke case, masked as runs compare | `smoke --record`, GNU R only |
| `smoke/passing/<case>.pass` | the smoke ratchet | `smoke --update` |
| `runner.R` | runs one test file in one process and streams events | |
| `canary/` | a package with one test per outcome, to check the harness on robigo | |
| `Dockerfile`, `apt.txt` | `robigo-oracle:4.5.0` and the system libraries the pinned packages link against, and the pinned test servers | |

A package enters the selection above 900,000 downloads in 30 days and 500
packages using it (distinct packages naming it under Depends, Imports or
LinkingTo, the R Observatory's "Used by"), and leaves only below 810,000 or
450, so a package at the line does not flap. The list is regenerated from
CRAN's index at the snapshot and cranlogs, because the Observatory's paged
listing refuses scripts and its data release is a 770 MB download. The
selection of snapshot 2026-11-01 was checked against that release: the same
20 packages.

## Test servers

The tests never reach the internet. A package whose tests need an HTTP
server gets one on the loopback of each test file's own network namespace,
started before R and killed with the file, the same for GNU R and robigo:

| `server` | what | the tests find it by |
|---|---|---|
| `httpbin` | `python3-httpbin` 0.10.2 (Debian trixie, pinned in `apt.txt`) on `127.0.0.1:80`, as `httpbin.org` | `HTTPBIN_TEST_SERVER=http://httpbin.org` |

curl uses it. Its helper reads `HTTPBIN_TEST_SERVER`, but through
`Sys.getenv("HTTPBIN_TEST_SERVER", find_test_server())`, and `Sys.getenv`
forces its default: the helper probes its fallback list whatever the
variable says, and stops when none of them answers. The last of the list is
`http://httpbin.org`, so that is the server's name. The name resolves to
127.0.0.1 in the whole oracle (`docker run --add-host`, and the options of
the CI container in `.github/workflows/packages.yml`); an `/etc/hosts` of each
worker's own would need a mount, which Docker's AppArmor profile refuses. A
worker without the server has nothing listening there, so the name still
leads nowhere, and the harness stops when the name does not resolve to the
loopback. Tests that name a host of the internet themselves
(`https://httpbin.org`, `https://google.com`) still find no network: GNU R
fails or skips them, so they are not scored. httr's tests are such: most of
them call `skip("httpbin is not reliable")` before anything else, and the rest
name their hosts, so httr runs without a server. A new server is an entry of
`SERVERS` in `scripts/package-suite.py` and a pinned package in `apt.txt`;
its responses are part of the recorded outcomes, so changing its version
means recording its packages again.

## Skips of the package's own

A package is green only when at least 80% of its blocks are scored, so that a
package whose tests mostly skip here (a missing Suggests, no network) does not
look done. Some skips have nothing to do with the machine: httr's
`skip_httpbin()` is `skip("httpbin is not reliable")`, called unconditionally.
`own-skips.tsv` lists such reasons for each package and version. A block GNU R
records as skipped, steadily and only for listed reasons, leaves the floor's
denominator. It is still scored as a block that robigo must skip the same way.
An entry needs a comment citing the line of the package's source that makes
the skip unconditional. `skip_on_ci()`, `skip_if_not_installed()`,
`skip_if_offline()` and tests of internet hosts are never listed: those are
exactly what the floor is for. The unit tests check that every entry has its
comment, matches the selection's version and appears in the recording, so a
new snapshot has to check the entries again.

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

## Own R packages (dogfooding)

The user's own R packages (github.com/sims1253) run as a separate group
(`--own`), scored and ratcheted like the CRAN selection but never mixed into
its numbers. They are pinned by git commit, not a PPM snapshot.

| file | what | written by |
|---|---|---|
| `own-selection.tsv` | the packages, pinned commits, test framework/filter, and exclusion reasons | by hand |
| `own-rproject.toml`, `own-rv.lock` | the non-excluded own packages and their dependencies, resolved from the CRAN snapshot plus git sources | `lock --own` |
| `own-pins.tsv` | each own package's version at its pinned commit | `lock --own` |
| `own-expected/<pkg>.jsonl.gz` | GNU R's outcome of every test file and block | `record --own`, which runs GNU R only |
| `own-passing/<pkg>/<file>/<id>.pass` | the ratchet | `run --own --update` |
| `smoke/cases/own-*.R` | user-level README/example scripts per own package | by hand |

```sh
scripts/package-suite.py lock --own
scripts/package-suite.py record --own [PKG...]
scripts/package-suite.py run --own [PKG...] [--update]
scripts/package-suite.py show --own PKG [--file PAT] [--all]
scripts/package-suite.py smoke own-dagriculture   # transcript-checked user scripts
```

Packages whose dependency tree is impractical in CI are listed in
`own-selection.tsv` with `reason_excluded` and are not run; they still get a
smoke case (absent until their dependencies are available). A filter drops
a test file only when GNU R itself cannot run it within the budget or it
dominates the nightly; docs/ledger/packages.md gives the measured time of
every file a filter drops. bayesfam keeps 8 files (33 s under GNU R, against
3,679 s and seven unfinished files unfiltered) and bayesgrove drops two
(245 s against 1,166 s). The nightly runs all six; a pull request runs
ciecam16.
