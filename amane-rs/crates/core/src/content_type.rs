use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 条目类型.
///
/// 与马赛克标记正交: 类型描述条目属于哪一类作品, 标记描述这一份文件自身的证据.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
    sqlx::Type,
)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum ContentType {
    Censored,
    Uncensored,
    Chinese,
    Western,
    Fc2,
    Amateur,
    Hentai,
}

impl ContentType {
    /// 全部取值, 顺序即界面上的展示顺序.
    pub const ALL: [Self; 7] = [
        Self::Censored,
        Self::Uncensored,
        Self::Chinese,
        Self::Western,
        Self::Fc2,
        Self::Amateur,
        Self::Hentai,
    ];

    /// 面向用户的名称. 界面不翻译枚举值, 只展示这里的名称.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Censored => "有码",
            Self::Uncensored => "无码",
            Self::Chinese => "中文字幕",
            Self::Western => "欧美",
            Self::Fc2 => "FC2",
            Self::Amateur => "素人",
            Self::Hentai => "动画",
        }
    }

    /// 由外部字符串还原; 未知取值返回 `None`, 由调用方决定如何报错.
    ///
    /// 取值来自 `as_str`, 不另立一份清单: 两份清单迟早会分叉.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Censored => "censored",
            Self::Uncensored => "uncensored",
            Self::Chinese => "chinese",
            Self::Western => "western",
            Self::Fc2 => "fc2",
            Self::Amateur => "amateur",
            Self::Hentai => "hentai",
        }
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::ContentType;

    #[test]
    fn parse_round_trips_every_variant() {
        for value in ContentType::ALL {
            assert_eq!(ContentType::parse(value.as_str()), Some(value));
            assert!(!value.label().is_empty(), "每个取值都要有展示名称");
        }

        assert_eq!(ContentType::parse("Censored"), None, "取值区分大小写");
        assert_eq!(ContentType::parse(""), None);
    }

    #[test]
    fn serializes_as_lowercase() {
        let cases = [
            (ContentType::Censored, "\"censored\""),
            (ContentType::Fc2, "\"fc2\""),
            (ContentType::Amateur, "\"amateur\""),
        ];

        for (value, expected) in cases {
            let encoded = serde_json::to_string(&value).expect("序列化不应失败");
            assert_eq!(encoded, expected);
        }
    }
}
