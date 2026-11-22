# Vendored libdeflate (decompression only)

These files are copied unmodified from libdeflate's release tarball and are
distributed under its MIT licence (`COPYING`, kept here), which is compatible
with robigo's GPL-2.0-or-later. Copyright (C) 2016 Eric Biggers, 2024 Google
LLC, and contributors as noted in each file.

GNU R 4.6.2 decompresses the lazy-load databases' records with libdeflate
when it is built with it (`R_decompress1`, src/main/connections.c,
`HAVE_LIBDEFLATE`). robigo-rt does the same (`crates/robigo-rt/src/serialize/
inflate.rs`; compiled by `crates/robigo-rt/build.rs`), so the results and
the "internal error N in R_decompress1 with libdeflate" codes are
libdeflate's own.

- Upstream: https://github.com/ebiggers/libdeflate
- Version: 1.26.
  - The 4.6.2 oracle image's GNU R links 1.23 (`extSoftVersion()["libdeflate"]`).
    1.23 does not build with GCC 16 (its x86 Adler-32 uses the
    `target("no-evex512")` attribute GCC 16 removed; 1.26's NEWS: "Fixed a
    build error with gcc 16").
  - The decompressor is the same in both: between 1.23 and 1.26,
    `libdeflate.h`'s `enum libdeflate_result` and the three functions'
    declarations are identical, `lib/zlib_decompress.c` is identical, and
    `lib/deflate_decompress.c` differs only by `restrict` qualifiers on its
    dispatcher's parameters.
  - A future port of `R_compress1` (libdeflate's compressor) would need the
    oracle's version for byte-identical records, since compressors may
    change their output between releases.
- Tarball: https://github.com/ebiggers/libdeflate/releases/download/v1.26/libdeflate-1.26.tar.gz
- sha256: `125856d4656e0feab660f94842f835923410c9281fedbcee64598c918da42b5a`

## Files

Only what the zlib decompressor needs. Paths are as in the tarball:

| file | why |
|---|---|
| `COPYING` | the licence |
| `libdeflate.h` | the public API: the three functions robigo-rt calls |
| `common_defs.h` | compiler and type definitions every source includes |
| `lib/lib_common.h` | the library's internal definitions |
| `lib/deflate_constants.h` | the DEFLATE format's constants |
| `lib/zlib_constants.h` | the zlib wrapper's constants |
| `lib/decompress_template.h` | the decompressor's body, instantiated per CPU variant |
| `lib/cpu_features_common.h` | the runtime CPU-feature detection's shared part |
| `lib/deflate_decompress.c` | the DEFLATE decompressor: `libdeflate_alloc_decompressor`, `libdeflate_free_decompressor` |
| `lib/zlib_decompress.c` | `libdeflate_zlib_decompress`: the zlib header, the stream, its Adler-32 |
| `lib/adler32.c` | the Adler-32 checksum the zlib trailer carries |
| `lib/utils.c` | the allocator hooks (`malloc`/`free`) the decompressor uses |
| `lib/x86/cpu_features.c`, `lib/x86/cpu_features.h` | x86's CPU-feature detection |
| `lib/x86/decompress_impl.h` | x86's BMI2 decompressor variant |
| `lib/x86/adler32_impl.h`, `lib/x86/adler32_template.h` | x86's SIMD Adler-32 variants |
| `lib/arm/cpu_features.c`, `lib/arm/cpu_features.h`, `lib/arm/adler32_impl.h` | the same for aarch64 (compiled to nothing on x86) |

Local changes: none. A change, if one is ever needed, goes in a patch file
beside these files, never into them.

## CPU variants and the perf rows

libdeflate picks its decompressor and Adler-32 at the first call, from the
CPU's features:
- the decompressor: a BMI2 variant, or a generic one;
- Adler-32: AVX-512 VNNI, AVX-VNNI, AVX2 or SSE2.

Under valgrind the guest's CPU is valgrind's, which has no AVX-512 or
AVX-VNNI. So on any AVX2 host callgrind runs `deflate_decompress_bmi2` and
`adler32_x86_avx2`, the pair the perf rows were measured with
(2026-11-21, AMD Ryzen 9 5900X). A host without AVX2 or BMI2 would run
other variants and count other instructions for the rdb boot's
decompression. Outside valgrind the choice only changes speed.

## Updating

1. Download the new release's tarball.
2. Set `VERSION` and `SHA256` in `scripts/vendor-libdeflate.sh` and update
   this README's version and sha256.
3. Run `scripts/vendor-libdeflate.sh PATH/TO/libdeflate-X.Y.tar.gz`, which
   checks the sha256 and copies the files above.
4. Rebuild; check that the list above is still complete (the build fails on
   a missing header).
5. Re-measure the rdb boot and the perf rows, and run the lazy-load cases.
