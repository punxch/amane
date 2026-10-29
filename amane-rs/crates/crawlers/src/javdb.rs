//! javdb. 先按番号搜索, 再取详情页面板.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, WebClient};
use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::dom::{non_empty, parse_date, parse_minutes, text_of};
use crate::{Capability, RawBody, ScrapedMetadata, Source, SourceDescriptor, SourceError};

pub const HOST: &str = "javdb.com";

const ID: &str = "javdb";

pub struct JavdbSource {
    client: Arc<WebClient>,
}

impl JavdbSource {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Source for JavdbSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let search_url = format!(
            "https://{HOST}/search?q={}&f=all",
            urlencode(number.as_str())
        );
        let search = self
            .client
            .get_text(&search_url)
            .await
            .map_err(transport_error)?;
        reject_blocked(&search)?;

        let path = first_result_path(&search, number)?;
        let detail_url = format!("https://{HOST}{path}");
        let detail = self
            .client
            .get_text(&detail_url)
            .await
            .map_err(transport_error)?;
        reject_blocked(&detail)?;

        parse_detail(&detail, number)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "JavDB",
    base_url: "https://javdb.com",
    host: HOST,
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

fn transport_error(error: NetError) -> SourceError {
    SourceError::Transport {
        id: ID,
        message: error.to_string(),
    }
}

/// 地区封锁页的识别标记. 站点按出口 IP 拒绝时返回一个短页, 不是影片页也不是「暂无内容」.
const BLOCK_MARKERS: &[&str] = &["prohibited in the country", "版權限制", "版权限制"];

/// 把封锁页认成「没有结果列表」会让人去修一个没坏的解析器.
fn reject_blocked(html: &str) -> Result<(), SourceError> {
    if BLOCK_MARKERS.iter().any(|marker| html.contains(marker)) {
        return Err(SourceError::Blocked { id: ID });
    }

    Ok(())
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// 搜索结果里第一个影片的路径.
///
/// 站点给出「暂无内容」时视为未收录; 没有结果列表时视为解析失败, 原文随错误返回.
fn first_result_path(html: &str, number: &MediaNumber) -> Result<String, SourceError> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"a.box[href^="/v/"]"#).expect("固定选择器");

    if let Some(path) = document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("href"))
    {
        return Ok(path.to_owned());
    }

