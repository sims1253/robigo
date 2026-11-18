## 2026-11-18: robigo-rt's locale comes from the environment, as GNU R's does at startup (L)

- What: robigo-rt's isolate always started in `C.UTF-8`, whatever the
  environment said. GNU R's build runs its steps under `LC_ALL=C`, where it
  writes "ANSI_X3.4-1968" as serialize's native encoding and `l10n_info()`
  says UTF-8 FALSE, so no record of an L4 database could be byte-identical.
- Now the front end asks the C library what `setlocale(<category>, "")`
  would set (`robigo_rtcore::clib::env_locale`: `newlocale` per category
  and `nl_langinfo_l(CODESET)`, and the process's own locale is not
  changed). The isolate's `Locale` follows from that. With a UTF-8 codeset
  it is a UTF-8 locale; with "ANSI_X3.4-1968" it is C. `LC_COLLATE` "C" or
  "POSIX" means C collation. serialize's header, `l10n_info()`,
  `mbcslocale` and `R_MB_CUR_MAX` already read the `Locale`.
- A locale the C library lacks: setup_Rmainloop's deferred "Setting
  <category> failed, using "C"" warning for each of its seven categories,
  then "During startup - " and the warnings. They are quiet when neither
  `LANG` nor `LC_ALL` is set and non-empty, as in GNU R.
- Known gap: any other codeset (Latin-1, ...) refuses at startup
  ("robigo-rt: a locale whose codeset is '…' is not implemented yet"),
  since the `Locale` models only C and UTF-8.
- Cases: `tests/rt/platform/locale-c.R` runs under `LC_ALL=C` through a new
  `.locale` sidecar, which `robigo-conformance`'s command and
  `scripts/regen-expected.sh` both read. `locale-utf8.R` has the same lines
  under C.UTF-8. The missing locale is in `crates/robigo-rtapi/tests/cli.rs`.
