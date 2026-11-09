//! In-process, coverage-guided fuzzing of the R parser.
//!
//! scripts/fuzz.py runs the same input class through the whole binary, one
//! process per input; this harness keeps one process and feeds libFuzzer's
//! coverage back into the search, so it reaches the deep paths of gram.y's
//! ports orders of magnitude faster. It runs local, never in CI.
//!
//!   cargo +nightly fuzz run parse --fork 16 -max_len=65536
//!
//! The parser takes `&str`, as `parse()` does, so invalid UTF-8 is lossily
//! converted like the CLI does. A stack overflow, panic or crash is a
//! finding (libFuzzer saves the input); `--fork` keeps exploring after one.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let arena = robigo_syntax::Arena::new();
    let _ = robigo_syntax::parse_program(&arena, &src);
});
