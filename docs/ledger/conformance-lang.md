# conformance-lang

## 2026-10-28: environment validation, dots substitution, and Ops NextMethod

- Why: replacement errors used internal calls; removal ignored environment locks; dots expansion lost pairlist semantics; `NextMethod` collapsed binary `.Method` vectors.
- Ports: `builtin.c:do_envirgets`; `envir.c:do_remove`, `RemoveVariable`, `pos2env`; `eval.c:do_eval` numeric-environment validation; `coerce.c:substitute`, `substituteList`, `do_substitute`; `objects.c:do_nextmethod` metadata update.
- Result: 1,308/1,371 -> 1,323/1,371, 15 additional passes; zero regressions. Kept. Overall gain from the starting binary is 97 cases; 98 corpus ratchet entries include the pre-existing unrecorded pass.
- Runtime edits remain in `builtins/envir.rs` and `builtins/lang.rs`. No shared `interp.rs` change is needed. Environment replacement takes ownership of arguments and mutates closures through `Rc::make_mut`.
- Verification: all three new scripts match GNU R 4.5.0 byte for byte; final filtered FastR rerun is 1,323/1,371 with zero regressions; the relevant help-page slice remains 5/8 with zero regressions. Runtime tests pass (17 unit, two integration); full conformance is 166/167 scripts, with the sole mismatch reproduced byte for byte by the starting binary. The compiled/interpreted comparison and VM guard checks pass. `cargo fmt --check` passes.
- Regression scripts: `environment-validation.R` and `substitute-dots.R`, recorded with `scripts/regen-expected.sh` inside `robigo-oracle:4.5.0`.

### Validation limits

- `tests/r/datetime/zones.R` has one stdout difference against the recorded oracle on this host. Both the saved starting binary and final binary emit identical stdout (589,363 bytes), stderr, and exit status. This is a pre-existing local mismatch, not a regression introduced by this track. No expected output or existing passing entry was changed.
- The host binary cannot run in the Debian oracle container because it requires `GLIBC_2.44`; the container provides an older glibc. CI builds its own binary in Debian trixie and remains the judge for the full corpus and this platform difference.
- Latest `main` advanced through the numerics-policy merge; `git merge-tree --write-tree origin/main HEAD` reports a clean merge, so no rebase is required.

### Remaining work

48 cases remain in the FastR filter:

- Class attributes and printing: 24. Preserve attributes on class vectors in `attrib.rs:class_gets` and `builtins/attrib.rs:do_class`; check pairlist attribute printing. These paths need coordination with their owners.
- Evaluator and argument diagnostics: nine. Fix the lexical call for missing values, promise errors through `eval` and `UseMethod`, empty numeric dots, and the `alist()` rendering of unused missing arguments. Port `eval.c`/`envir.c` lookup and `match.c:matchArgs_NR` in a separate evaluator batch. Include the remaining `switch` default missing-argument case.
- Parser diagnostics: four. Zero-length argument tags, repeated formals, and the unexpected-input context belong to the parser/CLI owners.
- Other tracks: eleven. S4 state (one), `.Device` startup (one), `getconst` (two), `gettext(NULL, NULL)` (one), system timeouts (two), subset/array validation (two), and the locked-environment assignment message from `Interp::define` (one). Function bytecode printing contributes one more case in this group.

The help-page failures remain `base/environment` (hash-profile example), `base/eval` (negative numeric frame selection), and `base/get` (missing-argument condition classes). Next steps: port numeric `eval` environment lookup against `R_GlobalContext` and preserve `missingArgError`/`getMissingError` classes through the condition machinery.

## 2026-10-28: lookup validation, modes, and numeric environments

