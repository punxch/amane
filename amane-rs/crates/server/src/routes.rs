use amane_core::TaskLogger;
use amane_core::config::Settings;
use amane_core::{
    LockableField, MediaNumber, MetadataFilter, MetadataInput, SchedulePatch, ScheduleSpec,
    TaskFilter, TaskKind, TaskServiceError, TaskSpec, TaskStatus,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};

use crate::dto::{
    ConfigState, ContentTypeOption, CreateMetadataRequest, CreateScheduleRequest,
    CreateTaskRequest, ExtractedNumber, FilterOptions, HealthResponse, ListQuery, LockRequest,
    MetadataDto, MetadataPage, ScheduleDto, SchedulePage, SettingsDto, SourceOption, TagRequest,
    TaskDto, TaskListQuery, TaskLogEntry, TaskLogPage, TaskPage, UpdateMetadataRequest,
    UpdateScheduleRequest,
};
use crate::error::{ApiError, ErrorBody};
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/metadata", get(list_metadata).post(create_metadata))
        .route(
            "/metadata/{number}",
            get(get_metadata).patch(update_metadata),
        )
        .route("/metadata/{number}/locks/{field}", put(set_lock))
        .route("/metadata/{number}/tags", post(add_tag))
        .route(
            "/metadata/{number}/tags/{name}",
            axum::routing::delete(remove_tag),
        )
        .route("/filters", get(filter_options))
        .route("/number/extract", get(extract_number))
        .route("/config", get(read_config).put(update_config))
        .route("/config/schema", get(config_schema))
        .route("/resources/{id}", get(get_resource))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", get(get_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/retry", post(retry_task))
        .route("/tasks/{id}/log", get(task_log))
        .route("/tasks/{id}/children", get(task_children))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    code: "not_found".to_owned(),
                    message: "接口不存在".to_owned(),
                }),
            )
        })
        .route("/schedules", get(list_schedules).post(create_schedule))
        .route(
            "/schedules/{id}",
            axum::routing::patch(update_schedule).delete(delete_schedule),
        )
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

/// 详情端点共用的装配: 演员、来源、标签、锁与逐字段视图.
async fn load_detail(state: &AppState, number: &MediaNumber) -> Result<MetadataDto, ApiError> {
    let row = state
        .runtime
        .metadata
        .find(number)
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
    dto.tags = state.runtime.metadata.tags(dto.id).await?;
    let locks = state.runtime.metadata.locks(dto.id).await?;
    dto.fields = dto.field_views(&locks);
    dto.locked_fields = locks
        .iter()
        .map(|field| field.as_str().to_owned())
        .collect();

    Ok(dto)
}

