## 2026-11-15: R1d, the builtin-author API after Gemini's E6 review

- What: the **new** items of Gemini's E6 review
  (`2026-11-15-e6-review-gemini.md`, PR #673, "pass with changes"), in
  `robigo-rtcore`, with the call sites they simplify ported in
  `robigo-rtbuiltins` and `robigo-rt`. Base: `main` at `af7a7f19`.
  - 1. **`Root::replace_with(cx, |cx| ...)`** (`root.rs`): `current =
    f(current)` in place. The slot keeps its old object until the closure
    succeeds, so the closure may read it, and the closure is retried after
    a collection as `Cx::build` retries it. A loop that walks or rebuilds
    a value through one root now pushes no value-stack slot per iteration.
    Ported loops:
    - `envir.rs`: `findVar1mode`'s environment walk and its promise root,
      `R_findDotsEnv`'s walk and its per-frame `capture`, `remove`'s walk;
    - `attrib/set.rs`: the two pairlist tag loops (`names<-` of a pairlist,
      `dimnames`) and their per-cell tag roots;
    - `subassign.rs`: the pairlist `[[<-`/`$<-` cell walk;
    - `options.rs`: the per-option scratch roots, `.Options`'s symbol and
      `baseenv`, made once;
    - `robigo-rt`: compiled assignment's `...` walk (`bcassign.rs`),
      `force_call_promises` and the `options(error)` list evaluation
      (`conditions.rs`).

    Loops that keep a root per element because they need every element
    later (argument lists, `sys.calls`, `unique`'s tables) are unchanged.
    So is `applydefine`'s per-level rooting, which nesting depth bounds as
    GNU R's `PROTECT`s are bounded.
  - 2. **One call for the small things**:
    - `Ref::as_scalar::<T>()` is the element of a length-1 vector of
      element type `T` in standard storage. One generic method stands for
      the review's `as_str_scalar`, `as_i32_scalar` and the others:
      `::<f64>`, `::<i32>`, `::<Lgl>`, `::<Char>` (NA included), `::<Ref>`.
    - `cx.build_scalar(out, x)` for any plain element (GNU R's
      `ScalarReal` and the like).
    - `cx.lang1`/`lang2`/`lang3` (GNU R's, arguments in a pairlist) and
      `cx.mk_string(bytes, enc)` on `&Cx`.

    Ported: 23 one-element `alloc_copy` results, 7 length-1 string reads,
    6 hand-built calls. rtbuiltins' own `coerce::mk_string` is gone.
    `mget`'s and the finalizer's calls keep their `LCONS` argument cells,
    because GNU R builds them that way.
  - 3. **Registry rows at compile time**: `row!("lapply", true)` (an
    `.Internal`), `row!("names")` (either kind) is a `const`. The new
    `const fn registry::find` searches `BUILTINS` when the program is
    compiled, so a name with no row does not compile. It replaces all 68
    `registry::lookup`/`by_name` calls in `robigo-rtbuiltins`' sources,
    each of which scanned the registry by string on every call to check
    arity. Shared functions now take their row
    (`fixed_env`, `is_family`, `dim_or_dimnames`), or pick one per `CODE`
    at compile time (`get`/`exists`/`get0`, `startsWith`/`endsWith`, the
    `duplicated` family, the logic operators, `paste`/`paste0`).
    `lookup` and `by_name` stay for the tests and the dispatch tables.
  - 4. **GNU R's pre-installed symbols**: `cx.sym(RSym::Names)` (`names`,
    `dim`, `dimnames`, `class`, `tsp`, `levels`, `row.names`, `comment`,
    `srcref`, `...`, `as.character`). They are interned when the isolate
    is made and are immortal, so a read allocates nothing and has no
    `NeedGc`. They live in `sym.rs`, the module M2's shared symbol arena
    replaces. Ported: 34 `cx.install(b"<name>")?` calls; attrib's
    `install_named`/`remove_named` now take an `RSym`.
  - 5. **`cx.mk_char` of bytes with a NUL** raises GNU R's
    `embedded nul in string: '<EncodeString>'` where it used to assert.
    Details under Decisions.
- Decisions, each where the review or the foreman left a choice:
  - **The embedded-NUL error rides the allocation-refusal path.** Nothing
    raises under `&Cx`, but every `NeedGc` is answered at a `&mut Cx`
    frame (`Cx::need_gc`, through `build`, `build_with` and
    `retry_alloc`). So `mk_char` records the string and returns a refusal
    whose type byte is no SEXPTYPE (`EMBEDDED_NUL`, 0xff), and `need_gc`
    raises the error instead of collecting. No caller changes.
  - **The string is shown through a hook.** `EncodeString` is
    `robigo-text`'s, which `robigo-rtcore` does not depend on, so the new
    `Hooks::encode_string`, installed by `robigo-rt`, formats it. In the
    recorder's UTF-8 locale this is `'a\0b\n'` and `'é\0\t'`, as GNU R
    4.6.2 shows them; in the C locale GNU R writes `\303\251`. A "bytes"
    string shows only up to its first NUL (EncodeString's `strlen`). Before
    rt installs its hook, only the NUL is escaped. No R-level case exists
    yet, because robigo-rt has no builtin that makes such bytes
    (`rawToChar`); the Rust tests cover both strings.
  - **`row!` is a macro over a `const fn`**, not a new builtin signature.
    Passing each builtin its row (GNU R's `op`) would change every
    builtin's signature (§2.4) for the same effect. A `const` costs
    nothing at run time, and a misspelt name fails to compile.
  - **Pre-installing 11 symbols at isolate creation changes nothing
    observable.** GNU R installs them in `InitNames` too, and rtcore's base
    listing-order oracle test still passes.
- Proof (WSL2 workstation, release, private target `~/.cache/cargo-target-r1d`):
  - `cargo test --release -p robigo-rtcore -p robigo-rtbuiltins -p
    robigo-rt`. `robigo-rtcore/tests/author.rs` gains `replace_with`'s walk
    over 100 environments with an unchanged value-stack depth,
    `as_scalar`, `build_scalar` with `lang1`/`lang3`/`mk_string`'s shapes,
    each `RSym` equal to `install`'s symbol, and the embedded-NUL error. The
    new `robigo-rt/tests/author.rs` checks the message through rt's
    `EncodeString`. `tests/api.rs` expects the refusal where it expected
    the panic.
  - The author guide's doctests use `build_scalar`, and the guide
    describes the rest.
  - RT conformance is 211/216, the same 5 failures as main.
    `scripts/rt-ci.py` passes all steps; clippy and fmt are clean.
- Cost (callgrind instructions per iteration, `scripts/idiom-vs-r.py
  --no-r`, against main `af7a7f19`):
  - `n <- inherits(x, "foo") + length(typeof(x)) + length(class(x))`:
    9,908 to 7,944 (-19.8%, about 650 instructions per arity check that
    no longer scans the registry).
  - The `trycatch` row: 139,417 to 132,893 (-4.7%).
  - `paste(q, i)`: 18,829 to 18,054 (-4.1%).
  - `match(q, keys)` over 1,000 keys: 99,567 to 99,306 (the hashing
    dominates).
  - The `plain` row: 6,895, unchanged.

  The `e7-*` rows fail on both binaries (`seq.default`, not in the M1
  subset), so `match` and `paste` were measured with equivalent setups.
- Review: Sol (`gpt-6.1-sol`, high). Round 1: approved, with one nit,
  fixed: a "bytes" string was encoded past its NUL, where GNU R's
  `EncodeString` stops at it (`strlen`). The fix has an assertion in
  `robigo-rt/tests/author.rs`. Round 2: approved.
