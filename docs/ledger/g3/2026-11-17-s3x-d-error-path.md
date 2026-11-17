## 2026-11-17: S3x-d, stop()'s builtins: c() and attributes<-

- What: the builtins of trycatch's error branch (the foreman's S3x-d (a);
  (c) is `2026-11-17-s3x-d-native-messages.md`). Port-faithful only, each
  measured on its own. Byte code's VECSUBSET fast path, the other part of
  the gap, touches `vm.rs` and waits for AC1 (#803); it comes in its own
  PR, measured on top of AC1.
- **`c()` and `unlist()`** (bind.c). `items()` collected each list into
  a `Vec` for every walk: the answer type, the fill, the names, the
  counts. It is now an iterator over the list or pairlist in place, as
  GNU R's loops over `VECTOR_ELT` and `CDR` are. `do_c` builds its
  compacted argument list (`R_listCompact`) only when a later argument is
  NULL. The `recursive=` and `use.names=` indices are no longer
  collected. `stop()`'s `structure()` calls `c(attributes(.Data),
  attrib)`, which made 9 heap allocations; it now makes 2.
- **`attributes<-` and `attr<-`** (attrib.c:do_attributesgets,
  do_attrgets, setAttrib; sysutils.c:installTrChar).
  - Each name was copied into a `Vec`, then installed, then matched by
    its bytes against the special names. Now `install_tr_char` looks a
    known symbol up from the string's bytes in place. A new or translated
    name goes through `translateChar` and `install` as before.
  - `set_attrib_sym` tells the special names apart by symbol, as
    `setAttrib` compares `name == R_NamesSymbol` and the rest.
  - `dim` goes in through `R_DimSymbol`.
  - `set_attrib(name bytes)` is `install` then `set_attrib_sym`.
- Not changed: `do_makelist` (594 instructions per call inclusive
  against GNU R's 941), `paste` and `names` (at or under GNU R's).
- Cases, recorded on GNU R 4.6.2 and ratcheted:
  - `tests/rt/bind/c-lists`: lists, pairlists and expression vectors, by
    `c()` and `unlist()`, recursive or not, with names, and NULLs after
    the first;
  - `tests/rt/attrib/attributes-install`: names the session has, new
    ones, UTF-8 ones and a Latin-1 one, through `attributes<-` and
    `attr<-`.
- Measurements: callgrind instructions per iteration
  (`scripts/callgrind-diff.py`, `ROBIGO_COMPILE=1`, 2N against N). The
  WSL2 workstation, Ryzen 9 5900X, rustc 1.100.0. Release `robigo-rt` at
  `3b79c451` (main `4d6efe47` merged in) against main's.
  - trycatch-err: 177,949 to 176,619 per caught error (-0.75%). Measured
    one change at a time: `c()` -775, `attributes<-` -426, the
    byte-string messages -119.
  - trycatch: 115,899 to 115,256 (-0.55%).
  - `scripts/perf.py rt-bars` against main reports 0 failures, with
    every row within ±0.04%.
- Proof: see the PR.
