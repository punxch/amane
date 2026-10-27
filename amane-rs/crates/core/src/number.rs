//! 番号: 从自由文本或文件名提取, 归一为 `大写前缀-数字` 的标准形态.
//!
//! 首版只覆盖四种形态: FC2、六位数字素人、字母前缀带分隔符、字母与数字连写.
//! 欧美日期型番号与素人数字段补齐尚未移植, 见 docs/dev/parsing.md.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::content_type::ContentType;

/// 无码番号前缀. 前缀自带分隔符; 未带分隔符的按字母段整体匹配.
const UNCENSORED_PREFIXES: &[&str] = &[
    "BT-", "CT-", "EMP-", "CCDV-", "CWP-", "CWPBD-", "DSAM-", "DRC-", "DRG-", "DRGBD-", "GACHI-",
    "GACHIG-", "HEYDOUGA", "JAV-", "LAF-", "LAFBD-", "HEYZO-", "KTG-", "KP-", "KG-", "LLDV-",
    "MCDV-", "MKD-", "MKBD-", "MMDV-", "NIP-", "PB-", "PT-", "QE-", "RED-", "RHJ-", "S2M-", "SKY-",
    "SKYHD-", "SMD-", "SMBD-", "SMDV-", "SSDV-", "SSKP-", "TRG-", "TS-", "XXX-AV-", "YKB-", "BIRD",
    "BOUGA", "H4610-", "C0930-", "H0930-", "S2M", "MCB3D",
];

/// 素人系站内号前缀; 只参与类型判定.
const SUREN_LABELS: &[&str] = &["LUXU-", "ARA-", "MIUM-", "NTK-", "MAAN-"];

/// 前缀内含数字的有码番号.
const CENSORED_PREFIXES: &[&str] = &["R18-"];

static FC2_WHOLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^fc2[-_ ]?(?:ppv[-_ ]?)?(\d{5,8})$").expect("固定正则"));
static AMATEUR_WHOLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{6})[-_](\d{2,4})$").expect("固定正则"));
static DASHED_WHOLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^([a-z]{1,6}\d{0,4})-(\d{2,5})(z?)$").expect("固定正则"));
static CONCAT_WHOLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^([a-z]{2,6})(\d{2,5})$").expect("固定正则"));

static FC2_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)fc2[-_ ]?(?:ppv[-_ ]?)?(\d{5,8})").expect("固定正则"));
static AMATEUR_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{6})[-_](\d{2,4})\b").expect("固定正则"));
static DASHED_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([a-z]{1,6}\d{0,4})-(\d{2,5})[a-z]?\b").expect("固定正则"));
static CONCAT_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([a-z]{2,6})(\d{2,5})\b").expect("固定正则"));

static AMATEUR_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{6}[-_]\d{2,4}$").expect("固定正则"));
static WESTERN_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^.]+\.\d{2}\.\d{2}\.\d{2}").expect("固定正则"));
static N_SERIES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^n\d{4}").expect("固定正则"));
static AMATEUR_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{3,}[A-Z]+-\d{2}").expect("固定正则"));
static CHINESE_MD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([^A-Z]|^)MD[A-Z-]*\d{4,}").expect("固定正则"));
static CHINESE_MKY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"MKY-[A-Z]+-\d{3,}").expect("固定正则"));
static DIGIT_PREFIX_CATALOG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{1,4}\d{1,4}-\d{2,4}[A-Z]?$").expect("固定正则"));
static CATALOG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Z]{2,}-\d{2,}Z?").expect("固定正则"));

/// 番号解析失败的原因.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NumberError {
    #[error("番号为空白")]
    Empty,
    #[error("无法识别番号: {0}")]
    Unrecognized(String),
}

/// 规范化后的番号.
///
/// 相等判定只比较范化后的字符串; 素人数字段补齐不参与比较.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaNumber(String);

