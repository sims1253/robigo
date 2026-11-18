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
- What the C locale reaches in the reader (Sol round 1):
  - A string literal without escapes is marked UTF-8 only when its parse
    is `known_to_be_utf8` (gram.y's mkString2). That flag is the session's
    for scripts, the console and `R_ParseVector`, and `parse()`'s own for
    `parse(text =)`. Before this, lowering marked every non-ASCII literal
    UTF-8, so `parse(text = <a native string>)` gave "UTF-8" where GNU R
    gives "unknown", even in C.UTF-8.
  - In a locale that is not UTF-8, a bare name with non-ASCII bytes is
    refused ("robigo: a name with non-ASCII characters in a non-UTF-8 locale
    is not implemented yet"). GNU R's lexer classifies such bytes with
    `isalpha` and raises its multibyte-parser error, which robigo-syntax
    does not port. Strings, comments and backquoted names read as in GNU R.
  - Collation by the environment's `LC_COLLATE` (`R_ICU_LOCALE` unset)
    stays ICU root for C.UTF-8 and English (ICU's "en" has no tailoring).
    For any other locale it refuses, as `R_ICU_LOCALE` already did: root
    order would be wrong there (Swedish sorts "z" before "ä").
- Known gap: any other codeset (Latin-1, ...) refuses at startup
  ("robigo-rt: a locale whose codeset is '…' is not implemented yet"),
  since the `Locale` models only C and UTF-8.
- Cases: `tests/rt/platform/locale-c.R` runs under `LC_ALL=C` through a new
  `.locale` sidecar, which `robigo-conformance`'s command and
  `scripts/regen-expected.sh` both read. `locale-utf8.R` has the same lines
  under C.UTF-8. The missing locale is in `crates/robigo-rtapi/tests/cli.rs`.
