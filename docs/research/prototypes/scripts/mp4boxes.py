#!/usr/bin/env python3
"""Minimal MP4 box walker: prints top-level box order (to check faststart) and any
edit lists (elst) per track, plus the track handler type and timescale."""
import struct
import sys

CONTAINERS = {b"moov", b"trak", b"mdia", b"edts", b"minf", b"stbl", b"dinf"}


def walk(f, start, end, depth, out, ctx):
    pos = start
    while pos < end:
        f.seek(pos)
        hdr = f.read(8)
        if len(hdr) < 8:
            break
        size, typ = struct.unpack(">I4s", hdr)
        hlen = 8
        if size == 1:
            size = struct.unpack(">Q", f.read(8))[0]
            hlen = 16
        elif size == 0:
            size = end - pos
        if depth == 0:
            out["top"].append((typ.decode("latin1"), size))
        if typ == b"trak":
            ctx = {"handler": None, "timescale": None, "elst": None}
            out["tracks"].append(ctx)
        if typ == b"mdhd" and ctx is not None:
            f.seek(pos + hlen)
            ver = f.read(1)[0]
            f.read(3)
            if ver == 1:
                f.read(16)
                ctx["timescale"] = struct.unpack(">I", f.read(4))[0]
            else:
                f.read(8)
                ctx["timescale"] = struct.unpack(">I", f.read(4))[0]
        if typ == b"hdlr" and ctx is not None:
            f.seek(pos + hlen + 8)
            ctx["handler"] = f.read(4).decode("latin1")
        if typ == b"elst" and ctx is not None:
            f.seek(pos + hlen)
            ver = f.read(1)[0]
            f.read(3)
            n = struct.unpack(">I", f.read(4))[0]
            entries = []
            for _ in range(n):
                if ver == 1:
                    dur, mtime = struct.unpack(">Qq", f.read(16))
                else:
                    dur, mtime = struct.unpack(">Ii", f.read(8))
                rate = struct.unpack(">hh", f.read(4))
                entries.append({"segment_duration_movie_ts": dur, "media_time": mtime, "rate": rate[0]})
            ctx["elst"] = entries
        if typ in CONTAINERS:
            walk(f, pos + hlen, pos + size, depth + 1, out, ctx)
        pos += size


def main(path):
    out = {"top": [], "tracks": []}
    with open(path, "rb") as f:
        f.seek(0, 2)
        end = f.tell()
        walk(f, 0, end, 0, out, None)
    order = [t for t, _ in out["top"]]
    print(f"file: {path}")
    print("top-level boxes:", order)
    if "moov" in order and "mdat" in order:
        print("faststart (moov before mdat):", order.index("moov") < order.index("mdat"))
    for t in out["tracks"]:
        ts = t["timescale"]
        el = t["elst"]
        if el:
            desc = []
            for e in el:
                mt = e["media_time"]
                secs = (mt / ts) if (ts and mt >= 0) else mt
                desc.append(f"media_time={mt} ({secs if isinstance(secs, int) else round(secs, 4)} s) rate={e['rate']}")
            print(f"track {t['handler']}: timescale={ts} elst=[{'; '.join(desc)}]")
        else:
            print(f"track {t['handler']}: timescale={ts} elst=none")


if __name__ == "__main__":
    for p in sys.argv[1:]:
        main(p)
