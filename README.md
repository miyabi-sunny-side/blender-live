# blender-live

blender-live keeps one headless Blender running and lets a coding agent drive it over
[MCP](https://modelcontextprotocol.io/), while you watch the results in a browser.

- The agent uses the MCP server at `/mcp`. It runs Python in the resident Blender session and
  renders stills, and the PNG comes back in the tool result. Animations render as cancellable jobs.
- You use the web page at `/`. It lists renders newest first and plays videos. It also shows job
  progress, the Blender process state and the other base camp files for download. New renders
  appear without reloading.

The scene stays in memory between requests, so nothing pays for starting Blender or reloading
a `.blend` again. One Rust binary serves both windows and supervises Blender: if Blender dies,
it is restarted and the page and the `status` tool say whether unsaved work was lost.

## Requirements

- Linux x86_64
- Blender 5.2 (runs as `blender -b`; no display needed — EEVEE renders through EGL)
- An NVIDIA driver for Cycles on the GPU (OptiX, falling back to CUDA)
- `ffmpeg` on `PATH` for MP4 output

## Install

Download `blender-live-linux-x86_64.tar.gz` from the
[latest release](https://github.com/miyabi-sunny-side/blender-live/releases/latest), check it
against the `.sha256` file next to it and unpack it:

```sh
sha256sum -c blender-live-linux-x86_64.tar.gz.sha256
tar -xzf blender-live-linux-x86_64.tar.gz
```

The archive holds a static `blender-live` binary and the license. To build from source instead,
see [Development](#development).

## Run

```sh
blender-live ~/blender-camp
```

The only argument is the **base camp**, the directory blender-live works in. It is created with
these directories if missing:

| Directory | Holds |
| --- | --- |
| `scenes/` | `.blend` files |
| `assets/` | models, textures and audio |
| `scripts/` | Python scripts that rebuild scenes repeatably |
| `renders/` | stills (`name.png`), videos (`name.mp4`) and their frames (`name/frame_0001.png`) |

Open `http://<host>:5014/` in a browser. The server listens on all interfaces and has no
authentication, so run it only on a trusted network.

| Variable | Meaning | Default |
| --- | --- | --- |
| `PORT` | HTTP port | `5014` |
| `LOG_LEVEL` | `off`, `error`, `warn`, `info`, `debug` or `trace` | `info` |
| `BLENDER_BIN` | Blender executable | `blender` on `PATH` |
| `BLENDER_GPUS` | GPUs Cycles may use: `nvidia-smi` indices such as `0` or `0,1`, or `all` | `0,1` |

Blender's own messages appear in the log at `debug` level.

## MCP tools

All paths are relative to the base camp. A path that leaves it — `..`, an absolute path, or a
symlink that points outside — is refused. Python itself is not sandboxed.

| Tool | What it does |
| --- | --- |
| `python` | Runs Python with `bpy` in the resident session and returns the value of a final expression, stdout and, on failure, the traceback. Variables persist between calls. `timeout_secs` defaults to 60. |
| `run_script` | Runs a base camp Python file (usually `scripts/*.py`) in the same session. |
| `scene_summary` | Collections, objects with transforms, cameras, lights, frame range, fps, render engine, resolution and Cycles GPUs. |
| `open_blend`, `save_blend`, `new_blend` | Opens a `.blend`, saves (in place or as a new path), or starts from the startup or an empty scene. |
| `render_still` | Renders `renders/{name}.png` with optional camera, frame, size, engine (`cycles`, `eevee`, `workbench`) and samples. Returns the image and its file and viewer URLs. |
| `render_animation` | Starts a job that renders a frame range to `renders/{name}/frame_NNNN.png` and encodes `renders/{name}.mp4` (H.264), optionally with an audio file from the base camp. |
| `job_status`, `job_cancel` | Progress of the jobs; cancelling stops after the current frame and keeps the frames written so far. |
| `status` | Process state, GPUs, the open `.blend`, unsaved changes, restarts and what the last crash lost. |
| `list_files` | Lists `renders`, `scenes`, `scripts` or `assets`, newest first. |

While a job is rendering frames, the other Blender tools answer `busy` instead of waiting.

### Register with Claude Code

```sh
claude mcp add --transport http blender-live http://<host>:5014/mcp
```

### Register with Codex

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.blender-live]
url = "http://<host>:5014/mcp"
```

## Choosing GPUs

Cycles uses OptiX on every GPU listed in `BLENDER_GPUS`, numbered as `nvidia-smi` numbers them.
The default `0,1` renders on both cards of a two-GPU machine. Use one index to keep the other card
free for other work, or `all` to leave the choice to the driver. EEVEE renders on the GPU that
EGL picks and is not affected by this setting.

`status` and `scene_summary` show the devices Blender actually enabled.

## MMD models

PMX models import with the MMD Tools extension. Install it once for the user that runs
blender-live:

```sh
blender --online-mode --command extension install mmd_tools --enable
```

Then import from the `python` tool, for example:

```python
import bpy, os
bpy.ops.mmd_tools.import_model(filepath=os.path.join(os.getcwd(), "assets/model/model.pmx"), scale=0.08)
```

Blender runs with the base camp as its working directory. glTF and FBX import with Blender's
built-in operators. Keep model data in `assets/`; many model licenses forbid redistributing it.

## When Blender crashes

If Blender exits or a request outlives its timeout, blender-live kills it if needed, starts a new
one and reopens the last saved `.blend`. The failed call returns an error, and the page and the
`status` tool report the restart and whether the scene had unsaved changes that were lost.

## Development

Requires Rust 1.96 (selected by `rust-toolchain.toml`) and Node.js 24. The binary embeds the UI,
so build the client first:

```sh
npm --prefix client ci
npm --prefix client run build
cargo run -- /tmp/camp
```

Checks to run before committing:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm --prefix client run format:check
npm --prefix client run check
npm --prefix client test
npm --prefix client run test:e2e   # Chromium against the server with the stub Blender
```

Tests use [`tests/fake_blender.py`](tests/fake_blender.py), a stand-in that speaks the bridge
protocol without Blender. The bridge that runs inside Blender is [`src/bridge.py`](src/bridge.py).
Releases are built by pushing a `v*` tag; see [`.github/workflows/release.yml`](.github/workflows/release.yml).
The page design follows [`DESIGN.md`](DESIGN.md).

## License

[MIT](LICENSE). The bridge script is MIT as well, which is compatible with Blender's GPL; Blender
itself is not bundled.