/// 手工修订字段. 修订过的字段同时加锁, 并把来源记为人工.
#[utoipa::path(
    patch,
    path = "/api/metadata/{number}",
    tag = "metadata",
    params(("number" = String, Path, description = "番号")),
    request_body = UpdateMetadataRequest,
    responses(
        (status = 200, body = MetadataDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn update_metadata(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(payload): Json<UpdateMetadataRequest>,
) -> Result<Json<MetadataDto>, ApiError> {
    let number = MediaNumber::parse(&number)?;
    if payload.is_empty() {
        return Err(ApiError::Invalid {
            code: "empty_patch",
            message: "没有要修改的字段".to_owned(),
        });
    }

    state
        .runtime
        .metadata
        .update_fields(&number, &payload.patch())
        .await?;

    Ok(Json(load_detail(&state, &number).await?))
}

/// 加锁或解锁单个字段.
#[utoipa::path(
    put,
    path = "/api/metadata/{number}/locks/{field}",
    tag = "metadata",
    params(
        ("number" = String, Path, description = "番号"),
        ("field" = String, Path, description = "字段标识")
    ),
    request_body = LockRequest,
    responses(
        (status = 200, body = MetadataDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn set_lock(
    State(state): State<AppState>,
    Path((number, field)): Path<(String, String)>,
    Json(payload): Json<LockRequest>,
) -> Result<Json<MetadataDto>, ApiError> {
    let number = MediaNumber::parse(&number)?;
    let field = LockableField::parse(&field).ok_or_else(|| ApiError::Invalid {
        code: "invalid_field",
        message: format!("未知字段: {field}"),
    })?;
    let row = state
        .runtime
        .metadata
        .find(&number)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            what: format!("影片 {number} 不存在"),
        })?;

    state
        .runtime
        .metadata
        .set_lock(row.id, field, payload.locked)
        .await?;

    Ok(Json(load_detail(&state, &number).await?))
}

/// 添加用户标签. 同名标签复用同一条记录.
#[utoipa::path(
    post,
    path = "/api/metadata/{number}/tags",
    tag = "metadata",
    params(("number" = String, Path, description = "番号")),
    request_body = TagRequest,
    responses(
        (status = 200, body = MetadataDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn add_tag(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(payload): Json<TagRequest>,
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

    state
        .runtime
        .metadata
        .add_tag(row.id, &payload.name)
        .await?;

    Ok(Json(load_detail(&state, &number).await?))
}

/// 移除用户标签.
#[utoipa::path(
    delete,
    path = "/api/metadata/{number}/tags/{name}",
    tag = "metadata",
    params(
        ("number" = String, Path, description = "番号"),
        ("name" = String, Path, description = "标签名")
    ),
    responses(
        (status = 200, body = MetadataDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn remove_tag(
    State(state): State<AppState>,
    Path((number, name)): Path<(String, String)>,
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

    state.runtime.metadata.remove_tag(row.id, &name).await?;

    Ok(Json(load_detail(&state, &number).await?))
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

    Ok(Json(load_detail(&state, &number).await?))
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

/// 当前配置. 令牌只返回给已授权的调用方, 且不参与表单之外的变化.
#[utoipa::path(
    get,
    path = "/api/config",
    tag = "config",
    responses(
        (status = 200, body = SettingsDto),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn read_config(State(state): State<AppState>) -> Result<Json<SettingsDto>, ApiError> {
    Ok(Json(SettingsDto::from_settings(&state.runtime.settings())))
}

/// 保存配置. 热配置就地生效; 冷配置只登记改动并提示重启.
#[utoipa::path(
    put,
    path = "/api/config",
    tag = "config",
    request_body = serde_json::Value,
    responses(
        (status = 200, body = ConfigState),
        (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    body: Result<Json<amane_core::config::Settings>, JsonRejection>,
) -> Result<Json<ConfigState>, ApiError> {
    // 拒绝同样返回统一错误体: 框架默认的纯文本 400 让界面拿不到字段名.
    let Json(incoming) = body.map_err(|error| ApiError::Invalid {
        code: "invalid_config",
        message: format!("配置不合法: {error}"),
    })?;

    // 先校验后生效: 越界取值不会替换旧的合法配置.
    let restart_required = state.runtime.cold_diff(&incoming);
    let applied = state
        .runtime
        .apply_hot(&incoming)
        .await
        .map_err(|error| ApiError::Invalid {
            code: "invalid_config",
            message: error.to_string(),
        })?;

    // 回显当前生效值, 不是提交值: 冷配置尚未生效, 界面据此显示真实状态.
    let settings = state.runtime.settings();

    Ok(Json(ConfigState {
        settings: SettingsDto::from_settings(&settings),
        applied,
        restart_required,
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
        schedule_id: None,
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
                .active()
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
        .active()
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

#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "schedule",
    responses(
        (status = 200, body = SchedulePage),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn list_schedules(State(state): State<AppState>) -> Result<Json<SchedulePage>, ApiError> {
    let rows = state.runtime.schedules.list().await?;
    let total = i64::try_from(rows.len()).unwrap_or(i64::MAX);

    Ok(Json(SchedulePage {
        total,
        items: rows.into_iter().map(ScheduleDto::from_schedule).collect(),
    }))
}

/// 登记一条定时任务. 表达式与时区在写入前验证, 非法取值返回 400.
#[utoipa::path(
    post,
    path = "/api/schedules",
    tag = "schedule",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, body = ScheduleDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn create_schedule(
    State(state): State<AppState>,
    Json(payload): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleDto>), ApiError> {
    let kind = TaskKind::parse(&payload.kind).ok_or_else(|| ApiError::Invalid {
        code: "invalid_task_kind",
        message: format!("未知任务类型: {}", payload.kind),
    })?;
    let spec = ScheduleSpec {
        kind,
        payload: payload.payload,
        cron: payload.cron,
        timezone: payload.timezone,
    };

    let row = state.runtime.schedules.create(&spec).await?;

    Ok((StatusCode::CREATED, Json(ScheduleDto::from_schedule(row))))
}

/// 启用或停用. 类型、载荷与表达式创建后不变, 需要变更时删除后重新登记.
#[utoipa::path(
    patch,
    path = "/api/schedules/{id}",
    tag = "schedule",
    params(("id" = i64, Path, description = "定时任务标识")),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, body = ScheduleDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateScheduleRequest>,
) -> Result<Json<ScheduleDto>, ApiError> {
    if payload.enabled.is_none() {
        return Err(ApiError::Invalid {
            code: "empty_patch",
            message: "没有要修改的字段".to_owned(),
        });
    }

    let row = state
        .runtime
        .schedules
        .update(
            id,
            &SchedulePatch {
                enabled: payload.enabled,
            },
        )
        .await?;

    Ok(Json(ScheduleDto::from_schedule(row)))
}

/// 删除登记. 已派生的任务保留, 只是失去来源引用.
#[utoipa::path(
    delete,
    path = "/api/schedules/{id}",
    tag = "schedule",
    params(("id" = i64, Path, description = "定时任务标识")),
    responses(
        (status = 204),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn delete_schedule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.runtime.schedules.delete(id).await?;

    Ok(StatusCode::NO_CONTENT)
}
