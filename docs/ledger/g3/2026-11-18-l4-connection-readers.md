## 2026-11-18: connections.c's text readers, the pushback, R_ExpandFileName's readline variant (L4, part a)

- What: the readers GNU R's package build reaches first
  (`crates/robigo-rt/src/connections/readers.rs`): `readLines`, `isOpen`,
  `isIncomplete`, `pushBack`, `pushBackLength` and `clearPushBack`, all
  reading through `conn_fgetc` (connections.c's `Rconn_fgetc` in full:
  `save2`, the pushback, the CR/CRLF mapping). C's `Rconn_fgetc` entry
  calls the same function, so a package reading a connection sees the
  pushback too.
- The pushback is GNU R's layout for C: the lines are `CString`s owned by
  the connection, and `con->PushBack` points at a table of their
  addresses, rebuilt on each change. C may lower `nPushBack`; robigo then
  drops the lines above it. Lines are freed where `con_close1` frees them
  (`close()`), not by `readLines`' own close.
- `Rconn_seek`/`buff_seek`/`file_seek`, for `readLines`' re-seek of an open
  non-blocking seekable connection (a file; the other classes refuse).
- `set_iconv`: an open drops `UTF8out` for a native or binary connection,
  as GNU R's does (`readLines` on a `gzcon` it opens keeps it, so its
  strings are marked UTF-8 as in GNU R). A text connection with another
  `encoding` now refuses its reads and writes ("robigo: re-encoding a
  connection is not implemented yet") instead of passing bytes through.
- **R_ExpandFileName follows readline.** GNU R's `UsingReadline` is TRUE
  unless `--no-readline`, in Rscript and `R -f` too, so the oracle expands
  with readline's `tilde_expand_word` first (sys-std.c), cuts any name of
  `R_PATH_MAX` bytes or more to 4095 bytes with the warning (with or
  without a tilde), and takes `HOME` even when empty. B5a's
  `R_ExpandFileName_unix` port stays as the fallback GNU R keeps. robigo-rt
  takes no `--no-readline`, so it always behaves as the default.
- The opens (`file_open`, `gzfile_open`) stash the expansion's warnings;
  `checked_open`, `open_with_mode`, the C `open` entry, `gzcon`'s reopen and
  `open` give them first, in GNU R's order. `checked_open` destroys the
  connection when any of them becomes an error (PR#18491's context).
- Cases: `tests/rt/connections/{readlines,pushback,isopen,expand-warning}.R`,
  recorded on 4.6.2, with byte-exact inputs in `tests/rt/connections/data/`
  (no `writeBin` in robigo-rt yet).
- Known gaps: `scan` and `readDCF` are part b. `readLines` of `stdin()`
  refuses, as every read of the standard connections does.
