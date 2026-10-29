//! HTTP 适配层. 业务逻辑在 `amane-core`, 这里只做路由、DTO 与错误映射.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use amane_core::config::Settings;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use utoipa::OpenApi;

pub mod dto;
pub mod error;
pub mod logging;
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
        routes::update_metadata,
        routes::set_lock,
        routes::add_tag,
        routes::remove_tag,
        routes::extract_number,
        routes::config_schema,
        routes::filter_options,
        routes::read_config,
        routes::update_config,
        routes::get_resource,
        routes::list_tasks,
        routes::create_task,
        routes::get_task,
        routes::cancel_task,
        routes::retry_task,
        routes::task_log,
        routes::task_children,
        routes::list_schedules,
        routes::create_schedule,
        routes::update_schedule,
        routes::delete_schedule,
        routes::list_libraries,
        routes::create_library,
        routes::delete_library,
        routes::scan_library,
        routes::list_library_files,
        routes::organize_preview,
        routes::organize_execute,
        routes::cleanup_manifest,
        routes::cleanup_execute,
        routes::trash_listing,
        routes::trash_restore,
    ),
    components(schemas(
        dto::MetadataDto,
        dto::MetadataFieldDto,
        dto::UpdateMetadataRequest,
        dto::LockRequest,
        dto::TagRequest,
        dto::MetadataPage,
        dto::CreateMetadataRequest,
        dto::HealthResponse,
        dto::ExtractedNumber,
        dto::TaskDto,
        dto::TaskPage,
        dto::CreateTaskRequest,
        dto::FilterOptions,
        dto::ContentTypeOption,
        dto::SourceOption,
        dto::TaskLogEntry,
        dto::TaskLogPage,
        dto::ScheduleDto,
        dto::SchedulePage,
        dto::CreateScheduleRequest,
        dto::ConfigState,
        dto::SettingsDto,
        dto::ColdSettingsDto,
        dto::HotSettingsDto,
        dto::UpdateScheduleRequest,
        dto::LibraryDto,
        dto::LibraryPage,
        dto::CreateLibraryRequest,
        dto::LibraryFileDto,
        dto::LibraryFilePage,
        dto::OrganizePreviewDto,
        dto::PreviewItemDto,
        dto::OrganizeExecuteRequest,
        dto::CleanupManifestDto,
        dto::CleanupItemDto,
        dto::TrashPageDto,
        dto::TrashItemDto,
        dto::RestoreRequest,
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

    let router = Router::new().nest("/api", api);
    let router = match ui {
        Some(dir) => mount_ui(router, dir),
        // 前端产物缺失时给出可读提示, 而不是白屏或纯文本 404.
        None => router.fallback(|| async {
            (
                StatusCode::OK,
                [("content-type", "text/html; charset=utf-8")],
                "<!doctype html><meta charset=\"utf-8\"><title>Amane</title>                 <p>前端产物不存在。在 amane-rs 目录运行 <code>just rs build-ui</code> 后重启。</p>",
            )
                .into_response()
        }),
    };

    // 日志层放在最外层: 静态文件请求同样记录, 否则「哪个页面慢」无从判断.
    router.layer(middleware::from_fn(logging::log_requests))
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

/// 启动运行时与任务调度所需的全部状态.
pub async fn boot(settings: Settings) -> std::io::Result<AppState> {
    let runtime = Runtime::boot(settings)
        .await
        .map_err(std::io::Error::other)?;
    Ok(AppState::new(runtime))
}

/// 绑定监听地址并启动服务循环, 立即返回实际监听的地址.
///
/// 端口填 `0` 表示由系统分配; 桌面壳据此避免端口冲突. 调用方持有返回的句柄,
/// 进程退出即停止服务; 登记与任务都已持久化, 重启后按基准时刻继续.
pub async fn start(
    state: AppState,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let bind = state.runtime.settings().cold.bind.clone();
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "amane-server 已启动");

    let worker = state.runtime.tasks.start();
    let scheduler_loop = state.runtime.scheduler.clone().spawn();
    let router = build_router(state.clone());

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await;

        state.runtime.tasks.shutdown();
        let _ = worker.await;
        drop(scheduler_loop);
    });

    Ok((address, handle))
}

/// 运行到停机信号, 命令行入口使用.
pub async fn serve(state: AppState) -> std::io::Result<std::net::SocketAddr> {
    let (address, handle) = start(state).await?;
    let _ = handle.await;

    Ok(address)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到中断信号, 正在停止");
}

/// 已在构建产物中附带前端时, 由同一端口提供静态文件.
fn ui_dir() -> Option<PathBuf> {
    let candidate = std::env::var("AMANE_UI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ui/dist"));

    candidate.join("index.html").exists().then_some(candidate)
}

