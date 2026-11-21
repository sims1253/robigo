# R_decompress1 on libdeflate, GNU R's own decoder (2026-11-21)

X/P1, the foreman's decision (B) of the decoder costing: our own binding
over vendored libdeflate. The interpreter of `decompress1` is L's.

## What changed

- **What GNU R does.** GNU R 4.6.2 built with libdeflate (the oracle image's
  is: libdeflate 1.23) decompresses the lazy-load databases' records in
  `R_decompress1` (connections.c:6647) with
  `libdeflate_zlib_decompress(d, p + 4, inlen - 4, buf, outlen, &actual_out)`,
  one decompressor in a function-local `static`.
- **What robigo-rt did.** It inflated with zlib-rs through flate2 (#958's
  reused inflater) and mapped flate2's statuses onto libdeflate's error
  numbers.
- **What it does now.** It calls libdeflate itself:
  - `vendor/libdeflate/`: libdeflate 1.26's decompression sources,
    unmodified, MIT. The README gives the URL, version, sha256, each file
    and why it is needed, and why 1.26 and not the oracle's 1.23 (1.23 does
    not build with GCC 16; the decompressor is the same in both).
    `scripts/vendor-libdeflate.sh` reproduces the directory from the
    tarball.
  - `crates/robigo-rt/build.rs` compiles them with `cc`, at upstream's
    release flags (`-O2 -DNDEBUG`), fixed rather than taken from the
    profile.
  - `serialize/inflate.rs`: a safe `Decompressor` over the three C
    functions, each `unsafe` block with its SAFETY comment, under the
    module's `#[allow(unsafe_code)]`.
  - `decompress1`: the decompressor is per-isolate state (`RdbInflater`,
    replacing #958's flate2 one). GNU R's is a process static; ours is the
    isolate's because all mutable runtime state is (g3.md §11), and isolates
    run on threads of their own.
  - The output buffer is zeroed, as before. An uninitialized one would save
    about 0.09M of the boot and is not worth an extra unsafe invariant.

## What R sees

- A record whose stream is bad data: the warning "internal error 1 in
  R_decompress1 with libdeflate" (`LIBDEFLATE_BAD_DATA`), then
  lazyLoadDBfetch's "lazy-load database '…' is corrupt".
- A header promising fewer bytes than the stream inflates to: the same,
  with 3 (`LIBDEFLATE_INSUFFICIENT_SPACE`).
- A header promising more: a success, with the stream's own bytes
  (`actual_out`).
  - GNU R passes a non-NULL `actual_out`, so libdeflate never returns
    `LIBDEFLATE_SHORT_OUTPUT` (2) there.
  - L's queued "internal error 2" item is therefore void.
  - The bot's low on #958 (the capacity invariant of `decompress_vec`)
    goes with flate2 on this path.
- `tests/rt/serialize/lazyload-corrupt.R` checks all three, plus a good
  record before and after, against GNU R 4.6.2's recording. It reads
  `data/corrupt.rdb`, which `data/make-corrupt-rdb.R` wrote on GNU R.

## Numbers

Main 60f1f4fd1 against 88dc28387 (this branch, main merged in). Both were
built into empty targets:

- `robigo-rt-main-60f1f4fd`, sha256 `fecdb14084a350b6011cb682e49f483ef8f51a93b2fa630f35e74b961543c967`;
- `robigo-rt-ld-88dc2838`, sha256 `92a4c14b1f66fe0950f082078785b5eba00d7ffa165b60cfb5920f710c0050ef`.

The boot is `--vanilla -e 1` under callgrind, the rdb boot from one
base-database tree written by main's binary:

| boot | main | this | change |
|---|---:|---:|---:|
| `ROBIGO_RT_BOOT=rdb` | 54,757,662 | 52,430,557 | −2,327,105 (−4.25%) |
| interim (M1) loader | 17,112,576 | 17,113,309 | +733 (no record is inflated) |

- **The decoding:** 4.42M in zlib-rs's `decompress_vec` became 2.02M in
  `libdeflate_zlib_decompress` for the 75 records (about 59k → 27k a record).
  `do_lazyload_dbfetch` with everything it calls went from 14.24M to
  11.95M.
- **Anonymous memory after the boot:** −28 KiB under rdb, +36 KiB under the
  interim boot. Peak RSS and file-backed memory move with code layout. The
  binary is 173 KB larger.
- **perf-rt (`scripts/rt-bars.sh`, median of two runs each):** every row is
  within ±0.01% instructions under both boots.
- **CPU variants under valgrind:** callgrind runs `deflate_decompress_bmi2`
  and `adler32_x86_avx2`. Valgrind's CPU has neither AVX-512 nor AVX-VNNI,
  so any AVX2 host picks that pair (vendor/libdeflate/README.md).
- **The earlier prototype** over the `libdeflater` crate measured −2.34M on
  the same main, the same decoder.

## Build, CI, Miri

- CI's rust image is Debian trixie, with GCC 14.2. The vendored sources
  compile with it at `build.rs`'s flags, checked in the 4.6.2 oracle image,
  which has the same GCC.
- Miri runs robigo-runtime's `value::` tests, robigo-rtcore and robigo-heap
  (ci.yml's miri job). None of them depends on robigo-rt, so the binding is
  never compiled under Miri, and no pure-Rust fallback is needed.
- There is no fuzz target for robigo-rt's fetches: `fuzz/fuzz_targets/rds.rs`
  drives robigo-runtime's unserialize.

## R_compress1

- **What robigo-rt compresses with:** zlib-rs through flate2 at level 6
  (`serialize::compress1`).
- **What GNU R compresses with:** built with libdeflate, it uses
  `libdeflate_zlib_compress` at level 6 (connections.c:6613).
- **So the compressed bytes differ by construction:** two encoders. L's
  record-by-record comparison (scripts/rt-rdb-compare.R) compares the
  records *decompressed*, which is where the identity holds: of the
  serialization, not of the compression.
- **The follow-up, if wanted:** byte-identical compressed records would
  need libdeflate's compressor at the oracle's version (1.23). That means
  vendoring `deflate_compress.c` and its matchfinders, with a patch for GCC
  16, or a newer oracle. It is not in this change.
