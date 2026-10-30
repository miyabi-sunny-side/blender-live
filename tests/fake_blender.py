#!/usr/bin/env python3
"""A stand-in for `blender` that speaks the bridge protocol without bpy."""

import ast
import contextlib
import io
import json
import os
import select
import socket
import struct
import sys
import time
import traceback
import zlib

args = sys.argv[sys.argv.index("--") + 1 :]
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(args[args.index("--socket") + 1])
buf = b""
ns = {}
file = None


def send(msg):
    sock.sendall(json.dumps(msg).encode() + b"\n")


def line(timeout=None):
    global buf
    while b"\n" not in buf:
        if timeout is not None and not select.select([sock], [], [], timeout)[0]:
            return None
        chunk = sock.recv(65536)
        if not chunk:
            sys.exit(0)
        buf += chunk
    raw, buf = buf.split(b"\n", 1)
    return json.loads(raw)


def png(shade, size=16):
    """A small grey PNG, valid enough for ffmpeg and browsers."""

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    rows = b"".join(b"\0" + bytes([shade]) * size for _ in range(size))
    header = struct.pack(">IIBBBBB", size, size, 8, 0, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def run(code):
    tree = ast.parse(code)
    last = tree.body.pop().value if tree.body and isinstance(tree.body[-1], ast.Expr) else None
    exec(compile(tree, "<fake>", "exec"), ns)
    return eval(compile(ast.Expression(last), "<fake>", "eval"), ns) if last else None


print("fake blender starting", os.environ.get("CUDA_VISIBLE_DEVICES"), flush=True)
send({"type": "ready", "version": "fake", "devices": ["Fake GPU"], "device_type": "OPTIX"})
while True:
    req = line()
    reply = {"type": "reply", "id": req["id"], "ok": True, "value": None}
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out):
            op = req["op"]
            if op == "exec":
                reply["value"] = run(req["code"])
            elif op in ("open", "save"):
                file = req.get("path") or file
                reply["value"] = {"file": file}
            elif op == "new":
                file = None
            elif op == "render_still":
                with open(req["output"], "wb") as f:
                    f.write(png(128))
                reply["value"] = {"seconds": 0.0}
            elif op == "render_frames":
                total = req["frame_end"] - req["frame_start"] + 1
                for done, frame in enumerate(range(req["frame_start"], req["frame_end"] + 1), 1):
                    msg = line(timeout=0)
                    if msg and msg["op"] == "cancel":
                        reply.update(ok=False, error="cancelled", cancelled=True)
                        break
                    with open(os.path.join(req["dir"], f"frame_{frame:04d}.png"), "wb") as f:
                        f.write(png(frame * 5 % 256))
                    send({"type": "progress", "id": req["id"], "done": done, "total": total})
                    time.sleep(0.05)
                else:
                    reply["value"] = {"frames": total, "fps": 24.0}
            else:
                reply["value"] = {"op": op}
    except BaseException as e:
        reply.update(ok=False, error=f"{type(e).__name__}: {e}", traceback=traceback.format_exc())
    reply["stdout"] = out.getvalue()
    reply["file"] = file
    send(reply)