    let empty_marker = Selector::parse(".empty-message").expect("固定选择器");
    if document.select(&empty_marker).next().is_some() {
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    Err(parse_failure("搜索结果页没有结果列表", html))
}

fn parse_failure(message: &str, html: &str) -> SourceError {
    SourceError::parse(
        ID,
        message.to_owned(),
        RawBody {
            content: html.to_owned(),
            extension: "html",
        },
    )
}

/// 详情页面板: 每个 `div.panel-block` 是「标签 + 值」.
fn parse_detail(html: &str, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
    let document = Html::parse_document(html);
    let block_selector = Selector::parse("div.panel-block").expect("固定选择器");
    let strong_selector = Selector::parse("strong").expect("固定选择器");
    let value_selector = Selector::parse("span.value").expect("固定选择器");
    let link_selector = Selector::parse("a").expect("固定选择器");
    let title_selector = Selector::parse("h2.title strong.current-title").expect("固定选择器");
    let origin_selector = Selector::parse("span.origin-title").expect("固定选择器");

    let (poster_url, cover_url) = cover_images(&document);
    let mut metadata = ScrapedMetadata {
        poster_urls: poster_url.into_iter().collect(),
        thumb_urls: cover_url.into_iter().collect(),
        title: document
            .select(&title_selector)
            .next()
            .map(|element| text_of(&element))
            .filter(|text| !text.is_empty())
            .or_else(|| {
                document
                    .select(&origin_selector)
                    .next()
                    .map(|element| text_of(&element))
                    .filter(|text| !text.is_empty())
            }),
        ..ScrapedMetadata::default()
    };

    if document.select(&block_selector).next().is_none() {
        return Err(parse_failure("详情页没有信息面板", html));
    }

    let mut matched_number = false;
    for block in document.select(&block_selector) {
        let label = block
            .select(&strong_selector)
            .next()
            .map(|element| text_of(&element))
            .unwrap_or_default();
        let Some(value_element) = block.select(&value_selector).next() else {
            continue;
        };
        let value = text_of(&value_element);

        // 站点按语言返回不同标签: 同一字段在中文页与 English 页名称不同.
        match label.trim_end_matches(':').trim() {
            "番號" | "ID" => {
                matched_number = value.replace(' ', "").eq_ignore_ascii_case(number.as_str());
            }
            "日期" | "Released Date" => metadata.release_date = parse_date(&value),
            "時長" | "Duration" => metadata.runtime_minutes = parse_minutes(&value),
            "導演" | "Director" => {
                metadata.directors = element_texts(&value_element, &link_selector)
            }
            "片商" | "製作商" | "Maker" => metadata.maker = non_empty(value),
            "發行" | "發行商" | "Label" => metadata.label = non_empty(value),
            "系列" | "Series" => metadata.series = non_empty(value),
            "類別" | "Tags" => metadata.genres = element_texts(&value_element, &link_selector),
            "演員" | "Actor(s)" => {
                metadata.actors = element_texts(&value_element, &link_selector)
            }
            _ => {}
        }
    }

    if !matched_number {
        tracing::warn!(number = %number, "javdb 详情页番号与请求不符");
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    Ok(metadata)
}

/// 竖版海报与横版封面.
///
/// 详情页把两张图都标成 `video-cover`, 靠 `data-movie-detail-target="cover"` 区分:
/// 带这个属性的是横版剧照, 另一个是竖版包装图.
fn cover_images(document: &Html) -> (Option<String>, Option<String>) {
    let selector = Selector::parse("img.video-cover").expect("固定选择器");
    let mut poster = None;
    let mut cover = None;

    for element in document.select(&selector) {
        let Some(src) = element.value().attr("src").map(str::trim) else {
            continue;
        };
        if src.is_empty() {
            continue;
        }
        let is_cover = element
            .value()
            .attr("data-movie-detail-target")
            .is_some_and(|value| value == "cover");
        if is_cover {
            cover.get_or_insert_with(|| src.to_owned());
        } else {
            poster.get_or_insert_with(|| src.to_owned());
        }
    }

    (poster, cover)
}

fn element_texts(element: &scraper::ElementRef<'_>, selector: &Selector) -> Vec<String> {
    element
        .select(selector)
        .map(|link| text_of(&link))
        .filter(|text| !text.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{first_result_path, parse_detail, reject_blocked, urlencode};

    const SEARCH: &str = r#"
        <div class="movie-list">
          <div class="item"><a href="/v/ZY5eq" class="box" title="目标">SSIS-001</a></div>
          <div class="item"><a href="/v/other" class="box" title="其它">SSIS-002</a></div>
        </div>
    "#;

    /// 空结果页: 没有结果链接, 但站点明确给出提示.
    const EMPTY_SEARCH: &str =
        r#"<div class="movie-list-empty"><div class="empty-message">暫無內容</div></div>"#;

    /// 既没有结果列表也没有提示, 通常是拦截页或改版.
    const CHALLENGE: &str = r#"<html><body><h1>Just a moment...</h1></body></html>"#;

    /// 地区封锁页: 站点按出口 IP 拒绝, 正文只有一句版权说明.
    const BLOCKED: &str = r#"<html><body>
        Due to copyright restrictions, access to this site is prohibited in the country where your internet is located.
        <br /><br />
        由於版權限制，本站禁止了你的網路所在國家的訪問。
    </body></html>"#;

    const DETAIL: &str = r#"
        <div class="video-detail">
          <h2 class="title is-4">
            <strong>SSIS-001 </strong>
            <strong class="current-title">中文标题</strong>
            <span style="display: none" class="origin-title">原题</span>
          </h2>
          <nav class="panel movie-panel-info">
            <div class="panel-block"><strong>番號:</strong><span class="value">SSIS-001</span></div>
            <div class="panel-block"><strong>日期:</strong><span class="value">2021-02-19</span></div>
            <div class="panel-block"><strong>時長:</strong><span class="value">150 分鐘</span></div>
            <div class="panel-block"><strong>片商:</strong><span class="value">エスワン</span></div>
            <div class="panel-block"><strong>系列:</strong><span class="value"></span></div>
            <div class="panel-block">
              <strong>演員:</strong>
              <span class="value"><a>葵つかさ</a><a>乙白さやか</a></span>
            </div>
            <div class="panel-block">
              <strong>類別:</strong>
              <span class="value"><a>ドラマ</a></span>
            </div>
          </nav>
        </div>
    "#;

    #[test]
    fn takes_first_result_link() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");

        assert_eq!(
            first_result_path(SEARCH, &number).expect("应取到链接"),
            "/v/ZY5eq"
        );
    }

    #[test]
    fn empty_result_page_is_not_found() {
        let number = MediaNumber::parse("ZZZ-999").expect("应解析成功");

        let result = first_result_path(EMPTY_SEARCH, &number);

        assert!(
            matches!(result, Err(super::SourceError::NotFound { .. })),
            "站点明确表示暂无内容时应报未收录"
        );
    }

    #[test]
    fn blocked_page_is_reported_as_blocked() {
        let error = reject_blocked(BLOCKED).expect_err("封锁页应报地区封锁");

        assert!(
            matches!(error, super::SourceError::Blocked { .. }),
            "封锁页不能混进解析失败, 实际 {error:?}"
        );
        assert!(
            error.to_string().contains("地区封锁"),
            "说明应指出换出口即可解决, 实际 {error}"
        );
    }

    #[test]
    fn ordinary_pages_are_not_taken_for_blocked() {
        for page in [SEARCH, EMPTY_SEARCH, CHALLENGE] {
            assert!(
                reject_blocked(page).is_ok(),
                "只有封锁页才报地区封锁, 实际误判了 {page}"
            );
        }
    }

    #[test]
    fn unrecognized_search_page_keeps_raw() {
        let number = MediaNumber::parse("ZZZ-999").expect("应解析成功");

        let error = first_result_path(CHALLENGE, &number).expect_err("应判解析失败");

        match &error {
            super::SourceError::Parse { raw, .. } => {
                assert_eq!(raw.content, CHALLENGE, "应保留完整原文");
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn parses_detail_panel() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");

        let scraped = parse_detail(DETAIL, &number).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("中文标题"));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2021-02-19".to_owned())
        );
        assert_eq!(scraped.runtime_minutes, Some(150));
        assert_eq!(scraped.maker.as_deref(), Some("エスワン"));
        assert_eq!(scraped.series, None, "空值不应写成空字符串");
        assert_eq!(scraped.actors, vec!["葵つかさ", "乙白さやか"]);
        assert_eq!(scraped.genres, vec!["ドラマ"]);
    }

    /// 详情页的两张图: 带 `data-movie-detail-target="cover"` 的是横版剧照, 另一张是竖版海报.
    const DETAIL_WITH_IMAGES: &str = r#"
        <div class="video-detail">
          <h2 class="title is-4"><strong class="current-title">标题</strong></h2>
          <div class="column column-video-cover">
            <img class="video-cover" width="600" height="404" data-movie-detail-target="cover"
                 src="https://example.invalid/covers/zy/ZY5eq.jpg" />
          </div>
          <nav class="panel movie-panel-info">
            <div class="panel-block"><strong>番號:</strong><span class="value">SSIS-001</span></div>
          </nav>
          <img src="https://example.invalid/posters/zy/ZY5eq.jpg" class="video-cover" />
        </div>
    "#;

    const DETAIL_EN: &str = r#"
        <div class="video-detail">
          <h2 class="title is-4"><strong class="current-title">English title</strong></h2>
          <nav class="panel movie-panel-info">
            <div class="panel-block"><strong>ID:</strong><span class="value">SSIS-001</span></div>
            <div class="panel-block"><strong>Released Date:</strong><span class="value">2021-02-19</span></div>
            <div class="panel-block"><strong>Duration:</strong><span class="value">150 min</span></div>
            <div class="panel-block"><strong>Actor(s):</strong><span class="value"><a>葵つかさ</a></span></div>
          </nav>
        </div>
    "#;

    #[test]
    fn parses_english_panel_labels() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");

        let scraped = parse_detail(DETAIL_EN, &number).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("English title"));
        assert_eq!(scraped.runtime_minutes, Some(150));
        assert_eq!(scraped.actors, vec!["葵つかさ"]);
    }

