// robigo-rtapi's C half (src/api.c: accessors, variadic functions,
// R_UnwindProtect), and the link flag that exports the API from the
// binaries: packages loaded with dlopen resolve GNU R's symbols against the
// executable (docs/g3.md §14.1).
fn main() {
    commit();
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    println!("cargo:rerun-if-changed=src/api.c");
    println!("cargo:rerun-if-changed=../robigo-rtcore/include/robigo_capi.h");
    // No unwind tables, as the heap's shim: nothing may unwind through C
    // frames of the API.
    // robigo_layout.h, which robigo-heap's build writes (its `links`)
    let layout = std::env::var("DEP_ROBIGO_HEAP_INCLUDE").unwrap_or_else(|e| {
        panic!(
            "robigo-rtapi: DEP_ROBIGO_HEAP_INCLUDE: {e} (robigo-heap's `links` key and its `cargo:include` line set it)"
        )
    });
    println!("cargo:rerun-if-changed={layout}/robigo_layout.h");
    // for the tests that compile C against robigo's headers
    println!("cargo:rustc-env=ROBIGO_LAYOUT_INCLUDE={layout}");
    cc::Build::new()
        .file("src/api.c")
        .include("../robigo-rtcore/include")
        .include(&layout)
        .flag_if_supported("-std=gnu11")
        .flag_if_supported("-fno-unwind-tables")
        .flag_if_supported("-fno-asynchronous-unwind-tables")
        .compile("robigo_rtapi_c");
    // The API's symbols in the executables' dynamic symbol table, which a
    // package dlopen'ed later resolves them against. Without it every
    // package fails to load with undefined symbols, so a target this does
    // not know says so at build time.
    let export = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => Some("-rdynamic"),
        Ok("macos") => Some("-Wl,-export_dynamic"),
        _ => None,
    };
    match export {
        Some(flag) => {
            println!("cargo:rustc-link-arg-bins={flag}");
            println!("cargo:rustc-link-arg-tests={flag}");
        }
        None => println!(
            "cargo:warning=robigo-rtapi: no flag to export the C API from executables on this target; packages will not load"
        ),
    }
    order_file();
}

/// The link order of robigo-rt's functions (`rt-starts.order`,
/// scripts/rt-order.py): the functions a start runs, hottest first, which
/// the linker packs at the front of `.text`, so a start faults in about
/// 2 MB of the executable instead of nearly all of its 4.7 MB
/// (docs/ledger/g3/2026-11-23-order-file.md). For the `robigo-rt` binary
/// only, and only where the link is LLD's, which takes the file: the
/// x86_64-unknown-linux-gnu target, whose default linker rustc's own LLD
/// is, when nothing names another linker (a `linker` of the target in
/// cargo's configuration, which cargo passes here as RUSTC_LINKER, or a
/// linker flag among the rustflags). Elsewhere the binary links unordered,
/// which is only slower to start; ROBIGO_NO_ORDER_FILE=1 asks for that too.
fn order_file() {
    println!("cargo:rerun-if-changed=rt-starts.order");
    println!("cargo:rerun-if-env-changed=ROBIGO_NO_ORDER_FILE");
    println!("cargo:rerun-if-env-changed=RUSTC_LINKER");
    if std::env::var_os("ROBIGO_NO_ORDER_FILE").is_some_and(|v| v == "1") {
        return;
    }
    let target = std::env::var("TARGET").unwrap_or_default();
    // (cargo's rustflags, unit-separated: one naming a linker, `-fuse-ld`,
    // or LLD's off-switch means the link may not be rustc's LLD)
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let other_linker = std::env::var_os("RUSTC_LINKER").is_some()
        || flags.split('\u{1f}').any(|f| f.contains("linker") || f.contains("fuse-ld"));
    if target != "x86_64-unknown-linux-gnu" || other_linker {
        println!(
            "cargo:warning=robigo-rtapi: robigo-rt links without its order file (target {target}{}): it starts slower, nothing else changes",
            if other_linker { ", another linker configured" } else { "" }
        );
        return;
    }
    // (the package's directory now, not `env!` at the script's compile: a
    // compiled build script is reused across checkouts sharing a target)
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("robigo-rtapi: cargo sets CARGO_MANIFEST_DIR");
    let order = std::path::Path::new(&manifest).join("rt-starts.order");
    println!("cargo:rustc-link-arg-bin=robigo-rt=-Wl,--symbol-ordering-file={}", order.display());
    // (a name the binary no longer has is skipped: scripts/rt-order.py check
    // counts them)
    println!("cargo:rustc-link-arg-bin=robigo-rt=-Wl,--no-warn-symbol-ordering");
}

