//! HTTP 适配层. 业务逻辑在 `amane-core`, 这里只做路由、DTO 与错误映射.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;

pub mod dto;
pub mod error;
pub mod routes;
pub mod state;

pub use state::{AppState, Runtime};

#[derive(OpenApi)]
#[openapi(
    info(title = "Amane", description = "影片元数据服务"),
    paths(
        routes::health,
        routes::list_metadata,
        routes::create_metadata,
        routes::get_metadata,
        routes::extract_number,
        routes::config_schema,
        routes::list_tasks,
        routes::create_task,
        routes::get_task,
        routes::cancel_task,
        routes::retry_task,
        routes::task_log,
        routes::task_children,
    ),
    components(schemas(
        dto::MetadataDto,
        dto::MetadataPage,
        dto::CreateMetadataRequest,
        dto::HealthResponse,
        dto::ExtractedNumber,
        dto::TaskDto,
        dto::TaskPage,
        dto::CreateTaskRequest,
        dto::TaskLogEntry,
        dto::TaskLogPage,
        error::ErrorBody,
    ))
)]
pub struct ApiDoc;

/// 生成 OpenAPI 文档. 版本号取自编译期, 避免与 `Cargo.toml` 漂移.
pub fn openapi() -> utoipa::openapi::OpenApi {
    let mut document = ApiDoc::openapi();
    document.info.version = env!("CARGO_PKG_VERSION").to_owned();
    document
}

pub fn build_router(state: AppState) -> Router {
    build_router_with_ui(state, ui_dir())
}

/// 与 [`build_router`] 相同, 但由调用方指定前端产物目录; 测试据此不需要环境变量.
pub fn build_router_with_ui(state: AppState, ui: Option<PathBuf>) -> Router {
    let api = routes::router()
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state);

    let router = Router::new()
        .nest("/api", api)
        .layer(TraceLayer::new_for_http());

    match ui {
        Some(dir) => mount_ui(router, dir),
        None => router,
    }
}

/// 前端产物与接口共用端口: 命中文件时按扩展名返回, 未命中回退到 `index.html`.
///
/// 不使用 `ServeDir::not_found_service` — 它保留 404 状态, 适用于自定义错误页,
/// 单页应用的深链接需要 200.
fn mount_ui(router: Router, dir: PathBuf) -> Router {
    let root = Arc::new(dir);
    let handler = move |uri: Uri| {
        let root = Arc::clone(&root);
        async move { serve_ui(&root, &uri).await }
    };

    router.fallback_service(get(handler))
}

async fn serve_ui(root: &Path, uri: &Uri) -> Response {
    let relative = uri.path().trim_start_matches('/');
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };

    if !relative.contains("..")
        && let Ok(bytes) = tokio::fs::read(root.join(relative)).await
    {
        return ([(CONTENT_TYPE, mime_for(relative))], bytes).into_response();
    }

    match tokio::fs::read(root.join("index.html")).await {
        Ok(bytes) => ([(CONTENT_TYPE, "text/html; charset=utf-8")], bytes).into_response(),
        Err(error) => (StatusCode::NOT_FOUND, format!("前端产物不可用: {error}")).into_response(),
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// 已在构建产物中附带前端时, 由同一端口提供静态文件.
fn ui_dir() -> Option<PathBuf> {
    let candidate = std::env::var("AMANE_UI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ui/dist"));

    candidate.join("index.html").exists().then_some(candidate)
}

async fn require_token(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(expected) = state.runtime.settings.cold.api_token.as_deref() else {
        return next.run(request).await;
    };

    let provided = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    if provided == Some(expected) {
        next.run(request).await
    } else {
        error::ApiError::Unauthorized.into_response()
    }
}

#[cfg(test)]
mod tests {
    use amane_core::config::{ColdSettings, HotSettings, Settings};
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    use super::{AppState, Runtime};

    async fn app(api_token: Option<&str>) -> (tempfile::TempDir, axum::Router) {
        let (dir, router, _state) = app_state(api_token).await;
        (dir, router)
    }

    async fn app_state(api_token: Option<&str>) -> (tempfile::TempDir, axum::Router, AppState) {
        let (dir, router, state, _ui) = app_full(api_token, None).await;
        (dir, router, state)
    }

