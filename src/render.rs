//! Render requests that write into `renders/`: stills and animation jobs.
use crate::{
    AppState,
    blender::{CallError, Outcome},
    encode::{Encode, ffmpeg_args},
    events::Event,
    job::{Job, JobError, Phase},
    naming::output_name,
    protocol::{Op, Shot},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use time::OffsetDateTime;
use tokio::sync::watch;
use tracing::warn;

pub struct Still {
    /// Base camp path of the PNG.
    pub path: String,
    pub png: Vec<u8>,
    pub seconds: f64,
    pub info: Value,
}

/// Refuses Blender requests while an animation job is rendering frames.
pub fn ensure_idle(state: &AppState) -> Result<(), String> {
    let jobs = state.jobs.lock().expect("jobs lock");
    match jobs.active() {
        Some(job) if job.phase == Phase::Rendering => Err(format!(
            "busy: job {} is rendering frame {}/{}; wait for it or call job_cancel",
            job.id,
            job.done_frames,
            job.total()
        )),
        _ => Ok(()),
    }
}

pub async fn still(state: &AppState, name: Option<&str>, shot: Shot) -> Result<Still, String> {
    ensure_idle(state)?;
    let name =
        output_name(name, "still", "png", OffsetDateTime::now_utc()).map_err(|e| e.to_string())?;
    let rel = format!("renders/{name}.png");
    let abs = state.camp().writable(&rel).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let Outcome { value, .. } = state
        .blender
        .call(
            Op::RenderStill {
                output: abs.to_string_lossy().into_owned(),
                shot,
            },
            None,
        )
        .await
        .map_err(|e| describe(&e))?;
    let png = tokio::fs::read(&abs)
        .await
        .map_err(|e| format!("render finished but {rel} is unreadable: {e}"))?;
    Ok(Still {
        path: rel,
        png,
        seconds: started.elapsed().as_secs_f64(),
        info: value,
    })
}

pub struct Animation {
    pub name: Option<String>,
    pub frame_start: Option<i32>,
    pub frame_end: Option<i32>,
    pub fps: Option<f64>,
    /// Base camp path of an audio track to put under the video.
    pub audio: Option<String>,
    pub shot: Shot,
}

/// Starts an animation job: frames into `renders/{name}/`, then
/// `renders/{name}.mp4`. Returns as soon as the job is registered.
pub async fn animation(state: &AppState, a: Animation) -> Result<Job, String> {
    ensure_idle(state)?;
    let name = output_name(a.name.as_deref(), "clip", "mp4", OffsetDateTime::now_utc())
        .map_err(|e| e.to_string())?;
    let camp = state.camp();
    let mp4 = camp
        .writable(&format!("renders/{name}.mp4"))
        .map_err(|e| e.to_string())?;
    let frames = camp
        .writable(&format!("renders/{name}/frame_0001.png"))
        .map_err(|e| e.to_string())?
        .parent()
        .expect("frame paths have a parent")
        .to_path_buf();
    let audio = match &a.audio {
        Some(rel) => Some(
            camp.existing(rel)
                .map_err(|e| format!("audio {rel}: {e}"))?,
        ),
        None => None,
    };
    clear_frames(&frames).map_err(|e| format!("cannot clear old frames: {e}"))?;

    let range = if a.frame_start.is_none() || a.frame_end.is_none() {
        let summary = state
            .blender
            .call(Op::Summary, None)
            .await
            .map_err(|e| describe(&e))?
            .value;
        let get = |key| summary[key].as_i64().and_then(|v| i32::try_from(v).ok());
        (get("frame_start"), get("frame_end"))
    } else {
        (None, None)
    };
    let frame_start = a.frame_start.or(range.0).unwrap_or(1);
    let frame_end = a.frame_end.or(range.1).unwrap_or(frame_start);

    let id = format!(
        "{:x}",
        OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
    );
    let created_ms = i64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
        .unwrap_or_default();
    let job = Job::new(id.clone(), name.clone(), frame_start, frame_end, created_ms)
        .map_err(|e| e.to_string())?;
    state
        .jobs
        .lock()
        .expect("jobs lock")
        .start(job.clone())
        .map_err(|e| e.to_string())?;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    state
        .cancels
        .lock()
        .expect("cancels lock")
        .insert(id.clone(), cancel_tx);
    publish_jobs(state);

    tokio::spawn(run(
        state.clone(),
        Plan {
            id,
            name,
            frames,
            mp4,
            audio,
            frame_start,
            frame_end,
            fps: a.fps,
            shot: a.shot,
        },
        cancel_rx,
    ));
    Ok(job)
}

struct Plan {
    id: String,
    name: String,
    frames: PathBuf,
    mp4: PathBuf,
    audio: Option<PathBuf>,
    frame_start: i32,
    frame_end: i32,
    fps: Option<f64>,
    shot: Shot,
}

/// Renders the frames, encodes the MP4 and records each step on the job.
async fn run(state: AppState, p: Plan, cancel: watch::Receiver<bool>) {
    let id = p.id.as_str();
    let op = Op::RenderFrames {
        dir: p.frames.to_string_lossy().into_owned(),
        frame_start: p.frame_start,
        frame_end: p.frame_end,
        shot: p.shot,
    };
    let progress_state = state.clone();
    let progress_id = p.id.clone();
    let result = state
        .blender
        .call_with_progress(op, Some(cancel.clone()), move |done, _| {
            update(&progress_state, &progress_id, |j| j.frame_rendered(done));
        })
        .await;
    match result {
        Ok(outcome) => {
            update(&state, id, Job::frames_finished);
            let encode = Encode {
                frames_dir: &p.frames,
                frame_start: p.frame_start,
                fps: p
                    .fps
                    .or_else(|| outcome.value["fps"].as_f64())
                    .unwrap_or(24.0),
                audio: p.audio.as_deref(),
                output: &p.mp4,
            };
            match run_ffmpeg(&encode, cancel).await {
                Ok(true) => update(&state, id, |j| {
                    j.encoded(vec![
                        format!("renders/{}.mp4", p.name),
                        format!("renders/{}", p.name),
                    ])
                }),
                Ok(false) => update(&state, id, |j| j.cancelled(kept(&p.name, j))),
                Err(e) => update(&state, id, |j| j.failed(e)),
            }
        }
        Err(CallError::Failed {
            cancelled: true, ..
        }) => update(&state, id, |j| j.cancelled(kept(&p.name, j))),
        Err(e) => update(&state, id, |j| j.failed(describe(&e))),
    }
    state.cancels.lock().expect("cancels lock").remove(id);
}

/// The frame directory, when the cancelled job wrote any frames.
fn kept(name: &str, job: &Job) -> Vec<String> {
    if job.done_frames > 0 {
        vec![format!("renders/{name}")]
    } else {
        Vec::new()
    }
}

pub fn cancel(state: &AppState, id: &str) -> Result<Job, JobError> {
    let job = state
        .jobs
        .lock()
        .expect("jobs lock")
        .update(id, |j| j.request_cancel().map(|()| j.clone()))?;
    if let Some(tx) = state.cancels.lock().expect("cancels lock").get(id) {
        let _ = tx.send(true);
    }
    publish_jobs(state);
    Ok(job)
}

fn update(state: &AppState, id: &str, f: impl FnOnce(&mut Job) -> Result<(), JobError>) {
    if let Err(e) = state.jobs.lock().expect("jobs lock").update(id, f) {
        warn!(id, "job transition refused: {e}");
    }
    publish_jobs(state);
}

pub fn publish_jobs(state: &AppState) {
    let jobs = state.jobs.lock().expect("jobs lock").list();
    let _ = state.bus.send(Event::Jobs(jobs));
}

fn clear_frames(dir: &Path) -> std::io::Result<()> {
    for item in std::fs::read_dir(dir)? {
        let path = item?.path();
        let is_frame = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("frame_"))
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if is_frame {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// `Ok(false)` when the job was cancelled during the encode.
async fn run_ffmpeg(e: &Encode<'_>, mut cancel: watch::Receiver<bool>) -> Result<bool, String> {
    let mut child = tokio::process::Command::new("ffmpeg")
        .args(ffmpeg_args(e))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("cannot run ffmpeg: {err}"))?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let read_stderr = async {
        let mut text = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut text).await;
        text
    };
    let wait = async {
        let (status, text) = tokio::join!(child.wait(), read_stderr);
        (status, text)
    };
    tokio::select! {
        (status, text) = wait => match status {
            Ok(s) if s.success() => Ok(true),
            Ok(s) => Err(format!("ffmpeg failed ({s}): {}", text.trim())),
            Err(err) => Err(format!("ffmpeg failed: {err}")),
        },
        _ = cancel.wait_for(|c| *c) => Ok(false),
    }
}

/// One message for the agent, keeping the Python traceback when there is one.
#[must_use]
pub fn describe(e: &CallError) -> String {
    match e {
        CallError::Failed {
            error,
            traceback: Some(tb),
            ..
        } => format!("{error}\n{tb}"),
        other => other.to_string(),
    }
}