/// `ROBIGO_RT_COMMIT`: the commit checked out when the binary was built,
/// which the binary carries as a marker string (src/main.rs) so that the
/// trees built by it can say what built them (scripts/rt-builder.sh); an
/// edit not committed shows only in the binary's sha256. Read from the
/// checkout's git files ($GIT_DIR, else .git or the worktree it names), not
/// by running git: git refuses a checkout another user owns ("dubious
/// ownership"), as CI's container jobs are, and the marker must not depend
/// on the caller's git configuration. Symbolic refs are followed, five
/// levels as git does. "unknown" outside a git checkout. Rerun when HEAD,
/// a ref it leads to, packed-refs or GIT_DIR changes.
fn commit() {
    use std::path::{Path, PathBuf};
    let watch = |path: &Path| println!("cargo:rerun-if-changed={}", path.display());
    let read = |p: &Path| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let is_sha = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
    // the checkout's root: this package's crates/robigo-rtapi, two up
    // (the package's directory now, from cargo's environment: a compiled
    // build script is reused across checkouts sharing a target directory,
    // and a compile-time path would read another checkout's HEAD)
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("robigo-rtapi: cargo sets CARGO_MANIFEST_DIR");
    let root = Path::new(&manifest).join("../..");
    // the git directory: $GIT_DIR when set (relative to the root, where
    // cargo is run from), else .git, which in a worktree is a file naming
    // it ("gitdir: X")
    println!("cargo:rerun-if-env-changed=GIT_DIR");
    let dot = root.join(".git");
    let gitdir: Option<PathBuf> = match std::env::var_os("GIT_DIR") {
        Some(g) => Some(root.join(g)),
        None if dot.is_dir() => Some(dot.clone()),
        None => {
            // (the pointer watched: retargeted, it names another git dir)
            if dot.is_file() {
                watch(&dot);
            }
            read(&dot).and_then(|s| s.strip_prefix("gitdir:").map(|g| root.join(g.trim())))
        }
    };
    let sha = gitdir.and_then(|gd| {
        // the shared directory of the refs (a worktree's commondir)
        // (watched where it exists, a worktree's; where it does not, the git
        // dir is its own and nothing makes one appear: a watched path that
        // is absent makes cargo rerun this script on every build)
        let cd = gd.join("commondir");
        if cd.is_file() {
            watch(&cd);
        }
        let common = read(&cd).map(|c| gd.join(c)).unwrap_or_else(|| gd.clone());
        // a ref's value: a loose file (the worktree's own, then the shared
        // one) or packed-refs' "<sha> <name>" line; watched either way
        let lookup = |name: &str| -> Option<String> {
            for d in [&gd, &common] {
                let loose = d.join(name);
                if let Some(s) = read(&loose) {
                    watch(&loose);
                    return Some(s);
                }
            }
            // packed: packed-refs, and the directory the ref's next commit
            // writes the loose file into
            let packed = common.join("packed-refs");
            watch(&packed);
            if let Some(dir) = common.join(name).ancestors().skip(1).find(|d| d.is_dir()) {
                watch(dir);
            }
            read(&packed)?.lines().find_map(|l| {
                let (s, n) = l.split_once(' ')?;
                (n == name && is_sha(s)).then(|| s.to_string())
            })
        };
        watch(&gd.join("HEAD"));
        let mut v = read(&gd.join("HEAD"))?;
        // a symbolic ref may name another (git follows five levels)
        for _ in 0..5 {
            match v.strip_prefix("ref:").map(str::trim) {
                Some(name) => v = lookup(name)?,
                None => return Some(v).filter(|h| is_sha(h)),
            }
        }
        None
    });
    println!("cargo:rustc-env=ROBIGO_RT_COMMIT={}", sha.unwrap_or_else(|| "unknown".to_string()));
}
