## 2026-11-17: L2a, base evaluated from source, namespaces, the search path, parse()

- What: the first part of m2m3-plan L2 (split as L2a, foreman 2026-11-17;
  the rest waits on B5a and C1-BC). Base: `main` at `a9c4fda3` (BW merged).
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
- **Base's bindings and the S3 memo are BW's** (#767, foreman
  2026-11-17). Before BW, this package had its own fetch promise and its
  own memo fix, because its traced bound grew from 220 to 343 closures and
  raised listgrow's peak RSS by 3.2%. The fetch promise named a
  definition's byte span in the pinned source file. The memo read a forced
  `.__S3MethodsTable__.` promise as its value and held the defining
  environment's version box. BW binds all of base lazily from an index
  with the same spans (`proto/base-wide.txt`), and fixes the memo the same
  way. Both of this package's versions were dropped for BW's on the merge.
  `tests/rt/s3/method-table-defenv.R` (once `method-table.R`, a name BW
  also took) passes on BW's loader and stays. It covers `length`'s
  internal dispatch through base's table, a global environment's own
  table, and a method found after the table that a non-function in the
  table hides.
- **Stage 0 on this tree** (`ROBIGO_BOOT_VERBOSE=1`): 1,308 top-level
  expressions, 36 fail, each on something robigo-rt does not have yet:
  - base: `.Internal(strptime)` (`.leap.seconds`),
    `.Internal(as.function.default)` (and the 19 `.GenericArgsEnv`
    definitions after it), `.Internal(body)`;
  - the profile: `.Internal(Version)` (and the lines that read
    `R.version`), `file.path`, `dir.exists`, `Sys.glob`, `Sys.getlocale`
    (B5a), `options(keep.source, scipen, max.print)`.
  - Every name robigo-rt then binds in base is one of GNU R's
    `ls(baseenv(), all.names = TRUE)`: 1,319 of 1,407. The 88 it lacks
    are those builtins, the profile's names and the failures'.
- `loadNamespace` from source needs no `Meta/` files: without
  `nsInfo.rds` it parses the NAMESPACE file, as GNU R's own build
  bootstraps tools (a missing `package.rds` only warns). It waits on
  B5a's file builtins.
- Known divergence: `R_IsNamespaceEnv` reads `.__NAMESPACE__.` and its
  `spec` with `R_findVarInFrame`, which calls an active binding's
  function. robigo-rt's namespace predicates (`ns.rs`, as `dispatch.rs`'s
  already did) run no R code: the serializer's walk, the compiler and S3
  dispatch call them where none may run. So an environment whose
  `.__NAMESPACE__.` is an active binding is not a namespace here. Neither
  GNU R nor any package makes one. A later package can make them read as
  GNU R does.
- Not yet, each an error saying so: source references
  (`keep.source = TRUE`), the parse errors GNU R raises as `parseError`
  conditions, parsing from the console, `parse(n = )` on an open
  connection that is not a file, attaching a user-defined database.
