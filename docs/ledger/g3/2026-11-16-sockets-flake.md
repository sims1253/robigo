## 2026-11-16: the gnur-tests `sockets` flake: an unanswered connect, now served on loopback

- What: the foreman's package. `sockets` failed `corpora (small, 1/1)` as a
  REGRESSION on the old runtime twice today (run 43805497322, job
  133096424681, `vm` tier; run 43829598457, job 133173956302), and both
  re-runs passed. The question was whether a fixed port, a timing race or
  a runner difference caused it.
- Tree: branch `g3/sockets-flake` on main `17f10bf8`. The binary under
  test was CI's own `robigo` artifact from main `f21d8b57`. It ran in the
  CI base image `ghcr.io/sims1253/robigo-ci:base-067c755acfb94a9c`, on an
  AMD Ryzen 9 5900X (WSL2 Linux 6.18).

### The cause

The case is GNU R's `tests/sockets.R`. It returns early unless
`nsl("cran.r-project.org")` resolves, then sends HEAD and GET for
`http://developer.R-project.org/inet-tests/ch11b.dat` over `make.socket`
on port 80. It is a test of the live internet. GNU R runs it only in
`make test-Internet-dev`, "intended to be run as a standalone target",
whose failures are allowed. `download.file.R`, the other case of that
target, is already excluded from the corpus as a network case.

The evidence, from the corpus results that CI uploads (`corpus-small-1of1`
and `-vm`, 200 jobs from 2026-11-13 19:02 to 2026-11-16 10:22 UTC):
- **Failure rate:** 4 of 200 jobs failed: 2026-11-14 14:13, 2026-11-16
  01:57, 04:49 and 08:38. Two were in the `vm` tier and two in the
  default tier, never both tiers of one run.
- **Two durations only:** every failure took 134.0 to 137.1 s. Every
  pass took 0.34 to 1.80 s (median 0.84). Nothing lies in between.
- **The mechanism:** the old runtime's `make.socket` is utils' own
  `sockconnect`, whose `Sock_connect` is a blocking `connect()` with no
  timeout of its own. A connect whose SYNs get no answer waits out the
  kernel's retries (`tcp_syn_retries` 6). GNU R 4.6.1, the same C code,
  run against the black-hole address 192.0.2.1:80, printed "socket
  error: Connection timed out" and "socket not established" after
  134.8 s, the failures' duration.
- **Runners:** all eight jobs of the two runs, failures and re-runs alike,
  ran on GitHub-hosted `ubuntu-24.04` runners (Azure) in the CI image's
  container. The corpora job has no self-hosted runner.

So the cause is none of the three suspects. developer.R-project.org (nginx)
sometimes left GitHub's runners unanswered for the whole connect timeout.
robigo behaved as GNU R does.

### The fix

The case now needs no network in CI. The case and its recordings are
unchanged, and so is its ratchet marker.
- **`net_fixture`** in `corpora.toml` (gnur-tests) names a directory,
  `www/`, and the hosts it stands in for, `developer.R-project.org` and
  `cran.r-project.org`.
- **`scripts/corpus.py run --net-fixture`** serves that directory on
  127.0.0.1:80 for the run and points the hosts there in `/etc/hosts`,
  which it restores afterwards. It needs root, which CI's container has.
  The corpora job passes the flag; it does nothing for a corpus without
  a fixture.
- **The server** speaks HTTP/1.1, so the connection stays open after HEAD,
  as the real server's does. It sends HEAD's headers for GET too, so the
  case's first read of the GET response, as long as HEAD's, takes exactly
  the headers.
- **The file:** `www/inet-tests/ch11b.dat` is the file the real server
  serves today: 2,102 bytes, 100 lines, R's `beaver2` data
  (sha256 `15cb8493…9824532`). The case's recorded `str()` line is
  checked against it.
- **Other cases:** no other case of any corpus contacts either host.
  `reg-tests-1e.R` names `http://cran.r-project.org/` in a
  `download.file()` call whose method check fails before any connection.

Run locally without the flag, the case still reaches the real server.

### Proof

- **Without network:** in the CI image with `--network none`, `sockets`
  passes with `--net-fixture` (0.15 s), and fails without it (`nsl()`
  finds nothing, so the case quits early). `/etc/hosts` afterwards
  holds no line of the fixture.
- **Repeated:** 100 runs per tier (default and `vm`) with `--network none`
  and `--net-fixture`: all 200 passed.
- **Tests:** `scripts/test_corpus.py` (`NetFixtureTests`) checks three
  things. The case's exact HEAD-then-GET exchange on one socket gets equal
  header lengths and the file. The hosts are mapped for the run and
  restored. The served file is the one GNU R 4.5.0 and 4.6.2 printed.
- **Workflow:** actionlint reports only the two info-level shellcheck
  notes that main's `ci.yml` already has.
