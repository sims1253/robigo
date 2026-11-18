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