- Why: lookup treated integer and double as distinct modes, accepted invalid modes, interpreted numeric environments as search-path positions, and reported internal calls in errors.
- Port: `envir.c:do_get`, `str2mode`, `findVar1mode`; `util.c:TypeTable`; `context.c:R_sysframe` through the existing frame walker.
- Result: 1,226/1,371 -> 1,308/1,371, an increase of 82; zero regressions. Kept. The 83 additions to `passing.txt` include the one pre-existing unrecorded pass.
- Verification: runtime crate tests pass (17 unit, two integration); focused lookup test matches Docker GNU R 4.5.0 byte for byte; relevant help-page examples remain 5/8 with zero regressions; `cargo fmt --check` passes.
- Tests: `tests/r/lang/lookup-modes.R` covers mode normalization, invalid names and modes, frame lookup, and existence checks that must not force promises. Expected output was recorded by `scripts/regen-expected.sh` in `robigo-oracle:4.5.0`.
- Remaining lookup failure: `.Device` is absent, which belongs to package/device startup.

## 2026-10-28: filtered baseline and failure groups

- Metric: 1,226/1,371 FastR cases pass; 145 fail; zero ratchet regressions.
- Environment: `R_HOME=$HOME/.local/opt/R-4.5.0`, release binary built from the starting branch.
- Command: `scripts/corpus.py run fastr --filter 'TestBuiltin_(do.call|match.arg|sys|environment|eval|quote|substitute|switch|tryCatch|stop|warning|UseMethod|get|exists|class)|functions/|TestEnvironments' --keep-output --jobs 12`.
- One existing unrecorded pass is present before changes. Count it separately from gains caused by this track.
- Grouping below uses the first observed cause. Some cases have more than one mismatch.

### Lookup numeric environments and error context

42 cases:

- `builtins/TestBuiltin_get/003a55f03ca9b364`
- `builtins/TestBuiltin_get/047198c115b716d0`
- `builtins/TestBuiltin_get/0ad098f2b814e118`
- `builtins/TestBuiltin_get/14c245d31cd76cba`
- `builtins/TestBuiltin_get/1b3824e338e6f5a2`
- `builtins/TestBuiltin_get/224a935d45d36be9`
- `builtins/TestBuiltin_get/25016f8375542379`
- `builtins/TestBuiltin_get/2c1117a9855db6e5`
- `builtins/TestBuiltin_get/323fb129fd3b0994`
- `builtins/TestBuiltin_get/3454c9209986e24f`
- `builtins/TestBuiltin_get/3c1fe18f7acedd8e`
- `builtins/TestBuiltin_get/413c0903ac6056ca`
- `builtins/TestBuiltin_get/46367365b220e4d0`
- `builtins/TestBuiltin_get/4a79e4bf493f1169`
- `builtins/TestBuiltin_get/5a85e1f7e04e23d1`
- `builtins/TestBuiltin_get/6052819c2a73fc51`
- `builtins/TestBuiltin_get/68b202218ea102b5`
- `builtins/TestBuiltin_get/7224c04a6691d66a`
- `builtins/TestBuiltin_get/7a4d0420c3608874`
- `builtins/TestBuiltin_get/9571c723f8fcf753`
- `builtins/TestBuiltin_get/a248720942252d66`
- `builtins/TestBuiltin_get/a968bd124d8e7dd5`
- `builtins/TestBuiltin_get/ad3ab84cda121581`
- `builtins/TestBuiltin_get/b769e1f74b1b6cba`
- `builtins/TestBuiltin_get/c7d2a8ecfb946453`
- `builtins/TestBuiltin_get/ca03151c1ed9256a`
- `builtins/TestBuiltin_get/ce7776053354e81a`
- `builtins/TestBuiltin_get/d0a855acbf67ddd6`
- `builtins/TestBuiltin_get/d6f3e15a451301c4`
- `builtins/TestBuiltin_get/deffeabda4b3d41c`
- `builtins/TestBuiltin_get/e63ac9cd2f6042be`
- `builtins/TestBuiltin_sysframe/292065ea8bc565a7`
- `builtins/TestBuiltin_sysframe/4a16de8e3465e114`
- `builtins/TestBuiltin_sysframe/52927f6bfffe2a4c`
- `builtins/TestBuiltin_sysframe/6e2bc46ee1d5705e`
- `builtins/TestBuiltin_sysframe/8c0b9b051ecccc2b`
- `builtins/TestBuiltin_sysframe/a2f1e21080de8a42`
- `builtins/TestBuiltin_sysframe/bdfd01755e9c942f`
- `builtins/TestBuiltin_sysframe/c302c7ca5aa77128`
- `builtins/TestBuiltin_sysframe/db495cc2b51654b2`
- `builtins/TestBuiltin_sysframe/efadca3df32fca17`
- `builtins/TestBuiltin_sysframe/f259ad494743f7e3`

