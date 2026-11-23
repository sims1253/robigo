#!/usr/bin/env python3
"""Small seeds for the rtc target (fuzz/seeds/rtc): the first N entries of
a real translation cache, rewritten as a whole file of their own. A row's
checksum covers its key and its data only (crates/robigo-rt/src/tcache/
format.rs, keyed_checksum), so the kept entries stay valid; only their
offsets move.

    fuzz/rtc_seeds.py .robigo/rt-home-db-cache/compiler.rtc fuzz/seeds/rtc 1 4 16
"""
import os
import struct
import sys

ROW = 8 + 4 + 8 + 4 + 4 + 8


def parse(b):
    if b[:4] != b"RTC1":
        sys.exit("rtc_seeds: not a translation cache (no RTC1 magic)")
    at = 8
    idlen = b[at]
    at += 1 + idlen
    (stlen,) = struct.unpack_from("<I", b, at)
    at += 4 + stlen
    (n,) = struct.unpack_from("<I", b, at)
    head = b[:at]
    at += 4
    rows = []
    for i in range(n):
        off, ordinal, shape, dat, length, chk = struct.unpack_from("<QIQIIQ", b, at + i * ROW)
        rows.append((off, ordinal, shape, b[dat:dat + length], chk))
    return head, rows


def write(head, rows):
    out = bytearray(head)
    out += struct.pack("<I", len(rows))
    data_at = len(out) + len(rows) * ROW
    data = bytearray()
    for off, ordinal, shape, d, chk in rows:
        out += struct.pack("<QIQIIQ", off, ordinal, shape, data_at + len(data), len(d), chk)
        data += d
    return bytes(out + data)


def main():
    src, outdir, *counts = sys.argv[1:]
    head, rows = parse(open(src, "rb").read())
    for n in map(int, counts):
        # (the table is sorted by key: a prefix of it is sorted too)
        p = os.path.join(outdir, f"{os.path.basename(src)[:-4]}-{n}.rtc")
        with open(p, "wb") as f:
            f.write(write(head, rows[:n]))
        print(p, os.path.getsize(p))


if __name__ == "__main__":
    main()
