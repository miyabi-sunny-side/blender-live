//! The supervisor against a stub Blender that speaks the bridge protocol.
#![allow(clippy::unwrap_used)] // helpers outside #[test] fns
use blender_live::{
    basecamp::BaseCamp,
    blender::{Blender, CallError, Config, ProcState, Status},
    gpu::GpuRetry,
    protocol::{Op, Shot},
};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::watch;

/// Hides the fake GPU while it exists in the base camp; lists the pids started without it.
const NO_GPU: &str = "no-gpu";

fn config(dir: &Path) -> Config {
    Config {
        bin: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_blender.py"),
        gpus: Some("0".into()),
        camp: BaseCamp::open(dir).unwrap(),
        grace: Duration::from_millis(500),
        gpu_retry: GpuRetry {
            first: Duration::from_millis(50),
            max: Duration::from_millis(200),
            quiet: Duration::ZERO,
        },
    }
}

async fn start() -> (tempfile::TempDir, Arc<Blender>) {
    let dir = tempfile::tempdir().unwrap();
    let blender = Blender::start(config(dir.path())).await.unwrap();
    (dir, blender)
}

/// Starts while the GPU is not there yet, as right after boot.
async fn start_without_gpu() -> (tempfile::TempDir, Arc<Blender>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(NO_GPU), "").unwrap();
    let blender = Blender::start(config(dir.path())).await.unwrap();
    assert_eq!(blender.status().devices, [""; 0], "the GPU was not hidden");
    (dir, blender)
}

fn gpu_appears(dir: &tempfile::TempDir) {
    std::fs::remove_file(dir.path().join(NO_GPU)).unwrap();
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn exec(code: &str) -> Op {
    Op::Exec {
        code: code.into(),
        timeout_secs: 5,
    }
}

async fn wait_for(blender: &Blender, f: impl Fn(&Status) -> bool) -> Status {
    for _ in 0..100 {
        let status = blender.status();
        if f(&status) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("status never matched: {:?}", blender.status());
}

#[tokio::test]
async fn the_session_keeps_state_between_requests() {
    let (_dir, blender) = start().await;
    let status = blender.status();
    assert_eq!(status.state, ProcState::Ready);
    assert_eq!(status.devices, ["Fake GPU"]);
    assert!(!status.unsaved, "fresh scene unsaved");

    blender
        .call(exec("x = 41\nprint('hi')"), None)
        .await
        .unwrap();
    let outcome = blender.call(exec("x + 1"), None).await.unwrap();
    assert_eq!(outcome.value, json!(42));
    assert!(
        blender.status().unsaved,
        "exec did not mark the scene unsaved"
    );

    let outcome = blender.call(exec("print('hi')"), None).await.unwrap();
    assert_eq!(outcome.stdout, "hi\n");
}

#[tokio::test]
async fn python_errors_come_back_with_their_traceback() {
    let (_dir, blender) = start().await;
    let Err(CallError::Failed {
        error, traceback, ..
    }) = blender.call(exec("1 / 0"), None).await
    else {
        panic!("division succeeded");
    };
    assert!(error.contains("ZeroDivisionError"), "{error}");
    assert!(traceback.unwrap().contains("Traceback"), "no traceback");
    assert_eq!(blender.status().state, ProcState::Ready);
}

#[tokio::test]
async fn a_crash_restarts_blender_and_reports_the_lost_scene() {
    let (_dir, blender) = start().await;
    blender.call(exec("x = 1"), None).await.unwrap();
    let result = blender.call(exec("import os\nos._exit(1)"), None).await;
    assert!(matches!(result, Err(CallError::Crashed(_))), "{result:?}");

    let status = wait_for(&blender, |s| s.state == ProcState::Ready && s.restarts == 1).await;
    let lost = status.lost.expect("lost scene not reported");
    assert!(lost.unsaved, "unsaved scene not flagged");
    assert_eq!(lost.exit_code, Some(1));
    assert!(!status.unsaved, "the restarted scene is fresh");

    let outcome = blender.call(exec("'x' in globals()"), None).await.unwrap();
    assert_eq!(outcome.value, json!(false), "state survived the restart");
}

#[tokio::test]
async fn a_crash_reports_the_output_of_the_failed_request_only() {
    let (_dir, blender) = start().await;
    // Startup output is long gone by the time real work fails.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let result = blender
        .call(
            exec("import os\nos.write(1, b'dying words\\n')\nos._exit(1)"),
            None,
        )
        .await;
    let Err(CallError::Crashed(message)) = result else {
        panic!("no crash: {result:?}");
    };
    assert!(message.contains("dying words"), "{message}");
    assert!(!message.contains("fake blender starting"), "{message}");
}

#[tokio::test]
async fn an_idle_crash_is_noticed_without_a_request() {
    let (_dir, blender) = start().await;
    blender
        .call(
            exec("import os, threading\nthreading.Timer(0.2, lambda: os._exit(3)).start()"),
            None,
        )
        .await
        .unwrap();
    let status = wait_for(&blender, |s| s.restarts == 1 && s.state == ProcState::Ready).await;
    assert_eq!(status.lost.unwrap().exit_code, Some(3));
}

#[tokio::test]
async fn a_saved_scene_is_not_reported_lost() {
    let (_dir, blender) = start().await;
    blender.call(exec("x = 1"), None).await.unwrap();
    blender
        .call(
            Op::Save {
                path: Some("/camp/scenes/a.blend".into()),
            },
            None,
        )
        .await
        .unwrap();
    assert!(!blender.status().unsaved, "save left the scene unsaved");
    let pid = blender.status().pid.unwrap().to_string();
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid])
        .status()
        .unwrap();
    assert!(killed.success(), "kill failed");
    let status = wait_for(&blender, |s| s.restarts == 1 && s.state == ProcState::Ready).await;
    assert!(!status.lost.unwrap().unsaved, "saved scene reported lost");
}

