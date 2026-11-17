## 2026-11-17: L1b, byte code in serialize, and L1's deferred items

- What: the follow-up to L1 (#752) that waited on C1-BC (#771). Base:
  `main` at `78dcf1ee`.
  - **Byte code** (serialize.c):
    - `WriteBC`: `ScanForCircles` finds the language and pairlist cells
      reached more than once. `WriteBC1` writes the code and then the
      constants. `WriteBCLang` writes a cell as `BCREPDEF` the first time
      and `BCREPREF` after, and a cell with attributes as `ATTRLANGSXP` or
      `ATTRLISTSXP`.
    - `ReadBC`, `ReadBC1`, `ReadBCConsts`, `ReadBCLang`: the code goes
      through `R_bcEncode` before the constants are read, and a
      `BCREPDEF` cell enters `reps` before its contents. A bad index gets
      `VECTOR_ELT`'s and `SET_VECTOR_ELT`'s messages. Byte code this R
      cannot run reads back as its expression (`R_BCVersionOK`,
      `R_BytecodeExpr`).
  - **robigo-rt keeps code decoded** (C1-BC), so `R_bcDecode` is a copy.
    The writer emits the code as an integer vector without allocating
    one, so the walk has no new effect point. `ScanForCircles` walks an
    explicit stack: the cells it finds do not depend on the order of the
    walk.
  - **Closures:** one whose body is a BCODESXP (from `bcClose` or
    `unserialize`) is written as it is. One that robigo-rt compiles
    itself where GNU R holds byte code is still refused, and so is such a
    promise, because there is no BCODESXP of GNU R's to write.
  - C1-BC's `bytecode::bc_encode` is `R_bcEncode` on its own, for the
    reader.
- **The bar** (`scripts/rt-rdb-roundtrip.sh`, oracle 4.6.2): every record of
  `base.rdb`, `compiler.rdb` and `tools.rdb` reads, byte code included.
  - 2,409 records serialize back to GNU R's round trip, 2,395 of them
    to their stored bytes: base 1,180 (all 1,180), compiler 297 (all
    297), tools 932 (918).
  - None differ and none fail.
  - 31 records (base 9, tools 22) hold one of base's compact sequences
    among byte code's constants. robigo-rt reads it as the standard
    vector, so it writes it back expanded. That gap is AC's (robigo-rt's
    ALTREP classes) and is counted apart, typed: the reader counts what
    it expanded (`serialize::take_compact_sequences_expanded`).
- **L1's deferred items:**
  - `rdb_roundtrip.rs`:
    - context on every panic;
    - the `.rdx` offsets checked against the `.rdb`;
    - `keys` renamed `rdx_records`, no longer shadowed;
    - the gap typed instead of read from a message;
    - one package list;
    - differing records listed first and never truncated.
  - `rt-rdb-roundtrip.sh` writes each `.roundtrip` beside itself and
    renames it into place.
  - `restore.rs`:
    - the `expect`s' invariants stated;
    - the table's shapes rejected with distinct messages;
    - chains checked without a `Vec` each;
    - `restore_frame`'s precondition, a new shell, checked in
      `restore_env`;
    - a test for each rejection branch. A table that is an ALTREP list
      has none: robigo-rtcore makes none to test with.
- Proof:
  - `tests/rt/serialize/bytecode.R` is recorded on 4.6.2 and
    byte-identical in xdr, binary and ascii, versions 2 and 3. It covers
    shared calls, attributes, a pairlist, nested code, a call that holds
    itself, a closure over byte code and an old version.
  - E9 under `ROBIGO_GC_MOVE=1` at every allocation point is clean: 10,016
    points of that case, and 11,442 of
    `tests/rt/serialize-closures/unserialize-e9.R`, which now holds byte
    code too.
