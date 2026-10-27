use amane_core::MediaNumber;
use amane_core::config::Settings;
use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;

use crate::dto::{
    CreateMetadataRequest, ExtractedNumber, HealthResponse, ListQuery, MetadataDto, MetadataPage,
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
