//! DMM (FANZA) 数字版. 经由 GraphQL 取数, 不读取页面.
//!
//! 页面路径受地区限制, API 不受限制; 见 `docs/dev/network.md`。

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, WebClient};
use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::{Capability, RawBody, ScrapedMetadata, Source, SourceDescriptor, SourceError};

/// DMM 数字版的主机名. 装配限速覆盖与构造端点都取这里.
pub const HOST: &str = "api.video.dmm.co.jp";

const ENDPOINT: &str = "https://api.video.dmm.co.jp/graphql";

const QUERY: &str = r#"query FetchDigitalContent($id: ID!) {
  ppvContent(id: $id) {
    title
    description
    deliveryStartDate
    duration
    actresses { name }
    directors { name }
    series { name }
    maker { name }
    label { name }
    genres { name }
    packageImage { largeUrl mediumUrl }
  }
}"#;

/// DMM 数字版的来源标识, 用于错误说明.
const ID: &str = "dmm";

pub struct DmmSource {
    client: Arc<WebClient>,
}

impl DmmSource {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }
}

/// 番号 → DMM 内容标识: 字母前缀小写, 数字段补足五位.
///
/// 素人番号 (带下划线) 不在此列的编码规则内, 交由其它来源处理。
pub fn content_id(number: &MediaNumber) -> Result<String, SourceError> {
    let raw = number.as_str();
    let (prefix, serial) = raw.split_once('-').ok_or_else(|| SourceError::NotFound {
        id: ID,
        number: raw.to_owned(),
    })?;

    // 数字段超过五位的是 FC2 一类的独立编码体系, 不属于数字版.
    if serial.is_empty()
        || serial.len() > 5
        || !serial.chars().all(|character| character.is_ascii_digit())
    {
        return Err(SourceError::NotFound {
            id: ID,
            number: raw.to_owned(),
        });
    }

    let prefix = prefix.to_ascii_lowercase();
    if prefix.len() < 2
        || !prefix
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(SourceError::NotFound {
            id: ID,
            number: raw.to_owned(),
        });
    }

    Ok(format!("{prefix}{serial:0>5}"))
}

#[async_trait]
impl Source for DmmSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let cid = content_id(number)?;
        let payload = json!({
            "operationName": "FetchDigitalContent",
            "variables": { "id": cid },
            "query": QUERY,
        });

        let body = self
            .client
            .post_text(ENDPOINT, &payload)
            .await
            .map_err(transport_error)?;
        let response = decode_body(&body)?;

        parse_digital(&response, number)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "DMM",
    base_url: "https://api.video.dmm.co.jp/graphql",
    host: HOST,
    rate: amane_net::Rate::each_second(2, 2),
    capabilities: &[Capability::Metadata],
};

fn transport_error(error: NetError) -> SourceError {
    SourceError::Transport {
        id: ID,
        message: error.to_string(),
    }
}

/// 解析响应体. 不是 JSON 时连同原文一起判解析失败: 维护页与拦截页走这条路径.
fn decode_body(body: &str) -> Result<Value, SourceError> {
    serde_json::from_str(body).map_err(|error| {
        SourceError::parse(
            ID,
            format!("响应不是 JSON: {error}"),
            RawBody {
                content: body.to_owned(),
                extension: "txt",
            },
        )
    })
}

fn parse_failure(message: &str, response: &Value) -> SourceError {
    SourceError::parse(
        ID,
        message.to_owned(),
        RawBody {
            content: response.to_string(),
            extension: "json",
        },
    )
}

