## 2026-11-17: A5b, tools' native code built from GNU R's sources and loaded by robigo-rt

- Why: L4's base build runs the tools database step, which loads tools.so
  and calls its Rd parser through `.External2`. A5a (#815) gave robigo-rt
  routine registration and the foreign calls. This entry is the A5 note's
  second half (`~/.cache/robigo-research/a5-dynload-note.md`), with the
  foreman's decisions C(1), H and B and his three conditions.
- **Headers (H):** robigo's own headers for GNU R's base-package natives,
  in `crates/robigo-rtapi/internal`. These are not package headers.
  - Files: `Defn.h`, `Parse.h`, `Rconnections.h`, `RBufferUtils.h`, and
    `config.h`, which holds the `HAVE_*` that tools tests (no
    `ENABLE_NLS`).
  - Each declaration has GNU R 4.6.2's name, type and size, with its
    `Defn.h` line.
  - `Defn.h` includes R.h and robigo's Rinternals.h, then robigo-rtcore's
    `Defn.h` (the contexts and gp masks) with `#include_next`.
  - The natives' include path: the internal headers, the C API's,
    robigo-rtcore's, `robigo_layout.h`, the tarball's `src/include`, then
    the tree's `include/` (`Rmath.h`). All 13 of tools' C files compile
    against it, and nothing comes from an installed R.
- **Per-isolate C globals** (`robigo_capi.h`, `capi.rs`, `capi.c`'s
  layout table):
  - the symbols `R_SrcfileSymbol` and `R_SrcrefSymbol`;
  - the parser's `R_ParseError`, `R_ParseContextLast`,
    `R_ParseContextLine`, `R_ParseErrorMsg[256]` and
    `R_ParseContext[256]`;
  - `mbcslocale` and `R_MB_CUR_MAX`, set from the isolate's locale when it
    is made and by `Sys.setlocale`.

  `Defn.h` makes them macros over `robigo_cg`. The 256-byte sizes are
  asserted in C and Rust (condition 1).
- **One exception to L6:** tools' `text.c` declares `mbcslocale` and
  `R_MB_CUR_MAX` itself (`LibExtern`), without `Defn.h`. So they are also
  process data (robigo-rtcore's `capi.rs`), set with the block's: the
  locale of the isolate that set one last.
- **Exports** (each a port of GNU R's function):
  - api.c:
    - `R_chk_calloc`, `R_chk_realloc`, `R_chk_free`;
    - `Rf_isNull`, `Rf_isLogical`, `Rf_isEnvironment`, `Rf_isString`,
      `Rf_imax2`;
    - `Rf_strchr` and `Rf_strchr_const`: strchr in robigo's UTF-8 and C
      locales;
    - platform.c's dirent wrappers;
    - memory.c's precious multi-sets, composed of the API as GNU R
      composes them.
  - sysutils.rs: `Rf_translateChar`, `Rf_translateCharUTF8`,
    `Rf_getCharCE`, `Rf_installTrChar`, `Rf_mbrtowc` (util.c's
    `Mbrtowc`, with its `<xx>` error), `R_FileExists`, `RC_fopen`.
  - eval.rs: `Rf_coerceVector`, `Rf_asBool`, `Rf_findVar`,
    `R_NewHashedEnv`, `Rf_isInteger`.
  - unwind.rs: `Rf_begincontext` (a leaf) and `Rf_endcontext`, over
    robigo-rtcore's `begin_foreign`/`end_foreign` for records in C
    frames. An exit runs a C record's `cend` in its leg, which is how
    parseRd's connection is closed on error.
  - gram.rs: `parseError`, source.c's, with `getParseContext` and
    `tabExpand`. robigo's parser records no `R_ParseErrorCol` or
    `R_ParseErrorFile` yet: column 0 and no file name, GNU R's values
    before any R-level parse error.
  - internet.rs: `extR_HTTPDCreate` and `extR_HTTPDStop`, the help
    server. They raise "not implemented yet" (condition 2).
  - The connections, below.

  With these, every symbol of tools.so's dynamic imports resolves against
  robigo-rt.
- **Connections (C(1)):** robigo-rt's `Conn` holds GNU R's public
  `struct Rconn` (`R_ext/Connections.h` version 1, robigo-rt's
  `rconn.rs`) as its own state.
  - What lives there: the flags, `mode`, `encname`, `status`, `save` and
    `save2`, the buffer fields, `class`, `description`, `id`, and
    `private` (the connection).
  - The layout is 488 bytes, measured against GNU R 4.6.2's header and
    asserted field by field in Rust and in robigo's `Rconnections.h`
    (condition 1).
  - The method pointers are robigo-rtapi's entries, installed in each
    isolate: open, close, `fgetc` (`dummy_fgetc`), `fgetc_internal`,
    read, write and fflush call robigo-rt. Destroy, vfprintf, seek and
    truncate raise "not implemented yet".
  - `getConnection`, `R_GetConnection` and `Rconn_fgetc` are exports
    (`Rconn_fgetc` is GNU R's `save2`/`save`/CR logic). C's pointer is
    found by its address in the table.
  - A text connection read from a regular file is buffered, as
    `set_buffer` does (4,096 bytes, freed at close). robigo's other readers
    take the buffered bytes first, so a mixed reader stays in step.
- **tools.so (B):** `scripts/rt-home.sh` builds
  `library/tools/libs/tools.so` from the tarball's `tools/src`, with
  tools' `Makefile.in` flags (`-DHAVE_CONFIG_H`, `C_VISIBILITY`).
  - It finds `robigo_layout.h` from robigo-heap's build-script `out_dir`
    (cargo's JSON).
  - The tree's stamp also hashes the headers and the compiler.
  - The rt job runs the script before its conformance steps
    (`scripts/rt-ci.py`, with a test).
  - `include/Rmath.h` is made as GNU R's `src/include/Makefile` makes
    it. The tarball has only `Rmath.h0.in`, and config.status fills its
    `@PACKAGE_VERSION@` and `@RMATH_HAVE_WORKING_LOG1P@`. robigo's result
    is byte-identical to the oracle image's installed `Rmath.h`.
    `gramRd.c` includes it. #828's first CI run failed without it: the
    rt container's R keeps its headers off the compiler's path, while a
    workstation's `/usr/include/Rmath.h` had hidden the gap. In the CI
    image, with every installed R file deleted, the script builds the tree.
    tools' objects then use only the checkout's, the tarball's and the
    tree's headers, and libc's.
  - **Narrower than the note:** `$R_HOME/include` holds only `Rmath.h`.
    The other natives are built against the checkout's headers, and
    installing them for packages (`R_INCLUDE_DIR`) waits for L4's
    `R CMD INSTALL`.
- **Oracle cases, recorded on GNU R 4.6.2:**
  - `tests/rt/dynload/tools-parse-rd.R`:
    - dyn.loads the tree's tools.so;
    - prints the DLLInfo, the registered routines (1/19/0/2) and
      `parseRd`'s symbol;
    - parses an Rd file through `file()` with `.External2`;
    - checks the `forceSymbols` error for a name.

    It passes and is ratcheted.
  - `tests/rt/capi/conn-state.R` (condition 3), with
    `tests/rt/capi/support/conn_view.c`: C reads the connection's flags,
    mode, class and description through `R_GetConnection`, then clears
    `isopen`, and `summary(con)$opened` is "closed". It passes and is
    ratcheted.
  - `tests/rt/dynload/library-dynam-tools.R`: `library.dynam("tools",
    "tools", .Library)`. It waits for `path.expand` (B5b): it is recorded
    and not ratcheted, and a one-line follow-up adds its marker.
  - The tools cases require the new conformance feature `r-home-tools`.
    robigo-rtapi's conformance test turns it on when the binary's `R_HOME`
    has tools.so.
- Also: #815's deferred OCR low (r4603919725). `.NAME`'s check now
  returns the name it validated (`SymId`), so `resolveNativeRoutine` no
  longer unwraps argument 0's type again.
