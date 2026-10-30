//! MCP tools over streamable HTTP at `/mcp`.
use crate::{
    AppState, files,
    naming::urls,
    protocol::{Engine, Op, Shot},
    render::{self, Animation, describe, ensure_idle},
};
use axum::Router;
use base64::Engine as _;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock as Content, Implementation, ServerCapabilities, ServerConfig,
    },
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Python {
    /// Python run in the resident Blender (`bpy` is imported). Variables persist
    /// between calls. The value of a final expression is returned.
    pub code: String,
    /// Seconds before the code is interrupted (default 60).
    pub timeout_secs: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// Base camp path of a Python file, e.g. `scripts/scene.py`.
    pub path: String,
    /// Seconds before the script is interrupted (default 600).
    pub timeout_secs: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlendPath {
    /// Base camp path of a `.blend`, e.g. `scenes/cube.blend`.
    pub path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePath {
    /// Base camp path to save as; omit to save the open file in place.
    pub path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewBlend {
    /// Start from an empty scene instead of the default cube, camera and light.
    #[serde(default)]
    pub empty: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Still {
    /// File stem under `renders/` (default `still-<UTC time>`). Reusing a name overwrites.
    pub name: Option<String>,
    /// Camera object name (default: the scene camera).
    pub camera: Option<String>,
    pub frame: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// cycles, eevee or workbench (default: the scene's engine).
    pub engine: Option<Engine>,
    pub samples: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Anim {
    /// Stem for `renders/{name}.mp4` and `renders/{name}/frame_NNNN.png` (default `clip-<UTC time>`).
    pub name: Option<String>,
    /// First frame (default: the scene's `frame_start`).
    pub frame_start: Option<i32>,
    /// Last frame, inclusive (default: the scene's `frame_end`).
    pub frame_end: Option<i32>,
    /// Video frame rate (default: the scene's fps).
    pub fps: Option<f64>,
    /// Base camp path of an audio file to put under the video, e.g. `assets/voice.wav`.
    pub audio: Option<String>,
    pub camera: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub engine: Option<Engine>,
    pub samples: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobId {
    pub id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaybeJobId {
    /// Omit to list every job, newest first.
    pub id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dir {
    /// renders, scenes, scripts or assets.
    pub dir: String,
}

#[derive(Clone)]
struct Tools {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

fn ok(value: &Value) -> CallToolResult {
    CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )])
}

fn fail(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.into())])
}

fn host(parts: &http::request::Parts) -> Option<&str> {
    parts.headers.get(http::header::HOST)?.to_str().ok()
}

#[tool_router]
impl Tools {
    fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    async fn run(&self, op: Op) -> CallToolResult {
        if let Err(busy) = ensure_idle(&self.state) {
            return fail(busy);
        }
        match self.state.blender.call(op, None).await {
            Ok(o) => ok(&json!({
                "value": o.value,
                "stdout": o.stdout,
                "file": o.file,
                "seconds": o.elapsed.as_secs_f64(),
            })),
            Err(e) => fail(describe(&e)),
        }
    }

    #[tool(
        description = "Run Python in the resident Blender session. The scene and variables stay between calls. Returns the value of a final expression, stdout and, on failure, the traceback."
    )]
    async fn python(&self, Parameters(a): Parameters<Python>) -> CallToolResult {
        self.run(Op::Exec {
            code: a.code,
            timeout_secs: a.timeout_secs.unwrap_or(60),
        })
        .await
    }

    #[tool(
        description = "Run a Python file from the base camp (usually scripts/) in the resident session, for repeatable scene building."
    )]
    async fn run_script(&self, Parameters(a): Parameters<Script>) -> CallToolResult {
        match self.state.camp().existing(&a.path) {
            Ok(path) => {
                self.run(Op::RunScript {
                    path: path.to_string_lossy().into_owned(),
                    timeout_secs: a.timeout_secs.unwrap_or(600),
                })
                .await
            }
            Err(e) => fail(format!("{}: {e}", a.path)),
        }
    }

    #[tool(
        description = "Summarize the scene: collections, objects with transforms, cameras, lights, frame range, fps, render engine, resolution and the Cycles GPUs."
    )]
    async fn scene_summary(&self) -> CallToolResult {
        self.run(Op::Summary).await
    }

    #[tool(description = "Open a .blend from the base camp, replacing the current scene.")]
    async fn open_blend(&self, Parameters(a): Parameters<BlendPath>) -> CallToolResult {
        match self.state.camp().existing(&a.path) {
            Ok(path) => {
                self.run(Op::Open {
                    path: path.to_string_lossy().into_owned(),
                })
                .await
            }
            Err(e) => fail(format!("{}: {e}", a.path)),
        }
    }

    #[tool(
        description = "Replace the current scene with Blender's startup scene, or an empty one."
    )]
    async fn new_blend(&self, Parameters(a): Parameters<NewBlend>) -> CallToolResult {
        self.run(Op::New { empty: a.empty }).await
    }

    #[tool(
        description = "Save the scene. With path (inside the base camp, usually scenes/*.blend) it saves as that file; without, it saves the open file."
    )]
    async fn save_blend(&self, Parameters(a): Parameters<SavePath>) -> CallToolResult {
        let path = match a.path.as_deref().map(|p| self.state.camp().writable(p)) {
            Some(Ok(path)) => Some(path.to_string_lossy().into_owned()),
            Some(Err(e)) => return fail(e.to_string()),
            None => None,
        };
        self.run(Op::Save { path }).await
    }

    #[tool(
        description = "Render a still PNG to renders/{name}.png and return the image plus its file and viewer URLs. Unset options keep the scene's values; Cycles uses the GPUs."
    )]
    async fn render_still(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(a): Parameters<Still>,
    ) -> CallToolResult {
        let shot = Shot {
            camera: a.camera,
            frame: a.frame,
            width: a.width,
            height: a.height,
            engine: a.engine,
            samples: a.samples,
        };
        match render::still(&self.state, a.name.as_deref(), shot).await {
            Ok(still) => {
                let (file_url, view_url) = urls(host(&parts), &still.path);
                let text = json!({
                    "path": still.path,
                    "file_url": file_url,
                    "view_url": view_url,
                    "seconds": still.seconds,
                    "render": still.info,
                });
                CallToolResult::success(vec![
                    Content::text(serde_json::to_string_pretty(&text).unwrap_or_default()),
                    Content::image(
                        base64::engine::general_purpose::STANDARD.encode(&still.png),
                        "image/png",
                    ),
                ])
            }
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "Start a background job that renders a frame range to renders/{name}/frame_NNNN.png and encodes renders/{name}.mp4 (H.264, optional audio). Returns the job; poll job_status, stop with job_cancel. Other Blender tools are refused while frames render."
    )]
    async fn render_animation(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(a): Parameters<Anim>,
    ) -> CallToolResult {
        let anim = Animation {
            name: a.name,
            frame_start: a.frame_start,
            frame_end: a.frame_end,
            fps: a.fps,
            audio: a.audio,
            shot: Shot {
                camera: a.camera,
                frame: None,
                width: a.width,
                height: a.height,
                engine: a.engine,
                samples: a.samples,
            },
        };
        match render::animation(&self.state, anim).await {
            Ok(job) => {
                let (file_url, view_url) = urls(host(&parts), &format!("renders/{}.mp4", job.name));
                ok(&json!({"job": job, "file_url": file_url, "view_url": view_url}))
            }
            Err(e) => fail(e),
        }
    }

    #[tool(description = "Progress of one animation job, or every job newest first.")]
    fn job_status(&self, Parameters(a): Parameters<MaybeJobId>) -> CallToolResult {
        let jobs = self.state.jobs.lock().expect("jobs lock");
        match a.id {
            Some(id) => match jobs.get(&id) {
                Some(job) => ok(&json!({"job": job, "total_frames": job.total()})),
                None => fail(format!("no job {id}")),
            },
            None => ok(&json!({"jobs": jobs.list()})),
        }
    }

    #[tool(
        description = "Cancel an animation job. Rendering stops after the current frame; frames already written stay."
    )]
    fn job_cancel(&self, Parameters(a): Parameters<JobId>) -> CallToolResult {
        match render::cancel(&self.state, &a.id) {
            Ok(job) => ok(&json!({"job": job})),
            Err(e) => fail(e.to_string()),
        }
    }

    #[tool(
        description = "The Blender process: state, pid, version, GPUs, open .blend, unsaved changes, restarts, last error and what the last crash lost."
    )]
    fn status(&self) -> CallToolResult {
        ok(&json!(self.state.blender.status()))
    }

    #[tool(description = "List renders, scenes, scripts or assets in the base camp, newest first.")]
    fn list_files(&self, Parameters(a): Parameters<Dir>) -> CallToolResult {
        match files::list(self.state.camp(), &a.dir) {
            Ok(entries) => ok(&json!({"entries": entries})),
            Err(e) => fail(e.to_string()),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "A resident headless Blender. Build scenes with python or run_script, look at \
                 them with render_still, keep reusable steps in scripts/ and .blend files in \
                 scenes/. Paths are relative to the base camp; paths outside it are refused.",
            )
    }
}

pub fn endpoint<S: Clone + Send + Sync + 'static>(state: &AppState) -> Router<S> {
    let tools = Tools::new(state.clone());
    let service = StreamableHttpService::new(
        move || Ok(tools.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().disable_allowed_hosts(),
    );
    Router::new().nest_service("/mcp", service)
}
