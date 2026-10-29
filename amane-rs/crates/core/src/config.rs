use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::content_type::ContentType;

/// 冷配置: 修改后需要重启进程才能生效.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ColdSettings {
    /// 数据目录. 数据库与资源文件都落在其下.
    pub data_dir: PathBuf,
    /// 监听地址. 桌面端由外壳分配端口, 服务端由配置文件或命令行指定.
    pub bind: String,
    /// 访问令牌; 为空时不校验.
    pub api_token: Option<String>,
}

impl Default for ColdSettings {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            bind: "127.0.0.1:8787".to_owned(),
            api_token: None,
        }
    }
}

/// 热配置: 修改后就地生效, 不需要重启进程, 正在运行的任务不受影响.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HotSettings {
    /// 同时运行的刮削任务上限.
    #[schemars(range(min = 1, max = 64), extend("x-widget" = "slider"))]
    pub concurrency: u32,
    /// 单次上游请求的超时时间.
    #[schemars(range(min = 1, max = 600), extend("x-widget" = "number"))]
    pub request_timeout_secs: u32,
    /// 每类影片的来源链; 链上顺序即字段优先级.
    #[schemars(extend("x-widget" = "content-routes"))]
    pub content_routes: ContentRoutes,
    /// 按 host 覆盖出站速率: 键是主机名, 值是每秒请求数.
    ///
    /// 优先级高于来源自己声明的速率. 覆盖后突发上限按 1 处理, 即严格匀速 —— 被限流时
    /// 用户要的是「放慢」, 不是「还能瞬间打一串」.
    pub host_rates: BTreeMap<String, f64>,
    /// 扫描规则: 媒体扩展名、大小阈值与文件黑名单.
    pub scan: ScanSettings,
    /// 整理规则: 目标路径模板.
    pub organize: OrganizeSettings,
}

impl Default for HotSettings {
    fn default() -> Self {
        Self {
            concurrency: 4,
            request_timeout_secs: 20,
            content_routes: ContentRoutes::default(),
            host_rates: BTreeMap::new(),
            scan: ScanSettings::default(),
            organize: OrganizeSettings::default(),
        }
    }
}

/// 类型路由: 内容类型 → 有序来源标识链.
///
/// 七类键冻结: 缺键补默认, 未知键丢弃. 保存时校验每条链非空且成员都是已知来源.
/// 未知键丢弃是给「配置文件来自更新的版本」留的余地: 旧版本读不懂的键直接忽略,
/// 而不是让整份配置读不出来.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ContentRoutes(BTreeMap<ContentType, Vec<String>>);

impl ContentRoutes {
    /// 某一类型的来源链; 未登记的类型返回空链.
    pub fn chain(&self, content_type: ContentType) -> &[String] {
        self.0
            .get(&content_type)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// 逐类型给出, 顺序即 [`ContentType::ALL`].
    pub fn entries(&self) -> impl Iterator<Item = (ContentType, &[String])> {
        ContentType::ALL
            .into_iter()
            .map(|content_type| (content_type, self.chain(content_type)))
    }

    /// 替换某一类型的来源链, 其余类型保持原样.
    pub fn with_chain(self, content_type: ContentType, chain: Vec<String>) -> Self {
        let mut routes = self.0;
        routes.insert(content_type, chain);
        Self(routes)
    }

    /// 由「类型 → 标识链」逐项构造; 未给出的类型取默认链.
    pub fn from_chains(chains: impl IntoIterator<Item = (ContentType, Vec<String>)>) -> Self {
        let mut routes = Self::default();
        for (content_type, chain) in chains {
            routes.0.insert(content_type, chain);
        }
        routes
    }

    /// 校验每条链非空且成员都是已知来源.
    pub fn validate(&self, known_sources: &[&str]) -> Result<(), ConfigError> {
        for (content_type, chain) in self.entries() {
            if chain.is_empty() {
                return Err(ConfigError::BadValue {
                    field: "content_routes",
                    reason: format!("{} 的来源链为空, 该类型将无人可用", content_type.label()),
                });
            }
            for source in chain {
                if !known_sources.contains(&source.as_str()) {
                    return Err(ConfigError::UnknownSource(source.clone()));
                }
            }
        }

        Ok(())
    }
}

impl<'de> Deserialize<'de> for ContentRoutes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = BTreeMap::<String, Vec<String>>::deserialize(deserializer)?;

