//! Animation jobs against a stub Blender and the real ffmpeg.
#![allow(clippy::unwrap_used)] // helpers outside #[test] fns
use blender_live::{
    AppState,
    basecamp::BaseCamp,
    blender::{Blender, Config},
    events,
    job::{Job, Phase},
    protocol::Shot,
    render::{self, Animation},
};
use std::{path::Path, process::Command, time::Duration};

async fn start() -> (tempfile::TempDir, AppState) {
    start_with_reserve(0).await
}

/// `reserve` is the free space the temp directory must have to be used.
async fn start_with_reserve(reserve: u64) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let blender = Blender::start(Config {
        bin: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_blender.py"),
        gpus: None,
        camp: BaseCamp::open(dir.path()).unwrap(),
        grace: Duration::from_millis(500),
        gpu_retry: blender_live::gpu::GpuRetry::default(),
        scratch_reserve: reserve,
    })
    .await
    .unwrap();
    (dir, AppState::new(blender, events::bus()))
}

fn clip(frame_end: i32, audio: Option<&str>) -> Animation {
    Animation {
        name: Some("clip".into()),
        frame_start: Some(1),
        frame_end: Some(frame_end),
        fps: Some(24.0),
        audio: audio.map(Into::into),
        shot: Shot::default(),
    }
}

fn job(state: &AppState, id: &str) -> Job {
    state.jobs.lock().unwrap().get(id).cloned().unwrap()
}

async fn wait_for(state: &AppState, id: &str, f: impl Fn(&Job) -> bool) -> Job {
    for _ in 0..400 {
        let current = job(state, id);
        if f(&current) {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job never matched: {:?}", job(state, id));
}

/// What the jobs left in Blender's run directory.
fn job_dirs(state: &AppState) -> Vec<String> {
    std::fs::read_dir(state.blender.run_dir())
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("job-"))
        .collect()
}

fn frames_in(mp4: &Path) -> String {
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
        .arg(mp4)
        .output()
        .unwrap();
    String::from_utf8(probe.stdout).unwrap().trim().to_owned()
}

#[tokio::test]
async fn a_job_encodes_every_frame_and_leaves_only_the_mp4() {
    let (dir, state) = start().await;
    let started = render::animation(&state, clip(12, None)).await.unwrap();
    let done = wait_for(&state, &started.id, |j| !j.is_active()).await;

    assert_eq!((done.phase, done.error.as_deref()), (Phase::Done, None));
    assert_eq!(done.outputs, ["renders/clip.mp4"]);
    assert_eq!(frames_in(&dir.path().join("renders/clip.mp4")), "12");
    assert!(
        !dir.path().join("renders/clip").exists(),
        "frames were written into the camp"
    );
    assert_eq!(job_dirs(&state), [""; 0], "temporary frames stayed");
}

#[tokio::test]
async fn a_cancelled_job_keeps_the_earlier_video_and_leaves_nothing_behind() {
    let (dir, state) = start().await;
    let mp4 = dir.path().join("renders/clip.mp4");
    std::fs::write(&mp4, b"earlier").unwrap();
    let started = render::animation(&state, clip(200, None)).await.unwrap();
    wait_for(&state, &started.id, |j| j.done_frames >= 3).await;
    assert_eq!(job_dirs(&state).len(), 1, "frames are not in the run dir");

    render::cancel(&state, &started.id).unwrap();
    let cancelled = wait_for(&state, &started.id, |j| !j.is_active()).await;

    assert_eq!(cancelled.phase, Phase::Cancelled);
    assert!(cancelled.done_frames < 200, "every frame rendered");
    assert_eq!(cancelled.outputs, [""; 0]);
    assert_eq!(std::fs::read(&mp4).unwrap(), b"earlier");
    assert!(
        !dir.path().join("renders/clip").exists(),
        "frames in the camp"
    );
    assert_eq!(job_dirs(&state), [""; 0], "temporary frames stayed");
}

#[tokio::test]
async fn a_full_temp_directory_moves_the_frames_under_the_camp() {
    let (dir, state) = start_with_reserve(u64::MAX).await;
    let spill = dir.path().join(".tmp");
    let started = render::animation(&state, clip(12, None)).await.unwrap();
    let rendering = wait_for(&state, &started.id, |j| j.done_frames >= 1).await;
    assert!(
        rendering.temp_in_camp,
        "the job does not say where it works"
    );
    assert!(spill.join(format!("job-{}", started.id)).is_dir());
    assert_eq!(
        job_dirs(&state),
        [""; 0],
        "the full temp directory was used"
    );

    let done = wait_for(&state, &started.id, |j| !j.is_active()).await;
    assert_eq!((done.phase, done.error.as_deref()), (Phase::Done, None));
    assert_eq!(frames_in(&dir.path().join("renders/clip.mp4")), "12");
    assert!(!spill.exists(), "temporary frames stayed in the camp");
}

#[tokio::test]
async fn a_failing_ffmpeg_fails_the_job_and_stops_blender() {
    let (dir, state) = start().await;
    std::fs::write(dir.path().join("assets/voice.wav"), b"not audio").unwrap();
    let started = render::animation(&state, clip(200, Some("assets/voice.wav")))
        .await
        .unwrap();
    let failed = wait_for(&state, &started.id, |j| !j.is_active()).await;

    assert_eq!(failed.phase, Phase::Failed);
    let error = failed.error.unwrap();
    assert!(error.starts_with("ffmpeg failed"), "{error}");
    assert!(failed.done_frames < 200, "Blender rendered every frame");
    assert!(
        !dir.path().join("renders/clip.mp4").exists(),
        "a broken MP4"
    );
    assert_eq!(job_dirs(&state), [""; 0], "temporary frames stayed");
    render::ensure_idle(&state).unwrap();
    state
        .blender
        .call(blender_live::protocol::Op::Summary, None)
        .await
        .unwrap();
}