#[tokio::test]
async fn a_hung_request_is_killed_after_its_timeout() {
    let (_dir, blender) = start().await;
    let started = std::time::Instant::now();
    let result = blender
        .call(
            Op::Exec {
                code: "import time\ntime.sleep(30)".into(),
                timeout_secs: 1,
            },
            None,
        )
        .await;
    assert!(matches!(result, Err(CallError::Timeout(_))), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "kill took too long"
    );
    wait_for(&blender, |s| s.restarts == 1 && s.state == ProcState::Ready).await;
    blender.call(exec("1"), None).await.unwrap();
}

#[tokio::test]
async fn frame_renders_report_progress_and_stop_on_cancel() {
    let (dir, blender) = start().await;
    let frames = dir.path().join("renders/clip");
    std::fs::create_dir_all(&frames).unwrap();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let mut seen = Vec::new();
    let result = blender
        .call_with_progress(
            Op::RenderFrames {
                dir: frames.to_string_lossy().into_owned(),
                frame_start: 1,
                frame_end: 40,
                shot: Shot::default(),
            },
            Some(cancel_rx),
            |done, total| {
                seen.push((done, total));
                if done == 3 {
                    cancel_tx.send(true).unwrap();
                }
            },
        )
        .await;
    assert!(
        matches!(
            result,
            Err(CallError::Failed {
                cancelled: true,
                ..
            })
        ),
        "{result:?}"
    );
    assert_eq!(seen[..3], [(1, 40), (2, 40), (3, 40)]);
    assert!(seen.len() < 40, "cancel ignored");
    assert_eq!(blender.status().state, ProcState::Ready);
}

#[tokio::test]
async fn a_gpu_that_appears_after_the_start_is_picked_up() {
    let (dir, blender) = start_without_gpu().await;
    let first = blender.status();
    assert_eq!(first.state, ProcState::Ready, "no GPU is not an error");
    assert_eq!(first.device_type, None);

    gpu_appears(&dir);
    let status = wait_for(&blender, |s| !s.devices.is_empty()).await;
    assert_eq!(status.devices, ["Fake GPU"]);
    assert_eq!(status.device_type.as_deref(), Some("OPTIX"));
    assert_eq!(status.state, ProcState::Ready);
    assert_eq!(status.restarts, 0, "recovery was reported as a crash");
    assert_eq!(status.lost, None, "recovery reported lost work");
    assert_eq!(status.gpu_note, None);
    assert_ne!(status.pid, first.pid, "the GPU needs a new process");
    wait_for(&blender, |_| !alive(first.pid.unwrap())).await;
    blender.call(exec("1"), None).await.unwrap();
}

