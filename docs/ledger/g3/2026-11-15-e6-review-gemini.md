## 2026-11-15: E6 ergonomics review of the builtin-author API (Gemini)

Second, independent E6 read (Gemini 4 Argon, high; council member, did not write the code) of PR #648
(R1a's object API, N1a's environments, B1's builtins), taken after Gemini's quota reset. Sol's review
(`2026-11-13-e6-ergonomics-review.md`) came first and R1c (#651) implemented its five changes, so
several items below are already done on main; the new ones are marked **new**.

**Verdict: pass with changes.** The `&Cx`/`&mut Cx` split, the unsized `Cx` (invisible to authors,
strictly better than `Pin<&mut Cx>`), branded `Ref<'g>`/`View<'g>`, generative scopes and the hook
table are sound; the six modules port GNU R builtins in `forbid(unsafe_code)` Rust with oracle parity.

Ranked changes (status on main 2026-11-15):

1. Make an unwritten `Out` impossible to miss — done in R1c (`#[must_use]`, `Builtin::invoke`'s
   unwritten-output check).
2. One-call rooted results — done in R1c (`Scope::capture`, `Root::replace_from`). **New:** a loop
   that roots its cursor each iteration (`s.root(cx, next)` then `reset`) grows the value stack until
   the scope ends (`envir.rs`'s environment walk); give the in-place replacement a form that takes a
   build closure, so a loop never pushes a slot per iteration.
3. **New:** scalar and string extractors on `Ref` (`as_str_scalar`, `as_i32_scalar`), a one-line
   scalar result, and `lang1`/`lang2`/`lang3`/`mk_string` on `&Cx`.
4. **New (speed):** builtins re-find their own registry row with `registry::lookup("<name>")` on every
   call to check arity — a linear scan by string. Give builtins their row (or a const index) instead.
5. **New:** pre-interned common symbols (`names`, `dim`, `dimnames`, `class`, `tsp`, `as.character`)
   as immortal getters on `&Cx`, removing `NeedGc` from attribute paths.

Also noted: `cx.mk_char` asserts on an embedded NUL where GNU R raises "embedded nul in string"
(**new**, an exactness bug); `.len().unwrap_or(0)` on package ALTREP — addressed by R1c's
`Ref::std_vector()`; `paste_all` redoes all formatting when a retry happens mid-loop (cost, not
correctness).

Follow-up: the **new** items are package R1d, before broad builtin porting (M4).
