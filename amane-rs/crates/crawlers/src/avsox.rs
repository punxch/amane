//! avsox. 页面是空壳, 元数据走 Yii 的 JSON 接口: 先取 csrf 令牌, 再调检索与详情.
//!
//! 唯一走 JSON 接口的来源: 原文留存的扩展名记 `json`, 记成 `html` 会误导排查.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, RequestOptions, WebClient};
use async_trait::async_trait;
use scraper::Html;
use serde_json::{Value, json};

use crate::dom::{attrs, parse_date};
use crate::number::select;
use crate::{Capability, RawBody, ScrapedMetadata, Source, SourceDescriptor, SourceError};

/// 请求实际打到的主机名; 限速按它分桶.
pub const HOST: &str = "avsox.click";

const BASE_URL: &str = "https://avsox.click";

const ID: &str = "avsox";

/// 站点语言. 多语言字段按它取值, 不混拼.
const LANG: &str = "cn";

/// 实体名字段的语言后缀, 声明顺序即取用顺序.
const NAME_SUFFIXES: [&str; 5] = ["", "_ja", "_en", "_cn", "_tw"];

pub struct AvsoxSource {
    client: Arc<WebClient>,
}

impl AvsoxSource {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }

    /// 调一次接口并取回响应文本.
    async fn post(
        &self,
        method: &str,
        payload: &Value,
        referer: &str,
        token: Option<&str>,
    ) -> Result<String, SourceError> {
        let url = format!("{BASE_URL}/javu/data/api/{method}");
        self.client
            .post_text_with(&url, payload, &options(token, referer))
            .await
            .map_err(transport_error)
    }
}

#[async_trait]
impl Source for AvsoxSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let token = self.csrf_token().await?;
        let search_referer = format!("{BASE_URL}/{LANG}/search/{}", number.as_str());

        let body = self
            .post(
                "search",
                &json!([{ "search": number.as_str(), "lang": LANG }, 60, 1]),
                &search_referer,
                token.as_deref(),
            )
            .await?;
        let search = decode(&body)?;
        let results = envelope_data(&search, number)?;
        let movie_id = pick_movie_id(results, number)?.ok_or_else(|| SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        })?;

        let detail_referer = format!("{BASE_URL}/{LANG}/movies/{movie_id}");
        let body = self
            .post(
                "getMovie",
                &json!([movie_id, LANG]),
                &detail_referer,
                token.as_deref(),
            )
            .await?;

        parse_movie(&decode(&body)?, number)
    }
}

