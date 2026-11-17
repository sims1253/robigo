## 2026-11-17: L2a, base evaluated from source, namespaces, the search path, parse()

- What: the first part of m2m3-plan L2 (split as L2a, foreman 2026-11-17;
  the rest waits on B5a and C1-BC). Base: `main` at `6bfbf91f`.
  - **Stage 0** (`ROBIGO_RT_BOOT=stage0`, beside M1's loader until L5):
    main.c's `R_ReplFile` of all of base's source (`library/base/R/base`)
    into the base namespace, then the system profile
    (`library/base/R/Rprofile`, `R_LoadProfile`), from the R_HOME tree
    `scripts/rt-home.sh` makes from the pinned tarball (F7b option A: what
    GNU R's build makes before the databases exist; L4 adds them). The
    shared PARSE_OK body of `R_ReplFile` and `Rf_ReplIteration` is
    `base::repl_eval_toplevel`, which Rscript's console loop now uses too
    (A1's seam for the `-f FILE` echo front end).
  - **Namespaces** (envir.c): the registry entries, `isNamespaceEnv`,
    `R_getNSValue` in full (`::`, `:::`, `getNamespaceValue`: exports,
    lazydata, base's `loadNamespace` and `getNamespaceName` called as GNU R
    calls them), `importIntoEnv`, `topenv`, `environmentName`,
    `internalsID`.
  - **The search path**: `attach` (a list, an environment, NULL; GNU R's
    hash layout through rtcore's new `Cx::hash_frame`, `R_HashFrame` then
    the resize loop), `detach`, `search`, `mkUnbound`.
  - **parse()** (source.c) from text and from connections (`R_ParseConn`;
    with `n`, the rest of a file connection left unread), `stdin()`,
    `writeLines`; the `expression()` special, `.isMethodsDispatchOn`,
    `substitute(expr, list)`.
- **M1's loader fetches a definition when it is first used**, as GNU R's
  lazy load does. A binding's promise was the lowered right-hand side.
  It is now the call `<fetch>(file, c(start, end))`, which names the
  right-hand side's bytes in the pinned source file, as `makeLazy`'s
  `lazyLoadDBfetch(key, file, ...)` names a value's offset and length.
  Forcing the promise parses and lowers that text. The bound grew from
  205 to 338 closures, through the namespace closures that `R_getNSValue`
  and `getNamespaceName` reach (`asNamespace`, then statically
  `loadNamespace`). Before the change, the bound's code took 450 KB of
  heap that never ran, which raised listgrow's peak RSS by 3.2% through
  heap growth. After it, listgrow's live LANGSXPs fall from 10,199 to 414
  and its peak RSS is below main's.
- **The S3 table's promise**: base's `.__S3MethodsTable__.` is now a
  lazy binding, as GNU R's is. Once it is forced, the dispatch memo
  looks through the promise to its value (`dispatch.rs`,
  `forced_value`). Without that, every lookup missed the memo and s3 ran
  30% more instructions.
  - The table is GNU R's, so each memo hit now checks the method's
    absence from it, as `R_LookupMethod` reads it. The check covers any
    binding, a non-function too, so the table's version box cannot
    replace it (`tests/rt/s3/method-table.R`).
  - What pays for that check: a binding of the table that is a promise
    is function-like, so the defining environment's version box moves
    when the binding changes. The memo then holds that box (`def_box`)
    and does not read the binding again.
  - s3 is +0.33% in instructions against main.
- **Stage 0 on this tree** (`ROBIGO_BOOT_VERBOSE=1`): 1,308 top-level
  expressions, 39 fail, each on a builtin robigo-rt does not have yet:
  base's `atan` (`pi`), `.Internal(strptime)` (`.leap.seconds`),
  `.Internal(as.function.default)` (and the 19 `.GenericArgsEnv`
  definitions after it), `.Internal(body)`; the profile's `R.Version`,
  `file.path`, `dir.exists`, `Sys.getlocale` (B5a), `sub`, `strsplit` (RX),
  `options(keep.source, scipen, max.print)`. Every name robigo-rt then
  binds in base is one of GNU R's `ls(baseenv(), all.names = TRUE)`
  (1,317 of 1,407); the 90 it lacks are those builtins, the profile's
  names and the failures'.
- `loadNamespace` from source needs no `Meta/` files: without
  `nsInfo.rds` it parses the NAMESPACE file, as GNU R's own build
  bootstraps tools (a missing `package.rds` only warns). It waits on
  B5a's file builtins.
- Not yet, each an error saying so: source references
  (`keep.source = TRUE`), the parse errors GNU R raises as `parseError`
  conditions, parsing from the console, `parse(n = )` on an open
  connection that is not a file, attaching a user-defined database.
