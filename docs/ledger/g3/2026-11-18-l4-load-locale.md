## 2026-11-18: readChar, loadFromConn2, Sys.getlocale, Sys.setlocale (L4)

- Why: two of rt-base-build.sh's step stops (#884). tools' sysdata step
  `load()`s a file, which reads its magic number with `readChar` and then
  calls `loadFromConn2`. base's mkdesc step calls
  `Sys.getlocale("LC_TIME")` and then `Sys.setlocale("LC_TIME", "C")`.
- **readChar** (connections.c's do_readchar, readFixedString,
  rawFixedString; `connections/readers.rs`): characters, or bytes with
  `useBytes`, of a connection (opened in "rb" and closed again when it was
  not open) or of a raw vector. Includes the UTF-8 checks, the NUL warning
  once per call, and the text-connection warning (GNU R's condition is
  always true for a text connection, since `nPushBack >= 0`).
- **loadFromConn2** (saveload.c's do_loadFromConn2, PRIMVAL 0, with
  `loadInfoFromConn2` from #880 as PRIMVAL 1, and RestoreToEnv): the
  magic number checked, the list or pairlist unserialized, each element
  bound in `envir`, with the old-style S4 warning (`warningcall(R_NilValue,
  ...)`). `verbose = TRUE` (R_InitReadItemDepth < 0, the tags printed as
  they are read) is refused.
- **Sys.getlocale / Sys.setlocale** (platform.c): the names come from the
  C library, as `setlocale(category, NULL)` would give them.
  - robigo-rt never sets the process's locale. Each isolate keeps the
    names per category (`clib::LocaleNames`, `Cx::locale_names`). At
    startup they come from `newlocale(mask, "")` of the categories GNU R
    sets (main.c), read back with glibc's `_NL_LOCALE_NAME`. LC_NUMERIC and
    the categories GNU R does not set stay "C".
  - `LC_ALL` is glibc's composite (setlocale.c's new_composite_name): the
    one name when every category has it, else
    `LC_CTYPE=...;LC_NUMERIC=...;...`.
  - Sys.setlocale resolves the new name with `newlocale` too, so "" reads
    the environment and "POSIX" becomes "C", as setlocale does, and a
    locale the C library lacks gives "" and GNU R's warning.
  - It changes only what robigo-rt models; anything else is refused,
    before any change is made. `LC_CTYPE` may change only within one
    codeset (UTF-8, or ASCII), `LC_COLLATE` not at all (the ICU collator's
    reset is not modelled), and the other categories only between the
    C-like locales (C, POSIX, C.UTF-8). LC_TIME's month names, for one,
    are C's in robigo-rt.
- Cases recorded on the 4.6.2 oracle and ratcheted:
  `tests/rt/connections/readchar.R`, `tests/rt/serialize/load.R` (on
  files GNU R saved, in `tests/rt/serialize/data/`),
  `tests/rt/platform/getlocale.R`, and `getlocale-c.R` under LC_ALL=C.
- Gaps found: `xzfile` connections (load of an xz-compressed file);
  `unlink` (B).
- After #854 merges: `clib::resolve` and `codeset_of` call `newlocale`,
  which reads the environment, so they take #854's `env_read()` guard
  (the probe stack has it).
- Stop list after this, on the probe stack (main 2c265f517 + this PR and
  #860, #823, #836, #872, #856, #852, #875, #877, #854) with
  rt-base-build-shim.R: 4 of 10 steps run. tools `all` now stops in
  `lengths()` of an object (E). base `mkdesc` stops in `iconv(sub =
  "byte")`, which is do_iconv's (B; the stand-in refuses `sub`).
