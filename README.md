<img src="assets/icon.svg" width="64" height="64" alt="">

# rr

An experimental runtime for the R language, written in Rust.

rr aims to run unmodified R code with GNU R's exact behaviour — same
printed output, same messages — and then to make it fast: a specializing
interpreter, vector fusion, and type information from
[ry](https://github.com/sims1253/ry). See [docs/design.md](docs/design.md)
for the research behind it, the architecture and the roadmap, and
[AGENTS.md](AGENTS.md) for the porting contract.

Status: early, but it runs real R code. The parser matches GNU R's on R's
own library sources. rr boots all of GNU R's base-package R code on top of
627 of R's 760 `.Primitive`/`.Internal` functions, starts up like R
(Renviron, profiles, a locked base namespace), and loads installed
packages from a GNU R 4.5.0 library through R's own `loadNamespace`:
`utils`, `stats`, `grDevices` and `graphics` work today. rr cannot load
packages' compiled code, so native routines are ported to Rust (75 of
stats' distribution functions plus 33 others, including R's formula
engine, LINPACK QR, `cov`, `glm` families, `read.csv`/`write.csv`); a
routine without a port raises an error saying so. `lm`, `glm`,
`summary`, `anova`, `predict`, `read.csv` and `write.csv` give output
identical to GNU R, and all 123 conformance scripts match byte for byte.

Not yet: S4 (`methods`), many stats routines (`optim`, `integrate`,
`fft`, `splines`, ...), packages with compiled code beyond base R's, and
locale-aware string collation. `RR_COMPILE=1` enables the bytecode VM;
`RR_TYPE_PROFILE=out.json` records observed types for comparison with
ry ([docs/ry-integration.md](docs/ry-integration.md)).

## Try it

```sh
cargo build --release
target/release/rr -e 'f <- function(n) if (n < 2) n else f(n - 1) + f(n - 2); f(20)'
target/release/rr script.R        # like Rscript script.R
target/release/rr                 # REPL
target/release/rr parse file.R    # parse tree in the oracle dump format
```

(The workspace may build into `~/.cache/cargo-target` if your cargo config
sets a shared target directory.)

## How correctness is checked

GNU R 4.5.0 is the oracle:

- `tests/parse/` — parse trees compared with GNU R's `parse()`. Regenerate
  with `scripts/regen-parse.sh`; run the big corpus (R's own sources) with
  `RR_PARSE_CORPUS=... RR_PARSE_DUMPS=... cargo test -p rr-syntax -- --ignored`.
- `tests/r/` — conformance scripts; stdout/stderr/exit status recorded from
  `Rscript` with `scripts/regen-expected.sh`. `tests/r/passing.txt` is a
  ratchet of scripts that must keep passing.

```sh
cargo test                                             # everything
RR_CONF_FILTER=tests/r/basics RR_CONF_DIFF=1 \
  cargo test -p rr-cli --test conformance -- --nocapture
```

## Layout

- `crates/rr-syntax` — lexer and parser
- `crates/rr-runtime` — object model, evaluator, builtins, printing
- `crates/rr-cli` — the `rr` binary and conformance runner
- `rlib/` — vendored GNU R base-package R sources (GPL-2+)
- `docs/` — design notes

## License

GPL-2.0-or-later, like GNU R, whose sources rr ports and vendors.
