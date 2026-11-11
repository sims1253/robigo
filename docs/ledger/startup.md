# Ledger: startup (default packages, lazy loading, serialization, connections)

## 2026-11-10: corrupt serialized counts end in an R error, not a failed allocation (#499)
- why: the fuzz harnesses (`fuzz/fuzz_targets/rds.rs`, `scripts/fuzz.py rds`)
  report corrupt streams whose declared counts make the reader request
  terabyte-scale allocations: cargo-fuzz (ASan) flags
  `try_reserve_exact<Value>` of 6.7 GB from a 257-byte input, and
  `Reader::refill`'s `vec![0u8; need]` aborts the process outright on a
  connection whose item length is corrupt (Rust's allocation-failure
  abort, no R error). The contract (fuzz.py docstring, GNU R itself) is
  an R error at worst.
- what: `Reader::check_count` bounds every stream-declared count by the
  bytes the input can still contain before any allocation (buffer
  streams only; connections can deliver more). Per-element minima are
  sound lower bounds (binary: exact element sizes; Ascii: one
  whitespace-terminated word), verified by a round-trip unit test — the
  first draft's 12-byte-per-string bound wrongly rejected NA strings
  (8 bytes: flags + length -1) and the test caught it. `refill` reserves
  its chunk and answers a failed reservation with the read error
  (InBytesConn). GNU R's message for both regimes is "read error"
  (verified against the local 4.5.0 oracle for a count that would
  allocate and one that would not). Found en route: `compact_intseq`
  panicked under overflow checks on a corrupt ALTREP state
  (`n1 + k*inc`); now wraps like the C it ports. Shared file value.rs,
  1-line change, called out.
- result: the saved ASan artifact now ends in an R error; a 249 s
  cargo-fuzz session (fork 4, same seed as the pre-fix OOM run) reports
  `oom/timeout/crash: 0/0/0` over 685k execs; `scripts/fuzz.py rds`
  clean (45k runs pre-fix already, the release binary never aborts on
  the buffer paths). Runtime unit tests hold (only the two documented
  glibc-oracle last-bit failures, reproduced on untouched main),
  round-trips in every format hold. Kept.

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

## 2026-11-11: accepted cost — corrupt-stream guards, ~+0.5–0.7% on s4 (Max approved)
- The corrupt-stream hardening (bounds on stream-declared counts, the
  repetition-slot validation, fallible connection reservations) costs
  ~+0.5–0.7% instruction count on the s4 benchmarks, which are
  dominated by namespace loading (callgrind: methods-load +0.47–0.52%
  vs main; CI slot-read +0.67%). The guards are irreducible
  per-definition/per-vector validation — the PR's whole purpose — and
  the 0.5% perf gate fails marginally on the s4 subset (all other
  benchmarks stay under).
- Max approved accepting and documenting the deviation (question-tool
  answer, "Accept + document"). The remaining ~+0.2% CI-vs-local
  offset comes from CI's debian-linked libc/mimalloc builds.
- Future startup-track work can offset it: `Reader::int` is 8.1M
  instructions (5%) on the same load.
