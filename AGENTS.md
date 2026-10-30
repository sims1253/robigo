# Working on robigo

robigo is a reimplementation of the R language runtime in Rust. Its goal is to
behave exactly like GNU R 4.5.0: same printed output, same error and
warning messages, same semantics, and to be the fastest R there is. Read
this before changing code, then:

- `docs/architecture.md`: the target architecture and crate boundaries.
- `docs/verification.md`: every check (tests, corpora, benchmarks, CI).
- `docs/workstreams.md`: the tracks agents work on and the protocol for
  parallel work (worktrees, ownership, ratchets, ledgers).
- `docs/design.md`: history and rationale.

Rules of thumb: anything that can be verified is solved, so measure before
and after every change. Effort and churn are never arguments; if a design
is faster for more work, it wins.

## Layout

- `crates/robigo-syntax` — lexer and parser. Produces R-shaped syntax trees.
  Checked against GNU R's `parse()` (`tests/parse`, `scripts/oracle-parse.R`).
- `crates/robigo-numerics` — GNU R's nmath (special functions, d/p/q/r,
  RNG), x87 long double, `%%`/`%/%`. Plain `f64` functions, no R objects.
- `crates/robigo-runtime` — the runtime.
  - `value.rs` object model (`Value`, `Vector<T>`, attributes, NA encodings),
    `env.rs` environments, `sym.rs` interned symbols.
  - `interp.rs` evaluator (eval, promises, argument matching, closures),
    `dispatch.rs` S3 dispatch, `conditions.rs` errors/warnings/handlers.
  - `attrib.rs` attribute access with R's validation (`get_attrib`,
    `set_attrib`), `coerce.rs` type coercion, `format.rs` number/string
    formatting, `print.rs` printing, `deparse.rs` deparsing.
  - `builtins/` the `.Primitive`/`.Internal` layer, one module per GNU R C
    source file (`arith.rs` ~ arithmetic.c/relop.c/logic.c, `attrib.rs` ~
    attrib.c, `bind.rs` ~ bind.c, ...).
  - `bootstrap.rs` loads GNU R's own base-package R code from `rlib/`.
- `crates/robigo-cli` — the `robigo` binary and the conformance test runner.
- `rlib/` — vendored GNU R 4.5.0 R sources (do not edit; see its README).
- `tests/r/**/*.R` — conformance scripts with GNU R's recorded output.
- `tests/corpora/` — large test corpora (FastR's snippets, GNU R's own tests
  and help-page examples) with GNU R's recorded output; `scripts/corpus.py`.
- `benchmarks/` — benchmark suites; `scripts/perf.py`.

## The porting contract

1. **GNU R's C sources are the spec.** A copy of the R 4.5.0 sources lives
   at `~/.cache/robigo-research/rsrc/R-4.5.0` (re-download from
   https://cran.r-project.org/src/base/R-4/R-4.5.0.tar.gz if missing).
   Port the C function; do not reimplement from memory. "Improvements" are
   bugs. Mark non-obvious ports with `// R: file.c:function`.
2. **Base R's R code runs on top of us.** Most of base is R code (in
   `rlib/base/R`) calling `.Internal(foo(...))` and `.Primitive`s. Implement
   the internal, not an R-level replacement. Check `src/main/names.c` for
   each function's `eval` column `XYZ`: `Y=1` means `.Internal` (otherwise
   `.Primitive`), `Z=1` means arguments are evaluated (builtin; `Z=0`:
   special), and `X` is result visibility (`0` visible, `1` invisible —
   use `.invisible()` —, `2` set by the function — use `.custom_vis()`).
3. **Messages are verbatim.** Copy error and warning texts exactly. Pick the
   right call to report:
   - R `errorcall(call, ...)` / `warningcall(call, ...)` →
     `Flow::error_call(call.clone(), msg)` / `interp.warningcall(call, msg)`
     where `call` is the builtin's own call.
   - R `error(...)` / `warning(...)` → `Flow::error(msg)` /
     `interp.warning(msg)`: reported against the context call (the calling
     closure, or the assignment call during `f(x) <- v`).
