# Ledger: startup (default packages, lazy loading, serialization, connections)

## 2026-10-27 (resumed): finish and verify the WIP; connections round out
- why: the resumed branch had a finished lazy-load/LZMA commit and an
  unverified WIP (raw/seek/binary connections, save/load on connections).
- what: verified the WIP against GNU R and completed it (visibility of
  writeBin/writeChar, rawOneString/readOneString semantics, fifo(), the
  iconv validity check at open, seek/truncate per connection kind);
  `.External` arity check (dotcode.c:563) — shared file, 2-token change;
  list.files/list.dirs ported (sysutils.rs, with a `TreBoolMatch` helper
  in grep.rs — shared file).
- result: fastr serialize/unserialize 316/326, TestConnections 131/132,
  readBin 13/13, listfiles 20/20, TestTypeConvert 31/31 (metrics before
  the resume: 167/326, 65/132, ~10/13, 3/20, 0/31); gnur-examples
  `^datasets/` 9/76 (lazy loading itself works; see open items);
  conformance tests/r and runtime unit tests hold. Kept.

## 2026-10-27: R 4.5.0's lazy-load compression is raw LZMA2, not zlib
- why: `R_decompress3` (connections.c) type 'Z' decodes a raw LZMA2 stream
  (lzma_raw_decoder, LZMA2 filter, preset 6) since R uses liblzma for the
  compact format; robigo tried zlib, so every `compress=3` database (all of
  datasets) failed to load.
- change: port `R_decompress3`/`R_decompress1` byte layout onto the `liblzma`
  crate (same C library R uses); error message becomes R's
  `lazy-load database '<file>' is corrupt`.
- result: `head(mtcars)` prints GNU R's output; kept.

## 2026-10-27: baseline
- fastr `TestBuiltin_(serialize|unserialize)`: 167/326; `TestConnections`: 65/132;
  `readBin|listfiles|TestTypeConvert`: 59/123.
- `head(mtcars)`: "lazy-load database is corrupt" (datasets' lazy-load DB).
- Missing-package warning fires once (already fixed by someone).

## Open items (ranked)

1. **ALTREP serialization** (fastr serialize 7, unserialize 2 cases):
   `serialize(1:10, version=3)` must write compact_intseq/compact_realseq
   class info (`OutInteger(PackFlags(ALTREP_SXP=238,...))` + serialized
   class list3 + the REALSXP(3) state). robigo has no ALTREP values; the
   `:`/`seq` results would need a marker that survives exactly as long as
   R's ALTREP-ness does. Touches value.rs/seq.rs (shared) — needs a design
   decision, likely a hidden compact-seq tag on Int/Real vectors created
   by `:`, `seq()` and friends, dropped by any materializing operation.
2. **Active bindings in serialized environments** (3 fastr serialize
   cases): WriteItem writes FRAME(s) binding cells; an active binding's
   cell carries ACTIVE_BINDING_MASK in the packed flags (`fd 08` vs
   `fd 00`) and unserialize (envir.c:2529) re-makes them via
   R_MakeActiveBinding. robigo's env serialization drops the bit;
   makeActiveBinding itself works. Fix is local to serialize.rs + env
   binding representation.
3. **Double error after a failed iconv open** (fastr TestConnections 1
   case): GNU R prints the `unsupported conversion ...` error and then a
   second `invalid connection` error with the same call during the
   unwind of the first (set_iconv_error destroys the connection before
   erroring). robigo produces only the first. Mechanism unclear without
   a C-level trace; needs a debugger session against GNU R.
4. **`unserialize(file(...))` traceback** (2 fastr cases): GNU R's trace
   is `unserialize -> file`, robigo's `unserialize -> typeof -> file`.
   GNU R omits the `typeof` frame only because base's thin `typeof`
   wrapper is byte-compiled and the BC CALL path does not push a context
   for the closure, while the AST path does (verified with interpreted
   vs compiler::cmpfun callers). A robigo fix would make closure frames
   lazy for VM-compiled callers — perf-vm territory (vm.rs, interp.rs).
5. **gnur-examples `^datasets/` 9/76**: lazy loading works; the failures
   are stats/graphics native routines (`numeric_deriv`, `TSconv`,
   `optim`, `C_par`) — G2 (GNU R's shared libraries), do-not-port.
   Same for `^grid/` (grid.so `initGrid`) and methods (G6).
