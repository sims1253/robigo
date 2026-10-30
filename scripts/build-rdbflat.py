#!/usr/bin/env python3
"""Build the flat (pre-inflated) cache for lazy-load databases.

Usage: build-rdbflat.py FETCHLOG FLATDIR

FETCHLOG is a file of lines "rdbpath<TAB>off<TAB>len<TAB>compressed"
enumerated by build-rdbflat-index.R (or ROBIGO_RDBLOG); each entry is
read from the rdb, inflated once here, and written to FLATDIR/<n>.flat
per source rdb (see crates/robigo-runtime/src/rdbflat.rs for the format).

The recorded `compressed` value follows R: serialize.c
do_lazyLoadDBfetch — NA (robigo logs NA_INTEGER, -2147483648) or 1
select R_decompress1 (a zlib stream after a 4-byte big-endian output
length), 3 R_decompress3 (the length, a type byte, then the stream:
'1' zlib, '0' stored, 'Z' raw LZMA2), 0 nothing.
"""
import os
import struct
import sys
import tempfile
import zlib

NA_INTEGER = -(2**31)


def publish(dest: str, image: bytes) -> None:
    # Readers mmap these files. Replacing the inode keeps existing
    # mappings valid while a cache is rebuilt; truncating it can SIGBUS.
    with tempfile.NamedTemporaryFile(dir=os.path.dirname(dest), delete=False) as f:
        tmp = f.name
        try:
            f.write(image)
            f.close()
            os.replace(tmp, dest)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)


def inflate_stream(buf: bytes, compressed: int) -> bytes:
    if compressed in (NA_INTEGER, 1):
        return zlib.decompress(buf[4:])
    if compressed == 3:
        t = buf[4:5]
        if t == b"0":
            return buf[5:]
        if t == b"1":
            return zlib.decompress(buf[5:])
        if t == b"Z":
            import lzma

            # R: connections.c R_decompress3 — raw LZMA2, preset 6
            dec = lzma.LZMADecompressor(
                format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA2, "preset": 6}]
            )
            return dec.decompress(buf[5:])
        raise SystemExit(f"unknown decompress3 type {t!r}")
    if compressed == 0:
        return buf
    raise SystemExit(f"unknown compressed flag {compressed!r}")


def inflate(buf: bytes, compressed: int) -> bytes:
    raw = inflate_stream(buf, compressed)
    if compressed != 0:
        if len(buf) < 4 or len(raw) != struct.unpack(">I", buf[:4])[0]:
            raise ValueError("uncompressed entry length mismatch")
    return raw


def main() -> None:
    log, flatdir = sys.argv[1], sys.argv[2]
    per_rdb: dict[str, dict[tuple[int, int], bytes]] = {}
    flags = {}
    stamps = {}
    for line in open(log):
        file, off, length, compressed = line.rstrip("\n").split("\t")
        d = per_rdb.setdefault(file, {})
        k = (int(off), int(length))
        flag = int(compressed)
        if file in flags and flags[file] != flag:
            raise ValueError(f"inconsistent compression flag: {file}")
        flags[file] = flag
        if k in d:
            continue
        with open(file, "rb") as f:
            st = os.fstat(f.fileno())
            stamp = (st.st_size, st.st_mtime_ns)
            if stamps.setdefault(file, stamp) != stamp:
                raise ValueError(f"database changed during enumeration: {file}")
            if k[0] < 0 or k[1] <= 0 or k[0] + k[1] > st.st_size:
                raise ValueError(f"entry outside database: {file} {k}")
            f.seek(k[0])
            buf = f.read(k[1])
        if len(buf) != k[1]:
            raise ValueError(f"truncated entry: {file} {k}")
        d[k] = inflate(buf, flag)
    os.makedirs(flatdir, exist_ok=True)
    for i, (file, entries) in enumerate(sorted(per_rdb.items())):
        st = os.stat(file)
        if stamps[file] != (st.st_size, st.st_mtime_ns):
            raise ValueError(f"database changed during build: {file}")
        path = file.encode()
        head = struct.pack("<8sQQQ", b"RFLAT01\n", st.st_size, st.st_mtime_ns, len(path)) + path
        data_start = len(head) + 8 + 24 * len(entries)
        index = bytearray()
        data = bytearray()
        pos = data_start
        for (off, clen), raw in sorted(entries.items()):
            index += struct.pack("<QIIQ", off, clen, len(raw), pos)
            data += raw
            pos += len(raw)
        dest = os.path.join(flatdir, f"{i:02d}-{os.path.basename(file)}.flat")
        publish(dest, head + struct.pack("<Q", len(entries)) + bytes(index) + bytes(data))
        print(f"build-rdbflat: {dest} ({len(entries)} entries, "
              f"{pos - data_start} bytes from {file})")
        # The database's .rdx index (same stem) is a gzip file that
        # lazyload.R's readRDS inflates whole at every open; cache that
        # too, as a single (0, 0) entry (rdbflat.rs serves it from the
        # gzfile open in builtins/io.rs).
        rdx = file[:-4] + ".rdx" if file.endswith(".rdb") else None
        if rdx and os.path.exists(rdx):
            import gzip

            with open(rdx, "rb") as f:
                raw = gzip.decompress(f.read())
            st = os.stat(rdx)
            path = rdx.encode()
            head = struct.pack("<8sQQQ", b"RFLAT01\n", st.st_size, st.st_mtime_ns, len(path)) + path
            body = struct.pack("<Q", 1) + struct.pack("<QIIQ", 0, 0, len(raw), len(head) + 8 + 24) + raw
            dest = os.path.join(flatdir, f"{i:02d}-{os.path.basename(rdx)}.flat")
            publish(dest, head + body)
            print(f"build-rdbflat: {dest} (rdx index, {len(raw)} bytes from {rdx})")


if __name__ == "__main__":
    main()
