# Pinned timezone data

These compiled TZif files come from Debian trixie's `tzdata
2026e-0+deb13u1`. The original recording's exact tzdata version is unknown.
`zones.Rout` was re-recorded with these files and Debian's GNU R 4.5.0-3.
IANA timezone data is public domain.

The fixture includes every named zone used by `zones.R` and `internals.R`.
Copies dereference Debian's links, so no file points outside this directory.
`posixrules` supplies default DST rules for POSIX strings such as `EST5EDT`.
`JST-9` and `<+0330>-3:30` are parsed POSIX strings; `Foo/Bar` deliberately
has no file. Both scripts run with `TZ=UTC` and explicit zone arguments,
so neither consults `/etc/localtime`. Debian 2026e has no `EST5EDT` file.

The adjacent `.tzdir` sidecars select this directory for both the oracle
recorder and every conformance execution mode. GNU R's `datetime.c:set_tz`
uses system `tzset()` on Debian; glibc reads `TZDIR`. Robigo already reads
`TZDIR` in `datetime/tz.rs:tzfile_read`, including for `posixrules`.
No runtime change is needed.

To regenerate from the pinned package in `robigo-oracle:4.5.0`, run from
the repository root:

```sh
docker run --rm -v "$PWD:/work" -w /work robigo-oracle:4.5.0 bash -c '
  set -e
  dpkg-query -W tzdata | grep -q "2026e-0+deb13u1$"
  cd tests/r/datetime/zoneinfo
  while read -r hash zone; do
    cp -L "/usr/share/zoneinfo/$zone" "$zone"
  done < SHA256SUMS
  sha256sum -c SHA256SUMS
'
docker run --rm -v "$PWD:/work" -w /work robigo-oracle:4.5.0 \
  bash scripts/regen-expected.sh tests/r/datetime/zones.R tests/r/datetime/internals.R
```

For a deliberate tzdata upgrade, install the chosen package first, update
the version above and regenerate `SHA256SUMS` after copying the files.
Always re-record the outputs with GNU R 4.5.0. Never edit them by hand.
Check that recordings match with different system tzdata versions while
keeping this fixture fixed.
