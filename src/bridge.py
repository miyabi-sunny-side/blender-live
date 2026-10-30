"""Runs inside `blender -b` and serves blender-live one request at a time.

Requests and replies are JSON lines on a Unix socket. The scene stays in
memory between requests; the Rust side owns timeouts, restarts and paths.
"""

import ast
import contextlib
import io
import json
import os
import select
import signal
import socket
import sys
import time
import traceback

import bpy

ARGS = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
SOCKET = ARGS[ARGS.index("--socket") + 1]


class Conn:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(path)
        self.buf = b""

    def send(self, msg):
        self.sock.sendall(json.dumps(msg, default=str).encode() + b"\n")

    def line(self, timeout=None):
        """The next line, None on timeout; exits when blender-live goes away."""
        while b"\n" not in self.buf:
            if timeout is not None:
                ready, _, _ = select.select([self.sock], [], [], timeout)
                if not ready:
                    return None
            chunk = self.sock.recv(65536)
            if not chunk:
                os._exit(0)
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        return json.loads(line)


class Cancelled(Exception):
    pass


def setup_gpu():
    """Uses OptiX (else CUDA) on every visible NVIDIA GPU for Cycles."""
    prefs = bpy.context.preferences.addons["cycles"].preferences
    for kind in ("OPTIX", "CUDA"):
        try:
            prefs.compute_device_type = kind
        except TypeError:
            continue
        prefs.refresh_devices()
        gpus = [d for d in prefs.devices if d.type == kind]
        if gpus:
            for d in prefs.devices:
                d.use = d.type == kind
            return kind, [d.name for d in gpus]
    prefs.compute_device_type = "NONE"
    return None, []


DEVICE_TYPE, DEVICES = setup_gpu()
NAMESPACE = {"__name__": "__blender_live__", "bpy": bpy}


@contextlib.contextmanager
def time_limit(seconds):
    def expire(_signum, _frame):
        raise TimeoutError(f"stopped after {seconds} s (timeout_secs)")

    previous = signal.signal(signal.SIGALRM, expire)
    signal.alarm(max(1, int(seconds)))
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)


def jsonable(value):
    try:
        json.dumps(value)
        return value
    except (TypeError, ValueError):
        return repr(value)


def run_code(source, filename):
    """Executes like a REPL cell: the value of a final expression is returned."""
    tree = ast.parse(source, filename, "exec")
    last = None
    if tree.body and isinstance(tree.body[-1], ast.Expr):
        last = ast.Expression(tree.body.pop().value)
    exec(compile(tree, filename, "exec"), NAMESPACE)
    if last is not None:
        return eval(compile(last, filename, "eval"), NAMESPACE)
    return None


def op_exec(req):
    with time_limit(req["timeout_secs"]):
        return jsonable(run_code(req["code"], "<blender-live>"))


def op_run_script(req):
    path = req["path"]
    with open(path, encoding="utf-8") as f:
        source = f.read()
    NAMESPACE["__file__"] = path
    with time_limit(req["timeout_secs"]):
        return jsonable(run_code(source, path))


def summary(_req=None):
    scene = bpy.context.scene

    def vec(v):
        return [round(x, 4) for x in v]

    objects = []
    for o in scene.objects:
        item = {
            "name": o.name,
            "type": o.type,
            "location": vec(o.location),
            "rotation_euler": vec(o.rotation_euler),
            "scale": vec(o.scale),
            "collections": [c.name for c in o.users_collection],
        }
        if o.parent:
            item["parent"] = o.parent.name
        if o.type == "LIGHT":
            item["light"] = {"type": o.data.type, "energy": o.data.energy}
        if o.type == "CAMERA":
            item["camera"] = {"lens": o.data.lens, "type": o.data.type}
        objects.append(item)

    def tree(col):
        return {
            "name": col.name,
            "objects": len(col.objects),
            "children": [tree(c) for c in col.children],
        }

    r = scene.render
    return {
        "file": bpy.data.filepath or None,
        "scene": scene.name,
        "collections": tree(scene.collection),
        "objects": objects,
        "cameras": [o.name for o in scene.objects if o.type == "CAMERA"],
        "lights": [o.name for o in scene.objects if o.type == "LIGHT"],
        "active_camera": scene.camera.name if scene.camera else None,
        "frame_start": scene.frame_start,
        "frame_end": scene.frame_end,
        "frame_current": scene.frame_current,
        "fps": r.fps / r.fps_base,
        "engine": r.engine,
        "resolution": [r.resolution_x, r.resolution_y, r.resolution_percentage],
        "cycles_device": scene.cycles.device,
        "gpu": {"type": DEVICE_TYPE, "devices": DEVICES},
    }


