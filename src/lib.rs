use axum::{
    Json, Router,
    extract::Path as UrlPath,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    response::Response,
    routing::get,
};
use serde::Serialize;
use tower_http::trace::TraceLayer;

static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/client/dist");

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct Item {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    status: &'static str,
    updated_at: &'static str,
    body: &'static str,
}

// Demo fixtures for the starter UI. A derived project replaces this data
// source with its own domain.
const ITEMS: &[Item] = &[
    Item {
        id: "theme",
        name: "テーマ切替",
        summary: "ハンバーガーメニューの1要素目からテーマ設定を開き、自動・ライト・ダークを選べます。",
        status: "stable",
        updated_at: "2026-08-07",
        body: "選択は localStorage に保存され、次回表示でも同じテーマで描画されます。\
        自動を選ぶと保存値と data-theme 属性が消え、OS の設定に従います。\
        モーダルは選択後も開いたままになるので、テーマの変化をその場で確認できます。",
    },
    Item {
        id: "router",
        name: "ルーター",
        summary: "依存を増やさない小さな History API ルーターがページ遷移を担います。",
        status: "stable",
        updated_at: "2026-08-07",
        body: "カードから詳細への遷移は pushState、ブラウザの戻るは popstate で復元します。\
        深い URL を直接開いてもサーバが index.html を返すため、リロードで同じページが表示されます。",
    },
    Item {
        id: "auto-reload",
        name: "自動再取得",
        summary: "トップページは表示のたびに API から一覧を取り直します。",
        status: "stable",
        updated_at: "2026-08-07",
        body: "初回表示に加えて、タブが非表示から表示に戻ったときにも一覧を再取得します。\
        読み込み中・空・エラー・成功の4状態を data-state 属性で公開しています。",
    },
    Item {
        id: "icons",
        name: "アイコン辞書",
        summary: "SVG アイコンは Icon.svelte の1ファイルに集約されています。",
        status: "stable",
        updated_at: "2026-08-07",
        body: "派生プロジェクトで作ったアイコンはこの辞書へ引き取り、家族全体で1箇所に育てます。\
        絵文字や文字グリフをアイコンとして使うことは禁止です。\
        以下は現在の辞書の全エントリの実物見本です。",
    },
];

pub fn app() -> Router {
    let api = Router::new()
        .route("/health", get(api_health))
        .route("/items", get(api_items))
        .route("/items/{id}", get(api_item))
        .fallback(api_not_found);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/", axum::routing::any(api_not_found))
        .nest("/api", api)
        .fallback_service(get(ui))
        .layer(TraceLayer::new_for_http())
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

async fn api_items() -> Json<&'static [Item]> {
    Json(ITEMS)
}

async fn api_item(UrlPath(id): UrlPath<String>) -> Response {
    match ITEMS.iter().find(|item| item.id == id) {
        Some(item) => Json(item).into_response(),
        None => (StatusCode::NOT_FOUND, "item not found\n").into_response(),
    }
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found\n")
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::app;

    async fn get(uri: &str) -> axum::response::Response {
        app()
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

            let response = app()
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
        let response = app()
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
    async fn api_items_lists_the_demo_fixtures() {
        let response = get("/api/items").await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.starts_with('['));
        assert!(body.contains(r#""id":"theme""#));
        assert!(body.contains(r#""name""#));
        assert!(body.contains(r#""updated_at""#));
    }

    #[tokio::test]
    async fn api_item_detail_returns_the_matching_item() {
        let response = get("/api/items/theme").await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains(r#""id":"theme""#));
        assert!(body.contains(r#""body""#));
    }

    #[tokio::test]
    async fn api_item_detail_rejects_unknown_ids() {
        let response = get("/api/items/missing").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_api_routes_do_not_fall_back_to_the_spa() {
        for uri in ["/api", "/api/", "/api/missing"] {
            let response = get(uri).await;

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
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
