use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 条目类型.
///
/// 与马赛克标记正交: 类型描述条目属于哪一类作品, 标记描述这一份文件自身的证据.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, sqlx::Type,
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
