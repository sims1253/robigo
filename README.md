<img src="assets/icon.svg" width="64" height="64" alt="">

# robigo

An experimental runtime for the R language, written in Rust. (*Rōbīgō* is
Latin for rust.)

robigo aims to run unmodified R code with GNU R's exact behaviour — same
printed output, same messages — and then to make it fast: a specializing
interpreter, vector fusion, and type information from
[ry](https://github.com/sims1253/ry). See [docs/design.md](docs/design.md)
for the research behind it, the architecture and the roadmap, and
[AGENTS.md](AGENTS.md) for the porting contract.

Status: early, but it runs real R code. The parser matches GNU R's on R's
own library sources. robigo boots all of GNU R's base-package R code on top of
627 of R's 760 `.Primitive`/`.Internal` functions, starts up like R
(Renviron, profiles, a locked base namespace), and loads installed
packages from a GNU R 4.5.0 library through R's own `loadNamespace`:
`utils`, `stats`, `grDevices` and `graphics` work today. robigo cannot load
packages' compiled code, so native routines are ported to Rust (75 of
stats' distribution functions plus 33 others, including R's formula
engine, LINPACK QR, `cov`, `glm` families, `read.csv`/`write.csv`); a
routine without a port raises an error saying so. `lm`, `glm`,
`summary`, `anova`, `predict`, `read.csv` and `write.csv` give output
identical to GNU R, and all 157 conformance scripts match byte for byte.

Not yet: S4 (`methods`), many stats routines (`optim`, `integrate`,
`fft`, `splines`, ...), packages with compiled code beyond base R's, and
locale-aware string collation. Closures and loops run in a register
bytecode VM, hot loops as Cranelift native code; `ROBIGO_COMPILE=0` runs
everything in the AST interpreter and `ROBIGO_JIT=0` turns off native code.
`ROBIGO_TYPE_PROFILE=out.json` records observed types for comparison with
ry ([docs/ry-integration.md](docs/ry-integration.md)).

## Progress

[![speed vs GNU R](https://img.shields.io/endpoint?url=https%3A%2F%2Fsims1253.github.io%2Frobigo%2Fbadges%2Fspeed.json)](https://sims1253.github.io/robigo/)
[![builtins](https://img.shields.io/endpoint?url=https%3A%2F%2Fsims1253.github.io%2Frobigo%2Fbadges%2Fbuiltins.json)](https://sims1253.github.io/robigo/)
[![GNU R's regression tests](https://img.shields.io/endpoint?url=https%3A%2F%2Fsims1253.github.io%2Frobigo%2Fbadges%2Fcorpus-gnur-tests.json)](https://sims1253.github.io/robigo/)
[![help-page examples](https://img.shields.io/endpoint?url=https%3A%2F%2Fsims1253.github.io%2Frobigo%2Fbadges%2Fcorpus-gnur-examples.json)](https://sims1253.github.io/robigo/)
[![FastR test snippets](https://img.shields.io/endpoint?url=https%3A%2F%2Fsims1253.github.io%2Frobigo%2Fbadges%2Fcorpus-fastr.json)](https://sims1253.github.io/robigo/)

Measured by CI on every merge to `main` (see
[docs/verification.md](docs/verification.md)); every benchmark and corpus
over time is on the [progress page](https://sims1253.github.io/robigo/).

<a href="https://sims1253.github.io/robigo/"><picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://sims1253.github.io/robigo/speed-dark.svg">
  <img alt="Speed of robigo relative to GNU R 4.5.0 over time" src="https://sims1253.github.io/robigo/speed.svg">
</picture></a>

<a href="https://sims1253.github.io/robigo/"><picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://sims1253.github.io/robigo/compat-dark.svg">
  <img alt="Share of each test corpus that matches GNU R 4.5.0 over time" src="https://sims1253.github.io/robigo/compat.svg">
</picture></a>

## Install

Nightly binaries for Linux (x86_64, aarch64) and macOS (aarch64,
x86_64) are on the [rolling nightly
release](https://github.com/sims1253/robigo/releases/tag/nightly)
(with `SHA256SUMS.txt`). Or install from git (Rust 1.99+, a C
compiler; on Linux also `zlib1g-dev libbz2-dev`):

```sh
cargo install --git https://github.com/sims1253/robigo robigo-cli
```

Platforms, checksums, and the GNU R interplay are in
[docs/distribution.md](docs/distribution.md).

## Try it

```sh
cargo build --release
target/release/robigo -e 'f <- function(n) if (n < 2) n else f(n - 1) + f(n - 2); f(20)'
target/release/robigo script.R        # like Rscript script.R
target/release/robigo                 # REPL
target/release/robigo parse file.R    # parse tree in the oracle dump format
```

(The workspace may build into `~/.cache/cargo-target` if your cargo config
sets a shared target directory.)

## Use in your project

Parse with `robigo-syntax` and compute with `robigo-numerics` from
crates.io; embed the full runtime (a git dependency until it is
publishable):

```sh
cargo add robigo-syntax robigo-numerics
cargo add robigo-runtime --git https://github.com/sims1253/robigo
```

```rust
let arena = robigo_syntax::Arena::new();
let exprs = robigo_syntax::parse_program(&arena, "1 + 1").unwrap();

let mut interp = robigo_runtime::Interp::new();
robigo_runtime::bootstrap::load_base(&mut interp).unwrap();
interp.run_source("x <- 1 + 1\n", true);
```

Details — library entry points, the crates.io table, and how releases
are cut — are in [docs/distribution.md](docs/distribution.md).

## How correctness is checked

GNU R 4.5.0 is the oracle:

- `tests/parse/` — parse trees compared with GNU R's `parse()`. Regenerate
  with `scripts/regen-parse.sh`; run the big corpus (R's own sources) with
  `ROBIGO_PARSE_CORPUS=... ROBIGO_PARSE_DUMPS=... cargo test -p robigo-syntax -- --ignored`.
- `tests/r/` — conformance scripts; stdout/stderr/exit status recorded from
  `Rscript` with `scripts/regen-expected.sh`. `tests/r/passing/` is a
  ratchet (one marker file per script) of scripts that must keep passing.

```sh
cargo test                                             # everything
ROBIGO_CONF_FILTER=tests/r/basics ROBIGO_CONF_DIFF=1 \
  cargo test -p robigo-cli --test conformance -- --nocapture
```

## Layout

- `crates/robigo-syntax` — lexer and parser
- `crates/robigo-numerics` — nmath and long double arithmetic
- `crates/robigo-runtime` — object model, evaluator, builtins, printing
- `crates/robigo-core` — object model and memory manager (G2)
- `crates/robigo-capi` — `libR.so`, GNU R 4.5's C API (G2)
- `crates/robigo-cli` — the `robigo` binary and conformance runner
- `rlib/` — vendored GNU R base-package R sources (GPL-2+)
- `docs/` — design notes (`distribution.md` for installs and releases)

## License

GPL-2.0-or-later, like GNU R, whose sources robigo ports and vendors.
