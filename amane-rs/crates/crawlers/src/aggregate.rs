//! 多源聚合: 标量取首个非空, 列表整份取首个非空来源, 不混拼.

use std::collections::BTreeMap;

use amane_core::FIELDS;

use crate::ScrapedMetadata;

/// 聚合结果.
#[derive(Debug, Clone, Default)]
pub struct Aggregated {
    pub metadata: ScrapedMetadata,
    /// 字段名 → 贡献该字段的来源标识.
    pub field_sources: BTreeMap<String, String>,
}

impl Aggregated {
    /// 字段来源的传输形态.
    pub fn field_sources_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.field_sources).unwrap_or(serde_json::Value::Null)
    }
}

/// 按给定的先后顺序合并多个来源的结果. 顺序即优先级.
pub fn aggregate(items: &[(&str, ScrapedMetadata)]) -> Aggregated {
    let mut merged = Aggregated::default();

    for (source, scraped) in items {
        // 标量字段的合并方式在字段表里, 这里只负责记下谁贡献了它.
        for spec in FIELDS {
            if (spec.merge)(&mut merged.metadata, scraped) {
                merged
                    .field_sources
                    .insert(spec.field.as_str().to_owned(), (*source).to_owned());
            }
        }

        // 封面与三类列表不可锁, 不参与逐字段修订, 各自整份合并.
        take_scalar(
            &mut merged,
            "cover_url",
            source,
            &scraped.cover_url,
            |metadata| &mut metadata.cover_url,
        );

        take_list(
            &mut merged,
            "directors",
            source,
            &scraped.directors,
            |metadata| &mut metadata.directors,
        );
        take_list(&mut merged, "actors", source, &scraped.actors, |metadata| {
            &mut metadata.actors
        });
        take_list(&mut merged, "genres", source, &scraped.genres, |metadata| {
            &mut metadata.genres
        });
    }

    merged
}

fn take_scalar<T: Clone>(
    merged: &mut Aggregated,
    field: &str,
    source: &str,
    value: &Option<T>,
    slot: impl Fn(&mut ScrapedMetadata) -> &mut Option<T>,
) {
    if slot(&mut merged.metadata).is_some() {
        return;
    }
    let Some(value) = value else {
        return;
    };

    *slot(&mut merged.metadata) = Some(value.clone());
    merged
        .field_sources
        .insert(field.to_owned(), source.to_owned());
}

fn take_list(
    merged: &mut Aggregated,
    field: &str,
    source: &str,
    values: &[String],
    slot: impl Fn(&mut ScrapedMetadata) -> &mut Vec<String>,
) {
    if values.is_empty() || !slot(&mut merged.metadata).is_empty() {
        return;
    }

    *slot(&mut merged.metadata) = values.to_vec();
    merged
        .field_sources
        .insert(field.to_owned(), source.to_owned());
}

#[cfg(test)]
mod tests {
    use super::aggregate;
    use crate::ScrapedMetadata;

    fn with_title(title: &str) -> ScrapedMetadata {
        ScrapedMetadata {
            title: Some(title.to_owned()),
            ..ScrapedMetadata::default()
        }
    }

    #[test]
    fn fills_gaps_from_later_sources() {
        let first = ScrapedMetadata {
            title: Some("标题".to_owned()),
            actors: Vec::new(),
            ..ScrapedMetadata::default()
        };
        let second = ScrapedMetadata {
            title: Some("另一个标题".to_owned()),
            maker: Some("片商".to_owned()),
            actors: vec!["演员".to_owned()],
            ..ScrapedMetadata::default()
        };

        let merged = aggregate(&[("a", first), ("b", second)]);

        assert_eq!(
            merged.metadata.title.as_deref(),
            Some("标题"),
            "标量取首个非空"
        );
        assert_eq!(
            merged.metadata.maker.as_deref(),
            Some("片商"),
            "缺的字段由后续来源补"
        );
        assert_eq!(merged.metadata.actors, vec!["演员".to_owned()]);
        assert_eq!(
            merged.field_sources.get("title").map(String::as_str),
            Some("a")
        );
        assert_eq!(
            merged.field_sources.get("maker").map(String::as_str),
            Some("b")
        );
    }

    #[test]
    fn keeps_first_non_empty_list_whole() {
        let first = ScrapedMetadata {
            actors: vec!["甲".to_owned()],
            ..ScrapedMetadata::default()
        };
        let second = ScrapedMetadata {
            actors: vec!["乙".to_owned(), "丙".to_owned()],
            ..ScrapedMetadata::default()
        };

        let merged = aggregate(&[("a", first), ("b", second)]);

        assert_eq!(
            merged.metadata.actors,
            vec!["甲".to_owned()],
            "列表整份取首个非空来源, 不混拼"
        );
    }

    #[test]
    fn genres_come_from_the_first_source_that_has_them() {
        let without_genres = ScrapedMetadata {
            title: Some("标题".to_owned()),
            ..ScrapedMetadata::default()
        };
        let with_genres = ScrapedMetadata {
            genres: vec!["ドラマ".to_owned(), "美少女".to_owned()],
            ..ScrapedMetadata::default()
        };

        let merged = aggregate(&[("a", without_genres), ("b", with_genres)]);

        assert_eq!(merged.metadata.genres, vec!["ドラマ", "美少女"]);
        assert_eq!(
            merged.field_sources.get("genres").map(String::as_str),
            Some("b"),
            "题材的字段来源应记到真正提供的那个来源"
        );
    }

    #[test]
    fn empty_input_yields_empty_result() {
        let merged = aggregate(&[]);

        assert!(merged.metadata.title.is_none());
        assert!(merged.field_sources.is_empty());
    }

    #[test]
    fn ignores_empty_lists_from_first_source() {
        let merged = aggregate(&[
            ("a", with_title("标题")),
            (
                "b",
                ScrapedMetadata {
                    actors: vec!["乙".to_owned()],
                    ..ScrapedMetadata::default()
                },
            ),
        ]);

        assert_eq!(merged.metadata.actors, vec!["乙".to_owned()]);
        assert_eq!(
            merged.field_sources.get("actors").map(String::as_str),
            Some("b")
        );
    }
}
