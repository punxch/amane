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

    /// 路径关键词给出的类型.
    ///
    /// 目录段与文件名一起看, 只认段首: 「这里番号」与「非欧美」不算.
    /// 关键词比番号形态可靠 —— 番号形态只能猜出字母前缀属于哪一套编号体系.
    pub fn from_path(path: &str) -> Option<Self> {
        for segment in path.split(['/', '\\']) {
            let name = segment.trim();
            if name.is_empty() {
                continue;
            }
            if name.eq_ignore_ascii_case("getchu")
                || name.starts_with("里番")
                || name.starts_with("裏番")
            {
                return Some(Self::Hentai);
            }
            if name.starts_with("欧美") {
                return Some(Self::Western);
            }
        }

        None
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
    use crate::MediaNumber;

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

    #[test]
    fn path_keywords_decide_the_type() {
        let cases = [
            ("里番/ABC-123.mp4", Some(ContentType::Hentai)),
            ("裏番/ABC-123.mp4", Some(ContentType::Hentai)),
            ("/media/getchu/ABC-123.mp4", Some(ContentType::Hentai)),
            ("Getchu/ABC-123.mp4", Some(ContentType::Hentai)),
            ("欧美/ABC-123.mp4", Some(ContentType::Western)),
            ("欧美合集/ABC-123.mp4", Some(ContentType::Western)),
            // 关键词只在段首算数.
            ("这里番号/ABC-123.mp4", None),
            ("非欧美/ABC-123.mp4", None),
            ("ABC-123.mp4", None),
        ];

        for (path, expected) in cases {
            assert_eq!(ContentType::from_path(path), expected, "路径 {path}");
        }
    }

    #[test]
    fn path_keywords_beat_number_shape() {
        // 番号本身看着像有码, 但它在里番目录下.
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        assert_eq!(number.content_type(), ContentType::Censored);
        assert_eq!(
            ContentType::from_path("里番/ABC-123.mp4"),
            Some(ContentType::Hentai),
            "路径关键词优先"
        );
    }
}
