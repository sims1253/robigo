# Ledger: startup (default packages, lazy loading, serialization, connections)

## 2026-10-27: baseline
- fastr `TestBuiltin_(serialize|unserialize)`: 167/326; `TestConnections`: 65/132;
  `readBin|listfiles|TestTypeConvert`: 59/123.
- `head(mtcars)`: "lazy-load database is corrupt" (datasets' lazy-load DB).
- Missing-package warning fires once (already fixed by someone).

## 2026-10-27: R 4.5.0's lazy-load compression is raw LZMA2, not zlib
- why: `R_decompress3` (connections.c) type 'Z' decodes a raw LZMA2 stream
  (lzma_raw_decoder, LZMA2 filter, preset 6) since R uses liblzma for the
  compact format; robigo tried zlib, so every `compress=3` database (all of
  datasets) failed to load.
- change: port `R_decompress3`/`R_decompress1` byte layout onto the `liblzma`
  crate (same C library R uses); error message becomes R's
  `lazy-load database '<file>' is corrupt`.
- result: `head(mtcars)` prints GNU R's output; kept.