    #[test]
    fn detail_without_panels_keeps_raw() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");

        let error = parse_detail(CHALLENGE, &number).expect_err("没有信息面板应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("信息面板"), "说明应指出缺什么: {message}");
                assert_eq!(raw.content, CHALLENGE, "应保留完整原文");
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn extracts_poster_and_cover_separately() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");

        let scraped = parse_detail(DETAIL_WITH_IMAGES, &number).expect("应解析成功");

        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://example.invalid/covers/zy/ZY5eq.jpg"),
            "横版剧照归入封面"
        );
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://example.invalid/posters/zy/ZY5eq.jpg"),
            "竖版包装图归入海报"
        );
    }

    #[test]
    fn missing_cover_leaves_only_poster() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");
        let html = DETAIL_WITH_IMAGES.replace("data-movie-detail-target=\"cover\"", "");

        let scraped = parse_detail(&html, &number).expect("应解析成功");

        assert!(scraped.thumb_urls.is_empty(), "没有横版图时列表为空");
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://example.invalid/covers/zy/ZY5eq.jpg"),
            "剩下的那张按海报处理"
        );
    }

    #[test]
    fn rejects_page_for_another_number() {
        let number = MediaNumber::parse("ABC-999").expect("应解析成功");

        let result = parse_detail(DETAIL, &number);

        assert!(
            matches!(result, Err(super::SourceError::NotFound { .. })),
            "番号不符时必须报未收录"
        );
    }

    #[test]
    fn falls_back_to_origin_title() {
        let number = MediaNumber::parse("SSIS-001").expect("应解析成功");
        let html = DETAIL.replace(r#"<strong class="current-title">中文标题</strong>"#, "");

        let scraped = parse_detail(&html, &number).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("原题"));
    }

    #[test]
    fn encodes_search_terms() {
        assert_eq!(urlencode("SSIS-001"), "SSIS-001");
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
    }
}
