use amane_core::TaskLogger;
use amane_core::config::Settings;
use amane_core::{MediaNumber, TaskFilter, TaskKind, TaskServiceError, TaskSpec, TaskStatus};
use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};

use crate::dto::{
    CreateMetadataRequest, CreateTaskRequest, ExtractedNumber, HealthResponse, ListQuery,
    MetadataDto, MetadataPage, TaskDto, TaskListQuery, TaskLogEntry, TaskLogPage, TaskPage,
};
use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/metadata", get(list_metadata).post(create_metadata))
        .route("/metadata/{number}", get(get_metadata))
        .route("/number/extract", get(extract_number))
        .route("/config/schema", get(config_schema))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", get(get_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/retry", post(retry_task))
        .route("/tasks/{id}/log", get(task_log))
        .route("/tasks/{id}/children", get(task_children))
}

#[utoipa::path(get, path = "/api/health", tag = "system", responses((status = 200, body = HealthResponse), (status = 401, body = crate::error::ErrorBody)))]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

#[utoipa::path(
    get,
    path = "/api/metadata",
    tag = "metadata",
    params(ListQuery),
    responses((status = 200, body = MetadataPage), (status = 401, body = crate::error::ErrorBody))
)]
pub async fn list_metadata(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MetadataPage>, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let repository = &state.runtime.metadata;

    let total = repository.count().await?;
    let items = repository
        .list(limit, offset)
        .await?
        .into_iter()
        .map(MetadataDto::from)
        .collect();

    Ok(Json(MetadataPage { total, items }))
}

