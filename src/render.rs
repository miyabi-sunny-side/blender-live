//! Render requests that end in `renders/`: stills and animation jobs.
use crate::{
    AppState,
    blender::{CallError, Outcome},
    encode::{Encode, encode, ffmpeg_args, frame_file},
    events::Event,
    job::{Job, JobError, Phase},
    naming::output_name,
    protocol::{Op, Shot},
    scratch::Scratch,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use time::OffsetDateTime;
use tokio::sync::{mpsc, watch};
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

/// Starts an animation job that ends in `renders/{name}.mp4`. Returns as
/// soon as the job is registered.
pub async fn animation(state: &AppState, a: Animation) -> Result<Job, String> {
    ensure_idle(state)?;
    let name = output_name(a.name.as_deref(), "clip", "mp4", OffsetDateTime::now_utc())
        .map_err(|e| e.to_string())?;
    let camp = state.camp();
    let mp4 = camp
        .writable(&format!("renders/{name}.mp4"))
        .map_err(|e| e.to_string())?;
    let audio = match &a.audio {
        Some(rel) => Some(
            camp.existing(rel)
                .map_err(|e| format!("audio {rel}: {e}"))?,
        ),
        None => None,
    };

    // ffmpeg starts with the first frame, so the rate is needed up front.
    let scene = if a.frame_start.is_none() || a.frame_end.is_none() || a.fps.is_none() {
        state
            .blender
            .call(Op::Summary, None)
            .await
            .map_err(|e| describe(&e))?
            .value
    } else {
        Value::Null
    };
    let frame = |key| scene[key].as_i64().and_then(|v| i32::try_from(v).ok());
    let frame_start = a.frame_start.or_else(|| frame("frame_start")).unwrap_or(1);
    let frame_end = a
        .frame_end
        .or_else(|| frame("frame_end"))
        .unwrap_or(frame_start);
    let fps = a.fps.or_else(|| scene["fps"].as_f64()).unwrap_or(24.0);

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
            mp4,
            audio,
            frame_start,
            frame_end,
            fps,
            shot: a.shot,
        },
        cancel_rx,
    ));
    Ok(job)
}

struct Plan {
    id: String,
    name: String,
    mp4: PathBuf,
    audio: Option<PathBuf>,
    frame_start: i32,
    frame_end: i32,
    fps: f64,
    shot: Shot,
}

/// How one pass over the frames ended.
enum End {
    Done,
    Cancelled,
    Failed(String),
    /// Blender itself went away, and the scene with it.
    Lost(String),
}

/// Renders and encodes, and records how the job ended. The frames wait for
/// ffmpeg in the temp directory; when that has no room, before or during
/// the job, the job runs under the base camp instead.
async fn run(state: AppState, p: Plan, cancel: watch::Receiver<bool>) {
    let id = p.id.as_str();
    let blender = &state.blender;
    let temp = if blender.temp_has_room() {
        Scratch::temp(blender.run_dir(), id).map_err(|e| e.to_string())
    } else {
        Err("less space is free than a job needs".into())
    };
    let (mut end, full) = match &temp {
        Ok(scratch) => {
            let end = pass(&state, &p, scratch, cancel.clone()).await;
            // Measured while the job's own files are still there: they count.
            (end, !blender.temp_has_room())
        }
        Err(e) => (
            End::Failed(format!("the temp directory cannot be used: {e}")),
            true,
        ),
    };
    drop(temp);
    if let End::Failed(why) = &end
        && full
    {
        warn!(
            id,
            "the temp directory has no room ({why}); the job runs under the base camp"
        );
        end = match Scratch::camp(state.camp(), id) {
            Ok(scratch) => {
                update(&state, id, Job::moved_to_camp);
                pass(&state, &p, &scratch, cancel).await
            }
            Err(e) => End::Failed(format!("no room for the frames: {why}; base camp: {e}")),
        };
    }
    match end {
        End::Done => update(&state, id, |j| {
            j.encoded(vec![format!("renders/{}.mp4", p.name)])
        }),
        End::Cancelled => update(&state, id, Job::cancelled),
        End::Failed(e) | End::Lost(e) => update(&state, id, |j| j.failed(e)),
    }
    state.cancels.lock().expect("cancels lock").remove(id);
}

/// Renders every frame into `scratch`, hands each one to ffmpeg as it is
/// done and moves the finished MP4 into the base camp.
async fn pass(
    state: &AppState,
    p: &Plan,
    scratch: &Scratch,
    mut cancel: watch::Receiver<bool>,
) -> End {
    let id = p.id.as_str();
    let dir = scratch.frames();
    let out = scratch.mp4();
    // Closed when the render call is over, which lets ffmpeg finish the MP4.
    let (frame_tx, frame_rx) = mpsc::unbounded_channel();
    let encoding = encode(
        ffmpeg_args(&Encode {
            fps: p.fps,
            audio: p.audio.as_deref(),
            output: &out,
        }),
        frame_rx,
    );
    tokio::pin!(encoding);

    // Stops Blender for the user's cancel and when ffmpeg gives up.
    let (halt, halted) = watch::channel(false);
    let op = Op::RenderFrames {
        dir: dir.to_string_lossy().into_owned(),
        frame_start: p.frame_start,
        frame_end: p.frame_end,
        shot: p.shot.clone(),
    };
    let rendering = state
        .blender
        .call_with_progress(op, Some(halted), move |done, _| {
            update(state, id, |j| j.frame_rendered(done));
            let frame = p
                .frame_start
                .saturating_add_unsigned(done.saturating_sub(1));
            let _ = frame_tx.send(frame_file(&dir, frame));
        });
    tokio::pin!(rendering);
    let mut encoded = None;
    let rendered = loop {
        tokio::select! {
            rendered = &mut rendering => break rendered,
            // ffmpeg only ends before the last frame when it failed.
            result = &mut encoding, if encoded.is_none() => {
                encoded = Some(result);
                halt.send_replace(true);
            }
            _ = cancel.wait_for(|c| *c), if !*halt.borrow() => {
                halt.send_replace(true);
            }
        }
    };
    match rendered {
        Ok(_) => {
            update(state, id, Job::frames_finished);
            let encoded = if let Some(result) = encoded {
                result
            } else {
                tokio::select! {
                    result = &mut encoding => result,
                    _ = cancel.wait_for(|c| *c) => return End::Cancelled,
                }
            };
            match encoded {
                Ok(()) => match keep(&out, &p.mp4).await {
                    Ok(()) => End::Done,
                    Err(e) => End::Failed(format!("cannot save renders/{}.mp4: {e}", p.name)),
                },
                Err(e) => End::Failed(e),
            }
        }
        Err(CallError::Failed {
            cancelled: true, ..
        }) => match encoded {
            Some(Err(e)) => End::Failed(e),
            _ => End::Cancelled,
        },
        Err(e @ CallError::Failed { .. }) => End::Failed(describe(&e)),
        Err(e) => End::Lost(describe(&e)),
    }
}

/// Moves the finished MP4 into the base camp, replacing an earlier one.
async fn keep(from: &Path, to: &Path) -> std::io::Result<()> {
    // The temp directory is usually another file system than the camp.
    if tokio::fs::rename(from, to).await.is_ok() {
        return Ok(());
    }
    tokio::fs::copy(from, to).await.map(|_| ())
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
