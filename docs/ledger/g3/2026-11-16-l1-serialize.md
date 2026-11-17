## 2026-11-16: L1, serialization and lazy-load databases on robigo-rt

- What: m2m3-plan L1 (docs/g3.md §10, §11): serialize.c on robigo-rt
  (`crates/robigo-rt/src/serialize/`), the part of connections.c it needs
  (`crates/robigo-rt/src/connections.rs`), and what an unserialized
  environment needs of robigo-rtcore (`env/restore.rs`). Base: `main` at
  `d25d0586`.
  - **Formats:** xdr, binary, ascii and asciihex, versions 2 and 3 (the
    native-encoding header; strings converted from the writer's native
    encoding as `ReadChar` does, for UTF-8, Latin-1 and ASCII). `%.16g`
    and `%a` as glibc's printf writes them, `%lg` as strtod reads them.
  - **WriteItem/ReadItem:** the reference table, the persistent-name hooks
    (called at every encounter, as `GetPersistentName` is), package and
    namespace environments (`PACKAGESXP`, `NAMESPACESXP` through
    `findPackageEnv` and `..getNamespace`), the iterative pairlist read,
    `R_ReadItemDepth`'s `lastname`. GNU R's reader's own changes are kept:
    a forced promise's PRENV and a closure's NULL environment become the
    base environment.
  - **Builtins:** `serialize`, `serializeb`, `unserialize`,
    `serializeToConn`, `unserializeFromConn`, `serializeInfoFromConn`,
    `lazyLoadDBinsertValue`, `lazyLoadDBfetch` (with serialize.c's cache of
    whole `.rdb` files under 10 MB), `lazyLoadDBflush`, `getVarsFromFrame`;
    `file`, `gzfile` (and `bzfile`, `xzfile`, `zstdfile` as not
    implemented), `gzcon`, `close`, `summary.connection`; `file.remove` and
    `Sys.getpid` for the tests' files. `R_compress1`/`R_decompress1` are
    zlib streams with GNU R's 4-byte length (the oracle's build uses
    libdeflate: its error numbers are kept).
  - **The walk and the collector.** Writing reads the graph raw. Its few
    points that may collect (the hook, a package environment's warning,
    `FRAME()` of a slot frame, an unboxed promise value) keep their results
    by their place in the walk; if one collected, the walk starts over and
    takes the kept results, so each effect happens once, in GNU R's order.
    Reading roots every object under construction (one scope per level) and
    re-reads it from its root after anything that may collect, so a
    refused allocation is retried with the partial objects alive and none
    is visible to R code before it is complete.
  - **Environments.** GNU R registers an environment before reading its
    contents, and robigo's hashed and unhashed environments differ in
    shape, so the reader looks ahead (the enclosure skipped by a syntactic
    skipper of the stream's grammar, byte code's included) for whether the
    table or the frame follows, makes that shell, and robigo-rtcore's
    `Cx::restore_env` fills it: a hashed table taken as read and recounted
    (`R_RestoreHashCount`), or each cell of FRAME bound oldest first as a
    cell-backed slot, so `FRAME()` lists them as they were and each keeps
    its gp bits (locks, active bindings).
- Not yet, each an error saying so: byte code (BCODESXP and the closures
  and promises GNU R holds as byte code: C1-BC, then this PR or a
  follow-up adds the BCREPDEF/BCREPREF representation, whose grammar the
  skipper already reads), `R_compile_pkgs` (L4 needs it with C1-BC's
  compiler), ALTREP classes (robigo-rt has none of its own; base's
  `compact_intseq` and `compact_realseq` are read as the standard vectors
  they expand to, as robigo-rt's `:` makes them; package classes live in
  robigo-rtapi), bzip2/xz/zstd, URLs, clipboards and fifos, re-encoding
  connections, and the finalizer that closes an unreferenced connection.
- gzip output: zlib-rs (flate2's backend), not zlib, so a `.rds` file's
  compressed bytes may differ from GNU R's while what they decompress to is
  the same; the bars compare decompressed bytes.
- **Bar 1, GNU R's own databases** (`scripts/rt-rdb-roundtrip.sh`,
  `crates/robigo-rt/tests/rdb_roundtrip.rs`, against GNU R's round trip of
  each record in the oracle image; release build of this branch, this
  machine):

  | database | records | as GNU R's round trip | of them their stored bytes | with byte code |
  |---|---|---|---|---|
  | base | 1,189 | 35 | 35 | 1,154 |
  | compiler | 297 | 155 | 155 | 142 |
  | tools | 954 | 101 | 87 | 853 |

  No record differs and none hits another gap. tools' 14 records whose
  round trip is not their stored bytes are environments holding forced
  promises: GNU R's own reader gives their PRENV the base environment, so
  GNU R's re-serialization is 12 bytes longer too, and robigo-rt's is
  GNU R's byte for byte.
- **Bar 2, the generated corpus** (`tests/rt/serialize/corpus.R`,
  `tests/rt/serialize-closures/frames.R`): about 60 values (each atomic
  type plain, empty, scalar, named and classed; long vectors; attributes;
  language objects; closures; builtins and specials; the special,
  hashed, unhashed, cyclic, shared, locked and classed environments), each
  in 4 formats and 2 versions: robigo-rt's bytes are GNU R 4.6.2's
  recording's. **Known gap:** the corpus has no `1:n` (nor `seq_along`,
  `seq_len` or any other result GNU R makes as an ALTREP object): GNU R
  4.6.2 writes `serialize(1:10)` in the `compact_intseq` ALTREP form,
  robigo-rt has a plain INTSXP there and writes that. robigo-rt's own
  ALTREP classes (altclasses.c) are package AC's.
- E9 (`scripts/rt-fail-alloc.py`): `tests/rt/serialize-closures/unserialize-e9.R`
  joins the default scripts; every injection point gives the uninjected
  output.

**Stop-gaps L1 adds** (for the stop-gap inventory of
`2026-11-16-m1-exit-evidence.md`; paths as there, `rt` is
`crates/robigo-rt/src`):

| area | what | where | owner |
|---|---|---|---|
| byte code | unserializing BCODESXP; serializing a closure or promise GNU R holds as byte code | `rt/serialize/read.rs`, `rt/serialize/write.rs` | L1b (after C1-BC) |
| compile on write | `R_compile_pkgs` (WriteItem's `R_cmpfun1`) | `rt/serialize/write.rs` | L1b/L4 |
| ALTREP | serializing a package ALTREP object; unserializing any ALTREP class but base's compact sequences (read as standard vectors); `1:n` written as a plain vector | `rt/serialize/write.rs`, `rt/serialize/read.rs` | AC (own classes), M5 (package classes) |
| writer restart | a hook that changes the object being serialized and collects: the restarted walk writes the changed contents where GNU R wrote the old (documented in `write.rs`) | `rt/serialize/write.rs` | follow-up (resumable walk) |
| encodings | strings serialized in a native encoding other than UTF-8, Latin-1 or ASCII; connections that re-encode | `rt/serialize/read.rs`, `rt/connections.rs` | M4 |
| connections | `bzfile`, `xzfile`, `zstdfile` and files compressed so; URLs; clipboards; fifos; `file("")` and `file("stdin")`; the finalizer that closes an unreferenced connection | `rt/connections.rs` | M4 |
| lazy-load | bzip2 and xz compressed databases | `rt/serialize/mod.rs` | M4 |
| primitives | unserializing a primitive robigo-rt lacks (GNU R warns and gives NULL only for names outside names.c) | `rt/serialize/read.rs` | M4 |
