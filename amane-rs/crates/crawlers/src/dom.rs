//! 站点页面的通用取数: 文本归一、绝对地址、日期与时长.
//!
//! 这些函数只认 DOM 结构, 不认任何一家站点的语义; 选择器由调用方给出.
//! 选择器都是写死的字面量, 解析失败属于编码错误, 因此这里直接断言.

use chrono::NaiveDate;
use scraper::{ElementRef, Html, Selector};

/// 元素的可见文本, 连续空白折叠成一个空格.
pub fn text_of(element: &ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 第一个匹配元素的可读文本; 空文本按没有处理.
pub fn first_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).expect("固定选择器");
    document
        .select(&selector)
        .next()
        .map(|element| text_of(&element))
        .and_then(non_empty)
}

/// 匹配元素的可读文本, 按出现顺序, 跳过空项.
pub fn texts(document: &Html, selector: &str) -> Vec<String> {
    let selector = Selector::parse(selector).expect("固定选择器");
    document
        .select(&selector)
        .map(|element| text_of(&element))
        .filter(|text| !text.is_empty())
        .collect()
}

/// 匹配元素的某个属性值, 按出现顺序, 跳过空值.
pub fn attrs(document: &Html, selector: &str, name: &str) -> Vec<String> {
    let selector = Selector::parse(selector).expect("固定选择器");
    document
        .select(&selector)
        .filter_map(|element| element.value().attr(name))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 相对地址补成绝对地址.
pub fn absolute(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_owned();
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }

    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        href.trim_start_matches('/')
    )
}

/// 空字符串不算有效值.
pub fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// 站点写日期的两种形态: 连字符与斜杠.
pub fn parse_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    for format in ["%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(value, format) {
            return Some(date);
        }
    }
    None
}

/// 文本里的第一个日期.
///
/// 站点常把日期写在标签后面 (`販売日：2021/03/04`), 因此按空白与冒号切段逐段试.
pub fn find_date(value: &str) -> Option<NaiveDate> {
    value
        .split(|character: char| character.is_whitespace() || matches!(character, ':' | '：'))
        .find_map(|part| parse_date(part.trim()))
}

/// 时长: 取文本里的第一段数字, 单位按分钟.
pub fn parse_minutes(value: &str) -> Option<i32> {
    let digits: String = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();

    digits.parse::<i32>().ok().filter(|minutes| *minutes > 0)
}

#[cfg(test)]
mod tests {
    use super::{absolute, find_date, parse_date, parse_minutes};

    #[test]
    fn completes_relative_addresses() {
        let cases = [
            ("https://a.example", "/v/1", "https://a.example/v/1"),
            ("https://a.example", "v/1", "https://a.example/v/1"),
            (
                "https://a.example",
                "https://b.example/v/1",
                "https://b.example/v/1",
            ),
            (
                "https://a.example",
                "//b.example/v/1",
                "https://b.example/v/1",
            ),
        ];

        for (base, href, expected) in cases {
            assert_eq!(absolute(base, href), expected, "{base} + {href}");
        }
    }

    #[test]
    fn reads_both_date_styles() {
        assert_eq!(
            parse_date("2021-02-19").map(|date| date.to_string()),
            Some("2021-02-19".to_owned())
        );
        assert_eq!(
            parse_date("2020/12/25").map(|date| date.to_string()),
            Some("2020-12-25".to_owned())
        );
        assert_eq!(parse_date("2021年2月"), None, "不完整日期不猜");
    }

    #[test]
    fn finds_date_after_a_label() {
        assert_eq!(
            find_date("販売日：2021/03/04").map(|date| date.to_string()),
            Some("2021-03-04".to_owned())
        );
        assert_eq!(find_date("2021-02-19"), parse_date("2021-02-19"));
        assert_eq!(find_date("没有日期"), None);
    }

    #[test]
    fn reads_minutes_from_labelled_text() {
        assert_eq!(parse_minutes("150 分鐘"), Some(150));
        assert_eq!(parse_minutes("120 min"), Some(120));
        assert_eq!(parse_minutes("—"), None);
        assert_eq!(parse_minutes("0"), None, "零时长不算有效值");
    }
}
