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

/// `ROBIGO_RT_COMMIT`: the commit checked out when the binary was built
/// (`git rev-parse HEAD`; "unknown" outside a checkout), which the binary
/// carries as a marker string (src/main.rs) so that the trees built by it
/// can say what built them (scripts/rt-builder.sh). An edit not committed
/// shows only in the binary's sha256. As robigo-cli's build ID: rerun when
/// HEAD, or the branch it names, moves.
fn commit() {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .and_then(|o| o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()))
            .filter(|s| !s.is_empty())
    };
    let watch = |path: &str| println!("cargo:rerun-if-changed={path}");
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        watch(&head);
    }
    if let Some(loose) = git(&["symbolic-ref", "-q", "HEAD"]).and_then(|b| git(&["rev-parse", "--git-path", &b])) {
        let loose = std::path::Path::new(&loose);
        if loose.exists() {
            watch(&loose.to_string_lossy());
        } else {
            // a packed branch: packed-refs, and the directory its next
            // commit writes the loose file into
            if let Some(packed) = git(&["rev-parse", "--git-path", "packed-refs"]) {
                watch(&packed);
            }
            if let Some(dir) = loose.ancestors().skip(1).find(|d| d.is_dir()) {
                watch(&dir.to_string_lossy());
            }
        }
    }
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ROBIGO_RT_COMMIT={sha}");
}
