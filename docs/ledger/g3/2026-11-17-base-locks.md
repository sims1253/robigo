## 2026-11-17: base locked after the system profile

- What: `setup_Rmainloop` locks base right after `R_LoadProfile` of the
  system profile (main.c): `R_LockEnvironment(R_BaseNamespace, TRUE)` (the
  frame and every binding; the namespace's bindings are baseenv()'s), then
  `R_LockEnvironment(R_BaseEnv, FALSE)`, then `.Device` and `.Devices`
  unlocked again. robigo-rt locked neither, so `assign("x", 1, envir =
  baseenv())` succeeded and `bindingIsLocked("paste", baseenv())` was
  FALSE. B found it. `base::lock_base` now does the three steps, at the
  end of M1's loader (whose profile bindings are among its lazy ones) and
  after stage 0's profile.
- The stores GNU R's C makes into base after that go past the locks, as
  `SET_SYMVALUE` does: `Cx::set_symvalue`, now used for `.Last.value`,
  `last.warning`, `.Traceback`, the option list (`options()` changes
  `.Options` in place in GNU R) and the primitives `define_builtins` binds
  (names.c binds them so).
- INTERIM(L5): the wide base loader's lazy bindings compute their values
  on first use, running base's own definition forms, which rebind the name
  (GNU R's build ran them before the locks). They run under
  `Cx::building_base`: a base binding changes past its lock there, and
  adding a binding to the locked base is refused with a message naming the
  loader. L5's boot from the databases deletes both with the loader.
- Not here: `environmentIsLocked`, `lockBinding`/`unlockBinding` and
  `parent.env<-` are B's (envir.c, B5b), so the case does not call them;
  B's own case lines ratchet once those land.
- Proof: `tests/rt/env/base-locks.R`, recorded on 4.6.2 and ratcheted (the
  locks, GNU R's errors, `.Last.value`, `last.warning` and an option set
  after them). Main fails it. fastr on robigo-rt has every case's result
  equal to main's (31,373; 0 regressions); the other rt corpora have 0
  regressions.
