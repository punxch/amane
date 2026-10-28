use amane_core::TaskLogger;
use amane_core::{
    LockableField, MediaNumber, MetadataFilter, MetadataInput, SchedulePatch, ScheduleSpec,
    TaskFilter, TaskKind, TaskServiceError, TaskSpec, TaskStatus,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};

use crate::dto::{
    CleanupItemDto, CleanupManifestDto, ConfigState, ContentTypeOption, CreateLibraryRequest,
    CreateMetadataRequest, CreateScheduleRequest, CreateTaskRequest, ExtractedNumber,
    FilterOptions, HealthResponse, LibraryCountsDto, LibraryDto, LibraryFileDto, LibraryFilePage,
    LibraryFileQuery, LibraryPage, ListQuery, LockRequest, MetadataDto, MetadataPage,
    OrganizeExecuteRequest, OrganizePreviewDto, PreviewItemDto, RelatedCounts, RestoreRequest,
    ScheduleDto, SchedulePage, SettingsDto, SourceOption, TagRequest, TaskDto, TaskListQuery,
    TaskLogEntry, TaskLogPage, TaskPage, TrashItemDto, TrashPageDto, UpdateConfigRequest,
    UpdateMetadataRequest, UpdateScheduleRequest,
};
use crate::error::{ApiError, ApiJson, ErrorBody};
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