        // 先铺默认再覆盖: 缺的键得到默认链, 未知的键被丢弃.
        let mut routes = Self::default().0;
        for (key, chain) in raw {
            if let Some(content_type) = ContentType::parse(&key) {
                routes.insert(content_type, chain);
            }
        }

        Ok(Self(routes))
    }
}

/// 默认来源链.
///
/// 只列已实现的来源: 校验要求每个标识都是已知来源, 列上还没落地的站点会让默认配置
/// 自己就存不回去. 每落地一个来源, 在它该出现的类型里追加.
fn default_content_routes() -> BTreeMap<ContentType, Vec<String>> {
    let chain = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();

    [
        (ContentType::Censored, chain(&["dmm", "javdb", "javbus"])),
        (ContentType::Uncensored, chain(&["javdb", "javbus"])),
        (ContentType::Fc2, chain(&["javdb"])),
        (ContentType::Chinese, chain(&["javdb"])),
        (ContentType::Amateur, chain(&["dmm", "javdb", "javbus"])),
        (ContentType::Western, chain(&["javdb"])),
        (ContentType::Hentai, chain(&["dmm", "javdb"])),
    ]
    .into_iter()
    .collect()
}

impl Default for ContentRoutes {
    fn default() -> Self {
        Self(default_content_routes())
    }
}

/// 整理规则. 模板在保存时解析校验, 渲染时不再处理语法错误.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OrganizeSettings {
    /// 视频目标路径模板; 相对库根, 占位符见 `PLACEHOLDERS`.
    pub video_template: String,
    /// 随视频一起整理的字幕扩展名; 空列表关闭字幕随配.
    #[schemars(extend("x-widget" = "list"))]
    pub subtitle_extensions: Vec<String>,
    /// 是否为每个视频生成 STRM 指针文件, 正文是视频的完整绝对路径.
    pub create_strm: bool,
}

impl Default for OrganizeSettings {
    fn default() -> Self {
        Self {
            video_template: "{maker}/{number}/{number}.{ext}".to_owned(),
            subtitle_extensions: crate::organize::DEFAULT_SUBTITLE_EXTENSIONS
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            create_strm: false,
        }
    }
}

impl OrganizeSettings {
    pub fn validate(&self) -> Result<(), ConfigError> {
        crate::organize::Template::parse(&self.video_template).map_err(|error| {
            ConfigError::BadValue {
                field: "organize.video_template",
                reason: error.to_string(),
            }
        })?;

        for extension in &self.subtitle_extensions {
            let normalized = crate::library::normalize_extension(extension);
            let body = normalized.trim_start_matches('.');
            if body.is_empty()
                || !body
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                return Err(ConfigError::BadValue {
                    field: "organize.subtitle_extensions",
                    reason: format!("扩展名 {extension:?} 不合法"),
                });
            }
        }

        Ok(())
    }
}

/// 扫描规则. 每次扫描开始时从配置快照编译, 改完下一次扫描即生效.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ScanSettings {
    /// 视为媒体的扩展名; 带前导点, 大小写不敏感.
    #[schemars(extend("x-widget" = "list"))]
    pub media_extensions: Vec<String>,
    /// 最小视频字节数; 0 表示关闭大小过滤.
    #[schemars(range(min = 0), extend("x-widget" = "number"))]
    pub min_file_size: u64,
    /// 文件黑名单正则; 与文件名 (含扩展名) 匹配, 命中即归入无效文件.
    #[schemars(extend("x-widget" = "list"))]
    pub blacklist_patterns: Vec<String>,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            media_extensions: crate::library::DEFAULT_MEDIA_EXTENSIONS
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            min_file_size: 0,
            blacklist_patterns: Vec::new(),
        }
    }
}

