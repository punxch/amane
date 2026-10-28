use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// 启用的来源标识; 为空表示全部启用.
    #[schemars(extend("x-widget" = "list"))]
    pub enabled_sources: Vec<String>,
}

impl Default for HotSettings {
    fn default() -> Self {
        Self {
            concurrency: 4,
            request_timeout_secs: 20,
            enabled_sources: Vec::new(),
        }
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

        for source in &self.enabled_sources {
            if !known_sources.contains(&source.as_str()) {
                return Err(ConfigError::UnknownSource(source.clone()));
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
    #[error("应用配置失败: {0}")]
    Submit(String),
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, HotSettings, Settings};

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
            enabled_sources: vec!["dmm".to_owned()],
        };

        assert!(settings.validate(&["dmm", "javdb"]).is_ok());
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
                .validate(&["dmm"])
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
            enabled_sources: vec!["nowhere".to_owned()],
            ..HotSettings::default()
        };

        match settings.validate(&["dmm"]) {
            Err(ConfigError::UnknownSource(name)) => assert_eq!(name, "nowhere"),
            other => panic!("应为未知来源错误, 实际 {other:?}"),
        }
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
