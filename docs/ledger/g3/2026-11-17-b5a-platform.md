## 2026-11-17: platform.c's file.exists and list.files, sys-unix.c's R_ExpandFileName (B5a)

- What: B5a's platform.c package. It ports `do_fileexists` and
  `do_listfiles` (`crates/robigo-rt/src/platform.rs`) and `ssort`, which
  `list.files` sorts with (`ssort_native` in rtbuiltins' `sort.rs`). It
  also ports sys-unix.c's `R_ExpandFileName_unix` in full, in
  `crates/robigo-rt/src/connections.rs`: `~user` from the password
  database (rtcore `clib`'s `passwd_home`), the fallback when `HOME` is
  unset or empty, and the "expanded path length … would be too long"
  warning.
- Known gaps, each with an owner:
  - **Connection callbacks drop the over-long expansion warning (owner: L,
    connections).** `file()`/`gzfile()`'s open callbacks have no `Cx`, so
    they call `expand_file_name`, which expands as GNU R does but cannot
    give that warning. Every other caller uses `r_expand_file_name`, which
    gives it. **Closed by L4's readers** (`2026-11-18-l4-connection-readers.md`):
    the opens stash the warning and their callers give it; GNU R's
    `R_ExpandFileName` takes readline's expansion first, which that entry
    also ports.
  - **`list.files` with a back reference in `pattern`, in a UTF-8 locale
    (owner: B, package B5-rxbt).** GNU R matches each file name as a
    multibyte string (`tre_regexec`). With back references, that runs
    TRE's backtracking matcher, whose multibyte walk does not restore its
    position when it backs up, and that changes results even on ASCII
    names: on the fixture, `"(.)\\1"` with `all.files`, `recursive` and
    `include.dirs` returns every entry. robigo-text's TRE has not ported
    that path, so `list.files` refuses such a pattern with "robigo: … is
    not implemented yet" instead of answering differently.
    - grep.c's own paths (bytes, wide, and the multibyte matching that
      `strsplit` and `regexec` do for unmarked input) were checked against
      GNU R with back references and agree
      (`tests/rt/regex/backref-utf8.R`, in the util.c package).