impl ScanSettings {
    /// 校验扩展名与黑名单正则. 非法取值在提交时拒绝, 不等下一次扫描才发现.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for extension in &self.media_extensions {
            let normalized = crate::library::normalize_extension(extension);
            let body = normalized.trim_start_matches('.');
            if body.is_empty()
                || !body
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                return Err(ConfigError::BadValue {
                    field: "scan.media_extensions",
                    reason: format!("扩展名 {extension:?} 不合法"),
                });
            }
        }

        for pattern in &self.blacklist_patterns {
            if regex::Regex::new(pattern).is_err() {
                return Err(ConfigError::BadValue {
                    field: "scan.blacklist_patterns",
                    reason: format!("正则 {pattern:?} 无法编译"),
                });
            }
        }

        Ok(())
    }
}

impl HotSettings {
    /// 校验取值范围与来源取值. schema 的 range 只影响表单, 服务端必须自己拦住越界值.
    pub fn validate(&self, known_sources: &[&str]) -> Result<(), ConfigError> {
        range("concurrency", u64::from(self.concurrency), 1, 64)?;
        range(
            "request_timeout_secs",
            u64::from(self.request_timeout_secs),
            1,
            600,
        )?;

        self.content_routes.validate(known_sources)?;
        self.validate_host_rates()?;

        self.scan.validate()?;
        self.organize.validate()
    }

    /// 速率覆盖必须能构成一个可用的令牌桶: 主机名非空, 每秒次数为正数.
    ///
    /// 零或负的速率会让限速器退化成「每秒一个」, 用户以为放慢了却得到默认值; 负的还会
    /// 让等待时长算成零, 直接变成不限速.
    fn validate_host_rates(&self) -> Result<(), ConfigError> {
        for (host, per_second) in &self.host_rates {
            if host.trim().is_empty() {
                return Err(ConfigError::BadValue {
                    field: "host_rates",
                    reason: "主机名不能为空".to_owned(),
                });
            }
            if host.contains('/') || host.contains(':') {
                return Err(ConfigError::BadValue {
                    field: "host_rates",
                    reason: format!("{host:?} 应是主机名, 不带协议与路径"),
                });
            }
            if !per_second.is_finite() || *per_second <= 0.0 {
                return Err(ConfigError::BadValue {
                    field: "host_rates",
                    reason: format!("{host} 的每秒请求数应为正数, 当前是 {per_second}"),
                });
            }
        }

        Ok(())
    }
}

/// 判断取值是否落在闭区间内.
fn range(field: &'static str, value: u64, min: u64, max: u64) -> Result<(), ConfigError> {
    if value < min || value > max {
        return Err(ConfigError::OutOfRange {
            field,
            value,
            min,
            max,
        });
    }

    Ok(())
}

/// 进程配置: 冷层只有重启后才可能变化, 热层就地替换.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub cold: ColdSettings,
    pub hot: HotSettings,
}