def op_open(req):
    bpy.ops.wm.open_mainfile(filepath=req["path"])
    return summary()


def op_new(req):
    bpy.ops.wm.read_homefile(use_empty=req["empty"])
    return summary()


def op_save(req):
    path = req.get("path")
    if path:
        bpy.ops.wm.save_as_mainfile(filepath=path, check_existing=False)
    elif bpy.data.filepath:
        bpy.ops.wm.save_mainfile()
    else:
        raise ValueError("the scene has no file yet; pass a path under scenes/")
    return {"file": bpy.data.filepath}


def apply_shot(req):
    scene = bpy.context.scene
    r = scene.render
    if req.get("engine"):
        r.engine = req["engine"]
    if r.engine == "CYCLES" and DEVICE_TYPE:
        scene.cycles.device = "GPU"
    if req.get("width"):
        r.resolution_x = req["width"]
    if req.get("height"):
        r.resolution_y = req["height"]
    if req.get("width") or req.get("height"):
        r.resolution_percentage = 100
    if req.get("samples"):
        if r.engine == "CYCLES":
            scene.cycles.samples = req["samples"]
        else:
            scene.eevee.taa_render_samples = req["samples"]
    if req.get("camera"):
        camera = scene.objects.get(req["camera"])
        if camera is None or camera.type != "CAMERA":
            raise ValueError(f"no camera named {req['camera']!r}")
        scene.camera = camera
    if scene.camera is None:
        raise ValueError("the scene has no camera; add one or pass camera")
    r.image_settings.file_format = "PNG"


def render_to(path):
    bpy.context.scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


def op_render_still(req):
    apply_shot(req)
    scene = bpy.context.scene
    if req.get("frame") is not None:
        scene.frame_set(req["frame"])
    started = time.monotonic()
    render_to(req["output"])
    return {
        "seconds": round(time.monotonic() - started, 3),
        "engine": scene.render.engine,
        "frame": scene.frame_current,
        "camera": scene.camera.name,
        "resolution": [scene.render.resolution_x, scene.render.resolution_y],
    }


def op_render_frames(req, conn):
    apply_shot(req)
    scene = bpy.context.scene
    start, end = req["frame_start"], req["frame_end"]
    total = end - start + 1
    started = time.monotonic()
    for done, frame in enumerate(range(start, end + 1), 1):
        message = conn.line(timeout=0)
        if message is not None and message.get("op") == "cancel":
            raise Cancelled(f"cancelled after {done - 1} of {total} frames")
        scene.frame_set(frame)
        render_to(os.path.join(req["dir"], f"frame_{frame:04d}.png"))
        conn.send({"type": "progress", "id": req["id"], "done": done, "total": total})
    return {
        "seconds": round(time.monotonic() - started, 3),
        "frames": total,
        "fps": scene.render.fps / scene.render.fps_base,
        "engine": scene.render.engine,
    }


OPS = {
    "exec": op_exec,
    "run_script": op_run_script,
    "summary": summary,
    "open": op_open,
    "new": op_new,
    "save": op_save,
    "render_still": op_render_still,
}


def handle(req, conn):
    out = io.StringIO()
    reply = {"type": "reply", "id": req["id"], "ok": True, "value": None}
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            if req["op"] == "render_frames":
                reply["value"] = op_render_frames(req, conn)
            elif req["op"] == "cancel":
                reply["value"] = "nothing to cancel"
            else:
                reply["value"] = OPS[req["op"]](req)
    except Cancelled as e:
        reply.update(ok=False, error=str(e), cancelled=True)
    except BaseException as e:  # noqa: BLE001 - every failure goes back to the agent
        reply.update(
            ok=False,
            error=f"{type(e).__name__}: {e}",
            traceback=traceback.format_exc(),
        )
    reply["stdout"] = out.getvalue()
    reply["file"] = bpy.data.filepath or None
    conn.send(reply)


def main():
    conn = Conn(SOCKET)
    conn.send(
        {
            "type": "ready",
            "version": bpy.app.version_string,
            "devices": DEVICES,
            "device_type": DEVICE_TYPE,
        }
    )
    while True:
        handle(conn.line(), conn)


main()
