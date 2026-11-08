//! `/tmp/perf-<pid>.map` for `ROBIGO_PERF_MAP`: what samply, perf and
//! callgrind need to name JIT frames, which are otherwise anonymous memory
//! (docs/profiling.md).
//!
//! A compile notes each function after `define_function` and flushes after
//! `finalize_definitions`; the gate, the sizing, the names (the module's own
//! declarations, tagged with the compiling thread) and the file stay here.
//! Disabled, `note` is one branch and `flush` does nothing.

use cranelift_codegen::Context;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};
use std::fs::File;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Functions defined by one compile, waiting for their finalized addresses.
#[derive(Default)]
pub(crate) struct PerfMap(Vec<(FuncId, usize)>);

impl PerfMap {
    /// Record `id` while `ctx` still holds its compiled code (before
    /// `clear_context`): cranelift-jit exposes only the start address.
    pub(crate) fn note(&mut self, id: FuncId, ctx: &Context) {
        if !enabled() {
            return;
        }
        if let Some(cc) = ctx.compiled_code() {
            self.0.push((id, cc.code_buffer().len()));
        }
    }

    /// Write the noted functions; call after `finalize_definitions`.
    pub(crate) fn flush(self, module: &JITModule) {
        if self.0.is_empty() {
            return;
        }
        // After a failed write the map may end in a torn line: stop there.
        static DEAD: AtomicBool = AtomicBool::new(false);
        let Some(file) = file() else { return };
        if DEAD.load(Ordering::Relaxed) {
            return;
        }
        // JIT state is thread-local, so declared names repeat across
        // threads; the tag keeps map entries distinct.
        let thread = std::thread::current().id();
        let mut lines = String::new();
        for (id, len) in self.0 {
            let ptr = module.get_finalized_function(id) as usize;
            let name = module.declarations().get_function_decl(id).linkage_name(id);
            lines.push_str(&format!("{ptr:x} {len:x} {name}#t{thread:?}\n"));
        }
        let mut f = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // One write per compile, under the lock: lines never interleave.
        if let Err(e) = f.write_all(lines.as_bytes())
            && !DEAD.swap(true, Ordering::Relaxed)
        {
            eprintln!("ROBIGO_PERF_MAP: map write failed ({e}); no further entries");
        }
    }
}

/// Read once. Empty, `0`, `false`, `no` and `off` (any case) disable.
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let v = std::env::var("ROBIGO_PERF_MAP").unwrap_or_default().to_lowercase();
        !matches!(v.as_str(), "" | "0" | "false" | "no" | "off")
    })
}

/// The process's map, opened on first use; `None` (warned once) if it
/// cannot be.
fn file() -> Option<&'static Mutex<File>> {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        // The exact path is the perf-map convention (perf/samply look only
        // here). create_new refuses whatever sits there — a stale map from
        // a recycled pid, or a planted symlink: remove the entry itself and
        // create afresh, never following it. 0600 keeps the code addresses
        // to this user in world-writable /tmp.
        let path = format!("/tmp/perf-{}.map", std::process::id());
        let open = || {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(&path)
        };
        let opened = match open() {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&path).and_then(|()| open())
            }
            r => r,
        };
        opened.map_err(|e| eprintln!("ROBIGO_PERF_MAP: cannot open {path}: {e}")).ok().map(Mutex::new)
    })
    .as_ref()
}
