# robigo: distribution (binaries, crates.io, releases)

How robigo ships, per platform, and how to use it from another project.
The README shows the short form; this file is the complete reference.

## Status

Experimental, version `0.0.1`. There are no stable versioned releases
yet. What exists:

- **Nightly binaries** (rolling `nightly` GitHub release, built daily
  from `main` when it moved). The Linux ones are profile-guided
  (`scripts/pgo-build.sh`).
- **crates.io** for the self-contained leaf crates
  (`robigo-syntax`, `robigo-numerics`, `robigo-core`).
- Everything else (the full runtime, `libR.so`, the CLI) via the
  nightly binary or a git dependency.

## Platforms

| target | runner | status |
|---|---|---|
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` | supported |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` | supported |
| `aarch64-apple-darwin` | `macos-14` | supported |
| `x86_64-apple-darwin` | `macos-15-intel` | supported |
| Windows | — | not supported yet |

Windows needs porting first: the CLI replaces its process for
`robigo CMD` (`std::os::unix`), the runtime maps its lazy-load cache
with `mmap` and implements fifos over Unix APIs, and the C build
(`crates/robigo-capi/build.rs`) assumes an ELF toolchain plus
libz/libbz2. Linux and macOS share the Unix code paths; the C build
selects the ELF shape (soname, `--whole-archive`) on Linux and the
`install_name`/dynamiclib shape on macOS.

Native package loading (`.Call`/`.External` against the exported C
API) additionally needs the executable's export list, which is wired
up on Linux only (`--dynamic-list`); on macOS pure-R code runs and
native loading is a later track.

## Install

### Nightly binary (recommended)

Download `robigo-<target>.tar.gz` from the
[rolling nightly release](https://github.com/sims1253/robigo/releases/tag/nightly),
which also has `SHA256SUMS.txt`:

```sh
curl -LO https://github.com/sims1253/robigo/releases/download/nightly/robigo-x86_64-unknown-linux-gnu.tar.gz
tar xzf robigo-x86_64-unknown-linux-gnu.tar.gz
./robigo-x86_64-unknown-linux-gnu/robigo --version
```

`robigo --version` reports the build id, e.g. `robigo 0.0.1
(nightly-2026-10-30-abc1234)`.

### From source / via cargo

Prerequisites: Rust 1.99.0 (see `rust-toolchain.toml`), a C compiler,
and on Linux the zlib/bzip2 headers (`zlib1g-dev libbz2-dev` on
Debian/Ubuntu; macOS ships both).

```sh
cargo install --git https://github.com/sims1253/robigo robigo-cli
robigo -e 'cat(1+1, "\n")'
```

From a checkout, `cargo build --release` puts the binary at
`target/release/robigo` (a shared cargo target directory, if
configured, changes where that lands — see the README).

### GNU R alongside robigo

The binary runs on its own: the base library's R sources are embedded
at build time. With a GNU R 4.5 installation present (`R_HOME`), it
additionally loads the rest of the installed library (`utils`,
`stats`, `grDevices`, `graphics`, ...) through R's own
`loadNamespace`, and reads that installation's `etc/` files and
profiles at startup, like R does. Without one, only base is
available.

## Use in your project

### Parse R (`robigo-syntax`, on crates.io)

```sh
cargo add robigo-syntax
```

```rust
let arena = robigo_syntax::Arena::new();
match robigo_syntax::parse_program(&arena, "1 + 1") {
    Ok(exprs) => println!("{} top-level expressions", exprs.len()),
    Err(e) => eprintln!("parse error: {}", e.message),
}
```

### Numerics (`robigo-numerics`, on crates.io)

```sh
cargo add robigo-numerics
```

```rust
// dnorm(0): same bits as GNU R's nmath.
let d = robigo_numerics::norm::dnorm(0.0, 0.0, 1.0, false);
```

### Run R (`robigo-runtime`, git dependency for now)

```sh
cargo add robigo-runtime --git https://github.com/sims1253/robigo
```

```rust
use robigo_runtime::{bootstrap, Interp};

fn main() -> Result<(), String> {
    let mut interp = Interp::new();
    bootstrap::load_base(&mut interp)?;
    if !interp.run_source("x <- 1 + 1\nprint(x)\n", true) {
        return Err("evaluation failed".to_string());
    }
    Ok(())
}
```

`Interp::new` + `bootstrap::load_base` is the embedding entry point;
`run_source` parses and evaluates at top level, printing visible
values, and reports success. Per-expression control is
`eval_toplevel` on values built with `robigo_runtime::lang`
(`crates/robigo-cli/src/main.rs` shows the full startup sequence:
profiles, `startup::run`, console handling).

### crates.io status

| crate | crates.io | notes |
|---|---|---|
| `robigo-syntax` | yes | parser only, no dependencies |
| `robigo-numerics` | yes | nmath + long double, no R objects |
| `robigo-core` | yes | object model + GC (`libc`, `cc` only) |
| `robigo-runtime` | no (`publish = false`) | `build.rs` embeds `../../rlib`, outside the package |
| `robigo-capi` | no (`publish = false`) | depends on `robigo-runtime` |
| `robigo-cli` | no (`publish = false`) | depends on `robigo-runtime` |

Making the rest publishable means moving the vendored R sources under
`crates/robigo-runtime/` (or fetching them at build time) so the
published package is self-contained, then lifting the three
`publish = false` flags in dependency order. The `publish` workflow
already publishes the ready crates; extend its crate list when the
blocker is gone.

## Releases (maintainers)

- **Nightly** (`.github/workflows/nightly.yml`): daily at 02:00 UTC and
  on manual dispatch (with `force` to rebuild an unchanged `main`). A
  `check` job compares `origin/main` against the `nightly` tag and
  skips the run when they match. Each platform builds
  `cargo build --release --locked -p robigo-cli` with `ROBIGO_RELEASE`
  set to `nightly-YYYY-MM-DD-<sha>` (baked into `robigo --version`),
  smoke-tests the binary with no `R_HOME`, and uploads a tarball
  (`robigo`, `README.md`, `LICENSE`). The `release` job tags
  `nightly-YYYY-MM-DD`, force-moves the rolling `nightly` tag, and
  recreates the rolling prerelease with the tarballs plus
  `SHA256SUMS.txt`.
- **crates.io** (`.github/workflows/publish.yml`): on tags `crates-v*`
  or manual dispatch (dry run by default). Publishes the ready crates
  in dependency order. Needs a `CARGO_REGISTRY_TOKEN` repository
  secret with publish rights.
- **Versioned releases**: tag the commit, update `[workspace.package]
  version`, and create the GitHub release from the nightly tarballs of
  that commit (re-run nightly with `force`, or build the same matrix
  by hand). Keep the release a prerelease while the version is `0.x`.