### Lookup mode matching, validation, and error context

25 cases:

- `builtins/TestBuiltin_exists/418f8055595a2379`
- `builtins/TestBuiltin_exists/47317e8c962bf115`
- `builtins/TestBuiltin_exists/4fe9a5a38bc20fba`
- `builtins/TestBuiltin_exists/57883aad1932e5f2`
- `builtins/TestBuiltin_exists/5d268081b13641d7`
- `builtins/TestBuiltin_exists/6853817fd3460ece`
- `builtins/TestBuiltin_exists/7ee3d2034c05d83a`
- `builtins/TestBuiltin_exists/82e47453c3afb2c0`
- `builtins/TestBuiltin_exists/8585917caf152c15`
- `builtins/TestBuiltin_exists/8e09920c8dd4ecf8`
- `builtins/TestBuiltin_exists/8f2d56444716cf57`
- `builtins/TestBuiltin_exists/95e1c991f0f054c8`
- `builtins/TestBuiltin_exists/9e2fab50bc1049e1`
- `builtins/TestBuiltin_exists/a7b96f957156b94f`
- `builtins/TestBuiltin_exists/b558b12ab7c4175b`
- `builtins/TestBuiltin_exists/c172ea1288e81ec4`
- `builtins/TestBuiltin_exists/cec50140b820e09b`
- `builtins/TestBuiltin_exists/d6f0b5c8fe4bb9e0`
- `builtins/TestBuiltin_exists/d7264f04b70fb9ef`
- `builtins/TestBuiltin_exists/ef39d65d908bdbac`
- `builtins/TestBuiltin_exists/f679d8718136f0d8`
- `builtins/TestBuiltin_get/7c2792a75b0b4604`
- `builtins/TestBuiltin_get/b752295f9ab75494`
- `builtins/TestBuiltin_get/de0b90da52c63d4d`
- `builtins/TestBuiltin_get/e5709e2d2c2fba55`

### Class attribute preservation and printing (outside owned builtin paths)

24 cases:

- `builtins/TestBuiltin_class/0eeb2da26bc55299`
- `builtins/TestBuiltin_class/2942914bb269562f`
- `builtins/TestBuiltin_class/2cbb920e22481b63`
- `builtins/TestBuiltin_class/31327d06aae24e1c`
- `builtins/TestBuiltin_class/405b61cdae7c0af2`
- `builtins/TestBuiltin_class/79379db58874ea30`
- `builtins/TestBuiltin_class/7c10e0c605ae6769`
- `builtins/TestBuiltin_class/9a304db5d890c609`
- `builtins/TestBuiltin_class/aafd9efb5307db8b`
- `builtins/TestBuiltin_class/d00ac2ea9320b9da`
- `builtins/TestBuiltin_class/d46eaa843be91c04`
- `builtins/TestBuiltin_class/e209e5e4a920bfe9`
- `builtins/TestBuiltin_classassign/2afc04aaab309166`
- `builtins/TestBuiltin_classassign/49667e4a1c1a73a8`
- `builtins/TestBuiltin_classassign/b2b4215a7276f9f7`
- `builtins/TestBuiltin_classassign/b5078f9e7ca19316`
- `builtins/TestBuiltin_classassign/c11ec8938214a677`
- `builtins/TestBuiltin_classassign/c3ed47b3a81ddbe1`
- `builtins/TestBuiltin_classassign/c4bb1332dbc8e2cb`
- `builtins/TestBuiltin_classassign/cdffdff4cd49c412`
- `builtins/TestBuiltin_classassign/d540aa19574734fc`
- `builtins/TestBuiltin_classassign/da0f5632421764dc`
- `builtins/TestBuiltin_classassign/e39f5d54faff13ae`
- `builtins/TestBuiltin_classassign/fd413a4cde477768`

### Evaluator validation, missing arguments, parser and vector behavior