    async fn app_full(
        api_token: Option<&str>,
        ui: Option<std::path::PathBuf>,
    ) -> (
        tempfile::TempDir,
        axum::Router,
        AppState,
        Option<std::path::PathBuf>,
    ) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let settings = Settings {
            cold: ColdSettings {
                data_dir: dir.path().to_path_buf(),
                api_token: api_token.map(str::to_owned),
                ..ColdSettings::default()
            },
            hot: HotSettings::default(),
        };
        let runtime = Runtime::boot(settings).await.expect("运行时应启动");
        let state = AppState::new(runtime);
        let router = super::build_router_with_ui(state.clone(), ui);
        (dir, router, state, None)
    }

    #[tokio::test]
    async fn create_then_list_metadata() {
        let (_dir, app) = app(None).await;

        let created = app
            .clone()
            .oneshot(
                Request::post("/api/metadata")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"number": "ssis001", "title": "标题"}"#))
                    .expect("应构造请求"),
            )
            .await
            .expect("应得到响应");
        assert_eq!(created.status(), StatusCode::CREATED);

        let body = to_bytes(created.into_body(), usize::MAX)
            .await
            .expect("应读取响应体");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("应为 JSON");
        assert_eq!(value["number"], "SSIS-001");

        let listed = app
            .oneshot(
                Request::get("/api/metadata")
                    .body(Body::empty())
                    .expect("应构造请求"),
            )
            .await
            .expect("应得到响应");
        let body = to_bytes(listed.into_body(), usize::MAX)
            .await
            .expect("应读取响应体");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("应为 JSON");
        assert_eq!(value["total"], 1);
        assert_eq!(value["items"][0]["number"], "SSIS-001");
    }

    #[tokio::test]
    async fn rejects_invalid_number_with_error_body() {
        let (_dir, app) = app(None).await;

        let response = app
            .oneshot(
                Request::post("/api/metadata")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"number": "not a number"}"#))
                    .expect("应构造请求"),
            )
            .await
            .expect("应得到响应");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("应读取响应体");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("应为 JSON");
        assert_eq!(value["code"], "invalid_number");
    }

    #[tokio::test]
    async fn requires_token_when_configured() {
        let (_dir, app) = app(Some("secret")).await;

        let denied = app
            .clone()
            .oneshot(
                Request::get("/api/health")
                    .body(Body::empty())
                    .expect("应构造请求"),
            )
            .await
            .expect("应得到响应");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let allowed = app
            .oneshot(
                Request::get("/api/health")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("应构造请求"),
            )
            .await
            .expect("应得到响应");
        assert_eq!(allowed.status(), StatusCode::OK);
    }

    async fn call(
        app: axum::Router,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
            .expect("应构造请求");
        let response = app.oneshot(request).await.expect("应得到响应");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("应读取响应体");
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("响应应为 JSON")
        };
        (status, value)
    }

    #[tokio::test]
    async fn task_endpoints_round_trip() {
        let (_dir, app) = app(None).await;

        let (status, created) = call(
            app.clone(),
            Method::POST,
            "/api/tasks",
            Some(r#"{"kind":"scrape","payload":{"number":"ABC-123"}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(created["status"], "queued");
        let id = created["id"].as_i64().expect("应带任务标识");

        let (status, page) = call(app.clone(), Method::GET, "/api/tasks?status=queued", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 1);

        // 不带过滤时必须同样返回条目: 参数编号错位曾让 items 为空而 total 正确.
        let (status, page) = call(app.clone(), Method::GET, "/api/tasks", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["items"].as_array().map(Vec::len), Some(1));

        let (status, cancelled) = call(
            app.clone(),
            Method::POST,
            &format!("/api/tasks/{id}/cancel"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cancelled["status"], "cancelled");

        let (status, retried) = call(
            app.clone(),
            Method::POST,
            &format!("/api/tasks/{id}/retry"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(retried["retry_of"], id);
        assert_eq!(retried["status"], "queued");

        let (status, _) = call(app, Method::GET, "/api/tasks/9999", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn task_endpoints_reject_unknown_vocabulary() {
        let (_dir, app) = app(None).await;

        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/tasks",
            Some(r#"{"kind":"unknown","payload":{}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_task_kind");

        let (status, body) = call(app, Method::GET, "/api/tasks?status=bogus", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_task_status");
    }

    #[tokio::test]
    async fn scrape_task_runs_and_records_log() {
        let (_dir, app, state) = app_state(None).await;
        let worker = state.runtime.tasks.start();

        let (status, created) = call(
            app.clone(),
            Method::POST,
            "/api/tasks",
            Some(r#"{"kind":"scrape","payload":{"number":"ABC-123"}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let id = created["id"].as_i64().expect("应带任务标识");

        let mut settled = serde_json::Value::Null;
        for _ in 0..200 {
            let (_, task) = call(app.clone(), Method::GET, &format!("/api/tasks/{id}"), None).await;
            if task["status"] == "failed" {
                settled = task;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        assert_eq!(settled["status"], "failed", "没有来源时任务应以失败结束");
        assert!(
            settled["error"]
                .as_str()
                .unwrap_or_default()
                .contains("没有来源返回结果"),
            "失败说明应指出没有来源返回结果"
        );

        let (status, log) = call(app, Method::GET, &format!("/api/tasks/{id}/log"), None).await;
        assert_eq!(status, StatusCode::OK);
        let lines = log["items"].as_array().expect("日志应为数组");
        assert!(!lines.is_empty(), "任务应留下日志");
        assert_eq!(lines.last().expect("应有内容")["level"], "error");

        state.runtime.tasks.shutdown();
        worker.abort();
    }

    #[test]
    fn openapi_lists_every_route() {
        let document = super::openapi();
        let json = serde_json::to_value(&document).expect("文档应可序列化");
        let paths = json["paths"].as_object().expect("paths 应为对象");

        for expected in [
            "/api/health",
            "/api/metadata",
            "/api/metadata/{number}",
            "/api/number/extract",
            "/api/config/schema",
            "/api/tasks",
            "/api/tasks/{id}",
            "/api/tasks/{id}/cancel",
            "/api/tasks/{id}/retry",
            "/api/tasks/{id}/log",
            "/api/tasks/{id}/children",
        ] {
            assert!(paths.contains_key(expected), "缺少路径 {expected}");
        }
        assert_eq!(json["info"]["version"], env!("CARGO_PKG_VERSION"));
    }
}

#[cfg(test)]
mod ui_tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::{AppState, Runtime, build_router_with_ui};
    use amane_core::config::Settings;

    fn ui_fixture(index: &str, asset: Option<(&str, &str)>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        std::fs::write(dir.path().join("index.html"), index).expect("应写入 index.html");
        if let Some((name, body)) = asset {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().expect("应有父目录")).expect("应创建子目录");
            std::fs::write(path, body).expect("应写入静态文件");
        }
        dir
    }

    async fn app(ui: std::path::PathBuf) -> (tempfile::TempDir, axum::Router) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let settings = Settings {
            cold: amane_core::config::ColdSettings {
                data_dir: dir.path().to_path_buf(),
                ..amane_core::config::ColdSettings::default()
            },
            ..Settings::default()
        };
        let runtime = Runtime::boot(settings).await.expect("运行时应启动");
        (dir, build_router_with_ui(AppState::new(runtime), Some(ui)))
    }

    async fn get(app: axum::Router, path: &str) -> (StatusCode, String, String) {
        let response = app
            .oneshot(Request::get(path).body(Body::empty()).expect("应构造请求"))
            .await
            .expect("应得到响应");
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("应读取响应体");
        (
            status,
            content_type,
            String::from_utf8_lossy(&body).into_owned(),
        )
    }

    #[tokio::test]
    async fn serves_assets_and_falls_back_to_index() {
        let ui = ui_fixture(
            "<!doctype html><title>app</title>",
            Some(("assets/app.js", "console.log(1)")),
        );
        let (_dir, app) = app(ui.path().to_path_buf()).await;

        let (status, content_type, body) = get(app.clone(), "/assets/app.js").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            content_type.starts_with("text/javascript"),
            "实际 {content_type}"
        );
        assert_eq!(body, "console.log(1)");

        let (status, content_type, body) = get(app.clone(), "/metadata/ABC-123").await;
        assert_eq!(status, StatusCode::OK, "深链接必须返回 200 而不是 404");
        assert!(content_type.starts_with("text/html"));
        assert!(body.contains("app"));

        let (status, _, _) = get(app, "/api/health").await;
        assert_eq!(status, StatusCode::OK, "接口路由不应被前端回退吞掉");
    }

    #[tokio::test]
    async fn rejects_path_traversal_without_leaking_files() {
        let ui = ui_fixture("<!doctype html><title>app</title>", None);
        let (_dir, app) = app(ui.path().to_path_buf()).await;

        let (status, _, body) = get(app, "/../../etc/passwd").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("app"), "越界路径应回退到 index.html");
        assert!(!body.contains("root:"), "越界路径不应读到库外文件");
    }
}
