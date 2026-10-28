//! 来源: 番号 → 影片元数据.
//!
//! 来源全部编译进二进制, 不加载外部插件. 新增站点只需实现 [`Source`] 并登记到 [`Registry`],
//! 宿主与前端不感知具体站点.

pub mod aggregate;
pub mod dmm;
pub mod javdb;

pub use aggregate::{Aggregated, aggregate};
pub use dmm::DmmSource;
pub use javdb::JavdbSource;

use std::sync::Arc;
use std::sync::RwLock;

use amane_core::MediaNumber;
use amane_net::WebClient;
use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 来源声明的能力. 聚合层据此决定向谁问什么.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// 按番号取影片元数据.
    Metadata,
    /// 按关键词搜索番号.
    Search,
    /// 取演员档案.
    Actor,
}

/// 来源的声明性事实.
#[derive(Debug, Clone)]
pub struct SourceDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    pub capabilities: &'static [Capability],
}

/// 单个来源抓到的影片元数据. 字段全部可选: 来源之间的覆盖范围不同.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScrapedMetadata {
    pub title: Option<String>,
    pub plot: Option<String>,
    pub release_date: Option<NaiveDate>,
    pub runtime_minutes: Option<i32>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub directors: Vec<String>,
    pub actors: Vec<String>,
    pub tags: Vec<String>,
    pub poster_url: Option<String>,
}

