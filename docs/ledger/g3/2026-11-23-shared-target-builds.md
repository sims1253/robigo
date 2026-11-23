# Checkouts sharing a target directory: "api.c's layout table has another length 48 vs 47" (2026-11-23)

X/P1, from A1's builtin-diff `ci` run, which failed twice with
`robigo-rtapi: api.c's layout table has another length` (48 against 47).
Every delegate keeps one cargo target directory across several worktrees,
so this is the ordinary way robigo is built, not an exotic one.

## The cause

`crates/robigo-heap/build.rs` compiles `src/layout/defs.rs` and
`src/types.rs` into itself (`#[path]` modules), and writes
`robigo_layout.h` from the table it was compiled with. Worktrees sharing a
target directory share that compiled build script: cargo judges it fresh by
modification time.

- **What the variable did and did not do:** each checkout's
  `.cargo/config.toml` sets `ROBIGO_WORKTREE` to the checkout, and the
  build scripts declare `cargo:rerun-if-env-changed=ROBIGO_WORKTREE`. That
  reruns the script for another checkout, but nothing recompiled it.
- **The failure:** a checkout whose `defs.rs` was older than another
  checkout's build ran that build's compiled script, and wrote the other
  checkout's header. The C side (`api.c`, 47 entries) then disagreed with
  the Rust side (`layout::TABLE`, 48), and robigo-rtapi's `check_layout`
  stopped the binary.
- **What A1 saw** ("cargo hashes workspace path dependencies without their
  path, so two worktrees share one OUT_DIR") is the visible half. The
  OUT_DIR is shared by design; which compiled build script runs was the
  defect.
- **Paths in `rerun-if-changed`, absolute or not, would not have fixed it:**
  they decide when a script reruns, not which compiled script runs.
- **Only robigo-heap's build script compiles checkout sources into itself.**
  The others read the checkout when they run: libdeflate's directory and the
  commit marker's git files, from the run-time `CARGO_MANIFEST_DIR` since
  53087c176. `ROBIGO_WORKTREE` covers those.

## The fix

- **Recompile per checkout:** robigo-heap's build script reads
  `option_env!("ROBIGO_WORKTREE")` at compile time, so rustc's dep-info
  makes cargo recompile it for another checkout. The heap library reads it
  too, as every other rt crate already did; it was the one rt crate
  without it, so its compiled `TABLE` could be reused as well.
- **A stale script stops:** at run time the script compares the
  `include_str!` copies of `defs.rs` and `types.rs` it was compiled with
  against the files under the run-time `CARGO_MANIFEST_DIR`. On a
  difference it stops with a message and never writes a header.
- **A build that cannot tell the checkouts apart is refused:** the build
  fails when `ROBIGO_WORKTREE` is unset, or names another checkout. The
  message names both cures (run cargo inside the checkout, or set the
  variable to the checkout's path). The check is in robigo-heap's build
  script, the lowest rt crate with one, so every rt build and
  `cargo test -p robigo-heap` passes through it.
  - **Unset:** cargo reads `.cargo/config.toml` from its working
    directory, so a cargo run outside the checkout (`--manifest-path`)
    does not set the variable.
  - **Another checkout's:** cargo's `[env]` does not override a variable
    already in the environment, so a shell that exported one checkout's
    value would pass it to every checkout.
- **No in-repo caller is affected:** none runs cargo outside a checkout.
  - CI's jobs run inside the checkout, and the container jobs inside its
    mount.
  - The docker steps run R, not cargo.
  - `scripts/build.sh`, `pgo-build.sh`, `build-vg.sh` and `lint.sh` `cd`
    to the root; `rt-ci.py` and `builtin-diff.py` pass `cwd=ROOT`.
  - Miri is run in the checkout.
  - The fuzz crate does not depend on robigo-heap.

## The measurements

Two worktrees: A, and B, which is A plus one layout entry (48 against 47).
They were built one after the other into one fresh target directory (dev
profile) and each binary was run. "Back-dated" is B with every file's mtime
set to 2020, older than any build.

**Before the fix** (A = edb14dcfb), cargo run inside each checkout, as
builtin-diff does:

| step | build | run | the binary's commit marker |
|---|---|---|---|
| 1 | A | ok | A's |
| 2 | B | `layout table has another length` | B's |
| 3 | A | ok | A's |
| 4 | B, back-dated | `layout table has another length` | B's |

B's header never had the extra entry.

**Outside the checkout,** with only the recompile fixes (b6f2776cc),
`--manifest-path` from `$HOME`: A after B reused B's robigo-heap library
and header, which agree with each other. The binary was A's crates over B's
heap, and it passed every check. That is why an unset variable is now an
error.

**After the fix** (0152f304e):

| run | result |
|---|---|
| inside each checkout: A, B, A, B back-dated | four binaries, each running and naming its own commit |
| outside, the variable unset | refused at the first build, with the message |
| outside, the variable set by hand: A, B, A, B back-dated | four binaries, each running and naming its own commit |
| inside B, A's value exported | refused, naming both checkouts |
| `cargo test -p robigo-heap` outside | refused |

The commit marker named the right commit in every run, before the fix too.
