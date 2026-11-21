# The gzip connection inflates in blocks (2026-11-21)

X/P1, item #1 of the start-up plan for the rdb boot
(`~/.cache/robigo-research/x-boot/PLAN.md`). The code is L's
(`crates/robigo-rt/src/connections.rs`, `GzReader`).

## What was slow

`readRDS(base.rdx)` unserializes from a gzip connection, and unserialize
asks for the bytes it needs, often 4 at a time. `GzReader::read_full`
inflated straight into each such request and ran a CRC-32 over each one's
few bytes: 7,176 inflate calls and 7,176 small CRCs, 7.96M instructions of
the boot's 66.7M (the CRC alone 3.16M: zlib-rs' `crc32_braid` copies short
inputs). GNU R's `R_gzread` (gzio.h) also inflates per call, and spends
5.5M on the same reads.

## The change

The reader already took its compressed input through a buffer of gzio.h's
`Z_BUFSIZE`. It now inflates its output a `Z_BUFSIZE` block at a time too,
serves small reads from that block and runs the CRC once per block. A read
of `Z_BUFSIZE` or more still inflates in place.

Reading ahead must not move an error to another read. Sol's three rounds
found where it did, and each is fixed:
- **The boundary.** A block filled ahead keeps the good bytes it inflated
  and the error it met after them. The read that reaches that boundary
  meets the error, with nothing copied, as it would have inflating in place:
  - a member's trailer is checked when its last byte comes out;
  - inflate goes on to the next block header or symbol, and so to a bad
    one, with the output full.
- **The error sticks.** Every later read meets an error again. For a bad
  deflate stream, the inflater is asked again, so the message is its own
  for a repeated call ("repeated call with bad state"), as before; a
  trailer's error is given again as it was.
- **An empty read** is `Ok(0)`, after an error too.

GNU R itself only warns on a bad CRC ("invalid or incomplete compressed
data", gzio.h:348-350), where robigo-rt raises an error. That difference
predates this change and is left as it was.

## Tests

- `gz_reader_tests` (connections.rs) checks three things: the bytes under
  small, exact and large read patterns across concatenated members; a bad
  CRC met by the read reaching the end, then by every later read; and a bad
  block met at its boundary, not before, with an empty read unaffected.
- Corrupt files, made outside R and run on main's binary and this one:
  flipped CRC, a bad block after padding, a bad block at the exact boundary
  and after a valid member, and three `readLines()` on each, through
  `gzfile` and `gzcon`. The results, error messages and warnings are
  identical. robigo-rt cannot write such files itself: it has no
  `readBin`/`writeBin` yet.
- `~/.cache/robigo-research/x-boot/gz-conn.R`, recorded on GNU R 4.6.2:
  three objects unserialized in a row from one gzfile connection,
  `readRDS`, `readLines` with `pushBack`. It gives the same output before
  and after, and GNU R's where robigo-rt implements the calls (it lacks
  gzfile `seek`, `readBin`, `tempdir`, `unlink`).

## Measured

On main `43c42fca`, binaries built from their exact trees into empty
targets, one database tree (`scripts/rt-home-db.sh`):

| | main | this |
|---|---:|---:|
| rdb boot (`ROBIGO_RT_BOOT=rdb --vanilla -e 1`) | 66,671,206 | 59,961,927 (−10.1%) |
| the gzip connection's reads (`conn_read_raw`) | 7.96M | 1.25M |
| peak RSS, median of 5 | 13,176 KiB | 13,252 KiB |

The RSS difference (+76 KiB) is the block buffer plus noise. perf-rt ran
twice per binary: every row is within ±0.07%, with geomean 1.0000. The
per-compile jit rows are within ±0.05%.

**Checks at `702059d7`:**
- debug `cargo test -p robigo-rt`: 146 pass;
- `cargo fmt --check`, `scripts/test-rt-ci.py` and `scripts/lint.sh` pass;
- `scripts/rt-ci.py` passes all but its `ROBIGO_RETRY_TORTURE=1`
  conformance step, where `bytecode/cmpfun-ops` timed out: one of main's
  four known timeouts there, which L moves out.

Sol (gpt-6.1-sol, high), three rounds:
- round 1: S1 (CRC) and S2 (deflate error) fixed;
- round 2: N1 (exact boundary) and N2 (sticky error) fixed;
- round 3: M1 (the repeated-call message) and M2 (an empty read) fixed,
  and checked against main's binary rather than in a fourth round.
