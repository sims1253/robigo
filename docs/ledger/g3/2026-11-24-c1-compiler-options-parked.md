## 2026-11-24: C1: compilerOptions (PR A) parked behind the observation record (PR B)

Branch `g3/c1-compiler-options`. The JIT's Rust compile reads cmp.R's
compilerOptions where cmp.R reads them: at the entry (make.toplevelContext)
and at each function context (make.functionContext, after its funEnv).
It ports getInlineInfo's optimize levels and cmpCall's findFunDef check.

**Status: parked, by the foreman's ruling.** A is neither gated nor filed.
- **Why:** Sol's round 3 on A found three cases that main gets right and A
  gets wrong (B1, B2, B4 below). Two of them, and #1029's eight cases, are
  failures of the retry model to order what A makes observable. Each
  effect in the middle of a compile (a read, a warning, a force) is a new
  way for the retry model to fail.
- **The order of work:** PR B (#1029, the observation record) goes first.
  A is then rebuilt on B. Its option reads, compile warnings and
  findFunDef's forces become observation kinds of the record, with a
  fresh budget of two Sol rounds.
- **What stays:** the ports that do not depend on order stay on the
  branch as they are: getInlineInfo's levels, `optimize = TRUE` and the
  findFunDef check. The cmpMultiColon accept test (B1) and the
  `function`-literal decline into cmpCall (B3) are not built yet.

### The cases

43 cases in `tests/rt/jit/` (`compile-options-*`, `compile-fundef-*`,
`compile-error-options-read`), recorded with GNU R 4.6.2: 22 ratcheted
and 20 recorded unratcheted. They are the suite that B and the rebuilt A
must pass.

**The unratcheted cases.** Each has an INTERIM header naming its owner:
- **PR B (#1029), order:**
  - fold-closure, fold-literal, fold-read-order and fold-shared-read;
  - fold-stale-binding (Sol S13), fold-shared-visits (S14),
    fold-rebound-primitive (S15) and fold-retry-shadow (g3);
  - retry-warning-shadow (B2), force-error-warnings (B4) and
    level-ops (B6).
- **The rebuilt A, ports:** compile-fundef-multicolon (B1),
  compile-fundef-literal-browser (B3) and compile-fundef-active (B5).
- **C1, ruled:**
  - optimize3 and optimize3-kept (B7): level 3 declines;
  - suppressall: the notes are not ported, an error is raised in place of
    them.
- **INTERIM(L5):** namespace-source and compile-fundef-namespace. M1's
  loader exempts closures under a namespace. Both pass under the rdb boot.
- **#1032:** level-list. robigo-rt's relop does not compare a list.

### Sol's round 3

Probes run under LC_ALL=C on GNU R 4.6.2, main 3693b1c3d and A b5b67847a.
"Class" says what kind of fix each needs: "port" is a piece of cmp.R not
ported yet; "order" is the retry model ordering an effect wrongly.

| case | GNU R | main | A | class |
|---|---|---|---|---|
| compile-fundef-multicolon (B1) | `[1] 7 [1] 7` | `[1] 7 [1] 7` | `[1] 7 forced :: [1] 7` | port; A regression |
| compile-options-retry-warning-shadow (B2) | `read read first first read first second second read second` | `first first first second second second` | `read read first first read second` | order; A regression (warnings lost) |
| compile-fundef-literal-browser (B3) | `forced` at the second call | none | none | port; older than A |
| compile-options-force-error-warnings (B4) | `warning ×3, force` | `warning ×3` | `force` | order; A regression (warnings lost) |
| compile-fundef-active (B5) | `active` at the second call | none | none | port; older than A |
| compile-options-level-ops (B6) | `n = 3` | `n = 0` | `n = 1` | order: Ops runs at the read, not at each getInlineInfo |
| compile-options-optimize3-kept (B7) | `99, 7` | `99, 99` | `99, 99` | ruled: level 3 declines |

### Kinds for PR B's table, from A

- **An option read:** make.toplevelContext's four `get(name,
  compilerOptions)`, cmp.R:997 and :1030.
- **A compile warning given:** replayed as given, never raised again
  (findLocals' `addVar`, cmpAssign's `as.name`). It replaces A's
  `Contexts::warned` count.
- **findFunDef's force:** `get(var, frame)` through findCenvVar's
  delayedAssign (cmp.R:401, :366-369). It covers a promise forced, a
  promise under evaluation (`get()`'s error fails the compile), and an
  active binding run.
