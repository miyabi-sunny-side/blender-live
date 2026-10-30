//! The supervisor against a stub Blender that speaks the bridge protocol.
#![allow(clippy::unwrap_used)] // helpers outside #[test] fns
use blender_live::{
    basecamp::BaseCamp,
    blender::{Blender, CallError, Config, ProcState, Status},
    protocol::{Op, Shot},
};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::watch;

async fn start() -> (tempfile::TempDir, Arc<Blender>) {
    let dir = tempfile::tempdir().unwrap();
    let camp = BaseCamp::open(dir.path()).unwrap();
    let blender = Blender::start(Config {
        bin: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_blender.py"),
        gpus: Some("0".into()),
        camp,
        grace: Duration::from_millis(500),
    })
    .await
    .unwrap();
    (dir, blender)
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
