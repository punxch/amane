//! 来源: 番号 → 影片元数据.
//!
//! 来源全部编译进二进制, 不加载外部插件. 新增站点只需实现 [`Source`] 并登记到 [`Registry`],
//! 宿主与前端不感知具体站点.

mod dom;

pub mod aggregate;
pub mod dmm;
pub mod javbus;
pub mod javdb;
pub mod number;

pub use aggregate::{Aggregated, aggregate};
/// 来源返回的元数据由内核定义: 分站快照要按同一份结构落库.
pub use amane_core::ScrapedMetadata;
pub use amane_net::Rate;
pub use dmm::DmmSource;
pub use javbus::JavbusSource;
pub use javdb::JavdbSource;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::RwLock;

use amane_core::config::ContentRoutes;
use amane_core::{ContentType, MediaNumber};
use amane_net::WebClient;
use async_trait::async_trait;
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
    /// 请求实际打到的主机名. 与 `base_url` 分开: 站点换域名时速率仍按同一台主机算.
    pub host: &'static str,
    /// 站点能承受的出站速率. 装配层据此建限速表, 配置可再按 host 覆盖.
    pub rate: Rate,
    pub capabilities: &'static [Capability],
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
    /// 站点按出口 IP 拒绝访问.
    ///
    /// 与 [`SourceError::Parse`] 分开: 前者换出口就能解决, 后者要等站点改版修复,
    /// 混在一起会让人去改一个没坏的解析器.
    #[error("来源 {id} 拒绝当前网络出口的访问 (地区封锁)")]
    Blocked { id: &'static str },
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

/// 已登记的来源集合. 按内容类型持有来源链, 链上顺序即聚合优先级.
#[derive(Default)]
/// 来源登记表.
///
/// 处理函数持有的是同一份 `Arc<Registry>`, 因此路由必须就地更新.
pub struct Registry {
    /// 全部已登记的来源, 按内置顺序.
    all: Vec<Arc<dyn Source>>,
    /// 每个内容类型的来源链; 未登记的类型为空.
    routes: RwLock<BTreeMap<ContentType, Vec<Arc<dyn Source>>>>,
}

impl Registry {
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self {
        let all: Vec<Arc<dyn Source>> = sources
            .into_iter()
            .map(|source| Arc::from(source) as Arc<dyn Source>)
            .collect();
        Self {
            all,
            routes: RwLock::new(BTreeMap::new()),
        }
    }

    /// 内置来源. 站点实现在各自的切片中登记, 这里保持唯一的装配点.
    pub fn builtin(client: Arc<WebClient>) -> Self {
        Self::new(vec![
            Box::new(DmmSource::new(Arc::clone(&client))),
            Box::new(JavdbSource::new(Arc::clone(&client))),
            Box::new(JavbusSource::new(client)),
        ])
    }

    /// 每个 host 的出站速率.
    ///
    /// 同一 host 上有多个来源时取更保守的一份: 站点只看到一台主机, 快的那份会让它收到没答应的频率.
    /// 配置里的覆盖按 host 生效, 优先级高于来源声明 —— 被限流时用户要能临时放慢.
    pub fn outbound_rates(&self, overrides: &BTreeMap<String, f64>) -> HashMap<String, Rate> {
        let mut rates: BTreeMap<&'static str, Rate> = BTreeMap::new();
        for source in &self.all {
            let descriptor = source.descriptor();
            rates
                .entry(descriptor.host)
                .and_modify(|current| *current = current.slower(descriptor.rate))
                .or_insert(descriptor.rate);
        }

        let mut table: HashMap<String, Rate> = rates
            .into_iter()
            .map(|(host, rate)| (host.to_owned(), rate))
            .collect();
        for (host, per_second) in overrides {
            // 覆盖只给每秒次数: 突发上限跟着变成 1, 即严格匀速, 这是「放慢」的语义.
            table.insert(host.clone(), Rate::per_second(1, *per_second));
        }

        table
    }

    /// 全部已登记的来源标识, 与当前路由无关.
    ///
    /// 配置校验与筛选选项都取这里: 停用一个来源后仍要允许把它重新加回链中,
    /// 也要允许按它筛选已经存在的旧条目.
    pub fn known_ids(&self) -> Vec<&'static str> {
        self.all
            .iter()
            .map(|source| source.descriptor().id)
            .collect()
    }

    /// 全部已登记的来源, 与当前路由无关.
    pub fn all(&self) -> &[Arc<dyn Source>] {
        &self.all
    }

    /// 某个内容类型的来源链, 顺序即聚合优先级.
    pub fn chain_for(&self, content_type: ContentType) -> Vec<Arc<dyn Source>> {
        self.routes
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(&content_type)
            .cloned()
            .unwrap_or_default()
    }

    /// 设置每类影片的来源链.
    ///
    /// 链上未登记的标识直接跳过: 配置文件可能来自有更多来源的版本.
    pub fn apply_routes(&self, routes: &ContentRoutes) {
        let next: BTreeMap<ContentType, Vec<Arc<dyn Source>>> = routes
            .entries()
            .map(|(content_type, chain)| {
                let sources = chain
                    .iter()
                    .filter_map(|id| {
                        self.all
                            .iter()
                            .find(|source| source.descriptor().id == id.as_str())
                            .cloned()
                    })
                    .collect();
                (content_type, sources)
            })
            .collect();

        *self
            .routes
            .write()
            .unwrap_or_else(|error| error.into_inner()) = next;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Capability, Registry, ScrapedMetadata, Source, SourceDescriptor};
    use amane_core::config::ContentRoutes;
    use amane_core::{ContentType, MediaNumber};
    use amane_net::Rate;
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
        stub_on(id, "example.invalid", Rate::each_second(1, 1), capabilities)
    }

    fn stub_on(
        id: &'static str,
        host: &'static str,
        rate: Rate,
        capabilities: &'static [Capability],
    ) -> Box<dyn Source> {
        Box::new(StubSource {
            descriptor: SourceDescriptor {
                id,
                label: id,
                base_url: "https://example.invalid",
                host,
                rate,
                capabilities,
            },
            title: id,
        })
    }

    /// 只关心某一类的链, 其余类型用默认值即可.
    fn routes_for(chain: &[&str]) -> ContentRoutes {
        let body = serde_json::json!({ "censored": chain }).to_string();
        serde_json::from_str(&body).expect("应解析成功")
    }

    fn chain_ids(registry: &Registry, content_type: ContentType) -> Vec<&'static str> {
        registry
            .chain_for(content_type)
            .iter()
            .map(|source| source.descriptor().id)
            .collect()
    }

    #[test]
    fn chain_order_follows_configuration() {
        let registry = Registry::new(vec![
            stub("first", &[Capability::Metadata]),
            stub("second", &[Capability::Metadata]),
        ]);

        registry.apply_routes(&routes_for(&["second", "first"]));

        assert_eq!(
            chain_ids(&registry, ContentType::Censored),
            ["second", "first"],
            "链上的顺序即聚合优先级"
        );
    }

    #[test]
    fn source_outside_the_chain_is_not_returned() {
        let registry = Registry::new(vec![
            stub("first", &[Capability::Metadata]),
            stub("second", &[Capability::Metadata]),
        ]);

        registry.apply_routes(&routes_for(&["second"]));

        assert_eq!(
            chain_ids(&registry, ContentType::Censored),
            ["second"],
            "不在链上的来源不参与"
        );
    }

    #[test]
    fn each_content_type_has_its_own_chain() {
        let registry = Registry::new(vec![
            stub("first", &[Capability::Metadata]),
            stub("second", &[Capability::Metadata]),
        ]);

        registry.apply_routes(&routes_for(&["first"]));

        assert_eq!(chain_ids(&registry, ContentType::Censored), ["first"]);
        assert!(
            chain_ids(&registry, ContentType::Uncensored).is_empty(),
            "默认链里的来源未登记时跳过, 不串到别的类型"
        );
    }

    #[test]
    fn unknown_id_in_chain_is_skipped() {
        let registry = Registry::new(vec![stub("first", &[Capability::Metadata])]);

        registry.apply_routes(&routes_for(&["nowhere", "first"]));

        assert_eq!(
            chain_ids(&registry, ContentType::Censored),
            ["first"],
            "未登记的标识跳过而不是整条链作废"
        );
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

        assert_eq!(
            registry.known_ids(),
            ["dmm", "javdb", "javbus"],
            "内置来源全部登记在册"
        );
        assert!(
            registry.chain_for(ContentType::Censored).is_empty(),
            "未应用路由前没有任何类型有链"
        );
    }

    #[test]
    fn same_host_takes_the_more_conservative_rate() {
        let registry = Registry::new(vec![
            stub_on(
                "fast",
                "shared.example",
                Rate::each_second(4, 8),
                &[Capability::Metadata],
            ),
            stub_on(
                "slow",
                "shared.example",
                Rate::each_second(1, 1),
                &[Capability::Metadata],
            ),
        ]);

        let rates = registry.outbound_rates(&BTreeMap::new());

        assert_eq!(
            rates.get("shared.example"),
            Some(&Rate::each_second(1, 1)),
            "同一 host 取更保守的一份"
        );
    }

    #[test]
    fn configuration_override_beats_declaration() {
        let registry = Registry::new(vec![stub_on(
            "fast",
            "fast.example",
            Rate::each_second(4, 8),
            &[Capability::Metadata],
        )]);
        let mut overrides = BTreeMap::new();
        overrides.insert("fast.example".to_owned(), 0.5);

        let rates = registry.outbound_rates(&overrides);

        assert_eq!(
            rates.get("fast.example"),
            Some(&Rate::per_second(1, 0.5)),
            "配置覆盖优先于来源声明"
        );
    }

    #[test]
    fn hosts_without_declaration_are_absent_from_the_table() {
        let registry = Registry::new(vec![stub("only", &[Capability::Metadata])]);

        let rates = registry.outbound_rates(&BTreeMap::new());

        assert_eq!(rates.len(), 1, "只登记声明过的 host");
        assert!(
            !rates.contains_key("cdn.example"),
            "未声明的 host 落到默认速率, 不凭空出现"
        );
    }
}
