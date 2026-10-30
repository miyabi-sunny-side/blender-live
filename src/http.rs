//! The browser window: JSON API, server-sent events, files and the UI.
use crate::{AppState, events::Event, files, render};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as UrlPath, Query, Request, State},
    http::{HeaderValue, StatusCode, Uri, header},
    response::{
        IntoResponse, Response,
        sse::{self, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tower::ServiceExt;
use tower_http::{services::ServeFile, trace::TraceLayer};

static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/client/dist");

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// Routes that need no Blender: health and the compiled UI.
pub fn shell<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/health", get(api_health))
        .fallback_service(get(ui))
}

pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/jobs", get(jobs))
        .route("/jobs/{id}/cancel", post(cancel))
        .route("/files/{dir}", get(list))
        .route("/events", get(events))
        .fallback(api_not_found);
    Router::new()
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/", axum::routing::any(api_not_found))
        .nest("/api", api)
        .route("/files/{*path}", get(file))
        .merge(crate::mcp::endpoint(&state))
        .merge(shell())
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "blender": state.blender.status(),
        "jobs": state.jobs.lock().expect("jobs lock").list(),
    }))
}

async fn jobs(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.jobs.lock().expect("jobs lock").list())
}

async fn cancel(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Response {
    match render::cancel(&state, &id) {
        Ok(job) => Json(job).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn list(State(state): State<AppState>, UrlPath(dir): UrlPath<String>) -> Response {
    match files::list(state.camp(), &dir) {
        Ok(entries) => Json(entries).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let initial = [
        Event::Status(Box::new(state.blender.status())),
        Event::Jobs(state.jobs.lock().expect("jobs lock").list()),
    ];
    let live = BroadcastStream::new(state.bus.subscribe()).map(move |item| match item {
        Ok(event) => event,
        // A slow browser missed events; make it fetch the lists again.
        Err(_) => Event::Files,
    });
    let stream = tokio_stream::iter(initial).chain(live).map(|event| {
        Ok(sse::Event::default()
            .json_data(event)
            .expect("events serialize"))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct FileQuery {
    download: Option<String>,
}

/// Serves a base camp file with range support; `?download` asks the
/// browser to save it. Paths that leave the camp are refused.
async fn file(
    State(state): State<AppState>,
    UrlPath(path): UrlPath<String>,
    Query(query): Query<FileQuery>,
    request: Request,
) -> Response {
    let resolved = match state.camp().existing(&path) {
        Ok(p) if p.is_file() => p,
        Ok(_) => return (StatusCode::NOT_FOUND, "not a file\n").into_response(),
        Err(e) => {
            let code = if matches!(e, crate::basecamp::PathError::NotFound) {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::FORBIDDEN
            };
            return (code, format!("{e}\n")).into_response();
        }
    };
    let mut response = match ServeFile::new(&resolved).oneshot(request).await {
        Ok(r) => r.map(Body::new),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response(),
    };
    if query.download.is_some() {
        let name = resolved
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let value = format!(
            "attachment; filename*=UTF-8''{}",
            crate::naming::encode_path(&name)
        );
        if let Ok(value) = HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .insert(header::CONTENT_DISPOSITION, value);
        }
    }
    response
}

async fn ui(uri: Uri) -> Response {
    let file = UI
        .get_file(uri.path().trim_start_matches('/'))
        .unwrap_or_else(|| {
            UI.get_file("index.html")
                .expect("build requires index.html")
        });
    let content_type = mime_guess::from_path(file.path()).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, content_type.as_ref())],
        file.contents(),
    )
        .into_response()
}

async fn healthz() -> &'static str {
    "ok\n"
}

async fn api_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn api_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::shell;

    async fn get(uri: &str) -> axum::response::Response {
        shell::<()>()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn ui_is_available_without_a_static_directory() {
        let response = get("/").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.starts_with(b"<!doctype html>"));
        assert!(std::str::from_utf8(&body).unwrap().contains("/assets/"));
    }

    #[tokio::test]
    async fn compiled_assets_are_served_with_their_content_types() {
        let response = get("/").await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        for (attribute, extension, content_type) in [
            ("src=\"", ".js", "text/javascript"),
            ("href=\"", ".css", "text/css"),
        ] {
            let asset = html
                .split(attribute)
                .skip(1)
                .filter_map(|part| part.split('"').next())
                .find(|path| path.ends_with(extension))
                .expect("compiled HTML references its asset");
            let response = get(asset).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], content_type);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(!body.is_empty());
            assert!(!body.starts_with(b"<!doctype html>"));

            let response = shell::<()>()
                .oneshot(
                    Request::builder()
                        .method("HEAD")
                        .uri(asset)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], content_type);
            assert!(
                to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn ui_rejects_mutating_requests() {
        let response = shell::<()>()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/projects/example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn liveness_is_lightweight_plain_text() {
        let response = get("/healthz").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            "ok\n"
        );
    }

    #[tokio::test]
    async fn api_health_returns_stable_json() {
        let response = get("/api/health").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            r#"{"status":"ok"}"#
        );
    }

    #[tokio::test]
    async fn unknown_client_routes_return_the_spa_with_success() {
        let response = get("/projects/example").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("content-type").unwrap(), "text/html");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.starts_with(b"<!doctype html>"));
    }
}
