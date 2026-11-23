//! The translation cache's file format as untrusted input (C1,
//! crates/robigo-rt/src/tcache/format.rs): any bytes, read as a `.rtc`
//! file, decode to entries or to nothing, never a panic. The runtime half
//! (each entry's typed check and `Verified::new` before it runs, against a
//! real tree) is scripts/rt-tcache-mutate.py's.
//!
//!   cargo +nightly fuzz run rtc --fork 16 -max_len=65536

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = robigo_rt::translation_cache_fuzz_decode(data);
});
