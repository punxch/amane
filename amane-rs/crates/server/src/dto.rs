use std::collections::{BTreeMap, BTreeSet};

use amane_core::{LockableField, Metadata, MetadataPatch, Pattern, Schedule, Task, TaskLogLine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 番号条目的传输形态.
///
/// 与 `amane_core::Metadata` 分开: 内核不感知传输层, 字段增删不会波及领域类型.
#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataDto {
    pub id: i64,
    pub number: String,
    pub title: Option<String>,
    pub plot: Option<String>,
    /// 上映日期, 形如 `2021-02-18`.
    pub release_date: Option<String>,
    /// 时长, 单位分钟.
    pub runtime_minutes: Option<i32>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub poster_url: Option<String>,
    /// 海报在服务端的地址; 为 `None` 时表示尚未存到本地.
    pub poster_local_url: Option<String>,
    /// 演员名单; 列表端点不返回, 只有详情端点填充.
    pub actors: Vec<String>,
    /// 本次刮削命中的来源标识; 只有详情端点填充.
    pub sources: Vec<String>,
    /// 字段名 → 贡献该字段的来源标识; `manual` 表示人工修订.
    pub field_sources: BTreeMap<String, String>,
    /// 用户备注.
    pub note: Option<String>,
    /// 被锁定的字段标识.
    pub locked_fields: Vec<String>,
    /// 用户标签.
    pub tags: Vec<String>,
    /// 逐字段视图; 只有详情端点填充.
    pub fields: Vec<MetadataFieldDto>,
    /// 类型, 取值见 `ContentType`.
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
            plot: value.plot,
            release_date: value.release_date.map(|date| date.to_string()),
            runtime_minutes: value.runtime_minutes,
            maker: value.maker,
            label: value.label,
            series: value.series,
            poster_url: value.poster_url,
            poster_local_url: value
                .poster_resource_id
                .map(|id| format!("/api/resources/{id}")),
            actors: Vec::new(),
            sources: Vec::new(),
            field_sources: value
                .field_sources
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default(),
            note: value.note,
            locked_fields: Vec::new(),
            tags: Vec::new(),
            fields: Vec::new(),
            content_type: value.content_type.as_str().to_owned(),
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

/// 详情页上的一个字段: 值、来源与锁定状态.
#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataFieldDto {
    /// 字段标识, 也是加锁与修订时使用的键.
    pub field: String,
    pub label: String,
    /// 已格式化为文本的值; `None` 表示未提供.
    pub value: Option<String>,
    /// 贡献该字段的来源标识; `manual` 表示人工修订.
    pub source: Option<String>,
    pub locked: bool,
}

impl MetadataDto {
    /// 逐字段视图. 界面据此渲染字段列表与锁按钮, 不自行维护字段清单.
    pub fn field_views(&self, locks: &BTreeSet<LockableField>) -> Vec<MetadataFieldDto> {
        LockableField::ALL
            .into_iter()
            .map(|field| MetadataFieldDto {
                field: field.as_str().to_owned(),
                label: field.label().to_owned(),
                value: match field {
                    LockableField::Title => self.title.clone(),
                    LockableField::Plot => self.plot.clone(),
                    LockableField::ReleaseDate => self.release_date.clone(),
                    LockableField::RuntimeMinutes => {
                        self.runtime_minutes.map(|minutes| minutes.to_string())
                    }
                    LockableField::Maker => self.maker.clone(),
                    LockableField::Label => self.label.clone(),
                    LockableField::Series => self.series.clone(),
                    LockableField::PosterUrl => self.poster_url.clone(),
                },
                source: self.field_sources.get(field.as_str()).cloned(),
                locked: locks.contains(&field),
            })
            .collect()
    }
}

/// 手工修订. 未提供的字段保持不变; 空字符串表示清空.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMetadataRequest {
    pub title: Option<String>,
    pub plot: Option<String>,
    /// 形如 `2021-02-18`; 空字符串表示清空.
    pub release_date: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub note: Option<String>,
}

