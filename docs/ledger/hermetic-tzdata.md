# Hermetic datetime conformance fixtures

CI's fresh Debian trixie image moved from tzdata 2026c-0+deb13u1 to
2026e-0+deb13u1. PRs #30 and #31 failed identically at `zones.Rout:2040`
inside Europe/Dublin, with recorded hour 12 versus runtime hour 11,
although main had passed minutes earlier. The host's different tzdata
also made this script fail before the change.

Pin the 22 required compiled zone files, including `posixrules`, from
Debian tzdata 2026e. The exact version behind the original recording is
unknown. `.tzdir` sidecars select the fixture before process startup in
both the GNU R recorder and every conformance execution mode. The other
named-zone conformance script, `internals.R`, uses the same fixture.
GNU R 4.5.0's `datetime.c:set_tz` calls system `tzset` on Debian; robigo's
existing glibc port already honors `TZDIR` in `tzfile_read`. Neither test
uses the host's default localtime file.

GNU R recordings were regenerated in `robigo-oracle:4.5.0`, with
`r-base-core 4.5.0-3`. Only seven lines in the Dublin block changed;
`internals.R` and both scripts' stderr/status recordings are unchanged.
Re-recording in a disposable container after downgrading system tzdata
to 2026c-0+deb13u1 produced identical SHA-256 hashes for all six outputs.
The fixture's checksums and documented regeneration commands pass.
Local release conformance passes 172/172 with all three test functions
passing, including interpreter/VM/JIT comparisons. `cargo fmt --check`
passes. A fresh Debian build inside `robigo-oracle:4.5.0` also passes
172/172 and all three conformance test functions. Ratchet entries and R
scripts are unchanged.

## Other timezone dependencies

A search of all conformance scripts found only `zones.R` and `internals.R`
using datetime conversions or named zones; both now use the fixture.
The same search of the corpora found named-zone conversions in:

- `gnur-tests/cases/datetime.R` and `datetime2.R` through `datetime5.R`
  (including Dublin in `datetime3.R`).
- `gnur-tests/cases/reg-tests-1c.R` (Stockholm), `reg-tests-1d.R`
  (Vienna and unset TZ), and `reg-tests-1e.R` (Berlin).
- `gnur-examples/cases/base/DateTimeClasses.R` (Los Angeles).
  `as.POSIXlt.R` also contains a commented New York example.

`gnur-examples/cases/base/timezones.R` enumerates `OlsonNames()`, so it
also depends on the installed database's names. Other datetime examples,
such as `strptime.R`, `weekday.POSIXt.R`, and `balancePOSIXlt.R`, mainly
use UTC or the default zone. These corpora have a separate recorder,
additional zones, upstream source imports and zone enumeration; changing
them requires a separate fixture/recording change. No corpus expectations
or passing lists were modified here.