impl MediaNumber {
    /// 整串解析. 字符串必须整体是番号, 带扩展名或附加文本时使用 [`MediaNumber::extract`].
    pub fn parse(raw: &str) -> Result<Self, NumberError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(NumberError::Empty);
        }
        canonicalize(trimmed)
            .map(Self)
            .ok_or_else(|| NumberError::Unrecognized(trimmed.to_owned()))
    }

    /// 从自由文本或完整路径中提取第一个番号.
    pub fn extract(text: &str) -> Option<Self> {
        extract_number(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 依据形态与前缀表判定条目类型.
    pub fn content_type(&self) -> ContentType {
        classify(&self.0)
    }

    /// 分隔符前的字母段; 无分隔符时取首次出现的字母段.
    pub fn prefix(&self) -> &str {
        match self.0.split_once('-') {
            Some((head, _)) => head,
            None => self.0.trim_end_matches(|c: char| !c.is_ascii_alphabetic()),
        }
    }
}

impl fmt::Display for MediaNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for MediaNumber {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

fn canonicalize(raw: &str) -> Option<String> {
    if let Some(captures) = FC2_WHOLE.captures(raw) {
        return Some(format!("FC2-{}", &captures[1]));
    }
    if let Some(captures) = AMATEUR_WHOLE.captures(raw) {
        return Some(format!("{}_{}", &captures[1], &captures[2]));
    }
    if let Some(captures) = DASHED_WHOLE.captures(raw) {
        let head = captures[1].to_ascii_uppercase();
        let tail = &captures[2];
        let suffix = captures[3].to_ascii_uppercase();
        return Some(format!("{head}-{tail}{suffix}"));
    }
    if let Some(captures) = CONCAT_WHOLE.captures(raw) {
        let head = captures[1].to_ascii_uppercase();
        return Some(format!("{head}-{}", &captures[2]));
    }
    None
}

fn extract_number(text: &str) -> Option<String> {
    if let Some(captures) = FC2_TOKEN.captures(text) {
        return Some(format!("FC2-{}", &captures[1]));
    }
    if let Some(captures) = AMATEUR_TOKEN.captures(text) {
        return Some(format!("{}_{}", &captures[1], &captures[2]));
    }
    if let Some(captures) = DASHED_TOKEN.captures(text) {
        let head = captures[1].to_ascii_uppercase();
        return Some(format!("{head}-{}", &captures[2]));
    }
    CONCAT_TOKEN
        .captures(text)
        .map(|captures| format!("{}-{}", captures[1].to_ascii_uppercase(), &captures[2]))
}

fn classify(number: &str) -> ContentType {
    let upper = number.to_ascii_uppercase();
    if upper.starts_with("FC2-") {
        return ContentType::Fc2;
    }
    if AMATEUR_CLASS.is_match(&upper) || N_SERIES.is_match(&upper) || WESTERN_DATE.is_match(&upper)
    {
        return ContentType::Uncensored;
    }
    if UNCENSORED_PREFIXES
        .iter()
        .any(|prefix| upper.starts_with(prefix))
    {
        return ContentType::Uncensored;
    }
    if CENSORED_PREFIXES
        .iter()
        .any(|prefix| upper.starts_with(prefix))
    {
        return ContentType::Censored;
    }
    if upper.contains("SIRO")
        || AMATEUR_SHAPE.is_match(&upper)
        || SUREN_LABELS.iter().any(|label| upper.starts_with(label))
    {
        return ContentType::Amateur;
    }
    if (CHINESE_MD.is_match(&upper) && !upper.contains("MDVR")) || CHINESE_MKY.is_match(&upper) {
        return ContentType::Chinese;
    }
    if DIGIT_PREFIX_CATALOG.is_match(&upper) || CATALOG.is_match(&upper) {
        return ContentType::Censored;
    }
    ContentType::Western
}

#[cfg(test)]
mod tests {
    use super::{ContentType, MediaNumber, NumberError};

    #[test]
    fn parses_whole_number() {
        let cases = [
            ("ABC-123", "ABC-123", ContentType::Censored),
            ("abc-123", "ABC-123", ContentType::Censored),
            ("SSIS001", "SSIS-001", ContentType::Censored),
            ("abc123", "ABC-123", ContentType::Censored),
            ("FC2-PPV-1234567", "FC2-1234567", ContentType::Fc2),
            ("FC2-1234567", "FC2-1234567", ContentType::Fc2),
            ("111111_001", "111111_001", ContentType::Uncensored),
            ("T28-123", "T28-123", ContentType::Censored),
            ("HEYZO-1234", "HEYZO-1234", ContentType::Uncensored),
            ("ABC-123Z", "ABC-123Z", ContentType::Censored),
        ];

        for (input, expected_number, expected_type) in cases {
            let parsed = MediaNumber::parse(input).expect("应解析成功");
            assert_eq!(parsed.as_str(), expected_number, "输入 {input}");
            assert_eq!(parsed.content_type(), expected_type, "输入 {input}");
        }
    }

    #[test]
    fn rejects_invalid_whole_strings() {
        let cases = [
            ("", NumberError::Empty),
            ("   ", NumberError::Empty),
            (
                "not-a-number",
                NumberError::Unrecognized("not-a-number".to_owned()),
            ),
            (
                "ABC-123.mp4",
                NumberError::Unrecognized("ABC-123.mp4".to_owned()),
            ),
            ("1234567", NumberError::Unrecognized("1234567".to_owned())),
            ("--", NumberError::Unrecognized("--".to_owned())),
        ];

        for (input, expected) in cases {
            assert_eq!(MediaNumber::parse(input), Err(expected), "输入 {input}");
        }
    }

    #[test]
    fn extracts_from_text_and_paths() {
        let cases = [
            ("ABC-123.mp4", Some("ABC-123")),
            ("[javdb]ABC-123-C.mp4", Some("ABC-123")),
            ("/movies/ABC-123/ABC-123.mp4", Some("ABC-123")),
            ("ABP-123-4K.mp4", Some("ABP-123")),
            ("IPX-001-C.mp4", Some("IPX-001")),
            ("SSIS001.mp4", Some("SSIS-001")),
            ("111111_001.mp4", Some("111111_001")),
            ("FC2-PPV-1234567.mp4", Some("FC2-1234567")),
            ("完全沒有番号", None),
            ("", None),
            ("2024-01-15", None),
        ];

        for (input, expected) in cases {
            let extracted = MediaNumber::extract(input).map(|number| number.as_str().to_owned());
            assert_eq!(extracted.as_deref(), expected, "输入 {input}");
        }
    }

    #[test]
    fn prefix_splits_on_separator() {
        let cases = [
            ("ABC-123", "ABC"),
            ("T28-123", "T28"),
            ("FC2-1234567", "FC2"),
            ("111111_001", ""),
        ];

        for (input, expected) in cases {
            let number = MediaNumber::parse(input).expect("应解析成功");
            assert_eq!(number.prefix(), expected, "输入 {input}");
        }
    }
}