/// 上游返回的原文.
///
/// `extension` 决定留存时的文件名后缀, 让原文能直接被对应的工具打开.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBody {
    pub content: String,
    pub extension: &'static str,
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("来源 {id} 未收录番号 {number}")]
    NotFound { id: &'static str, number: String },
    #[error("来源 {id} 请求失败: {message}")]
    Transport { id: &'static str, message: String },
    /// 取得了响应, 但其中没有可识别的结构.
    ///
    /// 与 [`SourceError::NotFound`] 的区别: 未收录是已经读懂的答复, 这里是没有可识别的结构.
    /// 后者通常意味着站点改版或被拦截, 需要检查原文才能定位.
    #[error("来源 {id} 返回的内容无法解析: {message}")]
    Parse {
        id: &'static str,
        message: String,
        raw: RawBody,
    },
}

impl SourceError {
    /// 没有可识别的结构, 连同原文一并返回.
    pub fn parse(id: &'static str, message: String, raw: RawBody) -> Self {
        Self::Parse { id, message, raw }
    }

    /// 需要留存的上游原文. 只有解析失败携带.
    pub fn raw(&self) -> Option<&RawBody> {
        match self {
            Self::Parse { raw, .. } => Some(raw),
            _ => None,
        }
    }
}

/// 一个来源.
#[async_trait]
pub trait Source: Send + Sync {
    fn descriptor(&self) -> &SourceDescriptor;

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError>;
}

/// 已登记的来源集合. 顺序即聚合优先级.
#[derive(Default)]
/// 来源登记表.
///
/// 处理函数持有的是同一份 `Arc<Registry>`, 因此启用集与顺序都必须就地更新.
pub struct Registry {
    /// 全部已登记的来源, 按内置顺序.
    all: Vec<Arc<dyn Source>>,
    /// 当前参与聚合的来源, 顺序即聚合优先级.
    active: RwLock<Vec<Arc<dyn Source>>>,
}

impl Registry {
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self {
        let all: Vec<Arc<dyn Source>> = sources
            .into_iter()
            .map(|source| Arc::from(source) as Arc<dyn Source>)
            .collect();
        let active = all.clone();
        Self {
            all,
            active: RwLock::new(active),
        }
    }

    /// 内置来源. 站点实现在各自的切片中登记, 这里保持唯一的装配点.
    pub fn builtin(client: Arc<WebClient>) -> Self {
        Self::new(vec![
            Box::new(DmmSource::new(Arc::clone(&client))),
            Box::new(JavdbSource::new(client)),
        ])
    }

    /// 当前参与聚合的来源.
    pub fn active(&self) -> Vec<Arc<dyn Source>> {
        self.active
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn len(&self) -> usize {
        self.active().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 设置启用集与顺序.
    ///
    /// 空列表表示全部启用, 按内置顺序; 非空列表的取值即参与聚合的来源,
    /// 列表顺序就是聚合优先级, 未列出的来源不参与.
    pub fn apply_enabled_order(&self, order: &[String]) {
        let next = if order.is_empty() {
            self.all.clone()
        } else {
            order
                .iter()
                .filter_map(|id| {
                    self.all
                        .iter()
                        .find(|source| source.descriptor().id == id.as_str())
                        .cloned()
                })
                .collect()
        };
        *self
            .active
            .write()
            .unwrap_or_else(|error| error.into_inner()) = next;
    }

    /// 按声明能力筛选来源.
    pub fn with_capability(&self, capability: Capability) -> Vec<Arc<dyn Source>> {
        self.active()
            .into_iter()
            .filter(move |source| source.descriptor().capabilities.contains(&capability))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, Registry, ScrapedMetadata, Source, SourceDescriptor};
    use amane_core::MediaNumber;
    use async_trait::async_trait;

    struct StubSource {
        descriptor: SourceDescriptor,
        title: &'static str,
    }

    #[async_trait]
    impl Source for StubSource {
        fn descriptor(&self) -> &SourceDescriptor {
            &self.descriptor
        }

        async fn fetch(
            &self,
            _number: &MediaNumber,
        ) -> Result<ScrapedMetadata, super::SourceError> {
            Ok(ScrapedMetadata {
                title: Some(self.title.to_owned()),
                ..ScrapedMetadata::default()
            })
        }
    }

    fn stub(id: &'static str, capabilities: &'static [Capability]) -> Box<dyn Source> {
        Box::new(StubSource {
            descriptor: SourceDescriptor {
                id,
                label: id,
                base_url: "https://example.invalid",
                capabilities,
            },
            title: id,
        })
    }

    #[test]
    fn filters_sources_by_capability() {
        let registry = Registry::new(vec![
            stub("with-search", &[Capability::Metadata, Capability::Search]),
            stub("metadata-only", &[Capability::Metadata]),
        ]);

        let ids: Vec<&str> = registry
            .with_capability(Capability::Search)
            .iter()
            .map(|source| source.descriptor().id)
            .collect();

        assert_eq!(ids, ["with-search"]);
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn reorder_follows_configured_priority() {
        let registry = Registry::new(vec![
            stub("first", &[Capability::Metadata]),
            stub("second", &[Capability::Metadata]),
        ]);

        registry.apply_enabled_order(&["second".to_owned(), "first".to_owned()]);

        let ids: Vec<&str> = registry
            .active()
            .iter()
            .map(|source| source.descriptor().id)
            .collect();
        assert_eq!(ids, ["second", "first"], "配置的顺序应决定聚合优先级");
    }

    #[test]
    fn empty_order_enables_every_source() {
        let registry = Registry::new(vec![
            stub("first", &[Capability::Metadata]),
            stub("second", &[Capability::Metadata]),
        ]);

        registry.apply_enabled_order(&[]);

        let ids: Vec<&str> = registry
            .active()
            .iter()
            .map(|source| source.descriptor().id)
            .collect();
        assert_eq!(ids, ["first", "second"]);
    }

    #[test]
    fn unlisted_source_is_disabled() {
        let registry = Registry::new(vec![
            stub("first", &[Capability::Metadata]),
            stub("second", &[Capability::Metadata]),
        ]);

        registry.apply_enabled_order(&["second".to_owned()]);

        let ids: Vec<&str> = registry
            .active()
            .iter()
            .map(|source| source.descriptor().id)
            .collect();
        assert_eq!(ids, ["second"], "未列出的来源不参与聚合");
    }

    #[test]
    fn builtin_registers_every_site() {
        let client = std::sync::Arc::new(
            amane_net::WebClient::new(
                std::time::Duration::from_secs(5),
                amane_net::RateLimiters::new(amane_net::Rate::per_second(1, 1.0)),
            )
            .expect("客户端应构造成功"),
        );
        let registry = Registry::builtin(client);

        let ids: Vec<&str> = registry
            .active()
            .iter()
            .map(|source| source.descriptor().id)
            .collect();
        assert_eq!(ids, ["dmm", "javdb"], "顺序即聚合优先级");
    }
}
