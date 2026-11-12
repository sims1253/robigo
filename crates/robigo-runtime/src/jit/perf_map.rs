//! What samply and perf need to see into JIT code, under `ROBIGO_PERF_MAP`
//! (docs/profiling.md). JIT functions are otherwise anonymous memory.
//!
//! - Any value but `jitdump`: `/tmp/perf-<pid>.map`, a name per function.
//! - `jitdump`: that map, and perf's jitdump (tools/perf/Documentation/
//!   jitdump-specification.txt): each function's code and a line table from
//!   its machine pcs to the VM ops it was built from. `perf inject --jit`
//!   turns them into ELF objects, so `perf report --sort sym,srcline` and
//!   `perf annotate` attribute samples per op. The lines point into a
//!   listing of the function's VM code written beside the dump
//!   (`<name>.ops`, line `pc + 1` is op `pc`, the last line the function's
//!   own glue: entry, exits, deopts). A call inlined into a function counts
//!   as the op that calls it. Code Cranelift makes on its own (constants
//!   rematerialized at their use, the egraph's rewrites) carries no source
//!   location; it is placed just before the instruction that uses it, so it
//!   counts as the next op.
//!
//! A compile tags the instructions of each op ([`tag`]), notes each function
//! after `define_function` and flushes after `finalize_definitions`; the
//! gate, the sizing, the names (the module's own declarations, tagged with
//! the compiling thread) and the files stay here. Disabled, `tag` and
//! `note` are one branch each and `flush` does nothing.

use crate::compile::Code;
use crate::sys;
use cranelift_codegen::Context;
use cranelift_codegen::MachSrcLoc;
use cranelift_codegen::ir::SourceLoc;
use cranelift_frontend::FunctionBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Functions defined by one compile, waiting for their finalized addresses.
#[derive(Default)]
pub(crate) struct PerfMap(Vec<Noted>);

struct Noted {
    id: FuncId,
    len: usize,
    /// For the jitdump.
    dump: Option<Dump>,
}

struct Dump {
    /// The machine code as compiled. (Relocations are applied only when
    /// the module finalizes, so calls of helpers show a zero target.)
    code: Vec<u8>,
    /// (code offset, listing line) where the line changes.
    lines: Vec<(u32, u32)>,
    /// The VM code's listing (`None`: no VM code, an entry thunk).
    listing: Option<String>,
}

/// Attribute the instructions built from here on to op `pc` (`None`: the
/// function's own glue). Only the jitdump reads it.
///
/// The location is the listing line: `pc + 1`, and 0 for glue. Cranelift
/// keeps locations relative to a function's first one, in 31 bits, and the
/// one just below that base would wrap to "none": the base is always 0.
pub(crate) fn tag(b: &mut FunctionBuilder, pc: Option<usize>) {
    if mode() == Mode::JitDump {
        b.func.params.ensure_base_srcloc(SourceLoc::new(0));
        b.set_srcloc(SourceLoc::new(pc.map_or(0, |pc| pc as u32 + 1)));
    }
}

impl PerfMap {
    /// Record `id`, built from `code` with [`tag`]s, while `ctx` still holds
    /// its compiled code (before `clear_context`): cranelift-jit exposes
    /// only the start address.
    pub(crate) fn note(&mut self, id: FuncId, ctx: &Context, code: Option<&Code>) {
        let mode = mode();
        if mode == Mode::Off {
            return;
        }
        let Some(cc) = ctx.compiled_code() else { return };
        let len = cc.code_buffer().len();
        let dump = (mode == Mode::JitDump).then(|| {
            let (lines, listing) = match code {
                Some(code) => {
                    let glue = code.ops.len() as u32 + 1;
                    (line_table(cc.buffer.get_srclocs_sorted(), len as u32, glue), Some(listing(code)))
                }
                None => (Vec::new(), None),
            };
            Dump { code: cc.code_buffer().to_vec(), lines, listing }
        });
        self.0.push(Noted { id, len, dump });
    }

