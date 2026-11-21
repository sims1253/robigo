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
}

/// `ROBIGO_RT_COMMIT`: the commit checked out when the binary was built,
/// which the binary carries as a marker string (src/main.rs) so that the
/// trees built by it can say what built them (scripts/rt-builder.sh); an
/// edit not committed shows only in the binary's sha256. Read from the
/// checkout's git files, not by running git: git refuses a checkout another
/// user owns ("dubious ownership"), as CI's container jobs are, and the
/// marker must not depend on the caller's git configuration. "unknown"
/// outside a git checkout. Rerun when HEAD, the ref it names, or
/// packed-refs moves.
fn commit() {
    use std::path::{Path, PathBuf};
    let watch = |path: &Path| println!("cargo:rerun-if-changed={}", path.display());
    let read = |p: &Path| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let is_sha = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
    // the checkout's root: this package's crates/robigo-rtapi, two up
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dot = root.join(".git");
    // a worktree's .git is a file naming its git directory ("gitdir: X")
    let gitdir: Option<PathBuf> = if dot.is_dir() {
        Some(dot.clone())
    } else {
        read(&dot).and_then(|s| s.strip_prefix("gitdir:").map(|g| root.join(g.trim())))
    };
    let sha = gitdir.and_then(|gd| {
        watch(&gd.join("HEAD"));
        // the shared directory of the refs (a worktree's commondir)
        let common = read(&gd.join("commondir")).map(|c| gd.join(c)).unwrap_or_else(|| gd.clone());
        let head = read(&gd.join("HEAD"))?;
        let Some(name) = head.strip_prefix("ref:").map(str::trim) else {
            // a detached HEAD holds the commit
            return Some(head).filter(|h| is_sha(h));
        };
        for d in [&gd, &common] {
            let loose = d.join(name);
            if let Some(s) = read(&loose).filter(|s| is_sha(s)) {
                watch(&loose);
                return Some(s);
            }
        }
        // a packed ref: packed-refs' "<sha> <name>" line, and the
        // directory the branch's next commit writes the loose file into
        let packed = common.join("packed-refs");
        watch(&packed);
        if let Some(dir) = common.join(name).ancestors().skip(1).find(|d| d.is_dir()) {
            watch(dir);
        }
        read(&packed)?.lines().find_map(|l| {
            let (s, n) = l.split_once(' ')?;
            (n == name && is_sha(s)).then(|| s.to_string())
        })
    });
    println!("cargo:rustc-env=ROBIGO_RT_COMMIT={}", sha.unwrap_or_else(|| "unknown".to_string()));
}
