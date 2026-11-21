## 2026-11-18: the profile's Autoloads environment at startup (L)

- What: `.AutoloadEnv` was an "unrun" base binding of the wide loader
  (INTERIM(L5)). The loader skipped the system profile's
  `attach(NULL, name = "Autoloads")`, so any code that touched the name
  raised. That includes cmp.R's top-level `local()`, which `get()`s every
  name in base, so the compiler namespace could not load from source.
- The wide base has a new kind, `boot` (`scripts/base-wide.R`,
  `proto/base-wide.txt`): forms run at startup, in order, in the base
  namespace, at the point where R_LoadProfile runs the profile (before
  `lock_base`). They are Common.R's three:
  - `attach(NULL, name = "Autoloads")`;
  - `.AutoloadEnv <- as.environment(2)`;
  - `assign(".Autoloaded", NULL, envir = .AutoloadEnv)`.

  A lazy binding would not do. `as.environment(2)` is only the Autoloads
  environment until something else is attached, so the binding has to be
  made when the profile runs.
- attach, detach, `search()`, `as.environment()` and `pos.to.env()` were
  already GNU R's in robigo-rt, and none of them needed a port. GNU R
  allows `detach("Autoloads")`, so robigo-rt does too.
- Cases: `tests/rt/env/autoload-env.R`. Five `tests/rt/bytecode/*` cases
  now pass and are ratcheted: cmpfun-ops, cmpfun, compile, fidelity and jit.

### 2026-11-21: with main merged (#856's compile-pkgs step)

- #856 (V4 step 3a) added the cli test
  `compiling_closures_on_serialize_when_the_compiler_cannot_load`. It held
  only while the compiler could not load because of `.AutoloadEnv`, and
  its doc said it would change once `.AutoloadEnv` was real and give way
  to `tests/rt/serialize/compile-pkgs.R`. With this PR it is real: the
  compiler loads, and the first `serialize()` writes byte code. That
  failed CI's `rt` and `test` jobs on 50c4b9c70. The test is removed, and
  `compile-pkgs.R` (recorded on 4.6.2, which compiles the closure) now
  passes and is ratcheted.
- Under gctorture the case takes 492 s alone here (it loads the compiler
  from source with a collection at every allocation; 0 s plain), so in
  rt-ci's gctorture conformance steps it timed out against their 600 s.
  It now requires `volume` (crates/robigo-conformance's KNOWN_FEATURES:
  too many allocations for gctorture) and does not run there. No smaller
  case covers loading the compiler under gctorture yet.