    /// Write the noted functions; call after `finalize_definitions`.
    pub(crate) fn flush(self, module: &JITModule) {
        if self.0.is_empty() {
            return;
        }
        // JIT state is thread-local, so declared names repeat across
        // threads; the kernel's thread id keeps them distinct.
        let tid = sys::gettid();
        let mut lines = String::new();
        let mut records = Vec::new();
        for n in self.0 {
            let ptr = module.get_finalized_function(n.id) as u64;
            let name = format!("{}#t{tid}", module.declarations().get_function_decl(n.id).linkage_name(n.id));
            lines.push_str(&format!("{ptr:x} {:x} {name}\n", n.len));
            if let Some(d) = n.dump {
                records.push((ptr, name, d));
            }
        }
        // One write per compile, under the lock: lines never interleave.
        let mut map = map_file().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(f) = map.as_mut()
            && let Err(e) = f.write_all(lines.as_bytes())
        {
            // The map may now end in a torn line: stop there.
            eprintln!("ROBIGO_PERF_MAP: map write failed ({e}); no further entries");
            *map = None;
        }
        drop(map);
        if !records.is_empty() {
            jitdump(tid, records);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Map,
    JitDump,
}

/// Read once. Empty, `0`, `false`, `no` and `off` (any case) disable;
/// `jitdump` adds the jitdump to the map.
fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let v = std::env::var("ROBIGO_PERF_MAP").unwrap_or_default().to_lowercase();
        match v.as_str() {
            "" | "0" | "false" | "no" | "off" => Mode::Off,
            "jitdump" => Mode::JitDump,
            _ => Mode::Map,
        }
    })
}

/// The process's map, opened on first use; `None` (reported once) if it
/// cannot be, or once a write has failed.
fn map_file() -> &'static Mutex<Option<File>> {
    static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        // The exact path is the perf-map convention (perf/samply look only
        // here). create_new refuses whatever sits there — a stale map from
        // a recycled pid, or a planted symlink: remove the entry itself and
        // create afresh, never following it. 0600 keeps the code addresses
        // to this user in world-writable /tmp.
        let path = format!("/tmp/perf-{}.map", std::process::id());
        let opened = match create_private(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&path).and_then(|()| create_private(&path))
            }
            r => r,
        };
        Mutex::new(opened.map_err(|e| eprintln!("ROBIGO_PERF_MAP: cannot open {path}: {e}")).ok())
    })
}

/// A new file only this user can read, open for reading too (the jitdump's
/// mapping needs that).
fn create_private(path: impl AsRef<std::path::Path>) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().create_new(true).read(true).write(true).mode(0o600).open(path)
}

/// The listing a function's line table points into.
fn listing(code: &Code) -> String {
    let mut s = String::new();
    for pc in 0..code.ops.len() {
        s.push_str(&format!("{pc:4} {}\n", code.op_text(pc)));
    }
    s.push_str("     (entry, exits and deopts)\n");
    s
}

/// The listing line of each stretch of `len` bytes of code, from the
/// locations [`tag`] set (0 is `glue`). Untagged code counts as the next
/// tagged stretch, except the prologue and code after the last tag (glue).
fn line_table(srclocs: &[MachSrcLoc], len: u32, glue: u32) -> Vec<(u32, u32)> {
    // (Compiled code's locations are absolute: the base is applied.)
    let tagged = srclocs.iter().filter_map(|s| {
        let loc = if s.loc.is_abs() { s.loc.as_abs() } else { SourceLoc::default() };
        (!loc.is_default()).then_some((s.start, s.end, if loc.bits() == 0 { glue } else { loc.bits() }))
    });
    let mut out: Vec<(u32, u32)> = Vec::new();
    let mut at = 0;
    for (start, end, line) in tagged {
        // The stretch claims the untagged code before it, back to `at`.
        let from = if at == 0 { start } else { at };
        if from > 0 && out.is_empty() {
            out.push((0, glue));
        }
        if out.last().is_none_or(|l| l.1 != line) {
            out.push((from, line));
        }
        at = end;
    }
    if out.is_empty() {
        out.push((0, glue));
    } else if at < len && out.last().is_some_and(|l| l.1 != glue) {
        out.push((at, glue));
    }
    out
}

// ----------------------------------------------------------------------
// The jitdump

