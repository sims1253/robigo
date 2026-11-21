Files GNU R 4.6.2 saved, for the load cases (robigo-rt has no `save`
yet): `x`, `y` and `f` in format 2 and 3 (gzip), `x` and `y` in ASCII and
uncompressed, an object with an old-style S4 class, and an
empty save. `dimnames-list.rda` is a list with 1-d dimnames and no names,
serialized after an RDX3 magic number.

`corrupt.rdb` is a lazy-load database of four compressed records for
`lazyload-corrupt.R` (robigo-rt has no `writeBin` yet): one good, one whose
stream is bad data, one whose header promises 10 bytes fewer than the
stream inflates to, and one whose header promises 100 more. It is written by
`make-corrupt-rdb.R`, run by GNU R 4.6.2 in this directory, which prints the
records' keys.
