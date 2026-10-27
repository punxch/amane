use amane_core::Metadata;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 条目的传输形态.
///
/// 与 `amane_core::Metadata` 分开: 内核不感知传输层, 字段增删不会波及领域类型.
#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataDto {
    pub id: i64,
    pub number: String,
    pub title: Option<String>,
    /// 条目类型, 取值见 `ContentType`.
    pub content_type: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Metadata> for MetadataDto {
    fn from(value: Metadata) -> Self {
        Self {
            id: value.id,
            number: value.number,
            title: value.title,
            content_type: value.content_type.as_str().to_owned(),
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataPage {
    /// 命中总数, 不受分页影响.
    pub total: i64,
    pub items: Vec<MetadataDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMetadataRequest {
    /// 番号; 允许带分隔符或连写, 服务端归一后写入.
    pub number: String,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// 单页条数, 默认 50, 上限 200.
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExtractQuery {
    /// 待解析的自由文本, 通常是文件名或完整路径.
    pub text: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExtractedNumber {
    pub number: String,
    pub content_type: String,
    pub prefix: String,
}
