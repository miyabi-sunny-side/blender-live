//! The browser API against a stub Blender.
#![allow(clippy::unwrap_used)] // helpers outside #[test] fns
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use blender_live::{
    AppState,
    basecamp::BaseCamp,
    blender::{Blender, Config},
    events, render,
};
use serde_json::Value;
use std::{path::Path, time::Duration};
use tower::ServiceExt;

async fn app() -> (tempfile::TempDir, AppState, Router) {
    let dir = tempfile::tempdir().unwrap();
    let camp = BaseCamp::open(&dir.path().join("camp")).unwrap();
    let blender = Blender::start(Config {
        bin: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_blender.py"),
        gpus: None,
        camp,
        grace: Duration::from_millis(500),
    })
    .await
    .unwrap();
    let state = AppState::new(blender, events::bus());
    (dir, state.clone(), blender_live::app(state))
}

async fn send(
    app: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body.to_vec())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn files_inside_the_camp_are_served_with_ranges_and_downloads() {
    let (_dir, state, app) = app().await;
    std::fs::write(state.camp().root().join("renders/モカ.png"), b"0123456789").unwrap();

    let (status, _, body) = send(&app, get("/files/renders/%E3%83%A2%E3%82%AB.png")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"0123456789");

    let ranged = Request::builder()
        .uri("/files/renders/%E3%83%A2%E3%82%AB.png")
        .header(header::RANGE, "bytes=2-4")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, ranged).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"234");

    let (_, headers, _) = send(
        &app,
        get("/files/renders/%E3%83%A2%E3%82%AB.png?download=1"),
    )
    .await;
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename*=UTF-8''%E3%83%A2%E3%82%AB.png"
    );
}

#[tokio::test]
async fn files_outside_the_camp_are_refused() {
    let (dir, state, app) = app().await;
    std::fs::write(dir.path().join("secret.txt"), b"secret").unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("secret.txt"),
        state.camp().root().join("assets/link.txt"),
    )
    .unwrap();
    for uri in [
        "/files/../secret.txt",
        "/files/renders/..%2F..%2Fsecret.txt",
        "/files/%2Fetc%2Fhostname",
        "/files/assets/link.txt",
    ] {
        let (status, _, body) = send(&app, get(uri)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
        assert!(!body.starts_with(b"secret"), "{uri} leaked");
    }
    let (status, _, _) = send(&app, get("/files/renders/missing.png")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn lists_and_status_are_json() {
    let (_dir, state, app) = app().await;
    std::fs::write(state.camp().root().join("scenes/cube.blend"), b"blend").unwrap();

    let (status, _, body) = send(&app, get("/api/files/scenes")).await;
    assert_eq!(status, StatusCode::OK);
    let entries: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries[0]["path"], "scenes/cube.blend");
    assert_eq!(entries[0]["kind"], "blend");

    let (status, _, _) = send(&app, get("/api/files/..")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, body) = send(&app, get("/api/status")).await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["blender"]["state"], "ready");
    assert_eq!(body["jobs"], Value::Array(vec![]));
}

#[tokio::test]
async fn unknown_api_routes_do_not_fall_back_to_the_spa() {
    let (_dir, _state, app) = app().await;
    for uri in ["/api", "/api/", "/api/missing"] {
        let (status, _, _) = send(&app, get(uri)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn an_animation_job_can_be_cancelled_from_the_browser() {
    let (_dir, state, app) = app().await;
    let job = render::animation(
        &state,
        render::Animation {
            name: Some("clip".into()),
            frame_start: Some(1),
            frame_end: Some(200),
            fps: Some(24.0),
            audio: None,
            shot: blender_live::protocol::Shot::default(),
        },
    )
    .await
    .unwrap();
    assert!(
        render::ensure_idle(&state).is_err(),
        "Blender tools were not refused while rendering"
    );

    let cancel = Request::builder()
        .method("POST")
        .uri(format!("/api/jobs/{}/cancel", job.id))
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app, cancel).await;
    assert_eq!(status, StatusCode::OK);

    for _ in 0..100 {
        let current = state.jobs.lock().unwrap().get(&job.id).cloned().unwrap();
        if !current.is_active() {
            assert_eq!(current.phase, blender_live::job::Phase::Cancelled);
            assert!(current.done_frames < 200, "every frame rendered");
            let cancel = Request::builder()
                .method("POST")
                .uri(format!("/api/jobs/{}/cancel", job.id))
                .body(Body::empty())
                .unwrap();
            let (status, _, _) = send(&app, cancel).await;
            assert_eq!(status, StatusCode::CONFLICT);
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job never stopped");
}
