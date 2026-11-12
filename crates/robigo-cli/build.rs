// The robigo executable exports GNU R's C API (robigo-capi) to the shared
// objects it loads, and nothing else (docs/core.md §6).
fn main() {
    let list = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../robigo-capi/exports.list");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", list.display());
    println!("cargo:rerun-if-env-changed=ROBIGO_RELEASE");
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    // --dynamic-list is an ELF (GNU ld) option: macOS and Windows link
    // without it (native package loading there is a later track).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,--dynamic-list={}", list.display());
    }
    // cargo does not relink when a file named in a link argument changes:
    // make the list's contents part of the build so a new export relinks
    let text = std::fs::read_to_string(&list).unwrap_or_default();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    println!("cargo:rustc-env=ROBIGO_EXPORTS_HASH={h:016x}");
    // Baked into `robigo --version` so a nightly binary identifies
    // itself. The nightly workflow sets ROBIGO_RELEASE to
    // nightly-YYYY-MM-DD-<sha>; local builds report dev-<sha>.
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .and_then(|o| (o.status.success()).then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()))
            .filter(|s| !s.is_empty())
    };
    let release = std::env::var("ROBIGO_RELEASE").unwrap_or_else(|_| {
        // A commit changes no file cargo watches: rerun when HEAD, or the
        // branch it names, moves. git prints paths relative to this
        // script's directory, the package's.
        let watch = |path: &str| println!("cargo:rerun-if-changed={}", path);
        if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
            watch(&head);
        }
        if let Some(loose) = git(&["symbolic-ref", "-q", "HEAD"]).and_then(|b| git(&["rev-parse", "--git-path", &b])) {
            let loose = std::path::Path::new(&loose);
            if loose.exists() {
                watch(&loose.to_string_lossy());
            } else {
                // A packed branch lives in packed-refs, and its next commit
                // writes the loose file, which shows in the nearest existing
                // directory above it (a missing path would rerun this script
                // on every build).
                if let Some(packed) = git(&["rev-parse", "--git-path", "packed-refs"]) {
                    watch(&packed);
                }
                if let Some(dir) = loose.ancestors().skip(1).find(|d| d.is_dir()) {
                    watch(&dir.to_string_lossy());
                }
            }
        }
        match git(&["rev-parse", "--short", "HEAD"]) {
            Some(sha) => format!("dev-{sha}"),
            None => "dev".to_string(),
        }
    });
    println!("cargo:rustc-env=ROBIGO_BUILD_ID={release}");
}
