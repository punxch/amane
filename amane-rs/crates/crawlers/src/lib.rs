//! 来源: 番号 → 影片元数据.
//!
//! 来源全部编译进二进制, 不加载外部插件. 新增站点只需实现 [`Source`] 并登记到 [`Registry`],
//! 宿主与前端不感知具体站点.

use amane_core::MediaNumber;
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
    pub runtime_minutes: Option<u32>,
    pub studio: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub directors: Vec<String>,
    pub actors: Vec<String>,
    pub tags: Vec<String>,
    pub poster_url: Option<String>,
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("来源 {id} 未收录番号 {number}")]
    NotFound { id: &'static str, number: String },
    #[error("来源 {id} 请求失败: {message}")]
    Transport { id: &'static str, message: String },
    #[error("来源 {id} 返回的内容无法解析: {message}")]
    Parse { id: &'static str, message: String },
}

/// 一个来源.
#[async_trait]
pub trait Source: Send + Sync {
    fn descriptor(&self) -> &SourceDescriptor;

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError>;
}

/// 已登记的来源集合. 顺序即聚合优先级.
#[derive(Default)]
pub struct Registry {
    sources: Vec<Box<dyn Source>>,
}

impl Registry {
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self {
        Self { sources }
    }

    /// 内置来源. 站点实现在各自的切片中登记, 这里保持唯一的装配点.
    pub fn builtin() -> Self {
        Self::new(Vec::new())
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Source> {
        self.sources.iter().map(|source| source.as_ref())
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// 按声明能力筛选来源.
    pub fn with_capability(&self, capability: Capability) -> impl Iterator<Item = &dyn Source> {
        self.iter()
            .filter(move |source| source.descriptor().capabilities.contains(&capability))
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
            .map(|source| source.descriptor().id)
            .collect();

        assert_eq!(ids, ["with-search"]);
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn builtin_starts_empty() {
        let registry = Registry::builtin();

        assert!(registry.is_empty(), "站点实现登记之前不应有来源");
    }
}