/// 配置取值不合法. 携带字段名与取值范围, 界面据此定位到具体控件.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{field} 应取 {min} 到 {max} 之间的整数, 当前是 {value}")]
    OutOfRange {
        field: &'static str,
        value: u64,
        min: u64,
        max: u64,
    },
    #[error("未知来源: {0}")]
    UnknownSource(String),
    #[error("{field} 不合法: {reason}")]
    BadValue { field: &'static str, reason: String },
    #[error("应用配置失败: {0}")]
    Submit(String),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        ConfigError, ContentRoutes, HotSettings, OrganizeSettings, ScanSettings, Settings,
    };
    use crate::content_type::ContentType;

    /// 已实现的来源标识. 默认链与校验都按这一份走.
    const KNOWN: &[&str] = &["dmm", "javdb", "javbus"];

    fn routes_with(content_type: ContentType, chain: &[&str]) -> ContentRoutes {
        let mut routes = ContentRoutes::default();
        routes.0.insert(
            content_type,
            chain.iter().map(|id| (*id).to_owned()).collect(),
        );
        routes
    }

    fn chain_of(routes: &ContentRoutes, content_type: ContentType) -> Vec<&str> {
        routes
            .chain(content_type)
            .iter()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn organize_settings_reject_malformed_template() {
        let settings = OrganizeSettings {
            video_template: "{number".to_owned(),
            ..OrganizeSettings::default()
        };
        match settings.validate() {
            Err(ConfigError::BadValue { field, .. }) => {
                assert_eq!(field, "organize.video_template");
            }
            other => panic!("应为模板错误, 实际 {other:?}"),
        }

        let settings = OrganizeSettings {
            video_template: "{plot}".to_owned(),
            ..OrganizeSettings::default()
        };
        match settings.validate() {
            Err(ConfigError::BadValue { reason, .. }) => {
                assert!(reason.contains("plot"), "应指出未知占位符, 实际 {reason}");
            }
            other => panic!("应为占位符错误, 实际 {other:?}"),
        }
    }

    #[test]
    fn scan_settings_reject_bad_extension_and_regex() {
        let settings = ScanSettings {
            media_extensions: vec!["mp4".to_owned(), "not an ext!".to_owned()],
            ..ScanSettings::default()
        };
        match settings.validate() {
            Err(ConfigError::BadValue { field, .. }) => {
                assert_eq!(field, "scan.media_extensions");
            }
            other => panic!("应为扩展名错误, 实际 {other:?}"),
        }

        let settings = ScanSettings {
            blacklist_patterns: vec!["(?i".to_owned()],
            ..ScanSettings::default()
        };
        match settings.validate() {
            Err(ConfigError::BadValue { field, .. }) => {
                assert_eq!(field, "scan.blacklist_patterns");
            }
            other => panic!("应为正则错误, 实际 {other:?}"),
        }
    }

    #[test]
    fn hot_settings_reject_unknown_fields() {
        let parsed = serde_json::from_str::<HotSettings>(r#"{"concurrency": 4, "unknown_key": 1}"#);
        assert!(parsed.is_err(), "未知字段必须拒绝");
    }

    #[test]
    fn hot_settings_carry_form_extensions() {
        let schema =
            serde_json::to_value(schemars::schema_for!(HotSettings)).expect("应生成 schema");
        let widget = &schema["properties"]["concurrency"]["x-widget"];

        assert_eq!(widget, "slider", "x-widget 必须保留在 schema 中");
    }

    #[test]
    fn hot_settings_accept_in_range_values() {
        let settings = HotSettings {
            concurrency: 1,
            request_timeout_secs: 600,
            ..HotSettings::default()
        };

        assert!(settings.validate(KNOWN).is_ok());
    }

    #[test]
    fn hot_settings_reject_out_of_range_values() {
        let cases = [
            (0, 20, "concurrency"),
            (65, 20, "concurrency"),
            (4, 0, "request_timeout_secs"),
            (4, 601, "request_timeout_secs"),
        ];

        for (concurrency, timeout, field) in cases {
            let settings = HotSettings {
                concurrency,
                request_timeout_secs: timeout,
                ..HotSettings::default()
            };
            let error = settings
                .validate(KNOWN)
                .expect_err(&format!("{field}={concurrency}/{timeout} 应被拒绝"));

            match error {
                ConfigError::OutOfRange { field: name, .. } => assert_eq!(name, field),
                other => panic!("应为越界错误, 实际 {other:?}"),
            }
        }
    }

    #[test]
    fn hot_settings_reject_unknown_source() {
        let settings = HotSettings {
            content_routes: routes_with(ContentType::Censored, &["nowhere"]),
            ..HotSettings::default()
        };

        match settings.validate(KNOWN) {
            Err(ConfigError::UnknownSource(name)) => assert_eq!(name, "nowhere"),
            other => panic!("应为未知来源错误, 实际 {other:?}"),
        }
    }

    #[test]
    fn content_routes_reject_empty_chain() {
        let routes = routes_with(ContentType::Censored, &[]);

        match routes.validate(KNOWN) {
            Err(ConfigError::BadValue { reason, .. }) => {
                assert!(reason.contains("有码"), "应指出是哪一类, 实际 {reason}");
            }
            other => panic!("空链必须拒绝, 实际 {other:?}"),
        }
    }

    #[test]
    fn content_routes_fill_missing_types_from_default() {
        let parsed: ContentRoutes =
            serde_json::from_str(r#"{"censored": ["javdb"]}"#).expect("应解析成功");

        assert_eq!(chain_of(&parsed, ContentType::Censored), ["javdb"]);
        assert_eq!(
            chain_of(&parsed, ContentType::Western),
            chain_of(&ContentRoutes::default(), ContentType::Western),
            "缺的键补默认"
        );
    }

    #[test]
    fn content_routes_drop_unknown_type_keys() {
        let parsed: ContentRoutes =
            serde_json::from_str(r#"{"podcast": ["dmm"]}"#).expect("未知键不应让整份配置读不出来");

        assert_eq!(
            parsed,
            ContentRoutes::default(),
            "未知类型丢弃, 其余保持默认"
        );
    }

    #[test]
    fn default_content_routes_reference_implemented_sources() {
        // 默认链里出现未实现的来源, 会让默认配置自己就存不回去.
        assert!(
            ContentRoutes::default().validate(KNOWN).is_ok(),
            "默认链只列已实现的来源"
        );
    }

    #[test]
    fn default_routes_give_every_type_a_source() {
        for (content_type, chain) in ContentRoutes::default().entries() {
            assert!(
                !chain.is_empty(),
                "{} 的默认链不能为空",
                content_type.label()
            );
        }
    }

    #[test]
    fn host_rates_reject_unusable_values() {
        let cases = [
            ("", 1.0, "主机名"),
            ("https://javdb.com", 1.0, "主机名"),
            ("javdb.com", 0.0, "正数"),
            ("javdb.com", -1.0, "正数"),
        ];

        for (host, per_second, expected) in cases {
            let mut rates = BTreeMap::new();
            rates.insert(host.to_owned(), per_second);
            let settings = HotSettings {
                host_rates: rates,
                ..HotSettings::default()
            };

            match settings.validate(KNOWN) {
                Err(ConfigError::BadValue { field, reason }) => {
                    assert_eq!(field, "host_rates");
                    assert!(
                        reason.contains(expected),
                        "{host}={per_second} 的说明应提到{expected}, 实际 {reason}"
                    );
                }
                other => panic!("{host}={per_second} 应被拒绝, 实际 {other:?}"),
            }
        }
    }

    #[test]
    fn host_rates_accept_positive_values() {
        let mut rates = BTreeMap::new();
        rates.insert("javdb.com".to_owned(), 0.5);
        let settings = HotSettings {
            host_rates: rates,
            ..HotSettings::default()
        };

        assert!(settings.validate(KNOWN).is_ok());
    }

    #[test]
    fn settings_fill_defaults_for_absent_fields() {
        let parsed = serde_json::from_str::<Settings>(r#"{"cold": {"bind": "0.0.0.0:9000"}}"#)
            .expect("缺省字段应填入默认值");

        assert_eq!(parsed.cold.bind, "0.0.0.0:9000");
        assert_eq!(parsed.cold.data_dir, std::path::PathBuf::from("./data"));
        assert_eq!(parsed.hot.concurrency, 4);
    }
}
