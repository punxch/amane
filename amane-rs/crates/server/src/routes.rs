use amane_core::TaskLogger;
use amane_core::config::Settings;
use amane_core::{
    MediaNumber, MetadataFilter, MetadataInput, TaskFilter, TaskKind, TaskServiceError, TaskSpec,
    TaskStatus,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use crate::dto::{
    ContentTypeOption, CreateMetadataRequest, CreateTaskRequest, ExtractedNumber, FilterOptions,
    HealthResponse, ListQuery, MetadataDto, MetadataPage, SourceOption, TaskDto, TaskListQuery,
    TaskLogEntry, TaskLogPage, TaskPage,
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
        .route("/filters", get(filter_options))
        .route("/number/extract", get(extract_number))
        .route("/config/schema", get(config_schema))
        .route("/resources/{id}", get(get_resource))
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
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<MetadataPage>, ApiError> {
    let Query(query) = parse_query(query)?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let filter = metadata_filter(&state, &query)?;
    let repository = &state.runtime.metadata;

    // 总数与列表用同一套条件, 否则翻到最后一页会出现空白页.
    let total = repository.count(&filter).await?;
    let items = repository
        .list(&filter, limit, offset)
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
    let input = MetadataInput {
        title: payload.title.as_deref(),
        ..MetadataInput::default()
    };
    let row = state.runtime.metadata.upsert(&number, &input).await?;

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
    let row = state
        .runtime
        .metadata
        .find(&number)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            what: format!("影片 {number} 不存在"),
        })?;

    let mut dto = MetadataDto::from(row);
    dto.actors = state.runtime.metadata.actors(dto.id).await?;
    dto.sources = state
        .runtime
        .metadata
        .snapshots(dto.id)
        .await?
        .into_iter()
        .map(|(source_id, _payload)| source_id)
        .collect();

    Ok(Json(dto))
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
    query: Result<Query<crate::dto::ExtractQuery>, QueryRejection>,
) -> Result<Json<ExtractedNumber>, ApiError> {
    let Query(query) = parse_query(query)?;
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
    query: Result<Query<TaskListQuery>, QueryRejection>,
) -> Result<Json<TaskPage>, ApiError> {
    let Query(query) = parse_query(query)?;
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

/// 查询参数解析失败也走统一错误体: 框架默认的纯文本 400 会让界面拿不到错误码.
fn parse_query<T>(query: Result<Query<T>, QueryRejection>) -> Result<Query<T>, ApiError> {
    query.map_err(|error| ApiError::Invalid {
        code: "invalid_query",
        message: format!("查询参数不合法: {error}"),
    })
}

/// 校验并转换筛选参数. 取值域有界的 (类型、来源) 在这里拒绝未知值,
/// 而不是静默返回空列表, 否则拼错一个字母看起来就像没有数据.
fn metadata_filter(state: &AppState, query: &ListQuery) -> Result<MetadataFilter, ApiError> {
    let content_type = match query.content_type.as_deref() {
        None => None,
        Some("") => None,
        Some(value) => {
            Some(
                amane_core::ContentType::parse(value).ok_or_else(|| ApiError::Invalid {
                    code: "invalid_content_type",
                    message: format!("未知类型: {value}"),
                })?,
            )
        }
    };

    let source = match query.source.as_deref() {
        None => None,
        Some("") => None,
        Some(value) => {
            let known = state
                .runtime
                .sources
                .iter()
                .any(|source| source.descriptor().id == value);
            if !known {
                return Err(ApiError::Invalid {
                    code: "unknown_source",
                    message: format!("未知来源: {value}"),
                });
            }
            Some(value.to_owned())
        }
    };

    let keyword = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    Ok(MetadataFilter {
        content_type,
        source,
        keyword,
    })
}

/// 筛选下拉框的可选项: 类型是封闭枚举, 来源取当前登记表.
#[utoipa::path(
    get,
    path = "/api/filters",
    tag = "metadata",
    responses(
        (status = 200, body = FilterOptions),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn filter_options(State(state): State<AppState>) -> Json<FilterOptions> {
    let content_types = amane_core::ContentType::ALL
        .into_iter()
        .map(|value| ContentTypeOption {
            value: value.as_str().to_owned(),
            label: value.label().to_owned(),
        })
        .collect();
    let sources = state
        .runtime
        .sources
        .iter()
        .map(|source| SourceOption {
            id: source.descriptor().id.to_owned(),
            label: source.descriptor().label.to_owned(),
        })
        .collect();

    Json(FilterOptions {
        content_types,
        sources,
    })
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

/// 资源内容. 地址即标识, 内容不可变, 因此可长期缓存.
#[utoipa::path(
    get,
    path = "/api/resources/{id}",
    tag = "resource",
    params(("id" = i64, Path, description = "资源标识")),
    responses(
        (status = 200, description = "资源内容"),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn get_resource(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let resource = state
        .runtime
        .resources
        .repository()
        .find(id)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            what: format!("资源 {id} 不存在"),
        })?;

    let bytes = state
        .runtime
        .resources
        .read(&resource)
        .await
        .map_err(|error| ApiError::Internal(format!("读取资源文件失败: {error}")))?;

    Ok((
        [
            (CONTENT_TYPE, resource.content_type),
            (
                CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_owned(),
            ),
        ],
        bytes,
    )
        .into_response())
}
