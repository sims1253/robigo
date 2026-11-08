//! In-process, coverage-guided fuzzing of `readRDS()`/`unserialize()`.
//!
//! scripts/fuzz.py mutates an uncompressed RDS file and runs the whole
//! binary per input; this harness feeds raw bytes straight into
//! `robigo_runtime::serialize::unserialize` in one process, with coverage.
//! A bare `Interp` is enough: unserialize resolves symbols lazily and a
//! malformed stream must end in `RResult::Err`, never a panic.
//!
//!   cargo +nightly fuzz run rds --fork 16 -max_len=65536

#![no_main]

use libfuzzer_sys::fuzz_target;
use robigo_runtime::interp::Interp;
use robigo_runtime::serialize;
use robigo_runtime::value::Value;

fuzz_target!(|data: &[u8]| {
    let mut interp = Interp::new();
    let _ = serialize::unserialize(&mut interp, data, Value::Null);
});