const JIT_CODE_LOAD: u32 = 0;
const JIT_CODE_DEBUG_INFO: u32 = 2;

#[cfg(target_arch = "x86_64")]
const ELF_MACH: u32 = 62; // EM_X86_64
#[cfg(target_arch = "aarch64")]
const ELF_MACH: u32 = 183; // EM_AARCH64
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const ELF_MACH: u32 = 0; // EM_NONE

struct DumpFile {
    file: File,
    /// Where the listings go: the dump's own directory.
    dir: PathBuf,
    /// `code_index` of the next function.
    next: u64,
}

/// Append the records of one compile, under the lock.
fn jitdump(tid: u32, functions: Vec<(u64, String, Dump)>) {
    let mut guard = dump_file().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(df) = guard.as_mut() else { return };
    let mut out = Vec::new();
    let mut failed = None;
    for (addr, name, d) in functions {
        let stamp = sys::monotonic_ns();
        if let Some(listing) = &d.listing {
            let path = df.dir.join(format!("{name}.ops"));
            if let Err(e) = std::fs::write(&path, listing) {
                failed = Some(format!("cannot write {}: {e}", path.display()));
                break;
            }
            debug_info_record(&mut out, stamp, addr, &d.lines, path.to_string_lossy().as_bytes());
        }
        code_load_record(&mut out, stamp, tid, addr, df.next, &name, &d.code);
        df.next += 1;
    }
    if failed.is_none()
        && let Err(e) = df.file.write_all(&out)
    {
        failed = Some(format!("write failed ({e})"));
    }
    if let Some(e) = failed {
        // A torn record would end the file for perf anyway.
        eprintln!("ROBIGO_PERF_MAP: jitdump {e}; no further records");
        *guard = None;
    }
}

/// The dump, created on first use as `jit-<pid>.dump` (the name `perf
/// inject` looks for) in a new directory under `$JITDUMPDIR`, or else
/// `~/.debug/jit` as perf's own agents do, with its header and the
/// executable mapping `perf record` sees it by.
fn dump_file() -> &'static Mutex<Option<DumpFile>> {
    static FILE: OnceLock<Mutex<Option<DumpFile>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let open = || -> Result<DumpFile, String> {
            let base = std::env::var_os("JITDUMPDIR")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".debug/jit")))
                .unwrap_or_else(std::env::temp_dir);
            std::fs::create_dir_all(&base).map_err(|e| format!("cannot create {}: {e}", base.display()))?;
            let template = base.join("robigo-XXXXXX");
            let dir = sys::mkdtemp(&template.to_string_lossy())
                .ok_or_else(|| format!("cannot create a directory in {}", base.display()))?;
            let dir = PathBuf::from(dir);
            let pid = std::process::id();
            let path = dir.join(format!("jit-{pid}.dump"));
            let mut file = create_private(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
            let mut header = Vec::with_capacity(40);
            put32(&mut header, 0x4A69_5444); // "JiTD", in the writer's byte order
            put32(&mut header, 1); // version
            put32(&mut header, 40); // header size
            put32(&mut header, ELF_MACH);
            put32(&mut header, 0);
            put32(&mut header, pid);
            put64(&mut header, sys::monotonic_ns());
            put64(&mut header, 0); // flags: timestamps are CLOCK_MONOTONIC
            file.write_all(&header).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            if !sys::map_exec_marker(&file) {
                return Err(format!("cannot map {}", path.display()));
            }
            Ok(DumpFile { file, dir, next: 0 })
        };
        Mutex::new(open().map_err(|e| eprintln!("ROBIGO_PERF_MAP: jitdump: {e}")).ok())
    })
}

fn put32(out: &mut Vec<u8>, x: u32) {
    out.extend_from_slice(&x.to_ne_bytes());
}

fn put64(out: &mut Vec<u8>, x: u64) {
    out.extend_from_slice(&x.to_ne_bytes());
}

/// A record's prefix; its size is patched in by [`end_record`].
fn begin_record(out: &mut Vec<u8>, id: u32, stamp: u64) -> usize {
    let start = out.len();
    put32(out, id);
    put32(out, 0);
    put64(out, stamp);
    start
}