async fn require_token(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let settings = state.runtime.settings();
    let Some(expected) = settings.cold.api_token.as_deref() else {
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
    use amane_core::{MediaNumber, MetadataInput, ScrapeWrite};
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    use super::{AppState, Runtime};

    /// 把日志写进内存, 供断言字段是否存在.
    struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuffer {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("应加锁").extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture_logs() -> (
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        tracing::dispatcher::DefaultGuard,
    ) {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = std::sync::Arc::clone(&buffer);
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(move || LogBuffer(std::sync::Arc::clone(&writer)))
            .finish();
        (buffer, tracing::subscriber::set_default(subscriber))
    }

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

    /// 创建三条不同类型的手工番号条目: 有码、FC2、以及标题里带 `%` 的一条.
    async fn seeded_app() -> (tempfile::TempDir, axum::Router) {
        let (dir, app) = app(None).await;
        for body in [
            r#"{"number":"ABC-001","title":"第一部"}"#,
            r#"{"number":"FC2-PPV-1234567","title":"第二部"}"#,
            r#"{"number":"XYZ-100","title":"百分比 100% 的片子"}"#,
        ] {
            let (status, _) = call(app.clone(), Method::POST, "/api/metadata", Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "夹具写入应成功: {body}");
        }

        (dir, app)
    }

    #[tokio::test]
    async fn metadata_list_filters_by_type_and_keyword() {
        let (_dir, app) = seeded_app().await;

        let (status, page) = call(
            app.clone(),
            Method::GET,
            "/api/metadata?content_type=fc2",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 1, "按类型筛选的总数应只计命中的条目");
        assert_eq!(page["items"][0]["number"], "FC2-1234567");

        let (status, page) = call(app.clone(), Method::GET, "/api/metadata?q=abc", None).await;
        assert_eq!(status, StatusCode::OK, "小写关键字应被接受");
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["number"], "ABC-001");

        // `%` 在 LIKE 里是通配符, 必须按字面匹配, 否则一条会变成三条.
        let (_, page) = call(app.clone(), Method::GET, "/api/metadata?q=100%25", None).await;
        assert_eq!(page["total"], 1, "关键字里的 `%` 不应变成通配符");
        assert_eq!(page["items"][0]["number"], "XYZ-100");

        let (_, page) = call(app.clone(), Method::GET, "/api/metadata?q=不存在的词", None).await;
        assert_eq!(page["total"], 0);
        assert_eq!(page["items"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn metadata_list_pages_with_consistent_total() {
        let (_dir, app) = seeded_app().await;

        let (_, first) = call(app.clone(), Method::GET, "/api/metadata?limit=2", None).await;
        assert_eq!(first["total"], 3);
        assert_eq!(first["items"].as_array().map(Vec::len), Some(2));

        let (_, last) = call(
            app.clone(),
            Method::GET,
            "/api/metadata?limit=2&offset=2",
            None,
        )
        .await;
        assert_eq!(last["total"], 3, "总数不应随分页变化");
        assert_eq!(last["items"].as_array().map(Vec::len), Some(1));

        let (_, beyond) = call(app.clone(), Method::GET, "/api/metadata?offset=99", None).await;
        assert_eq!(beyond["total"], 3, "越界偏移仍应报告总数");
        assert_eq!(beyond["items"].as_array().map(Vec::len), Some(0));

        let (status, clamped) = call(app.clone(), Method::GET, "/api/metadata?limit=0", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            clamped["items"].as_array().map(Vec::len),
            Some(1),
            "0 应按下限 1 处理"
        );
    }

    #[tokio::test]
    async fn metadata_list_rejects_unknown_filter_values() {
        let (_dir, app) = seeded_app().await;

        let (status, body) = call(
            app.clone(),
            Method::GET,
            "/api/metadata?content_type=bogus",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_content_type");

        let (status, body) = call(
            app.clone(),
            Method::GET,
            "/api/metadata?source=nowhere",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "unknown_source");

        let (status, body) = call(app.clone(), Method::GET, "/api/metadata?limit=abc", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "非数字分页参数应被拒绝");
        assert!(!body["message"].as_str().unwrap_or_default().is_empty());
    }

    /// 建一条手工番号条目, 供详情相关用例共用.
    async fn seeded_entry(app: &axum::Router) {
        let (status, _) = call(
            app.clone(),
            Method::POST,
            "/api/metadata",
            Some(r#"{"number":"ABC-001","title":"原标题"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    fn field_of<'a>(detail: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
        detail["fields"]
            .as_array()
            .expect("fields 应为数组")
            .iter()
            .find(|field| field["field"] == name)
            .expect("应包含该字段")
    }

    #[tokio::test]
    async fn manual_edit_locks_field_and_marks_it_manual() {
        let (_dir, app) = app(None).await;
        seeded_entry(&app).await;

        let (status, detail) = call(
            app.clone(),
            Method::PATCH,
            "/api/metadata/ABC-001",
            Some(r#"{"title":"人工标题","note":"记一笔"}"#),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["title"], "人工标题");
        assert_eq!(detail["note"], "记一笔");
        assert_eq!(
            detail["locked_fields"],
            serde_json::json!(["title"]),
            "手工修订应同时加锁"
        );
        let title = field_of(&detail, "title");
        assert_eq!(title["source"], "manual", "来源应标为人工");
        assert_eq!(title["locked"], true);
        assert_eq!(title["label"], "标题");
    }

    #[tokio::test]
    async fn lock_can_be_toggled_and_rejects_unknown_field() {
        let (_dir, app) = app(None).await;
        seeded_entry(&app).await;

        let (status, detail) = call(
            app.clone(),
            Method::PUT,
            "/api/metadata/ABC-001/locks/plot",
            Some(r#"{"locked":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["locked_fields"], serde_json::json!(["plot"]));
        assert_eq!(field_of(&detail, "plot")["locked"], true);

        let (status, detail) = call(
            app.clone(),
            Method::PUT,
            "/api/metadata/ABC-001/locks/plot",
            Some(r#"{"locked":false}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["locked_fields"], serde_json::json!([]));

        let (status, body) = call(
            app.clone(),
            Method::PUT,
            "/api/metadata/ABC-001/locks/actors",
            Some(r#"{"locked":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_field");

        let (status, _) = call(
            app.clone(),
            Method::PUT,
            "/api/metadata/ZZZ-999/locks/title",
            Some(r#"{"locked":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tags_dedupe_and_remove() {
        let (_dir, app) = app(None).await;
        seeded_entry(&app).await;

        let (status, detail) = call(
            app.clone(),
            Method::POST,
            "/api/metadata/ABC-001/tags",
            Some(r#"{"name":"收藏"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["tags"], serde_json::json!(["收藏"]));

        let (_, detail) = call(
            app.clone(),
            Method::POST,
            "/api/metadata/ABC-001/tags",
            Some(r#"{"name":" 收藏 "}"#),
        )
        .await;
        assert_eq!(
            detail["tags"],
            serde_json::json!(["收藏"]),
            "同名标签不应重复"
        );

        let (_, detail) = call(
            app.clone(),
            Method::POST,
            "/api/metadata/ABC-001/tags",
            Some(r#"{"name":"待看"}"#),
        )
        .await;
        assert_eq!(detail["tags"], serde_json::json!(["收藏", "待看"]));

        let (status, detail) = call(
            app.clone(),
            Method::DELETE,
            "/api/metadata/ABC-001/tags/%E6%94%B6%E8%97%8F",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["tags"], serde_json::json!(["待看"]));

        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/metadata/ABC-001/tags",
            Some(r#"{"name":"   "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
    }

    #[tokio::test]
    async fn patch_rejects_empty_body_and_bad_date() {
        let (_dir, app) = app(None).await;
        seeded_entry(&app).await;

        let (status, body) = call(
            app.clone(),
            Method::PATCH,
            "/api/metadata/ABC-001",
            Some("{}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "empty_patch");

        let (status, body) = call(
            app.clone(),
            Method::PATCH,
            "/api/metadata/ABC-001",
            Some(r#"{"release_date":"2021/02/18"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
    }

    #[tokio::test]
    async fn schedule_lifecycle_round_trip() {
        let (_dir, app) = app(None).await;

        let (status, created) = call(
            app.clone(),
            Method::POST,
            "/api/schedules",
            Some(
                r#"{"kind":"scrape","payload":{"number":"SSIS-001"},"cron":"0 3 * * *","timezone":"Asia/Shanghai"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["cron"], "0 3 * * *");
        assert_eq!(created["timezone"], "Asia/Shanghai");
        assert_eq!(created["enabled"], true);
        assert!(created["next_fire_at"].is_string(), "下次触发时刻应可显示");
        let id = created["id"].as_i64().expect("应带标识");

        let (status, page) = call(app.clone(), Method::GET, "/api/schedules", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 1);

        // 停用: 不再派生新任务.
        let (status, updated) = call(
            app.clone(),
            Method::PATCH,
            &format!("/api/schedules/{id}"),
            Some(r#"{"enabled":false}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["enabled"], false);

        let (status, body) = call(
            app.clone(),
            Method::PATCH,
            &format!("/api/schedules/{id}"),
            Some("{}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "empty_patch");

        let (status, _) = call(
            app.clone(),
            Method::DELETE,
            &format!("/api/schedules/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = call(
            app.clone(),
            Method::DELETE,
            &format!("/api/schedules/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn schedule_rejects_malformed_input() {
        let (_dir, app) = app(None).await;

        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/schedules",
            Some(
                r#"{"kind":"scrape","payload":{},"cron":"99 * * * *","timezone":"Asia/Shanghai"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_cron");

        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/schedules",
            Some(r#"{"kind":"scrape","payload":{},"cron":"0 3 * * *","timezone":"Mars/Phobos"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_timezone");

        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/schedules",
            Some(r#"{"kind":"bogus","payload":{},"cron":"0 3 * * *","timezone":"Asia/Shanghai"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_task_kind");
    }

    #[tokio::test]
    async fn config_round_trip_reports_applied_and_restart() {
        let (_dir, app) = app(None).await;

        let (status, current) = call(app.clone(), Method::GET, "/api/config", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(current["hot"]["concurrency"], 4);

        // 以当前值为基准构造请求体, 冷配置部分原样带回, 不构成改动.
        let mut hot_only = current.clone();
        hot_only["hot"]["concurrency"] = serde_json::json!(2);
        hot_only["hot"]["request_timeout_secs"] = serde_json::json!(20);
        let hot_only_text = serde_json::to_string(&hot_only).expect("配置应可序列化");

        let (status, updated) = call(
            app.clone(),
            Method::PUT,
            "/api/config",
            Some(&hot_only_text),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["applied"], serde_json::json!(["concurrency"]));
        assert_eq!(updated["restart_required"], serde_json::json!([]));
        assert_eq!(updated["settings"]["hot"]["concurrency"], 2);

        let (_, reloaded) = call(app.clone(), Method::GET, "/api/config", None).await;
        assert_eq!(reloaded["hot"]["concurrency"], 2, "热配置应立即生效");

        // 冷配置: 只登记改动并提示重启, 当前值不变.
        let mut cold_change = hot_only;
        cold_change["cold"]["bind"] = serde_json::json!("0.0.0.0:9000");
        let cold_text = serde_json::to_string(&cold_change).expect("配置应可序列化");
        let (status, changed) =
            call(app.clone(), Method::PUT, "/api/config", Some(&cold_text)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(changed["restart_required"], serde_json::json!(["bind"]));
        assert_eq!(changed["applied"], serde_json::json!([]));
        assert_eq!(
            changed["settings"]["cold"]["bind"], "127.0.0.1:8787",
            "冷配置不应就地生效"
        );
    }

    #[tokio::test]
    async fn disabling_a_source_removes_it_from_the_active_set() {
        let (_dir, app) = app(None).await;

        let (_, current) = call(app.clone(), Method::GET, "/api/config", None).await;
        let mut body = current.clone();
        body["hot"]["enabled_sources"] = serde_json::json!(["dmm"]);
        let text = serde_json::to_string(&body).expect("配置应可序列化");
        let (status, updated) = call(app.clone(), Method::PUT, "/api/config", Some(&text)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            updated["applied"]
                .as_array()
                .expect("applied 应为数组")
                .contains(&serde_json::json!("enabled_sources")),
            "来源变化应报告为已生效"
        );

        let (_, options) = call(app.clone(), Method::GET, "/api/filters", None).await;
        let ids: Vec<&str> = options["sources"]
            .as_array()
            .expect("来源应为数组")
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();
        assert_eq!(ids, ["dmm"], "未列出的来源不应出现在可选项里");
    }

    #[tokio::test]
    async fn re_enabling_a_disabled_source_is_allowed() {
        let (_dir, app) = app(None).await;
        let (_, current) = call(app.clone(), Method::GET, "/api/config", None).await;

        let mut disabled = current.clone();
        disabled["hot"]["enabled_sources"] = serde_json::json!(["dmm"]);
        let text = serde_json::to_string(&disabled).expect("配置应可序列化");
        let (status, _) = call(app.clone(), Method::PUT, "/api/config", Some(&text)).await;
        assert_eq!(status, StatusCode::OK);

        // 白名单必须取全部登记来源: 取启用集的话, 这里会被判成未知来源.
        let mut restored = current.clone();
        restored["hot"]["enabled_sources"] = serde_json::json!(["javdb", "dmm"]);
        let text = serde_json::to_string(&restored).expect("配置应可序列化");
        let (status, body) = call(app.clone(), Method::PUT, "/api/config", Some(&text)).await;
        assert_eq!(status, StatusCode::OK, "停用后应能重新启用, 实际 {body}");

        let (_, options) = call(app.clone(), Method::GET, "/api/filters", None).await;
        let ids: Vec<&str> = options["sources"]
            .as_array()
            .expect("来源应为数组")
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();
        assert_eq!(ids, ["javdb", "dmm"], "顺序即聚合优先级");
    }

    #[tokio::test]
    async fn malformed_body_returns_unified_error() {
        let (_dir, app) = app(None).await;

        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/tasks",
            Some(r#"{"kind":"scrape""#),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["code"], "invalid_body",
            "请求体解析失败也要走统一错误体, 实际 {body}"
        );
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "错误说明不能为空"
        );
    }

    #[tokio::test]
    async fn request_log_records_timing_without_token() {
        let (_dir, app) = app(None).await;
        let (buffer, _guard) = capture_logs();

        let (status, _) = call(app, Method::GET, "/api/health?token=secret-value", None).await;
        assert_eq!(status, StatusCode::OK);

        let logged = String::from_utf8(buffer.lock().expect("应加锁").clone()).expect("应为文本");
        assert!(logged.contains("请求完成"), "应记录请求完成, 实际 {logged}");
        assert!(logged.contains("elapsed_ms"), "应记录耗时");
        assert!(logged.contains("status"), "应记录状态码");
        assert!(
            !logged.contains("secret-value"),
            "查询串里的令牌不应进日志, 实际 {logged}"
        );
    }

    #[tokio::test]
    async fn deletes_entry_with_related_data() {
        let (_dir, app) = app(None).await;
        call(
            app.clone(),
            Method::POST,
            "/api/metadata",
            Some(r#"{"number":"ABC-001","title":"标题"}"#),
        )
        .await;
        call(
            app.clone(),
            Method::PUT,
            "/api/metadata/ABC-001/locks/title",
            Some(r#"{"locked":true}"#),
        )
        .await;
        call(
            app.clone(),
            Method::POST,
            "/api/metadata/ABC-001/tags",
            Some(r#"{"name":"收藏"}"#),
        )
        .await;

        let (_, detail) = call(app.clone(), Method::GET, "/api/metadata/ABC-001", None).await;
        assert_eq!(detail["related"]["locks"], 1, "删除前应告知关联的锁");
        assert_eq!(detail["related"]["tags"], 1, "删除前应告知关联的标签");

        let (status, body) = call(app.clone(), Method::DELETE, "/api/metadata/ABC-001", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_null(), "删除不返回正文");

        let (status, _) = call(app.clone(), Method::GET, "/api/metadata/ABC-001", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "删除后详情应 404");

        let (status, _) = call(app.clone(), Method::DELETE, "/api/metadata/ABC-001", None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "重复删除应 404 而不是静默成功"
        );
    }

    #[tokio::test]
    async fn deletes_entry_without_related_data() {
        let (_dir, app) = app(None).await;
        call(
            app.clone(),
            Method::POST,
            "/api/metadata",
            Some(r#"{"number":"ABC-002","title":"标题"}"#),
        )
        .await;

        let (_, detail) = call(app.clone(), Method::GET, "/api/metadata/ABC-002", None).await;
        assert_eq!(detail["related"]["actors"], 0);
        assert_eq!(detail["related"]["resources"], 0);

        let (status, _) = call(app.clone(), Method::DELETE, "/api/metadata/ABC-002", None).await;

        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn genres_are_returned_and_filterable() {
        let (_dir, app, state) = app_state(None).await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let genres = vec!["ドラマ".to_owned(), "美少女".to_owned()];
        state
            .runtime
            .metadata
            .apply_scraped(&ScrapeWrite {
                number: &number,
                input: MetadataInput {
                    genres: Some(&genres),
                    ..MetadataInput::default()
                },
                field_sources: std::collections::BTreeMap::from([(
                    "genres".to_owned(),
                    "dmm".to_owned(),
                )]),
            })
            .await
            .expect("应写入成功");

        let (status, detail) = call(app.clone(), Method::GET, "/api/metadata/ABC-001", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            detail["genres"],
            serde_json::json!(["ドラマ", "美少女"]),
            "详情应返回分类"
        );

        let (_, options) = call(app.clone(), Method::GET, "/api/filters", None).await;
        let listed: Vec<&str> = options["genres"]
            .as_array()
            .expect("分类应为数组")
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert!(listed.contains(&"ドラマ"), "筛选选项应含全部分类");

        let (status, page) = call(
            app.clone(),
            Method::GET,
            "/api/metadata?genre=%E7%BE%8E%E5%B0%91%E5%A5%B3",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 1, "按分类筛选应命中");

        let (status, page) = call(
            app.clone(),
            Method::GET,
            "/api/metadata?genre=nowhere",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 0, "分类是开放取值, 未知分类得到空列表");
    }

    #[tokio::test]
    async fn config_rejects_malformed_body_with_unified_error() {
        let (_dir, app) = app(None).await;
        let (_, current) = call(app.clone(), Method::GET, "/api/config", None).await;

        // 未知字段: 解析阶段就拒绝, 错误说明指出字段名.
        let mut unknown_field = current.clone();
        unknown_field["hot"]["typo"] = serde_json::json!(1);
        let text = serde_json::to_string(&unknown_field).expect("配置应可序列化");
        let (status, body) = call(app.clone(), Method::PUT, "/api/config", Some(&text)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["code"], "invalid_body",
            "请求体解析失败也要走统一错误体"
        );
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("typo"),
            "错误说明应指出具体字段名, 实际 {}",
            body["message"]
        );

        // 缺字段: 同样拒绝, 而不是用默认值把未提交的配置覆盖掉.
        let (status, body) = call(
            app.clone(),
            Method::PUT,
            "/api/config",
            Some(r#"{"cold":{"data_dir":"/tmp","bind":"127.0.0.1:0","api_token":null},"hot":{"concurrency":4,"request_timeout_secs":20,"enabled_sources":[]}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_body");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("scan"),
            "缺字段时说明应指出缺的是哪一项, 实际 {}",
            body["message"]
        );
    }

    #[tokio::test]
    async fn config_rejects_out_of_range_value_and_unknown_source() {
        let (_dir, app) = app(None).await;

        let (_, current) = call(app.clone(), Method::GET, "/api/config", None).await;

        let mut out_of_range = current.clone();
        out_of_range["hot"]["concurrency"] = serde_json::json!(99);
        let text = serde_json::to_string(&out_of_range).expect("配置应可序列化");
        let (status, body) = call(app.clone(), Method::PUT, "/api/config", Some(&text)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_config");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("concurrency"),
            "错误说明应指出越界字段名"
        );

        let mut unknown_source = current.clone();
        unknown_source["hot"]["enabled_sources"] = serde_json::json!(["nowhere"]);
        let text = serde_json::to_string(&unknown_source).expect("配置应可序列化");
        let (status, body) = call(app.clone(), Method::PUT, "/api/config", Some(&text)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("nowhere")
        );
    }

    #[tokio::test]
    async fn start_with_port_zero_picks_an_ephemeral_port() {
        let state = {
            let dir = tempfile::tempdir().expect("应创建临时目录");
            let settings = Settings {
                cold: ColdSettings {
                    data_dir: dir.path().to_path_buf(),
                    bind: "127.0.0.1:0".to_owned(),
                    ..ColdSettings::default()
                },
                ..Settings::default()
            };
            // 目录句柄要活到断言结束, 先拿地址再释放.
            let state = crate::boot(settings).await.expect("应启动");
            let (address, _handle) = crate::start(state.clone()).await.expect("应绑定端口");

            assert!(address.port() > 0, "端口 0 应被系统替换为实际端口");
            assert!(address.ip().is_loopback());
            address
        };
        let _ = state;
    }

    #[tokio::test]
    async fn missing_frontend_shows_readable_hint() {
        let (_dir, app) = app(None).await;

        let response = app
            .oneshot(Request::get("/").body(Body::empty()).expect("应构造请求"))
            .await
            .expect("应得到响应");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("应读取响应体");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("前端产物不存在"),
            "提示应说明缺什么, 实际 {text}"
        );
        assert!(text.contains("build-ui"), "提示应给出补救命令");
    }

    #[tokio::test]
    async fn filter_options_come_from_registry() {
        let (_dir, app) = app(None).await;

        let (status, options) = call(app, Method::GET, "/api/filters", None).await;

        assert_eq!(status, StatusCode::OK);
        let types = options["content_types"].as_array().expect("类型应为数组");
        assert_eq!(types.len(), 7, "每一种条目类型都要出现在可选项里");
        assert!(types.iter().all(|item| {
            item["label"]
                .as_str()
                .is_some_and(|label| !label.is_empty())
        }));

        let sources = options["sources"].as_array().expect("来源应为数组");
        let ids: Vec<&str> = sources
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();
        assert_eq!(ids, ["dmm", "javdb"]);
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

    #[tokio::test]
    async fn library_registration_rejects_unusable_paths_with_reason() {
        let (dir, app) = app(None).await;

        // 不存在的路径.
        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(r#"{"path": "/nowhere/amane-missing"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_path");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("不可用"),
            "应说明路径不可用, 实际 {}",
            body["message"]
        );

        // 是文件不是目录.
        let file = dir.path().join("plain.txt");
        std::fs::write(&file, "x").expect("应写入文件");
        let payload = serde_json::json!({ "path": file.display().to_string() });
        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(&serde_json::to_string(&payload).expect("应序列化")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("不是目录"),
            "应说明不是目录, 实际 {}",
            body["message"]
        );

        // 空路径.
        let (status, _) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(r#"{"path": "  "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn library_scan_round_trip_and_files_endpoint() {
        let (dir, app, state) = app_state(None).await;
        let worker = state.runtime.tasks.start();

        // 建一个媒体库目录: 一个媒体文件、一个附属文件、一个空子目录.
        let library = dir.path().join("library");
        std::fs::create_dir_all(library.join("empty")).expect("应创建库目录");
        std::fs::write(library.join("film.mp4"), vec![0u8; 64]).expect("应写入媒体");
        std::fs::write(library.join("note.txt"), "说明").expect("应写入附属");

        let payload = serde_json::json!({ "path": library.display().to_string() });
        let (status, created) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(&serde_json::to_string(&payload).expect("应序列化")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(created["path"].as_str().is_some());
        let id = created["id"].as_i64().expect("应带标识");

        // 重复登记同一目录应说明原因.
        let (status, body) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(&serde_json::to_string(&payload).expect("应序列化")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("已登记"),
            "应说明已登记, 实际 {}",
            body["message"]
        );

        // 派生扫描并等它完成.
        let (status, task) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/scan"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let task_id = task["id"].as_i64().expect("应带任务标识");
        let mut settled = serde_json::Value::Null;
        for _ in 0..200 {
            let (_, current) = call(
                app.clone(),
                Method::GET,
                &format!("/api/tasks/{task_id}"),
                None,
            )
            .await;
            if current["status"] == "done" || current["status"] == "failed" {
                settled = current;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            settled["status"], "done",
            "扫描应成功: {}",
            settled["error"]
        );

        // 统计: 媒体 1、未识别 1、残留目录 1.
        let (_, page) = call(app.clone(), Method::GET, "/api/libraries", None).await;
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["counts"]["media"], 1);
        assert_eq!(page["items"][0]["counts"]["unrecognized"], 1);
        assert_eq!(page["items"][0]["counts"]["leftover_dir"], 1);

        // 按类别过滤, 未识别文件不静默消失.
        let (_, files) = call(
            app.clone(),
            Method::GET,
            &format!("/api/libraries/{id}/files?kind=unrecognized"),
            None,
        )
        .await;
        assert_eq!(files["total"], 1);
        assert_eq!(files["items"][0]["path"], "note.txt");

        // 未知类别应被拒绝.
        let (status, body) = call(
            app.clone(),
            Method::GET,
            &format!("/api/libraries/{id}/files?kind=bogus"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_scan_kind");

        // 删除登记后 404.
        let (status, _) = call(
            app.clone(),
            Method::DELETE,
            &format!("/api/libraries/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(
            app,
            Method::POST,
            &format!("/api/libraries/{id}/scan"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        state.runtime.tasks.shutdown();
        worker.abort();
    }

    /// 整理预览: 只读, 不建目录不移动文件; 三类冲突与 unmatched 计数都要出现.
    #[tokio::test]
    async fn organize_preview_is_readonly_and_reports_conflicts() {
        let (dir, app, state) = app_state(None).await;

        // 默认模板 {maker}/{number}/{number}.{ext}: maker 空则目录段折叠.
        // 库目录: 一个目标被占 (exists)、一个已就位 (same_path)、一个无番号 (unmatched).
        let library = dir.path().join("library");
        std::fs::create_dir_all(library.join("ABC-123")).expect("应创建目录");
        std::fs::create_dir_all(library.join("XYZ-999")).expect("应创建目录");
        std::fs::create_dir_all(library.join("downloads")).expect("应创建目录");
        std::fs::write(library.join("downloads/ABC-123.mp4"), vec![0u8; 64]).expect("应写入源");
        std::fs::write(library.join("ABC-123/ABC-123.mp4"), vec![0u8; 64]).expect("应写入占用者");
        std::fs::write(library.join("XYZ-999/XYZ-999.mp4"), vec![0u8; 64]).expect("应写入就位者");
        std::fs::write(library.join("plain.mp4"), vec![0u8; 64]).expect("应写入无番号文件");

        let payload = serde_json::json!({ "path": library.display().to_string() });
        let (_, created) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(&serde_json::to_string(&payload).expect("应序列化")),
        )
        .await;
        let id = created["id"].as_i64().expect("应带标识");

        // 预览以扫描索引为输入; 直接登记四个 media 文件, 不跑扫描任务.
        use amane_core::LibraryFile;
        state
            .runtime
            .libraries
            .replace_files(
                id,
                &[
                    LibraryFile::new("downloads/ABC-123.mp4", amane_core::ScanKind::Media),
                    LibraryFile::new("XYZ-999/XYZ-999.mp4", amane_core::ScanKind::Media),
                    LibraryFile::new("plain.mp4", amane_core::ScanKind::Media),
                    LibraryFile::new("notes/readme.txt", amane_core::ScanKind::Unrecognized),
                ],
                &[],
            )
            .await
            .expect("应写入扫描索引");

        // 两次预览结果必须一致.
        let (_, first) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/organize/preview"),
            None,
        )
        .await;
        let (_, second) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/organize/preview"),
            None,
        )
        .await;
        assert_eq!(first, second, "预览必须可重复");

        assert_eq!(first["unmatched"], 1, "无番号文件计入 unmatched");
        let items = first["items"].as_array().expect("items 应为数组");
        assert_eq!(items.len(), 2, "只有可配对番号的文件进入预览");

        let abc = items
            .iter()
            .find(|item| item["number"] == "ABC-123")
            .expect("应含 ABC-123");
        assert_eq!(abc["source_path"], "downloads/ABC-123.mp4");
        assert_eq!(
            abc["target_path"], "ABC-123/ABC-123.mp4",
            "maker 空时目录段整段折叠"
        );
        assert_eq!(abc["conflict"], "exists", "目标已有文件应标 exists");

        let xyz = items
            .iter()
            .find(|item| item["number"] == "XYZ-999")
            .expect("应含 XYZ-999");
        assert_eq!(xyz["conflict"], "same_path", "源与目标相同应标 same_path");

        // number_mismatch: 目标路径被另一个番号的整理记录占用.
        state
            .runtime
            .organize
            .register(id, "ABC-123/ABC-123.mp4", "XYZ-999", "somewhere.mp4")
            .await
            .expect("应登记占用");
        let (_, third) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/organize/preview"),
            None,
        )
        .await;
        let items = third["items"].as_array().expect("items 应为数组");
        let abc = items
            .iter()
            .find(|item| item["number"] == "ABC-123")
            .expect("应含 ABC-123");
        assert_eq!(
            abc["conflict"], "number_mismatch",
            "登记占用者是别的番号应标 number_mismatch"
        );

        // 预览前后磁盘状态不变: 目录结构与文件数一致 (downloads、ABC-123、XYZ-999、plain.mp4).
        let entries: Vec<_> = std::fs::read_dir(&library).expect("应读取库目录").collect();
        assert_eq!(entries.len(), 4, "预览不得创建目录或移动文件");

        // not_found.
        let (status, _) = call(
            app.clone(),
            Method::POST,
            "/api/libraries/9999/organize/preview",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        state.runtime.tasks.shutdown();
    }

    /// 整理执行: 端到端. 目标路径与预览一致, 字幕随配, 登记写入, 跳过项在日志可读.
    #[tokio::test]
    async fn organize_execute_moves_files_end_to_end() {
        let (dir, app, state) = app_state(None).await;
        let worker = state.runtime.tasks.start();

        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).expect("应创建库目录");
        std::fs::write(library.join("ABC-123.mp4"), vec![0u8; 64]).expect("应写入视频");
        std::fs::write(library.join("ABC-123.srt"), b"sub").expect("应写入字幕");
        let payload = serde_json::json!({ "path": library.display().to_string() });
        let (_, created) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(&serde_json::to_string(&payload).expect("应序列化")),
        )
        .await;
        let id = created["id"].as_i64().expect("应带标识");

        use amane_core::LibraryFile;
        state
            .runtime
            .libraries
            .replace_files(
                id,
                &[LibraryFile::new("ABC-123.mp4", amane_core::ScanKind::Media)],
                &[],
            )
            .await
            .expect("应写入索引");

        // 未知处置被拒绝.
        let (status, body) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/organize/execute"),
            Some(r#"{"choices": {"a.mp4": "nuke"}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_action");

        // 执行.
        let (status, task) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/organize/execute"),
            Some("{}"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let task_id = task["id"].as_i64().expect("应带任务标识");
        let mut settled = serde_json::Value::Null;
        for _ in 0..200 {
            let (_, current) = call(
                app.clone(),
                Method::GET,
                &format!("/api/tasks/{task_id}"),
                None,
            )
            .await;
            if current["status"] == "done" || current["status"] == "failed" {
                settled = current;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            settled["status"], "done",
            "整理应成功: {}",
            settled["error"]
        );

        // 目标路径与预览一致 (maker 空折叠): ABC-123/ABC-123.mp4 + 字幕.
        assert!(library.join("ABC-123/ABC-123.mp4").exists());
        assert!(library.join("ABC-123/ABC-123.srt").exists());
        assert!(!library.join("ABC-123.mp4").exists());

        // 日志区分成功与跳过: 已移动行存在.
        let (_, log) = call(
            app.clone(),
            Method::GET,
            &format!("/api/tasks/{task_id}/log"),
            None,
        )
        .await;
        let lines = log["items"].as_array().expect("日志应为数组");
        let texts: Vec<&str> = lines
            .iter()
            .filter_map(|line| line["message"].as_str())
            .collect();
        assert!(
            texts.iter().any(|text| text.contains("已移动")),
            "成功项应有日志, 实际 {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("整理完成")),
            "应有汇总日志"
        );

        // 执行后的预览: 源已移走, 索引条目计为 stale 而不是误报冲突.
        let (_, preview) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/organize/preview"),
            None,
        )
        .await;
        assert_eq!(preview["stale"], 1, "已整理的源应计为过期索引");
        assert_eq!(
            preview["items"].as_array().expect("items 应为数组").len(),
            0,
            "过期条目不应进入冲突列表"
        );

        state.runtime.tasks.shutdown();
        worker.abort();
    }

    /// 清理与回收端到端: 清单 → 执行 → 回收列表 → 恢复 → 目标已存在 409.
    #[tokio::test]
    async fn cleanup_and_restore_round_trip() {
        let (dir, app, state) = app_state(None).await;
        let worker = state.runtime.tasks.start();

        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).expect("应创建库目录");
        std::fs::write(library.join("ads.mp4"), vec![0u8; 128]).expect("应写入无效文件");
        let payload = serde_json::json!({ "path": library.display().to_string() });
        let (_, created) = call(
            app.clone(),
            Method::POST,
            "/api/libraries",
            Some(&serde_json::to_string(&payload).expect("应序列化")),
        )
        .await;
        let id = created["id"].as_i64().expect("应带标识");

        use amane_core::LibraryFile;
        state
            .runtime
            .libraries
            .replace_files(
                id,
                &[
                    LibraryFile::unwanted(
                        "ads.mp4",
                        amane_core::UnwantedRule::Blacklist,
                        "(?i)ads".to_owned(),
                    ),
                    LibraryFile::new("film.mp4", amane_core::ScanKind::Media),
                ],
                &[],
            )
            .await
            .expect("应写入索引");
        std::fs::write(library.join("film.mp4"), vec![0u8; 64]).expect("应写入媒体");

        // 清单: 一项, 带大小与原因.
        let (_, manifest) = call(
            app.clone(),
            Method::GET,
            &format!("/api/libraries/{id}/cleanup"),
            None,
        )
        .await;
        assert_eq!(
            manifest["items"].as_array().expect("items 应为数组").len(),
            1
        );
        assert_eq!(manifest["items"][0]["path"], "ads.mp4");
        assert_eq!(manifest["items"][0]["size"], 128);
        assert_eq!(manifest["items"][0]["reason"], "blacklist: (?i)ads");
        assert_eq!(manifest["total_size"], 128);

        // 执行清理任务.
        let (status, task) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/cleanup/execute"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let task_id = task["id"].as_i64().expect("应带任务标识");
        let mut settled = serde_json::Value::Null;
        for _ in 0..200 {
            let (_, current) = call(
                app.clone(),
                Method::GET,
                &format!("/api/tasks/{task_id}"),
                None,
            )
            .await;
            if current["status"] == "done" || current["status"] == "failed" {
                settled = current;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            settled["status"], "done",
            "清理应成功: {}",
            settled["error"]
        );

        // 原路径不再存在; 回收目录里可查, 占用可统计.
        assert!(!library.join("ads.mp4").exists());
        let (_, trash) = call(
            app.clone(),
            Method::GET,
            &format!("/api/libraries/{id}/trash"),
            None,
        )
        .await;
        assert_eq!(trash["items"][0]["path"], "ads.mp4");
        assert_eq!(trash["items"][0]["size"], 128);
        assert_eq!(trash["total_size"], 128);

        // 恢复: 文件回到原路径, 回收目录清空.
        let (_, restored) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/trash/restore"),
            Some(r#"{"path": "ads.mp4"}"#),
        )
        .await;
        assert_eq!(
            restored["items"].as_array().expect("items 应为数组").len(),
            0,
            "恢复后回收目录应清空"
        );
        assert!(library.join("ads.mp4").exists());

        // 再次清理后制造原路径占用: 恢复被拒绝, 409 说明原因且不覆盖.
        // 恢复回来的条目按未识别登记 (原始归类随清理删除); 重新扫描后再次归入清理清单.
        // 这里直接模拟重扫结果: ads.mp4 又被黑名单命中, 然后派生第二次清理.
        state
            .runtime
            .libraries
            .replace_files(
                id,
                &[
                    LibraryFile::unwanted(
                        "ads.mp4",
                        amane_core::UnwantedRule::Blacklist,
                        "(?i)ads".to_owned(),
                    ),
                    LibraryFile::new("film.mp4", amane_core::ScanKind::Media),
                ],
                &[],
            )
            .await
            .expect("应重写索引");
        let (_, task) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/cleanup/execute"),
            None,
        )
        .await;
        let task_id = task["id"].as_i64().expect("应带任务标识");
        for _ in 0..200 {
            let (_, current) = call(
                app.clone(),
                Method::GET,
                &format!("/api/tasks/{task_id}"),
                None,
            )
            .await;
            if current["status"] == "done" || current["status"] == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            !library.join("ads.mp4").exists(),
            "第二次清理应把文件移入回收目录"
        );
        std::fs::write(library.join("ads.mp4"), b"occupied").expect("应写入占用者");
        let (status, body) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/trash/restore"),
            Some(r#"{"path": "ads.mp4"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("已存在"),
            "应说明原因, 实际 {}",
            body["message"]
        );
        assert!(library.join("ads.mp4").exists(), "占用者不得被覆盖");

        // 路径遍历防御.
        let (status, body) = call(
            app.clone(),
            Method::POST,
            &format!("/api/libraries/{id}/trash/restore"),
            Some(r#"{"path": "../../etc/passwd"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_path");

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
            "/api/metadata/{number}/locks/{field}",
            "/api/metadata/{number}/tags",
            "/api/metadata/{number}/tags/{name}",
            "/api/number/extract",
            "/api/config",
            "/api/config/schema",
            "/api/filters",
            "/api/tasks",
            "/api/tasks/{id}",
            "/api/tasks/{id}/cancel",
            "/api/tasks/{id}/retry",
            "/api/tasks/{id}/log",
            "/api/tasks/{id}/children",
            "/api/schedules",
            "/api/schedules/{id}",
            "/api/libraries",
            "/api/libraries/{id}",
            "/api/libraries/{id}/scan",
            "/api/libraries/{id}/files",
            "/api/libraries/{id}/organize/preview",
            "/api/libraries/{id}/organize/execute",
            "/api/libraries/{id}/cleanup",
            "/api/libraries/{id}/cleanup/execute",
            "/api/libraries/{id}/trash",
            "/api/libraries/{id}/trash/restore",
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
        assert_eq!(status, StatusCode::OK, "接口路由不应被前端回退忽略");
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