#[tokio::test]
async fn a_machine_without_a_gpu_keeps_its_session_while_it_is_checked() {
    let (dir, blender) = start_without_gpu().await;
    let first = blender.status();
    blender.call(exec("x = 41"), None).await.unwrap();

    let starts = || std::fs::read_to_string(dir.path().join(NO_GPU)).unwrap();
    wait_for(&blender, |_| starts().lines().count() >= 4).await;

    let started = std::time::Instant::now();
    let outcome = blender.call(exec("x + 1"), None).await.unwrap();
    assert_eq!(outcome.value, json!(42), "the session was replaced");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "a check delayed the request"
    );
    let status = blender.status();
    assert_eq!(status.state, ProcState::Ready);
    assert_eq!(status.pid, first.pid);
    assert_eq!(status.devices, [""; 0]);
    assert_eq!((status.restarts, status.last_error), (0, None));
    assert_eq!(status.gpu_note, None);
}

#[tokio::test]
async fn unsaved_work_holds_the_gpu_back_until_it_is_saved() {
    let (dir, blender) = start_without_gpu().await;
    let first = blender.status();
    blender.call(exec("x = 41"), None).await.unwrap();

    gpu_appears(&dir);
    let waiting = wait_for(&blender, |s| s.gpu_note.is_some()).await;
    assert_eq!(waiting.devices, [""; 0]);
    assert_eq!(waiting.pid, first.pid, "unsaved work was thrown away");
    let outcome = blender.call(exec("x + 1"), None).await.unwrap();
    assert_eq!(outcome.value, json!(42));

    let scene = dir.path().join("scenes/a.blend");
    std::fs::write(&scene, b"blend").unwrap();
    blender
        .call(
            Op::Save {
                path: Some(scene.to_string_lossy().into_owned()),
            },
            None,
        )
        .await
        .unwrap();
    let status = wait_for(&blender, |s| !s.devices.is_empty()).await;
    assert_eq!(status.file.as_deref(), Some("scenes/a.blend"));
    assert!(!status.unsaved, "the reopened scene is saved");
    assert_eq!(status.gpu_note, None);
    assert_eq!(status.restarts, 0);
    let reopened = blender.call(Op::Summary, None).await.unwrap();
    assert_eq!(reopened.file.as_deref(), Some("scenes/a.blend"));
}

#[tokio::test]
async fn a_gpu_that_went_away_again_is_no_longer_announced() {
    let (dir, blender) = start_without_gpu().await;
    let first = blender.status();
    blender.call(exec("x = 41"), None).await.unwrap();
    gpu_appears(&dir);
    wait_for(&blender, |s| s.gpu_note.is_some()).await;

    std::fs::write(dir.path().join(NO_GPU), "").unwrap();
    blender.call(Op::New { empty: true }, None).await.unwrap();
    let status = wait_for(&blender, |s| s.gpu_note.is_none()).await;
    assert_eq!(status.devices, [""; 0]);
    assert_eq!(status.pid, first.pid, "replaced by a process without a GPU");
}

#[tokio::test]
async fn a_restart_after_a_crash_recovers_the_gpu_too() {
    let (dir, blender) = start().await;
    std::fs::write(dir.path().join(NO_GPU), "").unwrap();
    let result = blender.call(exec("import os\nos._exit(1)"), None).await;
    assert!(matches!(result, Err(CallError::Crashed(_))), "{result:?}");
    wait_for(&blender, |s| {
        s.state == ProcState::Ready && s.restarts == 1 && s.devices.is_empty()
    })
    .await;

    gpu_appears(&dir);
    let status = wait_for(&blender, |s| !s.devices.is_empty()).await;
    assert_eq!(status.restarts, 1);
    blender.call(exec("1"), None).await.unwrap();
}

#[tokio::test]
async fn stopping_ends_blender_without_bringing_it_back() {
    let (_dir, blender) = start().await;
    let pid = blender.status().pid.unwrap();
    blender.stop();
    wait_for(&blender, |_| !alive(pid)).await;
    tokio::time::sleep(Duration::from_millis(700)).await;

    let status = blender.status();
    assert_eq!(status.pid, Some(pid), "Blender was started again");
    assert_eq!(status.restarts, 0);
    let result = blender.call(exec("1"), None).await;
    assert!(matches!(result, Err(CallError::Start(_))), "{result:?}");
    assert_eq!(blender.status().pid, Some(pid), "a request started Blender");
}