15 cases:

- `builtins/TestBuiltin_eval/a9778461f56584a2`
- `builtins/TestBuiltin_quote/0edb37a49cc7ad48`
- `builtins/TestBuiltin_quote/a5e009eb59edcea2`
- `builtins/TestBuiltin_quote/c111ca231dc1e680`
- `builtins/TestBuiltin_switch/677f61cda4c750b6`
- `builtins/TestBuiltin_switch/688aeea8d3f7702a`
- `functions/TestFunctions/058d6e9040ff68aa`
- `functions/TestFunctions/1326a05c5e7aad7f`
- `functions/TestFunctions/4a698b90399ba204`
- `functions/TestFunctions/7630c53d74e78312`
- `functions/TestFunctions/99e242105361cb30`
- `functions/TestFunctions/9a0ff91ac06c8e54`
- `functions/TestFunctions/b2cc3802485c2767`
- `functions/TestFunctions/c1681fdbc9589a91`
- `functions/TestFunctions/c5f23259d75a11f8`

### Lookup name validation, missing bindings, and error context

14 cases:

- `builtins/TestBuiltin_exists/24c81e18cb8aa291`
- `builtins/TestBuiltin_get/2cf3ef5475eeb419`
- `builtins/TestBuiltin_get/94869e1e61cf5638`
- `builtins/TestBuiltin_get/fe9700148cd41375`
- `functions/TestFunctions/4a61646eb4567215`
- `functions/TestFunctions/84a5916659209063`
- `library/base/TestEnvironments/1654596baba48690`
- `library/base/TestEnvironments/35748a2f54100ea3`
- `library/base/TestEnvironments/4ac82696be304b27`
- `library/base/TestEnvironments/5367a8da8482b2d2`
- `library/base/TestEnvironments/53b630f50dbe4810`
- `library/base/TestEnvironments/798e79d029ae8754`
- `library/base/TestEnvironments/d4a836b99627e184`
- `library/base/TestEnvironments/fdef8c955847280b`

### S3 NextMethod metadata and promise error context

6 cases:

- `functions/TestS3Dispatch/35f12e6e0a3c5224`
- `functions/TestS3Dispatch/53668ca0b24c7abf`
- `functions/TestS3Dispatch/7bc62135c9b78e30`
- `functions/TestS3Dispatch/926dc15e5b1ba14a`
- `functions/TestS3Dispatch/bb91fb087c146c41`
- `functions/TestS3Dispatch/e28592d9526fc3d9`

### Environment mutation, locking, and search path endpoints

6 cases:

- `library/base/TestEnvironments/39d974b8082ff42f`
- `library/base/TestEnvironments/742f5c7a79cb407d`
- `library/base/TestEnvironments/764e09241cf806fd`
- `library/base/TestEnvironments/91d250891c17dfa9`
- `library/base/TestEnvironments/a37873dae607c741`
- `library/base/TestEnvironments/c9ec8fbb7666bbef`

### Environment replacement validation and S4 preservation

5 cases:

- `builtins/TestBuiltin_environmentassign/3cb17b2eb22e605a`
- `builtins/TestBuiltin_environmentassign/6823ed839ffcb02f`
- `builtins/TestBuiltin_environmentassign/6d3581a46912bde7`
- `builtins/TestBuiltin_environmentassign/cd987d08652f8300`
- `builtins/TestBuiltin_environmentassign/dbf00906a62936b7`

### Missing internals and system timeout (outside track)

5 cases:

- `builtins/TestBuiltin_getconst/3155022d82be7bfe`
- `builtins/TestBuiltin_getconst/4015a2a976817c09`
- `builtins/TestBuiltin_gettext/42d89daf16ae7a49`
- `builtins/TestBuiltin_system/48a9902fa79b6740`
- `builtins/TestBuiltin_system/832f0ce166b7437e`

### Substitute dots expansion

3 cases:

- `builtins/TestBuiltin_substitute/5868c9cb39afe0ed`
- `builtins/TestBuiltin_substitute/bc7be81aded6089e`
- `functions/TestFunctions/c4b89173958d45e9`
