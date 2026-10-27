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

/// 热配置: 修改后由运行时重建生效, 不需要重启进程.
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

/// 进程配置: 冷热两层分别持有, 热层整体替换.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub cold: ColdSettings,
    pub hot: HotSettings,
}

#[cfg(test)]
mod tests {
    use super::{HotSettings, Settings};

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
    fn settings_fill_defaults_for_absent_fields() {
        let parsed = serde_json::from_str::<Settings>(r#"{"cold": {"bind": "0.0.0.0:9000"}}"#)
            .expect("缺省字段应填入默认值");

        assert_eq!(parsed.cold.bind, "0.0.0.0:9000");
        assert_eq!(parsed.cold.data_dir, std::path::PathBuf::from("./data"));
        assert_eq!(parsed.hot.concurrency, 4);
    }
}