/// 解析 GraphQL 响应.
///
/// 三种结论分开: 没有可识别的结构视为解析失败并保留原文, `ppvContent` 为 null 视为未收录,
/// 其余按字段缺失处理.
fn parse_digital(response: &Value, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
    let data = &response["data"];
    if !data.is_object() {
        return Err(parse_failure("响应缺少 data 对象", response));
    }

    let content = &data["ppvContent"];
    if content.is_null() {
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }
    if !content.is_object() {
        return Err(parse_failure("ppvContent 不是对象", response));
    }

    let duration = content["duration"].as_i64().unwrap_or_default();
    let release_date = content["deliveryStartDate"]
        .as_str()
        .and_then(|raw| raw.get(..10))
        .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());

    Ok(ScrapedMetadata {
        title: text(&content["title"]),
        plot: text(&content["description"]),
        release_date,
        runtime_minutes: (duration > 0).then(|| i32::try_from(duration / 60).unwrap_or_default()),
        maker: text(&content["maker"]["name"]),
        label: text(&content["label"]["name"]),
        series: text(&content["series"]["name"]),
        directors: names(&content["directors"]),
        actors: names(&content["actresses"]),
        genres: names(&content["genres"]),
        poster_urls: text(&content["packageImage"]["largeUrl"])
            .or_else(|| text(&content["packageImage"]["mediumUrl"]))
            .into_iter()
            .collect(),
        // 数字版接口只给竖版包装图, 没有横版图; 横版由其它来源补.
        thumb_urls: Vec::new(),
        ..ScrapedMetadata::default()
    })
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| text(&item["name"]))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;
    use serde_json::json;

    use super::{content_id, decode_body, parse_digital};

    #[test]
    fn maps_number_to_content_id() {
        let cases = [
            ("SSIS-001", "ssis00001"),
            ("IPX-001", "ipx00001"),
            ("T28-123", "t2800123"),
            ("ABC-1234", "abc01234"),
        ];

        for (raw, expected) in cases {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            assert_eq!(
                content_id(&number).expect("应映射成功"),
                expected,
                "输入 {raw}"
            );
        }
    }

    #[test]
    fn rejects_numbers_outside_digital_scheme() {
        let cases = ["111111_001", "FC2-1234567", "ABC-123Z"];

        for raw in cases {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            assert!(content_id(&number).is_err(), "输入 {raw} 不应有内容标识");
        }
    }

    #[test]
    fn parses_full_response() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");
        let response = json!({
            "data": {
                "ppvContent": {
                    "title": "标题",
                    "description": "简介",
                    "deliveryStartDate": "2021-02-18T01:00:00Z",
                    "duration": 8826,
                    "actresses": [{ "name": "葵つかさ" }, { "name": "乙白さやか" }],
                    "directors": [{ "name": "粟原" }],
                    "series": null,
                    "maker": { "name": "エスワン" },
                    "label": { "name": "S1 NO.1 STYLE" },
                    "genres": [{ "name": "ドラマ" }],
                    "packageImage": { "largeUrl": "https://example.invalid/p.jpg", "mediumUrl": null }
                }
            }
        });

        let scraped = parse_digital(&response, &number).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("标题"));
        assert_eq!(scraped.runtime_minutes, Some(147));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2021-02-18".to_owned())
        );
        assert_eq!(scraped.maker.as_deref(), Some("エスワン"));
        assert_eq!(scraped.series, None);
        assert_eq!(scraped.actors, vec!["葵つかさ", "乙白さやか"]);
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://example.invalid/p.jpg")
        );
    }

    #[test]
    fn missing_content_is_not_found() {
        let number = MediaNumber::parse("ZZZ-999").expect("应解析成功");
        let response = json!({ "data": { "ppvContent": null } });

        let result = parse_digital(&response, &number);

        assert!(matches!(result, Err(super::SourceError::NotFound { .. })));
    }

    #[test]
    fn non_json_body_keeps_raw() {
        let body = "<html><body>维护中</body></html>";

        let error = decode_body(body).expect_err("非 JSON 应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("JSON"), "说明应指出期望的格式: {message}");
                assert_eq!(raw.content, body, "应保留完整原文");
                assert_eq!(raw.extension, "txt");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
        assert!(
            !error.to_string().contains("维护中"),
            "错误说明不应携带原文, 实际 {error}"
        );
    }

    #[test]
    fn structural_anomaly_keeps_raw_response() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");
        let cases = [
            (json!({ "errors": [{ "message": "维护中" }] }), "data"),
            (json!({ "data": { "ppvContent": "维护中" } }), "ppvContent"),
        ];

        for (response, missing) in cases {
            let error = parse_digital(&response, &number).expect_err("结构不符应判解析失败");

            match &error {
                super::SourceError::Parse { message, raw, .. } => {
                    assert!(
                        message.contains(missing),
                        "说明应指出缺的是什么, 实际 {message}"
                    );
                    assert_eq!(raw.content, response.to_string(), "应保留完整原文");
                    assert_eq!(raw.extension, "json");
                }
                other => panic!("应判解析失败, 实际 {other:?}"),
            }
            assert!(
                !error.to_string().contains("维护中"),
                "错误说明不应携带原文, 实际 {error}"
            );
        }
    }

    #[test]
    fn tolerates_partial_content() {
        let number = MediaNumber::parse("ZZZ-001").expect("应解析成功");
        let response = json!({ "data": { "ppvContent": { "title": "" , "duration": 0 } } });

        let scraped = parse_digital(&response, &number).expect("应解析成功");

        assert_eq!(scraped.title, None, "空字符串不算有效值");
        assert_eq!(scraped.runtime_minutes, None);
        assert!(scraped.actors.is_empty());
    }
}
