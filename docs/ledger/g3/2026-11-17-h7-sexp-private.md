## 2026-11-17: H7, a private Sexp field; no safe public function takes a Sexp (#736)

- Why (#736): `robigo_heap::Sexp`'s field was `pub`, so safe code could
  forge one, including robigo-rtbuiltins, which forbids `unsafe`. About 30
  safe functions dereference a `Sexp` argument, so passing them a forged
  one was undefined behaviour from safe code, against docs/architecture.md's
  promise that a safe function is safe to call from any safe code. Two of
  #736's witnesses:
  - `cx.set_early_frame(Sexp(dangling), 0)` makes a traced root;
  - `sym_name(cx, Sexp(8))` reads a PRINTNAME at address 24.
- What:
  - `Sexp(*mut u8)` has a private field. Outside robigo-heap a `Sexp` comes
    from the heap, from `Sexp::NULL`, from a `Ref` (`Ref::sexp`), or from
    `unsafe fn Sexp::from_ptr` (an address C handed back, a word a code
    object or a slot kept). `Sexp::as_ptr` reads the address, which is safe.
  - Every safe `pub fn` of robigo-heap, -rtcore, -rt, -rtapi and
    -rtbuiltins that took a `Sexp` is now an `unsafe fn` with a
    `# Safety` section naming what each `Sexp` must be (a live object, a
    symbol, a call, a code object's constant). That is 34 functions:
    - rt's VM entry points (`call`, `call_closure`, `find_fun`, `get_var*`,
      `symbol_value`, `sym_name`, ...);
    - rtcore's early-frame functions, `hand_to_c`, `owns_raw`,
      `release_raw` and `Slot::object`;
    - heap's `Core::release`;
    - rtapi's `R_altrep_inherits` and the ALTREP method setters, as
      `pub unsafe extern "C" fn` (C sees the same symbols).

    The early-frame comparisons and `Core::release` only compare addresses
    today; their contracts keep a later read sound.
  - Also from #736's list:
    - `errors::text`, which reads a C string, is an `unsafe fn`; its
      `entry!` callers bind each message once, under C's contract;
    - `vmaxset` and `R_ReleaseObject` were already made unsafe on main
      (#742), and R_ReleaseObject's contract now names `x`.
  - `scripts/check-sexp-safety.py` rejects a safe public function (`pub fn`,
    `pub extern "C" fn`, a macro's `fn $name`) of those crates whose
    parameters name `Sexp` outside a closure type. A callback is handed
    objects by the callee, which vouches for them. `scripts/lint.sh` runs
    it, so the lint job and the pre-push hook do, and
    `scripts/test-check-sexp-safety.py` holds 15 positive and negative
    controls (CI's tooling step).
  - The old runtime's robigo-capi (`cpromise_memo_put` in #736) uses
    robigo-core's own `Sexp` and is frozen (L2), so it is not changed here.
- Each `unsafe` block added at a call site names its reason: a code
  object's constant (`v.k`), a `Ref`'s or a rooted value's object, the
  caller's contract, a C contract. Most VM call sites pass constants the
  instruction just loaded from the running code object.
- Measured (this box, release, `scripts/perf.py rt-bars --main`): main
  `66c1a0b1` (sha256 cdf11294…, build id 691e1d81…) against this branch
  `ea0673d9` (5069c0cc…, 89088084…), each built in a fresh private target
  from its own checkout. Every row is ±0.00% in instructions; peak RSS is
  within ±0.5%; 0 failures. `unsafe` changes no code.
- Verified: the full `scripts/rt-ci.py` (310/316, every mode, 400 injected
  failures), Miri on robigo-rtcore, `scripts/lint.sh` (with the new check),
  the check's controls, and actionlint (main's two shellcheck notes).