/// 取媒体库登记; 不存在时给出统一的 404 说明.
async fn require_library(state: &AppState, id: i64) -> Result<amane_core::LibraryRoot, ApiError> {
    state
        .runtime
        .libraries
        .find(id)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            what: format!("媒体库 {id} 不存在"),
        })
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/metadata", get(list_metadata).post(create_metadata))
        .route(
            "/metadata/{number}",
            get(get_metadata)
                .patch(update_metadata)
                .delete(delete_metadata),
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
        .route("/libraries", get(list_libraries).post(create_library))
        .route("/libraries/{id}", axum::routing::delete(delete_library))
        .route("/libraries/{id}/scan", post(scan_library))
        .route("/libraries/{id}/files", get(list_library_files))
        .route("/libraries/{id}/organize/preview", post(organize_preview))
        .route("/libraries/{id}/organize/execute", post(organize_execute))
        .route("/libraries/{id}/cleanup", get(cleanup_manifest))
        .route("/libraries/{id}/cleanup/execute", post(cleanup_execute))
        .route("/libraries/{id}/trash", get(trash_listing))
        .route("/libraries/{id}/trash/restore", post(trash_restore))
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
    ApiJson(payload): ApiJson<CreateMetadataRequest>,
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
        .map(|snapshot| snapshot.source_id)
        .collect();
    dto.tags = state.runtime.metadata.tags(dto.id).await?;
    let locks = state.runtime.metadata.locks(dto.id).await?;
    dto.fields = dto.field_views(&locks);
    dto.locked_fields = locks
        .iter()
        .map(|field| field.as_str().to_owned())
        .collect();

    // 删除前要告诉用户会连带删掉什么, 计数取自上面已经查出来的清单, 不额外查询数据库.
    dto.related = RelatedCounts {
        actors: dto.actors.len(),
        locks: dto.locked_fields.len(),
        tags: dto.tags.len(),
        sources: dto.sources.len(),
        resources: usize::from(dto.poster_local_url.is_some()),
    };

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
    ApiJson(payload): ApiJson<UpdateMetadataRequest>,
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

/// 删除条目.
///
/// 关联数据随主行级联删除; 不再被引用的资源文件在事务提交后清理. 顺序反过来会让条目
/// 指向不存在的文件, 这个顺序最坏只留下一个无主文件.
#[utoipa::path(
    delete,
    path = "/api/metadata/{number}",
    tag = "metadata",
    params(("number" = String, Path, description = "番号")),
    responses(
        (status = 204, description = "已删除"),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn delete_metadata(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<StatusCode, ApiError> {
    let number = MediaNumber::parse(&number)?;
    let outcome = state.runtime.metadata.delete(&number).await?;

    for resource in &outcome.resources {
        if let Err(error) = state.runtime.resources.remove_file(resource).await {
            tracing::warn!(resource = %resource.file_name, %error, "删除资源文件失败");
        }
    }

    Ok(StatusCode::NO_CONTENT)
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
    ApiJson(payload): ApiJson<LockRequest>,
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
    ApiJson(payload): ApiJson<TagRequest>,
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
    request_body = UpdateConfigRequest,
    responses(
        (status = 200, body = ConfigState),
        (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<UpdateConfigRequest>,
) -> Result<Json<ConfigState>, ApiError> {
    let incoming = request.into_settings();

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
    Json(
        serde_json::to_value(schemars::schema_for!(amane_core::config::Settings))
            .expect("配置 schema 可序列化"),
    )
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
    ApiJson(payload): ApiJson<CreateTaskRequest>,
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

/// 查询参数解析失败也走统一错误体: 框架默认的纯文本 400 会让界面无法取得错误码.
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

    // 分类是开放取值, 不做白名单校验: 写错一个字得到空列表, 而不是让整个筛选报错.
    let genre = query
        .genre
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    Ok(MetadataFilter {
        content_type,
        source,
        keyword,
        genre,
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
pub async fn filter_options(
    State(state): State<AppState>,
) -> Result<Json<FilterOptions>, ApiError> {
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

    let genres = state.runtime.metadata.genre_options().await?;

    Ok(Json(FilterOptions {
        content_types,
        sources,
        genres,
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
    ApiJson(payload): ApiJson<CreateScheduleRequest>,
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
    ApiJson(payload): ApiJson<UpdateScheduleRequest>,
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

/// 登记的媒体库与最近一次扫描的各类统计.
#[utoipa::path(
    get,
    path = "/api/libraries",
    tag = "library",
    responses(
        (status = 200, body = LibraryPage),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn list_libraries(State(state): State<AppState>) -> Result<Json<LibraryPage>, ApiError> {
    let roots = state.runtime.libraries.list().await?;
    let mut items = Vec::with_capacity(roots.len());
    for root in roots {
        let counts = state.runtime.libraries.counts(root.id).await?;
        items.push(LibraryDto {
            id: root.id,
            path: root.path,
            created_at: root.created_at,
            counts: LibraryCountsDto::from(counts),
        });
    }
    let total = i64::try_from(items.len()).unwrap_or(i64::MAX);

    Ok(Json(LibraryPage { total, items }))
}

/// 验证要登记的路径: 必须存在、是目录且可读, 否则给出可读的原因.
fn validate_root_path(raw: &str) -> Result<std::path::PathBuf, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Invalid {
            code: "invalid_path",
            message: "路径不能为空".to_owned(),
        });
    }
    let path = std::path::PathBuf::from(trimmed);
    let metadata = std::fs::metadata(&path).map_err(|error| ApiError::Invalid {
        code: "invalid_path",
        message: format!("路径 {} 不可用: {error}", path.display()),
    })?;
    if !metadata.is_dir() {
        return Err(ApiError::Invalid {
            code: "invalid_path",
            message: format!("{} 不是目录", path.display()),
        });
    }
    std::fs::read_dir(&path).map_err(|error| ApiError::Invalid {
        code: "invalid_path",
        message: format!("目录 {} 无法读取: {error}", path.display()),
    })?;

    // 规范化为绝对路径, 相对路径与符号链接在登记时定型.
    path.canonicalize().map_err(|error| ApiError::Invalid {
        code: "invalid_path",
        message: format!("解析 {} 失败: {error}", path.display()),
    })
}

/// 登记一个媒体库根目录. 登记本身不扫描; 扫描由任务派生.
#[utoipa::path(
    post,
    path = "/api/libraries",
    tag = "library",
    request_body = CreateLibraryRequest,
    responses(
        (status = 201, body = LibraryDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn create_library(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<CreateLibraryRequest>,
) -> Result<(StatusCode, Json<LibraryDto>), ApiError> {
    let path = validate_root_path(&payload.path)?;
    let root = state.runtime.libraries.create(&path).await?;

    Ok((
        StatusCode::CREATED,
        Json(LibraryDto {
            id: root.id,
            path: root.path,
            created_at: root.created_at,
            counts: LibraryCountsDto::default(),
        }),
    ))
}

/// 移除登记与扫描索引. 磁盘文件不受影响.
#[utoipa::path(
    delete,
    path = "/api/libraries/{id}",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    responses(
        (status = 204),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn delete_library(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.runtime.libraries.delete(id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// 派生一次扫描. 结果在任务完成后由登记列表与文件端点呈现.
#[utoipa::path(
    post,
    path = "/api/libraries/{id}/scan",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    responses(
        (status = 202, body = TaskDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn scan_library(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<(StatusCode, Json<TaskDto>), ApiError> {
    require_library(&state, id).await?;

    let task = state
        .runtime
        .tasks
        .submit(amane_core::TaskSpec::new(
            amane_core::TaskKind::Scan,
            serde_json::json!({ "library_id": id }),
        ))
        .await
        .map_err(map_task_error)?;

    Ok((StatusCode::ACCEPTED, Json(TaskDto::from(task))))
}

/// 按类别列出扫描索引.
#[utoipa::path(
    get,
    path = "/api/libraries/{id}/files",
    tag = "library",
    params(
        ("id" = i64, Path, description = "媒体库标识"),
        LibraryFileQuery
    ),
    responses(
        (status = 200, body = LibraryFilePage),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn list_library_files(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    query: Result<Query<LibraryFileQuery>, QueryRejection>,
) -> Result<Json<LibraryFilePage>, ApiError> {
    require_library(&state, id).await?;
    let Query(query) = parse_query(query)?;
    let kind = match query.kind.as_deref() {
        None | Some("") => None,
        Some(value) => {
            Some(
                amane_core::ScanKind::parse(value).ok_or_else(|| ApiError::Invalid {
                    code: "invalid_scan_kind",
                    message: format!("未知类别: {value}"),
                })?,
            )
        }
    };
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let (total, rows) = state
        .runtime
        .libraries
        .files(id, kind, limit, offset)
        .await?;

    Ok(Json(LibraryFilePage {
        total,
        items: rows
            .into_iter()
            .map(|file| LibraryFileDto {
                path: file.path,
                kind: file.kind.as_str().to_owned(),
                rule: file.rule,
                detail: file.detail,
            })
            .collect(),
    }))
}

/// 整理预览: 只读地给出每个媒体文件的目标路径与冲突, 不创建目录也不移动文件.
///
/// 文件按文件名配对番号, 条目缺席时以番号参与渲染; 重复调用结果一致.
#[utoipa::path(
    post,
    path = "/api/libraries/{id}/organize/preview",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    responses(
        (status = 200, body = OrganizePreviewDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn organize_preview(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<OrganizePreviewDto>, ApiError> {
    let root = require_library(&state, id).await?;

    let settings = state.runtime.settings();
    let template = amane_core::Template::parse(&settings.hot.organize.video_template)
        .map_err(|error| ApiError::Internal(format!("路径模板无法解析: {error}")))?;

    // 扫描索引里的媒体文件是预览的全部对象: 按页取会让超出页大小的文件既不预览也不提示.
    let media = state
        .runtime
        .libraries
        .all_files(id, Some(amane_core::ScanKind::Media))
        .await?;

    // 逐文件提取番号; 提取不出的计入 unmatched, 不进入预览.
    let mut unmatched = 0i64;
    let mut numbers: Vec<(usize, amane_core::MediaNumber)> = Vec::new();
    for (index, file) in media.iter().enumerate() {
        match amane_core::number_from_path(&file.path) {
            Some(number) => numbers.push((index, number)),
            None => unmatched += 1,
        }
    }

    // 条目查询并行发出: 读池多连接, 一次配齐全部条目.
    // 查询失败必须上抛: 当成「条目缺席」会把预览渲染成另一副样子, 界面上看不出区别.
    let metadata = &state.runtime.metadata;
    let entries: Vec<Option<amane_core::Metadata>> = futures::future::join_all(
        numbers
            .iter()
            .map(|(_, number)| async move { metadata.find(number).await }),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|error| ApiError::Internal(format!("查询番号条目失败: {error}")))?;

    let mut preview_requests: Vec<amane_core::PreviewRequest> = Vec::with_capacity(numbers.len());
    for ((index, number), entry) in numbers.iter().zip(&entries) {
        preview_requests.push(amane_core::PreviewRequest {
            source_rel: &media[*index].path,
            number: number.as_str(),
            entry: entry.as_ref(),
        });
    }

    let plans: std::collections::BTreeMap<String, String> = state
        .runtime
        .organize
        .list_for_root(root.id)
        .await?
        .into_iter()
        .map(|plan| (plan.path, plan.number))
        .collect();

    let preview = amane_core::build_preview(&template, &root.path_buf(), &preview_requests, &plans);

    Ok(Json(OrganizePreviewDto {
        items: preview
            .items
            .into_iter()
            .map(|item| PreviewItemDto {
                source_path: item.source_path,
                target_path: item.target_path,
                number: item.number,
                conflict: item.conflict.map(|kind| kind.as_str().to_owned()),
            })
            .collect(),
        unmatched: unmatched + preview.unmatched,
        stale: preview.stale,
    }))
}

/// 整理执行: 按预览目标移动文件. 冲突项没有明确选择时跳过, 不会默默覆盖.
///
/// 目标路径由服务端按当前配置与条目重新渲染, 不信任客户端提交的目标路径.
#[utoipa::path(
    post,
    path = "/api/libraries/{id}/organize/execute",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    request_body = OrganizeExecuteRequest,
    responses(
        (status = 202, body = TaskDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn organize_execute(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    ApiJson(payload): ApiJson<OrganizeExecuteRequest>,
) -> Result<(StatusCode, Json<TaskDto>), ApiError> {
    // 处置取值在提交时校验, 拼错的动词不进队列.
    for action in payload.choices.values() {
        if amane_core::ConflictAction::parse(action).is_none() {
            return Err(ApiError::Invalid {
                code: "invalid_action",
                message: format!("未知处置: {action}"),
            });
        }
    }

    require_library(&state, id).await?;

    let task = state
        .runtime
        .tasks
        .submit(amane_core::TaskSpec::new(
            amane_core::TaskKind::Organize,
            serde_json::json!({
                "library_id": id,
                "choices": payload.choices,
            }),
        ))
        .await
        .map_err(map_task_error)?;

    Ok((StatusCode::ACCEPTED, Json(TaskDto::from(task))))
}

/// 清理清单: 无效文件与系统产物, 带大小与命中原因. 只读, 不直接删除.
#[utoipa::path(
    get,
    path = "/api/libraries/{id}/cleanup",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    responses(
        (status = 200, body = CleanupManifestDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn cleanup_manifest(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<CleanupManifestDto>, ApiError> {
    let root = require_library(&state, id).await?;
    // 清单必须是全量: 按页取会让超出页大小的文件既不列出也不清理.
    let index = state.runtime.libraries.all_files(id, None).await?;
    let items = amane_core::build_cleanup_manifest(&index, &root.path_buf());
    let total_size = items.iter().map(|item| item.size).sum();

    Ok(Json(CleanupManifestDto {
        items: items
            .into_iter()
            .map(|item| CleanupItemDto {
                path: item.path,
                size: item.size,
                reason: item.reason,
            })
            .collect(),
        total_size,
    }))
}

/// 派生清理任务. 清单在任务内重建, 空清单不动任何文件.
#[utoipa::path(
    post,
    path = "/api/libraries/{id}/cleanup/execute",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    responses(
        (status = 202, body = TaskDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn cleanup_execute(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<(StatusCode, Json<TaskDto>), ApiError> {
    require_library(&state, id).await?;

    let task = state
        .runtime
        .tasks
        .submit(amane_core::TaskSpec::new(
            amane_core::TaskKind::Cleanup,
            serde_json::json!({ "library_id": id }),
        ))
        .await
        .map_err(map_task_error)?;

    Ok((StatusCode::ACCEPTED, Json(TaskDto::from(task))))
}

/// 回收目录的内容与占用.
#[utoipa::path(
    get,
    path = "/api/libraries/{id}/trash",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    responses(
        (status = 200, body = TrashPageDto),
        (status = 404, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn trash_listing(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TrashPageDto>, ApiError> {
    let root = require_library(&state, id).await?;

    // 遍历是纯阻塞 IO, 交给阻塞线程池.
    let root_path = root.path_buf();
    let items = tokio::task::spawn_blocking(move || amane_core::list_trash(&root_path))
        .await
        .map_err(|error| ApiError::Internal(format!("读取回收目录失败: {error}")))?;
    let total_size = amane_core::trash_total_size(&items);

    Ok(Json(TrashPageDto {
        items: items
            .into_iter()
            .map(|item| TrashItemDto {
                path: item.path,
                size: item.size,
            })
            .collect(),
        total_size,
    }))
}

/// 把回收目录里的一个文件恢复到原路径. 目标已存在时拒绝而不覆盖.
#[utoipa::path(
    post,
    path = "/api/libraries/{id}/trash/restore",
    tag = "library",
    params(("id" = i64, Path, description = "媒体库标识")),
    request_body = RestoreRequest,
    responses(
        (status = 200, body = TrashPageDto),
        (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody)
    )
)]
pub async fn trash_restore(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    ApiJson(payload): ApiJson<RestoreRequest>,
) -> Result<Json<TrashPageDto>, ApiError> {
    let root = require_library(&state, id).await?;

    // 路径遍历防御: 相对路径不得逃出回收目录映射.
    if payload.path.contains("..") {
        return Err(ApiError::Invalid {
            code: "invalid_path",
            message: "路径不得包含 ..".to_owned(),
        });
    }

    let root_path = root.path_buf();
    let relative = payload.path.clone();
    let restored =
        tokio::task::spawn_blocking(move || amane_core::restore_from_trash(&root_path, &relative))
            .await
            .map_err(|error| ApiError::Internal(format!("恢复线程异常退出: {error}")))?
            .map_err(|error| match error {
                amane_core::organize::RestoreError::TargetExists(path) => ApiError::Conflict {
                    message: format!("原路径 {path} 已存在文件, 恢复被拒绝"),
                },
                amane_core::organize::RestoreError::NotFound(path) => ApiError::NotFound {
                    what: format!("回收目录中不存在 {path}"),
                },
                other => ApiError::Invalid {
                    code: "restore_failed",
                    message: other.to_string(),
                },
            })?;

    tracing::info!(library = id, path = %restored, "回收文件已恢复");

    // 恢复后的文件重新进入扫描索引 (按未识别登记); 重新扫描会重新归类.
    state
        .runtime
        .libraries
        .insert_file(
            id,
            &amane_core::LibraryFile::new(&restored, amane_core::ScanKind::Unrecognized),
        )
        .await?;

    // 恢复后回显回收目录现状, 界面无需二次刷新.
    let root_path = root.path_buf();
    let items = tokio::task::spawn_blocking(move || amane_core::list_trash(&root_path))
        .await
        .map_err(|error| ApiError::Internal(format!("读取回收目录失败: {error}")))?;
    let total_size = amane_core::trash_total_size(&items);

    Ok(Json(TrashPageDto {
        items: items
            .into_iter()
            .map(|item| TrashItemDto {
                path: item.path,
                size: item.size,
            })
            .collect(),
        total_size,
    }))
}