4. **Internal generics dispatch.** Primitives that are internal generics in
   R (`DispatchOrEval`, `DispatchGroup`) must call `interp.try_dispatch(...)`
   / `interp.try_dispatch_group(...)` first when the object has a class.
5. **Copy-on-modify.** `Value`s are reference counted; mutate through
   `Rc::make_mut`, which copies only when shared. Replacement functions
   (`[<-`, `names<-`, ...) receive the object by value so that `x[i] <- v`
   in a loop can update in place: take arguments out of `args` with
   `std::mem::replace` instead of cloning, and validate before mutating.
6. **No placeholders that guess.** If something is not implemented, raise an
   error that says so (e.g. `"robigo: '<feature>' is not implemented yet"`)
   rather than returning a plausible value.

## Writing a builtin

```rust
// builtins/foo.rs
pub static DEFS: &[Builtin] = &[
    prim("bar", do_bar, 2),               // .Primitive, evaluated args
    special("baz", do_baz, -1),           // .Primitive, unevaluated args
    internal("qux", do_qux, 3).invisible(), // .Internal
];

fn do_bar(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    ...
}
```

Modules are registered in `builtins/mod.rs` (`TABLES`). Useful helpers:
`builtins::match_args` (R's argument matching for builtins with named
formals), `check_arity`, `arg`, `coerce::{to_real, to_int, to_str, ...,
as_logical, as_integer, as_real, as_string}`, `interp.coerce(v, VType)`
(coercion with R's warnings), `attrib::{get_attrib, set_attrib,
copy_most_attrib}`, `interp.call_value(f, args, env)` to call an R function.

## Testing

See `docs/verification.md` for everything; in short:

- `scripts/build.sh` builds the release binary into `.robigo/bin/robigo`
  (worktrees share cargo's target directory; this keeps yours separate).
- `cargo test` runs unit tests, the parser oracle and the conformance suite.
- `scripts/corpus.py run fastr --filter 'TestBuiltin_paste'` runs a slice of
  a corpus; `scripts/corpus.py show fastr ID` diffs one case.
- `scripts/perf.py run --metric instr --filter shootout/` measures
  instruction counts against GNU R.
- Conformance scripts: `tests/r/<area>/<name>.R`. Record GNU R's output
  with `scripts/regen-expected.sh tests/r/<area>/<name>.R` (writes `.Rout`,
  `.Rerr`, `.Rstatus`; GNU R runs with `R_DEFAULT_PACKAGES=NULL`, so only
  base functions are available — no `head`, `str`, `median`, `setNames`).
- Run a subset: `ROBIGO_CONF_FILTER=tests/r/strings ROBIGO_CONF_DIFF=1 cargo test
  -p robigo-cli --test conformance -- --nocapture`.
- Ratchets (`tests/r/passing/`, `tests/corpora/*/passing/`) are one empty
  marker file per case, `<case>.pass`, so adding cases never conflicts.
  Never remove markers to make a test pass; `ROBIGO_CONF_UPDATE=1` and
  `scripts/corpus.py run ... --update` create them.
- `ROBIGO_BOOT_VERBOSE=1 robigo -e 1` lists base-library top-level expressions that
  still fail to evaluate — a good source of missing internals.
- Keep test scripts small and focused: Rscript stops at the first error,
  so one unimplemented function hides everything after it.

## Machine

2 cores, shared `~/.cache/cargo-target` (see `~/.cargo/config.toml`).
Builds are slow and other agents build too: one build at a time
(`scripts/build.sh` takes a lock), build only the crates you need
(`cargo test -p robigo-numerics`), run corpora with `--filter`, and leave
the full runs to CI. `cargo fmt` before committing (CI checks it).
