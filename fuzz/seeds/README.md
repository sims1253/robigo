# Seed corpora for the cargo-fuzz targets

Each directory seeds the target of its name (`fuzz/fuzz_targets/<target>.rs`).
They are kept apart from `fuzz/corpus/` (ignored), the corpus a run writes:
`cargo fuzz run T fuzz/corpus/T fuzz/seeds/T` reads both and adds only to
the first. CI replays them, and the nightly fuzzes from them (docs/verification.md,
"Fuzzing"). Every file comes from the repository or from what
`scripts/rt-home-db.sh` writes; none from anywhere else.

- `parse/`: the `tests/r` scripts of at most 1,500 bytes (447), minimized with
  `cargo +nightly-2026-11-02 fuzz cmin -O parse` (321 kept: 1,570 coverage
  edges, against 1,593 for all 632 scripts). A file's name is its path with
  `/` as `_`.
- `rds/`: the uncompressed `.rds` fixtures under `tests/r` and `tests/rt`
  (24), minimized with `cargo +nightly-2026-11-02 fuzz cmin -O rds` (20 kept).
  As in `parse/`, a file's name is its path with `/` as `_` (cmin names its
  output by content hash; each kept file was copied back from its source).
- `rtc/`: the first 1, 4 and 16 entries of a `compiler.rtc` written by
  `scripts/rt-home-db.sh` (at main 31f5a01b3), each a whole file of its own:
  `fuzz/rtc_seeds.py .robigo/rt-home-db-cache/compiler.rtc fuzz/seeds/rtc 1 4 16`.
  (A real cache is 0.2-1.9 MB; these keep the header, table and entry formats
  in 22 KB.)

A target's inputs change its corpus's value. After a format change,
regenerate that directory the way its line says.
