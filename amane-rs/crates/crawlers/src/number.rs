//! 番号在站点上的写法差异: 检索词的多种形态, 以及搜索结果与请求番号的同一性判定.
//!
//! 站点的检索是模糊匹配, 首条结果未必是同一部; 而站内地址里的番号又常与规范形态差一截
//! (省略开头的系列数字段, FC2 写不写 `PPV`). 两件事分开: [`variants`] 决定试哪些写法,
//! [`select`] 决定哪条结果算命中.

use amane_core::MediaNumber;

/// 站内番号与检索番号的同一性, 声明顺序即精确度.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberMatch {
    None,
    /// 只有一侧写了开头的数字段, 其余部分全等.
    WithoutLeadingDigits,
    Exact,
}

/// 该番号在站点上可能的检索词, 原样写法在前, 去重保序.
///
/// 只处理 FC2 的 `PPV` 词缀: 官方站与索引站各写一种. 站内地址省略开头数字段的情况由
/// [`match_number`] 在比对时放宽, 不需要额外的检索词 —— 番号提取本身不产出带系列号的形态.
pub fn variants(number: &MediaNumber) -> Vec<String> {
    let raw = number.as_str();
    let mut variants: Vec<String> = vec![raw.to_owned()];

    if let Some(article_id) = fc2_article_id(raw) {
        for variant in [format!("FC2-PPV-{article_id}"), format!("FC2-{article_id}")] {
            if !variants.contains(&variant) {
                variants.push(variant);
            }
        }
    }

    variants
}

/// 站内番号与检索番号是否同一部, 以及精确度.
pub fn match_number(found: &str, query: &str) -> NumberMatch {
    if found.trim().is_empty() || query.trim().is_empty() {
        return NumberMatch::None;
    }
    if fold(found) == fold(query) {
        return NumberMatch::Exact;
    }
    if let (Some(left), Some(right)) = (fc2_article_id(found), fc2_article_id(query))
        && left == right
    {
        return NumberMatch::Exact;
    }

    // 只有一侧写了数字段才放宽: 两侧都写了却不同, 是两部不同的片.
    let (found_digits, found_rest) = split_leading_digits(found);
    let (query_digits, query_rest) = split_leading_digits(query);
    if !found_rest.is_empty()
        && found_rest == query_rest
        && (found_digits.is_empty() || query_digits.is_empty())
    {
        return NumberMatch::WithoutLeadingDigits;
    }

    NumberMatch::None
}

/// 候选番号中与 `query` 同一部者的下标.
///
/// 精确优先; 退而只在「仅差数字段」的候选**唯一**时取它 —— 出现两个及以上说明站点上确有
/// 同名不同系列号的条目, 猜一个会返回无关影片, 因此视为未命中.
pub fn select(found: &[String], query: &str) -> Option<usize> {
    let mut exact = None;
    let mut loose = Vec::new();

    for (index, item) in found.iter().enumerate() {
        match match_number(item, query) {
            NumberMatch::Exact if exact.is_none() => exact = Some(index),
            NumberMatch::Exact => {}
            NumberMatch::WithoutLeadingDigits => loose.push(index),
            NumberMatch::None => {}
        }
    }

    exact.or(match loose.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// 同一性比较用的折叠形态: 大写并只留字母数字.
fn fold(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

/// FC2 番号的商品 ID; 不是 FC2 形态时为 `None`.
fn fc2_article_id(number: &str) -> Option<String> {
    let folded = fold(number);
    let rest = folded.strip_prefix("FC2")?;
    let digits = rest.strip_prefix("PPV").unwrap_or(rest);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.to_owned())
}

/// 番号开头的数字段; 没有时为 `None`.
///
/// 素人系的系列代码写在番号最前面 (如 `390JAC-079` 的 `390`), 站点写不写它不一致.
/// 后面不是字母时不剥: 裸数字号不参与这条规则.
fn leading_digits(number: &str) -> Option<String> {
    let end = number
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(number.len());
    if end == 0 {
        return None;
    }
    let next = number[end..].chars().next()?;
    next.is_ascii_alphabetic().then(|| number[..end].to_owned())
}

/// 折叠后的番号 → (开头的数字段, 其余部分).
fn split_leading_digits(number: &str) -> (String, String) {
    let folded = fold(number);
    match leading_digits(&folded) {
        Some(digits) => {
            let rest = folded[digits.len()..].to_owned();
            (digits, rest)
        }
        None => (String::new(), folded),
    }
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{NumberMatch, match_number, select, variants};

    fn parsed(raw: &str) -> MediaNumber {
        MediaNumber::parse(raw).expect("应解析成功")
    }

    #[test]
    fn keeps_original_writing_first() {
        assert_eq!(variants(&parsed("SSIS-001")), ["SSIS-001"]);
    }

    #[test]
    fn offers_both_fc2_writings() {
        assert_eq!(
            variants(&parsed("FC2-1234567")),
            ["FC2-1234567", "FC2-PPV-1234567"],
            "官方站写 PPV, 索引站不写, 两种都要试"
        );
    }

    #[test]
    fn strips_leading_series_digits() {
        assert_eq!(
            variants(&parsed("JAC-079")),
            ["JAC-079"],
            "站内写不写系列号由比对放宽, 不再多试一种检索词"
        );
    }

    #[test]
    fn matches_folded_and_fc2_forms() {
        let cases = [
            ("SSIS-001", "SSIS-001", NumberMatch::Exact),
            ("ssis001", "SSIS-001", NumberMatch::Exact),
            ("FC2-PPV-1234567", "FC2-1234567", NumberMatch::Exact),
            ("JAC-079", "390JAC-079", NumberMatch::WithoutLeadingDigits),
            ("390JAC-079", "JAC-079", NumberMatch::WithoutLeadingDigits),
            ("SSIS-002", "SSIS-001", NumberMatch::None),
            ("200MIUM-1000", "300MIUM-1000", NumberMatch::None),
            ("ABF-375", "ABF-35", NumberMatch::None),
            ("", "SSIS-001", NumberMatch::None),
        ];

        for (found, query, expected) in cases {
            assert_eq!(match_number(found, query), expected, "{found} vs {query}");
        }
    }

    #[test]
    fn select_prefers_exact_over_loose() {
        let found = ["JAC-079".to_owned(), "390JAC-079".to_owned()];

        assert_eq!(
            select(&found, "390JAC-079"),
            Some(1),
            "精确优先于仅差数字段"
        );
    }

    #[test]
    fn select_refuses_ambiguous_loose_candidates() {
        let found = ["200MIUM-1000".to_owned(), "300MIUM-1000".to_owned()];

        assert_eq!(
            select(&found, "MIUM-1000"),
            None,
            "两个同名不同系列号时猜一个会返回无关影片"
        );
    }

    #[test]
    fn select_takes_the_only_loose_candidate() {
        let found = ["JAC-079".to_owned()];

        assert_eq!(select(&found, "390JAC-079"), Some(0));
    }
}