impl UpdateMetadataRequest {
    /// 一个字段都没提供时没有可写的内容, 调用方应拒绝而不是空转一次更新.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.plot.is_none()
            && self.release_date.is_none()
            && self.runtime_minutes.is_none()
            && self.maker.is_none()
            && self.label.is_none()
            && self.series.is_none()
            && self.note.is_none()
    }

    pub fn patch(&self) -> MetadataPatch<'_> {
        MetadataPatch {
            title: self.title.as_deref(),
            plot: self.plot.as_deref(),
            release_date: self.release_date.as_deref(),
            runtime_minutes: self.runtime_minutes,
            maker: self.maker.as_deref(),
            label: self.label.as_deref(),
            series: self.series.as_deref(),
            note: self.note.as_deref(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LockRequest {
    pub locked: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TagRequest {
    pub name: String,
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
    /// 单页条数, 默认 50, 上限 200; 小于 1 按 1 处理.
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// 类型: censored / uncensored / chinese / western / fc2 / amateur / hentai.
    pub content_type: Option<String>,
    /// 来源标识; 只列出该来源贡献过内容的番号条目.
    pub source: Option<String>,
    /// 关键字, 匹配番号与标题, 忽略大小写.
    pub q: Option<String>,
}

/// 筛选可选项. 取值域由服务端给出, 界面不自行维护枚举.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilterOptions {
    pub content_types: Vec<ContentTypeOption>,
    pub sources: Vec<SourceOption>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentTypeOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SourceOption {
    pub id: String,
    pub label: String,
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

/// 任务的传输形态.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskDto {
    pub id: i64,
    /// 任务类型, 取值见 `TaskKind`.
    pub kind: String,
    /// 任务状态, 取值见 `TaskStatus`.
    pub status: String,
    pub payload: serde_json::Value,
    pub priority: i32,
    pub attempts: i32,
    pub error: Option<String>,
    /// 重试来源任务的标识.
    pub retry_of: Option<i64>,
    /// 由哪条定时任务派生; 手工提交为 `None`.
    pub schedule_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<Task> for TaskDto {
    fn from(value: Task) -> Self {
        let payload = serde_json::from_str(&value.payload).unwrap_or(serde_json::Value::Null);
        Self {
            id: value.id,
            kind: value.kind.to_string(),
            status: value.status.as_str().to_owned(),
            payload,
            priority: value.priority,
            attempts: value.attempts,
            error: value.error,
            retry_of: value.retry_of,
            schedule_id: value.schedule_id,
            created_at: value.created_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskPage {
    pub total: i64,
    pub items: Vec<TaskDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTaskRequest {
    /// 任务类型: scrape / organize / refresh / cleanup. 取值见 `TaskKind`.
    pub kind: String,
    /// 任务载荷, 结构由类型决定.
    pub payload: serde_json::Value,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TaskListQuery {
    /// 按状态过滤: queued / running / done / failed / cancelled.
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskLogEntry {
    pub at: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

impl From<TaskLogLine> for TaskLogEntry {
    fn from(value: TaskLogLine) -> Self {
        Self {
            at: value.at,
            level: value.level,
            message: value.message,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskLogPage {
    pub items: Vec<TaskLogEntry>,
}

/// 定时任务的传输形态.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleDto {
    pub id: i64,
    /// 任务类型, 取值见 `TaskKind`.
    pub kind: String,
    pub payload: serde_json::Value,
    pub cron: String,
    /// IANA 时区名, 触发时刻按它计算.
    pub timezone: String,
    pub enabled: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
    /// 下一次触发时刻, 按登记的时区计算.
    pub next_fire_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ScheduleDto {
    pub fn from_schedule(value: Schedule) -> Self {
        let next_fire_at = Pattern::parse(&value.cron, &value.timezone)
            .ok()
            .and_then(|pattern| {
                pattern
                    .next_after(value.last_fired_at.unwrap_or_else(chrono::Utc::now))
                    .ok()
            });

        Self {
            id: value.id,
            kind: value.kind.to_string(),
            payload: serde_json::from_str(&value.payload).unwrap_or(serde_json::Value::Null),
            cron: value.cron,
            timezone: value.timezone,
            enabled: value.enabled,
            last_fired_at: value.last_fired_at,
            next_fire_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SchedulePage {
    pub total: i64,
    pub items: Vec<ScheduleDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateScheduleRequest {
    /// 任务类型: scrape / organize / refresh / cleanup.
    pub kind: String,
    /// 任务载荷, 结构由类型决定.
    pub payload: serde_json::Value,
    /// 五段式 cron 表达式.
    pub cron: String,
    /// IANA 时区名, 例如 `Asia/Shanghai`.
    pub timezone: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateScheduleRequest {
    pub enabled: Option<bool>,
}

/// 当前配置的传输形态. 与 `Settings` 分开, 令牌只在被显式设置时才出现.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SettingsDto {
    pub cold: ColdSettingsDto,
    pub hot: HotSettingsDto,
}

/// 保存配置的请求体.
///
/// 与 `Settings` 分开: 领域类型不参与传输, 数据目录在这里以字符串出现.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfigRequest {
    pub cold: ColdSettingsDto,
    pub hot: HotSettingsDto,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ColdSettingsDto {
    pub data_dir: String,
    pub bind: String,
    pub api_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HotSettingsDto {
    pub concurrency: u32,
    pub request_timeout_secs: u32,
    pub enabled_sources: Vec<String>,
    pub scan: ScanSettingsDto,
    pub organize: OrganizeSettingsDto,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanSettingsDto {
    pub media_extensions: Vec<String>,
    pub min_file_size: u64,
    pub blacklist_patterns: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OrganizeSettingsDto {
    pub video_template: String,
    /// 随视频一起整理的字幕扩展名; 空列表关闭字幕随配.
    pub subtitle_extensions: Vec<String>,
    /// 是否为每个视频生成 STRM 指针文件.
    pub create_strm: bool,
}

impl SettingsDto {
    pub fn from_settings(value: &amane_core::config::Settings) -> Self {
        Self {
            cold: ColdSettingsDto {
                data_dir: value.cold.data_dir.display().to_string(),
                bind: value.cold.bind.clone(),
                api_token: value.cold.api_token.clone(),
            },
            hot: HotSettingsDto {
                concurrency: value.hot.concurrency,
                request_timeout_secs: value.hot.request_timeout_secs,
                enabled_sources: value.hot.enabled_sources.clone(),
                scan: ScanSettingsDto {
                    media_extensions: value.hot.scan.media_extensions.clone(),
                    min_file_size: value.hot.scan.min_file_size,
                    blacklist_patterns: value.hot.scan.blacklist_patterns.clone(),
                },
                organize: OrganizeSettingsDto {
                    video_template: value.hot.organize.video_template.clone(),
                    subtitle_extensions: value.hot.organize.subtitle_extensions.clone(),
                    create_strm: value.hot.organize.create_strm,
                },
            },
        }
    }
}

impl UpdateConfigRequest {
    /// 转为内核配置. 数据目录在这里才从字符串变成路径.
    pub fn into_settings(self) -> amane_core::config::Settings {
        amane_core::config::Settings {
            cold: amane_core::config::ColdSettings {
                data_dir: std::path::PathBuf::from(self.cold.data_dir),
                bind: self.cold.bind,
                api_token: self.cold.api_token,
            },
            hot: amane_core::config::HotSettings {
                concurrency: self.hot.concurrency,
                request_timeout_secs: self.hot.request_timeout_secs,
                enabled_sources: self.hot.enabled_sources,
                scan: amane_core::config::ScanSettings {
                    media_extensions: self.hot.scan.media_extensions,
                    min_file_size: self.hot.scan.min_file_size,
                    blacklist_patterns: self.hot.scan.blacklist_patterns,
                },
                organize: amane_core::config::OrganizeSettings {
                    video_template: self.hot.organize.video_template,
                    subtitle_extensions: self.hot.organize.subtitle_extensions,
                    create_strm: self.hot.organize.create_strm,
                },
            },
        }
    }
}

/// 保存配置的结果: 已生效的热配置字段与需要重启的冷配置字段.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfigState {
    pub settings: SettingsDto,
    /// 已就地生效的字段名.
    pub applied: Vec<String>,
    /// 发生变化的冷配置字段; 非空表示需要重启进程.
    pub restart_required: Vec<String>,
}

/// 媒体库登记的传输形态.
#[derive(Debug, Serialize, ToSchema)]
pub struct LibraryDto {
    pub id: i64,
    /// 登记的绝对路径.
    pub path: String,
    pub created_at: DateTime<Utc>,
    /// 最近一次扫描的各类统计; 从未扫描时全为 0.
    pub counts: LibraryCountsDto,
}

/// 每个类别的条数.
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct LibraryCountsDto {
    pub media: i64,
    pub unwanted: i64,
    pub system: i64,
    pub unrecognized: i64,
    pub leftover_dir: i64,
}

impl From<amane_core::KindCounts> for LibraryCountsDto {
    fn from(value: amane_core::KindCounts) -> Self {
        Self {
            media: value.media,
            unwanted: value.unwanted,
            system: value.system,
            unrecognized: value.unrecognized,
            leftover_dir: value.leftover_dir,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLibraryRequest {
    /// 要登记的目录路径.
    pub path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LibraryPage {
    pub total: i64,
    pub items: Vec<LibraryDto>,
}

/// 扫描索引里一条记录的传输形态.
#[derive(Debug, Serialize, ToSchema)]
pub struct LibraryFileDto {
    /// 相对库根目录的相对路径.
    pub path: String,
    /// 归类, 取值见 `ScanKind`: media / unwanted / system / unrecognized / leftover_dir.
    pub kind: String,
    /// 无效文件命中的规则名: blacklist 或 undersized.
    pub rule: Option<String>,
    /// 命中的模式原文或补充说明.
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LibraryFilePage {
    pub total: i64,
    pub items: Vec<LibraryFileDto>,
}

/// 列出扫描索引时的类别过滤.
#[derive(Debug, Deserialize, IntoParams)]
pub struct LibraryFileQuery {
    /// 只列出该类别; 缺省列出全部.
    pub kind: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// 整理预览的传输形态. 预览不写盘, 冲突由后端判定, 处置选择在执行端点给出.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizePreviewDto {
    pub items: Vec<PreviewItemDto>,
    /// 提取不出番号而无法参与整理的媒体文件数.
    pub unmatched: i64,
    /// 索引里有但磁盘上已不存在的源文件数 (整理过或被外部移动).
    pub stale: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewItemDto {
    /// 源文件, 相对库根.
    pub source_path: String,
    /// 模板渲染出的目标路径, 相对库根.
    pub target_path: String,
    pub number: String,
    /// 冲突类别: exists / number_mismatch / same_path; 为空表示可直接整理.
    pub conflict: Option<String>,
}

/// 整理执行的请求体. choices 以源相对路径为键; 缺省跳过冲突项.
#[derive(Debug, Deserialize, ToSchema)]
pub struct OrganizeExecuteRequest {
    /// 源相对路径 → 处置: `overwrite` / `rename`.
    #[serde(default)]
    pub choices: std::collections::HashMap<String, String>,
}

/// 清理清单. 只读: 展示待删文件、大小与命中原因, 不直接删除.
#[derive(Debug, Serialize, ToSchema)]
pub struct CleanupManifestDto {
    pub items: Vec<CleanupItemDto>,
    /// 待删文件的总字节数.
    pub total_size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CleanupItemDto {
    /// 相对库根的路径.
    pub path: String,
    pub size: u64,
    /// 命中的规则说明.
    pub reason: String,
}

/// 回收目录的内容与占用.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrashPageDto {
    pub items: Vec<TrashItemDto>,
    /// 回收目录的总占用 (字节).
    pub total_size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TrashItemDto {
    /// 相对回收目录的路径, 即原相对路径.
    pub path: String,
    pub size: u64,
}

/// 恢复请求.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreRequest {
    /// 相对回收目录的路径.
    pub path: String,
}