fn end_record(out: &mut [u8], start: usize) {
    let size = (out.len() - start) as u32;
    out[start + 4..start + 8].copy_from_slice(&size.to_ne_bytes());
}

/// `JIT_CODE_DEBUG_INFO`: before the function's `JIT_CODE_LOAD`.
fn debug_info_record(out: &mut Vec<u8>, stamp: u64, addr: u64, lines: &[(u32, u32)], file: &[u8]) {
    let start = begin_record(out, JIT_CODE_DEBUG_INFO, stamp);
    put64(out, addr);
    put64(out, lines.len() as u64);
    for &(at, line) in lines {
        put64(out, addr + at as u64);
        put32(out, line);
        put32(out, 0); // discriminator
        out.extend_from_slice(file);
        out.push(0);
    }
    out.resize(start + (out.len() - start).next_multiple_of(8), 0);
    end_record(out, start);
}

/// `JIT_CODE_LOAD`: the code follows the name directly. (perf reads it
/// from the record's end, samply right after the name: no padding.)
fn code_load_record(out: &mut Vec<u8>, stamp: u64, tid: u32, addr: u64, index: u64, name: &str, code: &[u8]) {
    let start = begin_record(out, JIT_CODE_LOAD, stamp);
    put32(out, std::process::id());
    put32(out, tid);
    put64(out, addr); // vma
    put64(out, addr); // code_addr
    put64(out, code.len() as u64);
    put64(out, index);
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(code);
    end_record(out, start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_codegen::ir::MaybeRelSourceLoc;

    /// A stretch tagged with location `loc` (`None`: untagged).
    fn loc(start: u32, end: u32, loc: Option<u32>) -> MachSrcLoc {
        MachSrcLoc { start, end, loc: MaybeRelSourceLoc::abs(loc.map_or(SourceLoc::default(), SourceLoc::new)) }
    }

    #[test]
    fn untagged_code_counts_as_the_next_op() {
        // Prologue 0..4 untagged; glue 4..6; op 3 (line 4) at 6..10 and
        // again at 10..12; an untagged constant 12..14 for op 5 (line 6) at
        // 14..20; deopt glue 20..22; an untagged epilogue to 24.
        let s = [
            loc(0, 4, None),
            loc(4, 6, Some(0)),
            loc(6, 10, Some(4)),
            loc(10, 12, Some(4)),
            loc(12, 14, None),
            loc(14, 20, Some(6)),
            loc(20, 22, Some(0)),
            loc(22, 24, None),
        ];
        assert_eq!(line_table(&s, 24, 9), vec![(0, 9), (6, 4), (12, 6), (20, 9)]);
        // An op right after the prologue; code past the last tag.
        assert_eq!(line_table(&[loc(4, 8, Some(2))], 12, 9), vec![(0, 9), (4, 2), (8, 9)]);
        assert_eq!(line_table(&[], 8, 9), vec![(0, 9)]);
    }

    #[test]
    fn records_are_sized_and_code_follows_the_name() {
        let mut out = Vec::new();
        debug_info_record(&mut out, 7, 0x1000, &[(0, 1), (4, 2)], b"f.ops");
        let size = u32::from_ne_bytes(out[4..8].try_into().unwrap()) as usize;
        assert_eq!(size, out.len());
        assert_eq!(size % 8, 0);
        // header 16 + addr + count 16 + 2 entries of 16 + "f.ops\0"
        assert_eq!(size, (32 + 2 * (16 + 6) as usize).next_multiple_of(8));
        // The code right after the name, and nothing after the code.
        let mut out = Vec::new();
        let name = "robigo_loop_0#t1";
        code_load_record(&mut out, 7, 1, 0x1000, 0, name, &[0xc3; 5]);
        let size = u32::from_ne_bytes(out[4..8].try_into().unwrap()) as usize;
        assert_eq!(size, out.len());
        let code_size = u64::from_ne_bytes(out[40..48].try_into().unwrap()) as usize;
        let at = 56 + name.len() + 1;
        assert_eq!(&out[56..at], b"robigo_loop_0#t1\0");
        assert_eq!(&out[at..at + code_size], &[0xc3; 5]);
        assert_eq!(at + code_size, size);
    }
}