impl AvsoxSource {
    /// 壳页里的 csrf 令牌.
    ///
    /// 取不到时按无令牌继续: 站点可能不再要求它, 也可能因此回 403. 后者会以请求失败的形式
    /// 冒出来 (见 [`SourceError::Transport`]), 与「页面结构变了」区分得开.
    async fn csrf_token(&self) -> Result<Option<String>, SourceError> {
        let shell = self
            .client
            .get_text_with(&format!("{BASE_URL}/{LANG}/"), &RequestOptions::new())
            .await
            .map_err(transport_error)?;

        let token = csrf_token(&shell);
        if token.is_none() {
            tracing::warn!("avsox 壳页没有 csrf 令牌, 按无令牌发送请求");
        }

        Ok(token)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "AVSOX",
    base_url: BASE_URL,
    hosts: &[HOST],
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

/// 接口要求的请求头. 站点按 `X-Requested-With` 区分页内脚本与直接访问.
fn options(token: Option<&str>, referer: &str) -> RequestOptions {
    let options = RequestOptions::new()
        .header("accept", "application/json, text/plain, */*")
        .header("origin", BASE_URL)
        .header("referer", referer)
        .header("x-requested-with", "XMLHttpRequest");

    match token {
        Some(token) => options.header("x-csrf-token", token),
        None => options,
    }
}

fn transport_error(error: NetError) -> SourceError {
    SourceError::Transport {
        id: ID,
        message: error.to_string(),
    }
}

fn parse_failure(message: &str, raw: &str) -> SourceError {
    SourceError::parse(
        ID,
        message.to_owned(),
        RawBody {
            content: raw.to_owned(),
            extension: "json",
        },
    )
}

fn csrf_token(html: &str) -> Option<String> {
    let document = Html::parse_document(html);

    attrs(&document, r#"meta[name="csrf-token"]"#, "content")
        .into_iter()
        .next()
}

/// 响应体 → JSON. 不是 JSON 时连同原文一起判解析失败.
fn decode(body: &str) -> Result<Value, SourceError> {
    serde_json::from_str(body)
        .map_err(|error| parse_failure(&format!("响应不是 JSON: {error}"), body))
}

/// 拆开 `{code, data}` 信封.
///
/// `code` 不是 200 说明站点给出了答复但没有内容 (检索无结果时也走这条); 连 `code` 都没有
/// 说明拿到的不是这个接口的响应, 属于结构变化.
fn envelope_data<'a>(envelope: &'a Value, number: &MediaNumber) -> Result<&'a Value, SourceError> {
    let Some(code) = envelope.get("code").and_then(Value::as_i64) else {
        return Err(parse_failure("接口响应缺少 code", &envelope.to_string()));
    };
    if code == 401 || code == 403 {
        // 令牌无效时站点也用 200 包一层: 这属于请求被拒, 不是「没有这部片」.
        return Err(SourceError::Transport {
            id: ID,
            message: format!("接口返回 {code}, 多半是 csrf 令牌无效"),
        });
    }
    if code != 200 {
        tracing::warn!(code, "avsox 接口未返回数据");
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    envelope
        .get("data")
        .ok_or_else(|| parse_failure("接口响应缺少 data", &envelope.to_string()))
}

/// 检索结果里与请求番号同一部的影片标识.
///
/// 结果不是数组说明接口换了结构, 不是「没有这部片」: 两者都要能分开报出来.
fn pick_movie_id(results: &Value, number: &MediaNumber) -> Result<Option<String>, SourceError> {
    let Some(items) = results.as_array() else {
        return Err(parse_failure("检索结果不是数组", &results.to_string()));
    };
    let found: Vec<String> = items
        .iter()
        .map(|item| text(&item["movieFanHao"]).unwrap_or_default())
        .collect();
    let Some(index) = select(&found, number.as_str()) else {
        return Ok(None);
    };

    Ok(text(&items[index]["movieId"]))
}

fn parse_movie(envelope: &Value, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
    let movie = envelope_data(envelope, number)?;
    if !movie.is_object() {
        return Err(parse_failure("详情响应缺少影片对象", &envelope.to_string()));
    }
    if text(&movie["movieFanHao"]).is_none() {
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    let director = named(&movie["director"], "director");
    let extrafanart: Vec<String> = movie["sampleLarge"]
        .as_array()
        .map(|items| items.iter().filter_map(text).collect())
        .unwrap_or_default();

    let mut scraped = ScrapedMetadata {
        title: first_text(
            movie,
            &["title", "title_cn", "title_ja", "title_en", "title_tw"],
        ),
        plot: first_text(
            movie,
            &[
                "description",
                "description_cn",
                "description_ja",
                "description_en",
                "description_tw",
            ],
        ),
        release_date: text(&movie["releaseDate"]).as_deref().and_then(parse_date),
        runtime_minutes: movie["length"]
            .as_i64()
            .and_then(|minutes| i32::try_from(minutes).ok())
            .filter(|minutes| *minutes > 0),
        maker: named(&movie["studio"], "studio"),
        label: named(&movie["label"], "label"),
        series: named(&movie["series"], "series"),
        directors: director.into_iter().collect(),
        actors: named_list(&movie["star"], "star"),
        genres: named_list(&movie["genre"], "genre"),
        poster_urls: text(&movie["posterSmall"]).into_iter().collect(),
        thumb_urls: text(&movie["posterLarge"]).into_iter().collect(),
        ..ScrapedMetadata::default()
    };
    if !extrafanart.is_empty() {
        scraped.extrafanart_urls.insert(ID.to_owned(), extrafanart);
    }

    Ok(scraped)
}

/// 多语言字段按站点语言取一个, 不拼.
fn first_text(movie: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| text(&movie[*key]))
}

/// 实体名称: `studioName` 一类的字段按语言后缀逐个试.
fn named(entity: &Value, prefix: &str) -> Option<String> {
    // 同一字段有的返回对象, 有的返回单元素数组, 统一取第一个.
    let entity = match entity {
        Value::Array(items) => items.first()?,
        other => other,
    };
    if !entity.is_object() {
        return None;
    }

    NAME_SUFFIXES
        .iter()
        .find_map(|suffix| text(&entity[format!("{prefix}Name{suffix}")]))
}

fn named_list(value: &Value, prefix: &str) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| named(item, prefix))
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;
    use amane_net::NetError;
    use serde_json::json;

    use super::{csrf_token, decode, parse_movie, pick_movie_id, transport_error};

    const SHELL: &str = r#"<html><head><meta name="csrf-token" content="tok123" /></head></html>"#;

    fn number(raw: &str) -> MediaNumber {
        MediaNumber::parse(raw).expect("应解析成功")
    }