#[utoipa::path(
    post,
    path = "/api/metadata",
    tag = "metadata",
    request_body = CreateMetadataRequest,
    responses(
        (status = 201, body = MetadataDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn create_metadata(
    State(state): State<AppState>,
    Json(payload): Json<CreateMetadataRequest>,
) -> Result<(StatusCode, Json<MetadataDto>), ApiError> {
    let number = MediaNumber::parse(&payload.number)?;
    let row = state
        .runtime
        .metadata
        .upsert(&number, payload.title.as_deref())
        .await?;

    Ok((StatusCode::CREATED, Json(MetadataDto::from(row))))
}

#[utoipa::path(
    get,
    path = "/api/metadata/{number}",
    tag = "metadata",
    params(("number" = String, Path, description = "番号")),
    responses(
        (status = 200, body = MetadataDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn get_metadata(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<Json<MetadataDto>, ApiError> {
    let number = MediaNumber::parse(&number)?;
    state
        .runtime
        .metadata
        .find(&number)
        .await?
        .map(MetadataDto::from)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound {
            what: format!("影片 {number} 不存在"),
        })
}

#[utoipa::path(
    get,
    path = "/api/number/extract",
    tag = "parsing",
    params(crate::dto::ExtractQuery),
    responses(
        (status = 200, body = ExtractedNumber),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn extract_number(
    Query(query): Query<crate::dto::ExtractQuery>,
) -> Result<Json<ExtractedNumber>, ApiError> {
    let number = MediaNumber::extract(&query.text).ok_or_else(|| ApiError::NotFound {
        what: format!("无法从「{}」识别番号", query.text),
    })?;

    Ok(Json(ExtractedNumber {
        number: number.as_str().to_owned(),
        content_type: number.content_type().as_str().to_owned(),
        prefix: number.prefix().to_owned(),
    }))
}

/// 配置表单 schema.
///
/// 由 `schemars` 从内核的配置类型生成, 保留 `x-widget` 等表单提示;
/// 文档侧声明为通用对象, 避免与 `utoipa` 的类型 schema 互相覆盖.
#[utoipa::path(get, path = "/api/config/schema", tag = "config", responses(
        (status = 200, body = serde_json::Value, description = "配置项的 JSON Schema"),
        (status = 401, body = crate::error::ErrorBody)
    ))]
pub async fn config_schema() -> Json<serde_json::Value> {
    Json(serde_json::to_value(schemars::schema_for!(Settings)).expect("配置 schema 可序列化"))
}

#[utoipa::path(
    get,
    path = "/api/tasks",
    tag = "task",
    params(TaskListQuery),
    responses(
        (status = 200, body = TaskPage),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<TaskPage>, ApiError> {
    let status = match query.status.as_deref() {
        None => None,
        Some(value) => Some(TaskStatus::parse(value).ok_or_else(|| ApiError::Invalid {
            code: "invalid_task_status",
            message: format!("未知任务状态: {value}"),
        })?),
    };
    let filter = TaskFilter { status };
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let repository = state.runtime.tasks.repository();

    let total = repository.count(filter).await?;
    let items = repository
        .list(filter, limit, offset)
        .await?
        .into_iter()
        .map(TaskDto::from)
        .collect();

    Ok(Json(TaskPage { total, items }))
}

#[utoipa::path(
    post,
    path = "/api/tasks",
    tag = "task",
    request_body = CreateTaskRequest,
    responses(
        (status = 202, body = TaskDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn create_task(
    State(state): State<AppState>,
    Json(payload): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<TaskDto>), ApiError> {
    let kind = TaskKind::parse(&payload.kind).ok_or_else(|| ApiError::Invalid {
        code: "invalid_task_kind",
        message: format!("未知任务类型: {}", payload.kind),
    })?;

    let spec = TaskSpec {
        kind,
        payload: payload.payload,
        priority: payload.priority,
    };
    let task = state
        .runtime
        .tasks
        .submit(spec)
        .await
        .map_err(map_task_error)?;

    Ok((StatusCode::ACCEPTED, Json(TaskDto::from(task))))
}

#[utoipa::path(
    get,
    path = "/api/tasks/{id}",
    tag = "task",
    params(("id" = i64, Path, description = "任务标识")),
    responses(
        (status = 200, body = TaskDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TaskDto>, ApiError> {
    state
        .runtime
        .tasks
        .repository()
        .find(id)
        .await?
        .map(TaskDto::from)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound {
            what: format!("任务 {id} 不存在"),
        })
}

#[utoipa::path(
    post,
    path = "/api/tasks/{id}/cancel",
    tag = "task",
    params(("id" = i64, Path, description = "任务标识")),
    responses(
        (status = 200, body = TaskDto),
        (status = 409, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TaskDto>, ApiError> {
    let task = state
        .runtime
        .tasks
        .cancel(id)
        .await
        .map_err(map_task_error)?;

    Ok(Json(TaskDto::from(task)))
}

#[utoipa::path(
    post,
    path = "/api/tasks/{id}/retry",
    tag = "task",
    params(("id" = i64, Path, description = "任务标识")),
    responses(
        (status = 202, body = TaskDto),
        (status = 409, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn retry_task(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<(StatusCode, Json<TaskDto>), ApiError> {
    let task = state
        .runtime
        .tasks
        .retry(id)
        .await
        .map_err(map_task_error)?;

    Ok((StatusCode::ACCEPTED, Json(TaskDto::from(task))))
}

#[utoipa::path(
    get,
    path = "/api/tasks/{id}/log",
    tag = "task",
    params(("id" = i64, Path, description = "任务标识")),
    responses(
        (status = 200, body = TaskLogPage),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn task_log(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TaskLogPage>, ApiError> {
    if state.runtime.tasks.repository().find(id).await?.is_none() {
        return Err(ApiError::NotFound {
            what: format!("任务 {id} 不存在"),
        });
    }

    let lines = TaskLogger::read(state.runtime.tasks.log_root(), id)
        .await
        .map_err(|error| ApiError::Internal(format!("读取任务日志失败: {error}")))?;

    Ok(Json(TaskLogPage {
        items: lines.into_iter().map(TaskLogEntry::from).collect(),
    }))
}

fn map_task_error(error: TaskServiceError) -> ApiError {
    match error {
        TaskServiceError::Repository(inner) => inner.into(),
        TaskServiceError::NotCancellable(id) => ApiError::Conflict {
            message: format!("任务 {id} 不在可取消状态"),
        },
    }
}

/// 后继任务. 用于在界面上展开一次刮削实际派生了哪些步骤.
#[utoipa::path(
    get,
    path = "/api/tasks/{id}/children",
    tag = "task",
    params(("id" = i64, Path, description = "任务标识")),
    responses(
        (status = 200, body = TaskPage),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn task_children(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TaskPage>, ApiError> {
    let repository = state.runtime.tasks.repository();
    if repository.find(id).await?.is_none() {
        return Err(ApiError::NotFound {
            what: format!("任务 {id} 不存在"),
        });
    }

    let rows = repository.children(id).await?;
    let total = i64::try_from(rows.len()).unwrap_or(i64::MAX);

    Ok(Json(TaskPage {
        total,
        items: rows.into_iter().map(TaskDto::from).collect(),
    }))
}