    fn movie() -> serde_json::Value {
        json!({
            "code": 200,
            "data": {
                "movieId": "abc",
                "movieFanHao": "HEYZO-1234",
                "title": "中文标题",
                "title_ja": "原題",
                "description_cn": "中文简介",
                "description_ja": "日本語の紹介",
                "releaseDate": "2020-12-25",
                "length": 120,
                "posterSmall": "https://example.invalid/p.jpg",
                "posterLarge": "https://example.invalid/c.jpg",
                "sampleLarge": ["https://example.invalid/s1.jpg", "https://example.invalid/s2.jpg"],
                "studio": { "studioName": "片商" },
                "label": { "labelName_cn": "厂牌" },
                "series": [{ "seriesName": "系列" }],
                "director": { "directorName": "导演" },
                "star": [{ "starName": "演员甲" }, { "starName_cn": "演员乙" }],
                "genre": [{ "genreName": "题材甲" }, { "genreName_cn": "题材乙" }]
            }
        })
    }

    #[test]
    fn reads_csrf_token_from_shell() {
        assert_eq!(csrf_token(SHELL).as_deref(), Some("tok123"));
        assert_eq!(
            csrf_token("<html><head></head></html>"),
            None,
            "没有令牌时返回 None, 由调用方决定怎么继续"
        );
    }

    #[test]
    fn picks_exact_number_from_results() {
        let results = json!([
            { "movieFanHao": "HEYZO-1235", "movieId": "other" },
            { "movieFanHao": "HEYZO-1234", "movieId": "target" }
        ]);

        assert_eq!(
            pick_movie_id(&results, &number("HEYZO-1234"))
                .expect("应解析成功")
                .as_deref(),
            Some("target"),
            "检索结果是模糊匹配, 不能取首条"
        );
    }

    #[test]
    fn empty_results_are_not_found() {
        assert_eq!(
            pick_movie_id(&json!([]), &number("HEYZO-1234")).expect("空结果不是解析失败"),
            None
        );
    }

    #[test]
    fn non_array_results_keep_raw() {
        let results = json!({ "movieFanHao": "HEYZO-1234" });

        let error =
            pick_movie_id(&results, &number("HEYZO-1234")).expect_err("不是数组应判解析失败");

        assert!(matches!(error, super::SourceError::Parse { .. }));
    }

    #[test]
    fn maps_multilingual_fields_by_site_language() {
        let scraped = parse_movie(&movie(), &number("HEYZO-1234")).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("中文标题"));
        assert_eq!(
            scraped.plot.as_deref(),
            Some("中文简介"),
            "简介按站点语言取, 不把两种语言拼在一起"
        );
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2020-12-25".to_owned())
        );
        assert_eq!(scraped.runtime_minutes, Some(120));
        assert_eq!(scraped.maker.as_deref(), Some("片商"));
        assert_eq!(scraped.label.as_deref(), Some("厂牌"), "名称带语言后缀");
        assert_eq!(scraped.series.as_deref(), Some("系列"));
        assert_eq!(scraped.directors, vec!["导演".to_owned()]);
        assert_eq!(
            scraped.actors,
            vec!["演员甲".to_owned(), "演员乙".to_owned()]
        );
        assert_eq!(
            scraped.genres,
            vec!["题材甲".to_owned(), "题材乙".to_owned()]
        );
    }

    #[test]
    fn keeps_poster_cover_and_samples() {
        let scraped = parse_movie(&movie(), &number("HEYZO-1234")).expect("应解析成功");

        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://example.invalid/p.jpg")
        );
        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://example.invalid/c.jpg")
        );
        assert_eq!(scraped.extrafanart_urls.get("avsox").map(Vec::len), Some(2));
    }

    #[test]
    fn non_json_body_keeps_raw_as_json() {
        let body = "<html><body>维护中</body></html>";

        let error = decode(body).expect_err("非 JSON 应判解析失败");

        match &error {
            super::SourceError::Parse { raw, .. } => {
                assert_eq!(raw.content, body);
                assert_eq!(raw.extension, "json", "原文后缀记 json 而不是 html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn changed_envelope_keeps_raw() {
        let envelope = json!({ "items": [], "message": "改版了" });

        let error =
            parse_movie(&envelope, &number("HEYZO-1234")).expect_err("缺 code 应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("code"), "说明应指出缺什么: {message}");
                assert_eq!(raw.extension, "json");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn empty_data_is_not_found() {
        let envelope = json!({ "code": 404, "data": null });

        let error = parse_movie(&envelope, &number("HEYZO-1234")).expect_err("无数据应判未收录");

        assert!(matches!(error, super::SourceError::NotFound { .. }));
    }

    #[test]
    fn forbidden_is_a_transport_error_not_a_parse_error() {
        let error = transport_error(NetError::Rejected { status: 403 });

        match &error {
            super::SourceError::Transport { message, .. } => {
                assert!(message.contains("403"), "说明应带上状态码: {message}");
            }
            other => panic!("令牌无效时站点回 403, 应判请求失败, 实际 {other:?}"),
        }
    }
}
